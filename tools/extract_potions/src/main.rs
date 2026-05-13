//! Potion data extractor.
//!
//! 64 PotionModel subclasses. Pool membership is split between
//! `Models/PotionPools/*.cs` (direct refs) and `Core/Timeline/Epochs/*.cs`
//! (character pools delegate to Epoch.Potions lists). We scan both
//! directories to assemble pool→potion membership.
//!
//! Per-potion data: rarity (Common/Uncommon/Rare/Event/Token), usage
//! (CombatOnly/AnyTime/Automatic), target_type, canonical_vars.

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

const EPOCHS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sts2_decompiled\sts2\MegaCrit\sts2\Core\Timeline\Epochs";

const OUTPUT_REL: &str = r"crates\sts2-sim\data\potions.json";

#[derive(Debug, Serialize, Deserialize)]
struct PotionData {
    id: String,
    /// Pool(s) and/or Epoch(s) that list this potion. Pool names are stripped
    /// of the "PotionPool" suffix; Epoch names keep their full name.
    pools: Vec<String>,
    rarity: String,
    usage: String,
    target_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    canonical_vars: Vec<DynamicVarSpec>,
}

fn main() -> Result<()> {
    let models_dir = std::env::var("STS2_DECOMPILE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODELS_DIR));
    if !models_dir.exists() {
        bail!("decompile Models dir not found: {}", models_dir.display());
    }
    let potions_dir = models_dir.join("Potions");
    let pools_dir = models_dir.join("PotionPools");
    let epochs_dir = PathBuf::from(EPOCHS_DIR);

    // Build pool/epoch -> [potion_ids] reverse map by scanning both dirs for
    // ModelDb.Potion<X>() refs.
    let mut potion_to_pools: BTreeMap<String, Vec<String>> = BTreeMap::new();
    scan_pool_refs(&pools_dir, "PotionPool", &mut potion_to_pools)?;
    if epochs_dir.exists() {
        scan_pool_refs(&epochs_dir, "Epoch", &mut potion_to_pools)?;
    }
    for (_id, v) in potion_to_pools.iter_mut() {
        v.sort();
        v.dedup();
    }

    let mut out: Vec<PotionData> = Vec::new();
    for entry in fs::read_dir(&potions_dir)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|s| s.to_str()) != Some("cs") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let source = fs::read_to_string(&p)
            .with_context(|| format!("reading {}", p.display()))?;
        if !source.contains(": PotionModel") {
            continue;
        }
        let rarity = parse_enum_property(&source, "Rarity", "PotionRarity")
            .ok_or_else(|| anyhow!("no Rarity override in {}", stem))?;
        let usage = parse_enum_property(&source, "Usage", "PotionUsage")
            .ok_or_else(|| anyhow!("no Usage override in {}", stem))?;
        let target_type = parse_enum_property(&source, "TargetType", "TargetType");
        let canonical_vars = parse_canonical_vars(&source);
        let pools = potion_to_pools.get(stem).cloned().unwrap_or_default();
        out.push(PotionData {
            id: stem.to_string(),
            pools,
            rarity,
            usage,
            target_type,
            canonical_vars,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));

    let workspace_root = workspace_root_from_manifest(env!("CARGO_MANIFEST_DIR"))?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_string_pretty(&out)?)?;
    eprintln!("wrote {} potions to {}", out.len(), output.display());
    Ok(())
}

fn scan_pool_refs(
    dir: &Path,
    name_suffix: &str,
    out: &mut BTreeMap<String, Vec<String>>,
) -> Result<()> {
    let potion_ref = Regex::new(r"ModelDb\.Potion<(\w+)>\(\)")?;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            // Recurse one level into Epochs (e.g., character/character subdirs).
            scan_pool_refs(&p, name_suffix, out)?;
            continue;
        }
        if p.extension().and_then(|s| s.to_str()) != Some("cs") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let pool_name = stem.strip_suffix(name_suffix).unwrap_or(stem);
        let source = fs::read_to_string(&p)?;
        for c in potion_ref.captures_iter(&source) {
            let potion = c[1].to_string();
            let entry = out.entry(potion).or_default();
            if !entry.iter().any(|x| x == pool_name) {
                entry.push(pool_name.to_string());
            }
        }
    }
    Ok(())
}

fn parse_enum_property(source: &str, prop: &str, enum_name: &str) -> Option<String> {
    let body = extract_property_body(source, prop)?;
    let rx = Regex::new(&format!(r"return\s+{}\.(\w+)\s*;", regex::escape(enum_name))).ok()?;
    rx.captures(&body).map(|c| c[1].to_string())
}

// Reserve method-body extractor for future use (e.g., parsing OnUse).
#[allow(dead_code)]
fn _silence(s: &str) -> Option<String> {
    extract_method_body(s, "_")
}
