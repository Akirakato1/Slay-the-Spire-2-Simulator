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
use extractors_common::{
    DynamicVarSpec, extract_method_body, extract_property_body, parse_canonical_vars,
    workspace_root_from_manifest,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_MODELS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sts2_decompiled\sts2\MegaCrit\sts2\Core\Models";

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
    /// `CanonicalKeywords` set. Names like "Exhaust", "Innate", "Ethereal",
    /// "Retain", "EndTurn", "Volatile". Routing keywords (Exhaust) drive
    /// hand→pile placement in play_card; others (Innate/Retain/Ethereal)
    /// gate draw/discard timing.
    #[serde(default)]
    keywords: Vec<String>,
    canonical_vars: Vec<DynamicVarSpec>,
    upgrade_deltas: Vec<UpgradeDelta>,
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
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

    // 2. Build reverse map card_id -> pool_name. Every card must be in
    //    exactly one pool.
    let mut card_to_pool: BTreeMap<String, String> = BTreeMap::new();
    for (pool, ids) in &pool_membership {
        for id in ids {
            if let Some(prev) = card_to_pool.insert(id.clone(), pool.clone()) {
                bail!("card {} is in two pools: {} and {}", id, prev, pool);
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
    //    should have made it into the table.
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

    let workspace_root = workspace_root_from_manifest(env!("CARGO_MANIFEST_DIR"))?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&cards)?;
    fs::write(&output, json)?;
    eprintln!("wrote {} cards to {}", cards.len(), output.display());
    Ok(())
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
        let pool_name = stem
            .strip_suffix("CardPool")
            .ok_or_else(|| anyhow!("unexpected pool file: {}", stem))?;
        // Mock pool is test-only.
        if pool_name == "Mock" {
            continue;
        }
        let source = fs::read_to_string(&path)?;
        let body = extract_method_body(&source, "GenerateAllCards")
            .ok_or_else(|| anyhow!("no GenerateAllCards body in {}", path.display()))?;
        let mut ids: Vec<String> = card_ref
            .captures_iter(&body)
            .map(|c| c[1].to_string())
            .collect();
        ids.sort();
        ids.dedup();
        out.insert(pool_name.to_string(), ids);
    }
    Ok(out)
}

fn parse_card(id: &str, pool: &str, source: &str) -> Result<CardData> {
    // Ctor: `: base(N, CardType.X, CardRarity.Y, TargetType.Z, BOOL)`.
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

    let max_upgrade_level = parse_int_property(source, "MaxUpgradeLevel").unwrap_or(1);
    let has_energy_cost_x =
        parse_bool_property(source, "HasEnergyCostX").unwrap_or(false);
    let tags = parse_canonical_tags(source);
    let keywords = parse_canonical_keywords(source);
    let canonical_vars = parse_canonical_vars(source);
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
        keywords,
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

fn parse_canonical_keywords(source: &str) -> Vec<String> {
    let Some(body) = extract_property_body(source, "CanonicalKeywords") else {
        return Vec::new();
    };
    let rx = Regex::new(r"CardKeyword\.(\w+)").unwrap();
    let mut keywords: Vec<String> = rx
        .captures_iter(&body)
        .map(|c| c[1].to_string())
        .collect();
    keywords.sort();
    keywords.dedup();
    keywords
}

fn parse_upgrade_body(source: &str) -> (Vec<UpgradeDelta>, i32) {
    let Some(body) = extract_method_body(source, "OnUpgrade") else {
        return (Vec::new(), 0);
    };
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
