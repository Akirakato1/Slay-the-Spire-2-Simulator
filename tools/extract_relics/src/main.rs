//! One-shot relic-data extractor.
//!
//! Walks the StS2 decompile, parses each `Models/Relics/*.cs` and
//! `Models/RelicPools/*.cs`, and emits a JSON table to
//! `crates/sts2-sim/data/relics.json`.
//!
//! Relics are simpler than cards:
//!   - No positional base ctor — RelicModel uses a parameterless ctor and
//!     all data is in virtual property overrides.
//!   - `Rarity` is the only mandatory data override.
//!   - `CanonicalVars` (optional) holds magic numbers / counters.
//!   - No `OnUpgrade` — relics don't upgrade.
//!
//! Re-run on game updates: `cargo run -p extract-relics`.

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

const OUTPUT_REL: &str = r"crates\sts2-sim\data\relics.json";

#[derive(Debug, Serialize, Deserialize)]
struct RelicData {
    id: String,
    /// Pools that list this relic in their `GenerateAllRelics()`. Most
    /// relics belong to a single pool; some (LastingCandy, ...) appear in
    /// both Shared and Event sources.
    pools: Vec<String>,
    rarity: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    canonical_vars: Vec<DynamicVarSpec>,
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

    let pools_dir = models_dir.join("RelicPools");
    let relics_dir = models_dir.join("Relics");

    let pool_membership = parse_pools(&pools_dir)?;

    let mut relic_to_pools: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (pool, ids) in &pool_membership {
        for id in ids {
            let entry = relic_to_pools.entry(id.clone()).or_default();
            if !entry.contains(pool) {
                entry.push(pool.clone());
            }
        }
    }
    for (_id, pools) in relic_to_pools.iter_mut() {
        pools.sort();
    }

    let mut relics: Vec<RelicData> = Vec::new();
    for (id, pools) in &relic_to_pools {
        let path = relics_dir.join(format!("{id}.cs"));
        if !path.exists() {
            bail!("relic source file not found for id {}", id);
        }
        let source = fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let relic = parse_relic(id, pools, &source)
            .with_context(|| format!("parsing {} ({})", id, path.display()))?;
        relics.push(relic);
    }

    let on_disk = list_relic_ids_on_disk(&relics_dir)?;
    for id in &on_disk {
        if !relic_to_pools.contains_key(id) {
            bail!(
                "relic {} exists on disk but is not in any RelicPool's GenerateAllRelics()",
                id
            );
        }
    }

    relics.sort_by(|a, b| a.id.cmp(&b.id));

    let workspace_root = workspace_root_from_manifest(env!("CARGO_MANIFEST_DIR"))?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&relics)?;
    fs::write(&output, json)?;
    eprintln!("wrote {} relics to {}", relics.len(), output.display());
    Ok(())
}

fn parse_pools(pools_dir: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    let relic_ref = Regex::new(r"ModelDb\.Relic<([A-Za-z_][A-Za-z0-9_]*)>\(\)")?;
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
            .strip_suffix("RelicPool")
            .ok_or_else(|| anyhow!("unexpected pool file: {}", stem))?;
        let source = fs::read_to_string(&path)?;
        let body = extract_method_body(&source, "GenerateAllRelics")
            .ok_or_else(|| anyhow!("no GenerateAllRelics body in {}", path.display()))?;
        let mut ids: Vec<String> = relic_ref
            .captures_iter(&body)
            .map(|c| c[1].to_string())
            .collect();
        ids.sort();
        ids.dedup();
        out.insert(pool_name.to_string(), ids);
    }
    Ok(out)
}

fn parse_relic(id: &str, pools: &[String], source: &str) -> Result<RelicData> {
    let rarity = parse_rarity(source)
        .ok_or_else(|| anyhow!("no Rarity override in {}", id))?;
    let canonical_vars = parse_canonical_vars(source);
    Ok(RelicData {
        id: id.to_string(),
        pools: pools.to_vec(),
        rarity,
        canonical_vars,
    })
}

fn parse_rarity(source: &str) -> Option<String> {
    let body = extract_property_body(source, "Rarity")?;
    let rx = Regex::new(r"return\s+RelicRarity\.(\w+)\s*;").ok()?;
    rx.captures(&body).map(|c| c[1].to_string())
}

fn list_relic_ids_on_disk(relics_dir: &Path) -> Result<Vec<String>> {
    // Some files in Relics/ are utility classes (e.g., VakuuCardSelector
    // implements ICardSelector, not RelicModel). Filter to only files whose
    // body actually derives from RelicModel.
    let mut out: Vec<String> = Vec::new();
    for entry in fs::read_dir(relics_dir)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|s| s.to_str()) != Some("cs") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let source = fs::read_to_string(&p)?;
        if source.contains(": RelicModel") {
            out.push(stem.to_string());
        }
    }
    out.sort();
    Ok(out)
}
