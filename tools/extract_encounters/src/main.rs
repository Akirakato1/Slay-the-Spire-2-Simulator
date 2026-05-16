//! Encounter data extractor.
//!
//! Captures per-encounter: room_type (Monster/Elite/Boss), IsWeak (only
//! meaningful when room_type=Monster — separates weak-pool from regular-
//! pool), slot names, the canonical (Monster, slot) spawn list, the
//! broader "possible monsters" set, encounter tags (used by C# for the
//! `AddWithoutRepeatingTags` no-repeat constraint), and which acts
//! include the encounter (walked from each `Acts/{ActName}.cs`
//! `GenerateAllEncounters()` body).

use anyhow::{Context, Result, bail};
use extractors_common::{extract_method_body, extract_property_body, workspace_root_from_manifest};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_MODELS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sts2_decompiled\sts2\MegaCrit\sts2\Core\Models";

const OUTPUT_REL: &str = r"crates\sts2-sim\data\encounters.json";

#[derive(Debug, Serialize, Deserialize)]
struct EncounterData {
    id: String,
    room_type: Option<String>,
    /// Only meaningful when room_type=Monster. C# `EncounterModel.IsWeak`
    /// splits the Monster pool into the "first N hallway fights" weak
    /// pool and the regular pool. Defaults false for Elite/Boss.
    #[serde(default)]
    is_weak: bool,
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
    /// `EncounterTag` enum values used by C# `AddWithoutRepeatingTags`
    /// to avoid back-to-back same-archetype encounters.
    #[serde(default)]
    tags: Vec<String>,
    /// Which acts include this encounter, walked from each act's
    /// `GenerateAllEncounters()` body. An encounter may appear in
    /// multiple acts; an empty list means none of the canonical acts
    /// (Overgrowth/Hive/Glory/Underdocks) list it.
    #[serde(default)]
    acts: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MonsterSpawn {
    monster: String,
    slot: String,
}

const ACT_NAMES: &[&str] = &["Overgrowth", "Hive", "Glory", "Underdocks"];

fn main() -> Result<()> {
    let models_dir = std::env::var("STS2_DECOMPILE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODELS_DIR));
    if !models_dir.exists() {
        bail!("decompile Models dir not found: {}", models_dir.display());
    }
    let dir = models_dir.join("Encounters");
    let acts_dir = models_dir.join("Acts");

    // First walk: build encounter_id -> Vec<ActId> map by parsing every
    // canonical act file's GenerateAllEncounters() body.
    let act_pools = parse_act_pools(&acts_dir)?;
    // Invert: encounter -> list of acts.
    let mut encounter_acts: HashMap<String, Vec<String>> = HashMap::new();
    for (act, ids) in &act_pools {
        for id in ids {
            encounter_acts.entry(id.clone()).or_default().push(act.clone());
        }
    }

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
        let mut acts = encounter_acts
            .get(stem)
            .cloned()
            .unwrap_or_default();
        acts.sort();
        out.push(EncounterData {
            id: stem.to_string(),
            room_type: parse_enum_property(&source, "RoomType", "RoomType"),
            is_weak: parse_bool_property(&source, "IsWeak"),
            slots: parse_slots(&source),
            canonical_monsters: parse_canonical_monsters(&source),
            possible_monsters: parse_possible_monsters(&source),
            tags: parse_tags(&source),
            acts,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));

    let workspace_root = workspace_root_from_manifest(env!("CARGO_MANIFEST_DIR"))?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_string_pretty(&out)?)?;

    // Stats so the human running the extractor can sanity-check.
    let weak = out.iter().filter(|e| e.is_weak).count();
    let monster = out.iter().filter(|e| e.room_type.as_deref() == Some("Monster")).count();
    let elite = out.iter().filter(|e| e.room_type.as_deref() == Some("Elite")).count();
    let boss = out.iter().filter(|e| e.room_type.as_deref() == Some("Boss")).count();
    let act_assigned = out.iter().filter(|e| !e.acts.is_empty()).count();
    eprintln!(
        "wrote {} encounters ({} weak / {} monster / {} elite / {} boss; {} act-assigned)",
        out.len(),
        weak,
        monster,
        elite,
        boss,
        act_assigned,
    );
    let printed: HashSet<&str> = HashSet::new();
    let _ = printed;
    eprintln!("per-act pool sizes:");
    for act in ACT_NAMES {
        let n = out.iter().filter(|e| e.acts.iter().any(|a| a == act)).count();
        eprintln!("  {:<11} {}", act, n);
    }
    eprintln!("output → {}", output.display());
    Ok(())
}

/// Walk `Acts/{Name}.cs` files. Each `GenerateAllEncounters()` returns a
/// `new EncounterModel[]` array of `ModelDb.Encounter<X>()` calls. We
/// extract every `X` ID per act.
fn parse_act_pools(acts_dir: &Path) -> Result<HashMap<String, Vec<String>>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for act_name in ACT_NAMES {
        let path = acts_dir.join(format!("{}.cs", act_name));
        if !path.exists() {
            eprintln!("warn: act file missing: {}", path.display());
            continue;
        }
        let source = fs::read_to_string(&path)?;
        let Some(body) = extract_method_body(&source, "GenerateAllEncounters") else {
            eprintln!("warn: no GenerateAllEncounters() body in {}", path.display());
            continue;
        };
        let rx = Regex::new(r"ModelDb\.Encounter<(\w+)>\(\)").unwrap();
        let ids: Vec<String> = rx
            .captures_iter(&body)
            .map(|c| c[1].to_string())
            .collect();
        out.insert(act_name.to_string(), ids);
    }
    Ok(out)
}

fn parse_enum_property(source: &str, prop: &str, enum_name: &str) -> Option<String> {
    let body = extract_property_body(source, prop)?;
    let rx = Regex::new(&format!(r"return\s+{}\.(\w+)\s*;", regex::escape(enum_name))).ok()?;
    rx.captures(&body).map(|c| c[1].to_string())
}

/// Read a `public override bool IsWeak { get { return X; } }` style
/// property. Defaults to `false` if the property is absent (which the
/// base class does — only weak monster encounters override it).
fn parse_bool_property(source: &str, prop: &str) -> bool {
    let Some(body) = extract_property_body(source, prop) else {
        return false;
    };
    let rx = Regex::new(r"return\s+(true|false)\s*;").unwrap();
    rx.captures(&body)
        .map(|c| &c[1] == "true")
        .unwrap_or(false)
}

/// Tags property body contains `EncounterTag.X` references. We extract
/// every distinct enum member.
fn parse_tags(source: &str) -> Vec<String> {
    let Some(body) = extract_property_body(source, "Tags") else {
        return Vec::new();
    };
    let rx = Regex::new(r"EncounterTag\.(\w+)").unwrap();
    let mut v: Vec<String> = rx
        .captures_iter(&body)
        .map(|c| c[1].to_string())
        .collect();
    v.sort();
    v.dedup();
    // Drop the explicit `None` tag — semantically equivalent to "no tags."
    v.retain(|t| t != "None");
    v
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
