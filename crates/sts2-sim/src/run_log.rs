//! Parser for `.run` files — the JSON-serialized completed-run records
//! StS2 writes locally and ships through `sts2_stats`. Schema reference:
//! `sts2_stats/sample runs/run_file_fields.txt`.
//!
//! Pure deserialization for now; reconstruction (driving the simulator
//! through recorded choices to rebuild game state at each decision point)
//! lands in a follow-up once `RunState` exists.
//!
//! Schema version is 8/9 in observed samples. New fields land additively,
//! so most structs use `#[serde(default)]` to tolerate missing keys.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Top-level `.run` document.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RunLog {
    pub win: bool,
    pub was_abandoned: bool,
    pub ascension: i32,
    pub game_mode: String,
    pub seed: String,
    pub start_time: i64,
    pub run_time: i32,
    pub build_id: String,
    pub platform_type: String,
    pub acts: Vec<String>,
    pub killed_by_encounter: String,
    pub killed_by_event: String,
    pub modifiers: Vec<Value>,
    pub schema_version: i32,
    /// Outer index = act, inner index = sequential map point visited.
    pub map_point_history: Vec<Vec<NodeEntry>>,
    pub players: Vec<PlayerFinalState>,
}

impl Default for RunLog {
    fn default() -> Self {
        Self {
            win: false,
            was_abandoned: false,
            ascension: 0,
            game_mode: String::new(),
            seed: String::new(),
            start_time: 0,
            run_time: 0,
            build_id: String::new(),
            platform_type: String::new(),
            acts: Vec::new(),
            killed_by_encounter: String::new(),
            killed_by_event: String::new(),
            modifiers: Vec::new(),
            schema_version: 0,
            map_point_history: Vec::new(),
            players: Vec::new(),
        }
    }
}

/// One visited map point (one floor in the run). Captures the room model
/// faced and per-player stats accrued during the visit.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct NodeEntry {
    pub map_point_type: String,
    pub player_stats: Vec<PlayerStats>,
    pub rooms: Vec<RoomInfo>,
}

/// Per-floor, per-player stats and choices recorded during the visit.
/// Every field is optional in the source JSON — players that didn't act
/// on a given floor (e.g. coop, downed) get sparse entries.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PlayerStats {
    /// In coop runs this is a Steam ID (17-digit). Solo runs use small
    /// ints like 1. i64 covers both.
    pub player_id: i64,
    // HP
    pub current_hp: i32,
    pub max_hp: i32,
    pub damage_taken: i32,
    pub hp_healed: i32,
    pub max_hp_gained: i32,
    pub max_hp_lost: i32,
    // Gold
    pub current_gold: i32,
    pub gold_gained: i32,
    pub gold_lost: i32,
    pub gold_spent: i32,
    pub gold_stolen: i32,
    // Cards
    pub card_choices: Vec<CardChoice>,
    pub cards_gained: Vec<CardRef>,
    pub cards_removed: Vec<CardRef>,
    pub cards_transformed: Vec<CardTransform>,
    pub cards_enchanted: Vec<CardEnchanted>,
    pub upgraded_cards: Vec<String>,
    pub bought_colorless: Vec<String>,
    // Relics
    pub relic_choices: Vec<RelicChoice>,
    pub bought_relics: Vec<String>,
    pub ancient_choice: Vec<AncientChoice>,
    // Potions
    pub potion_choices: Vec<PotionChoice>,
    pub potion_used: Vec<String>,
    pub potion_discarded: Vec<String>,
    pub bought_potions: Vec<String>,
    // Events / rest sites
    pub event_choices: Vec<EventChoice>,
    pub rest_site_choices: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RoomInfo {
    pub map_point_type: String,
    pub room_type: String,
    pub model_id: String,
    pub monster_ids: Vec<String>,
    pub turns_taken: i32,
}

/// A card identifier optionally enriched with deck-context fields. Used in
/// card_choices, cards_gained, etc.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CardRef {
    pub id: String,
    /// Floor on which this card entered the deck. Absent in some contexts
    /// (e.g. starter deck has it set to 1 throughout; some reward entries
    /// omit it).
    pub floor_added_to_deck: Option<i32>,
    /// 0 = unupgraded, 1+ = upgraded levels.
    pub current_upgrade_level: Option<i32>,
    /// Some cards carry an enchantment payload.
    pub enchantment: Option<EnchantmentRef>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct EnchantmentRef {
    pub id: String,
    pub amount: i32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CardChoice {
    pub card: CardRef,
    pub was_picked: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CardTransform {
    pub original_card: CardRef,
    pub final_card: CardRef,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CardEnchanted {
    pub card: CardRef,
    /// String form of the enchantment id (separate from `card.enchantment`,
    /// which is the structured form). The schema doc lists both.
    pub enchantment: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RelicChoice {
    pub choice: String,
    pub was_picked: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AncientChoice {
    /// Source uses "TextKey" with that casing.
    #[serde(rename = "TextKey")]
    pub text_key: String,
    pub title: LocalizedKey,
    pub was_chosen: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct LocalizedKey {
    pub key: String,
    pub table: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PotionChoice {
    pub choice: String,
    pub was_picked: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct EventChoice {
    pub title: LocalizedKey,
    /// Map of variable_name → typed value. Present when the event option
    /// recorded ints / bools / strings (e.g. quantity picked). Schema is
    /// `{name: {type, decimal_value?, bool_value?, string_value?}}`.
    pub variables: Option<serde_json::Map<String, Value>>,
}

/// Final state of one player at run end.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PlayerFinalState {
    pub character: String,
    /// Solo runs use 1, coop uses Steam IDs (17-digit). i64 covers both.
    pub id: i64,
    pub max_potion_slot_count: i32,
    /// Schema doc says `potions[]` is a list of strings, but observed
    /// shape is `{id, slot_index}` objects.
    pub potions: Vec<PotionEntry>,
    pub deck: Vec<CardRef>,
    pub relics: Vec<RelicEntry>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PotionEntry {
    pub id: String,
    pub slot_index: i32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RelicEntry {
    pub id: String,
    pub floor_added_to_deck: i32,
    pub props: Option<RelicProps>,
}

/// Custom state some relics carry forward across the run (counters etc.).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RelicProps {
    pub ints: Vec<NamedInt>,
    pub bools: Vec<NamedBool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct NamedInt {
    pub name: String,
    pub value: i32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct NamedBool {
    pub name: String,
    pub value: bool,
}

/// Parse a `.run` file from a JSON string.
pub fn from_str(s: &str) -> serde_json::Result<RunLog> {
    serde_json::from_str(s)
}

/// Parse a `.run` file from a file path.
pub fn from_path<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<RunLog> {
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_object_uses_defaults() {
        let run: RunLog = from_str("{}").unwrap();
        assert!(!run.win);
        assert_eq!(run.ascension, 0);
        assert!(run.acts.is_empty());
        assert!(run.players.is_empty());
    }

    #[test]
    fn minimal_run_parses() {
        let json = r#"{
            "win": true,
            "ascension": 9,
            "seed": "ABC",
            "schema_version": 8,
            "acts": ["ACT.OVERGROWTH"],
            "players": [{
                "character": "CHARACTER.IRONCLAD",
                "id": 1,
                "deck": [{"id": "CARD.STRIKE_IRONCLAD", "floor_added_to_deck": 1}]
            }]
        }"#;
        let run: RunLog = from_str(json).unwrap();
        assert!(run.win);
        assert_eq!(run.ascension, 9);
        assert_eq!(run.seed, "ABC");
        assert_eq!(run.players.len(), 1);
        assert_eq!(run.players[0].character, "CHARACTER.IRONCLAD");
        assert_eq!(run.players[0].deck.len(), 1);
        assert_eq!(run.players[0].deck[0].id, "CARD.STRIKE_IRONCLAD");
        assert_eq!(run.players[0].deck[0].floor_added_to_deck, Some(1));
    }

    #[test]
    fn relic_with_props_parses() {
        let json = r#"{
            "id": "RELIC.GALACTIC_DUST",
            "floor_added_to_deck": 43,
            "props": {"ints": [{"name": "StarsSpent", "value": 7}]}
        }"#;
        let r: RelicEntry = serde_json::from_str(json).unwrap();
        assert_eq!(r.id, "RELIC.GALACTIC_DUST");
        let props = r.props.expect("props present");
        assert_eq!(props.ints.len(), 1);
        assert_eq!(props.ints[0].name, "StarsSpent");
        assert_eq!(props.ints[0].value, 7);
    }

    #[test]
    fn ancient_choice_renames_text_key_field() {
        let json = r#"{
            "TextKey": "POMANDER",
            "title": {"key": "POMANDER.title", "table": "relics"},
            "was_chosen": false
        }"#;
        let a: AncientChoice = serde_json::from_str(json).unwrap();
        assert_eq!(a.text_key, "POMANDER");
        assert!(!a.was_chosen);
    }
}
