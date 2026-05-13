//! Modifier (run-mode / Neow option) data extractor.
//!
//! Modifiers are heavy on behavior — their effects implement specific
//! virtuals (TryModifyRewardsLate, AfterCardChangedPiles, ...). Pure data is
//! sparse: just the id and the `ClearsPlayerDeck` flag.

use anyhow::{Context, Result, bail};
use extractors_common::{extract_property_body, workspace_root_from_manifest};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const DEFAULT_MODELS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sts2_decompiled\sts2\MegaCrit\sts2\Core\Models";

const OUTPUT_REL: &str = r"crates\sts2-sim\data\modifiers.json";

#[derive(Debug, Serialize, Deserialize)]
struct ModifierData {
    id: String,
    #[serde(default)]
    clears_player_deck: bool,
}

fn main() -> Result<()> {
    let models_dir = std::env::var("STS2_DECOMPILE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODELS_DIR));
    if !models_dir.exists() {
        bail!("decompile Models dir not found: {}", models_dir.display());
    }
    let dir = models_dir.join("Modifiers");

    let mut out: Vec<ModifierData> = Vec::new();
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
        if !source.contains(": ModifierModel") {
            continue;
        }
        out.push(ModifierData {
            id: stem.to_string(),
            clears_player_deck: parse_bool(&source, "ClearsPlayerDeck").unwrap_or(false),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));

    let workspace_root = workspace_root_from_manifest(env!("CARGO_MANIFEST_DIR"))?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_string_pretty(&out)?)?;
    eprintln!("wrote {} modifiers to {}", out.len(), output.display());
    Ok(())
}

fn parse_bool(source: &str, name: &str) -> Option<bool> {
    let body = extract_property_body(source, name)?;
    if body.contains("return true") {
        Some(true)
    } else if body.contains("return false") {
        Some(false)
    } else {
        None
    }
}
