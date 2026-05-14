//! Encounter data extractor.
//!
//! Captures per-encounter: room_type (Monster/Elite/Boss), slot names, the
//! canonical (Monster, slot) spawn list, and the broader "possible monsters"
//! set. Behavior — randomized variations of who-spawns-where — is deferred.

use anyhow::{Context, Result, bail};
use extractors_common::{extract_method_body, extract_property_body, workspace_root_from_manifest};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const DEFAULT_MODELS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sts2_decompiled\sts2\MegaCrit\sts2\Core\Models";

const OUTPUT_REL: &str = r"crates\sts2-sim\data\encounters.json";

#[derive(Debug, Serialize, Deserialize)]
struct EncounterData {
    id: String,
    room_type: Option<String>,
    #[serde(default)]
    slots: Vec<String>,
    /// Canonical spawn: list of (monster_id, slot) tuples from
    /// `GenerateMonsters()`. Order matches the C# source.
    #[serde(default)]
    canonical_monsters: Vec<MonsterSpawn>,
    /// Broader set of monsters that may appear (from `AllPossibleMonsters`).
    /// Often a superset of the canonical spawn for variation.
    #[serde(default)]
    possible_monsters: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MonsterSpawn {
    monster: String,
    slot: String,
}

fn main() -> Result<()> {
    let models_dir = std::env::var("STS2_DECOMPILE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODELS_DIR));
    if !models_dir.exists() {
        bail!("decompile Models dir not found: {}", models_dir.display());
    }
    let dir = models_dir.join("Encounters");

    let mut out: Vec<EncounterData> = Vec::new();
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
        if !source.contains(": EncounterModel") {
            continue;
        }
        out.push(EncounterData {
            id: stem.to_string(),
            room_type: parse_enum_property(&source, "RoomType", "RoomType"),
            slots: parse_slots(&source),
            canonical_monsters: parse_canonical_monsters(&source),
            possible_monsters: parse_possible_monsters(&source),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));

    let workspace_root = workspace_root_from_manifest(env!("CARGO_MANIFEST_DIR"))?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_string_pretty(&out)?)?;
    eprintln!("wrote {} encounters to {}", out.len(), output.display());
    Ok(())
}

fn parse_enum_property(source: &str, prop: &str, enum_name: &str) -> Option<String> {
    let body = extract_property_body(source, prop)?;
    let rx = Regex::new(&format!(r"return\s+{}\.(\w+)\s*;", regex::escape(enum_name))).ok()?;
    rx.captures(&body).map(|c| c[1].to_string())
}

fn parse_slots(source: &str) -> Vec<String> {
    let Some(body) = extract_property_body(source, "Slots") else {
        return Vec::new();
    };
    let rx = Regex::new(r#""(\w+)""#).unwrap();
    let mut v: Vec<String> = rx
        .captures_iter(&body)
        .map(|c| c[1].to_string())
        .collect();
    v.dedup();
    v
}

fn parse_canonical_monsters(source: &str) -> Vec<MonsterSpawn> {
    let Some(body) = extract_method_body(source, "GenerateMonsters") else {
        return Vec::new();
    };
    // Two construction shapes both end with the ValueTuple<…> ctor:
    //   new ValueTuple<MonsterModel, string>(
    //       ModelDb.Monster<X>().ToMutable(), "slot")
    //   new ValueTuple<MonsterModel, string>(
    //       ModelDb.Monster<X>().ToMutable(), null)
    // and a third where the monster is built into a local first:
    //   X x = (X)ModelDb.Monster<X>().ToMutable();
    //   x.IsFront = true;
    //   new ValueTuple<MonsterModel, string>(x, null)
    //
    // For the third shape we don't have a way to know `x`'s underlying
    // type without parsing the local-decl. We do a two-stage approach:
    //   1. Direct ctor: regex over `ModelDb.Monster<X>()...,(slot|null)`.
    //   2. If direct ctor yielded nothing, walk locals: each `X y = (X)
    //      ModelDb.Monster<X>().ToMutable();` is a Monster<X> slot; the
    //      ordering in the body matches the spawn order in the returned
    //      array. Null slots only — IsFront / similar properties aren't
    //      passed through (the simulator derives that from slot index).
    let direct =
        Regex::new(r#"ModelDb\.Monster<(\w+)>\(\)[^,]*,\s*(?:"(\w+)"|null)"#)
            .unwrap();
    let direct_hits: Vec<MonsterSpawn> = direct
        .captures_iter(&body)
        .map(|c| MonsterSpawn {
            monster: c[1].to_string(),
            slot: c.get(2).map(|m| m.as_str().to_string()).unwrap_or_default(),
        })
        .collect();
    if !direct_hits.is_empty() {
        return direct_hits;
    }
    // Fallback: per-local pattern.
    let local =
        Regex::new(r#"(?:[A-Z]\w*)\s+\w+\s*=\s*\(\w+\)\s*ModelDb\.Monster<(\w+)>\(\)"#)
            .unwrap();
    local
        .captures_iter(&body)
        .map(|c| MonsterSpawn {
            monster: c[1].to_string(),
            slot: String::new(),
        })
        .collect()
}

/// Collects every `ModelDb.Monster<X>()` reference in the encounter file.
/// Some encounters (BowlbugsNormal) keep their roster in a static
/// `Dictionary<MonsterModel, int>` field rather than in `AllPossibleMonsters`,
/// so a whole-file scan is the most robust signal.
fn parse_possible_monsters(source: &str) -> Vec<String> {
    let rx = Regex::new(r"ModelDb\.Monster<(\w+)>\(\)").unwrap();
    let mut v: Vec<String> = rx
        .captures_iter(source)
        .map(|c| c[1].to_string())
        .collect();
    v.sort();
    v.dedup();
    v
}
