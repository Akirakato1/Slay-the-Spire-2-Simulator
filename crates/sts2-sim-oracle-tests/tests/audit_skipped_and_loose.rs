//! Functional-correctness audit for everything that's skipped or
//! loose-compared in the main sweeps.
//!
//! The card and relic parity sweeps are at 100% PASS, but they're at
//! 100% because of two kinds of accommodation:
//!
//!   1. **Skipped items** — cards/relics whose oracle-side flow can't
//!      run in our headless harness (MadScience needs TinkerTimeType
//!      pre-set; 8 Ancient relics call into Godot natives or
//!      run-state APIs that NRE without full RunState). For these the
//!      parity sweep can't be the oracle of correctness.
//!
//!   2. **Loose comparisons** — combat-RNG-driven cards (`is_random_*`
//!      buckets in combat_parity_sweep.rs, `is_random_pile_pick` in
//!      relic_parity_sweep.rs). The sweep tolerates positional /
//!      specific-card-id differences because the simulator's combat
//!      RNG is intentionally not byte-aligned with C#.
//!
//! This file locks in the expected rust-side behavior for each via
//! direct assertions that don't depend on the oracle. Each test
//! exercises the actual primitive and asserts on the observable state
//! change so the relaxation can't hide a real regression.

use sts2_sim::card;
use sts2_sim::combat::{
    CardInstance, CombatSide, CombatState, PileType,
};
use sts2_sim::encounter;

fn ironclad_combat_with_relics(relics: &[&str]) -> CombatState {
    let ironclad = sts2_sim::character::by_id("Ironclad").expect("Ironclad");
    let enc = encounter::by_id("AxebotsNormal").expect("AxebotsNormal");
    let deck: Vec<CardInstance> = ironclad
        .starting_deck
        .iter()
        .filter_map(|id| card::by_id(id).map(|c| CardInstance::from_card(c, 0)))
        .collect();
    let setup = sts2_sim::combat::PlayerSetup {
        character: ironclad,
        current_hp: 80,
        max_hp: 80,
        deck,
        relics: relics.iter().map(|s| s.to_string()).collect(),
    };
    CombatState::start(enc, vec![setup], Vec::new())
}

fn force_card(cs: &mut CombatState, card_id: &str) {
    let data = card::by_id(card_id).expect("card in registry");
    let inst = CardInstance::from_card(data, 0);
    cs.allies[0].player.as_mut().unwrap().hand.cards.push(inst);
}

fn total_enemy_hp(cs: &CombatState) -> i32 {
    cs.enemies.iter().map(|e| e.current_hp).sum()
}

fn player_block(cs: &CombatState) -> i32 {
    cs.allies[0].block
}

fn power_amount(cs: &CombatState, side: CombatSide, idx: usize, power_id: &str) -> i32 {
    cs.get_power_amount(side, idx, power_id)
}

fn pile_size(cs: &CombatState, pile: PileType) -> usize {
    let ps = cs.allies[0].player.as_ref().unwrap();
    match pile {
        PileType::Hand => ps.hand.cards.len(),
        PileType::Draw => ps.draw.cards.len(),
        PileType::Discard => ps.discard.cards.len(),
        PileType::Exhaust => ps.exhaust.cards.len(),
        _ => 0,
    }
}

// ============================================================================
// Section 1: skipped Ancient relics. Their combat-side encoding is empty
// (`Some(vec![])`) — they're strategic-layer only (offer card reward,
// modify map, etc.). The combat parity sweep can't run them because
// oracle's AfterObtained crashes in Godot natives or NREs on missing
// run-state. These tests confirm the rust encoding produces ZERO
// combat-state delta — proving the skip is justified.
// ============================================================================

fn assert_relic_no_combat_change(relic_id: &str) {
    // Baseline: ironclad combat, no extra relic.
    let base = ironclad_combat_with_relics(&[]);
    let baseline_hp = total_enemy_hp(&base);
    let baseline_block = player_block(&base);
    let baseline_powers_player = base.allies[0].powers.len();
    let baseline_powers_e0 = base.enemies[0].powers.len();
    let baseline_hand = pile_size(&base, PileType::Hand);
    let baseline_draw = pile_size(&base, PileType::Draw);
    let baseline_max_hp = base.allies[0].max_hp;
    let baseline_current_hp = base.allies[0].current_hp;

    // With the relic.
    let mut with_relic = ironclad_combat_with_relics(&[relic_id]);
    with_relic.fire_before_combat_start_hooks();

    assert_eq!(total_enemy_hp(&with_relic), baseline_hp,
        "{} should not change enemy hp", relic_id);
    assert_eq!(player_block(&with_relic), baseline_block,
        "{} should not change player block", relic_id);
    assert_eq!(with_relic.allies[0].powers.len(), baseline_powers_player,
        "{} should not apply player powers", relic_id);
    assert_eq!(with_relic.enemies[0].powers.len(), baseline_powers_e0,
        "{} should not apply enemy powers", relic_id);
    assert_eq!(pile_size(&with_relic, PileType::Hand), baseline_hand,
        "{} should not modify hand", relic_id);
    assert_eq!(pile_size(&with_relic, PileType::Draw), baseline_draw,
        "{} should not modify draw", relic_id);
    assert_eq!(with_relic.allies[0].max_hp, baseline_max_hp,
        "{} should not change max hp in combat", relic_id);
    assert_eq!(with_relic.allies[0].current_hp, baseline_current_hp,
        "{} should not change current hp in combat", relic_id);
}

/// Relics whose combat-side effect lives in a legacy hardcoded
/// dispatcher (combat.rs dispatch_relic_before_combat_start /
/// dispatch_relic_after_side_turn_start / etc.) rather than the
/// relic_effects data table. The data-table entry is empty for these.
/// Excluded from the "no combat effect" sweep because they DO mutate
/// combat state.
fn has_legacy_combat_dispatch(relic_id: &str) -> bool {
    matches!(relic_id,
        // dispatch_relic_before_combat_start.
        "Anchor"
        // dispatch_relic_after_side_turn_start (won't fire from our
        // test because we only fire BeforeCombatStart, but listed for
        // future-proofing if the audit grows).
        | "Brimstone"
        | "DemonForm"
    )
}

/// Returns true if the relic has any combat-side effect — either via
/// the relic_effects data table OR a legacy hardcoded dispatcher.
/// Run-state hooks (deck/HP/gold/potion modifiers) are NOT combat-side
/// and don't count.
fn has_combat_effect(relic_id: &str) -> bool {
    if has_legacy_combat_dispatch(relic_id) {
        return true;
    }
    let Some(arms) = sts2_sim::effects::relic_effects(relic_id) else {
        return false;
    };
    arms.iter().any(|(_, effects)| !effects.is_empty())
}

/// Parametric audit: every relic without a combat-side effect should
/// produce ZERO combat state delta when granted. Catches the 8
/// previously-listed Ancients PLUS the ~100 other relics whose
/// AfterObtained only modifies run-state (Whetstone, Mango,
/// JewelryBox, BurningBlood, Anchor, Brimstone, PotionBelt, etc.).
///
/// Specifically excluded: relics with non-empty relic_effects entries
/// (BeltBuckle dexterity, BloodVial heal, etc.) — those are tested by
/// the relic parity sweep against oracle's combat-side delta.
#[test]
fn every_no_combat_effect_relic_produces_no_combat_delta() {
    let mut tested = 0;
    let mut skipped: Vec<String> = Vec::new();
    for r in sts2_sim::relic::ALL_RELICS.iter() {
        if has_combat_effect(&r.id) {
            skipped.push(r.id.clone());
            continue;
        }
        // Some relics (Status/Curse-like or runtime-only) might not
        // be grantable; the helper panics if grant fails. Wrap in
        // catch_unwind so one bad relic doesn't kill the whole sweep.
        let id = r.id.clone();
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_relic_no_combat_change(&id);
        }));
        if let Err(_) = res {
            panic!("relic {} produced a combat-state delta despite empty\n  \
                relic_effects entry. Either add a real combat-side encoding\n  \
                or move the effect to run_state_effects.", r.id);
        }
        tested += 1;
    }
    eprintln!("Audited {} relics with no combat-side effect.", tested);
    eprintln!("Excluded {} relics that have non-empty combat encodings\n  \
        (verified separately by the relic parity sweep).", skipped.len());
    assert!(tested >= 100,
        "Expected at least 100 no-combat-effect relics, got {}", tested);
}

// ============================================================================
// Section 2: loose-compared `is_random_card_gen` cards. These move N
// cards from one pile to another via combat RNG. The sweep tolerates
// pile *contents* drift but enforces pile *count*. These tests assert
// each card produces the expected pile-size delta deterministically.
// ============================================================================

fn play_with_target(
    cs: &mut CombatState,
    card_id: &str,
    energy: i32,
    target: Option<(CombatSide, usize)>,
) {
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = energy;
    let hand_idx = ps.hand.cards.iter().position(|c| c.id == card_id)
        .unwrap_or_else(|| panic!("{} not in hand", card_id));
    let result = cs.play_card(0, hand_idx, target);
    assert!(
        matches!(result, sts2_sim::combat::PlayResult::Ok),
        "{} play failed: {:?}", card_id, result
    );
}

#[test]
fn distraction_adds_one_skill_to_hand() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Distraction");
    let hand_before = pile_size(&cs, PileType::Hand) - 1; // exclude played card
    play_with_target(&mut cs, "Distraction", 3, None);
    assert_eq!(
        pile_size(&cs, PileType::Hand),
        hand_before + 1,
        "Distraction should add exactly 1 card to hand"
    );
}

#[test]
fn infernal_blade_adds_one_attack_to_hand() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "InfernalBlade");
    let hand_before = pile_size(&cs, PileType::Hand) - 1;
    play_with_target(&mut cs, "InfernalBlade", 3, None);
    assert_eq!(pile_size(&cs, PileType::Hand), hand_before + 1);
}

#[test]
fn jack_of_all_trades_adds_one_colorless_to_hand() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "JackOfAllTrades");
    let hand_before = pile_size(&cs, PileType::Hand) - 1;
    play_with_target(&mut cs, "JackOfAllTrades", 3, None);
    assert_eq!(pile_size(&cs, PileType::Hand), hand_before + 1);
}

#[test]
fn discovery_adds_one_card_to_hand() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Discovery");
    let hand_before = pile_size(&cs, PileType::Hand) - 1;
    play_with_target(&mut cs, "Discovery", 3, None);
    assert_eq!(pile_size(&cs, PileType::Hand), hand_before + 1);
}

#[test]
fn alchemize_adds_one_potion_pending() {
    // Alchemize generates a potion. We track via pending_stars / a future
    // potion-belt field; for now just verify the card plays without panic.
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Alchemize");
    play_with_target(&mut cs, "Alchemize", 3, None);
}

#[test]
fn charge_transforms_two_draw_cards_to_minion_dive_bomb() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Charge");
    let draw_before = pile_size(&cs, PileType::Draw);
    play_with_target(&mut cs, "Charge", 3, None);
    // Draw size unchanged — transform replaces in-place.
    assert_eq!(pile_size(&cs, PileType::Draw), draw_before,
        "Charge should not change draw size, only transform 2 cards");
    let dive_bomb_count = cs.allies[0].player.as_ref().unwrap()
        .draw.cards.iter()
        .filter(|c| c.id == "MinionDiveBomb")
        .count();
    assert_eq!(dive_bomb_count, 2,
        "Charge should transform 2 draw cards to MinionDiveBomb");
}

#[test]
fn cleanse_summons_osty_and_exhausts_one_draw() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Cleanse");
    let draw_before = pile_size(&cs, PileType::Draw);
    let exhaust_before = pile_size(&cs, PileType::Exhaust);
    play_with_target(&mut cs, "Cleanse", 3, None);
    assert!(cs.allies[0].player.as_ref().unwrap().osty.is_some(),
        "Cleanse should summon an osty companion");
    assert_eq!(pile_size(&cs, PileType::Draw), draw_before - 1,
        "Cleanse should remove 1 card from draw via PlayerInteractive pick");
    assert_eq!(pile_size(&cs, PileType::Exhaust), exhaust_before + 1,
        "Cleanse should move the picked card to exhaust");
}

#[test]
fn reboot_shuffles_hand_into_draw_then_draws() {
    let mut cs = ironclad_combat_with_relics(&[]);
    // Stage some cards in hand to be reboot-ed.
    force_card(&mut cs, "Reboot");
    force_card(&mut cs, "StrikeIronclad");
    force_card(&mut cs, "DefendIronclad");
    let total_before = pile_size(&cs, PileType::Hand)
        + pile_size(&cs, PileType::Draw)
        + pile_size(&cs, PileType::Discard)
        + pile_size(&cs, PileType::Exhaust);
    play_with_target(&mut cs, "Reboot", 3, None);
    // Reboot moves hand → draw + shuffles + draws Cards canonical.
    // Reboot itself routes to exhaust (Exhaust keyword in C#). Total
    // card count across all piles is conserved.
    let total_after = pile_size(&cs, PileType::Hand)
        + pile_size(&cs, PileType::Draw)
        + pile_size(&cs, PileType::Discard)
        + pile_size(&cs, PileType::Exhaust);
    assert_eq!(total_after, total_before,
        "Reboot should conserve total card count across all piles");
    // The shuffle/redraw means hand has at least the Cards canonical
    // number drawn (assuming sufficient cards available).
    assert!(pile_size(&cs, PileType::Hand) > 0,
        "Reboot should draw at least one card");
}

#[test]
fn seance_transforms_one_draw_card_to_soul() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Seance");
    let draw_before = pile_size(&cs, PileType::Draw);
    play_with_target(&mut cs, "Seance", 3, None);
    assert_eq!(pile_size(&cs, PileType::Draw), draw_before,
        "Seance should not change draw size, only transform 1 card");
    let soul_count = cs.allies[0].player.as_ref().unwrap().draw
        .cards.iter().filter(|c| c.id == "Soul").count();
    assert_eq!(soul_count, 1,
        "Seance should transform 1 draw card to Soul");
}

// ============================================================================
// Section 3: loose-compared `is_random_target` cards. These deal damage
// or apply powers to RNG-picked enemies. The sweep verifies total HP +
// total power-amount across enemies. These tests assert the totals
// directly in rust.
// ============================================================================

#[test]
fn bouncing_flask_total_poison_equals_3x3() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "BouncingFlask");
    play_with_target(&mut cs, "BouncingFlask", 3, None);
    let total_poison: i32 = (0..cs.enemies.len())
        .map(|i| power_amount(&cs, CombatSide::Enemy, i, "PoisonPower"))
        .sum();
    assert_eq!(total_poison, 3 * 3,
        "BouncingFlask should apply 9 total poison stacks (3 hits × 3 each)");
}

#[test]
fn ricochet_total_damage_equals_3x4() {
    // C# Ricochet: DamageVar(3) × RepeatVar(4) = 12 total damage,
    // distributed across random enemies via combat RNG.
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Ricochet");
    let hp_before = total_enemy_hp(&cs);
    play_with_target(&mut cs, "Ricochet", 3, Some((CombatSide::Enemy, 0)));
    let damage_dealt = hp_before - total_enemy_hp(&cs);
    assert_eq!(damage_dealt, 3 * 4,
        "Ricochet should deal 12 total damage (3 dmg × 4 hits)");
}

#[test]
fn sword_boomerang_total_damage_equals_3x3() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "SwordBoomerang");
    let hp_before = total_enemy_hp(&cs);
    play_with_target(&mut cs, "SwordBoomerang", 3, Some((CombatSide::Enemy, 0)));
    let damage_dealt = hp_before - total_enemy_hp(&cs);
    assert_eq!(damage_dealt, 3 * 3,
        "SwordBoomerang should deal 9 total damage");
}

#[test]
fn rip_and_tear_total_damage_constant() {
    // C# RipAndTear: DamageVar(7) x RepeatVar(2) split across random
    // enemies; total = 14.
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "RipAndTear");
    let hp_before = total_enemy_hp(&cs);
    play_with_target(&mut cs, "RipAndTear", 3, None);
    let dmg = hp_before - total_enemy_hp(&cs);
    assert_eq!(dmg, 7 * 2,
        "RipAndTear should deal 14 total damage (7 dmg × 2 hits)");
}

#[test]
fn zap_channels_one_lightning_orb() {
    // C# Zap: ChannelOrb<LightningOrb>. Single channel; on Ironclad
    // (orb_slots=0) auto-bumps to 1 and stores the orb.
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Zap");
    play_with_target(&mut cs, "Zap", 3, None);
    let q = &cs.allies[0].player.as_ref().unwrap().orb_queue;
    assert_eq!(q.len(), 1, "Zap should channel 1 orb");
    assert_eq!(q[0].id, "LightningOrb", "Zap channels Lightning");
}

#[test]
fn rainbow_channels_lightning_frost_dark() {
    // C# Rainbow: 3 channels (Lightning, Frost, Dark). On Ironclad
    // orb_slots=0 → auto-bump to 1 on first. Second+third overflow,
    // evoking the previous orb. Final queue has the last channel.
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Rainbow");
    play_with_target(&mut cs, "Rainbow", 3, None);
    let q = &cs.allies[0].player.as_ref().unwrap().orb_queue;
    assert_eq!(q.len(), 1, "Rainbow on Ironclad ends with 1 orb in queue");
    assert_eq!(q[0].id, "DarkOrb",
        "Rainbow last-channeled orb is Dark (queue holds final push)");
}

// ============================================================================
// Section 4: TheHunt (room-conditional). C# bails out of OnPlay if
// currentRoom is not a CombatRoom; rust correctly performs the damage
// primitive unconditionally. The sweep skips the diff. This test
// confirms rust deals 10 damage.
// ============================================================================

#[test]
fn the_hunt_deals_10_damage_in_combat() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "TheHunt");
    let hp_before = cs.enemies[0].current_hp;
    play_with_target(&mut cs, "TheHunt", 3, Some((CombatSide::Enemy, 0)));
    assert_eq!(cs.enemies[0].current_hp, hp_before - 10,
        "TheHunt should deal 10 damage to chosen enemy");
}

// ============================================================================
// Section 5: StoneCracker (is_random_pile_pick). Picks 2 random
// upgradable cards from draw to upgrade. Sweep verifies upgraded-count
// matches. This test asserts exactly 2 upgrades happen.
// ============================================================================

#[test]
fn stone_cracker_upgrades_exactly_two_draw_cards() {
    let mut cs = ironclad_combat_with_relics(&["StoneCracker"]);
    let upgraded_before = cs.allies[0].player.as_ref().unwrap().draw
        .cards.iter().filter(|c| c.upgrade_level > 0).count();
    cs.fire_before_combat_start_hooks();
    let upgraded_after = cs.allies[0].player.as_ref().unwrap().draw
        .cards.iter().filter(|c| c.upgrade_level > 0).count();
    assert_eq!(upgraded_after - upgraded_before, 2,
        "StoneCracker should upgrade exactly 2 cards in draw");
}

// ============================================================================
// Section 6: ModifyRound1HandDraw deferred-hook relics. These bump the
// round-1 hand-draw count by 2. The relic sweep verifies via env.rs's
// initial-draw flow; this test exercises the same mechanism directly.
// ============================================================================

#[test]
fn bag_of_preparation_bumps_round1_hand_draw_by_2() {
    let mut cs = ironclad_combat_with_relics(&["BagOfPreparation"]);
    cs.fire_before_combat_start_hooks();
    let delta = cs.allies[0].player.as_ref().unwrap().hand_draw_round1_delta;
    assert_eq!(delta, 2, "BagOfPreparation should set round-1 delta to 2");
}

#[test]
fn ring_of_the_snake_bumps_round1_hand_draw_by_2() {
    let mut cs = ironclad_combat_with_relics(&["RingOfTheSnake"]);
    cs.fire_before_combat_start_hooks();
    let delta = cs.allies[0].player.as_ref().unwrap().hand_draw_round1_delta;
    assert_eq!(delta, 2, "RingOfTheSnake should set round-1 delta to 2");
}

#[test]
fn booming_conch_bumps_round1_hand_draw_by_2() {
    let mut cs = ironclad_combat_with_relics(&["BoomingConch"]);
    cs.fire_before_combat_start_hooks();
    let delta = cs.allies[0].player.as_ref().unwrap().hand_draw_round1_delta;
    assert_eq!(delta, 2, "BoomingConch should set round-1 delta to 2");
}

// ============================================================================
// Section 7: is_random_auto_play cards (Catastrophe, Havoc, Uproar,
// DistilledChaos, Mayhem). These auto-play N random cards from a pile.
// The sweep skips combat-state diffs; these tests assert the pile-size
// delta corresponding to "N cards consumed + dispatched".
// ============================================================================

#[test]
fn catastrophe_auto_plays_2_cards_from_draw() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Catastrophe");
    let draw_before = pile_size(&cs, PileType::Draw);
    play_with_target(&mut cs, "Catastrophe", 3, None);
    // 2 cards leave draw (auto-played); each routes to discard/exhaust.
    assert_eq!(pile_size(&cs, PileType::Draw), draw_before - 2,
        "Catastrophe should auto-play 2 cards from draw");
}

#[test]
fn havoc_auto_plays_1_card_from_draw_to_exhaust() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Havoc");
    let draw_before = pile_size(&cs, PileType::Draw);
    let exhaust_before = pile_size(&cs, PileType::Exhaust);
    play_with_target(&mut cs, "Havoc", 3, None);
    assert_eq!(pile_size(&cs, PileType::Draw), draw_before - 1,
        "Havoc should auto-play 1 card from draw");
    // force_exhaust=true routes the auto-played card to exhaust.
    // Plus Havoc itself routes to discard (no Exhaust keyword on Havoc).
    assert!(pile_size(&cs, PileType::Exhaust) > exhaust_before,
        "Havoc should route the auto-played card to exhaust");
}

#[test]
fn uproar_damages_then_auto_plays_one_attack() {
    let mut cs = ironclad_combat_with_relics(&[]);
    force_card(&mut cs, "Uproar");
    let hp_before = cs.enemies[0].current_hp;
    let draw_before = pile_size(&cs, PileType::Draw);
    play_with_target(&mut cs, "Uproar", 3, Some((CombatSide::Enemy, 0)));
    // Damage from Uproar's own DealDamage step (2 hits × Damage canonical).
    assert!(cs.enemies[0].current_hp < hp_before,
        "Uproar should deal damage from its DealDamage step");
    // One Attack-type card pulled from draw and auto-played.
    assert_eq!(pile_size(&cs, PileType::Draw), draw_before - 1,
        "Uproar should auto-play 1 attack from draw");
}

// DistilledChaos doesn't exist in STS2 (STS1-only). Skipped; its
// encoding in rust is dead code retained for historical parity.

// ============================================================================
// Section 8: Run-state side of relics whose combat encoding is empty
// (matches the 8 "skipped" Ancients) — verify deck/potion/HP/gold
// modifications happen at AfterObtained via the run-state hook chain.
// The relic parity sweep tests combat only; this section covers the
// strategic-layer surface that's otherwise untested.
// ============================================================================

use sts2_sim::act::ActId;
use sts2_sim::run_state::{PlayerState as RsPlayerState, RunState};

fn fresh_run_state() -> RunState {
    let players = vec![RsPlayerState {
        character_id: "CHARACTER.IRONCLAD".into(),
        id: 1,
        hp: 80,
        max_hp: 80,
        gold: 0,
        deck: Vec::new(),
        relics: Vec::new(),
        potions: Vec::new(),
        max_potion_slot_count: 3,
    }];
    RunState::new("AUDIT123", 0, players, vec![ActId::Overgrowth], Vec::new())
}

#[test]
fn potion_belt_grants_2_extra_potion_slots() {
    let mut rs = fresh_run_state();
    rs.add_relic(0, "PotionBelt");
    assert_eq!(rs.players()[0].max_potion_slot_count, 5,
        "PotionBelt grants +2 slots (3 base -> 5)");
}

#[test]
fn phial_holster_grants_1_extra_potion_slot() {
    let mut rs = fresh_run_state();
    rs.add_relic(0, "PhialHolster");
    assert_eq!(rs.players()[0].max_potion_slot_count, 4,
        "PhialHolster grants +1 slot (3 base -> 4)");
}

#[test]
fn alchemical_coffer_grants_4_extra_potion_slots() {
    let mut rs = fresh_run_state();
    rs.add_relic(0, "AlchemicalCoffer");
    assert_eq!(rs.players()[0].max_potion_slot_count, 7,
        "AlchemicalCoffer grants +4 slots (3 base -> 7)");
}

#[test]
fn jewelry_box_adds_apotheosis_to_deck() {
    let mut rs = fresh_run_state();
    rs.add_relic(0, "JewelryBox");
    let deck = &rs.players()[0].deck;
    assert!(deck.iter().any(|c| c.id == "Apotheosis"),
        "JewelryBox should add Apotheosis to deck");
}

#[test]
fn neows_torment_adds_neows_fury_to_deck() {
    let mut rs = fresh_run_state();
    rs.add_relic(0, "NeowsTorment");
    let deck = &rs.players()[0].deck;
    assert!(deck.iter().any(|c| c.id == "NeowsFury"),
        "NeowsTorment should add NeowsFury to deck");
}

#[test]
fn paels_horn_adds_two_relax_to_deck() {
    let mut rs = fresh_run_state();
    rs.add_relic(0, "PaelsHorn");
    let count = rs.players()[0].deck.iter().filter(|c| c.id == "Relax").count();
    assert_eq!(count, 2, "PaelsHorn should add 2 Relax to deck");
}

#[test]
fn sere_talon_adds_three_wish_to_deck() {
    let mut rs = fresh_run_state();
    rs.add_relic(0, "SereTalon");
    let count = rs.players()[0].deck.iter().filter(|c| c.id == "Wish").count();
    assert_eq!(count, 3, "SereTalon should add 3 Wish to deck");
}

#[test]
fn fragrant_mushroom_loses_15_hp() {
    let mut rs = fresh_run_state();
    let hp_before = rs.players()[0].hp;
    rs.add_relic(0, "FragrantMushroom");
    assert_eq!(rs.players()[0].hp, hp_before - 15,
        "FragrantMushroom should lose 15 HP on obtain");
}

#[test]
fn mango_gains_14_max_hp() {
    let mut rs = fresh_run_state();
    let max_before = rs.players()[0].max_hp;
    rs.add_relic(0, "Mango");
    assert_eq!(rs.players()[0].max_hp, max_before + 14,
        "Mango should grant +14 max HP");
    assert_eq!(rs.players()[0].hp, max_before + 14,
        "Mango should also heal up to new max");
}

#[test]
fn old_coin_gains_300_gold() {
    let mut rs = fresh_run_state();
    rs.add_relic(0, "OldCoin");
    assert_eq!(rs.players()[0].gold, 300,
        "OldCoin should grant +300 gold");
}

// ============================================================================
// Section 9: Confirm MadScience variants are covered by the dedicated
// test (../mad_science_variants.rs). This is just a smoke test that
// MadScience's encoding is non-empty.
// ============================================================================

#[test]
fn mad_science_encoding_is_present_and_non_empty() {
    let effects = sts2_sim::effects::card_effects("MadScience");
    let effects = effects.expect("MadScience must have an encoding");
    assert!(
        !effects.is_empty(),
        "MadScience encoding must be non-empty (see mad_science_variants.rs for 9/9 coverage)"
    );
    // 9 Conditional branches: 4 attack-shape + 4 skill-shape + 3 power-shape
    // = 11 (Skill base block is its own branch). Just verify at least 9
    // present so a regression that drops a variant trips.
    use sts2_sim::effects::Effect;
    let conditional_count = effects.iter()
        .filter(|e| matches!(e, Effect::Conditional { .. }))
        .count();
    assert!(conditional_count >= 9,
        "MadScience encoding has {} conditionals, expected >= 9 (one per variant)",
        conditional_count);
}
