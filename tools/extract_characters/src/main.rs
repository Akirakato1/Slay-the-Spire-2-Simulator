//! Character data extractor.
//!
//! Captures the playable-character constants: starting HP, starting gold,
//! starting deck composition (card class names with multiplicity preserved
//! as order), starting relics, and the names of the character's card /
//! potion / relic pools.
//!
//! These feed both Phase 1 observation (player state setup) and the
//! reward / deck-generation infrastructure of Phase 2.

use anyhow::{Context, Result, anyhow, bail};
use extractors_common::{extract_property_body, workspace_root_from_manifest};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const DEFAULT_MODELS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sts2_decompiled\sts2\MegaCrit\sts2\Core\Models";

const OUTPUT_REL: &str = r"crates\sts2-sim\data\characters.json";

#[derive(Debug, Serialize, Deserialize)]
struct CharacterData {
    id: String,
    starting_hp: Option<i32>,
    starting_gold: Option<i32>,
    /// Card pool name without the "CardPool" suffix (e.g., "Ironclad").
    card_pool: Option<String>,
    potion_pool: Option<String>,
    relic_pool: Option<String>,
    /// Starting deck as a list of card class names. Multiplicity is the
    /// number of times an id repeats (e.g., 5x StrikeIronclad).
    #[serde(default)]
    starting_deck: Vec<String>,
    #[serde(default)]
    starting_relics: Vec<String>,
}

fn main() -> Result<()> {
    let models_dir = std::env::var("STS2_DECOMPILE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODELS_DIR));
    if !models_dir.exists() {
        bail!("decompile Models dir not found: {}", models_dir.display());
    }
    let dir = models_dir.join("Characters");

    let mut out: Vec<CharacterData> = Vec::new();
    for entry in fs::read_dir(&dir)? {
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
        if !source.contains(": CharacterModel") {
            continue;
        }
        out.push(parse_character(stem, &source)?);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));

    let workspace_root = workspace_root_from_manifest(env!("CARGO_MANIFEST_DIR"))?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_string_pretty(&out)?)?;
    eprintln!("wrote {} characters to {}", out.len(), output.display());
    Ok(())
}

fn parse_character(id: &str, source: &str) -> Result<CharacterData> {
    Ok(CharacterData {
        id: id.to_string(),
        starting_hp: parse_int_property(source, "StartingHp"),
        starting_gold: parse_int_property(source, "StartingGold"),
        card_pool: parse_pool_property(source, "CardPool", "CardPool"),
        potion_pool: parse_pool_property(source, "PotionPool", "PotionPool"),
        relic_pool: parse_pool_property(source, "RelicPool", "RelicPool"),
        starting_deck: parse_id_list(source, "StartingDeck", r"ModelDb\.Card<(\w+)>\(\)")?,
        starting_relics: parse_id_list(
            source,
            "StartingRelics",
            r"ModelDb\.Relic<(\w+)>\(\)",
        )?,
    })
}

fn parse_int_property(source: &str, name: &str) -> Option<i32> {
    let body = extract_property_body(source, name)?;
    let rx = Regex::new(r"return\s+(-?\d+)\s*;").ok()?;
    rx.captures(&body).and_then(|c| c[1].parse().ok())
}

/// Reads `return ModelDb.CardPool<XYZ>();` style accessor; strips the
/// `suffix` so we get just the canonical pool name.
fn parse_pool_property(source: &str, prop: &str, suffix: &str) -> Option<String> {
    let body = extract_property_body(source, prop)?;
    let rx = Regex::new(&format!(
        r"ModelDb\.{}\s*<\s*(\w+)\s*>\s*\(\s*\)",
        regex::escape(suffix)
    ))
    .ok()?;
    let cap = rx.captures(&body)?;
    Some(cap[1].strip_suffix(suffix).unwrap_or(&cap[1]).to_string())
}

fn parse_id_list(source: &str, prop: &str, ref_pattern: &str) -> Result<Vec<String>> {
    let Some(body) = extract_property_body(source, prop) else {
        return Ok(Vec::new());
    };
    let rx = Regex::new(ref_pattern)?;
    Ok(rx
        .captures_iter(&body)
        .map(|c| c[1].to_string())
        .collect())
}
