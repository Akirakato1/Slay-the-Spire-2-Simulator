//! Monster data extractor.
//!
//! Captures id, base + ascension-adjusted HP ranges. Behavior (intent
//! selection, AI move state machines) is deferred.
//!
//! HP overrides use one of two patterns:
//!   1. `return AscensionHelper.GetValueIfAscension(Level, ASCENDED, BASE);`
//!      — record both, so the agent's combat observation has ascension data.
//!   2. `return N;` — direct literal; record as base, ascended is None.

use anyhow::{Context, Result, bail};
use extractors_common::{extract_property_body, workspace_root_from_manifest};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const DEFAULT_MODELS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sts2_decompiled\sts2\MegaCrit\sts2\Core\Models";

const OUTPUT_REL: &str = r"crates\sts2-sim\data\monsters.json";

#[derive(Debug, Serialize, Deserialize)]
struct MonsterData {
    id: String,
    /// Base (A0) minimum initial HP roll. None if the property wasn't
    /// found in a parseable form.
    min_hp_base: Option<i32>,
    /// Base maximum initial HP roll.
    max_hp_base: Option<i32>,
    /// HP minimums under the ToughEnemies ascension (when applicable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_hp_ascended: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_hp_ascended: Option<i32>,
}

fn main() -> Result<()> {
    let models_dir = std::env::var("STS2_DECOMPILE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODELS_DIR));
    if !models_dir.exists() {
        bail!("decompile Models dir not found: {}", models_dir.display());
    }
    let dir = models_dir.join("Monsters");

    // Two-pass scan: first collect all monster files and their parent
    // class. A file is a monster if its `: ParentClass` chain reaches
    // `MonsterModel` (directly or via an intermediate abstract base
    // like `DecimillipedeSegment` or `FlailKnight`).
    let parent_rx = Regex::new(r"class\s+(\w+)\s*:\s*(\w+)").unwrap();
    let mut files: Vec<(String, PathBuf, String, String)> = Vec::new();
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
        let Some(c) = parent_rx.captures(&source) else {
            continue;
        };
        let class_name = c[1].to_string();
        if class_name != stem {
            continue;
        }
        let parent = c[2].to_string();
        files.push((class_name, p.clone(), source, parent));
    }
    // is_monster(name) = parent chain reaches MonsterModel
    use std::collections::HashMap;
    let parent_of: HashMap<String, String> =
        files.iter().map(|(n, _, _, p)| (n.clone(), p.clone())).collect();
    let is_monster = |start: &str| -> bool {
        let mut cur = start.to_string();
        for _ in 0..16 {
            if cur == "MonsterModel" {
                return true;
            }
            match parent_of.get(&cur) {
                Some(p) => cur = p.clone(),
                None => return false,
            }
        }
        false
    };

    let mut out: Vec<MonsterData> = Vec::new();
    for (name, _path, source, _parent) in &files {
        if !is_monster(name) {
            continue;
        }
        // Walk up the chain so missing properties on a subclass fall
        // through to the parent (DecimillipedeSegmentFront etc.
        // inherit HP from DecimillipedeSegment).
        let mut min_base = None;
        let mut min_asc = None;
        let mut max_base = None;
        let mut max_asc = None;
        let mut cur_source = source.clone();
        let mut cur_name = name.clone();
        for _ in 0..16 {
            if min_base.is_none() {
                let (b, a) = parse_hp(&cur_source, "MinInitialHp");
                min_base = b;
                min_asc = a;
            }
            if max_base.is_none() {
                let (b, a) = parse_hp(&cur_source, "MaxInitialHp");
                max_base = b;
                max_asc = a;
                if max_base.is_none()
                    && extract_property_body(&cur_source, "MaxInitialHp")
                        .map(|b| b.contains("this.MinInitialHp"))
                        .unwrap_or(false)
                {
                    max_base = min_base;
                    max_asc = min_asc;
                }
            }
            if min_base.is_some() && max_base.is_some() {
                break;
            }
            // Walk up.
            let parent = match parent_of.get(&cur_name) {
                Some(p) => p.clone(),
                None => break,
            };
            if parent == "MonsterModel" {
                break;
            }
            let parent_file = files.iter().find(|(n, _, _, _)| n == &parent);
            match parent_file {
                Some((_, _, src, _)) => {
                    cur_source = src.clone();
                    cur_name = parent;
                }
                None => break,
            }
        }
        out.push(MonsterData {
            id: name.clone(),
            min_hp_base: min_base,
            max_hp_base: max_base,
            min_hp_ascended: min_asc,
            max_hp_ascended: max_asc,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));

    let workspace_root = workspace_root_from_manifest(env!("CARGO_MANIFEST_DIR"))?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_string_pretty(&out)?)?;
    eprintln!("wrote {} monsters to {}", out.len(), output.display());
    Ok(())
}

/// Returns (base, ascended) HP. The C# uses:
///   `AscensionHelper.GetValueIfAscension(Level, ASCENDED, BASE)` — third
///   arg is the A0 value; second is the boosted value used when the named
///   ascension level (e.g., ToughEnemies, the +HP modifier) is met.
fn parse_hp(source: &str, prop: &str) -> (Option<i32>, Option<i32>) {
    let Some(body) = extract_property_body(source, prop) else {
        return (None, None);
    };
    let asc_rx = Regex::new(
        r"AscensionHelper\.GetValueIfAscension\(\s*\w+\.\w+\s*,\s*(-?\d+)\s*,\s*(-?\d+)\s*\)",
    )
    .unwrap();
    if let Some(c) = asc_rx.captures(&body) {
        return (
            c[2].parse().ok(),
            c[1].parse().ok(),
        );
    }
    let lit_rx = Regex::new(r"return\s+(-?\d+)\s*;").unwrap();
    if let Some(c) = lit_rx.captures(&body) {
        return (c[1].parse().ok(), None);
    }
    (None, None)
}
