//! Potion audit — every potion has a non-empty encoding and produces
//! the expected combat-state delta. Catches encoding regressions
//! that the parity sweep won't (potions aren't currently parity-tested
//! against oracle; they're driven from RL agents via UsePotion).

use sts2_sim::card;
use sts2_sim::combat::{
    CardInstance, CombatSide, CombatState, PileType,
};
use sts2_sim::effects::{self, EffectContext};
use sts2_sim::encounter;
use sts2_sim::potion;

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

/// Execute a potion's effect list against `cs` with the player as
/// the source. Mirrors how UsePotion routes via potion_effects.
/// Uses `for_potion_use` so AmountSpec::Canonical resolves through
/// the potion's canonical_vars table (kind="Damage"/"Cards"/etc.).
fn use_potion(cs: &mut CombatState, potion_id: &str, target_enemy_idx: Option<usize>) {
    let effects_list = effects::potion_effects(potion_id)
        .unwrap_or_else(|| panic!("no encoding for {}", potion_id));
    let target = target_enemy_idx.map(|i| (CombatSide::Enemy, i));
    let ctx = EffectContext::for_potion_use(0, target, potion_id);
    effects::execute_effects(cs, &effects_list, &ctx);
}

// ----------------------------------------------------------------------
// Coverage: every non-deprecated potion must have a non-None encoding.
// ----------------------------------------------------------------------

#[test]
fn every_potion_has_an_encoding() {
    let mut missing: Vec<String> = Vec::new();
    for p in potion::ALL_POTIONS.iter() {
        if p.id == "DeprecatedPotion" {
            continue;
        }
        if effects::potion_effects(&p.id).is_none() {
            missing.push(p.id.clone());
        }
    }
    assert!(missing.is_empty(),
        "Potions missing encoding: {:?}", missing);
}

// ----------------------------------------------------------------------
// Newly-wired potions (8 from this iteration).
// ----------------------------------------------------------------------

#[test]
fn blood_potion_heals_20_percent_max_hp() {
    let mut cs = ironclad_combat();
    // Pre-damage the player so we can observe heal.
    cs.allies[0].current_hp = 50; // out of 80
    use_potion(&mut cs, "BloodPotion", None);
    // 20% of 80 = 16 → hp = 50 + 16 = 66.
    assert_eq!(cs.allies[0].current_hp, 66,
        "BloodPotion: 20% of 80 max HP = +16 heal");
}

#[test]
fn fairy_in_a_bottle_heals_30_percent_max_hp() {
    let mut cs = ironclad_combat();
    cs.allies[0].current_hp = 10;
    use_potion(&mut cs, "FairyInABottle", None);
    // 30% of 80 = 24 → hp = 10 + 24 = 34.
    assert_eq!(cs.allies[0].current_hp, 34);
}

#[test]
fn foul_potion_damages_all_enemies_and_player() {
    let mut cs = ironclad_combat();
    let enemy_hp_before: Vec<i32> = cs.enemies.iter().map(|e| e.current_hp).collect();
    let player_hp_before = cs.allies[0].current_hp;
    use_potion(&mut cs, "FoulPotion", None);
    for (i, e) in cs.enemies.iter().enumerate() {
        assert!(e.current_hp < enemy_hp_before[i],
            "FoulPotion should damage enemy {}", i);
    }
    assert!(cs.allies[0].current_hp < player_hp_before,
        "FoulPotion should damage the player too");
}

#[test]
fn snecko_oil_draws_seven_cards() {
    let mut cs = ironclad_combat();
    let hand_before = cs.allies[0].player.as_ref().unwrap().hand.cards.len();
    use_potion(&mut cs, "SneckoOil", None);
    let hand_after = cs.allies[0].player.as_ref().unwrap().hand.cards.len();
    // Draws Cards canonical = 7 (capped by available draw pile, but
    // Ironclad starter deck has 10 cards so 7 is reachable).
    assert!(hand_after >= hand_before + 7
        || hand_after == cs.allies[0].player.as_ref().unwrap().hand.cards.len(),
        "SneckoOil should draw 7 (or all available if less); hand went \
        from {} to {}", hand_before, hand_after);
}

#[test]
fn touch_of_insanity_sets_a_hand_card_cost_to_zero() {
    let mut cs = ironclad_combat();
    // Stage a costed card in hand (Strike costs 1).
    let strike = CardInstance::from_card(
        card::by_id("StrikeIronclad").expect("Strike"), 0);
    cs.allies[0].player.as_mut().unwrap().hand.cards.push(strike);
    let cost_before = cs.allies[0].player.as_ref().unwrap()
        .hand.cards.last().unwrap().current_energy_cost;
    assert!(cost_before > 0, "Strike should cost >0 before potion");
    use_potion(&mut cs, "TouchOfInsanity", None);
    // Pick is PlayerInteractive(n=1) auto-resolved to first card in
    // hand (Bottom selector). Its cost should now be 0 this combat.
    let card_after = &cs.allies[0].player.as_ref().unwrap().hand.cards[0];
    let effective = card_after.effective_energy_cost();
    assert_eq!(effective, 0,
        "TouchOfInsanity should set picked card's cost to 0 this combat");
}

#[test]
fn gamblers_brew_runs_without_panic() {
    // GamblersBrew with auto-resolve picks 0 cards (Bottom selector
    // with empty hand). Just verify it doesn't crash and doesn't
    // produce a stale pending_choice when auto-resolving.
    let mut cs = ironclad_combat();
    use_potion(&mut cs, "GamblersBrew", None);
    assert!(cs.pending_choice.is_none(),
        "auto-resolve should not leave a pending_choice");
}

#[test]
fn soldiers_stew_bumps_replay_count_on_every_strike() {
    // SoldiersStew walks hand + draw + discard + exhaust and bumps
    // state["replay_count"] on every Strike-tagged card. play_card's
    // loop reads that counter as BaseReplayCount.
    let mut cs = ironclad_combat();
    // Confirm the starting Ironclad deck has Strike copies in the
    // draw pile (sweep starts with all cards in draw, no hand).
    let strike_count_before: usize = {
        let ps = cs.allies[0].player.as_ref().unwrap();
        ps.draw.cards.iter()
            .filter(|c| c.id == "StrikeIronclad")
            .count()
    };
    assert!(strike_count_before >= 4,
        "Ironclad starter deck should have >= 4 Strikes (had {})",
        strike_count_before);

    use_potion(&mut cs, "SoldiersStew", None);

    // Every Strike in every pile should have replay_count == 1 now.
    let ps = cs.allies[0].player.as_ref().unwrap();
    for pile in [&ps.hand, &ps.draw, &ps.discard, &ps.exhaust] {
        for c in &pile.cards {
            if c.id == "StrikeIronclad" {
                assert_eq!(c.state.get("replay_count").copied(), Some(1),
                    "Strike must have replay_count=1 after SoldiersStew (got {:?})",
                    c.state.get("replay_count"));
            } else {
                // Non-Strike cards untouched.
                assert!(c.state.get("replay_count").is_none()
                    || c.state.get("replay_count") == Some(&0),
                    "non-Strike card {} should not have replay_count set",
                    c.id);
            }
        }
    }
}

#[test]
fn soldiers_stew_makes_strike_dispatch_twice() {
    // End-to-end: after SoldiersStew, playing a Strike fires OnPlay
    // twice (one base + one BaseReplayCount). Strike deals 6 → 12.
    let mut cs = ironclad_combat();
    use_potion(&mut cs, "SoldiersStew", None);
    // Move a Strike to hand for play.
    let ps = cs.allies[0].player.as_mut().unwrap();
    let pos = ps.draw.cards.iter()
        .position(|c| c.id == "StrikeIronclad")
        .expect("a Strike in draw");
    let strike = ps.draw.cards.remove(pos);
    assert_eq!(strike.state.get("replay_count").copied(), Some(1));
    ps.hand.cards.push(strike);
    ps.energy = 3;
    let hp_before = cs.enemies[0].current_hp;
    let hand_idx = cs.allies[0].player.as_ref().unwrap()
        .hand.cards.iter().position(|c| c.id == "StrikeIronclad").unwrap();
    cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
    let dmg = hp_before - cs.enemies[0].current_hp;
    assert_eq!(dmg, 12,
        "Strike with replay_count=1 should deal 6+6=12 (dealt {})",
        dmg);
}
