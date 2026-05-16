//! Reward-offer primitive audit.
//!
//! The first run-state-side choice surface: card / relic / potion
//! offers. Mirrors combat's `AwaitPlayerChoice` flow but operates on
//! the run-state. Two paths exercised:
//!   (a) Auto-resolve (default): card-reward skip / forced-pick relic
//!       reward / potion drop into belt.
//!   (b) Deferred RL path: `pending_offer` set, `resolve_run_state_offer`
//!       applies a specific pick. Validation errors (out-of-range,
//!       duplicate, wrong count) re-raise.

use sts2_sim::effects::{self, Effect, EffectContext};
use sts2_sim::run_state::{OfferKind, PlayerState, RunState};

fn fresh_run_state() -> RunState {
    let player = PlayerState {
        character_id: "Ironclad".to_string(),
        id: 1,
        hp: 80,
        max_hp: 80,
        gold: 99,
        deck: Vec::new(),
        relics: Vec::new(),
        potions: Vec::new(),
        max_potion_slot_count: 3,
        card_shop_removals_used: 0,
    };
    RunState::new(
        "seed",
        0,
        vec![player],
        vec![sts2_sim::act::ActId::Overgrowth],
        Vec::new(),
    )
}

fn exec(rs: &mut RunState, effects_list: Vec<Effect>) {
    let ctx = EffectContext::for_relic_hook(0, "");
    effects::execute_run_state_effects(rs, 0, &effects_list);
    let _ = ctx; // ctx not used by run-state path; placeholder for shape parity
}

// ----------------------------------------------------------------------
// Section A: auto-resolve fallback.
// ----------------------------------------------------------------------

#[test]
fn card_reward_skip_default_under_auto_resolve() {
    // Standard card reward: 3 options, n_min=0, n_max=1. Auto-resolve
    // takes 0 (skip-allowed shape).
    let mut rs = fresh_run_state();
    assert!(rs.auto_resolve_offers);
    exec(&mut rs, vec![Effect::OfferCardReward {
        options: vec!["StrikeIronclad".to_string(),
                      "DefendIronclad".to_string(),
                      "Anger".to_string()],
        n_min: 0,
        n_max: 1,
        source: Some("PostCombatReward".to_string()),
    }]);
    assert!(rs.pending_offer.is_none(),
        "Auto-resolve should clear pending_offer");
    assert_eq!(rs.players()[0].deck.len(), 0,
        "n_min=0 auto-resolve skips → deck unchanged");
}

#[test]
fn relic_reward_forced_pick_under_auto_resolve() {
    // Treasure room: 1 relic offered with skip NOT allowed (n_min=1).
    // Auto-resolve picks options[0].
    let mut rs = fresh_run_state();
    exec(&mut rs, vec![Effect::OfferRelicReward {
        options: vec!["BurningBlood".to_string()],
        n_min: 1,
        n_max: 1,
        source: Some("TreasureRoom".to_string()),
    }]);
    let relics = &rs.players()[0].relics;
    assert_eq!(relics.len(), 1, "1 relic should be granted");
    assert_eq!(relics[0].id, "BurningBlood");
}

#[test]
fn potion_reward_drops_into_belt() {
    let mut rs = fresh_run_state();
    exec(&mut rs, vec![Effect::OfferPotionReward {
        options: vec!["BlockPotion".to_string()],
        n_min: 1,
        n_max: 1,
        source: None,
    }]);
    assert_eq!(rs.players()[0].potions.len(), 1);
    assert_eq!(rs.players()[0].potions[0].id, "BlockPotion");
}

#[test]
fn potion_reward_silently_dropped_when_belt_full() {
    let mut rs = fresh_run_state();
    // Fill the 3-slot belt.
    for id in ["BlockPotion", "FirePotion", "EnergyPotion"] {
        rs.add_potion(0, id);
    }
    assert_eq!(rs.players()[0].potions.len(), 3);
    // Try to add a 4th — should be silently dropped.
    exec(&mut rs, vec![Effect::OfferPotionReward {
        options: vec!["BloodPotion".to_string()],
        n_min: 1,
        n_max: 1,
        source: None,
    }]);
    assert_eq!(rs.players()[0].potions.len(), 3,
        "Potion drop when belt full silently noops");
    let ids: Vec<&str> = rs.players()[0].potions.iter()
        .map(|p| p.id.as_str()).collect();
    assert!(!ids.contains(&"BloodPotion"),
        "BloodPotion should not have landed");
}

// ----------------------------------------------------------------------
// Section B: deferred RL path.
// ----------------------------------------------------------------------

#[test]
fn deferred_offer_sets_pending_choice() {
    let mut rs = fresh_run_state();
    rs.auto_resolve_offers = false;
    exec(&mut rs, vec![Effect::OfferCardReward {
        options: vec!["StrikeIronclad".to_string(),
                      "DefendIronclad".to_string(),
                      "Anger".to_string()],
        n_min: 0,
        n_max: 1,
        source: Some("EliteReward".to_string()),
    }]);
    let offer = rs.pending_offer.as_ref().expect("offer was staged");
    assert_eq!(offer.kind, OfferKind::Card);
    assert_eq!(offer.options.len(), 3);
    assert_eq!(offer.n_min, 0);
    assert_eq!(offer.n_max, 1);
    assert_eq!(offer.source.as_deref(), Some("EliteReward"));
    // Deck NOT yet mutated.
    assert_eq!(rs.players()[0].deck.len(), 0);
}

#[test]
fn resolve_offer_applies_picked_card() {
    let mut rs = fresh_run_state();
    rs.auto_resolve_offers = false;
    exec(&mut rs, vec![Effect::OfferCardReward {
        options: vec!["StrikeIronclad".to_string(),
                      "DefendIronclad".to_string(),
                      "Anger".to_string()],
        n_min: 0,
        n_max: 1,
        source: None,
    }]);
    // Agent picks index 2 (Anger).
    effects::resolve_run_state_offer(&mut rs, &[2])
        .expect("resolve succeeds");
    assert!(rs.pending_offer.is_none());
    let deck = &rs.players()[0].deck;
    assert_eq!(deck.len(), 1);
    assert_eq!(deck[0].id, "Anger");
}

#[test]
fn resolve_offer_skip_with_zero_picks() {
    let mut rs = fresh_run_state();
    rs.auto_resolve_offers = false;
    exec(&mut rs, vec![Effect::OfferCardReward {
        options: vec!["StrikeIronclad".to_string()],
        n_min: 0,
        n_max: 1,
        source: None,
    }]);
    effects::resolve_run_state_offer(&mut rs, &[]).expect("skip allowed");
    assert!(rs.pending_offer.is_none());
    assert_eq!(rs.players()[0].deck.len(), 0,
        "Empty picks → deck unchanged");
}

#[test]
fn resolve_offer_rejects_below_min() {
    let mut rs = fresh_run_state();
    rs.auto_resolve_offers = false;
    exec(&mut rs, vec![Effect::OfferRelicReward {
        options: vec!["BurningBlood".to_string()],
        n_min: 1,
        n_max: 1,
        source: None,
    }]);
    let err = effects::resolve_run_state_offer(&mut rs, &[]).unwrap_err();
    assert!(err.contains("outside [1, 1]"),
        "Expected count-out-of-range error, got: {}", err);
    // Offer must be restored for retry.
    assert!(rs.pending_offer.is_some(),
        "Validation failure must restore the pending offer");
}

#[test]
fn resolve_offer_rejects_above_max() {
    let mut rs = fresh_run_state();
    rs.auto_resolve_offers = false;
    exec(&mut rs, vec![Effect::OfferCardReward {
        options: vec!["StrikeIronclad".to_string(),
                      "DefendIronclad".to_string()],
        n_min: 0,
        n_max: 1,
        source: None,
    }]);
    let err = effects::resolve_run_state_offer(&mut rs, &[0, 1]).unwrap_err();
    assert!(err.contains("outside [0, 1]"));
    assert!(rs.pending_offer.is_some());
}

#[test]
fn resolve_offer_rejects_out_of_range_index() {
    let mut rs = fresh_run_state();
    rs.auto_resolve_offers = false;
    exec(&mut rs, vec![Effect::OfferCardReward {
        options: vec!["StrikeIronclad".to_string()],
        n_min: 0,
        n_max: 1,
        source: None,
    }]);
    let err = effects::resolve_run_state_offer(&mut rs, &[5]).unwrap_err();
    assert!(err.contains("out of range"));
    assert!(rs.pending_offer.is_some());
}

#[test]
fn resolve_offer_rejects_duplicate_index() {
    // Edge: an offer with n_max > 1 should reject duplicate picks
    // (the same option can't be picked twice).
    let mut rs = fresh_run_state();
    rs.auto_resolve_offers = false;
    exec(&mut rs, vec![Effect::OfferCardReward {
        options: vec!["StrikeIronclad".to_string(),
                      "DefendIronclad".to_string()],
        n_min: 0,
        n_max: 2,
        source: None,
    }]);
    let err = effects::resolve_run_state_offer(&mut rs, &[0, 0]).unwrap_err();
    assert!(err.contains("duplicate"));
    assert!(rs.pending_offer.is_some());
}

// ----------------------------------------------------------------------
// Section C: relic offer fires AfterObtained hook.
// ----------------------------------------------------------------------

#[test]
fn relic_offer_fires_after_obtained_hook() {
    // Mango grants +14 MaxHp on obtain via run_state_effects.
    // The offer flow must route through `add_relic` which fires the hook.
    let mut rs = fresh_run_state();
    let max_hp_before = rs.players()[0].max_hp;
    exec(&mut rs, vec![Effect::OfferRelicReward {
        options: vec!["Mango".to_string()],
        n_min: 1,
        n_max: 1,
        source: None,
    }]);
    let max_hp_after = rs.players()[0].max_hp;
    assert_eq!(max_hp_after - max_hp_before, 14,
        "Mango should grant +14 MaxHp on obtain (was {}, now {})",
        max_hp_before, max_hp_after);
    assert_eq!(rs.players()[0].relics.len(), 1);
    assert_eq!(rs.players()[0].relics[0].id, "Mango");
}
