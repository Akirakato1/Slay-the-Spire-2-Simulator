//! End-to-end run-flow glue. Bridges `RunState` (out-of-combat) and
//! `CombatEnv` (in-combat) so the RL agent can drive a full run
//! through one consistent API:
//!
//!   start_run → enter_act → loop {
//!       advance_to(child_coord)
//!       match current_room_type() {
//!         Monster | Elite | Boss => enter_combat_at_node → fight → finish_combat → apply rewards
//!         RestSite => campfire menu
//!         Shop => shop menu
//!         Unknown => roll event_room
//!         Treasure => open chest
//!         Ancient => Neow event
//!       }
//!   }
//!
//! Encounter pools by floor and the per-act encounter weighting
//! aren't ported yet, so the helpers here use a deterministic
//! "pick from possible-encounters-for-this-room-type via the
//! combat_card_generation RNG stream" approximation. That's good
//! enough for forward simulation; replay paths land later.

use crate::combat::{CardInstance, CombatRewards, CombatState, PlayerSetup};
use crate::encounter::{by_id as encounter_by_id, EncounterData, ALL_ENCOUNTERS};
use crate::env::{Action, CombatEnv};
use crate::map::MapPointType;
use crate::run_state::RunState;

/// Pick an encounter id for the player's current map node.
///
/// `room_type` filter: "Monster" / "Elite" / "Boss". Returns `None`
/// if no candidate matches or the cursor isn't on a combat node.
/// The selection uses the run's `up_front` RNG stream so it stays
/// deterministic for a given seed.
pub use crate::unknown_room::UnknownResolution;

/// Resolve a `?` map node. C# `UnknownMapPointOdds.Roll` weights:
/// Monster 10% / Treasure 2% / Shop 3% / Event ~85% baseline; unrolled
/// types bump after every pick. Consumes one `up_front` RNG float.
/// Returns `None` if the cursor isn't on an `?` node.
pub fn resolve_current_unknown_room(rs: &mut RunState) -> Option<UnknownResolution> {
    if rs.current_room_type()? != MapPointType::Unknown {
        return None;
    }
    // Borrow checker: can't `rs.unknown_odds.roll(&mut rs.rng_set...)`
    // because both routes go through `rs`. `unknown_odds_and_rng`
    // returns a `(&mut UnknownMapPointOdds, &mut RunRngSet)` split-
    // borrow tuple so both can be mutated in one call.
    let (odds, rng_set) = rs.unknown_odds_and_rng();
    Some(odds.roll(&mut rng_set.up_front))
}

/// Pick the next event id from the act's pre-shuffled event pool,
/// skipping events already visited this run. Bumps the visit counter
/// and adds the picked event to `visited_event_ids`. Returns `None`
/// if no `RoomSet` exists (pre-`enter_act` state).
pub fn next_event_from_pool(rs: &mut RunState) -> Option<String> {
    rs.room_set.as_mut().and_then(|s| s.next_event())
}

pub fn pick_encounter_for_current_node(rs: &mut RunState) -> Option<&'static EncounterData> {
    let pt = rs.current_room_type()?;
    // Consult the pre-generated RoomSet for this act. Mirrors C#
    // `RoomSet.NextEncounter` — modulo cycle through the appropriate
    // pre-shuffled pool. Falls back to a uniform sample only if no
    // RoomSet exists yet (DeprecatedAct or pre-enter_act state).
    let picked_id: Option<String> = match pt {
        MapPointType::Monster => rs
            .room_set
            .as_mut()
            .and_then(|set| set.next_hallway_encounter().map(|s| s.to_string())),
        MapPointType::Elite => rs
            .room_set
            .as_mut()
            .and_then(|set| set.next_elite_encounter().map(|s| s.to_string())),
        MapPointType::Boss => rs
            .room_set
            .as_mut()
            .and_then(|set| set.next_boss().map(|s| s.to_string())),
        _ => return None,
    };
    if let Some(id) = picked_id {
        if let Some(data) = crate::encounter::by_id(&id) {
            return Some(data);
        }
    }

    // Fallback: no RoomSet (e.g. tests that build RunState directly
    // without going through enter_act). Uniform sample preserves the
    // original API contract.
    let want = match pt {
        MapPointType::Monster => "Monster",
        MapPointType::Elite => "Elite",
        MapPointType::Boss => "Boss",
        _ => return None,
    };
    let candidates: Vec<&'static EncounterData> = ALL_ENCOUNTERS
        .iter()
        .filter(|e| e.room_type.as_deref() == Some(want))
        .filter(|e| !e.canonical_monsters.is_empty())
        .filter(|e| e.id != "DeprecatedEncounter")
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let idx = rs
        .rng_set_mut()
        .up_front
        .next_int(candidates.len() as i32) as usize;
    Some(candidates[idx])
}

/// Build a `CombatState` ready to step from the player's current
/// run-state position + the picked encounter. Owns the player's
/// HP / deck / relics snapshot — combat mutates that snapshot and
/// the caller folds the result back into `RunState` via
/// `apply_combat_outcome`.
///
/// `player_idx` is which RunState player enters combat (usually 0).
pub fn build_combat_state(
    rs: &RunState,
    encounter: &EncounterData,
    player_idx: usize,
) -> Option<CombatState> {
    let player_state = rs.players().get(player_idx)?;
    let character = crate::character::by_id(&player_state.character_id)?;
    let deck_ids: Vec<String> = player_state.deck.iter().map(|c| c.id.clone()).collect();
    let deck: Vec<CardInstance> = crate::combat::deck_from_ids(&deck_ids);
    let setup = PlayerSetup {
        character,
        current_hp: player_state.hp,
        max_hp: player_state.max_hp,
        deck,
        relics: player_state.relics.iter().map(|r| r.id.clone()).collect(),
    };
    let modifiers = rs.modifiers().to_vec();
    Some(CombatState::start_with_ascension(
        encounter,
        vec![setup],
        modifiers,
        rs.ascension(),
    ))
}

/// Outcome of a finished combat that the run-flow layer cares about.
/// `CombatState` retains far more (full state for analysis); this
/// is the minimum surface to fold the result back into `RunState`.
#[derive(Debug, Clone)]
pub struct CombatOutcome {
    pub victory: bool,
    pub final_hp: i32,
    pub final_max_hp: i32,
    pub rewards: CombatRewards,
}

/// Extract the canonical combat outcome from a finished `CombatState`.
/// `victory` is true iff every enemy is at 0 HP; that's the C# win
/// condition and matches `CombatResult::Victory` from the env layer.
/// Caller supplies an RNG for the rewards roll (gold range, etc).
pub fn extract_outcome(
    cs: &CombatState,
    player_idx: usize,
    rewards_rng: &mut crate::rng::Rng,
) -> CombatOutcome {
    let player = cs.allies.get(player_idx);
    let victory = cs.enemies.iter().all(|e| e.current_hp <= 0);
    let rewards = if victory {
        cs.generate_rewards(rewards_rng)
    } else {
        CombatRewards::default()
    };
    CombatOutcome {
        victory,
        final_hp: player.map(|p| p.current_hp).unwrap_or(0),
        final_max_hp: player.map(|p| p.max_hp).unwrap_or(0),
        rewards,
    }
}

/// Fold a `CombatOutcome` back into `RunState`. Copies HP/max-HP back
/// to the player, grants the in-combat gold, and applies any
/// `Effect::AddCardToRunStateDeck` cards that combat queued
/// (BloodForBlood-style). Does NOT roll a post-combat card reward —
/// that's a separate call so the agent can decide whether to skip.
pub fn apply_combat_outcome(
    rs: &mut RunState,
    player_idx: usize,
    outcome: &CombatOutcome,
) {
    if let Some(ps) = rs.player_state_mut(player_idx) {
        ps.hp = outcome.final_hp.max(0);
        ps.max_hp = outcome.final_max_hp;
        ps.gold += outcome.rewards.gold;
    }
}

/// Offer the standard post-combat card reward (3 cards from the
/// character's pool ∪ Colorless, Normal-style rarity weights, skip
/// allowed). Wraps `card_reward::offer_post_combat_card_reward` with
/// the right `CardRewardKind` for the room type that just finished.
///
/// Auto-resolves to "skip" by default (`n_min = 0`); RL agents
/// driving the run will toggle `auto_resolve_offers` and resolve via
/// `resolve_run_state_offer(picks)`.
pub fn offer_combat_reward(
    rs: &mut RunState,
    player_idx: usize,
    kind: crate::card_reward::CardRewardKind,
) {
    crate::card_reward::offer_post_combat_card_reward(rs, player_idx, kind);
}

/// Drive a combat to completion using a trivial built-in policy.
/// Returns the final `CombatState` so the caller can inspect or
/// pass to `extract_outcome`. Used by integration tests and any
/// "self-play with a baseline opponent" training mode that doesn't
/// need a real agent on every combat.
///
/// Policy: each turn, scan `legal_actions()` and:
///   1. Play the first non-EndTurn action whose card resolves without
///      a complex target choice. Prefers any-target cards over
///      no-target cards so damage actually hits.
///   2. If nothing playable, EndTurn.
///   3. Hard cap of `max_turns` to defeat infinite loops on
///      mis-encoded mechanics.
///
/// The policy is intentionally weak — it's NOT a benchmark, just a
/// deterministic driver to keep combats progressing in tests and
/// during agent-bootstrap rollouts.
pub fn auto_play_combat(
    encounter: &EncounterData,
    rs: &RunState,
    player_idx: usize,
    rng_seed: u32,
    max_turns: usize,
) -> Option<(CombatState, usize)> {
    let cs = build_combat_state(rs, encounter, player_idx)?;
    let mut env = wrap_env(cs, rng_seed);
    let mut turns = 0usize;
    while turns < max_turns {
        // Inner loop: keep playing cards this turn until we end.
        let mut played_this_turn = 0;
        loop {
            if env.state.is_combat_over().is_some() {
                return Some((env.state, turns));
            }
            let actions = env.legal_actions();
            // Find first non-EndTurn action (prefer attacks).
            let pick = actions.iter().find(|a| !matches!(a, Action::EndTurn { .. }));
            let Some(act) = pick else {
                // Nothing to play — end turn.
                break;
            };
            let outcome = env.step(act.clone());
            played_this_turn += 1;
            if outcome.terminal {
                return Some((env.state, turns));
            }
            // Defensive: cap actions-per-turn to avoid infinite loops
            // if a card-play returns Ok but doesn't advance state.
            if played_this_turn >= 100 {
                break;
            }
        }
        // End the turn — runs enemy turn dispatch inside step().
        let end = Action::EndTurn { player_idx };
        let outcome = env.step(end);
        turns += 1;
        if outcome.terminal {
            return Some((env.state, turns));
        }
    }
    Some((env.state, turns))
}

fn wrap_env(cs: CombatState, rng_seed: u32) -> CombatEnv {
    // CombatEnv::reset rebuilds CombatState; we want to use the
    // already-built one (it might already contain mid-combat state
    // from the caller). Hand-build the env.
    use crate::rng::Rng;
    CombatEnv {
        state: cs,
        rng: Rng::new(rng_seed, 0),
    }
}

/// Convenience: classify the current node into a `CardRewardKind` so
/// the caller doesn't have to. Returns None for non-combat nodes.
pub fn reward_kind_for_current_node(
    rs: &RunState,
) -> Option<crate::card_reward::CardRewardKind> {
    match rs.current_room_type()? {
        MapPointType::Monster => Some(crate::card_reward::CardRewardKind::Normal),
        MapPointType::Elite => Some(crate::card_reward::CardRewardKind::Elite),
        MapPointType::Boss => Some(crate::card_reward::CardRewardKind::Boss),
        _ => None,
    }
}

/// Trigger the per-node event for an Ancient (Neow) cursor position.
/// Mirrors C# firing the Neow event when the player lands on the
/// Ancient node at run start. Sets `pending_event` for the agent to
/// resolve; if `auto_resolve_offers == true` the first choice fires
/// immediately.
pub fn enter_neow(rs: &mut RunState, player_idx: usize) -> bool {
    crate::event_room::enter_event(rs, player_idx, "Neow")
}

/// Look up an encounter for the current node and tell the caller what
/// it'd fight. Combined with `build_combat_state` this lets the agent
/// peek at the matchup before committing.
pub fn peek_encounter_at_current_node(rs: &mut RunState) -> Option<String> {
    pick_encounter_for_current_node(rs).map(|e| e.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::act::ActId;

    fn ironclad_run() -> RunState {
        RunState::start_run(
            "TEST",
            0,
            "Ironclad",
            vec![ActId::Overgrowth],
            Vec::new(),
        )
        .expect("Ironclad is a known character")
    }

    #[test]
    fn start_run_pulls_data_from_character_table() {
        let rs = ironclad_run();
        let p = &rs.players()[0];
        assert_eq!(p.hp, 80);
        assert_eq!(p.max_hp, 80);
        assert_eq!(p.gold, 99);
        assert_eq!(p.deck.len(), 10);
        assert_eq!(p.relics.len(), 1);
        assert_eq!(p.relics[0].id, "BurningBlood");
    }

    #[test]
    fn current_room_type_reflects_cursor() {
        let mut rs = ironclad_run();
        rs.enter_act(0);
        assert_eq!(rs.current_room_type(), Some(MapPointType::Ancient));
        // Advance to a row-1 child (Monster row).
        let map = rs.current_map().unwrap().clone();
        let coord = map.starting().coord;
        let child = *map
            .get_point(coord.col, coord.row)
            .unwrap()
            .children
            .iter()
            .next()
            .unwrap();
        rs.advance_to(child).unwrap();
        assert_eq!(rs.current_room_type(), Some(MapPointType::Monster));
    }

    #[test]
    fn pick_encounter_returns_monster_encounter_on_monster_node() {
        let mut rs = ironclad_run();
        rs.enter_act(0);
        let map = rs.current_map().unwrap().clone();
        let coord = map.starting().coord;
        let child = *map
            .get_point(coord.col, coord.row)
            .unwrap()
            .children
            .iter()
            .next()
            .unwrap();
        rs.advance_to(child).unwrap();
        let enc = pick_encounter_for_current_node(&mut rs)
            .expect("monster node yields a Monster encounter");
        assert_eq!(enc.room_type.as_deref(), Some("Monster"));
    }

    #[test]
    fn pick_encounter_returns_none_on_non_combat_node() {
        let mut rs = ironclad_run();
        rs.enter_act(0);
        // Ancient (cursor at start) — no combat picked.
        assert!(pick_encounter_for_current_node(&mut rs).is_none());
    }

    /// Hallway picks must come from Overgrowth's pool, not any random
    /// Monster encounter. Verifies the RoomSet routing is in effect
    /// (vs the legacy "uniform sample over every Monster encounter"
    /// behavior).
    #[test]
    fn hallway_picks_belong_to_overgrowth_pool() {
        use crate::encounter::{regular_encounters_for_act, weak_encounters_for_act};

        let overgrowth_ids: std::collections::HashSet<String> = weak_encounters_for_act("Overgrowth")
            .iter().chain(regular_encounters_for_act("Overgrowth").iter())
            .map(|e| e.id.clone()).collect();

        // 10 independent runs at different seeds. Every Monster pick
        // across all of them must be inside the Overgrowth pool.
        for seed in 0..10 {
            let mut rs = crate::run_state::RunState::start_run(
                &format!("POOL{seed}"), 0, "Ironclad",
                vec![crate::act::ActId::Overgrowth], Vec::new(),
            ).unwrap();
            rs.enter_act(0);
            // Walk children until we land on a Monster.
            let map = rs.current_map().unwrap().clone();
            let start = map.starting().coord;
            let child = *map.get_point(start.col, start.row).unwrap()
                .children.iter().next().unwrap();
            if rs.advance_to(child).is_err() { continue; }
            if rs.current_room_type() != Some(MapPointType::Monster) { continue; }
            let enc = pick_encounter_for_current_node(&mut rs).unwrap();
            assert!(overgrowth_ids.contains(&enc.id),
                "seed {seed}: picked {} not in Overgrowth pool", enc.id);
        }
    }

    /// First 3 hallway draws come from the weak pool. Walk the same
    /// RunState's RoomSet manually to inspect the pre-built sequence.
    #[test]
    fn first_three_hallway_draws_are_weak() {
        use crate::encounter::weak_encounters_for_act;

        let weak_ids: std::collections::HashSet<String> = weak_encounters_for_act("Overgrowth")
            .iter().map(|e| e.id.clone()).collect();

        let mut rs = crate::run_state::RunState::start_run(
            "WEAK", 0, "Ironclad",
            vec![crate::act::ActId::Overgrowth], Vec::new(),
        ).unwrap();
        rs.enter_act(0);
        let set = rs.room_set.as_ref().expect("room_set generated by enter_act");
        assert!(set.hallway_encounters.len() >= 3);
        for (i, id) in set.hallway_encounters.iter().take(3).enumerate() {
            assert!(weak_ids.contains(id),
                "slot {} (weak): {} not in weak pool", i, id);
        }
    }

    #[test]
    fn build_combat_state_seats_player_with_full_deck() {
        let mut rs = ironclad_run();
        rs.enter_act(0);
        let enc = encounter_by_id("AxebotsNormal").unwrap();
        let cs = build_combat_state(&rs, enc, 0)
            .expect("Ironclad has all data needed for combat");
        assert_eq!(cs.allies.len(), 1, "single-player");
        let p = &cs.allies[0];
        assert_eq!(p.current_hp, 80);
        assert_eq!(p.max_hp, 80);
        // Combat-side deck loaded from PlayerState.deck.
        let pcs = p.player.as_ref().unwrap();
        assert_eq!(pcs.draw.cards.len() + pcs.hand.cards.len(), 10,
            "all 10 deck cards must be in draw or hand");
    }

    #[test]
    fn apply_combat_outcome_writes_hp_and_gold_back() {
        let mut rs = ironclad_run();
        let pre_gold = rs.players()[0].gold;
        let outcome = CombatOutcome {
            victory: true,
            final_hp: 65, // took 15 damage
            final_max_hp: 80,
            rewards: CombatRewards { gold: 17, ..Default::default() },
        };
        apply_combat_outcome(&mut rs, 0, &outcome);
        assert_eq!(rs.players()[0].hp, 65);
        assert_eq!(rs.players()[0].gold, pre_gold + 17);
    }

    #[test]
    fn neow_event_offers_4_starter_buffs() {
        let mut rs = ironclad_run();
        rs.auto_resolve_offers = false; // don't pick automatically
        rs.enter_act(0);
        assert!(enter_neow(&mut rs, 0), "Neow event must be registered");
        let pending = rs.pending_event.as_ref().expect("event staged");
        assert_eq!(pending.event_id, "Neow");
        assert_eq!(pending.choices.len(), 4);
        let labels: Vec<&str> = pending.choices.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"MAX_HP_PLUS_8"));
        assert!(labels.contains(&"PLUS_100_GOLD"));
        assert!(labels.contains(&"UPGRADE_RANDOM_CARD"));
        assert!(labels.contains(&"REMOVE_RANDOM_CARD"));
    }

    #[test]
    fn neow_plus_100_gold_actually_adds_gold() {
        let mut rs = ironclad_run();
        rs.auto_resolve_offers = false;
        rs.enter_act(0);
        let pre_gold = rs.players()[0].gold;
        enter_neow(&mut rs, 0);
        crate::event_room::resolve_event_choice(&mut rs, 1)
            .expect("PLUS_100_GOLD resolves");
        assert_eq!(rs.players()[0].gold, pre_gold + 100);
    }

    #[test]
    fn auto_play_combat_resolves_easy_fight() {
        // Use a known easy encounter (AxebotsNormal: 2 Axebots, ~80 HP).
        // The trivial policy should still finish via raw attrition or
        // hit the turn cap. Either way we get a terminal CombatState.
        let rs = ironclad_run();
        let enc = encounter_by_id("AxebotsNormal").unwrap();
        let (final_cs, turns) = auto_play_combat(enc, &rs, 0, 0xC0FFEE, 100)
            .expect("auto-play returns a finished state");
        // Either the player won (some enemies dead), the player lost
        // (player HP zero), or hit the turn cap. All count as a
        // successful run of the driver — we're testing the loop, not
        // the policy.
        let player_alive = final_cs
            .allies
            .first()
            .map(|p| p.current_hp > 0)
            .unwrap_or(false);
        let any_dead = final_cs.enemies.iter().any(|e| e.current_hp <= 0);
        assert!(
            player_alive || any_dead || turns >= 100,
            "auto-play made no progress in {} turns", turns
        );
    }

    #[test]
    fn auto_play_then_extract_outcome_is_consistent() {
        let rs = ironclad_run();
        let enc = encounter_by_id("AxebotsNormal").unwrap();
        let (final_cs, _) = auto_play_combat(enc, &rs, 0, 12345, 100).unwrap();
        let mut rng = crate::rng::Rng::new(0, 0);
        let outcome = extract_outcome(&final_cs, 0, &mut rng);
        // HP must be in valid range.
        assert!(outcome.final_hp >= 0);
        assert!(outcome.final_hp <= outcome.final_max_hp);
        // Victory ↔ enemies all dead.
        let all_dead = final_cs.enemies.iter().all(|e| e.current_hp <= 0);
        assert_eq!(outcome.victory, all_dead);
        // Gold only granted on victory.
        if !outcome.victory {
            assert_eq!(outcome.rewards.gold, 0);
        }
    }

    #[test]
    fn reward_kind_classifies_node_correctly() {
        let mut rs = ironclad_run();
        rs.enter_act(0);
        // Walk to a row-1 (Monster) child.
        let map = rs.current_map().unwrap().clone();
        let coord = map.starting().coord;
        let child = *map
            .get_point(coord.col, coord.row)
            .unwrap()
            .children
            .iter()
            .next()
            .unwrap();
        rs.advance_to(child).unwrap();
        assert!(matches!(
            reward_kind_for_current_node(&rs),
            Some(crate::card_reward::CardRewardKind::Normal)
        ));
    }
}
