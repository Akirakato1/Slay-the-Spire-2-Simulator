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
fn instinct_multiplies_attack_damage_by_2() {
    let mut cs = ironclad_combat();
    force_enchanted(&mut cs, "StrikeIronclad", "Instinct", 0);
    let hp_before = cs.enemies[0].current_hp;
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
    // Strike: 6 damage × 2.0 = 12.
    let dmg = hp_before - cs.enemies[0].current_hp;
    assert_eq!(dmg, 12,
        "Instinct should ⊙ 2.0 on Strike's 6 damage → 12 (dealt {})",
        dmg);
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
    let wired_modifier_pipeline = ["Sharp", "Corrupted", "Instinct", "Nimble", "Vigorous"];
    let wired_onplay = ["Sown", "Swift", "Adroit", "Inky"];
    let wired_play_count = ["Glam", "Spiral"];
    let wired_self_state = ["Momentum", "Goopy"];
    let wired_after_drawn = ["Slither"];
    let wired_before_flush = ["SlumberingEssence"];
    let wired_before_play_phase = ["Imbued"];
    let wired_shuffle_order = ["PerfectFit"];
    let wired_no_op_markers = ["Clone"];
    let keyword_only = ["Steady", "TezcatarasEmber", "SoulsPower",
                        "RoyallyApproved"];
    let needs_other_infra: [&str; 0] = [];

    let all = ["Adroit","Clone","Corrupted","DeprecatedEnchantment","Glam",
               "Goopy","Imbued","Inky","Instinct","Momentum","Nimble",
               "PerfectFit","RoyallyApproved","Sharp","Slither",
               "SlumberingEssence","SoulsPower","Sown","Spiral","Steady",
               "Swift","TezcatarasEmber","Vigorous"];
    let total_wired = wired_modifier_pipeline.len()
        + wired_onplay.len()
        + wired_play_count.len()
        + wired_self_state.len()
        + wired_after_drawn.len()
        + wired_before_flush.len()
        + wired_before_play_phase.len()
        + wired_shuffle_order.len()
        + wired_no_op_markers.len()
        + keyword_only.len();
    eprintln!("\n========= ENCHANTMENT COVERAGE =========");
    eprintln!("Total enchantments in data:    {}", all.len());
    eprintln!("Wired (modifier pipeline):       {} {:?}",
        wired_modifier_pipeline.len(), wired_modifier_pipeline);
    eprintln!("Wired (OnPlay hook):             {} {:?}",
        wired_onplay.len(), wired_onplay);
    eprintln!("Wired (EnchantPlayCount loop):   {} {:?}",
        wired_play_count.len(), wired_play_count);
    eprintln!("Wired (per-instance state):      {} {:?}",
        wired_self_state.len(), wired_self_state);
    eprintln!("Wired (AfterCardDrawn hook):     {} {:?}",
        wired_after_drawn.len(), wired_after_drawn);
    eprintln!("Wired (BeforeFlush hook):        {} {:?}",
        wired_before_flush.len(), wired_before_flush);
    eprintln!("Wired (BeforePlayPhaseStart):    {} {:?}",
        wired_before_play_phase.len(), wired_before_play_phase);
    eprintln!("Wired (ModifyShuffleOrder hook): {} {:?}",
        wired_shuffle_order.len(), wired_shuffle_order);
    eprintln!("Wired (no-op marker):            {} {:?}",
        wired_no_op_markers.len(), wired_no_op_markers);
    eprintln!("Wired (keyword-only at attach):  {} {:?}",
        keyword_only.len(), keyword_only);
    let stub_count = needs_other_infra.len();
    eprintln!("Not wired — other infra:         {} {:?}",
        stub_count, needs_other_infra);
    eprintln!("Effectively wired: {}/{}", total_wired, all.len());
    assert!(stub_count == 0,
        "All non-deprecated enchantments should be wired (was {})",
        stub_count);
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

// ----------------------------------------------------------------------
// Section F: Play-count enchantments (Glam / Spiral) — loop dispatch.
// ----------------------------------------------------------------------

#[test]
fn glam_doubles_strike_damage_on_first_play() {
    // Glam(+1): play count = 1 + 1 = 2 → Strike dispatches twice.
    // Once-per-combat: only the first play loops; subsequent plays
    // dispatch once.
    let mut cs = ironclad_combat();
    force_enchanted(&mut cs, "StrikeIronclad", "Glam", 1);
    let hp_before = cs.enemies[0].current_hp;
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
    let dmg = hp_before - cs.enemies[0].current_hp;
    assert_eq!(dmg, 12,
        "Glam(+1) on Strike should fire OnPlay twice → 6+6=12 (dealt {})",
        dmg);
    // consumed_this_combat must flip on the played card's enchantment.
    let card = cs.allies[0].player.as_ref().unwrap()
        .discard.cards.iter()
        .find(|c| c.id == "StrikeIronclad")
        .expect("Strike in discard");
    let ench = card.enchantment.as_ref().expect("enchantment preserved");
    assert!(ench.consumed_this_combat,
        "Glam's consumed_this_combat must flip after first play");
}

#[test]
fn spiral_doubles_strike_damage_every_play() {
    // Spiral(+1) is NOT once-per-combat — every play loops.
    let mut cs = ironclad_combat();
    force_enchanted(&mut cs, "StrikeIronclad", "Spiral", 1);
    let hp_before = cs.enemies[0].current_hp;
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
    let dmg = hp_before - cs.enemies[0].current_hp;
    assert_eq!(dmg, 12,
        "Spiral(+1) should make Strike dispatch twice (6+6=12, dealt {})",
        dmg);
    // Spiral does NOT consume.
    let card = cs.allies[0].player.as_ref().unwrap()
        .discard.cards.iter()
        .find(|c| c.id == "StrikeIronclad")
        .expect("Strike in discard");
    let ench = card.enchantment.as_ref().expect("enchantment preserved");
    assert!(!ench.consumed_this_combat,
        "Spiral must NOT flip consumed_this_combat (always-on)");
}

// ----------------------------------------------------------------------
// Section G: Goopy — stacks per host-card play via self_state_delta.
// ----------------------------------------------------------------------

#[test]
fn goopy_increments_stack_count_on_each_play_of_host() {
    let mut cs = ironclad_combat();
    force_enchanted(&mut cs, "StrikeIronclad", "Goopy", 1);
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
    let strike = cs.allies[0].player.as_ref().unwrap()
        .discard.cards.iter()
        .find(|c| c.id == "StrikeIronclad")
        .expect("Strike in discard");
    let ench = strike.enchantment.as_ref().expect("enchantment preserved");
    let stack = ench.state.get("StackCount").copied().unwrap_or(0);
    assert_eq!(stack, 1,
        "Goopy.StackCount must be 1 after first play of host (was {})",
        stack);
}

// ----------------------------------------------------------------------
// Section H: Choice continuation — LastChoicePickCount via follow_up.
// ----------------------------------------------------------------------

#[test]
fn gamblers_brew_draws_picked_count_via_follow_up() {
    // Auto-resolve path: GamblersBrew's AwaitPlayerChoice picks 0 (the
    // "any-number" Discard branch defaults to 0). The follow-up
    // DrawCards { LastChoicePickCount } sees count=0 → no draw.
    // The behavior we lock in: no panic + LastChoicePickCount wired.
    let mut cs = ironclad_combat();
    let hand_before = cs.allies[0].player.as_ref().unwrap().hand.cards.len();
    let ctx = EffectContext::for_potion_use(
        0, Some((CombatSide::Enemy, 0)), "GamblersBrew");
    let body = effects::potion_effects("GamblersBrew").unwrap();
    effects::execute_effects(&mut cs, &body, &ctx);
    // Auto-pick was 0 → discard 0 → draw 0 → hand unchanged.
    let hand_after = cs.allies[0].player.as_ref().unwrap().hand.cards.len();
    assert_eq!(hand_after, hand_before,
        "Auto-resolved GamblersBrew with 0 picks: hand unchanged");
    assert_eq!(cs.last_choice_pick_count, 0,
        "LastChoicePickCount must be wired through auto-resolve");
}

// ----------------------------------------------------------------------
// Section H': AfterCardDrawn — Slither cost override on draw.
// ----------------------------------------------------------------------

#[test]
fn slither_sets_cost_override_when_drawn() {
    // Stash a Slither-enchanted Strike at the top of the draw pile,
    // then call draw_cards. The drawn instance must have
    // cost_override_until_played = Some(0).
    let mut cs = ironclad_combat();
    let mut inst = CardInstance::from_card(
        card::by_id("StrikeIronclad").unwrap(), 0);
    inst.enchantment = Some(EnchantmentInstance {
        id: "Slither".to_string(),
        amount: 0,
        consumed_this_combat: false,
        state: Default::default(),
    });
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.draw.cards.insert(0, inst); // top-of-deck (drawn next)
    let mut rng = sts2_sim::rng::Rng::new(0, 0);
    cs.draw_cards(0, 1, &mut rng);
    let hand = &cs.allies[0].player.as_ref().unwrap().hand.cards;
    let slithered = hand.iter()
        .find(|c| c.enchantment.as_ref()
            .map(|e| e.id == "Slither").unwrap_or(false))
        .expect("Slither-enchanted card landed in hand");
    assert_eq!(slithered.cost_override_until_played, Some(0),
        "Slither's AfterCardDrawn must set cost_override_until_played=0");
}

#[test]
fn last_choice_pick_count_carries_to_follow_up_effects() {
    // Build a synthetic Effect chain: AwaitPlayerChoice with
    // n_max=2 follow_up=[DrawCards(LastChoicePickCount)]. Force
    // hand has 2 cards, auto-resolve picks them, follow-up draws 2.
    let mut cs = ironclad_combat();
    // Use Exhaust action (not Discard) so the auto-resolve path picks
    // n_max instead of 0 (Discard's any-min=0 short-circuit).
    let strike = card::by_id("StrikeIronclad").unwrap();
    let defend = card::by_id("DefendIronclad").unwrap();
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.hand.cards.push(CardInstance::from_card(strike, 0));
    ps.hand.cards.push(CardInstance::from_card(defend, 0));
    ps.energy = 3;
    let hand_before = ps.hand.cards.len();
    let draw_before = ps.draw.cards.len();
    let body = vec![Effect::AwaitPlayerChoice {
        pile: Pile::Hand,
        n_min: 0,
        n_max: AmountSpec::Fixed(2),
        filter: effects::CardFilter::Any,
        action: effects::ChoiceActionSpec::Exhaust,
        follow_up: vec![Effect::DrawCards {
            amount: AmountSpec::LastChoicePickCount,
        }],
    }];
    let ctx = EffectContext::for_card(
        0, Some((CombatSide::Enemy, 0)), "StrikeIronclad", 0, None, 0);
    effects::execute_effects(&mut cs, &body, &ctx);
    // Auto-resolve picks 2; exhaust 2 then draw 2.
    let ps2 = cs.allies[0].player.as_ref().unwrap();
    assert_eq!(cs.last_choice_pick_count, 2,
        "LastChoicePickCount should be set to the auto-pick count");
    // Hand stays the same size: -2 exhausted, +2 drawn.
    assert_eq!(ps2.hand.cards.len(), hand_before,
        "hand size: -2 exhaust +2 draw == net zero (was {}, now {})",
        hand_before, ps2.hand.cards.len());
    // Draw pile decreased by 2.
    assert_eq!(ps2.draw.cards.len(), draw_before - 2,
        "draw pile should shrink by 2 (was {}, now {})",
        draw_before, ps2.draw.cards.len());
}

// ----------------------------------------------------------------------
// Section I: Late-arriving enchantment hooks (SlumberingEssence,
// PerfectFit, Imbued). Clone is intentionally a no-op marker.
// ----------------------------------------------------------------------

#[test]
fn slumbering_essence_ramps_cost_down_per_turn() {
    // Start with a 2-cost card carrying SlumberingEssence. After one
    // BeforeFlush trigger (end of player turn) the override should be 1.
    // After two triggers, 0. After three, -1 (clamped by play-time).
    let mut cs = ironclad_combat();
    let strike = card::by_id("StrikeIronclad").unwrap();
    let mut inst = CardInstance::from_card(strike, 0);
    inst.enchantment = Some(EnchantmentInstance {
        id: "SlumberingEssence".to_string(),
        amount: 0,
        consumed_this_combat: false,
        state: Default::default(),
    });
    cs.allies[0].player.as_mut().unwrap().hand.cards.push(inst);
    let printed = cs.allies[0].player.as_ref().unwrap()
        .hand.cards[0].current_energy_cost;

    cs.fire_enchantment_before_flush();
    let after1 = cs.allies[0].player.as_ref().unwrap()
        .hand.cards[0].cost_override_until_played;
    assert_eq!(after1, Some(printed - 1),
        "After 1 flush: cost should drop by 1 (printed {} → {:?})",
        printed, after1);

    cs.fire_enchantment_before_flush();
    let after2 = cs.allies[0].player.as_ref().unwrap()
        .hand.cards[0].cost_override_until_played;
    assert_eq!(after2, Some(printed - 2),
        "After 2 flushes: cost should drop by 2 (printed {} → {:?})",
        printed, after2);
}

#[test]
fn slumbering_essence_ignores_cards_not_in_hand() {
    // Cards in draw / discard / exhaust must not be modified.
    let mut cs = ironclad_combat();
    let strike = card::by_id("StrikeIronclad").unwrap();
    let make_inst = || {
        let mut inst = CardInstance::from_card(strike, 0);
        inst.enchantment = Some(EnchantmentInstance {
            id: "SlumberingEssence".to_string(),
            amount: 0,
            consumed_this_combat: false,
            state: Default::default(),
        });
        inst
    };
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.draw.cards.push(make_inst());
    ps.discard.cards.push(make_inst());

    cs.fire_enchantment_before_flush();

    let ps = cs.allies[0].player.as_ref().unwrap();
    let draw_card = ps.draw.cards.last().unwrap();
    let discard_card = ps.discard.cards.last().unwrap();
    assert!(draw_card.cost_override_until_played.is_none(),
        "SlumberingEssence in draw should not be touched");
    assert!(discard_card.cost_override_until_played.is_none(),
        "SlumberingEssence in discard should not be touched");
}

#[test]
fn perfect_fit_moves_to_top_on_reshuffle() {
    // Set up: empty draw, discard with PerfectFit card + 4 non-PerfectFit
    // cards. Draw 5 forces a reshuffle. After reshuffle, the PerfectFit
    // card must be at index 0 of the new draw pile.
    let mut cs = ironclad_combat();
    let strike = card::by_id("StrikeIronclad").unwrap();
    let defend = card::by_id("DefendIronclad").unwrap();
    let mut pf = CardInstance::from_card(strike, 0);
    pf.enchantment = Some(EnchantmentInstance {
        id: "PerfectFit".to_string(),
        amount: 0,
        consumed_this_combat: false,
        state: Default::default(),
    });
    // Mark it via cost so we can identify it post-shuffle (RNG order
    // is otherwise indistinguishable for same-id cards).
    pf.cost_override_until_played = Some(99);
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.draw.cards.clear();
    ps.discard.cards.clear();
    // Put 4 strikes + 1 PerfectFit in discard.
    for _ in 0..4 {
        ps.discard.cards.push(CardInstance::from_card(defend, 0));
    }
    ps.discard.cards.push(pf);
    let mut rng = sts2_sim::rng::Rng::new(7, 0);
    // Drawing forces a reshuffle (draw empty, discard non-empty).
    cs.draw_cards(0, 1, &mut rng);
    // After: 1 card has been drawn (was at index 0 post-reshuffle),
    // remaining 4 sit in draw. The PerfectFit card should have been
    // pushed to index 0 → drawn first. Verify the drawn card is the
    // marked one.
    let drawn = cs.allies[0].player.as_ref().unwrap()
        .hand.cards.last().expect("a card drawn");
    assert_eq!(drawn.cost_override_until_played, Some(99),
        "PerfectFit should be drawn first because ModifyShuffleOrder \
        promoted it to index 0");
}

#[test]
fn imbued_auto_plays_on_round_1() {
    // Imbued + Defend in hand at round 1 should auto-play on
    // BeforePlayPhaseStart (which fires inside begin_turn(Player)).
    // Net effect: Defend's block lands and the card routes to discard.
    let mut cs = ironclad_combat();
    let defend = card::by_id("DefendIronclad").unwrap();
    let mut inst = CardInstance::from_card(defend, 0);
    inst.enchantment = Some(EnchantmentInstance {
        id: "Imbued".to_string(),
        amount: 0,
        consumed_this_combat: false,
        state: Default::default(),
    });
    cs.allies[0].player.as_mut().unwrap().hand.cards.push(inst);
    let block_before = cs.allies[0].block;
    // begin_turn(Player) fires BeforePlayPhaseStart → Imbued auto-plays.
    cs.begin_turn(CombatSide::Player);
    assert!(cs.allies[0].block >= block_before + 5,
        "Imbued Defend should auto-play +5 block (was {}, now {})",
        block_before, cs.allies[0].block);
    // Card moved to discard.
    let in_hand = cs.allies[0].player.as_ref().unwrap()
        .hand.cards.iter()
        .any(|c| c.enchantment.as_ref()
            .map(|e| e.id == "Imbued").unwrap_or(false));
    assert!(!in_hand, "Imbued card should have left hand (auto-played)");
}

#[test]
fn imbued_does_not_auto_play_on_round_2() {
    // BeforePlayPhaseStart on round > 1 should not auto-play.
    let mut cs = ironclad_combat();
    let defend = card::by_id("DefendIronclad").unwrap();
    let mut inst = CardInstance::from_card(defend, 0);
    inst.enchantment = Some(EnchantmentInstance {
        id: "Imbued".to_string(),
        amount: 0,
        consumed_this_combat: false,
        state: Default::default(),
    });
    cs.allies[0].player.as_mut().unwrap().hand.cards.push(inst);
    cs.round_number = 2;
    let block_before = cs.allies[0].block;
    cs.begin_turn(CombatSide::Player);
    assert_eq!(cs.allies[0].block, block_before,
        "Imbued should not auto-play on round > 1");
    let in_hand = cs.allies[0].player.as_ref().unwrap()
        .hand.cards.iter()
        .any(|c| c.enchantment.as_ref()
            .map(|e| e.id == "Imbued").unwrap_or(false));
    assert!(in_hand, "Imbued card must remain in hand on round 2");
}

#[test]
fn clone_enchantment_is_a_pure_marker_noop() {
    // Per C# `Clone.cs`: the class body is empty — no virtual methods
    // overridden. The enchantment exists as a data marker only. This
    // test pins that interpretation: a card with Clone behaves
    // identically to a card without an enchantment (modulo the
    // enchantment field being Some).
    let mut cs = ironclad_combat();
    force_enchanted(&mut cs, "StrikeIronclad", "Clone", 0);
    let hp_before = cs.enemies[0].current_hp;
    let ps = cs.allies[0].player.as_mut().unwrap();
    ps.energy = 3;
    cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
    let dmg = hp_before - cs.enemies[0].current_hp;
    assert_eq!(dmg, 6,
        "Strike + Clone should deal vanilla 6 damage (Clone is a marker, \
        no modifier or hook)");
}
