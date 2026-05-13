//! Encounter data table. 88 EncounterModel subclasses; records the canonical
//! (monster, slot) spawn list, broader possible-monster set, slot layout,
//! and room_type (Monster/Elite/Boss).

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize)]
pub struct EncounterData {
    pub id: String,
    /// `RoomType` enum: "Monster", "Elite", "Boss", etc.
    pub room_type: Option<String>,
    #[serde(default)]
    pub slots: Vec<String>,
    #[serde(default)]
    pub canonical_monsters: Vec<MonsterSpawn>,
    #[serde(default)]
    pub possible_monsters: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MonsterSpawn {
    pub monster: String,
    pub slot: String,
}

const ENCOUNTERS_JSON: &str = include_str!("../data/encounters.json");

pub static ALL_ENCOUNTERS: LazyLock<Vec<EncounterData>> = LazyLock::new(|| {
    let mut v: Vec<EncounterData> =
        serde_json::from_str(ENCOUNTERS_JSON).expect("encounters.json parse failed");
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
});

pub static ENCOUNTER_INDEX: LazyLock<HashMap<&'static str, &'static EncounterData>> =
    LazyLock::new(|| {
        ALL_ENCOUNTERS
            .iter()
            .map(|e| (e.id.as_str(), e))
            .collect()
    });

pub fn by_id(id: &str) -> Option<&'static EncounterData> {
    ENCOUNTER_INDEX.get(id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_is_eighty_eight() {
        assert_eq!(ALL_ENCOUNTERS.len(), 88);
    }

    #[test]
    fn axebots_normal_spawns_two_axebots() {
        let e = by_id("AxebotsNormal").expect("AxebotsNormal present");
        assert_eq!(e.room_type.as_deref(), Some("Monster"));
        assert_eq!(e.slots, vec!["front", "back"]);
        assert_eq!(e.canonical_monsters.len(), 2);
        assert!(e.canonical_monsters.iter().all(|m| m.monster == "Axebot"));
    }

    /// Every encounter (except deprecated placeholders) should reference
    /// at least one monster somewhere.
    #[test]
    fn encounters_reference_at_least_one_monster() {
        for e in ALL_ENCOUNTERS.iter() {
            if e.id == "DeprecatedEncounter" {
                continue;
            }
            assert!(
                !e.canonical_monsters.is_empty() || !e.possible_monsters.is_empty(),
                "encounter {} references no monsters",
                e.id
            );
        }
    }
}
