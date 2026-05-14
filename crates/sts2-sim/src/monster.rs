//! Monster data table. 117 MonsterModel subclasses; captures HP ranges
//! (base + ToughEnemies-ascended). Behavior — intent selection / move
//! state machines / damage dealt — is deferred to the combat port.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize)]
pub struct MonsterData {
    pub id: String,
    pub min_hp_base: Option<i32>,
    pub max_hp_base: Option<i32>,
    /// HP under the ToughEnemies ascension. None for monsters without
    /// ascension-scaled HP.
    #[serde(default)]
    pub min_hp_ascended: Option<i32>,
    #[serde(default)]
    pub max_hp_ascended: Option<i32>,
}

const MONSTERS_JSON: &str = include_str!("../data/monsters.json");

pub static ALL_MONSTERS: LazyLock<Vec<MonsterData>> = LazyLock::new(|| {
    let mut v: Vec<MonsterData> =
        serde_json::from_str(MONSTERS_JSON).expect("monsters.json parse failed");
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
});

pub static MONSTER_INDEX: LazyLock<HashMap<&'static str, &'static MonsterData>> =
    LazyLock::new(|| ALL_MONSTERS.iter().map(|m| (m.id.as_str(), m)).collect());

pub fn by_id(id: &str) -> Option<&'static MonsterData> {
    MONSTER_INDEX.get(id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_matches_extracted() {
        // 121 = 117 direct MonsterModel subclasses + 4 indirect
        // subclasses now picked up by the inheritance-walking
        // extractor (DecimillipedeSegmentFront/Middle/Back +
        // MysteriousKnight). HP inherited from intermediate
        // abstract bases (DecimillipedeSegment, FlailKnight).
        assert_eq!(ALL_MONSTERS.len(), 121);
    }

    #[test]
    fn axebot_signature_matches() {
        let m = by_id("Axebot").expect("Axebot present");
        assert_eq!(m.min_hp_base, Some(40));
        assert_eq!(m.max_hp_base, Some(44));
        assert_eq!(m.min_hp_ascended, Some(42));
        assert_eq!(m.max_hp_ascended, Some(46));
    }

    #[test]
    fn bowlbug_egg_signature_matches() {
        let m = by_id("BowlbugEgg").expect("BowlbugEgg present");
        assert_eq!(m.min_hp_base, Some(21));
        assert_eq!(m.max_hp_base, Some(22));
    }

    /// Most monsters should have parseable HP. Allow a small slack budget
    /// for unusual classes (bosses with computed HP).
    #[test]
    fn most_monsters_have_parseable_hp() {
        let with_hp = ALL_MONSTERS
            .iter()
            .filter(|m| m.min_hp_base.is_some() && m.max_hp_base.is_some())
            .count();
        assert!(
            with_hp >= ALL_MONSTERS.len() * 80 / 100,
            "only {} of {} monsters parsed HP",
            with_hp,
            ALL_MONSTERS.len()
        );
    }
}
