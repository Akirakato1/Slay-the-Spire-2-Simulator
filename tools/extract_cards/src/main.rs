//! One-shot card-data extractor.
//!
//! Walks the StS2 decompile, parses each `Models/Cards/*.cs` and
//! `Models/CardPools/*.cs`, and emits a JSON table to
//! `crates/sts2-sim/data/cards.json`.
//!
//! Why text-parse and not reflection on `sts2.dll`?
//!   - The decompile is already our spec for every other port.
//!   - Card files are tiny and follow a mechanical pattern out of the
//!     decompiler — regex is sufficient.
//!   - Avoids the ModelDb / GodotSharp singleton dance reflection would need.
//!
//! Re-run on game updates: `cargo run -p extract-cards`.

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// Default decompile root on the project's primary dev machine; can be
// overridden by STS2_DECOMPILE_DIR. We point at the `Core/Models` directory.
const DEFAULT_MODELS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sts2_decompiled\sts2\MegaCrit\sts2\Core\Models";

// Output is relative to the workspace root (sim/).
const OUTPUT_REL: &str = r"crates\sts2-sim\data\cards.json";

#[derive(Debug, Serialize, Deserialize)]
struct CardData {
    id: String,
    pool: String,
    energy_cost: i32,
    card_type: String,
    rarity: String,
    target_type: String,
    show_in_library: bool,
    has_energy_cost_x: bool,
    max_upgrade_level: i32,
    /// `base.EnergyCost.UpgradeBy(N)` in OnUpgrade — change to base energy
    /// cost on upgrade. Most cards leave cost unchanged on upgrade (0).
    /// Common pattern is `-1`; BansheesCry uses `-2`.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    energy_cost_upgrade_delta: i32,
    tags: Vec<String>,
    canonical_vars: Vec<CardVar>,
    upgrade_deltas: Vec<UpgradeDelta>,
}

fn is_zero_i32(v: &i32) -> bool { *v == 0 }

#[derive(Debug, Serialize, Deserialize)]
struct CardVar {
    /// Class base name (e.g., "Damage", "Block", "Cards", "Power", "Dynamic").
    /// "Power" + a generic produce PowerVar<T>; "Dynamic" + a key produce the
    /// generic DynamicVar("key", value) base used for power-on-card with no
    /// dedicated subclass.
    kind: String,
    /// Generic type parameter, e.g. "StrengthPower" for PowerVar<StrengthPower>.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    generic: Option<String>,
    /// String key passed to a keyed var constructor (DynamicVar("Accelerant", 1m)).
    /// The CardModel indexer references the var by this key for upgrade ops.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    base_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    value_prop: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpgradeDelta {
    var_kind: String,
    delta: f64,
}

fn main() -> Result<()> {
    let models_dir = std::env::var("STS2_DECOMPILE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODELS_DIR));
    if !models_dir.exists() {
        bail!(
            "decompile Models dir not found: {} (set STS2_DECOMPILE_DIR)",
            models_dir.display()
        );
    }

    let pools_dir = models_dir.join("CardPools");
    let cards_dir = models_dir.join("Cards");

    // Mock cards (Models/Cards/Mocks/) and MockCardPool are test-only stubs
    // — they exist for the game's internal test harness, not for play. We
    // skip them entirely so the agent's card vocabulary is real-card-only.

    // 1. Pool membership: each (non-mock) CardPool's GenerateAllCards() lists
    //    card class names via `ModelDb.Card<XYZ>()`.
    let pool_membership = parse_pools(&pools_dir)?;

    // 2. Build reverse map card_id -> pool_name. Every card must be in exactly
    //    one pool.
    let mut card_to_pool: BTreeMap<String, String> = BTreeMap::new();
    for (pool, ids) in &pool_membership {
        for id in ids {
            if let Some(prev) = card_to_pool.insert(id.clone(), pool.clone()) {
                bail!(
                    "card {} is in two pools: {} and {}",
                    id, prev, pool
                );
            }
        }
    }

    // 3. Locate each card file in Cards/<id>.cs.
    let mut cards: Vec<CardData> = Vec::new();
    for (id, pool) in &card_to_pool {
        let path = cards_dir.join(format!("{id}.cs"));
        if !path.exists() {
            bail!("card source file not found for id {}", id);
        }
        let source = fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let card = parse_card(id, pool, &source)
            .with_context(|| format!("parsing {} ({})", id, path.display()))?;
        cards.push(card);
    }

    // 4. Sanity: every concrete CardModel subclass on disk (excluding mocks)
    //    should have made it into the table. Otherwise we silently lost one.
    let on_disk = list_card_ids_on_disk(&cards_dir)?;
    for id in &on_disk {
        if !card_to_pool.contains_key(id) {
            bail!(
                "card {} exists on disk but is not in any CardPool's GenerateAllCards()",
                id
            );
        }
    }

    cards.sort_by(|a, b| a.id.cmp(&b.id));

    // 5. Write to crates/sts2-sim/data/cards.json. Run from the workspace
    //    root (which is where `cargo run -p extract-cards` lands).
    let workspace_root = workspace_root()?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&cards)?;
    fs::write(&output, json)?;
    eprintln!(
        "wrote {} cards to {}",
        cards.len(),
        output.display()
    );
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    // Cargo runs `cargo run -p extract-cards` from the workspace root, so
    // CARGO_MANIFEST_DIR points at this crate; walk up two levels to reach
    // the workspace root (sim/).
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| anyhow!("could not derive workspace root from {}", manifest.display()))?;
    Ok(workspace.to_path_buf())
}

fn parse_pools(pools_dir: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    let card_ref = Regex::new(r"ModelDb\.Card<([A-Za-z_][A-Za-z0-9_]*)>\(\)")?;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in fs::read_dir(pools_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("cs") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("bad path {}", path.display()))?;
        // File names are like "IroncladCardPool.cs". Strip the suffix.
        let pool_name = stem
            .strip_suffix("CardPool")
            .ok_or_else(|| anyhow!("unexpected pool file: {}", stem))?;
        // Mock pool is test-only.
        if pool_name == "Mock" {
            continue;
        }
        let source = fs::read_to_string(&path)?;
        // Only count refs inside the GenerateAllCards method body. The file
        // may have other references (epoch filters, etc.) but they live in
        // FilterThroughEpochs and don't establish membership.
        let body = extract_method_body(&source, "GenerateAllCards")
            .ok_or_else(|| anyhow!("no GenerateAllCards body in {}", path.display()))?;
        let mut ids: Vec<String> = card_ref
            .captures_iter(&body)
            .map(|c| c[1].to_string())
            .collect();
        // Some pools (CurseCardPool) emit cards via helper enumerations
        // outside GenerateAllCards. We rely on GenerateAllCards being the
        // canonical list; cross-checked later against the on-disk inventory.
        ids.sort();
        ids.dedup();
        out.insert(pool_name.to_string(), ids);
    }
    Ok(out)
}

/// Returns the text between the matching outer braces of the named method.
/// Method signature can span multiple lines; we look for `<name>(` then walk
/// to the first `{` and brace-match.
fn extract_method_body(source: &str, name: &str) -> Option<String> {
    // Find `<name>(` not preceded by a word character (so we don't match
    // suffixed names).
    let needle = format!("{name}(");
    let mut search = 0;
    while let Some(pos) = source[search..].find(&needle) {
        let abs = search + pos;
        // Require previous char to not be a word char (rough boundary check).
        let prev = source[..abs].chars().rev().next();
        if matches!(prev, Some(c) if c.is_ascii_alphanumeric() || c == '_') {
            search = abs + needle.len();
            continue;
        }
        // Walk forward to find the opening `{` of the method body.
        let after = &source[abs + needle.len()..];
        let open = after.find('{')?;
        let body_start = abs + needle.len() + open + 1;
        // Brace-match from body_start.
        let mut depth: i32 = 1;
        let bytes = source.as_bytes();
        let mut i = body_start;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if depth != 0 {
            return None;
        }
        return Some(source[body_start..i - 1].to_string());
    }
    None
}

/// Returns the body of an expression-bodied or block-bodied property/method
/// recognized by `<name>\s*{?\s*get\s*{` or `<name>\s*=>`. Used for property
/// overrides like `MaxUpgradeLevel`, `HasEnergyCostX`, `CanonicalVars`,
/// `CanonicalTags`.
fn extract_property_body(source: &str, name: &str) -> Option<String> {
    // Look for a line that contains the property name followed by `{` or `=>`.
    // We use a narrow heuristic: find `<name>` after a type keyword.
    // Pattern 1: `... <name>\s*{` (auto-property or block body)
    // Pattern 2: `... <name>\s*=>` (expression body)
    let rx = Regex::new(&format!(
        r"(?m)\b{}\s*(\{{|=>)",
        regex::escape(name)
    ))
    .ok()?;
    let m = rx.find(source)?;
    let after = &source[m.end()..];
    // For expression-bodied, the body is until the next `;` at depth 0
    // (parenthesis depth, since braces don't appear in expression bodies).
    if m.as_str().ends_with("=>") {
        let mut depth: i32 = 0;
        let bytes = after.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => depth -= 1,
                b';' if depth == 0 => return Some(after[..i].to_string()),
                _ => {}
            }
            i += 1;
        }
        return None;
    }
    // Block body: find the first `get` block. Walk forward to the next `{`
    // and brace-match.
    let get_idx = after.find("get")?;
    let after_get = &after[get_idx + 3..];
    let open = after_get.find('{')?;
    let body_start_rel = get_idx + 3 + open + 1;
    let mut depth: i32 = 1;
    let bytes = after.as_bytes();
    let mut i = body_start_rel;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    if depth != 0 {
        return None;
    }
    Some(after[body_start_rel..i - 1].to_string())
}

fn parse_card(id: &str, pool: &str, source: &str) -> Result<CardData> {
    // Ctor: `: base(N, CardType.X, CardRarity.Y, TargetType.Z, BOOL)`.
    // The bool arg defaults to true if omitted in base ctor, but the
    // concrete card classes always pass it explicitly in the samples.
    let ctor_rx = Regex::new(
        r":\s*base\s*\(\s*(-?\d+)\s*,\s*CardType\.(\w+)\s*,\s*CardRarity\.(\w+)\s*,\s*TargetType\.(\w+)\s*(?:,\s*(true|false))?\s*\)",
    )?;
    let ctor = ctor_rx
        .captures(source)
        .ok_or_else(|| anyhow!("no `: base(...)` ctor match for {}", id))?;
    let energy_cost: i32 = ctor[1].parse()?;
    let card_type = ctor[2].to_string();
    let rarity = ctor[3].to_string();
    let target_type = ctor[4].to_string();
    let show_in_library: bool = ctor
        .get(5)
        .map(|m| m.as_str() == "true")
        .unwrap_or(true);

    // MaxUpgradeLevel override (default 1).
    let max_upgrade_level = parse_int_property(source, "MaxUpgradeLevel").unwrap_or(1);

    // HasEnergyCostX override (default false).
    let has_energy_cost_x =
        parse_bool_property(source, "HasEnergyCostX").unwrap_or(false);

    // CanonicalTags: optional. Pattern: `new HashSet<CardTag> { CardTag.X, ... }`.
    let tags = parse_canonical_tags(source);

    // CanonicalVars: optional. Pattern: single-element list or array of
    // DynamicVar subclass constructors.
    let canonical_vars = parse_canonical_vars(source);

    // OnUpgrade body: each `base.DynamicVars.X.UpgradeValueBy(Nm)` is a delta,
    // plus the energy-cost delta if `base.EnergyCost.UpgradeBy(N)` appears.
    let (upgrade_deltas, energy_cost_upgrade_delta) = parse_upgrade_body(source);

    Ok(CardData {
        id: id.to_string(),
        pool: pool.to_string(),
        energy_cost,
        card_type,
        rarity,
        target_type,
        show_in_library,
        has_energy_cost_x,
        max_upgrade_level,
        energy_cost_upgrade_delta,
        tags,
        canonical_vars,
        upgrade_deltas,
    })
}

fn parse_int_property(source: &str, name: &str) -> Option<i32> {
    let body = extract_property_body(source, name)?;
    let rx = Regex::new(r"return\s+(-?\d+)\s*;").ok()?;
    rx.captures(&body).and_then(|c| c[1].parse().ok())
}

fn parse_bool_property(source: &str, name: &str) -> Option<bool> {
    let body = extract_property_body(source, name)?;
    if body.contains("return true") {
        Some(true)
    } else if body.contains("return false") {
        Some(false)
    } else {
        None
    }
}

fn parse_canonical_tags(source: &str) -> Vec<String> {
    let Some(body) = extract_property_body(source, "CanonicalTags") else {
        return Vec::new();
    };
    let rx = Regex::new(r"CardTag\.(\w+)").unwrap();
    let mut tags: Vec<String> = rx
        .captures_iter(&body)
        .map(|c| c[1].to_string())
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

/// Matches every `new XyzVar(<...>)?(...)` constructor in the CanonicalVars
/// body. Handles the two arg shapes we've seen in the decompile:
///   - keyed:  `new DynamicVar("Key", 1m)`
///   - bare:   `new DamageVar(5m, ValueProp.Move)` / `new CardsVar(3)`
/// The numeric literal's `m` suffix (C# decimal) is optional — some bare-int
/// constructors omit it (e.g., `CardsVar(3)`).
fn parse_canonical_vars(source: &str) -> Vec<CardVar> {
    let Some(body) = extract_property_body(source, "CanonicalVars") else {
        return Vec::new();
    };
    let rx = Regex::new(
        r#"new\s+(\w+)Var(?:<(\w+)>)?\s*\(\s*(?:"(\w+)"\s*,\s*(-?\d+(?:\.\d+)?)m?|(-?\d+(?:\.\d+)?)m?)?\s*(?:,\s*ValueProp\.(\w+))?"#,
    )
    .unwrap();
    rx.captures_iter(&body)
        .map(|c| {
            let kind = c[1].to_string();
            let generic = c.get(2).map(|m| m.as_str().to_string());
            let key = c.get(3).map(|m| m.as_str().to_string());
            let base_value: Option<f64> = c
                .get(4)
                .or_else(|| c.get(5))
                .and_then(|m| m.as_str().parse().ok());
            let value_prop = c.get(6).map(|m| m.as_str().to_string());
            CardVar {
                kind,
                generic,
                key,
                base_value,
                value_prop,
            }
        })
        .collect()
}

fn parse_upgrade_body(source: &str) -> (Vec<UpgradeDelta>, i32) {
    let Some(body) = extract_method_body(source, "OnUpgrade") else {
        return (Vec::new(), 0);
    };
    // DynamicVars patterns:
    //   1. base.DynamicVars.<VarName>.UpgradeValueBy(Nm)
    //   2. base.DynamicVars["<Key>"].UpgradeValueBy(Nm)
    let dot_rx = Regex::new(
        r#"base\.DynamicVars\.(\w+)\.UpgradeValueBy\(\s*(-?\d+(?:\.\d+)?)m?\s*\)"#,
    )
    .unwrap();
    let idx_rx = Regex::new(
        r#"base\.DynamicVars\["(\w+)"\]\.UpgradeValueBy\(\s*(-?\d+(?:\.\d+)?)m?\s*\)"#,
    )
    .unwrap();
    let energy_rx =
        Regex::new(r"base\.EnergyCost\.UpgradeBy\(\s*(-?\d+)\s*\)").unwrap();

    let mut deltas: Vec<UpgradeDelta> = Vec::new();
    for c in dot_rx.captures_iter(&body) {
        deltas.push(UpgradeDelta {
            var_kind: c[1].to_string(),
            delta: c[2].parse().unwrap_or(0.0),
        });
    }
    for c in idx_rx.captures_iter(&body) {
        deltas.push(UpgradeDelta {
            var_kind: c[1].to_string(),
            delta: c[2].parse().unwrap_or(0.0),
        });
    }
    let energy_delta: i32 = energy_rx
        .captures(&body)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(0);
    (deltas, energy_delta)
}

fn list_card_ids_on_disk(cards_dir: &Path) -> Result<Vec<String>> {
    // Mocks/ subdirectory holds test stubs; we exclude them.
    let mut out: Vec<String> = Vec::new();
    for entry in fs::read_dir(cards_dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("cs") {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                out.push(stem.to_string());
            }
        }
    }
    out.sort();
    Ok(out)
}
