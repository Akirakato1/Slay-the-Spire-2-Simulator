//! Shared helpers for the decompile-text-parse extractors.
//!
//! Pulled out when the third extractor (powers) was about to copy the same
//! `extract_method_body`/`extract_property_body`/`parse_canonical_vars`
//! functions for the third time. Keep this crate small and focused on the
//! pure-text C# parsing primitives — anything model-specific (pool reverse
//! maps, schema-typed deserialization) stays in the per-data extractor.

use anyhow::{Result, anyhow};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One entry from a `CanonicalVars` property body. The same shape is used by
/// cards, relics, and powers — the C# decompile emits `new <X>Var(...)`
/// constructors uniformly across them. Per-data crates may copy this struct
/// into their own runtime schema with stricter typing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DynamicVarSpec {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub generic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub base_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value_prop: Option<String>,
}

/// Returns the text between the matching outer braces of the named method.
/// Method signature can span multiple lines; we look for `<name>(` then walk
/// to the first `{` and brace-match. Returns `None` if no body is found or
/// the braces are unbalanced.
pub fn extract_method_body(source: &str, name: &str) -> Option<String> {
    let needle = format!("{name}(");
    let mut search = 0;
    while let Some(pos) = source[search..].find(&needle) {
        let abs = search + pos;
        // Boundary check: the byte before the name must not be a word char,
        // so we don't match suffixed names like `MaybeFoo` when looking for
        // `Foo`.
        let prev = source[..abs].chars().rev().next();
        if matches!(prev, Some(c) if c.is_ascii_alphanumeric() || c == '_') {
            search = abs + needle.len();
            continue;
        }
        let after = &source[abs + needle.len()..];
        let open = after.find('{')?;
        let body_start = abs + needle.len() + open + 1;
        let mut depth: i32 = 1;
        let bytes = source.as_bytes();
        let mut i = body_start;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if depth != 0 {
            return None;
        }
        return Some(source[body_start..i - 1].to_string());
    }
    None
}

/// Returns the body of a property `get` block (block-bodied) or expression
/// body (`=>`). Handles both forms the decompile emits.
pub fn extract_property_body(source: &str, name: &str) -> Option<String> {
    let rx = Regex::new(&format!(r"(?m)\b{}\s*(\{{|=>)", regex::escape(name))).ok()?;
    let m = rx.find(source)?;
    let after = &source[m.end()..];
    if m.as_str().ends_with("=>") {
        let mut depth: i32 = 0;
        let bytes = after.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => depth -= 1,
                b';' if depth == 0 => return Some(after[..i].to_string()),
                _ => {}
            }
            i += 1;
        }
        return None;
    }
    // Block body: walk into the first `get { ... }`.
    let get_idx = after.find("get")?;
    let after_get = &after[get_idx + 3..];
    let open = after_get.find('{')?;
    let body_start_rel = get_idx + 3 + open + 1;
    let mut depth: i32 = 1;
    let bytes = after.as_bytes();
    let mut i = body_start_rel;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    if depth != 0 {
        return None;
    }
    Some(after[body_start_rel..i - 1].to_string())
}

/// Parses every `new <X>Var(...)` constructor inside the `CanonicalVars`
/// property body. Returns an empty vec if the property is not overridden.
///
/// Recognizes two argument shapes:
///   - keyed:  `new DynamicVar("Key", 1m)`
///   - bare:   `new DamageVar(5m, ValueProp.Move)` / `new CardsVar(3)`
///
/// The numeric literal's `m` suffix (C# decimal) is optional — some bare-int
/// constructors omit it.
pub fn parse_canonical_vars(source: &str) -> Vec<DynamicVarSpec> {
    let Some(body) = extract_property_body(source, "CanonicalVars") else {
        return Vec::new();
    };
    let rx = Regex::new(
        r#"new\s+(\w+)Var(?:<(\w+)>)?\s*\(\s*(?:"(\w+)"\s*,\s*(-?\d+(?:\.\d+)?)m?|(-?\d+(?:\.\d+)?)m?)?\s*(?:,\s*ValueProp\.(\w+))?"#,
    )
    .unwrap();
    rx.captures_iter(&body)
        .map(|c| {
            let kind = c[1].to_string();
            let generic = c.get(2).map(|m| m.as_str().to_string());
            let key = c.get(3).map(|m| m.as_str().to_string());
            let base_value: Option<f64> = c
                .get(4)
                .or_else(|| c.get(5))
                .and_then(|m| m.as_str().parse().ok());
            let value_prop = c.get(6).map(|m| m.as_str().to_string());
            DynamicVarSpec {
                kind,
                generic,
                key,
                base_value,
                value_prop,
            }
        })
        .collect()
}

/// Walks two levels up from the caller crate's `CARGO_MANIFEST_DIR` to find
/// the workspace root (sim/). Each extractor lives at
/// `sim/tools/<crate>/Cargo.toml`, so two `parent()` calls land on `sim/`.
///
/// Caller must pass `env!("CARGO_MANIFEST_DIR")` — `env!` is resolved at the
/// caller's compile time, not this lib's, so we accept the value rather than
/// reading it here.
pub fn workspace_root_from_manifest(manifest_dir: &str) -> Result<PathBuf> {
    let manifest = Path::new(manifest_dir);
    let workspace = manifest
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| anyhow!("could not derive workspace root from {}", manifest.display()))?;
    Ok(workspace.to_path_buf())
}
