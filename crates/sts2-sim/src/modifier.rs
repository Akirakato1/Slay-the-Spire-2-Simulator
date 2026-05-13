//! Modifier (run-mode / Neow option) data table.
//!
//! 17 entries (includes `DeprecatedModifier` placeholder). Modifier
//! semantics live in their behavior virtuals (TryModifyRewardsLate, etc.)
//! and are out of scope for the data port — recorded here only as
//! id + `clears_player_deck` for now.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize)]
pub struct ModifierData {
    pub id: String,
    #[serde(default)]
    pub clears_player_deck: bool,
}

const MODIFIERS_JSON: &str = include_str!("../data/modifiers.json");

pub static ALL_MODIFIERS: LazyLock<Vec<ModifierData>> = LazyLock::new(|| {
    let mut v: Vec<ModifierData> =
        serde_json::from_str(MODIFIERS_JSON).expect("modifiers.json parse failed");
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
});

pub static MODIFIER_INDEX: LazyLock<HashMap<&'static str, &'static ModifierData>> =
    LazyLock::new(|| ALL_MODIFIERS.iter().map(|m| (m.id.as_str(), m)).collect());

pub fn by_id(id: &str) -> Option<&'static ModifierData> {
    MODIFIER_INDEX.get(id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_is_seventeen() {
        assert_eq!(ALL_MODIFIERS.len(), 17);
    }

    #[test]
    fn known_modifiers_present() {
        assert!(by_id("AllStar").is_some());
        assert!(by_id("Midas").is_some());
        assert!(by_id("Hoarder").is_some());
        assert!(by_id("SealedDeck").is_some());
    }
}
