//! One-shot power-data extractor.
//!
//! Walks the StS2 decompile, parses each `Models/Powers/*.cs`, and emits a
//! JSON table to `crates/sts2-sim/data/powers.json`.
//!
//! Powers are flat (no pool registry — any power can be applied by any
//! source). Membership = "every class in Models/Powers/ that derives from
//! PowerModel" (some files in there are helper/utility classes that aren't
//! actually powers).
//!
//! Per-power data captured:
//!   - `Type` override → PowerType (Buff/Debuff)
//!   - `StackType` override → PowerStackType (Counter/Single)
//!   - `AllowNegative` override → bool (default false)
//!   - `CanonicalVars` (optional) — same DynamicVar palette as cards/relics
//!
//! Behavior virtuals (`AfterSideTurnStart`, `ModifyDamageAdditive`,
//! `ModifyDamageMultiplicative`, ...) are deferred.
//!
//! Re-run on game updates: `cargo run -p extract-powers`.

use anyhow::{Context, Result, anyhow, bail};
use extractors_common::{
    DynamicVarSpec, extract_property_body, parse_canonical_vars,
    workspace_root_from_manifest,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_MODELS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sts2_decompiled\sts2\MegaCrit\sts2\Core\Models";

const OUTPUT_REL: &str = r"crates\sts2-sim\data\powers.json";

#[derive(Debug, Serialize, Deserialize)]
struct PowerData {
    id: String,
    /// `PowerType` from the C# override: "None" / "Buff" / "Debuff".
    /// Defaults to "None" when not overridden (rare).
    power_type: String,
    /// `PowerStackType` from the C# override: "None" / "Counter" / "Single".
    stack_type: String,
    /// `AllowNegative` virtual. Default false. Strength is the canonical
    /// AllowNegative=true case (Weak/etc. can drive it negative).
    #[serde(default, skip_serializing_if = "is_false")]
    allow_negative: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    canonical_vars: Vec<DynamicVarSpec>,
}

fn is_false(v: &bool) -> bool {
    !*v
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
    let powers_dir = models_dir.join("Powers");

    let ids = list_power_ids_on_disk(&powers_dir)?;
    let mut powers: Vec<PowerData> = Vec::new();
    for id in &ids {
        let path = powers_dir.join(format!("{id}.cs"));
        let source = fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let power = parse_power(id, &source)
            .with_context(|| format!("parsing {} ({})", id, path.display()))?;
        powers.push(power);
    }

    powers.sort_by(|a, b| a.id.cmp(&b.id));

    let workspace_root = workspace_root_from_manifest(env!("CARGO_MANIFEST_DIR"))?;
    let output = workspace_root.join(OUTPUT_REL);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&powers)?;
    fs::write(&output, json)?;
    eprintln!("wrote {} powers to {}", powers.len(), output.display());
    Ok(())
}

fn parse_power(id: &str, source: &str) -> Result<PowerData> {
    // Type override is the only one that's *required* — every power must be
    // Buff or Debuff. Most powers override it directly; TemporaryStrengthPower
    // subclasses (SetupStrikePower etc.) inherit their Type from the abstract
    // parent, which returns Buff (IsPositive=true default) or Debuff
    // (IsPositive=false). We default to Buff and let an `IsPositive`
    // override in the subclass flip to Debuff.
    let inherits_temp_strength = source.contains(": TemporaryStrengthPower");
    let power_type = match parse_enum_property(source, "Type", "PowerType") {
        Some(t) => t,
        None if inherits_temp_strength => {
            if parse_bool_property(source, "IsPositive") == Some(false) {
                "Debuff".to_string()
            } else {
                "Buff".to_string()
            }
        }
        None => bail!("no PowerType override in {}", id),
    };
    let stack_type = parse_enum_property(source, "StackType", "PowerStackType")
        .unwrap_or_else(|| {
            if inherits_temp_strength {
                // TemporaryStrengthPower hardcodes StackType.Counter.
                "Counter".to_string()
            } else {
                "None".to_string()
            }
        });
    let allow_negative = parse_bool_property(source, "AllowNegative").unwrap_or(false);
    let canonical_vars = parse_canonical_vars(source);
    Ok(PowerData {
        id: id.to_string(),
        power_type,
        stack_type,
        allow_negative,
        canonical_vars,
    })
}

fn parse_enum_property(source: &str, prop: &str, enum_name: &str) -> Option<String> {
    let body = extract_property_body(source, prop)?;
    let rx = Regex::new(&format!(r"return\s+{}\.(\w+)\s*;", regex::escape(enum_name))).ok()?;
    rx.captures(&body).map(|c| c[1].to_string())
}

fn parse_bool_property(source: &str, name: &str) -> Option<bool> {
    let body = extract_property_body(source, name)?;
    if body.contains("return true") {
        Some(true)
    } else if body.contains("return false") {
        Some(false)
    } else {
        None
    }
}

fn list_power_ids_on_disk(powers_dir: &Path) -> Result<Vec<String>> {
    // Filter to files whose body actually derives from PowerModel, OR
    // from a known abstract intermediate that derives from PowerModel.
    // SetupStrikePower etc. extend TemporaryStrengthPower, which is the
    // only such intermediate today. As more intermediates appear, add
    // them here — keeping the list explicit (rather than transitively
    // resolving) avoids accidentally pulling in helper classes.
    const INTERMEDIATE_POWER_BASES: &[&str] = &[
        "TemporaryStrengthPower",
    ];
    let mut out: Vec<String> = Vec::new();
    for entry in fs::read_dir(powers_dir)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|s| s.to_str()) != Some("cs") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let source = fs::read_to_string(&p)?;
        // Skip abstract base classes themselves — they're declared
        // `public abstract class X`.
        if source.contains(&format!("abstract class {}", stem)) {
            continue;
        }
        let mut is_power = source.contains(": PowerModel");
        if !is_power {
            for base in INTERMEDIATE_POWER_BASES {
                if source.contains(&format!(": {}", base)) {
                    is_power = true;
                    break;
                }
            }
        }
        if is_power {
            out.push(stem.to_string());
        }
    }
    out.sort();
    Ok(out)
}
