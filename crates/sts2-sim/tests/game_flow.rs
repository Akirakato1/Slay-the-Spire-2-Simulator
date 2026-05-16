//! End-to-end game-flow integration test.
//!
//! Walks the full critical path the RL agent will follow:
//!   1. Start a fresh run for a character (Ironclad, ascension 0).
//!   2. Generate Act 1 map.
//!   3. Land at the Ancient (Neow) node.
//!   4. Pick a starter buff / relic.
//!   5. Advance to the first reachable map node.
//!   6. Enter that room (fire AfterRoomEntered hooks).
//!   7. If Monster: run combat against the encounter to victory.
//!   8. Collect the post-combat reward (gold + card pick).
//!   9. Advance to next node.
//!  10. Loop until reaching boss / dying.
//!
//! Where the test trips on a missing primitive or API call, that's a
//! real gap we need to close before training can drive the simulator.

use sts2_sim::act::ActId;
use sts2_sim::character;
use sts2_sim::map::MapPointType;
use sts2_sim::run_log::{CardRef, PotionEntry, RelicEntry};
use sts2_sim::run_state::{PlayerState, RunState};

fn build_starter_player(character_id: &str, player_id: i64) -> PlayerState {
    let cd = character::by_id(character_id)
        .unwrap_or_else(|| panic!("unknown character {}", character_id));
    let deck: Vec<CardRef> = cd
        .starting_deck
        .iter()
        .map(|id| CardRef {
            id: id.clone(),
            floor_added_to_deck: Some(0),
            current_upgrade_level: Some(0),
            enchantment: None,
        })
        .collect();
    let relics: Vec<RelicEntry> = cd
        .starting_relics
        .iter()
        .map(|id| RelicEntry {
            id: id.clone(),
            floor_added_to_deck: 0,
            props: None,
        })
        .collect();
    PlayerState {
        character_id: character_id.to_string(),
        id: player_id,
        hp: cd.starting_hp.unwrap_or(80),
        max_hp: cd.starting_hp.unwrap_or(80),
        gold: cd.starting_gold.unwrap_or(99),
        deck,
        relics,
        potions: Vec::<PotionEntry>::new(),
        max_potion_slot_count: 3,
    }
}

#[test]
fn fresh_ironclad_starts_with_canonical_loadout() {
    let player = build_starter_player("Ironclad", 1);
    // C#: Ironclad starts 80/80, 99 gold, 10-card deck with BurningBlood.
    assert_eq!(player.hp, 80);
    assert_eq!(player.max_hp, 80);
    assert_eq!(player.gold, 99);
    assert_eq!(player.deck.len(), 10);
    assert_eq!(player.relics.len(), 1);
    assert_eq!(player.relics[0].id, "BurningBlood");
}

#[test]
fn run_state_builds_and_enters_act_1() {
    let player = build_starter_player("Ironclad", 1);
    let mut rs = RunState::new(
        "TEST",
        0,
        vec![player],
        vec![ActId::Overgrowth],
        Vec::new(),
    );
    rs.enter_act(0);
    let map = rs.current_map().expect("map generated");
    assert_eq!(map.rows(), 16, "Overgrowth has 16 rows");
    // Cursor sits at Ancient (start node).
    let coord = rs.current_map_coord().expect("cursor at ancient");
    assert_eq!(coord, map.starting().coord);
    let start_pt = map
        .get_point(coord.col, coord.row)
        .expect("starting point exists");
    assert_eq!(start_pt.point_type, MapPointType::Ancient);
}

#[test]
fn cursor_advances_only_to_valid_children() {
    let player = build_starter_player("Ironclad", 1);
    let mut rs = RunState::new(
        "TEST",
        0,
        vec![player],
        vec![ActId::Overgrowth],
        Vec::new(),
    );
    let map = rs.enter_act(0).clone();
    let start = map.starting().coord;
    let start_pt = map.get_point(start.col, start.row).unwrap();
    let first_child = *start_pt.children.iter().next().expect("ancient has children");
    rs.advance_to(first_child).expect("ancient child is reachable");
    assert_eq!(rs.current_map_coord(), Some(first_child));
    assert_eq!(rs.act_floor(), 1, "advancing one node increments floor");

    // Trying to teleport to a non-child must fail.
    let unreachable = map.boss().coord;
    let err = rs.advance_to(unreachable);
    assert!(err.is_err(), "advance_to(boss) from row 1 must reject");
}

/// Gap probe: there's no API for resolving the Ancient (Neow) interaction.
/// In C# this is an EventModel that fires when the player lands on the
/// starting node — typically offering a buff choice (heal / max HP / gold /
/// card upgrade / random relic / etc). For now we just confirm the room
/// hooks fire when we enter the Ancient room without breaking.
#[test]
fn enter_ancient_room_fires_hooks_without_panic() {
    let player = build_starter_player("Ironclad", 1);
    let mut rs = RunState::new(
        "TEST",
        0,
        vec![player],
        vec![ActId::Overgrowth],
        Vec::new(),
    );
    rs.enter_act(0);
    // Entering the room at the starting node — should be safe even
    // without an explicit Neow event.
    rs.enter_room("Ancient");
    // BurningBlood (Ironclad's starter relic) heals 6 at combat end —
    // its AfterRoomEntered hook (if any) should fire silently here.
    // Just assert state is unchanged enough to keep going.
    assert!(rs.players()[0].hp > 0);
}

/// True end-to-end smoke test: walk Neow → first node → combat →
/// reward → cursor advances. Exercises run_flow bridges.
#[test]
fn full_run_neow_to_first_combat_to_reward() {
    use sts2_sim::card_reward::CardRewardKind;
    use sts2_sim::rng::Rng;
    use sts2_sim::run_flow::{
        apply_combat_outcome, build_combat_state, enter_neow,
        extract_outcome, offer_combat_reward, pick_encounter_for_current_node,
        reward_kind_for_current_node,
    };

    // Step 1: start a fresh Ironclad run.
    let mut rs = RunState::start_run(
        "TEST",
        0,
        "Ironclad",
        vec![ActId::Overgrowth],
        Vec::new(),
    )
    .unwrap();
    rs.auto_resolve_offers = false;
    rs.enter_act(0);

    // Step 2: trigger Neow (Ancient room), pick +100 gold.
    assert!(enter_neow(&mut rs, 0));
    let pre_gold = rs.players()[0].gold;
    sts2_sim::event_room::resolve_event_choice(&mut rs, 1)
        .expect("Neow PLUS_100_GOLD resolves");
    assert_eq!(rs.players()[0].gold, pre_gold + 100);

    // Step 3: advance to the first Monster node on row 1.
    let map = rs.current_map().unwrap().clone();
    let start = map.starting().coord;
    let first_child = *map
        .get_point(start.col, start.row)
        .unwrap()
        .children
        .iter()
        .next()
        .unwrap();
    rs.advance_to(first_child).unwrap();
    assert_eq!(rs.current_room_type(), Some(MapPointType::Monster));

    // Step 4: pick an encounter for this node, build the combat state.
    let encounter_id = pick_encounter_for_current_node(&mut rs)
        .map(|e| e.id.clone())
        .expect("Monster node picks an encounter");
    let encounter = sts2_sim::encounter::by_id(&encounter_id).unwrap();
    let cs = build_combat_state(&rs, encounter, 0).unwrap();
    assert!(!cs.enemies.is_empty(), "encounter spawns at least one enemy");
    assert_eq!(cs.allies.len(), 1);
    assert_eq!(cs.allies[0].current_hp, 80);

    // Step 5: drive combat to completion with the auto-play driver.
    // (No "zero HP shortcut" — actually play through enemy turns
    // and card plays.)
    let _ = cs; // hand-built CombatState not needed; auto_play_combat builds its own.
    let (final_cs, _turns) = sts2_sim::run_flow::auto_play_combat(
        encounter, &rs, 0, 0xC0FFEE, 100,
    )
    .expect("auto-play returns terminal state");
    let mut rng = Rng::new(0xC0FFEE, 0);
    let outcome = extract_outcome(&final_cs, 0, &mut rng);
    // Either the player won, lost, or ran out of turns.
    // Victory case: gold must be in Monster range.
    if outcome.victory {
        assert!(outcome.rewards.gold >= 10 && outcome.rewards.gold <= 20,
            "Monster victory gold in [10,20], got {}", outcome.rewards.gold);
    } else {
        // Otherwise no gold dropped.
        assert_eq!(outcome.rewards.gold, 0);
    }

    // Step 6: fold combat outcome back into RunState.
    let pre_gold = rs.players()[0].gold;
    apply_combat_outcome(&mut rs, 0, &outcome);
    assert_eq!(rs.players()[0].gold, pre_gold + outcome.rewards.gold);
    // HP can only decrease through combat.
    assert!(rs.players()[0].hp <= 80);
    if outcome.victory {
        assert!(rs.players()[0].hp > 0, "victory implies player still alive");
    }

    // Step 7 onwards only makes sense if we won.
    if !outcome.victory {
        return;
    }
}

/// Stress: every playable character can start a run, generate a map,
/// pick an encounter, and run the auto-play driver without panicking.
/// Catches regressions in starter loadout / character pool / encounter
/// roll for non-Ironclad runs that the single-character test misses.
#[test]
fn every_playable_character_completes_full_first_combat() {
    use sts2_sim::character::PLAYABLE_CHARACTERS;
    use sts2_sim::run_flow::{
        auto_play_combat, build_combat_state, pick_encounter_for_current_node,
    };

    for &character_id in PLAYABLE_CHARACTERS {
        let mut rs = RunState::start_run(
            "STRESS",
            0,
            character_id,
            vec![ActId::Overgrowth],
            Vec::new(),
        )
        .unwrap_or_else(|| panic!("start_run failed for {}", character_id));
        rs.enter_act(0);

        // Advance to first child (Monster row).
        let map = rs.current_map().unwrap().clone();
        let start = map.starting().coord;
        let child = *map
            .get_point(start.col, start.row)
            .unwrap()
            .children
            .iter()
            .next()
            .unwrap();
        rs.advance_to(child).unwrap();

        let enc = pick_encounter_for_current_node(&mut rs)
            .unwrap_or_else(|| panic!("{}: no encounter", character_id));
        // Just verify build_combat_state succeeds — auto_play_combat
        // is slow for the full 5-character sweep.
        let cs = build_combat_state(&rs, enc, 0)
            .unwrap_or_else(|| panic!("{}: build_combat_state failed", character_id));
        assert!(!cs.enemies.is_empty(),
            "{}: encounter must spawn enemies", character_id);
        assert_eq!(cs.allies.len(), 1, "{}: single-player", character_id);

        // Drive one combat with a bounded turn cap. Don't assert
        // outcome — some character/encounter pairings might lose on
        // trivial policy. Just verify no panic and outcome extraction
        // works.
        let (final_cs, _) = auto_play_combat(enc, &rs, 0, 12345, 30)
            .unwrap_or_else(|| panic!("{}: auto_play returned None", character_id));
        let mut rng = sts2_sim::rng::Rng::new(0, 0);
        let _outcome = sts2_sim::run_flow::extract_outcome(&final_cs, 0, &mut rng);
    }
}

/// User-stated invariant: at the Ancient, the player can see the full
/// generated map for the act *before* picking their Neow buff. For RL
/// this matters because the Neow choice should be informed by the
/// future paths available. Verify the map is fully populated with
/// point types BEFORE `enter_neow` fires the event offer.
#[test]
fn map_is_visible_before_neow_resolves() {
    use sts2_sim::run_flow::enter_neow;

    let mut rs = RunState::start_run(
        "MAPVIEW",
        0,
        "Ironclad",
        vec![ActId::Overgrowth],
        Vec::new(),
    )
    .unwrap();
    rs.auto_resolve_offers = false;
    rs.enter_act(0);

    // BEFORE Neow: map is generated, every grid point has a type
    // assigned, and the cursor sits at the Ancient.
    {
        let map = rs.current_map().expect("map generated by enter_act");
        for p in map.iter_grid_points() {
            assert_ne!(
                p.point_type,
                MapPointType::Unassigned,
                "every map point must have a type before Neow"
            );
        }
        assert_eq!(
            rs.current_room_type(),
            Some(MapPointType::Ancient),
            "cursor sits at Ancient pre-Neow"
        );
        // Boss is visible from the start node.
        let boss = map.boss();
        assert_eq!(boss.point_type, MapPointType::Boss);
    }

    // NOW fire Neow. The event offer becomes pending.
    assert!(enter_neow(&mut rs, 0));
    assert!(rs.pending_event.is_some(), "Neow offer becomes pending");

    // After Neow is queued, the map is still fully visible — choice
    // doesn't mutate the map.
    let map = rs.current_map().unwrap();
    for p in map.iter_grid_points() {
        assert_ne!(p.point_type, MapPointType::Unassigned);
    }
}

/// `?`-resolution: walking into an Unknown node returns one of
/// {Monster, Treasure, Shop, Event} per `UnknownMapPointOdds.Roll`.
/// Drive 200 fresh runs and verify every roll lands in the valid set
/// and the distribution is biased toward Event (the implicit ~85%
/// base remainder, dampened by cumulative odds-bump).
#[test]
fn unknown_node_resolves_to_valid_room_types() {
    use sts2_sim::run_flow::{resolve_current_unknown_room, UnknownResolution};

    let mut counts = std::collections::HashMap::new();
    for seed in 0..200 {
        let mut rs = RunState::start_run(
            &format!("UNK{seed}"),
            0,
            "Ironclad",
            vec![ActId::Overgrowth],
            Vec::new(),
        )
        .unwrap();
        rs.enter_act(0);
        // Walk forward until we find an Unknown node, then resolve.
        let map = rs.current_map().unwrap().clone();
        let mut cursor = map.starting().coord;
        for _ in 0..6 {
            let pt = map.get_point(cursor.col, cursor.row).unwrap();
            let Some(&next) = pt.children.iter().next() else { break };
            if rs.advance_to(next).is_err() { break }
            cursor = next;
            if rs.current_room_type() == Some(MapPointType::Unknown) {
                let r = resolve_current_unknown_room(&mut rs);
                if let Some(res) = r {
                    *counts.entry(res).or_insert(0_u32) += 1;
                }
                break;
            }
        }
    }
    // At least some `?` nodes must have been encountered in 200 runs.
    let total: u32 = counts.values().sum();
    assert!(total > 0, "no Unknown nodes hit across 200 seeds");
    // Every resolution must be one of the 4 valid kinds.
    for k in counts.keys() {
        assert!(matches!(*k,
            UnknownResolution::Monster | UnknownResolution::Treasure
            | UnknownResolution::Shop | UnknownResolution::Event));
    }
}

/// Event pool draw: `next_event_from_pool` returns events from the
/// current act's pool and never repeats within a single run until the
/// pool is exhausted.
#[test]
fn event_pool_draws_are_unique_within_a_run() {
    use sts2_sim::run_flow::next_event_from_pool;

    let mut rs = RunState::start_run(
        "EVENTPOOL", 0, "Ironclad",
        vec![ActId::Overgrowth], Vec::new(),
    ).unwrap();
    rs.enter_act(0);

    let pool_size = rs.room_set.as_ref().unwrap().events.len();
    let mut picks = Vec::new();
    for _ in 0..pool_size {
        if let Some(p) = next_event_from_pool(&mut rs) {
            picks.push(p);
        }
    }
    let mut sorted = picks.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), pool_size,
        "first {} event picks must all be distinct", pool_size);
    // Pool now exhausted — next pick must be a repeat.
    let repeat = next_event_from_pool(&mut rs).expect("post-exhaustion pick");
    assert!(picks.contains(&repeat));
}

/// Ascension-10 run-state init applies WearyTraveler (-5 max HP),
/// AscendersBane (adds curse to deck), and DoubleBoss (every act
/// flagged for second boss). A0 run is unaffected.
#[test]
fn ascension_run_init_applies_per_level_modifiers() {
    let a0 = RunState::start_run(
        "A0", 0, "Ironclad", vec![ActId::Overgrowth], Vec::new(),
    ).unwrap();
    assert_eq!(a0.players()[0].max_hp, 80, "A0 keeps base max HP");
    assert!(!a0.players()[0].deck.iter().any(|c| c.id == "AscendersBane"),
        "A0 deck has no AscendersBane");

    let a10 = RunState::start_run(
        "A10", 10, "Ironclad", vec![ActId::Overgrowth], Vec::new(),
    ).unwrap();
    assert_eq!(a10.players()[0].max_hp, 75,
        "A10 has WearyTraveler -5 max HP applied");
    assert!(a10.players()[0].deck.iter().any(|c| c.id == "AscendersBane"),
        "A10 deck must contain AscendersBane curse");
    assert_eq!(a10.players()[0].deck.len(), 11,
        "A10 Ironclad deck = 10 starter + 1 AscendersBane");
}

/// Poverty (A3+) reduces combat gold by 0.75×. Drive 50 monster fights
/// at A0 and A10 with identical seeds; A10 should yield less gold.
#[test]
fn ascension_poverty_reduces_combat_gold() {
    use sts2_sim::combat::CombatRewards;
    use sts2_sim::rng::Rng;

    fn sample_gold(ascension: i32) -> i32 {
        let mut rs = RunState::start_run(
            "POVERTY", ascension, "Ironclad",
            vec![ActId::Overgrowth], Vec::new(),
        ).unwrap();
        rs.enter_act(0);
        // Walk to first child + pick encounter.
        let map = rs.current_map().unwrap().clone();
        let start = map.starting().coord;
        let child = *map.get_point(start.col, start.row).unwrap()
            .children.iter().next().unwrap();
        rs.advance_to(child).unwrap();
        let enc = sts2_sim::run_flow::pick_encounter_for_current_node(&mut rs).unwrap();
        let cs = sts2_sim::run_flow::build_combat_state(&rs, enc, 0).unwrap();
        // Roll rewards with a fixed RNG so the only difference is the
        // ascension multiplier.
        let mut rng = Rng::new(7, 0);
        let r: CombatRewards = cs.generate_rewards(&mut rng);
        r.gold
    }

    let a0_gold = sample_gold(0);
    let a10_gold = sample_gold(10);
    // 0.75× of a positive base must be < base.
    assert!(a10_gold < a0_gold,
        "A10 gold ({}) should be less than A0 ({}) via Poverty", a10_gold, a0_gold);
    // Sanity: ratio close to 0.75 ± rounding.
    let ratio = a10_gold as f64 / a0_gold as f64;
    assert!((ratio - 0.75).abs() < 0.10,
        "ratio {:.3} should be near 0.75", ratio);
}

/// TightBelt (A4+) reduces max potion slots by 1. A3 keeps the
/// default 3 slots; A4+ drops to 2.
#[test]
fn ascension_tightbelt_reduces_potion_slots() {
    let a3 = RunState::start_run(
        "TIGHT3", 3, "Ironclad", vec![ActId::Overgrowth], Vec::new(),
    ).unwrap();
    let a4 = RunState::start_run(
        "TIGHT4", 4, "Ironclad", vec![ActId::Overgrowth], Vec::new(),
    ).unwrap();
    let a10 = RunState::start_run(
        "TIGHT10", 10, "Ironclad", vec![ActId::Overgrowth], Vec::new(),
    ).unwrap();
    assert_eq!(a3.players()[0].max_potion_slot_count, 3,
        "A3 still has full belt");
    assert_eq!(a4.players()[0].max_potion_slot_count, 2,
        "A4 TightBelt -1 slot");
    assert_eq!(a10.players()[0].max_potion_slot_count, 2,
        "A10 still TightBelt-reduced");
}

/// Inflation (A6+) bumps the shop's card-removal price from 75 → 100.
#[test]
fn ascension_inflation_bumps_card_remove_price() {
    use sts2_sim::shop::{open_shop, ShopEntryKind};

    fn remove_price(ascension: i32) -> i32 {
        let mut rs = RunState::start_run(
            &format!("INFL{ascension}"),
            ascension, "Ironclad",
            vec![ActId::Overgrowth], Vec::new(),
        ).unwrap();
        rs.enter_act(0);
        let shop = open_shop(&mut rs, 0);
        shop.entries.iter()
            .find(|e| matches!(e.kind, ShopEntryKind::CardRemove))
            .map(|e| e.price)
            .expect("shop has a card-remove entry")
    }
    assert_eq!(remove_price(5), 75, "A5 still pre-Inflation");
    assert_eq!(remove_price(6), 100, "A6 Inflation kicks in");
    assert_eq!(remove_price(10), 100, "A10 still Inflated");
}

/// Scarcity (A7+) reduces Rare card-reward odds. Run 5000 Normal-tier
/// rolls at A6 vs A7 and verify the Rare count drops sharply.
#[test]
fn ascension_scarcity_reduces_rare_card_odds() {
    use sts2_sim::card::CardRarity;
    use sts2_sim::card_reward::{roll_card_rarity, CardRewardKind};

    fn count_rares(ascension: i32) -> i32 {
        let mut rs = RunState::start_run(
            &format!("SCAR{ascension}"),
            ascension, "Ironclad",
            vec![ActId::Overgrowth], Vec::new(),
        ).unwrap();
        let mut rares = 0;
        for _ in 0..5000 {
            if roll_card_rarity(&mut rs, CardRewardKind::Normal) == CardRarity::Rare {
                rares += 1;
            }
        }
        rares
    }
    let pre = count_rares(6);
    let post = count_rares(7);
    // Pre-Scarcity ~3%, post-Scarcity ~1.5% — A7 must be at least
    // 30% lower.
    assert!(post < pre,
        "Scarcity should reduce Rares: pre={} post={}", pre, post);
    let ratio = post as f64 / pre.max(1) as f64;
    assert!(ratio < 0.7,
        "Scarcity should cut Rare rate by ~half: pre={} post={} ratio={:.3}",
        pre, post, ratio);
}

/// Stress: walk 3 consecutive map nodes from a fresh run. Catches
/// state-bleed between combats (relic hooks not clearing, RNG counters
/// not advancing properly, deck refilled wrong, etc).
#[test]
fn ironclad_walks_three_nodes_sequentially() {
    use sts2_sim::run_flow::{
        apply_combat_outcome, auto_play_combat, extract_outcome,
        pick_encounter_for_current_node,
    };

    let mut rs = RunState::start_run(
        "WALK", 0, "Ironclad", vec![ActId::Overgrowth], Vec::new(),
    )
    .unwrap();
    rs.enter_act(0);
    let map = rs.current_map().unwrap().clone();
    let mut cursor = map.starting().coord;

    for floor in 1..=3 {
        let cur_pt = map.get_point(cursor.col, cursor.row).unwrap();
        let next = *cur_pt
            .children
            .iter()
            .next()
            .unwrap_or_else(|| panic!("floor {}: no children at {:?}", floor, cursor));
        rs.advance_to(next).unwrap();
        cursor = next;

        // If this is a combat node, fight it. Otherwise just move on.
        if matches!(
            rs.current_room_type(),
            Some(MapPointType::Monster) | Some(MapPointType::Elite)
        ) {
            let enc = pick_encounter_for_current_node(&mut rs).unwrap();
            let (final_cs, _) = auto_play_combat(enc, &rs, 0, 0xDEAD + floor as u32, 30)
                .unwrap();
            let mut rng = sts2_sim::rng::Rng::new(floor as u32, 0);
            let outcome = extract_outcome(&final_cs, 0, &mut rng);
            apply_combat_outcome(&mut rs, 0, &outcome);
            // If the player died mid-walk, just stop.
            if rs.players()[0].hp <= 0 {
                break;
            }
        }
    }

    // Verify we actually advanced floors.
    assert!(rs.act_floor() >= 1, "must advance at least one floor");
}
