//! Enchantment audit + duplication semantics.
//!
//! Enchantments are per-instance modifiers on cards. The data table
//! has 22 entries; the modifier pipeline wires 4 directly into damage
//! / block calculations (Sharp, Corrupted, Nimble, Vigorous); the
//! OnPlay enchantments (Sown, Swift, Adroit, Inky) fire after the
//! card's OnPlay body. Once-per-combat enchantments (Sown, Glam,
//! Swift, Vigorous) track `consumed_this_combat` per-instance.
//!
//! CRITICAL FOR RL: Duplication mechanics (Anger, DualWield,
//! CloneSourceCardToPile) must produce duplicates with FRESH
//! enchantment state — `consumed_this_combat=false` on each copy,
//! so once-per-combat triggers fire on each duplicate independently.
//! This file proves that invariant.

use sts2_sim::card;
use sts2_sim::combat::{
    apply_enchantment_on_play, CardInstance, CombatSide, CombatState,
    EnchantmentInstance, PileType,
};
use sts2_sim::effects::{
    self, AmountSpec, Effect, EffectContext, Pile,
};
use sts2_sim::encounter;

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

fn force_enchanted(cs: &mut CombatState, card_id: &str, ench_id: &str, amount: i32) {
    let data = card::by_id(card_id).expect("card in registry");
    let mut inst = CardInstance::from_card(data, 0);
    inst.enchantment = Some(EnchantmentInstance {
        id: ench_id.to_string(),
        amount,
        consumed_this_combat: false,
        state: Default::default(),
    });
    cs.allies[0].player.as_mut().unwrap().hand.cards.push(inst);
}

// ----------------------------------------------------------------------
// Section A: each OnPlay enchantment fires its effect on first play.
// ----------------------------------------------------------------------

#[test]
fn sown_grants_energy_on_first_play() {
    let mut cs = ironclad_combat();
    cs.allies[0].player.as_mut().unwrap().energy = 0;
    force_enchanted(&mut cs, "StrikeIronclad", "Sown", 2);
    // Play the Strike (target enemy 0 since it's an Attack).
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 1;
    cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
    // Sown granted +2 energy on top of the 1-1=0 from the play.
    assert_eq!(cs.allies[0].player.as_ref().unwrap().energy, 2,
        "Sown should grant +2 energy on first play");
}

#[test]
fn sown_consumed_flag_set_after_first_play() {
    let mut cs = ironclad_combat();
    force_enchanted(&mut cs, "StrikeIronclad", "Sown", 2);
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
    // Card is now in discard. Find it and check the consumed flag.
    let card = cs.allies[0].player.as_ref().unwrap()
        .discard.cards.iter()
        .find(|c| c.id == "StrikeIronclad").expect("Strike in discard");
    let ench = card.enchantment.as_ref().expect("enchantment preserved");
    assert!(ench.consumed_this_combat,
        "Sown's consumed_this_combat must flip true after first play");
}

#[test]
fn adroit_grants_block_every_play() {
    // Adroit is NOT once-per-combat — it fires GainBlock on every play.
    let mut cs = ironclad_combat();
    force_enchanted(&mut cs, "DefendIronclad", "Adroit", 4);
    let block_before = cs.allies[0].block;
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 0, None);
    // Defend gives 5 block; Adroit adds 4 more.
    assert!(cs.allies[0].block >= block_before + 4 + 5,
        "Adroit should add 4 block on top of Defend's 5 ({} -> {})",
        block_before, cs.allies[0].block);
}

// ----------------------------------------------------------------------
// Section B: duplication semantics — the critical invariant.
// ----------------------------------------------------------------------

#[test]
fn cloning_a_sown_card_produces_fresh_enchantment_on_copy() {
    // Stage a card and clone it via CloneSourceCardToPile (Anger /
    // self-replicate pattern). The clone should carry the enchantment
    // with consumed_this_combat=false even though the source's flag
    // was set to true by a previous play.
    let mut cs = ironclad_combat();
    let mut inst = CardInstance::from_card(
        card::by_id("Anger").expect("Anger"), 0);
    inst.enchantment = Some(EnchantmentInstance {
        id: "Sown".to_string(),
        amount: 1,
        // Simulate the original was played once already this combat.
        consumed_this_combat: true,
        state: Default::default(),
    });
    cs.allies[0].player.as_mut().unwrap().hand.cards.push(inst);

    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    // Anger's OnPlay: damage + CloneSourceCardToPile(Discard, copies=1).
    let result = cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
    assert!(matches!(result, sts2_sim::combat::PlayResult::Ok),
        "Anger play failed: {:?}", result);

    // Anger now in discard; the clone is also in discard.
    let discard = &cs.allies[0].player.as_ref().unwrap().discard.cards;
    let anger_copies: Vec<&CardInstance> = discard.iter()
        .filter(|c| c.id == "Anger").collect();
    assert_eq!(anger_copies.len(), 2,
        "Discard should hold 2 Anger cards (original + clone)");

    // Ordering: legacy Anger dispatch pushes the clone to discard
    // DURING OnPlay (step 5), then play_card's step-6 routing pushes
    // the original Anger to discard. So discard = [clone, original].
    // Find each by consumed flag — the clone is consumed=false (fresh).
    let clone = anger_copies.iter()
        .find(|c| c.enchantment.as_ref()
            .map(|e| !e.consumed_this_combat)
            .unwrap_or(false))
        .expect("CRITICAL: no Anger copy with fresh Sown enchantment found — \
            cloning regressed");
    let clone_ench = clone.enchantment.as_ref().unwrap();
    assert_eq!(clone_ench.id, "Sown");
    assert!(!clone_ench.consumed_this_combat,
        "CRITICAL: cloned card's Sown must have consumed_this_combat=false \
        — this is the RL training invariant the user flagged");
    // And the OTHER Anger in discard must be the consumed-flag-set original.
    let original = anger_copies.iter()
        .find(|c| c.enchantment.as_ref()
            .map(|e| e.consumed_this_combat)
            .unwrap_or(false))
        .expect("Original Anger should retain consumed=true on its Sown");
    assert!(original.enchantment.as_ref().unwrap().consumed_this_combat);
}

#[test]
fn dual_wield_clone_preserves_fresh_enchantment() {
    // DualWield picks an Attack/Power from hand and clones it.
    let mut cs = ironclad_combat();
    // Stage a Strike-with-Sown in hand for DualWield to pick.
    force_enchanted(&mut cs, "StrikeIronclad", "Sown", 1);
    // Mark its consumed flag pre-emptively to verify the clone resets.
    let strike_idx = cs.allies[0].player.as_ref().unwrap()
        .hand.cards.iter().position(|c| c.id == "StrikeIronclad").unwrap();
    cs.allies[0].player.as_mut().unwrap()
        .hand.cards[strike_idx].enchantment.as_mut().unwrap()
        .consumed_this_combat = true;
    // Stage DualWield itself.
    let dw = card::by_id("DualWield").expect("DualWield");
    cs.allies[0].player.as_mut().unwrap().hand.cards.push(
        CardInstance::from_card(dw, 0));

    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    // Play DualWield (target Self / None — it picks from hand).
    let dw_idx = cs.allies[0].player.as_ref().unwrap()
        .hand.cards.iter().position(|c| c.id == "DualWield").unwrap();
    cs.play_card(0, dw_idx, None);

    // After play, hand has the picked Strike + a clone of it. Both
    // should be Strikes with Sown enchantment; the clone must have
    // consumed_this_combat=false.
    let hand = &cs.allies[0].player.as_ref().unwrap().hand.cards;
    let strikes_with_sown: Vec<&CardInstance> = hand.iter()
        .filter(|c| c.id == "StrikeIronclad"
            && c.enchantment.as_ref().map(|e| e.id == "Sown").unwrap_or(false))
        .collect();
    assert!(strikes_with_sown.len() >= 1,
        "DualWield should leave at least one Sown-enchanted Strike (was {} found)",
        strikes_with_sown.len());
    // At least one should be the fresh clone (consumed=false).
    let fresh_count = strikes_with_sown.iter()
        .filter(|c| !c.enchantment.as_ref().unwrap().consumed_this_combat)
        .count();
    assert!(fresh_count >= 1,
        "DualWield clone must reset consumed_this_combat; original was \
        marked consumed=true, but no fresh copy found");
}

// ----------------------------------------------------------------------
// Section C: Modifier pipeline — Sharp damage, Nimble block, etc.
// ----------------------------------------------------------------------

#[test]
fn sharp_adds_amount_to_attack_damage() {
    let mut cs = ironclad_combat();
    force_enchanted(&mut cs, "StrikeIronclad", "Sharp", 5);
    let hp_before = cs.enemies[0].current_hp;
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
    // Strike: 6 damage. Sharp(5): +5 → 11.
    let dmg = hp_before - cs.enemies[0].current_hp;
    assert_eq!(dmg, 6 + 5,
        "Sharp(5) should add 5 to Strike's 6 damage (dealt {})", dmg);
}

#[test]
fn nimble_adds_amount_to_block() {
    let mut cs = ironclad_combat();
    let block_before = cs.allies[0].block;
    force_enchanted(&mut cs, "DefendIronclad", "Nimble", 3);
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 0, None);
    // Defend: 5 block. Nimble(3): +3 → 8.
    let block = cs.allies[0].block - block_before;
    assert_eq!(block, 5 + 3,
        "Nimble(3) should add 3 to Defend's 5 block (gained {})", block);
}

#[test]
fn corrupted_multiplies_attack_damage_by_1_5() {
    let mut cs = ironclad_combat();
    force_enchanted(&mut cs, "StrikeIronclad", "Corrupted", 0);
    let hp_before = cs.enemies[0].current_hp;
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
    // Strike: 6 damage × 1.5 = 9.
    let dmg = hp_before - cs.enemies[0].current_hp;
    assert_eq!(dmg, 9,
        "Corrupted should ×1.5 Strike's 6 damage to 9 (dealt {})", dmg);
}

// ----------------------------------------------------------------------
// Section D: Coverage report — which enchantments are wired.
// ----------------------------------------------------------------------

#[test]
fn enchantment_coverage_report() {
    // Lists each of the 22 enchantments and what's wired today.
    // This is a documentation test that prints status; doesn't fail.
    let wired_modifier_pipeline = ["Sharp", "Corrupted", "Nimble", "Vigorous"];
    let wired_onplay = ["Sown", "Swift", "Adroit", "Inky"];
    let keyword_only = ["Steady", "TezcatarasEmber", "SoulsPower",
                        "RoyallyApproved", "Goopy"];
    let needs_play_count_hook = ["Glam", "Spiral"];
    let needs_other_infra = ["Imbued", "PerfectFit", "SlumberingEssence",
                             "Slither", "Clone", "Momentum"];

    let all = ["Adroit","Clone","Corrupted","DeprecatedEnchantment","Glam",
               "Goopy","Imbued","Inky","Instinct","Momentum","Nimble",
               "PerfectFit","RoyallyApproved","Sharp","Slither",
               "SlumberingEssence","SoulsPower","Sown","Spiral","Steady",
               "Swift","TezcatarasEmber","Vigorous"];
    let total_wired = wired_modifier_pipeline.len()
        + wired_onplay.len()
        + keyword_only.len();
    eprintln!("\n========= ENCHANTMENT COVERAGE =========");
    eprintln!("Total enchantments in data:  {}", all.len());
    eprintln!("Wired (modifier pipeline):   {} {:?}",
        wired_modifier_pipeline.len(), wired_modifier_pipeline);
    eprintln!("Wired (OnPlay hook):         {} {:?}",
        wired_onplay.len(), wired_onplay);
    eprintln!("Wired (keyword-only at attach time): {} {:?}",
        keyword_only.len(), keyword_only);
    eprintln!("Not wired — play-count hook: {} {:?}",
        needs_play_count_hook.len(), needs_play_count_hook);
    eprintln!("Not wired — other infra:     {} {:?}",
        needs_other_infra.len(), needs_other_infra);
    eprintln!("Effectively wired: {}/{}", total_wired, all.len());
}

// ----------------------------------------------------------------------
// Section E: Direct test of apply_enchantment_on_play. Pure helper.
// ----------------------------------------------------------------------

#[test]
fn apply_enchantment_on_play_sown_grants_energy_directly() {
    let mut cs = ironclad_combat();
    cs.allies[0].player.as_mut().unwrap().energy = 3;
    let ench = EnchantmentInstance {
        id: "Sown".to_string(),
        amount: 2,
        consumed_this_combat: false,
        state: Default::default(),
    };
    apply_enchantment_on_play(&mut cs, 0, &ench, None);
    assert_eq!(cs.allies[0].player.as_ref().unwrap().energy, 5,
        "Direct Sown call should grant +2 energy");
}

#[test]
fn apply_enchantment_on_play_sown_skips_when_consumed() {
    let mut cs = ironclad_combat();
    cs.allies[0].player.as_mut().unwrap().energy = 3;
    let ench = EnchantmentInstance {
        id: "Sown".to_string(),
        amount: 2,
        consumed_this_combat: true,
        state: Default::default(), // already fired this combat
    };
    apply_enchantment_on_play(&mut cs, 0, &ench, None);
    assert_eq!(cs.allies[0].player.as_ref().unwrap().energy, 3,
        "Sown with consumed=true should be a no-op");
}
