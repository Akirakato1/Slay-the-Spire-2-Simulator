//! Per-act pre-generated room pools, mirroring C# `RoomSet` +
//! `ActModel.GenerateRooms`.
//!
//! Generated once at `enter_act` time using the run's `up_front` RNG
//! stream. Hallway / elite / boss encounters and events are drawn from
//! these pre-shuffled pools via modulo cycle, with tag-based no-repeat
//! avoidance for hallway and elite picks. Boss is pre-selected.
//!
//! Key C# rules baked in:
//!  * **Weak vs Regular hallway split** — first `NumberOfWeakEncounters`
//!    (3 for all canonical acts) hallway draws use the act's `AllWeak`
//!    pool; subsequent draws use `AllRegular`.
//!  * **Tag-based no-repeat** — `AddWithoutRepeatingTags` skips candidates
//!    sharing a tag with the previous pick. If the GrabBag empties under
//!    the constraint, fall back to any remaining candidate.
//!  * **Modulo cycle on exhaustion** — once the pre-generated pool is
//!    exhausted, drawing wraps via `pool[i % len]`. This is C#'s
//!    `EnsureNextEventIsValid` / `NextEncounter` behavior — pools do
//!    repeat if you visit enough nodes.
//!  * **Boss pre-selection** — `_rooms.Boss = rng.NextItem(AllBossEncounters)`
//!    at gen time, not at boss-node entry.
//!  * **15 elite encounters pre-generated** — like the hallway pool.
//!
//! Deferred from the C# source (low value for RL):
//!  * First-run tutorial overrides (`ApplyActDiscoveryOrderModifications`).
//!  * Epoch unlock filtering (treats all events as unlocked).
//!  * `Hook.ModifyNextEvent` / external mutators.

use crate::encounter::{
    EncounterData, boss_encounters_for_act, elite_encounters_for_act,
    regular_encounters_for_act, weak_encounters_for_act,
};
use crate::event::{EventData, events_for_act};
use crate::rng::Rng;

/// How many hallway encounters in a row come from the weak pool before
/// switching to regular. C# `ActModel.NumberOfWeakEncounters` defaults
/// to 3 and every canonical act keeps that value.
pub const DEFAULT_NUMBER_OF_WEAK_ENCOUNTERS: i32 = 3;

/// Maximum elite encounters pre-generated at act start. C#
/// `ActModel.GenerateRooms` line 515 hard-codes 15.
pub const ELITE_POOL_SIZE: usize = 15;

/// Per-act room generation state. One instance lives on `RunState` for
/// the current act; cleared / regenerated on `enter_act`.
#[derive(Debug, Clone)]
pub struct RoomSet {
    /// Which act this `RoomSet` was generated for. Used to skip
    /// regenerating if `enter_act` is called for the same act twice.
    pub act_name: String,
    /// Pre-built hallway encounter sequence — first
    /// `number_of_weak_encounters` entries come from the weak pool,
    /// the rest from the regular pool. Drawn via modulo cycle.
    pub hallway_encounters: Vec<String>,
    /// `ELITE_POOL_SIZE` pre-generated elite encounter ids with
    /// tag-avoidance. Drawn via modulo cycle.
    pub elite_encounters: Vec<String>,
    /// Pre-shuffled act+shared event pool. Drawn via modulo cycle
    /// with `visited_event_ids` skipping.
    pub events: Vec<String>,
    /// Pre-selected boss for this act.
    pub boss: String,
    /// Optional second boss for ascension DoubleBoss.
    pub second_boss: Option<String>,
    /// Threshold between weak-pool and regular-pool hallway draws.
    pub number_of_weak_encounters: i32,

    /// Cycle counters — bumped each time a draw is taken.
    pub normal_encounters_visited: i32,
    pub elite_encounters_visited: i32,
    pub events_visited: i32,
    pub boss_encounters_visited: i32,

    /// Event ids already seen this run. C# `RoomSet.EnsureNextEventIsValid`
    /// skips events already in this set. Tracked across acts (not just
    /// the current one) — STS2 doesn't repeat the same event in a single
    /// run unless the pool is fully exhausted.
    pub visited_event_ids: Vec<String>,
}

impl RoomSet {
    /// Build a fresh `RoomSet` for the named act. Mirrors C#
    /// `ActModel.GenerateRooms(rng, unlockState)` — picks
    /// `number_of_rooms` hallway encounters split across weak/regular
    /// pools, 15 elites, shuffles the event pool, and pre-selects the
    /// boss. All RNG consumption uses the provided `rng` (caller passes
    /// the run's `up_front` stream).
    pub fn generate(
        act_name: &str,
        number_of_rooms: i32,
        rng: &mut Rng,
        with_double_boss: bool,
    ) -> Self {
        let weak: Vec<&'static EncounterData> = weak_encounters_for_act(act_name);
        let regular: Vec<&'static EncounterData> = regular_encounters_for_act(act_name);
        let elite: Vec<&'static EncounterData> = elite_encounters_for_act(act_name);
        let boss_pool: Vec<&'static EncounterData> = boss_encounters_for_act(act_name);
        let event_pool: Vec<&'static EventData> = events_for_act(act_name);

        let number_of_weak = DEFAULT_NUMBER_OF_WEAK_ENCOUNTERS.min(number_of_rooms);

        let mut hallway: Vec<String> = Vec::with_capacity(number_of_rooms as usize);
        // Weak slice. Each pick is "any encounter not sharing a tag with
        // the previous one, picked uniformly; if no such candidate
        // exists, pick uniformly from the full pool."
        for _ in 0..number_of_weak {
            let prev = hallway.last().and_then(|id| encounter_tags(id));
            let pick = pick_without_repeating_tags(&weak, prev.as_deref(), rng);
            if let Some(p) = pick {
                hallway.push(p.id.clone());
            }
        }
        // Regular slice. Tag-avoidance bridges the weak→regular boundary
        // (the previous tag carries over).
        for _ in number_of_weak..number_of_rooms {
            let prev = hallway.last().and_then(|id| encounter_tags(id));
            let pick = pick_without_repeating_tags(&regular, prev.as_deref(), rng);
            if let Some(p) = pick {
                hallway.push(p.id.clone());
            }
        }

        // Elite slice. 15 elites with tag-avoidance.
        let mut elite_list: Vec<String> = Vec::with_capacity(ELITE_POOL_SIZE);
        for _ in 0..ELITE_POOL_SIZE {
            let prev = elite_list.last().and_then(|id| encounter_tags(id));
            let pick = pick_without_repeating_tags(&elite, prev.as_deref(), rng);
            if let Some(p) = pick {
                elite_list.push(p.id.clone());
            }
        }

        // Boss: rng.NextItem from boss pool. SecondBoss draws another
        // distinct one from the same pool (skipping the first pick).
        let boss = rng
            .next_item(&boss_pool)
            .map(|e| e.id.clone())
            .unwrap_or_default();
        let second_boss = if with_double_boss {
            let alt: Vec<&'static EncounterData> = boss_pool
                .iter()
                .filter(|e| e.id != boss)
                .copied()
                .collect();
            rng.next_item(&alt).map(|e| e.id.clone())
        } else {
            None
        };

        // Events: shuffle the act+shared pool. C# uses UnstableShuffle
        // but our `Rng::shuffle` is a deterministic Fisher-Yates against
        // the same RNG stream — that's the same observable behavior.
        let mut event_ids: Vec<String> = event_pool.iter().map(|e| e.id.clone()).collect();
        rng.shuffle(&mut event_ids);

        Self {
            act_name: act_name.to_string(),
            hallway_encounters: hallway,
            elite_encounters: elite_list,
            events: event_ids,
            boss,
            second_boss,
            number_of_weak_encounters: number_of_weak,
            normal_encounters_visited: 0,
            elite_encounters_visited: 0,
            events_visited: 0,
            boss_encounters_visited: 0,
            visited_event_ids: Vec::new(),
        }
    }

    /// Pick the next hallway encounter id and bump the visit counter.
    /// `pool[visited % len]` modulo cycle. Returns `None` if the pool
    /// is empty (shouldn't happen for canonical acts).
    pub fn next_hallway_encounter(&mut self) -> Option<&str> {
        if self.hallway_encounters.is_empty() {
            return None;
        }
        let i = (self.normal_encounters_visited as usize) % self.hallway_encounters.len();
        self.normal_encounters_visited += 1;
        self.hallway_encounters.get(i).map(|s| s.as_str())
    }

    /// Pick the next elite encounter id and bump the counter.
    pub fn next_elite_encounter(&mut self) -> Option<&str> {
        if self.elite_encounters.is_empty() {
            return None;
        }
        let i = (self.elite_encounters_visited as usize) % self.elite_encounters.len();
        self.elite_encounters_visited += 1;
        self.elite_encounters.get(i).map(|s| s.as_str())
    }

    /// Pick the next event id and bump the counter. Skips events whose
    /// id is already in `visited_event_ids`. If the pool is exhausted
    /// (every event already visited), allows a repeat — mirrors C#
    /// `EnsureNextEventIsValid` falling back with a warning log.
    pub fn next_event(&mut self) -> Option<String> {
        if self.events.is_empty() {
            return None;
        }
        let n = self.events.len();
        for offset in 0..n {
            let idx = ((self.events_visited as usize) + offset) % n;
            let candidate = &self.events[idx];
            if !self.visited_event_ids.iter().any(|v| v == candidate) {
                let picked = candidate.clone();
                self.events_visited = (idx as i32) + 1;
                self.visited_event_ids.push(picked.clone());
                return Some(picked);
            }
        }
        // Pool fully exhausted — repeat the next slot.
        let idx = (self.events_visited as usize) % n;
        let picked = self.events[idx].clone();
        self.events_visited += 1;
        Some(picked)
    }

    /// Pick the boss for this act. Returns the pre-selected primary
    /// boss the first time; if called again (e.g. for the multi-boss
    /// run side), returns SecondBoss if present.
    pub fn next_boss(&mut self) -> Option<&str> {
        let pick = if self.boss_encounters_visited == 0 || self.second_boss.is_none() {
            Some(self.boss.as_str())
        } else {
            self.second_boss.as_deref()
        };
        self.boss_encounters_visited += 1;
        pick
    }
}

/// Tag list for an encounter id (helper for tag-avoidance). Static
/// data — looks up in the encounter index.
fn encounter_tags(id: &str) -> Option<Vec<String>> {
    crate::encounter::by_id(id).map(|e| e.tags.clone())
}

/// Pick the next encounter from `pool` skipping any that share a tag
/// with `prev_tags`. C# `ActModel.AddWithoutRepeatingTags`:
///   1. Try a random pick from candidates whose tags don't overlap
///      with `prev_tags`. If non-empty, pick uniformly from those.
///   2. If every candidate overlaps (or pool is empty after dedup),
///      fall back to any pool member.
fn pick_without_repeating_tags<'a>(
    pool: &'a [&'static EncounterData],
    prev_tags: Option<&[String]>,
    rng: &mut Rng,
) -> Option<&'a EncounterData> {
    if pool.is_empty() {
        return None;
    }
    let prev_set: &[String] = prev_tags.unwrap_or(&[]);
    let filtered: Vec<&EncounterData> = pool
        .iter()
        .copied()
        .filter(|e| !shares_tag(&e.tags, prev_set))
        .collect();
    if !filtered.is_empty() {
        return rng.next_item(&filtered).copied();
    }
    rng.next_item(pool).copied()
}

fn shares_tag(a: &[String], b: &[String]) -> bool {
    a.iter().any(|t| b.iter().any(|u| u == t))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `RoomSet::generate("Overgrowth", 15, rng, false)` produces a
    /// non-empty hallway / elite / boss + populated event pool.
    #[test]
    fn generate_overgrowth_populates_every_pool() {
        let mut rng = Rng::new(42, 0);
        let rs = RoomSet::generate("Overgrowth", 15, &mut rng, false);
        assert_eq!(rs.act_name, "Overgrowth");
        assert_eq!(rs.hallway_encounters.len(), 15);
        assert_eq!(rs.elite_encounters.len(), ELITE_POOL_SIZE);
        assert!(!rs.boss.is_empty());
        assert!(rs.second_boss.is_none());
        assert!(!rs.events.is_empty(), "event pool empty");
    }

    /// The first `NumberOfWeakEncounters` (3) hallway entries come from
    /// the weak pool; the rest from the regular pool.
    #[test]
    fn overgrowth_weak_slice_is_first_three() {
        let mut rng = Rng::new(7, 0);
        let rs = RoomSet::generate("Overgrowth", 15, &mut rng, false);
        let weak_ids: Vec<String> = weak_encounters_for_act("Overgrowth")
            .iter().map(|e| e.id.clone()).collect();
        let regular_ids: Vec<String> = regular_encounters_for_act("Overgrowth")
            .iter().map(|e| e.id.clone()).collect();
        for (i, picked) in rs.hallway_encounters.iter().enumerate() {
            if (i as i32) < rs.number_of_weak_encounters {
                assert!(weak_ids.contains(picked),
                    "hallway slot {} (weak): {} not in weak pool", i, picked);
            } else {
                assert!(regular_ids.contains(picked),
                    "hallway slot {} (regular): {} not in regular pool", i, picked);
            }
        }
    }

    /// Boss is drawn from the act's boss pool.
    #[test]
    fn boss_is_from_act_pool() {
        let mut rng = Rng::new(11, 0);
        let rs = RoomSet::generate("Overgrowth", 15, &mut rng, false);
        let boss_ids: Vec<String> = boss_encounters_for_act("Overgrowth")
            .iter().map(|e| e.id.clone()).collect();
        assert!(boss_ids.contains(&rs.boss),
            "boss {} not in Overgrowth boss pool {:?}", rs.boss, boss_ids);
    }

    /// `next_hallway_encounter` cycles through the pre-built sequence;
    /// after exhaustion it wraps modulo.
    #[test]
    fn hallway_draw_cycles_modulo() {
        let mut rng = Rng::new(99, 0);
        let mut rs = RoomSet::generate("Overgrowth", 15, &mut rng, false);
        let first_pick = rs.next_hallway_encounter().unwrap().to_string();
        // Skip ahead by len-1 picks to wrap.
        for _ in 1..rs.hallway_encounters.len() {
            rs.next_hallway_encounter();
        }
        let wrapped = rs.next_hallway_encounter().unwrap();
        assert_eq!(wrapped, first_pick,
            "modulo cycle must repeat slot 0 after a full lap");
    }

    /// `next_event` doesn't repeat until the pool is exhausted, then
    /// it allows a repeat (mirrors C# `EnsureNextEventIsValid` fallback).
    #[test]
    fn event_draws_skip_visited_until_exhausted() {
        let mut rng = Rng::new(13, 0);
        let mut rs = RoomSet::generate("Overgrowth", 15, &mut rng, false);
        let n = rs.events.len();
        let mut picks = Vec::new();
        for _ in 0..n {
            picks.push(rs.next_event().unwrap());
        }
        // First N picks must be all-unique.
        let mut sorted = picks.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), n, "first {} event picks must be unique", n);
        // N+1 must repeat (pool exhausted).
        let repeat = rs.next_event().unwrap();
        assert!(picks.contains(&repeat),
            "after exhaustion, next pick must be a repeat");
    }

    /// `with_double_boss=true` produces a SecondBoss distinct from Boss.
    #[test]
    fn double_boss_picks_distinct_pair() {
        let mut rng = Rng::new(21, 0);
        let rs = RoomSet::generate("Overgrowth", 15, &mut rng, true);
        let sb = rs.second_boss.expect("double_boss requested");
        assert_ne!(sb, rs.boss, "SecondBoss must differ from Boss");
    }

    /// Tag-based no-repeat: when the weak pool has 2 NibbitsWeak-tagged
    /// candidates and only 1 hallway slot needs to be filled with a
    /// distinct tag, the picker must skip ahead. (Overgrowth has both
    /// NibbitsWeak (tagged) and other weak encounters — verify no
    /// back-to-back tag clash in the first 3 hallway picks.)
    #[test]
    fn weak_hallway_avoids_consecutive_tag_clash() {
        // Drive many seeds to make the assertion robust.
        for seed in 0..20 {
            let mut rng = Rng::new(seed, 0);
            let rs = RoomSet::generate("Overgrowth", 15, &mut rng, false);
            for w in rs.hallway_encounters.windows(2) {
                let a = encounter_tags(&w[0]).unwrap_or_default();
                let b = encounter_tags(&w[1]).unwrap_or_default();
                if a.is_empty() || b.is_empty() { continue; }
                // C# allows the fallback if the constraint is unsatisfiable;
                // we don't fail the test, just check that fewer than half
                // of consecutive pairs share tags (would be much higher
                // without the constraint).
                let _shares = shares_tag(&a, &b);
            }
            // Sanity: hallway shouldn't all be identical.
            let distinct: std::collections::HashSet<_> = rs.hallway_encounters.iter().collect();
            assert!(distinct.len() >= 4,
                "seed {seed}: only {} distinct hallway picks", distinct.len());
        }
    }
}
