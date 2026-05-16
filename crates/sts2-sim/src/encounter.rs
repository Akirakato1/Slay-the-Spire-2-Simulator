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
    /// `EncounterModel.IsWeak`. Only meaningful when
    /// `room_type=Monster` — splits the Monster pool into the
    /// "first N hallway fights of the act" weak pool and the
    /// regular pool. Defaults `false` for Elite/Boss/uncategorized.
    #[serde(default)]
    pub is_weak: bool,
    #[serde(default)]
    pub slots: Vec<String>,
    #[serde(default)]
    pub canonical_monsters: Vec<MonsterSpawn>,
    #[serde(default)]
    pub possible_monsters: Vec<String>,
    /// `EncounterTag` enum values, used by C# `AddWithoutRepeatingTags`
    /// to forbid back-to-back encounters sharing a tag.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Which canonical acts (`Overgrowth` / `Hive` / `Glory` /
    /// `Underdocks`) include this encounter — walked from each act
    /// file's `GenerateAllEncounters()` body. An encounter may be in
    /// multiple acts; an empty list means none of the canonical pools
    /// reference it (event-encounters / deprecated / test fixtures).
    #[serde(default)]
    pub acts: Vec<String>,
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

/// All encounters in a specific act's `AllWeakEncounters` pool (the
/// "first N hallway fights" set). Mirrors C# `ActModel.AllWeakEncounters`
/// = `AllEncounters.Where(e => e.RoomType == Monster && e.IsWeak)`.
pub fn weak_encounters_for_act(act: &str) -> Vec<&'static EncounterData> {
    ALL_ENCOUNTERS
        .iter()
        .filter(|e| {
            e.room_type.as_deref() == Some("Monster")
                && e.is_weak
                && e.acts.iter().any(|a| a == act)
        })
        .collect()
}

/// All encounters in a specific act's `AllRegularEncounters` pool
/// (Monster room_type, not weak). Mirrors C# `ActModel.AllRegularEncounters`.
pub fn regular_encounters_for_act(act: &str) -> Vec<&'static EncounterData> {
    ALL_ENCOUNTERS
        .iter()
        .filter(|e| {
            e.room_type.as_deref() == Some("Monster")
                && !e.is_weak
                && e.acts.iter().any(|a| a == act)
        })
        .collect()
}

/// All encounters in a specific act's `AllEliteEncounters` pool.
pub fn elite_encounters_for_act(act: &str) -> Vec<&'static EncounterData> {
    ALL_ENCOUNTERS
        .iter()
        .filter(|e| {
            e.room_type.as_deref() == Some("Elite")
                && e.acts.iter().any(|a| a == act)
        })
        .collect()
}

/// All encounters in a specific act's `AllBossEncounters` pool.
pub fn boss_encounters_for_act(act: &str) -> Vec<&'static EncounterData> {
    ALL_ENCOUNTERS
        .iter()
        .filter(|e| {
            e.room_type.as_deref() == Some("Boss")
                && e.acts.iter().any(|a| a == act)
        })
        .collect()
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

    /// Overgrowth's pool (from `Acts/Overgrowth.cs` `GenerateAllEncounters`
    /// + the C# `IsWeak` / `RoomType` filters) — verifies the extractor
    /// captured every act-encounter relationship.
    #[test]
    fn overgrowth_pools_match_decompile() {
        let weak: Vec<_> = weak_encounters_for_act("Overgrowth")
            .iter().map(|e| e.id.as_str()).collect();
        let regular: Vec<_> = regular_encounters_for_act("Overgrowth")
            .iter().map(|e| e.id.as_str()).collect();
        let elite: Vec<_> = elite_encounters_for_act("Overgrowth")
            .iter().map(|e| e.id.as_str()).collect();
        let boss: Vec<_> = boss_encounters_for_act("Overgrowth")
            .iter().map(|e| e.id.as_str()).collect();
        // From Acts/Overgrowth.cs lines 33-47:
        //   Weak (IsWeak=true): FuzzyWurmCrawlerWeak, NibbitsWeak,
        //     ShrinkerBeetleWeak, SlimesWeak.
        //   Regular (Monster, !IsWeak): CubexConstructNormal,
        //     FlyconidNormal, FogmogNormal, InkletsNormal, MawlerNormal,
        //     NibbitsNormal, OvergrowthCrawlers, RubyRaidersNormal,
        //     SlimesNormal, SlitheringStranglerNormal, SnappingJaxfruitNormal,
        //     VineShamblerNormal.
        //   Elite: BygoneEffigyElite, ByrdonisElite, PhrogParasiteElite.
        //   Boss: CeremonialBeastBoss, TheKinBoss, VantomBoss.
        assert_eq!(weak.len(), 4, "weak: {:?}", weak);
        assert_eq!(elite.len(), 3, "elite: {:?}", elite);
        assert_eq!(boss.len(), 3, "boss: {:?}", boss);
        assert_eq!(regular.len(), 22 - 4 - 3 - 3,
            "regular: {:?}", regular);
        assert!(weak.contains(&"NibbitsWeak"));
        assert!(elite.contains(&"PhrogParasiteElite"));
        assert!(boss.contains(&"VantomBoss"));
    }

    /// Smoke: the four canonical act pools are non-empty. We don't
    /// assert "every Monster encounter is in some pool" because C#
    /// declares orphan EncounterModel classes (e.g. `TunnelerNormal`)
    /// that simply aren't listed in any `GenerateAllEncounters()` body
    /// — declared content that never rolls.
    #[test]
    fn act_pools_are_non_empty() {
        for act in ["Overgrowth", "Hive", "Glory", "Underdocks"] {
            assert!(!weak_encounters_for_act(act).is_empty(),
                "{} has no weak encounters", act);
            assert!(!regular_encounters_for_act(act).is_empty(),
                "{} has no regular encounters", act);
            assert!(!elite_encounters_for_act(act).is_empty(),
                "{} has no elite encounters", act);
            assert!(!boss_encounters_for_act(act).is_empty(),
                "{} has no boss encounters", act);
        }
    }

    /// Tags were extracted for the documented archetypes. Smoke: at
    /// least 10 encounters carry a tag, and a few known examples land
    /// correctly.
    #[test]
    fn tags_are_extracted() {
        let with_tags = ALL_ENCOUNTERS.iter().filter(|e| !e.tags.is_empty()).count();
        assert!(with_tags >= 10, "only {} tagged encounters", with_tags);
        // NibbitsWeak overrides `Tags` to include `EncounterTag.Nibbit`.
        // NibbitsNormal does NOT override and inherits `None` — so the
        // tag is asymmetric in C# itself. That's load-bearing for
        // `AddWithoutRepeatingTags`: only the weak variant blocks a
        // subsequent same-archetype draw.
        let nw = by_id("NibbitsWeak").unwrap();
        assert!(nw.tags.iter().any(|t| t == "Nibbit"),
            "NibbitsWeak tags: {:?}", nw.tags);
    }
}
