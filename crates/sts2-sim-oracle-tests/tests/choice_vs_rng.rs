//! Distinguish RNG vs player-CHOICE card mechanics.
//!
//! Some cards use RNG to pick their target ("exhaust a random card
//! from hand", TrueGrit unupgraded) — the agent has nothing to learn,
//! the simulator just rolls.
//!
//! Other cards present the player a CHOICE ("exhaust a card from
//! hand", TrueGrit upgraded; "put a card from discard onto draw",
//! Headbutt). The agent must learn which card to pick.
//!
//! For RL training the simulator must emit a choice REQUEST when a
//! choice point is reached — otherwise the agent has no observation
//! to act on. The `auto_resolve_choices` flag on CombatState controls
//! this: when true (default, used by parity sweeps and replay), the
//! simulator auto-picks; when false, it pauses with
//! `CombatState.pending_choice` set so RL drivers can route an
//! `Action::ResolveChoice { picks }` back into the simulator.
//!
//! This file proves the distinction is wired correctly:
//!   - TrueGrit (unupgraded): exhausts via RNG, no pending_choice
//!     emitted even with `auto_resolve_choices == false`.
//!   - TrueGrit+ (upgraded): with auto-resolve OFF, sets pending_choice
//!     and stops. The agent's `resolve_pending_choice` picks resolve
//!     the action.

use sts2_sim::card;
use sts2_sim::combat::{
    CardInstance, CombatSide, CombatState, PileType,
};
use sts2_sim::encounter;
use sts2_sim::effects::resolve_pending_choice;

fn ironclad_combat() -> CombatState {
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
        relics: vec!["BurningBlood".to_string()],
    };
    CombatState::start(enc, vec![setup], Vec::new())
}

fn force_card(cs: &mut CombatState, card_id: &str, upgrade: i32) {
    let data = card::by_id(card_id).expect("card in registry");
    let inst = CardInstance::from_card(data, upgrade);
    cs.allies[0].player.as_mut().unwrap().hand.cards.push(inst);
}

fn hand_size(cs: &CombatState) -> usize {
    cs.allies[0].player.as_ref().unwrap().hand.cards.len()
}

fn exhaust_size(cs: &CombatState) -> usize {
    cs.allies[0].player.as_ref().unwrap().exhaust.cards.len()
}

#[test]
fn true_grit_unupgraded_resolves_via_rng_no_choice_emitted() {
    let mut cs = ironclad_combat();
    cs.auto_resolve_choices = false; // Strict mode — never auto-resolve.

    // Stage extra hand cards so the random pick has a candidate set.
    force_card(&mut cs, "StrikeIronclad", 0);
    force_card(&mut cs, "DefendIronclad", 0);
    force_card(&mut cs, "TrueGrit", 0); // index 2

    let hand_before = hand_size(&cs);
    let exhaust_before = exhaust_size(&cs);

    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 2, None); // play TrueGrit

    // Unupgraded TrueGrit is RNG → no pending_choice even with strict mode.
    assert!(
        cs.pending_choice.is_none(),
        "TrueGrit unupgraded should NOT emit a choice request — it's RNG."
    );
    // One card got exhausted, hand shrank by TrueGrit + one extra.
    assert_eq!(exhaust_size(&cs), exhaust_before + 1,
        "TrueGrit should exhaust exactly 1 hand card");
    assert_eq!(hand_size(&cs), hand_before - 2,
        "Hand should lose TrueGrit (played) + 1 exhausted card");
}

#[test]
fn true_grit_upgraded_emits_choice_request_in_strict_mode() {
    let mut cs = ironclad_combat();
    cs.auto_resolve_choices = false;

    // Stage candidates.
    force_card(&mut cs, "StrikeIronclad", 0);
    force_card(&mut cs, "DefendIronclad", 0);
    force_card(&mut cs, "TrueGrit", 1); // upgraded, hand_idx 2

    let exhaust_before = exhaust_size(&cs);

    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 2, None); // play TrueGrit+

    // Upgraded TrueGrit → choice request, no exhaust applied yet.
    let pc = cs.pending_choice.as_ref()
        .expect("TrueGrit+ MUST emit a pending_choice in strict mode");
    assert_eq!(pc.source_card_id, "TrueGrit");
    assert_eq!(pc.pile, PileType::Hand);
    assert_eq!(pc.n_min, 0);
    assert_eq!(pc.n_max, 1);
    assert_eq!(exhaust_size(&cs), exhaust_before,
        "No exhaust until ResolveChoice fires");

    // Agent picks hand[0] (a Strike). Resolve it.
    let r = resolve_pending_choice(&mut cs, &[0]);
    assert!(r.is_ok(), "resolve_pending_choice failed: {:?}", r);
    assert!(cs.pending_choice.is_none(),
        "pending_choice should clear after resolution");
    assert_eq!(exhaust_size(&cs), exhaust_before + 1,
        "Resolving with 1 pick should exhaust 1 card");
}

#[test]
fn auto_resolve_default_preserves_existing_behavior() {
    // Sanity: with auto_resolve_choices=true (default), TrueGrit+
    // behaves identically to unupgraded for combat-state output —
    // both resolve in one shot, no pending_choice.
    let mut cs = ironclad_combat();
    // Don't toggle auto_resolve_choices — it defaults to true.
    assert!(cs.auto_resolve_choices,
        "auto_resolve_choices should default to true for backward compat");

    force_card(&mut cs, "StrikeIronclad", 0);
    force_card(&mut cs, "TrueGrit", 1);

    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 1, None);

    assert!(cs.pending_choice.is_none(),
        "auto-resolve mode should never leave a pending_choice");
}

#[test]
fn resolve_pending_choice_validates_pick_count_against_n_max() {
    let mut cs = ironclad_combat();
    cs.auto_resolve_choices = false;
    force_card(&mut cs, "StrikeIronclad", 0);
    force_card(&mut cs, "DefendIronclad", 0);
    force_card(&mut cs, "TrueGrit", 1);

    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 2, None);
    assert!(cs.pending_choice.is_some());

    // Try to pick 2 cards when n_max == 1.
    let r = resolve_pending_choice(&mut cs, &[0, 1]);
    assert!(r.is_err(), "Should reject pick count > n_max");
    // The pending_choice should still be there for retry.
    assert!(cs.pending_choice.is_some(),
        "Failed resolve should preserve pending_choice for retry");

    // Now pick 1 — should succeed.
    let r = resolve_pending_choice(&mut cs, &[0]);
    assert!(r.is_ok(), "Single-pick resolve failed: {:?}", r);
}

#[test]
fn resolve_pending_choice_allows_zero_picks_when_n_min_is_zero() {
    // TrueGrit+ uses n_min=0 (allows skip — C# FromHand with MinSelect 0).
    let mut cs = ironclad_combat();
    cs.auto_resolve_choices = false;
    force_card(&mut cs, "StrikeIronclad", 0);
    force_card(&mut cs, "TrueGrit", 1);

    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 1, None);

    let exhaust_before = exhaust_size(&cs);
    let r = resolve_pending_choice(&mut cs, &[]);
    assert!(r.is_ok(), "Skipping a 0-min choice should succeed: {:?}", r);
    assert_eq!(exhaust_size(&cs), exhaust_before,
        "Skipping resolves without exhausting anything");
}

#[test]
fn resolve_pending_choice_rejects_out_of_range_index() {
    let mut cs = ironclad_combat();
    cs.auto_resolve_choices = false;
    force_card(&mut cs, "StrikeIronclad", 0);
    force_card(&mut cs, "TrueGrit", 1);

    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 1, None);
    // Hand now has just Strike at index 0.
    let r = resolve_pending_choice(&mut cs, &[99]);
    assert!(r.is_err(), "Out-of-range pick should be rejected");
    assert!(cs.pending_choice.is_some(),
        "pending_choice should survive a rejected pick");
}
