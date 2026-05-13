//! Enchantment data extractor.
//!
//! 23 concrete EnchantmentModel subclasses (Mocks/ skipped). Captures
//! per-enchantment flags + which CardType values appear in any
//! `CanEnchantCardType` override (the agent's combat observation needs to
//! know which cards an enchantment could attach to).

use anyhow::{Context, Result, bail};
use extractors_common::{
    DynamicVarSpec, extract_method_body, extract_property_body, parse_canonical_vars,
    workspace_root_from_manifest,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const DEFAULT_MODELS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sts2_decompiled\sts2\MegaCrit\sts2\Core\Models";

const OUTPUT_REL: &str = r"crates\sts2-sim\data\enchantments.json";

#[derive(Debug, Serialize, Deserialize)]
struct EnchantmentData {
    id: String,
    #[serde(default)]
    has_extra_card_text: bool,
    #[serde(default)]
    show_amount: bool,
    /// CardType values referenced in the CanEnchantCardType override. Empty
    /// = method not overridden (= applies to any card per the base default).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    applicable_card_types: Vec<String>,
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
    let dir = models_dir.join("Enchantments");

    let mut out: Vec<EnchantmentData> = Vec::new();
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
        if !source.contains(": EnchantmentModel") {
            continue;
        }
        out.push(EnchantmentData {
            id: stem.to_string(),
            has_extra_card_text: parse_bool(&source, "HasExtraCardText").unwrap_or(false),
            show_amount: parse_bool(&source, "ShowAmount").unwrap_or(false),
            applicable_card_types: parse_applicable_types(&source),
            canonical_vars: parse_canonical_vars(&source),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));

    let workspace_root = workspace_root_from_manifest(env!("CARGO_MANIFEST_DIR"))?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_string_pretty(&out)?)?;
    eprintln!("wrote {} enchantments to {}", out.len(), output.display());
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

fn parse_applicable_types(source: &str) -> Vec<String> {
    let Some(body) = extract_method_body(source, "CanEnchantCardType") else {
        return Vec::new();
    };
    let rx = Regex::new(r"CardType\.(\w+)").unwrap();
    let mut v: Vec<String> = rx
        .captures_iter(&body)
        .map(|c| c[1].to_string())
        .collect();
    v.sort();
    v.dedup();
    v
}
