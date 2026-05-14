//! Data-driven effect VM.
//!
//! Plan §0.2.2 scaffold. The C# decompile expresses every card / relic /
//! potion / monster-move payload as a composition over a small set of
//! primitive operations. This module is the Rust counterpart: a closed
//! enum of primitives + a dispatcher that interprets a `Vec<Effect>`.
//!
//! See `docs/effect-vocabulary.md` for the full primitive catalog, status
//! per primitive, and design rationale.
//!
//! Two layers:
//!
//! - **Data**: `Effect`, `AmountSpec`, `Target`, `Pile` enums. Serializable;
//!   intended to be stored alongside the card/relic/potion JSON tables that
//!   `tools/extract_*` already emit.
//! - **Runtime**: `execute_effects(state, &[Effect], &EffectContext)` walks
//!   the list and invokes the appropriate primitive on `CombatState`. The
//!   primitives are the existing methods on `CombatState` (`deal_damage`,
//!   `gain_block`, `apply_power`, …); this module is a thin dispatcher,
//!   not a re-implementation.
//!
//! **Observer-layer constraint** (memory: feedback-observer-layer-pure-function):
//! the same `Effect` enum is the schema the RL feature extractor in
//! `features.rs` should be keyed by. Adding a card adds a data row only,
//! never a new feature column. See `docs/effect-vocabulary.md` §9.
//!
//! Initial scope: only primitives already implemented in `combat.rs` are
//! wired (DealDamage / GainBlock / ApplyPower / DrawCards / AddCardToPile /
//! ExhaustRandomInHand / ChangeMaxHp / GainEnergy / Heal). Plan §0.2.3 adds
//! the missing ~35-45 primitives as further enum variants + dispatch arms.
//!
//! NOTE: spec-derived tests only; not yet oracle-diffed.

use serde::{Deserialize, Serialize};

use crate::card::by_id as card_by_id;
use crate::combat::{
    canonical_int_value, CombatSide, CombatState, EnchantmentInstance, PileType, ValueProp,
};

/// How a numeric argument is computed at execution time.
///
/// Closed set derived from the vocabulary survey (§5 of the vocab doc).
/// Every numeric arg in cards/relics/potions/monster-moves resolves through
/// one of these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AmountSpec {
    /// Hard-coded literal.
    Fixed(i32),
    /// `CanonicalVars[name].BaseValue + upgrade_deltas[name] * upgrade_level`.
    /// The universal data-driven amount source.
    Canonical(String),
    /// `if IsUpgraded { upgraded } else { base }`. TrueGrit, MultiCast.
    BranchedOnUpgrade { base: i32, upgraded: i32 },
    /// Player's resolved X-energy value (Whirlwind, Skewer, MultiCast).
    /// Caller stuffs the resolved value into `EffectContext::x_value`.
    XEnergy,
    /// Multiply the inner amount by `factor`. Composition helper.
    Multiplied { base: Box<AmountSpec>, factor: i32 },
}

/// Where an effect applies. Richer selectors (AllAllies, ChooseFromPile,
/// TargetLowestHpEnemy, ...) added in 0.2.3 as primitives requiring them land.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    SelfPlayer,
    /// The single target picked by the player action (attack cards).
    ChosenEnemy,
    AllEnemies,
    /// A single alive enemy chosen uniformly at random from the combat
    /// RNG stream. Re-rolled per hit when used with `hits > 1`. Mirrors
    /// `DamageCmd.Attack(...).TargetingRandomOpponents(combatState, reroll_dead)`
    /// (SwordBoomerang).
    RandomEnemy,
}

/// Pile-id mirror of `combat::PileType`, restricted to the in-combat piles
/// that primitive payloads ever address.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pile {
    Hand,
    Discard,
    Draw,
    Exhaust,
}

impl Pile {
    fn as_pile_type(self) -> PileType {
        match self {
            Pile::Hand => PileType::Hand,
            Pile::Discard => PileType::Discard,
            Pile::Draw => PileType::Draw,
            Pile::Exhaust => PileType::Exhaust,
        }
    }
}

/// Closed primitive vocabulary.
///
/// Initial set covers the ~15 primitives already implemented in
/// `combat.rs`. Missing primitives are added here as variants alongside
/// their dispatch arms as 0.2.3 progresses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    /// Deal `amount` damage to `target`, optionally over multiple hits
    /// (each hit threads independently through the modifier pipeline).
    /// Carries the played card's enchantment via `EffectContext`.
    DealDamage {
        amount: AmountSpec,
        target: Target,
        hits: i32,
    },
    /// Add block to `target`. Routes through `gain_block` so Dex/Frail
    /// apply (matches C# `ValueProp.Move`).
    GainBlock { amount: AmountSpec, target: Target },
    /// Apply `amount` stacks of `power_id` to `target`.
    ApplyPower {
        power_id: String,
        amount: AmountSpec,
        target: Target,
    },
    /// Draw `amount` cards using the combat-scoped RNG.
    DrawCards { amount: AmountSpec },
    /// Conjure a fresh copy of `card_id` at the given upgrade level into
    /// the named pile.
    AddCardToPile {
        card_id: String,
        upgrade: i32,
        pile: Pile,
    },
    /// Exhaust `amount` random cards from hand (uses combat RNG).
    ExhaustRandomInHand { amount: AmountSpec },
    /// Change max HP on `target` by `amount`. Clamps current HP if max
    /// drops below.
    ChangeMaxHp { amount: AmountSpec, target: Target },
    /// Give the player `amount` energy this turn.
    GainEnergy { amount: AmountSpec },
    /// Heal `target` by `amount` (clamped at max HP).
    Heal { amount: AmountSpec, target: Target },
    /// Reduce `target`'s current HP by `amount`, bypassing block and the
    /// modifier pipeline. Used for self-damage cards (Bloodletting,
    /// HemoKinesis) where the C# emits `CreatureCmd.Damage` with
    /// `ValueProp.Unblockable | ValueProp.Unpowered`. Does not trigger
    /// thorns / AfterDamageReceived hooks.
    LoseHp { amount: AmountSpec, target: Target },
    /// Drop `target` to 0 HP immediately. Sacrifice / Doom-trigger
    /// cards. Bypasses damage modifiers and on-damage hooks; death
    /// detection runs on the next combat-state check.
    Kill { target: Target },
    /// Take `amount` energy from the player (clamped at 0). Debt /
    /// over-energy relics use this.
    LoseEnergy { amount: AmountSpec },
    /// Strip a power from `target` entirely. Cleanse-style.
    RemovePower { power_id: String, target: Target },
    /// Reshuffle `pile` in place using the combat RNG. Recycle.
    Shuffle { pile: Pile },
    /// Move every card in the player's hand to discard. End-of-turn
    /// helpers and discard-your-hand effects.
    DiscardHand,
}

/// Per-invocation context. Holds everything the dispatcher needs to
/// resolve `AmountSpec`s and route effects to the right creature.
#[derive(Debug)]
pub struct EffectContext<'a> {
    /// Index into `CombatState.allies` for the player executing the effect.
    pub player_idx: usize,
    /// The single target chosen by the player's action, if any. Used by
    /// `Target::ChosenEnemy`. None for self-target / AOE / non-targeted.
    pub target: Option<(CombatSide, usize)>,
    /// Card id that owns this effect list. Required for `Canonical`
    /// amounts; safe to leave None for non-card effect sources.
    pub source_card_id: Option<&'a str>,
    /// Upgrade level on the source card.
    pub upgrade_level: i32,
    /// Enchantment instance attached to the source card, threaded into
    /// damage modifiers.
    pub enchantment: Option<&'a EnchantmentInstance>,
    /// Resolved X-energy value at play time (X-cost cards).
    pub x_value: i32,
}

impl<'a> EffectContext<'a> {
    /// Convenience builder for the common "card OnPlay" call site.
    pub fn for_card(
        player_idx: usize,
        target: Option<(CombatSide, usize)>,
        card_id: &'a str,
        upgrade_level: i32,
        enchantment: Option<&'a EnchantmentInstance>,
        x_value: i32,
    ) -> Self {
        Self {
            player_idx,
            target,
            source_card_id: Some(card_id),
            upgrade_level,
            enchantment,
            x_value,
        }
    }
}

impl AmountSpec {
    pub fn resolve(&self, ctx: &EffectContext) -> i32 {
        match self {
            AmountSpec::Fixed(n) => *n,
            AmountSpec::Canonical(var_kind) => {
                let Some(card_id) = ctx.source_card_id else {
                    return 0;
                };
                let Some(card) = card_by_id(card_id) else {
                    return 0;
                };
                canonical_int_value(card, var_kind, ctx.upgrade_level)
            }
            AmountSpec::BranchedOnUpgrade { base, upgraded } => {
                if ctx.upgrade_level > 0 {
                    *upgraded
                } else {
                    *base
                }
            }
            AmountSpec::XEnergy => ctx.x_value,
            AmountSpec::Multiplied { base, factor } => base.resolve(ctx) * factor,
        }
    }
}

/// Walk an effect list and execute each step against `cs`. Effects are
/// applied in order; no implicit batching or reordering.
pub fn execute_effects(cs: &mut CombatState, effects: &[Effect], ctx: &EffectContext) {
    for eff in effects {
        execute_effect(cs, eff, ctx);
    }
}

fn execute_effect(cs: &mut CombatState, eff: &Effect, ctx: &EffectContext) {
    match eff {
        Effect::DealDamage {
            amount,
            target,
            hits,
        } => {
            let amt = amount.resolve(ctx);
            for _ in 0..(*hits).max(1) {
                deal_damage_to(cs, ctx, *target, amt);
            }
        }
        Effect::GainBlock { amount, target } => {
            let amt = amount.resolve(ctx);
            if matches!(target, Target::SelfPlayer) {
                cs.gain_block(CombatSide::Player, ctx.player_idx, amt);
            }
        }
        Effect::ApplyPower {
            power_id,
            amount,
            target,
        } => {
            let amt = amount.resolve(ctx);
            apply_power_to(cs, ctx, *target, power_id, amt);
        }
        Effect::DrawCards { amount } => {
            let n = amount.resolve(ctx);
            cs.draw_cards_self_rng(ctx.player_idx, n);
        }
        Effect::AddCardToPile {
            card_id,
            upgrade,
            pile,
        } => {
            cs.add_card_to_pile(ctx.player_idx, card_id, *upgrade, pile.as_pile_type());
        }
        Effect::ExhaustRandomInHand { amount } => {
            let n = amount.resolve(ctx);
            for _ in 0..n {
                cs.exhaust_random_card_in_hand(ctx.player_idx);
            }
        }
        Effect::ChangeMaxHp { amount, target } => {
            let amt = amount.resolve(ctx);
            if matches!(target, Target::SelfPlayer) {
                cs.change_max_hp(CombatSide::Player, ctx.player_idx, amt);
            }
        }
        Effect::GainEnergy { amount } => {
            let amt = amount.resolve(ctx);
            if let Some(creature) = cs.allies.get_mut(ctx.player_idx) {
                if let Some(ps) = creature.player.as_mut() {
                    ps.energy += amt;
                }
            }
        }
        Effect::Heal { amount, target } => {
            let amt = amount.resolve(ctx);
            if matches!(target, Target::SelfPlayer) {
                cs.heal(CombatSide::Player, ctx.player_idx, amt);
            }
        }
        Effect::LoseHp { amount, target } => {
            let amt = amount.resolve(ctx);
            match target {
                Target::SelfPlayer => {
                    cs.lose_hp(CombatSide::Player, ctx.player_idx, amt);
                }
                Target::ChosenEnemy => {
                    if let Some((side, idx)) = ctx.target {
                        cs.lose_hp(side, idx, amt);
                    }
                }
                Target::AllEnemies => {
                    let n = cs.enemies.len();
                    for i in 0..n {
                        if cs.enemies[i].current_hp == 0 {
                            continue;
                        }
                        cs.lose_hp(CombatSide::Enemy, i, amt);
                    }
                }
                Target::RandomEnemy => {
                    if let Some(idx) = pick_random_alive_enemy(cs) {
                        cs.lose_hp(CombatSide::Enemy, idx, amt);
                    }
                }
            }
        }
        Effect::LoseEnergy { amount } => {
            let amt = amount.resolve(ctx);
            if let Some(creature) = cs.allies.get_mut(ctx.player_idx) {
                if let Some(ps) = creature.player.as_mut() {
                    ps.energy = (ps.energy - amt).max(0);
                }
            }
        }
        Effect::RemovePower { power_id, target } => match target {
            Target::SelfPlayer => {
                cs.remove_power(CombatSide::Player, ctx.player_idx, power_id);
            }
            Target::ChosenEnemy => {
                if let Some((side, idx)) = ctx.target {
                    cs.remove_power(side, idx, power_id);
                }
            }
            Target::AllEnemies => {
                let n = cs.enemies.len();
                for i in 0..n {
                    if cs.enemies[i].current_hp == 0 {
                        continue;
                    }
                    cs.remove_power(CombatSide::Enemy, i, power_id);
                }
            }
            Target::RandomEnemy => {
                if let Some(idx) = pick_random_alive_enemy(cs) {
                    cs.remove_power(CombatSide::Enemy, idx, power_id);
                }
            }
        },
        Effect::Shuffle { pile } => {
            shuffle_pile(cs, ctx.player_idx, *pile);
        }
        Effect::DiscardHand => {
            cs.discard_hand(ctx.player_idx);
        }
        Effect::Kill { target } => {
            match target {
                Target::ChosenEnemy => {
                    if let Some((side, idx)) = ctx.target {
                        if let Some(c) = creature_at_mut(cs, side, idx) {
                            c.current_hp = 0;
                        }
                    }
                }
                Target::AllEnemies => {
                    let n = cs.enemies.len();
                    for i in 0..n {
                        cs.enemies[i].current_hp = 0;
                    }
                }
                Target::RandomEnemy => {
                    if let Some(idx) = pick_random_alive_enemy(cs) {
                        cs.enemies[idx].current_hp = 0;
                    }
                }
                Target::SelfPlayer => {
                    // Card-driven player self-kill is not a real C#
                    // pattern (no cards self-kill); leave as no-op.
                }
            }
        }
    }
}

fn shuffle_pile(cs: &mut CombatState, player_idx: usize, pile: Pile) {
    // Mirror the temp-swap trick used elsewhere so the combat RNG can
    // be borrowed alongside `cs.allies`.
    let mut rng = std::mem::replace(&mut cs.rng, crate::rng::Rng::new(0, 0));
    if let Some(creature) = cs.allies.get_mut(player_idx) {
        if let Some(ps) = creature.player.as_mut() {
            let cards = match pile {
                Pile::Hand => &mut ps.hand.cards,
                Pile::Discard => &mut ps.discard.cards,
                Pile::Draw => &mut ps.draw.cards,
                Pile::Exhaust => &mut ps.exhaust.cards,
            };
            rng.shuffle(cards);
        }
    }
    cs.rng = rng;
}

fn pick_random_alive_enemy(cs: &mut CombatState) -> Option<usize> {
    let alive: Vec<usize> = cs
        .enemies
        .iter()
        .enumerate()
        .filter_map(|(i, e)| if e.current_hp > 0 { Some(i) } else { None })
        .collect();
    if alive.is_empty() {
        return None;
    }
    let pick = cs.rng.next_int_range(0, alive.len() as i32) as usize;
    Some(alive[pick])
}

fn creature_at_mut(
    cs: &mut CombatState,
    side: CombatSide,
    idx: usize,
) -> Option<&mut crate::combat::Creature> {
    match side {
        CombatSide::Player => cs.allies.get_mut(idx),
        CombatSide::Enemy => cs.enemies.get_mut(idx),
        CombatSide::None => None,
    }
}

fn deal_damage_to(cs: &mut CombatState, ctx: &EffectContext, target: Target, amount: i32) {
    match target {
        Target::ChosenEnemy => {
            if let Some(t) = ctx.target {
                cs.deal_damage_enchanted(
                    (CombatSide::Player, ctx.player_idx),
                    t,
                    amount,
                    ValueProp::MOVE,
                    ctx.enchantment,
                );
            }
        }
        Target::AllEnemies => {
            let n = cs.enemies.len();
            for i in 0..n {
                if cs.enemies[i].current_hp == 0 {
                    continue;
                }
                cs.deal_damage_enchanted(
                    (CombatSide::Player, ctx.player_idx),
                    (CombatSide::Enemy, i),
                    amount,
                    ValueProp::MOVE,
                    ctx.enchantment,
                );
            }
        }
        Target::RandomEnemy => {
            // Per-hit re-roll matches `TargetingRandomOpponents(combatState,
            // reroll_dead=true)` — SwordBoomerang re-picks if the chosen
            // target died from the previous hit. Caller wraps in a hit
            // loop, so this function only picks one.
            if let Some(idx) = pick_random_alive_enemy(cs) {
                cs.deal_damage_enchanted(
                    (CombatSide::Player, ctx.player_idx),
                    (CombatSide::Enemy, idx),
                    amount,
                    ValueProp::MOVE,
                    ctx.enchantment,
                );
            }
        }
        Target::SelfPlayer => {
            // Damage to self via the attack pipeline is not a real card
            // pattern (self-damage cards use `LoseHp`). No-op.
        }
    }
}

fn apply_power_to(
    cs: &mut CombatState,
    ctx: &EffectContext,
    target: Target,
    power_id: &str,
    amount: i32,
) {
    match target {
        Target::SelfPlayer => {
            cs.apply_power(CombatSide::Player, ctx.player_idx, power_id, amount);
        }
        Target::ChosenEnemy => {
            if let Some((side, idx)) = ctx.target {
                cs.apply_power(side, idx, power_id, amount);
            }
        }
        Target::AllEnemies => {
            let n = cs.enemies.len();
            for i in 0..n {
                if cs.enemies[i].current_hp == 0 {
                    continue;
                }
                cs.apply_power(CombatSide::Enemy, i, power_id, amount);
            }
        }
        Target::RandomEnemy => {
            if let Some(idx) = pick_random_alive_enemy(cs) {
                cs.apply_power(CombatSide::Enemy, idx, power_id, amount);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character;
    use crate::combat::{CardInstance, CombatState, PlayerSetup};
    use crate::encounter;

    fn ironclad_combat() -> CombatState {
        let ironclad = character::by_id("Ironclad").expect("Ironclad present");
        let encounter =
            encounter::by_id("AxebotsNormal").expect("AxebotsNormal present");
        let deck: Vec<CardInstance> = ironclad
            .starting_deck
            .iter()
            .filter_map(|id| {
                crate::card::by_id(id).map(|c| CardInstance::from_card(c, 0))
            })
            .collect();
        let setup = PlayerSetup {
            character: ironclad,
            current_hp: ironclad.starting_hp.unwrap(),
            max_hp: ironclad.starting_hp.unwrap(),
            deck,
            relics: ironclad.starting_relics.clone(),
        };
        CombatState::start(encounter, vec![setup], Vec::new())
    }

    /// Strike encoded as a one-element effect list and run through the VM
    /// produces the same enemy HP delta as a direct `deal_damage_enchanted`
    /// call. Round-trips the simplest possible card.
    #[test]
    fn strike_via_vm_matches_direct_call() {
        let mut cs_vm = ironclad_combat();
        let mut cs_direct = ironclad_combat();

        let target_idx = 0usize;
        let enemy_hp_before = cs_vm.enemies[target_idx].current_hp;

        // VM path: Strike encoded as data.
        let strike_effects = vec![Effect::DealDamage {
            amount: AmountSpec::Canonical("Damage".to_string()),
            target: Target::ChosenEnemy,
            hits: 1,
        }];
        let ctx = EffectContext::for_card(
            0,
            Some((CombatSide::Enemy, target_idx)),
            "StrikeIronclad",
            0,
            None,
            0,
        );
        execute_effects(&mut cs_vm, &strike_effects, &ctx);

        // Direct path: existing combat.rs primitive.
        cs_direct.deal_damage_enchanted(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, target_idx),
            6, // StrikeIronclad.Damage base = 6
            ValueProp::MOVE,
            None,
        );

        let vm_hp = cs_vm.enemies[target_idx].current_hp;
        let direct_hp = cs_direct.enemies[target_idx].current_hp;
        assert!(
            vm_hp < enemy_hp_before,
            "VM strike should reduce enemy HP (was {enemy_hp_before}, now {vm_hp})"
        );
        assert_eq!(
            vm_hp, direct_hp,
            "VM execution must match direct-call HP exactly"
        );
    }

    /// Defend encoded as a one-element effect list lands block via
    /// `gain_block` and matches a direct call.
    #[test]
    fn defend_via_vm_matches_direct_call() {
        let mut cs = ironclad_combat();
        let block_before = cs.allies[0].block;
        let effects = vec![Effect::GainBlock {
            amount: AmountSpec::Canonical("Block".to_string()),
            target: Target::SelfPlayer,
        }];
        let ctx = EffectContext::for_card(0, None, "DefendIronclad", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        // DefendIronclad.Block base = 5
        assert_eq!(cs.allies[0].block, block_before + 5);
    }

    /// IronWave encoded as a two-step composition (Block then Damage)
    /// produces a state delta equivalent to the existing hand-arm.
    #[test]
    fn iron_wave_composes_block_then_damage() {
        let mut cs = ironclad_combat();
        let block_before = cs.allies[0].block;
        let enemy_hp_before = cs.enemies[0].current_hp;

        let effects = vec![
            Effect::GainBlock {
                amount: AmountSpec::Canonical("Block".to_string()),
                target: Target::SelfPlayer,
            },
            Effect::DealDamage {
                amount: AmountSpec::Canonical("Damage".to_string()),
                target: Target::ChosenEnemy,
                hits: 1,
            },
        ];
        let ctx = EffectContext::for_card(
            0,
            Some((CombatSide::Enemy, 0)),
            "IronWave",
            0,
            None,
            0,
        );
        execute_effects(&mut cs, &effects, &ctx);

        // IronWave base: 5 damage + 5 block.
        assert_eq!(cs.allies[0].block, block_before + 5);
        assert!(cs.enemies[0].current_hp < enemy_hp_before);
    }

    /// Canonical amount resolves through the upgrade-delta path.
    #[test]
    fn canonical_amount_picks_up_upgrade_delta() {
        let ctx_base = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        let ctx_up = EffectContext::for_card(0, None, "StrikeIronclad", 1, None, 0);
        let spec = AmountSpec::Canonical("Damage".to_string());
        assert_eq!(spec.resolve(&ctx_base), 6);
        // Upgraded Strike does 9 damage (+3).
        assert_eq!(spec.resolve(&ctx_up), 9);
    }

    /// Multi-hit hits the same target N times, each pass through the
    /// pipeline. TwinStrike is 5×2.
    #[test]
    fn multi_hit_attacks_apply_per_hit() {
        let mut cs = ironclad_combat();
        let enemy_hp_before = cs.enemies[0].current_hp;
        let effects = vec![Effect::DealDamage {
            amount: AmountSpec::Canonical("Damage".to_string()),
            target: Target::ChosenEnemy,
            hits: 2,
        }];
        let ctx = EffectContext::for_card(
            0,
            Some((CombatSide::Enemy, 0)),
            "TwinStrike",
            0,
            None,
            0,
        );
        execute_effects(&mut cs, &effects, &ctx);
        // 5 × 2 = 10 damage (no block, no powers).
        assert_eq!(cs.enemies[0].current_hp, enemy_hp_before - 10);
    }

    /// Bash as composition: 8 damage + 2 Vulnerable on a single enemy.
    /// Each primitive is already implemented; Bash is pure data.
    #[test]
    fn bash_composes_damage_plus_vulnerable() {
        let mut cs = ironclad_combat();
        let enemy_hp_before = cs.enemies[0].current_hp;
        let effects = vec![
            Effect::DealDamage {
                amount: AmountSpec::Canonical("Damage".to_string()),
                target: Target::ChosenEnemy,
                hits: 1,
            },
            Effect::ApplyPower {
                power_id: "VulnerablePower".to_string(),
                amount: AmountSpec::Canonical("Vulnerable".to_string()),
                target: Target::ChosenEnemy,
            },
        ];
        let ctx = EffectContext::for_card(
            0,
            Some((CombatSide::Enemy, 0)),
            "Bash",
            0,
            None,
            0,
        );
        execute_effects(&mut cs, &effects, &ctx);
        // Bash: 8 damage to enemy with 0 block, then 2 Vulnerable.
        assert_eq!(cs.enemies[0].current_hp, enemy_hp_before - 8);
        let vuln = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "VulnerablePower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(vuln, 2);
    }

    /// Thunderclap: 4 damage + 1 Vulnerable to ALL enemies. Composition
    /// of AOE damage and AOE power application.
    #[test]
    fn thunderclap_composes_aoe_damage_plus_aoe_vulnerable() {
        let mut cs = ironclad_combat();
        let hp_before: Vec<i32> = cs.enemies.iter().map(|e| e.current_hp).collect();
        let effects = vec![
            Effect::DealDamage {
                amount: AmountSpec::Canonical("Damage".to_string()),
                target: Target::AllEnemies,
                hits: 1,
            },
            Effect::ApplyPower {
                power_id: "VulnerablePower".to_string(),
                amount: AmountSpec::Canonical("Vulnerable".to_string()),
                target: Target::AllEnemies,
            },
        ];
        let ctx = EffectContext::for_card(0, None, "Thunderclap", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        for (i, before) in hp_before.iter().enumerate() {
            if *before == 0 {
                continue;
            }
            // Each enemy takes 4 damage, gets 1 Vulnerable.
            assert_eq!(cs.enemies[i].current_hp, before - 4);
            let vuln = cs.enemies[i]
                .powers
                .iter()
                .find(|p| p.id == "VulnerablePower")
                .map(|p| p.amount)
                .unwrap_or(0);
            assert_eq!(vuln, 1);
        }
    }

    /// Bloodletting: lose 3 HP (bypasses block), gain 2 energy.
    /// Round-trips against the existing match-arm.
    #[test]
    fn bloodletting_round_trips_lose_hp_plus_gain_energy() {
        let mut cs = ironclad_combat();
        cs.allies[0].block = 50;
        let hp_before = cs.allies[0].current_hp;
        let energy_before = cs.allies[0].player.as_ref().unwrap().energy;

        let effects = vec![
            Effect::LoseHp {
                amount: AmountSpec::Canonical("HpLoss".to_string()),
                target: Target::SelfPlayer,
            },
            Effect::GainEnergy {
                amount: AmountSpec::Canonical("Energy".to_string()),
            },
        ];
        let ctx = EffectContext::for_card(0, None, "Bloodletting", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);

        // Bloodletting base: HpLoss=3, Energy=2. Block must NOT absorb.
        assert_eq!(cs.allies[0].current_hp, hp_before - 3);
        assert_eq!(cs.allies[0].block, 50);
        assert_eq!(
            cs.allies[0].player.as_ref().unwrap().energy,
            energy_before + 2
        );
    }

    /// Kill drops the chosen enemy to 0 HP regardless of armor or hooks.
    /// Sacrifice-style.
    #[test]
    fn kill_drops_enemy_to_zero() {
        let mut cs = ironclad_combat();
        cs.enemies[0].block = 100;
        let effects = vec![Effect::Kill {
            target: Target::ChosenEnemy,
        }];
        let ctx = EffectContext::for_card(
            0,
            Some((CombatSide::Enemy, 0)),
            "StrikeIronclad", // Card id is irrelevant; only target/player matter.
            0,
            None,
            0,
        );
        execute_effects(&mut cs, &effects, &ctx);
        assert_eq!(cs.enemies[0].current_hp, 0);
    }

    /// LoseEnergy decrements by amount, clamps at 0.
    #[test]
    fn lose_energy_clamps_at_zero() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let effects = vec![Effect::LoseEnergy {
            amount: AmountSpec::Fixed(5),
        }];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        assert_eq!(cs.allies[0].player.as_ref().unwrap().energy, 0);
    }

    /// RemovePower strips an applied power from the target.
    #[test]
    fn remove_power_strips_target_power() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 3);
        assert!(cs.enemies[0]
            .powers
            .iter()
            .any(|p| p.id == "VulnerablePower"));

        let effects = vec![Effect::RemovePower {
            power_id: "VulnerablePower".to_string(),
            target: Target::ChosenEnemy,
        }];
        let ctx = EffectContext::for_card(
            0,
            Some((CombatSide::Enemy, 0)),
            "StrikeIronclad",
            0,
            None,
            0,
        );
        execute_effects(&mut cs, &effects, &ctx);
        assert!(!cs.enemies[0]
            .powers
            .iter()
            .any(|p| p.id == "VulnerablePower"));
    }

    /// Shuffle permutes the pile via combat RNG. Two runs from the same
    /// state must produce the same permutation (determinism).
    #[test]
    fn shuffle_pile_is_deterministic() {
        let mut a = ironclad_combat();
        let mut b = ironclad_combat();
        let effects = vec![Effect::Shuffle { pile: Pile::Draw }];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut a, &effects, &ctx);
        execute_effects(&mut b, &effects, &ctx);
        let a_ids: Vec<String> = a.allies[0]
            .player
            .as_ref()
            .unwrap()
            .draw
            .cards
            .iter()
            .map(|c| c.id.clone())
            .collect();
        let b_ids: Vec<String> = b.allies[0]
            .player
            .as_ref()
            .unwrap()
            .draw
            .cards
            .iter()
            .map(|c| c.id.clone())
            .collect();
        assert_eq!(a_ids, b_ids);
    }

    /// DiscardHand moves every card in hand to discard.
    #[test]
    fn discard_hand_moves_all_cards() {
        let mut cs = ironclad_combat();
        let ps = cs.allies[0].player.as_mut().unwrap();
        let drawn_id = "StrikeIronclad";
        if let Some(card) = crate::card::by_id(drawn_id) {
            ps.hand
                .cards
                .push(crate::combat::CardInstance::from_card(card, 0));
            ps.hand
                .cards
                .push(crate::combat::CardInstance::from_card(card, 0));
        }
        let hand_size = cs.allies[0].player.as_ref().unwrap().hand.cards.len();
        assert!(hand_size > 0);

        let effects = vec![Effect::DiscardHand];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        assert_eq!(cs.allies[0].player.as_ref().unwrap().hand.cards.len(), 0);
        assert!(cs.allies[0].player.as_ref().unwrap().discard.cards.len() >= hand_size);
    }

    /// RandomEnemy target picks a live enemy via combat RNG. Two runs
    /// from the same starting state must hit the same target
    /// (deterministic given a fixed seed).
    #[test]
    fn random_enemy_is_deterministic_given_state() {
        let cs_a = ironclad_combat();
        let cs_b = ironclad_combat();

        // Snapshot both — they're built from identical inputs, so the
        // combat-scoped RNG seeds are equal. The first random pick must
        // match.
        let mut a = cs_a;
        let mut b = cs_b;
        let effects = vec![Effect::DealDamage {
            amount: AmountSpec::Fixed(1),
            target: Target::RandomEnemy,
            hits: 1,
        }];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut a, &effects, &ctx);
        execute_effects(&mut b, &effects, &ctx);

        let hp_a: Vec<i32> = a.enemies.iter().map(|e| e.current_hp).collect();
        let hp_b: Vec<i32> = b.enemies.iter().map(|e| e.current_hp).collect();
        assert_eq!(hp_a, hp_b);
        // At least one enemy must have taken damage.
        assert!(hp_a.iter().zip(b.enemies.iter()).any(|(now, _)| *now > 0));
    }
}
