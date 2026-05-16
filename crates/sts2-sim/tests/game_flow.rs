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

    // Step 5: simulate "we won" by zeroing every enemy. (Driving a
    // real combat is the env's job — we just want to verify the
    // outcome→runstate fold.)
    let mut cs = cs;
    for e in cs.enemies.iter_mut() {
        e.current_hp = 0;
    }
    let mut rng = Rng::new(0xC0FFEE, 0);
    let outcome = extract_outcome(&cs, 0, &mut rng);
    assert!(outcome.victory);
    assert!(outcome.rewards.gold >= 10 && outcome.rewards.gold <= 20,
        "Monster reward gold in [10,20]: got {}", outcome.rewards.gold);

    // Step 6: fold combat outcome back into RunState.
    let pre_gold = rs.players()[0].gold;
    apply_combat_outcome(&mut rs, 0, &outcome);
    assert_eq!(rs.players()[0].gold, pre_gold + outcome.rewards.gold);

    // Step 7: offer post-combat card reward — should stage 3 options.
    let kind = reward_kind_for_current_node(&rs).unwrap();
    assert!(matches!(kind, CardRewardKind::Normal));
    offer_combat_reward(&mut rs, 0, kind);
    let pending = rs.pending_offer.as_ref().expect("card reward offered");
    assert_eq!(pending.options.len(), 3, "3-card post-combat reward");

    // Step 8: skip the reward (n_min = 0) and advance to the next node.
    sts2_sim::effects::resolve_run_state_offer(&mut rs, &[])
        .expect("skip resolves");
    let map = rs.current_map().unwrap().clone();
    let cur_coord = rs.current_map_coord().unwrap();
    let cur_pt = map.get_point(cur_coord.col, cur_coord.row).unwrap();
    let next = *cur_pt.children.iter().next().expect("row-1 has children");
    rs.advance_to(next).expect("advance to row 2");
    assert_eq!(rs.act_floor(), 2);
}
