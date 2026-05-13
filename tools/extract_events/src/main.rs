//! Event data extractor.
//!
//! Captures id, canonical magic numbers, and the localization keys of the
//! initial choice options from each Event's `GenerateInitialOptions()`.
//! Option-outcome behavior (Immerse/Abstain/etc.) is deferred — the data
//! port records arity + labels so the agent's strategic head can index
//! choices even before behavior is implemented.

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

const OUTPUT_REL: &str = r"crates\sts2-sim\data\events.json";

#[derive(Debug, Serialize, Deserialize)]
struct EventData {
    id: String,
    #[serde(default)]
    is_shared: bool,
    /// Localization keys of the initial-page event options. Captures the
    /// option labels for use as feature ids in the agent's observation,
    /// even though the option outcomes aren't ported yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    initial_option_labels: Vec<String>,
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
    let dir = models_dir.join("Events");

    let mut out: Vec<EventData> = Vec::new();
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
        if !source.contains(": EventModel") {
            continue;
        }
        out.push(EventData {
            id: stem.to_string(),
            is_shared: parse_bool(&source, "IsShared").unwrap_or(false),
            initial_option_labels: parse_initial_options(&source),
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
    eprintln!("wrote {} events to {}", out.len(), output.display());
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

fn parse_initial_options(source: &str) -> Vec<String> {
    let Some(body) = extract_method_body(source, "GenerateInitialOptions") else {
        return Vec::new();
    };
    // EventOption ctor: `new EventOption(this, <func>, "LABEL.LOC.KEY", ...)`.
    // Match the third arg (the string literal label). Labels mix upper/lower
    // case ("ABYSSAL_BATHS.pages.INITIAL.options.IMMERSE").
    let rx = Regex::new(r#"new\s+EventOption\([^,]*,[^,]*,\s*"([A-Za-z0-9_\.]+)""#).unwrap();
    rx.captures_iter(&body)
        .map(|c| c[1].to_string())
        .collect()
}
