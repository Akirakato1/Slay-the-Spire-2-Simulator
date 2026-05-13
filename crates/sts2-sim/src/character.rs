//! Character data table. 8 entries: 5 playable + Deprived (debug) +
//! RandomCharacter (any-character mode) + DeprecatedCharacter placeholder.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize)]
pub struct CharacterData {
    pub id: String,
    pub starting_hp: Option<i32>,
    pub starting_gold: Option<i32>,
    pub card_pool: Option<String>,
    pub potion_pool: Option<String>,
    pub relic_pool: Option<String>,
    #[serde(default)]
    pub starting_deck: Vec<String>,
    #[serde(default)]
    pub starting_relics: Vec<String>,
}

const CHARACTERS_JSON: &str = include_str!("../data/characters.json");

pub static ALL_CHARACTERS: LazyLock<Vec<CharacterData>> = LazyLock::new(|| {
    let mut v: Vec<CharacterData> =
        serde_json::from_str(CHARACTERS_JSON).expect("characters.json parse failed");
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
});

pub static CHARACTER_INDEX: LazyLock<HashMap<&'static str, &'static CharacterData>> =
    LazyLock::new(|| {
        ALL_CHARACTERS
            .iter()
            .map(|c| (c.id.as_str(), c))
            .collect()
    });

pub fn by_id(id: &str) -> Option<&'static CharacterData> {
    CHARACTER_INDEX.get(id).copied()
}

/// The five playable character ids in canonical order.
pub const PLAYABLE_CHARACTERS: &[&str] =
    &["Ironclad", "Silent", "Defect", "Regent", "Necrobinder"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_is_eight() {
        assert_eq!(ALL_CHARACTERS.len(), 8);
    }

    #[test]
    fn all_five_playable_characters_present() {
        for &ch in PLAYABLE_CHARACTERS {
            let c = by_id(ch).unwrap_or_else(|| panic!("missing {}", ch));
            assert!(c.starting_hp.is_some(), "{} has no HP", ch);
            assert!(c.card_pool.is_some(), "{} has no card pool", ch);
            assert!(!c.starting_deck.is_empty(), "{} has empty deck", ch);
            assert!(!c.starting_relics.is_empty(), "{} has no starting relic", ch);
        }
    }

    #[test]
    fn ironclad_signature_matches() {
        let ic = by_id("Ironclad").unwrap();
        assert_eq!(ic.starting_hp, Some(80));
        assert_eq!(ic.starting_gold, Some(99));
        assert_eq!(ic.card_pool.as_deref(), Some("Ironclad"));
        assert_eq!(ic.starting_deck.iter().filter(|c| *c == "StrikeIronclad").count(), 5);
        assert_eq!(ic.starting_deck.iter().filter(|c| *c == "DefendIronclad").count(), 4);
        assert!(ic.starting_deck.contains(&"Bash".to_string()));
        assert_eq!(ic.starting_relics, vec!["BurningBlood"]);
    }

    #[test]
    fn silent_has_twelve_card_starter_deck() {
        // 5 Strikes + 5 Defends + Neutralize + Survivor.
        let s = by_id("Silent").unwrap();
        assert_eq!(s.starting_deck.len(), 12);
        assert_eq!(s.starting_relics, vec!["RingOfTheSnake"]);
    }
}
