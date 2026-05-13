//! Affliction data table. Thin status-flag adjuncts to Powers; 8 entries.
//! Captures `has_extra_card_text` and `is_stackable` virtual overrides.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize)]
pub struct AfflictionData {
    pub id: String,
    #[serde(default)]
    pub has_extra_card_text: bool,
    #[serde(default)]
    pub is_stackable: bool,
}

const AFFLICTIONS_JSON: &str = include_str!("../data/afflictions.json");

pub static ALL_AFFLICTIONS: LazyLock<Vec<AfflictionData>> = LazyLock::new(|| {
    let mut v: Vec<AfflictionData> =
        serde_json::from_str(AFFLICTIONS_JSON).expect("afflictions.json parse failed");
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
});

pub static AFFLICTION_INDEX: LazyLock<HashMap<&'static str, &'static AfflictionData>> =
    LazyLock::new(|| {
        ALL_AFFLICTIONS
            .iter()
            .map(|a| (a.id.as_str(), a))
            .collect()
    });

pub fn by_id(id: &str) -> Option<&'static AfflictionData> {
    AFFLICTION_INDEX.get(id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_is_eight() {
        assert_eq!(ALL_AFFLICTIONS.len(), 8);
    }

    #[test]
    fn galvanized_is_stackable() {
        let g = by_id("Galvanized").expect("Galvanized present");
        assert!(g.is_stackable);
        assert!(g.has_extra_card_text);
    }

    #[test]
    fn bound_has_extra_card_text() {
        let b = by_id("Bound").expect("Bound present");
        assert!(b.has_extra_card_text);
        assert!(!b.is_stackable);
    }
}
