//! Orb data extractor. Walks `Models/Orbs/` (top level only; skips `Mock/`)
//! and emits `crates/sts2-sim/data/orbs.json`.
//!
//! Captured per orb:
//!   - `PassiveVal` and `EvokeVal` overrides — both wrap a numeric literal
//!     in `base.ModifyOrbValue(Nm)`, the canonical pre-modifier value.
//!
//! Behavior (`Passive`, `Evoke`, `BeforeTurnEndOrbTrigger`) is deferred.

use anyhow::{Context, Result, anyhow, bail};
use extractors_common::{extract_property_body, workspace_root_from_manifest};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_MODELS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sts2_decompiled\sts2\MegaCrit\sts2\Core\Models";

const OUTPUT_REL: &str = r"crates\sts2-sim\data\orbs.json";

#[derive(Debug, Serialize, Deserialize)]
struct OrbData {
    id: String,
    passive_val: Option<f64>,
    evoke_val: Option<f64>,
}

fn main() -> Result<()> {
    let models_dir = std::env::var("STS2_DECOMPILE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODELS_DIR));
    if !models_dir.exists() {
        bail!("decompile Models dir not found: {}", models_dir.display());
    }
    let orbs_dir = models_dir.join("Orbs");

    let mut orbs: Vec<OrbData> = Vec::new();
    for entry in fs::read_dir(&orbs_dir)? {
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
        if !source.contains(": OrbModel") {
            continue;
        }
        orbs.push(parse_orb(stem, &source)?);
    }
    orbs.sort_by(|a, b| a.id.cmp(&b.id));

    let workspace_root = workspace_root_from_manifest(env!("CARGO_MANIFEST_DIR"))?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_string_pretty(&orbs)?)?;
    eprintln!("wrote {} orbs to {}", orbs.len(), output.display());
    Ok(())
}

fn parse_orb(id: &str, source: &str) -> Result<OrbData> {
    Ok(OrbData {
        id: id.to_string(),
        passive_val: parse_orb_value(source, "PassiveVal"),
        evoke_val: parse_orb_value(source, "EvokeVal"),
    })
}

/// Parses the orb's PassiveVal / EvokeVal. Three return shapes seen so far:
///   - direct literal: `return base.ModifyOrbValue(3m);` or `return 3m;`
///   - private field reference: `return this._evokeVal;` — look up the
///     field's initializer (`private decimal _evokeVal = 6m;`) for the
///     starting value, since fields are mutated during play.
///   - computed from another property: GlassOrb's `EvokeVal = PassiveVal * 2`
///     — leave as None; the computation isn't a static value.
fn parse_orb_value(source: &str, prop: &str) -> Option<f64> {
    let body = extract_property_body(source, prop)?;
    let lit_rx = Regex::new(
        r"return\s+(?:base\.ModifyOrbValue\(\s*)?(-?\d+(?:\.\d+)?)m?\s*\)?\s*;",
    )
    .ok()?;
    if let Some(c) = lit_rx.captures(&body) {
        return c[1].parse().ok();
    }
    let field_rx = Regex::new(r"this\.(_\w+)").ok()?;
    if let Some(c) = field_rx.captures(&body) {
        let field = &c[1];
        let init_rx = Regex::new(&format!(
            r"private\s+decimal\s+{}\s*=\s*(-?\d+(?:\.\d+)?)m?",
            regex::escape(field)
        ))
        .ok()?;
        if let Some(ic) = init_rx.captures(source) {
            return ic[1].parse().ok();
        }
    }
    None
}

#[allow(dead_code)]
fn _path_silence(_: &Path) {}
