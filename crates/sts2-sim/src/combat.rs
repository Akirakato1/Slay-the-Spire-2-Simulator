//! Combat data structures + state-management primitives — Phase 0.2.
//!
//! Adds the pure-state machinery (turn flow, pile rotation, block clear)
//! without any of the per-card / per-power / per-monster *behavior* virtuals.
//! Those land in the next sub-port and constitute most of the remaining
//! Phase 0.2 effort.
//!
//! Naming mirrors the C# decompile where reasonable:
//!   - `CombatState.{Allies, Enemies, RoundNumber, CurrentSide, Encounter, Modifiers}`
//!   - `Creature.{CurrentHp, MaxHp, Block, Powers}` plus side-specific subfields
//!   - `CardPile` per `PileType`
//!
//! Diffs from C# worth flagging:
//!   - C# stores Powers as a list of distinct `PowerModel` instances. Rust
//!     stores `Vec<PowerInstance>` of (id, amount) records. Insertion order
//!     is preserved (matches the C# small-list iteration semantics).
//!   - C# uses one polymorphic `Creature` class with `IsPlayer`. Rust uses
//!     a single `Creature` struct with a `CreatureKind` discriminator and
//!     optional player/monster sub-state, avoiding `enum`-variant boilerplate
//!     for the many fields that are shared.

use crate::card::{by_id as card_by_id, CardData, CardType, TargetType};
use crate::character::CharacterData;
use crate::encounter::EncounterData;
use crate::monster::MonsterData;
use crate::power::{by_id as power_by_id, PowerStackType};
use crate::rng::Rng;

/// Default player energy at the start of each combat turn. (StS1/StS2
/// standard; the actual game lookup includes relic/affliction modifiers that
/// the behavior port will apply.)
pub const DEFAULT_TURN_ENERGY: i32 = 3;

/// C# `CombatSide`. `None` is a sentinel — combat is always Player or Enemy.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CombatSide {
    None,
    Player,
    Enemy,
}

/// C# `PileType`. `Deck` is the strategic-layer deck (not a combat pile);
/// `Play` is the transient pile while a card resolves. Combat-pile rotation
/// happens between Draw / Hand / Discard / Exhaust.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PileType {
    None,
    Draw,
    Hand,
    Discard,
    Exhaust,
    Play,
    Deck,
}

/// Distinguishes player creatures (piles + energy) from enemies (intent).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CreatureKind {
    Player,
    Monster,
    /// Summons / minions belonging to the player side.
    Summon,
}

#[derive(Clone, Debug)]
pub struct Creature {
    pub kind: CreatureKind,
    /// `CharacterData.id` for players, `MonsterData.id` for monsters/summons.
    pub model_id: String,
    /// Position slot string from the encounter ("front" / "back" / etc.).
    /// Empty for the player creature.
    pub slot: String,
    pub current_hp: i32,
    pub max_hp: i32,
    pub block: i32,
    pub powers: Vec<PowerInstance>,
    pub afflictions: Vec<AfflictionInstance>,
    /// Populated for players only.
    pub player: Option<PlayerState>,
    /// Populated for monsters only.
    pub monster: Option<MonsterState>,
}

#[derive(Clone, Debug)]
pub struct PlayerState {
    pub draw: CardPile,
    pub hand: CardPile,
    pub discard: CardPile,
    pub exhaust: CardPile,
    /// Current energy this turn.
    pub energy: i32,
    /// Per-turn energy refresh amount. Modified by relics like Velvet
    /// Choker, afflictions, etc. — behavior port will plumb those.
    pub turn_energy: i32,
}

#[derive(Clone, Debug)]
pub struct MonsterState {
    /// Currently-selected move id (matches a key in the monster's move state
    /// machine once that's ported). `None` until intent is resolved.
    pub intent_move: Option<String>,
    /// Computed intent values if known (attack damage × hit count, block,
    /// etc.). Empty until the intent pipeline runs.
    pub intent_values: Vec<IntentValue>,
}

#[derive(Clone, Debug)]
pub struct IntentValue {
    /// "Damage", "Block", "Buff", "Debuff", etc. — matches the C#
    /// `AbstractIntent` subclass family used in MonsterMoves/Intents/.
    pub kind: String,
    pub amount: i32,
    /// Hit count for multi-hit attacks; 1 for everything else.
    pub hits: i32,
}

#[derive(Clone, Debug)]
pub struct PowerInstance {
    /// `PowerData.id` (e.g., "StrengthPower").
    pub id: String,
    pub amount: i32,
}

#[derive(Clone, Debug)]
pub struct AfflictionInstance {
    /// `AfflictionData.id` (e.g., "Galvanized").
    pub id: String,
    pub amount: i32,
}

#[derive(Clone, Debug)]
pub struct CardPile {
    pub pile_type: PileType,
    pub cards: Vec<CardInstance>,
}

impl CardPile {
    pub fn new(pile_type: PileType) -> Self {
        Self {
            pile_type,
            cards: Vec::new(),
        }
    }
    pub fn with_cards(pile_type: PileType, cards: Vec<CardInstance>) -> Self {
        Self { pile_type, cards }
    }
    pub fn len(&self) -> usize {
        self.cards.len()
    }
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct CardInstance {
    /// `CardData.id`.
    pub id: String,
    pub upgrade_level: i32,
    /// Current effective energy cost. Starts at `CardData.energy_cost` minus
    /// `CardData.energy_cost_upgrade_delta * upgrade_level`. Modified by
    /// in-combat effects (Mummified Hand, etc.) at play time.
    pub current_energy_cost: i32,
    /// Combat-scoped tags ("retain_this_turn", "free_this_turn", ...).
    /// Cleared between turns by the behavior port.
    pub tags_this_turn: Vec<String>,
}

impl CardInstance {
    /// Instantiate a card from its static data table. Honors energy-cost
    /// upgrade delta when `upgrade_level > 0`. Caller is responsible for
    /// validating the id is in the table.
    pub fn from_card(card: &CardData, upgrade_level: i32) -> Self {
        let base_cost = card.energy_cost;
        let upgraded_cost = if upgrade_level > 0 {
            (base_cost + card.energy_cost_upgrade_delta).max(0)
        } else {
            base_cost
        };
        Self {
            id: card.id.clone(),
            upgrade_level,
            current_energy_cost: upgraded_cost,
            tags_this_turn: Vec::new(),
        }
    }
}

/// Full combat state. Owned by the run; constructed on entering a combat
/// room, dropped on leaving.
#[derive(Clone, Debug)]
pub struct CombatState {
    /// Encounter id (`EncounterData.id`). `None` for ad-hoc unit-test
    /// combats.
    pub encounter_id: Option<String>,
    pub round_number: i32,
    pub current_side: CombatSide,
    /// Modifier ids active in this run; copied in at start so combat doesn't
    /// reach back to RunState every lookup.
    pub modifier_ids: Vec<String>,
    /// Player creatures and their summons.
    pub allies: Vec<Creature>,
    /// Enemy creatures from the encounter spawn.
    pub enemies: Vec<Creature>,
    /// Creatures that escaped (Loonbat-style fleeing enemies) — separate
    /// from `enemies` because they shouldn't be valid targets but still
    /// matter for some end-of-combat rewards.
    pub escaped: Vec<Creature>,
}

impl CombatState {
    /// Empty constructor; useful for unit tests that want to set up state
    /// piece by piece. Normal flow uses [`CombatState::start`].
    pub fn empty() -> Self {
        Self {
            encounter_id: None,
            round_number: 1,
            current_side: CombatSide::Player,
            modifier_ids: Vec::new(),
            allies: Vec::new(),
            enemies: Vec::new(),
            escaped: Vec::new(),
        }
    }

    /// Set up a fresh combat. Mirrors the canonical entry point in C#:
    ///   - Encounter's `canonical_monsters` populate `enemies`.
    ///   - Each player's starting deck (resolved from character data) loads
    ///     into `draw`.
    ///   - HP/relics carry in from caller-provided `PlayerSetup`.
    ///   - Round 1, Player side.
    ///
    /// The caller (combat-room behavior, later) handles: shuffling the draw
    /// pile through the run RNG, drawing the opening hand, applying
    /// combat-start relic hooks.
    pub fn start(
        encounter: &EncounterData,
        players: Vec<PlayerSetup>,
        modifier_ids: Vec<String>,
    ) -> Self {
        let allies: Vec<Creature> = players
            .into_iter()
            .map(Creature::from_player_setup)
            .collect();
        let enemies: Vec<Creature> = encounter
            .canonical_monsters
            .iter()
            .map(|spawn| Creature::from_monster_spawn(&spawn.monster, &spawn.slot))
            .collect();
        Self {
            encounter_id: Some(encounter.id.clone()),
            round_number: 1,
            current_side: CombatSide::Player,
            modifier_ids,
            allies,
            enemies,
            escaped: Vec::new(),
        }
    }

    // ---------- Turn-loop state machine -----------------------------------
    //
    // The C# CombatManager runs an async turn loop that fires hooks at each
    // boundary (BeforeSideTurnStart, AfterTurnEnd, ...). Those hooks land
    // with the behavior port. The methods below are the pure-state pieces:
    // they shuffle bookkeeping but don't run any model code.

    /// Player turn → Enemy turn → Player turn. Each Player-side begin is the
    /// start of a new round; we bump `round_number` then. Sets `current_side`.
    pub fn begin_turn(&mut self, side: CombatSide) {
        if side == CombatSide::Player && self.current_side == CombatSide::Enemy {
            self.round_number += 1;
        }
        self.current_side = side;
        // Block survives one creature's *own* turn end → wipe at the start
        // of that side's next turn. This matches StS rules: block from
        // Defend persists through enemy attacks, then resets when you play
        // again. We clear on this side's begin, not on end.
        match side {
            CombatSide::Player => {
                for ally in self.allies.iter_mut() {
                    ally.block = 0;
                    // Energy refresh: fill to per-turn allotment. C# routes
                    // this through Hook.ModifyEnergyGain which lets relics
                    // (Velvet Choker, etc.) tweak the amount; until those
                    // hooks land, refill directly to turn_energy.
                    if let Some(ps) = ally.player.as_mut() {
                        ps.energy = ps.turn_energy;
                    }
                }
            }
            CombatSide::Enemy => {
                for enemy in self.enemies.iter_mut() {
                    enemy.block = 0;
                }
            }
            CombatSide::None => {}
        }
        // AfterSideTurnStart hook pass — currently just Poison.
        // Hook firing order proper will land in #70; for now we run the
        // ticks directly so combat math stays correct.
        self.tick_start_of_turn_powers(side);
    }

    /// Apply each creature's start-of-turn power effects when that
    /// creature's side begins its turn. Currently models PoisonPower:
    /// deal `Amount` damage (Unblockable | Unpowered → block-bypassing),
    /// then decrement the stack by 1.
    ///
    /// Snapshots ticks before applying so a death during one tick doesn't
    /// disrupt iteration. Tick uses `lose_hp` (bypasses block) per the
    /// `ValueProp.Unblockable` flag the C# passes.
    pub fn tick_start_of_turn_powers(&mut self, side: CombatSide) {
        let mut ticks: Vec<(usize, i32)> = Vec::new();
        let list = match side {
            CombatSide::Player => &self.allies,
            CombatSide::Enemy => &self.enemies,
            CombatSide::None => return,
        };
        for (idx, creature) in list.iter().enumerate() {
            if creature.current_hp == 0 {
                continue;
            }
            if let Some(p) = creature.powers.iter().find(|p| p.id == "PoisonPower") {
                if p.amount > 0 {
                    ticks.push((idx, p.amount));
                }
            }
        }
        for (idx, amount) in ticks {
            self.lose_hp(side, idx, amount);
            self.decrement_power(side, idx, "PoisonPower", 1);
        }
    }

    /// Pure end-of-turn bookkeeping for the side just finishing:
    ///   - Player side: discard the hand (StS rule; cards with retain
    ///     keyword stay, but tag-based exemptions land with behavior).
    ///   - Energy refresh for players happens at the *next* `begin_turn`
    ///     after the behavior port wires in modifiers; we leave energy alone
    ///     here so the test surface stays predictable.
    pub fn end_turn(&mut self) {
        if self.current_side == CombatSide::Player {
            for ally in self.allies.iter_mut() {
                let Some(ps) = ally.player.as_mut() else {
                    continue;
                };
                // Move hand → discard wholesale. Retain handling lives in
                // the behavior port (inspects CardInstance::tags_this_turn).
                ps.discard.cards.append(&mut ps.hand.cards);
            }
        }
    }

    /// Draw up to `n` cards from the first player's draw pile, reshuffling
    /// discard into draw when draw is exhausted. Stops early if both piles
    /// are empty. Uses `rng.shuffle()` (== C# `Rng.Shuffle` Fisher-Yates),
    /// matching `RunState.Rng.Shuffle` semantics. Returns the number drawn.
    pub fn draw_cards(&mut self, player_idx: usize, n: i32, rng: &mut Rng) -> i32 {
        let Some(creature) = self.allies.get_mut(player_idx) else {
            return 0;
        };
        let Some(ps) = creature.player.as_mut() else {
            return 0;
        };
        let mut drawn = 0;
        for _ in 0..n {
            if ps.draw.is_empty() {
                if ps.discard.is_empty() {
                    break;
                }
                // Reshuffle: drain discard into draw, then shuffle in place.
                ps.draw.cards.append(&mut ps.discard.cards);
                rng.shuffle(&mut ps.draw.cards);
            }
            // StS draws from the TOP of the draw pile; C# uses
            // RemoveAt(Count-1)-style pops. The shuffle determines order
            // before we pop, so pop_back is fine.
            if let Some(card) = ps.draw.cards.pop() {
                ps.hand.cards.push(card);
                drawn += 1;
            }
        }
        drawn
    }

    /// Move every card in the named player's hand to discard. Useful for
    /// end-of-turn and effects like "Discard your hand."
    pub fn discard_hand(&mut self, player_idx: usize) {
        let Some(creature) = self.allies.get_mut(player_idx) else {
            return;
        };
        let Some(ps) = creature.player.as_mut() else {
            return;
        };
        ps.discard.cards.append(&mut ps.hand.cards);
    }

    // ---------- Damage / block / HP primitives ----------------------------
    //
    // These are the bare arithmetic that the C# damage pipeline wraps in
    // hooks (ModifyDamageAdditive, ModifyDamageMultiplicative, Intangible
    // flooring, AfterDamageTaken, ...). The behavior port plumbs those
    // hooks; these primitives stay the same.

    /// Apply `amount` damage to one creature. Block absorbs first; remainder
    /// drops `current_hp` to a floor of 0. Returns `DamageOutcome`
    /// describing how the damage split for callers / hook listeners.
    pub fn apply_damage(
        &mut self,
        side: CombatSide,
        target_idx: usize,
        amount: i32,
    ) -> DamageOutcome {
        let Some(target) = creature_mut(self, side, target_idx) else {
            return DamageOutcome::default();
        };
        damage_creature(target, amount)
    }

    /// Heal a creature; saturates at `max_hp`.
    pub fn heal(&mut self, side: CombatSide, target_idx: usize, amount: i32) -> i32 {
        let Some(target) = creature_mut(self, side, target_idx) else {
            return 0;
        };
        let before = target.current_hp;
        target.current_hp = (target.current_hp + amount.max(0)).min(target.max_hp);
        target.current_hp - before
    }

    /// Reduce HP without going through block. Used by self-damage cards,
    /// Pact's End, etc. Floors at 0.
    pub fn lose_hp(&mut self, side: CombatSide, target_idx: usize, amount: i32) -> i32 {
        let Some(target) = creature_mut(self, side, target_idx) else {
            return 0;
        };
        let actual = amount.max(0).min(target.current_hp);
        target.current_hp -= actual;
        actual
    }

    /// Permanent max-HP change. Clamps `current_hp` down if max drops below
    /// it. Negative `delta` reduces, positive adds. Returns the actual
    /// delta applied (max_hp won't go below 1).
    pub fn change_max_hp(
        &mut self,
        side: CombatSide,
        target_idx: usize,
        delta: i32,
    ) -> i32 {
        let Some(target) = creature_mut(self, side, target_idx) else {
            return 0;
        };
        let new_max = (target.max_hp + delta).max(1);
        let actual = new_max - target.max_hp;
        target.max_hp = new_max;
        if target.current_hp > target.max_hp {
            target.current_hp = target.max_hp;
        }
        actual
    }

    /// Add `amount` block to a creature. Floors at 0 (no negative block).
    pub fn gain_block(
        &mut self,
        side: CombatSide,
        target_idx: usize,
        amount: i32,
    ) -> i32 {
        let Some(target) = creature_mut(self, side, target_idx) else {
            return 0;
        };
        let actual = amount.max(0);
        target.block += actual;
        actual
    }

    // ---------- Power apply / decrement / lookup --------------------------
    //
    // Reflects the PowerData metadata (stack_type, allow_negative) without
    // invoking any per-power behavior hooks. The behavior port wires in:
    //   - StrengthPower.ModifyDamageAdditive
    //   - VulnerablePower.ModifyDamageMultiplicative
    //   - PoisonPower.AfterSideTurnStart (poison ticks)
    //   - Power application VFX / commands
    // None of those change the arithmetic here.

    /// Apply `amount` of a power to a creature, honoring the power's
    /// `stack_type`. Returns the resulting stack count (or 0 if the power
    /// id is unknown or the target doesn't exist).
    ///
    /// Stack-type rules:
    ///   - Counter: accumulate. If `allow_negative` is false, clamp at 0
    ///     and remove the stack when it hits 0. Strength is the canonical
    ///     allow_negative=true case (Weak can drive it negative).
    ///   - Single: 0 → set 1. 1+ amount or another apply → stays 1. The
    ///     full-on/off semantics live in the behavior port; for now we
    ///     just record presence.
    pub fn apply_power(
        &mut self,
        side: CombatSide,
        target_idx: usize,
        power_id: &str,
        amount: i32,
    ) -> i32 {
        let Some(power) = power_by_id(power_id) else {
            return 0;
        };
        let Some(target) = creature_mut(self, side, target_idx) else {
            return 0;
        };
        match power.stack_type {
            PowerStackType::Counter => {
                if let Some(existing) =
                    target.powers.iter_mut().find(|p| p.id == power_id)
                {
                    existing.amount += amount;
                    if !power.allow_negative && existing.amount < 0 {
                        existing.amount = 0;
                    }
                    if existing.amount == 0 && !power.allow_negative {
                        let new_amount = 0;
                        target.powers.retain(|p| p.id != power_id);
                        return new_amount;
                    }
                    existing.amount
                } else {
                    let mut starting = amount;
                    if !power.allow_negative && starting < 0 {
                        starting = 0;
                    }
                    if starting == 0 && !power.allow_negative {
                        return 0;
                    }
                    target.powers.push(PowerInstance {
                        id: power_id.to_string(),
                        amount: starting,
                    });
                    starting
                }
            }
            PowerStackType::Single | PowerStackType::None => {
                if target.powers.iter().any(|p| p.id == power_id) {
                    1
                } else {
                    target.powers.push(PowerInstance {
                        id: power_id.to_string(),
                        amount: 1,
                    });
                    1
                }
            }
        }
    }

    /// Decrement a counter-style power by `amount` (defaults to 1). Removes
    /// the stack if it hits 0 (and the power doesn't allow negatives).
    /// No-op for unknown power ids or absent powers.
    pub fn decrement_power(
        &mut self,
        side: CombatSide,
        target_idx: usize,
        power_id: &str,
        amount: i32,
    ) -> i32 {
        self.apply_power(side, target_idx, power_id, -amount)
    }

    /// Returns the current stack count of a power on a creature, or 0 if
    /// the power isn't applied.
    pub fn get_power_amount(
        &self,
        side: CombatSide,
        target_idx: usize,
        power_id: &str,
    ) -> i32 {
        let creature = match side {
            CombatSide::Player => self.allies.get(target_idx),
            CombatSide::Enemy => self.enemies.get(target_idx),
            CombatSide::None => None,
        };
        creature
            .and_then(|c| c.powers.iter().find(|p| p.id == power_id))
            .map(|p| p.amount)
            .unwrap_or(0)
    }

    // ---------- Damage modifier pipeline ----------------------------------
    //
    // Mirrors C# `Hook.ModifyDamage` / `ModifyDamageInternal`:
    //   1. Card enchantment additive + multiplicative (TODO; current
    //      CardInstance doesn't carry enchantment).
    //   2. For each hook listener: ModifyDamageAdditive accumulates.
    //   3. For each hook listener: ModifyDamageMultiplicative composes.
    //   4. For each hook listener: ModifyDamageCap caps the result.
    //   5. Math.Max(0, num); cast to int (truncation toward zero).
    //
    // C# iterates "hook listeners" (every power on every creature in the
    // combat); each per-power method checks `dealer == base.Owner` or
    // `target == base.Owner` and returns the identity (0 for additive,
    // 1 for multiplicative) when it doesn't apply. We get the same
    // numeric result by directly indexing dealer's powers vs target's
    // powers and routing each contribution to the appropriate phase.
    //
    // Decimal vs f64: C# uses System.Decimal. Game damage is small
    // integer-scale and the multiplicative factors we've seen are
    // {0.75, 1.5} — all exact in f64. Factor count stays modest so we
    // don't accumulate rounding error in practice.

    /// Compute final integer damage after applying every active modifier
    /// (Strength on dealer, Vulnerable on target, Weak on dealer, ...
    /// later Intangible cap, etc.). The caller still routes the returned
    /// integer through `apply_damage` for the block→HP split.
    pub fn modify_damage(
        &self,
        dealer: (CombatSide, usize),
        target: (CombatSide, usize),
        raw: i32,
        props: ValueProp,
    ) -> i32 {
        let mut num = raw as f64;

        let dealer_powers = creature_powers(self, dealer);
        let target_powers = creature_powers(self, target);

        for power in dealer_powers {
            num += power_additive_dealer(power, props);
        }
        for power in dealer_powers {
            num *= power_multiplicative_dealer(power, props);
        }
        for power in target_powers {
            num *= power_multiplicative_target(power, props);
        }

        // Cap pass: take the smallest cap any target-side power supplies.
        // C# Hook tracks `num4 = MaxValue` and any listener's lower cap
        // floors the result. IntangiblePower returns 1 to its owner.
        let mut cap = f64::MAX;
        for power in target_powers {
            let c = power_damage_cap_target(power);
            if c < cap {
                cap = c;
            }
        }
        if num > cap {
            num = cap;
        }

        let clamped = num.max(0.0);
        clamped as i32
    }

    /// Check if combat has resolved. Returns `Some(Victory)` if every
    /// enemy is at 0 HP, `Some(Defeat)` if every player creature is, or
    /// `None` if combat continues. Escaped enemies don't count toward
    /// either side (matches StS rules: fleeing enemies neither lose nor
    /// keep you fighting).
    pub fn is_combat_over(&self) -> Option<CombatResult> {
        let all_enemies_dead = !self.enemies.is_empty()
            && self.enemies.iter().all(|c| c.current_hp == 0);
        let all_players_dead = !self.allies.is_empty()
            && self
                .allies
                .iter()
                .filter(|c| c.kind == CreatureKind::Player)
                .all(|c| c.current_hp == 0);
        if all_players_dead {
            // Defeat takes precedence over Victory if both somehow happen
            // in the same instant — matches C# combat-end ordering where
            // player-death checks run before victory checks.
            Some(CombatResult::Defeat)
        } else if all_enemies_dead {
            Some(CombatResult::Victory)
        } else {
            None
        }
    }

    /// Convenience: compose `modify_damage` with `apply_damage`. Most card
    /// behaviors deal damage through this entrypoint.
    pub fn deal_damage(
        &mut self,
        dealer: (CombatSide, usize),
        target: (CombatSide, usize),
        raw: i32,
        props: ValueProp,
    ) -> DamageOutcome {
        let modified = self.modify_damage(dealer, target, raw, props);
        self.apply_damage(target.0, target.1, modified)
    }

    // ---------- Card play action ------------------------------------------
    //
    // Mirrors the C# CardManager.PlayCard / CardModel.OnPlay path at the
    // state level: validate energy + target, deduct energy, route the
    // card hand → play → discard/exhaust, invoke the OnPlay dispatcher.
    //
    // The dispatcher is a single match (see `dispatch_on_play`) — each
    // ported card adds one arm. Cards whose OnPlay isn't yet ported
    // return `PlayResult::Unhandled`; the rest of the state changes
    // (energy deduction, pile routing) still happen so the test harness
    // can observe partial progress while we incrementally fill in the
    // dispatcher.

    /// Play a card from the named player's hand. Validates and (if OK)
    /// deducts energy, runs OnPlay, and routes the card to discard or
    /// exhaust per its type. Returns a `PlayResult` distinguishing the
    /// failure modes.
    pub fn play_card(
        &mut self,
        player_idx: usize,
        hand_idx: usize,
        target: Option<(CombatSide, usize)>,
    ) -> PlayResult {
        // 1. Locate hand card + verify energy. Borrow scope kept tight so
        //    the subsequent state mutations don't fight the borrow checker.
        let card_id;
        let upgrade_level;
        let energy_cost;
        let card_data: &'static CardData;
        let max_target_side;
        let max_target_idx;
        {
            let Some(creature) = self.allies.get(player_idx) else {
                return PlayResult::InvalidHand;
            };
            let Some(ps) = creature.player.as_ref() else {
                return PlayResult::InvalidHand;
            };
            let Some(card) = ps.hand.cards.get(hand_idx) else {
                return PlayResult::InvalidHand;
            };
            let Some(data) = card_by_id(&card.id) else {
                return PlayResult::UnknownCard;
            };
            card_id = card.id.clone();
            upgrade_level = card.upgrade_level;
            energy_cost = card.current_energy_cost;
            card_data = data;
            if ps.energy < energy_cost {
                return PlayResult::InsufficientEnergy {
                    available: ps.energy,
                    required: energy_cost,
                };
            }
            // Snapshot enemy / ally counts for target validation; can't
            // hold a reference into self.allies past here.
            max_target_side = self.enemies.len();
            max_target_idx = self.allies.len();
        }

        // 2. Target validation by CardData.target_type. Player-aimed
        //    target types (SelfTarget, AnyPlayer) currently support only
        //    the single-player case (target == None → defaults to self).
        match validate_target(card_data.target_type, target, max_target_idx, max_target_side, player_idx) {
            Ok(()) => {}
            Err(e) => return e,
        }

        // 3. Deduct energy.
        {
            let ps = self.allies[player_idx].player.as_mut().unwrap();
            ps.energy -= energy_cost;
        }

        // 4. Remove the card from hand into a temporary "play" position.
        //    We hold it here until OnPlay finishes; some cards (e.g.,
        //    exhausting attacks) need their CardInstance available during
        //    OnPlay before routing.
        let played_card = {
            let ps = self.allies[player_idx].player.as_mut().unwrap();
            ps.hand.cards.remove(hand_idx)
        };

        // 5. Dispatch OnPlay. The handler may mutate cs freely.
        let handled = dispatch_on_play(
            self,
            &card_id,
            upgrade_level,
            player_idx,
            target,
        );

        // 6. Route the card per its type. Status/Curse cards exhaust
        //    by default; Attack/Skill/Power go to discard unless the
        //    card's keyword set includes Exhaust (not yet ported).
        let dest = match card_data.card_type {
            CardType::Status | CardType::Curse => PileType::Exhaust,
            _ => PileType::Discard,
        };
        let ps = self.allies[player_idx].player.as_mut().unwrap();
        match dest {
            PileType::Discard => ps.discard.cards.push(played_card),
            PileType::Exhaust => ps.exhaust.cards.push(played_card),
            _ => ps.discard.cards.push(played_card),
        }

        if handled {
            PlayResult::Ok
        } else {
            PlayResult::Unhandled
        }
    }
}

/// Outcome of [`CombatState::play_card`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PlayResult {
    /// OnPlay dispatched and ran cleanly; energy spent; card routed.
    Ok,
    /// Card-state changes (energy, routing) happened, but no OnPlay
    /// implementation is registered for this card yet. Useful during
    /// incremental porting — tests can call play_card on un-ported
    /// cards and see the routing without crashing.
    Unhandled,
    /// hand_idx is out of bounds, or player_idx doesn't reference a
    /// valid player creature.
    InvalidHand,
    /// Card energy cost exceeds the player's current energy.
    InsufficientEnergy { available: i32, required: i32 },
    /// Target violates the card's `target_type`: missing when required,
    /// present when not allowed, or pointing to a dead/missing creature.
    InvalidTarget,
    /// The card's `id` is not in the static `CardData` table.
    UnknownCard,
}

fn validate_target(
    target_type: TargetType,
    target: Option<(CombatSide, usize)>,
    n_allies: usize,
    n_enemies: usize,
    player_idx: usize,
) -> Result<(), PlayResult> {
    match target_type {
        TargetType::None | TargetType::AllEnemies | TargetType::AllAllies
        | TargetType::RandomEnemy | TargetType::TargetedNoCreature => {
            // No specific target needed; ignore any passed target.
            Ok(())
        }
        TargetType::SelfTarget => {
            // Allow either None (defaults to self) or explicit
            // (Player, player_idx). Anything else is invalid.
            match target {
                None => Ok(()),
                Some((CombatSide::Player, idx)) if idx == player_idx => Ok(()),
                _ => Err(PlayResult::InvalidTarget),
            }
        }
        TargetType::AnyEnemy => {
            match target {
                Some((CombatSide::Enemy, idx)) if idx < n_enemies => Ok(()),
                _ => Err(PlayResult::InvalidTarget),
            }
        }
        TargetType::AnyPlayer | TargetType::AnyAlly => {
            match target {
                Some((CombatSide::Player, idx)) if idx < n_allies => Ok(()),
                None => Ok(()),
                _ => Err(PlayResult::InvalidTarget),
            }
        }
        TargetType::Osty => {
            // Special target type — minimal handling for now.
            Ok(())
        }
    }
}

/// OnPlay dispatcher. Each ported card adds one arm. Returns true if the
/// card's effect was applied, false if its OnPlay isn't ported yet.
///
/// Per C# semantics, OnPlay can mutate the entire CombatState (damage,
/// block, draw cards, apply powers). We pass `&mut CombatState` for the
/// dispatch and let each handler call the high-level primitives
/// (`deal_damage`, `gain_block`, `apply_power`, `draw_cards`, ...).
fn dispatch_on_play(
    cs: &mut CombatState,
    card_id: &str,
    upgrade_level: i32,
    player_idx: usize,
    target: Option<(CombatSide, usize)>,
) -> bool {
    match card_id {
        // All 5 Strike variants: deal Damage to single AnyEnemy target,
        // routed through the modifier pipeline with ValueProp.Move.
        "StrikeIronclad" | "StrikeSilent" | "StrikeDefect" | "StrikeRegent"
        | "StrikeNecrobinder" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            cs.deal_damage(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
            );
            true
        }
        // All 5 Defend variants: gain Block on self. The C# calls
        // CreatureCmd.GainBlock which threads through block-modifier hooks
        // (Frail / Dexterity) — those powers aren't ported yet, so for
        // now we go straight to gain_block. Once Frail/Dexterity land,
        // wrap this in a modify_block pipeline analogous to modify_damage.
        "DefendIronclad" | "DefendSilent" | "DefendDefect" | "DefendRegent"
        | "DefendNecrobinder" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let block = canonical_int_value(card, "Block", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            true
        }
        _ => false,
    }
}

/// Resolve the effective integer value of one of a card's canonical vars
/// at a given upgrade level. Sums the base value with any
/// `upgrade_deltas` whose `var_kind` matches, scaled by `upgrade_level`.
///
/// For Strike (Damage var, base 6, delta +3) at level 1 this returns 9.
/// For Defend (Block, base 5, delta +3) at level 1, 8.
fn canonical_int_value(card: &CardData, var_kind: &str, upgrade_level: i32) -> i32 {
    let base = card
        .canonical_vars
        .iter()
        .find(|v| v.kind == var_kind)
        .and_then(|v| v.base_value)
        .unwrap_or(0.0);
    let delta_sum: f64 = card
        .upgrade_deltas
        .iter()
        .filter(|d| d.var_kind == var_kind)
        .map(|d| d.delta)
        .sum();
    let total = base + delta_sum * upgrade_level as f64;
    total as i32
}

/// `ValueProp` flags — mirrors C# `MegaCrit.Sts2.Core.ValueProps.ValueProp`.
/// `is_powered_attack()` is the predicate that gates damage modifiers:
/// `Move && !Unpowered`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub struct ValueProp(pub u8);

impl ValueProp {
    pub const NONE: ValueProp = ValueProp(0);
    pub const UNBLOCKABLE: ValueProp = ValueProp(2);
    pub const UNPOWERED: ValueProp = ValueProp(4);
    pub const MOVE: ValueProp = ValueProp(8);
    pub const SKIP_HURT_ANIM: ValueProp = ValueProp(16);

    pub const fn has(self, flag: ValueProp) -> bool {
        (self.0 & flag.0) == flag.0
    }
    pub const fn with(self, flag: ValueProp) -> ValueProp {
        ValueProp(self.0 | flag.0)
    }
    pub const fn is_powered_attack(self) -> bool {
        self.has(ValueProp::MOVE) && !self.has(ValueProp::UNPOWERED)
    }
}

fn creature_powers(cs: &CombatState, who: (CombatSide, usize)) -> &[PowerInstance] {
    let creature = match who.0 {
        CombatSide::Player => cs.allies.get(who.1),
        CombatSide::Enemy => cs.enemies.get(who.1),
        CombatSide::None => None,
    };
    creature.map(|c| c.powers.as_slice()).unwrap_or(&[])
}

fn power_additive_dealer(power: &PowerInstance, props: ValueProp) -> f64 {
    if !props.is_powered_attack() {
        return 0.0;
    }
    match power.id.as_str() {
        // StrengthPower.ModifyDamageAdditive: +Amount on powered attacks
        // from the owner. allow_negative=true → Strength can be negative
        // (Weak-style debuffs subtract Strength).
        "StrengthPower" => power.amount as f64,
        _ => 0.0,
    }
}

fn power_multiplicative_dealer(power: &PowerInstance, props: ValueProp) -> f64 {
    if !props.is_powered_attack() {
        return 1.0;
    }
    match power.id.as_str() {
        // WeakPower: ×0.75 on powered attacks from the owner. (Paper
        // Krane / Debilitate further tweak the factor; not modeled here.)
        "WeakPower" => 0.75,
        _ => 1.0,
    }
}

fn power_multiplicative_target(power: &PowerInstance, props: ValueProp) -> f64 {
    if !props.is_powered_attack() {
        return 1.0;
    }
    match power.id.as_str() {
        // VulnerablePower: ×1.5 on powered attacks landing on the owner.
        "VulnerablePower" => 1.5,
        _ => 1.0,
    }
}

/// Damage cap from a target-side power. `f64::MAX` means no cap (matches
/// C# decimal.MaxValue). The smallest cap across all target-side powers
/// floors the post-multiplicative damage.
///
/// IntangiblePower's `ModifyDamageCap`: 1 when target == owner. (TheBoot
/// relic raises the cap to 5; not modeled until relic hooks land.) The
/// C# check `if (target != base.Owner) return decimal.MaxValue;` is
/// implicit here — we only invoke this on target-side powers, so
/// "target == owner" is structurally enforced.
fn power_damage_cap_target(power: &PowerInstance) -> f64 {
    match power.id.as_str() {
        "IntangiblePower" => 1.0,
        _ => f64::MAX,
    }
}

// ---------- Monster intent selection (Axebot) ---------------------------
//
// Reflects C# `MonsterMoveStateMachine` + `RandomBranchState.GetNextState`:
//   total = sum of weights
//   roll  = rng.NextFloat(total)   // in [0, total)
//   subtract each weight in registration order; return the first state
//   where roll <= 0
//
// We don't yet have the generic state-machine abstraction — Axebot's
// pattern is direct-baked here. Once a second / third monster ports, the
// shared pieces (RandomBranchState, MoveRepeatType, weighted pick) will
// factor out cleanly.

/// Axebot's selectable moves, in the C# state-machine declaration order.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AxebotIntent {
    BootUp,
    OneTwo,
    Sharpen,
    HammerUppercut,
}

impl AxebotIntent {
    pub fn id(self) -> &'static str {
        match self {
            AxebotIntent::BootUp => "BOOT_UP_MOVE",
            AxebotIntent::OneTwo => "ONE_TWO_MOVE",
            AxebotIntent::Sharpen => "SHARPEN_MOVE",
            AxebotIntent::HammerUppercut => "HAMMER_UPPERCUT_MOVE",
        }
    }
}

// Per-move payload constants — C# Axebot.cs private getters. Ascension
// scaling deferred: A0 values hardcoded for now (the higher branch of each
// `AscensionHelper.GetValueIfAscension(...)` is what changes at the named
// ascension level). When ascension is plumbed into CombatState, switch to
// the conditional values.
//
//   OneTwoDamage: A0=5, A1+=6  (DeadlyEnemies)
//   HammerUppercutDamage: A0=8, A1+=10  (DeadlyEnemies)
//   BootUp block: const 10
//   BootUp strength gain: const 1
//   Sharpen strength gain: const 4
const AXEBOT_ONE_TWO_DAMAGE: i32 = 5;
const AXEBOT_ONE_TWO_HITS: i32 = 2;
const AXEBOT_HAMMER_UPPERCUT_DAMAGE: i32 = 8;
const AXEBOT_BOOT_UP_BLOCK: i32 = 10;
const AXEBOT_BOOT_UP_STRENGTH_GAIN: i32 = 1;
const AXEBOT_SHARPEN_STRENGTH_GAIN: i32 = 4;

/// Execute one Axebot move's payload. Caller is responsible for picking
/// the intent ahead of time and routing the appropriate target. Mirrors
/// C# Axebot's per-move handlers (BootUpMove / OneTwoMove / SharpenMove /
/// HammerUppercutMove), minus the audio/animation calls.
pub fn execute_axebot_move(
    cs: &mut CombatState,
    axebot_idx: usize,
    target_player_idx: usize,
    intent: AxebotIntent,
) {
    let attacker = (CombatSide::Enemy, axebot_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        AxebotIntent::BootUp => {
            // GainBlock(self, 10) + Apply<StrengthPower>(self, 1).
            cs.gain_block(CombatSide::Enemy, axebot_idx, AXEBOT_BOOT_UP_BLOCK);
            cs.apply_power(
                CombatSide::Enemy,
                axebot_idx,
                "StrengthPower",
                AXEBOT_BOOT_UP_STRENGTH_GAIN,
            );
        }
        AxebotIntent::OneTwo => {
            // Two attacks of OneTwoDamage. Each goes through modifiers
            // independently (block recomputes between hits per StS rules).
            for _ in 0..AXEBOT_ONE_TWO_HITS {
                cs.deal_damage(attacker, player, AXEBOT_ONE_TWO_DAMAGE, ValueProp::MOVE);
            }
        }
        AxebotIntent::Sharpen => {
            cs.apply_power(
                CombatSide::Enemy,
                axebot_idx,
                "StrengthPower",
                AXEBOT_SHARPEN_STRENGTH_GAIN,
            );
        }
        AxebotIntent::HammerUppercut => {
            // Single attack, then apply Weak + Frail to the player.
            cs.deal_damage(
                attacker,
                player,
                AXEBOT_HAMMER_UPPERCUT_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(CombatSide::Player, target_player_idx, "WeakPower", 1);
            cs.apply_power(CombatSide::Player, target_player_idx, "FrailPower", 1);
        }
    }
}

/// Pick Axebot's next intent. C# Axebot.GenerateMoveStateMachine:
///   - First turn (no `last_intent`): BOOT_UP_MOVE.
///   - All subsequent turns: weighted random across
///     {OneTwo:2, Sharpen:1 unless just played, HammerUppercut:2} with the
///     C# subtract-and-compare iteration on `rng.NextFloat(total)`.
///
/// `rng` must be the monster's per-encounter `monster.Rng` instance (in
/// C# derived from `RunState.Rng.Seed + map_coord`). Tests can pass a
/// deterministically-seeded `Rng::new(seed, 0)`.
pub fn pick_axebot_intent(rng: &mut Rng, last_intent: Option<AxebotIntent>) -> AxebotIntent {
    if last_intent.is_none() {
        return AxebotIntent::BootUp;
    }
    let sharpen_blocked = matches!(last_intent, Some(AxebotIntent::Sharpen));
    let w_one_two: f32 = 2.0;
    let w_sharpen: f32 = if sharpen_blocked { 0.0 } else { 1.0 };
    let w_hammer: f32 = 2.0;
    let total = w_one_two + w_sharpen + w_hammer;
    let mut roll = rng.next_float(total);
    // Iteration order matches C#'s RandomBranchState.States list order.
    roll -= w_one_two;
    if roll <= 0.0 {
        return AxebotIntent::OneTwo;
    }
    roll -= w_sharpen;
    if roll <= 0.0 {
        return AxebotIntent::Sharpen;
    }
    // Last branch — math guarantees roll - w_hammer <= 0 since
    // initial roll < total. Return without further check.
    AxebotIntent::HammerUppercut
}

/// Result of a resolved combat. Reported by [`CombatState::is_combat_over`]
/// when the combat ends.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CombatResult {
    Victory,
    Defeat,
}

/// Outcome of a single `apply_damage` call. Useful for combat-log replay
/// and for upstream hooks that need to know whether HP actually moved.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DamageOutcome {
    /// Damage absorbed by block.
    pub blocked: i32,
    /// Damage that bypassed block and hit HP.
    pub hp_lost: i32,
    /// True if this damage instance reduced HP to 0.
    pub fatal: bool,
}

fn creature_mut(
    cs: &mut CombatState,
    side: CombatSide,
    idx: usize,
) -> Option<&mut Creature> {
    match side {
        CombatSide::Player => cs.allies.get_mut(idx),
        CombatSide::Enemy => cs.enemies.get_mut(idx),
        CombatSide::None => None,
    }
}

fn damage_creature(target: &mut Creature, amount: i32) -> DamageOutcome {
    if amount <= 0 {
        return DamageOutcome::default();
    }
    let blocked = amount.min(target.block);
    target.block -= blocked;
    let mut hp_lost = amount - blocked;
    if hp_lost > target.current_hp {
        hp_lost = target.current_hp;
    }
    target.current_hp -= hp_lost;
    DamageOutcome {
        blocked,
        hp_lost,
        fatal: target.current_hp == 0,
    }
}

/// Input bundle for setting up one player creature at combat start. The
/// caller (RunState → combat-room transition) resolves character data,
/// current HP, and the actual deck (which may differ from the character's
/// starting deck after card rewards / removals / upgrades).
#[derive(Clone, Debug)]
pub struct PlayerSetup<'a> {
    pub character: &'a CharacterData,
    pub current_hp: i32,
    pub max_hp: i32,
    /// `CardInstance` list to load into the draw pile (already-upgraded,
    /// already-cloned).
    pub deck: Vec<CardInstance>,
}

impl Creature {
    fn from_player_setup(setup: PlayerSetup<'_>) -> Self {
        Self {
            kind: CreatureKind::Player,
            model_id: setup.character.id.clone(),
            slot: String::new(),
            current_hp: setup.current_hp,
            max_hp: setup.max_hp,
            block: 0,
            powers: Vec::new(),
            afflictions: Vec::new(),
            player: Some(PlayerState {
                draw: CardPile::with_cards(PileType::Draw, setup.deck),
                hand: CardPile::new(PileType::Hand),
                discard: CardPile::new(PileType::Discard),
                exhaust: CardPile::new(PileType::Exhaust),
                energy: DEFAULT_TURN_ENERGY,
                turn_energy: DEFAULT_TURN_ENERGY,
            }),
            monster: None,
        }
    }

    fn from_monster_spawn(monster_id: &str, slot: &str) -> Self {
        let data = crate::monster::by_id(monster_id);
        let (min_hp, max_hp) = data
            .map(|m| (m.min_hp_base.unwrap_or(1), m.max_hp_base.unwrap_or(1)))
            .unwrap_or((1, 1));
        // Use the max HP as the starting roll. The behavior port will route
        // the per-encounter HP roll through the run's monster-HP RNG stream.
        let _ = min_hp;
        Self {
            kind: CreatureKind::Monster,
            model_id: monster_id.to_string(),
            slot: slot.to_string(),
            current_hp: max_hp,
            max_hp,
            block: 0,
            powers: Vec::new(),
            afflictions: Vec::new(),
            player: None,
            monster: Some(MonsterState {
                intent_move: None,
                intent_values: Vec::new(),
            }),
        }
    }
}

/// Helper: instantiate a deck from a list of card ids (e.g.,
/// `CharacterData::starting_deck`). Cards default to upgrade level 0;
/// missing ids are silently skipped (callers should validate upstream).
pub fn deck_from_ids(ids: &[String]) -> Vec<CardInstance> {
    ids.iter()
        .filter_map(|id| card_by_id(id).map(|c| CardInstance::from_card(c, 0)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character;
    use crate::encounter;
    use crate::monster;

    #[test]
    fn empty_combat_state_round_1_player_side() {
        let cs = CombatState::empty();
        assert_eq!(cs.round_number, 1);
        assert_eq!(cs.current_side, CombatSide::Player);
        assert!(cs.allies.is_empty());
        assert!(cs.enemies.is_empty());
    }

    #[test]
    fn ironclad_vs_axebots_normal_initial_state() {
        let ironclad = character::by_id("Ironclad").expect("Ironclad present");
        let encounter =
            encounter::by_id("AxebotsNormal").expect("AxebotsNormal present");
        let deck = deck_from_ids(&ironclad.starting_deck);

        // Sanity on deck reconstruction before we drop it into combat.
        assert_eq!(deck.len(), 10);

        let setup = PlayerSetup {
            character: ironclad,
            current_hp: ironclad.starting_hp.unwrap(),
            max_hp: ironclad.starting_hp.unwrap(),
            deck,
        };
        let cs = CombatState::start(encounter, vec![setup], Vec::new());

        // Encounter wiring.
        assert_eq!(cs.encounter_id.as_deref(), Some("AxebotsNormal"));
        assert_eq!(cs.enemies.len(), 2);
        for e in &cs.enemies {
            assert_eq!(e.kind, CreatureKind::Monster);
            assert_eq!(e.model_id, "Axebot");
        }

        // Player wiring.
        assert_eq!(cs.allies.len(), 1);
        let p = &cs.allies[0];
        assert_eq!(p.kind, CreatureKind::Player);
        assert_eq!(p.model_id, "Ironclad");
        assert_eq!(p.current_hp, 80);
        assert_eq!(p.max_hp, 80);
        assert_eq!(p.block, 0);

        let ps = p.player.as_ref().expect("player has PlayerState");
        assert_eq!(ps.draw.len(), 10);
        assert!(ps.hand.is_empty());
        assert!(ps.discard.is_empty());
        assert!(ps.exhaust.is_empty());
        assert_eq!(ps.energy, DEFAULT_TURN_ENERGY);

        // Round / side.
        assert_eq!(cs.round_number, 1);
        assert_eq!(cs.current_side, CombatSide::Player);
    }

    #[test]
    fn monster_starts_at_max_hp() {
        // Axebot rolls between min and max HP at runtime; the scaffolding
        // populates max as the default until the HP-roll RNG is wired in.
        let axebot = monster::by_id("Axebot").expect("Axebot present");
        let creature = Creature::from_monster_spawn("Axebot", "front");
        assert_eq!(creature.max_hp, axebot.max_hp_base.unwrap());
        assert_eq!(creature.current_hp, creature.max_hp);
        assert_eq!(creature.slot, "front");
    }

    #[test]
    fn upgraded_card_energy_cost_uses_delta() {
        // BansheesCry: base 9 energy, upgrade delta -2. Upgraded copy
        // should cost 7.
        let bc = card_by_id("BansheesCry").expect("BansheesCry present");
        let unupgraded = CardInstance::from_card(bc, 0);
        let upgraded = CardInstance::from_card(bc, 1);
        assert_eq!(unupgraded.current_energy_cost, 9);
        assert_eq!(upgraded.current_energy_cost, 7);
    }

    // ---------- Turn-loop primitive tests ---------------------------------

    fn ironclad_combat() -> CombatState {
        let ironclad = character::by_id("Ironclad").expect("Ironclad present");
        let encounter =
            encounter::by_id("AxebotsNormal").expect("AxebotsNormal present");
        let deck = deck_from_ids(&ironclad.starting_deck);
        let setup = PlayerSetup {
            character: ironclad,
            current_hp: ironclad.starting_hp.unwrap(),
            max_hp: ironclad.starting_hp.unwrap(),
            deck,
        };
        CombatState::start(encounter, vec![setup], Vec::new())
    }

    #[test]
    fn side_flip_increments_round_on_player_reentry() {
        let mut cs = ironclad_combat();
        assert_eq!(cs.round_number, 1);
        assert_eq!(cs.current_side, CombatSide::Player);

        // Player → Enemy: still round 1.
        cs.end_turn();
        cs.begin_turn(CombatSide::Enemy);
        assert_eq!(cs.round_number, 1);
        assert_eq!(cs.current_side, CombatSide::Enemy);

        // Enemy → Player: round 2.
        cs.end_turn();
        cs.begin_turn(CombatSide::Player);
        assert_eq!(cs.round_number, 2);
        assert_eq!(cs.current_side, CombatSide::Player);

        // Player → Enemy → Player: round 3.
        cs.end_turn();
        cs.begin_turn(CombatSide::Enemy);
        cs.end_turn();
        cs.begin_turn(CombatSide::Player);
        assert_eq!(cs.round_number, 3);
    }

    // ---------- Poison tick tests -----------------------------------------

    #[test]
    fn poison_ticks_at_owner_side_begin() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PoisonPower", 3);
        let max_hp = cs.enemies[0].max_hp;
        // Begin Enemy turn → poison ticks the Axebot.
        cs.begin_turn(CombatSide::Enemy);
        // 3 damage bypassing block; stack decrements to 2.
        assert_eq!(cs.enemies[0].current_hp, max_hp - 3);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PoisonPower"),
            2
        );
    }

    #[test]
    fn poison_does_not_tick_on_opposing_side_begin() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PoisonPower", 5);
        let max_hp = cs.enemies[0].max_hp;
        // Begin Player turn — enemy's poison should not fire.
        cs.begin_turn(CombatSide::Player);
        assert_eq!(cs.enemies[0].current_hp, max_hp);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PoisonPower"),
            5
        );
    }

    #[test]
    fn poison_bypasses_block() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PoisonPower", 4);
        cs.enemies[0].block = 100;
        let max_hp = cs.enemies[0].max_hp;
        cs.begin_turn(CombatSide::Enemy);
        // begin_turn clears block first, but even if block were retained
        // (e.g., Loop relic), Poison uses lose_hp which bypasses block.
        // Here block is cleared at begin, so 4 damage chips HP directly.
        assert_eq!(cs.enemies[0].current_hp, max_hp - 4);
    }

    #[test]
    fn poison_decrements_to_zero_removes_stack() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PoisonPower", 1);
        cs.begin_turn(CombatSide::Enemy);
        // After 1 → tick → 1 damage, then decrement → 0 → stack removed.
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PoisonPower"),
            0
        );
        assert!(!cs.enemies[0]
            .powers
            .iter()
            .any(|p| p.id == "PoisonPower"));
    }

    #[test]
    fn lethal_poison_marks_creature_dead() {
        let mut cs = ironclad_combat();
        // Set a low-HP Axebot then big poison.
        cs.enemies[0].current_hp = 2;
        cs.apply_power(CombatSide::Enemy, 0, "PoisonPower", 10);
        cs.begin_turn(CombatSide::Enemy);
        assert_eq!(cs.enemies[0].current_hp, 0);
    }

    #[test]
    fn player_energy_refreshes_at_begin_player_turn() {
        let mut cs = ironclad_combat();
        // Spend energy down.
        cs.allies[0].player.as_mut().unwrap().energy = 0;
        // Enemy turn first (no refresh).
        cs.begin_turn(CombatSide::Enemy);
        assert_eq!(cs.allies[0].player.as_ref().unwrap().energy, 0);
        // Player turn: refresh to turn_energy (3 default).
        cs.begin_turn(CombatSide::Player);
        assert_eq!(cs.allies[0].player.as_ref().unwrap().energy, 3);
    }

    #[test]
    fn block_clears_at_begin_turn() {
        let mut cs = ironclad_combat();
        cs.allies[0].block = 7;
        cs.enemies[0].block = 4;

        // Player begin: clears player block, leaves enemy alone.
        cs.begin_turn(CombatSide::Player);
        assert_eq!(cs.allies[0].block, 0);
        assert_eq!(cs.enemies[0].block, 4);

        // Now switch to Enemy: clears enemy block.
        cs.end_turn();
        cs.begin_turn(CombatSide::Enemy);
        assert_eq!(cs.enemies[0].block, 0);
    }

    #[test]
    fn draw_five_from_ten_card_deck_uses_no_reshuffle() {
        let mut cs = ironclad_combat();
        let mut rng = Rng::new(12345, 0);
        let drawn = cs.draw_cards(0, 5, &mut rng);
        assert_eq!(drawn, 5);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.hand.len(), 5);
        assert_eq!(ps.draw.len(), 5);
        assert!(ps.discard.is_empty());
    }

    #[test]
    fn draw_more_than_deck_size_triggers_reshuffle() {
        // 10-card deck. Manually move 7 cards to discard (simulating a
        // mid-combat state), then ask for 5 — first 3 come from draw,
        // discard is reshuffled into draw, last 2 come from the reshuffled
        // pile. Total drawn = 5, both piles non-empty after.
        let mut cs = ironclad_combat();
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            for _ in 0..7 {
                let card = ps.draw.cards.pop().unwrap();
                ps.discard.cards.push(card);
            }
            assert_eq!(ps.draw.len(), 3);
            assert_eq!(ps.discard.len(), 7);
        }

        let mut rng = Rng::new(42, 0);
        let drawn = cs.draw_cards(0, 5, &mut rng);
        assert_eq!(drawn, 5);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.hand.len(), 5);
        // 3 + 7 = 10 total; minus 5 in hand = 5 remaining in draw,
        // 0 in discard (was emptied during reshuffle).
        assert_eq!(ps.draw.len(), 5);
        assert!(ps.discard.is_empty());
    }

    #[test]
    fn draw_stops_when_both_piles_empty() {
        let mut cs = ironclad_combat();
        {
            // Empty the draw pile into exhaust to simulate burned-out hand.
            let ps = cs.allies[0].player.as_mut().unwrap();
            ps.exhaust.cards.append(&mut ps.draw.cards);
        }
        let mut rng = Rng::new(7, 0);
        let drawn = cs.draw_cards(0, 5, &mut rng);
        assert_eq!(drawn, 0);
    }

    #[test]
    fn discard_hand_moves_all_to_discard() {
        let mut cs = ironclad_combat();
        let mut rng = Rng::new(1, 0);
        cs.draw_cards(0, 5, &mut rng);
        assert_eq!(cs.allies[0].player.as_ref().unwrap().hand.len(), 5);

        cs.discard_hand(0);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert!(ps.hand.is_empty());
        assert_eq!(ps.discard.len(), 5);
    }

    #[test]
    fn end_turn_on_player_side_discards_hand() {
        let mut cs = ironclad_combat();
        let mut rng = Rng::new(1, 0);
        cs.draw_cards(0, 5, &mut rng);
        cs.end_turn();
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert!(ps.hand.is_empty());
        assert_eq!(ps.discard.len(), 5);
    }

    // ---------- Damage primitive tests -----------------------------------

    #[test]
    fn damage_below_block_only_reduces_block() {
        let mut cs = ironclad_combat();
        cs.enemies[0].block = 10;
        let outcome = cs.apply_damage(CombatSide::Enemy, 0, 6);
        assert_eq!(outcome.blocked, 6);
        assert_eq!(outcome.hp_lost, 0);
        assert!(!outcome.fatal);
        assert_eq!(cs.enemies[0].block, 4);
        let max_hp = cs.enemies[0].max_hp;
        assert_eq!(cs.enemies[0].current_hp, max_hp);
    }

    #[test]
    fn damage_exceeding_block_chips_hp() {
        let mut cs = ironclad_combat();
        cs.enemies[0].block = 5;
        let max_hp = cs.enemies[0].max_hp;
        let outcome = cs.apply_damage(CombatSide::Enemy, 0, 12);
        assert_eq!(outcome.blocked, 5);
        assert_eq!(outcome.hp_lost, 7);
        assert!(!outcome.fatal);
        assert_eq!(cs.enemies[0].block, 0);
        assert_eq!(cs.enemies[0].current_hp, max_hp - 7);
    }

    #[test]
    fn lethal_damage_saturates_at_zero_hp_and_marks_fatal() {
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = 4;
        let outcome = cs.apply_damage(CombatSide::Enemy, 0, 100);
        assert_eq!(outcome.hp_lost, 4);
        assert!(outcome.fatal);
        assert_eq!(cs.enemies[0].current_hp, 0);
    }

    #[test]
    fn zero_and_negative_damage_are_noops() {
        let mut cs = ironclad_combat();
        let before_hp = cs.enemies[0].current_hp;
        assert_eq!(cs.apply_damage(CombatSide::Enemy, 0, 0), DamageOutcome::default());
        assert_eq!(cs.apply_damage(CombatSide::Enemy, 0, -5), DamageOutcome::default());
        assert_eq!(cs.enemies[0].current_hp, before_hp);
    }

    #[test]
    fn heal_saturates_at_max_hp() {
        let mut cs = ironclad_combat();
        cs.allies[0].current_hp = 50;
        let healed = cs.heal(CombatSide::Player, 0, 200);
        assert_eq!(healed, 30); // 50 -> 80 cap
        assert_eq!(cs.allies[0].current_hp, 80);
    }

    #[test]
    fn lose_hp_bypasses_block() {
        let mut cs = ironclad_combat();
        cs.allies[0].block = 20;
        let lost = cs.lose_hp(CombatSide::Player, 0, 7);
        assert_eq!(lost, 7);
        assert_eq!(cs.allies[0].block, 20);
        assert_eq!(cs.allies[0].current_hp, 73);
    }

    #[test]
    fn change_max_hp_clamps_current() {
        let mut cs = ironclad_combat();
        cs.allies[0].current_hp = 80;
        // Drop max_hp by 30 → current must follow down.
        let delta = cs.change_max_hp(CombatSide::Player, 0, -30);
        assert_eq!(delta, -30);
        assert_eq!(cs.allies[0].max_hp, 50);
        assert_eq!(cs.allies[0].current_hp, 50);

        // Gain max_hp back; current stays (does not auto-heal).
        let delta = cs.change_max_hp(CombatSide::Player, 0, 20);
        assert_eq!(delta, 20);
        assert_eq!(cs.allies[0].max_hp, 70);
        assert_eq!(cs.allies[0].current_hp, 50);
    }

    #[test]
    fn gain_block_adds() {
        let mut cs = ironclad_combat();
        cs.gain_block(CombatSide::Player, 0, 5);
        cs.gain_block(CombatSide::Player, 0, 3);
        assert_eq!(cs.allies[0].block, 8);
        cs.gain_block(CombatSide::Player, 0, -10);
        assert_eq!(cs.allies[0].block, 8);
    }

    // ---------- Power primitive tests ------------------------------------

    #[test]
    fn apply_strength_counter_accumulates() {
        let mut cs = ironclad_combat();
        // Strength is Counter + AllowNegative.
        let after = cs.apply_power(CombatSide::Player, 0, "StrengthPower", 2);
        assert_eq!(after, 2);
        let after = cs.apply_power(CombatSide::Player, 0, "StrengthPower", 3);
        assert_eq!(after, 5);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            5
        );
    }

    #[test]
    fn strength_allows_negative_via_weak_style_apply() {
        // Strength's allow_negative=true means Weak-style debuffs that
        // apply negative Strength stack downward.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", 2);
        let after = cs.apply_power(CombatSide::Player, 0, "StrengthPower", -5);
        assert_eq!(after, -3);
        // Stack stays even though negative — Strength.allow_negative=true.
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            -3
        );
    }

    #[test]
    fn poison_decrement_to_zero_removes_stack() {
        // PoisonPower is Counter + !allow_negative.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PoisonPower", 4);
        let after = cs.decrement_power(CombatSide::Enemy, 0, "PoisonPower", 4);
        assert_eq!(after, 0);
        // Should be gone from the stack, not lingering at 0.
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PoisonPower"),
            0
        );
        assert!(cs.enemies[0]
            .powers
            .iter()
            .all(|p| p.id != "PoisonPower"));
    }

    #[test]
    fn poison_decrement_below_zero_clamps_to_zero() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PoisonPower", 3);
        let after = cs.decrement_power(CombatSide::Enemy, 0, "PoisonPower", 10);
        assert_eq!(after, 0);
    }

    #[test]
    fn negative_apply_on_non_allow_negative_power_is_noop() {
        // PoisonPower doesn't allow negative. Applying -5 fresh should
        // result in nothing being added.
        let mut cs = ironclad_combat();
        let after = cs.apply_power(CombatSide::Enemy, 0, "PoisonPower", -5);
        assert_eq!(after, 0);
        assert!(cs.enemies[0].powers.is_empty());
    }

    #[test]
    fn lookup_returns_zero_when_power_absent() {
        let cs = ironclad_combat();
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            0
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            0
        );
    }

    #[test]
    fn unknown_power_id_is_noop() {
        let mut cs = ironclad_combat();
        let after = cs.apply_power(CombatSide::Player, 0, "NotAPowerName", 5);
        assert_eq!(after, 0);
        assert!(cs.allies[0].powers.is_empty());
    }

    // ---------- Damage modifier pipeline tests ---------------------------
    //
    // Expected values hand-computed from the C# spec:
    //   Strength contributes additively to dealer's outgoing damage on
    //   powered attacks. Vulnerable multiplies *1.5 on target's incoming
    //   damage. Weak multiplies *0.75 on dealer's outgoing. The pipeline
    //   does additive first then multiplicative, then truncates toward 0
    //   (C#'s `(int)decimal` cast).

    fn powered_move() -> ValueProp {
        ValueProp::MOVE
    }

    #[test]
    fn no_modifiers_returns_raw_damage() {
        let cs = ironclad_combat();
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
        );
        assert_eq!(d, 6);
    }

    #[test]
    fn strength_adds_to_dealer() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", 2);
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
        );
        assert_eq!(d, 8);
    }

    #[test]
    fn vulnerable_multiplies_on_target() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 1);
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
        );
        assert_eq!(d, 9); // 6 * 1.5 = 9
    }

    #[test]
    fn weak_multiplies_dealer_with_truncation() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "WeakPower", 3);
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
        );
        assert_eq!(d, 4); // 6 * 0.75 = 4.5 -> trunc 4
    }

    #[test]
    fn strength_plus_vulnerable_stacks_additive_then_multiplicative() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", 2);
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 1);
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
        );
        // (6 + 2) * 1.5 = 12
        assert_eq!(d, 12);
    }

    #[test]
    fn strength_vulnerable_and_weak_compose() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", 2);
        cs.apply_power(CombatSide::Player, 0, "WeakPower", 3);
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 1);
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
        );
        // (6 + 2) * 0.75 * 1.5 = 9.0
        assert_eq!(d, 9);
    }

    #[test]
    fn unpowered_props_bypass_all_modifiers() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", 5);
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 1);
        // No Move flag → not a powered attack → no modifiers apply.
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            ValueProp::NONE,
        );
        assert_eq!(d, 6);
        // Even with Move flag, Unpowered overrides.
        let d2 = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            ValueProp::MOVE.with(ValueProp::UNPOWERED),
        );
        assert_eq!(d2, 6);
    }

    #[test]
    fn negative_strength_subtracts() {
        // Weak-style debuff drives Strength below zero (allow_negative=true).
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", -2);
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
        );
        assert_eq!(d, 4);
    }

    #[test]
    fn damage_clamps_to_zero_after_modifiers() {
        // Strength of -10 on a 6-damage strike → -4, clamps to 0.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", -10);
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
        );
        assert_eq!(d, 0);
    }

    #[test]
    fn intangible_caps_incoming_damage_at_one() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "IntangiblePower", 1);
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            100,
            powered_move(),
        );
        assert_eq!(d, 1);
    }

    #[test]
    fn intangible_does_not_amplify_below_cap() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "IntangiblePower", 1);
        // Small attacks still resolve at their original value if below cap.
        // Actually Strike 6 capped at 1 → 1. Below cap means attack < 1
        // already; let's test attack = 0 (still 0).
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            0,
            powered_move(),
        );
        assert_eq!(d, 0);
    }

    #[test]
    fn intangible_only_caps_target_not_dealer() {
        // Player has Intangible. Player attacks Axebot — Axebot takes
        // full damage (Intangible doesn't cap the player's outgoing damage).
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "IntangiblePower", 1);
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
        );
        assert_eq!(d, 6);
    }

    #[test]
    fn intangible_caps_after_vulnerable_multiplier() {
        // Even with Vulnerable on Intangible target, cap still floors at 1.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "IntangiblePower", 1);
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 1);
        let d = cs.modify_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
        );
        assert_eq!(d, 1);
    }

    #[test]
    fn deal_damage_threads_modifier_then_block() {
        // Vulnerable enemy with 5 block, Strike 6 → modified 9, blocks 5,
        // chips 4 HP.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 1);
        cs.enemies[0].block = 5;
        let max_hp = cs.enemies[0].max_hp;
        let outcome = cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
        );
        assert_eq!(outcome.blocked, 5);
        assert_eq!(outcome.hp_lost, 4);
        assert_eq!(cs.enemies[0].block, 0);
        assert_eq!(cs.enemies[0].current_hp, max_hp - 4);
    }

    // ---------- Card-play action tests -----------------------------------
    //
    // OnPlay dispatch is empty in this commit — every play returns
    // `Unhandled` (after energy/routing happen). Subsequent commits
    // register Strike + Defend etc. and that result flips to `Ok`.

    /// Draw a known card name to position 0 of hand. Searches the draw
    /// pile until found and pops it to hand. Avoids depending on shuffle
    /// order for setup.
    fn draw_specific(cs: &mut CombatState, card_id: &str) {
        let ps = cs.allies[0].player.as_mut().unwrap();
        let pos = ps
            .draw
            .cards
            .iter()
            .position(|c| c.id == card_id)
            .unwrap_or_else(|| panic!("no {} in draw", card_id));
        let card = ps.draw.cards.remove(pos);
        ps.hand.cards.push(card);
    }

    #[test]
    fn play_card_unhandled_still_spends_energy_and_routes_to_discard() {
        let mut cs = ironclad_combat();
        // Bash isn't dispatched yet (will land in the archetype-expansion
        // task). Confirm the "Unhandled but state-changes-still-happen"
        // path: energy spent, card routed to discard.
        draw_specific(&mut cs, "Bash");
        let result = cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
        assert_eq!(result, PlayResult::Unhandled);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.energy, 1); // Bash costs 2.
        assert!(ps.hand.is_empty());
        assert_eq!(ps.discard.len(), 1);
        assert_eq!(ps.discard.cards[0].id, "Bash");
    }

    #[test]
    fn play_card_insufficient_energy_rejects() {
        let mut cs = ironclad_combat();
        draw_specific(&mut cs, "Bash");
        // Bash costs 2; set energy to 1 to force rejection.
        cs.allies[0].player.as_mut().unwrap().energy = 1;
        let result = cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
        assert!(matches!(
            result,
            PlayResult::InsufficientEnergy { available: 1, required: 2 }
        ));
        // Nothing should have changed.
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.energy, 1);
        assert_eq!(ps.hand.len(), 1);
        assert!(ps.discard.is_empty());
    }

    #[test]
    fn play_card_invalid_hand_idx() {
        let mut cs = ironclad_combat();
        // No cards in hand → any hand_idx is invalid.
        let result = cs.play_card(0, 0, None);
        assert_eq!(result, PlayResult::InvalidHand);
    }

    #[test]
    fn play_card_missing_target_for_attack() {
        let mut cs = ironclad_combat();
        draw_specific(&mut cs, "StrikeIronclad");
        // Strike targets AnyEnemy — None is invalid.
        let result = cs.play_card(0, 0, None);
        assert_eq!(result, PlayResult::InvalidTarget);
        // State should be unchanged (validation happens before deduction).
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.energy, 3);
        assert_eq!(ps.hand.len(), 1);
    }

    #[test]
    fn play_card_self_target_accepts_none_and_self_explicit() {
        let mut cs = ironclad_combat();
        draw_specific(&mut cs, "DefendIronclad");
        // DefendIronclad: TargetType::SelfTarget; None should work.
        // (Now dispatched → Ok rather than Unhandled.)
        let r1 = cs.play_card(0, 0, None);
        assert_eq!(r1, PlayResult::Ok);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.energy, 2); // Defend costs 1.
        assert_eq!(ps.discard.len(), 1);
    }

    #[test]
    fn play_card_invalid_target_idx() {
        let mut cs = ironclad_combat();
        draw_specific(&mut cs, "StrikeIronclad");
        // Only 2 enemies; idx=99 invalid.
        let result = cs.play_card(0, 0, Some((CombatSide::Enemy, 99)));
        assert_eq!(result, PlayResult::InvalidTarget);
    }

    #[test]
    fn play_card_invalid_player_idx() {
        let mut cs = ironclad_combat();
        let result = cs.play_card(99, 0, None);
        assert_eq!(result, PlayResult::InvalidHand);
    }

    // ---------- Combat-end detection tests --------------------------------

    #[test]
    fn fresh_combat_is_not_over() {
        let cs = ironclad_combat();
        assert!(cs.is_combat_over().is_none());
    }

    #[test]
    fn all_enemies_dead_is_victory() {
        let mut cs = ironclad_combat();
        for e in cs.enemies.iter_mut() {
            e.current_hp = 0;
        }
        assert_eq!(cs.is_combat_over(), Some(CombatResult::Victory));
    }

    #[test]
    fn partial_kill_is_not_over() {
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = 0;
        // Second Axebot still alive.
        assert!(cs.is_combat_over().is_none());
    }

    #[test]
    fn all_players_dead_is_defeat() {
        let mut cs = ironclad_combat();
        cs.allies[0].current_hp = 0;
        assert_eq!(cs.is_combat_over(), Some(CombatResult::Defeat));
    }

    #[test]
    fn defeat_takes_precedence_over_victory() {
        // Both sides 0 HP simultaneously — defeat reported, matching C#
        // ordering of player-death checks before victory checks.
        let mut cs = ironclad_combat();
        cs.allies[0].current_hp = 0;
        for e in cs.enemies.iter_mut() {
            e.current_hp = 0;
        }
        assert_eq!(cs.is_combat_over(), Some(CombatResult::Defeat));
    }

    // ---------- Strike + Defend OnPlay tests -----------------------------

    #[test]
    fn strike_ironclad_deals_six_damage() {
        let mut cs = ironclad_combat();
        draw_specific(&mut cs, "StrikeIronclad");
        let axebot_hp = cs.enemies[0].current_hp;
        let r = cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, axebot_hp - 6);
        // Energy spent, card in discard.
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.energy, 2);
        assert_eq!(ps.discard.cards[0].id, "StrikeIronclad");
    }

    #[test]
    fn upgraded_strike_deals_nine() {
        let mut cs = ironclad_combat();
        // Inject an upgraded StrikeIronclad into hand.
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            let strike = card_by_id("StrikeIronclad").unwrap();
            ps.hand.cards.push(CardInstance::from_card(strike, 1));
        }
        let axebot_hp = cs.enemies[0].current_hp;
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // Strike: base 6 + upgrade delta 3 = 9.
        assert_eq!(cs.enemies[0].current_hp, axebot_hp - 9);
    }

    #[test]
    fn strike_with_strength_threads_modifier() {
        let mut cs = ironclad_combat();
        draw_specific(&mut cs, "StrikeIronclad");
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", 2);
        let axebot_hp = cs.enemies[0].current_hp;
        let r = cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 6 base + 2 Strength = 8.
        assert_eq!(cs.enemies[0].current_hp, axebot_hp - 8);
    }

    #[test]
    fn strike_against_vulnerable_does_nine() {
        let mut cs = ironclad_combat();
        draw_specific(&mut cs, "StrikeIronclad");
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 1);
        let axebot_hp = cs.enemies[0].current_hp;
        let r = cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 6 * 1.5 = 9.
        assert_eq!(cs.enemies[0].current_hp, axebot_hp - 9);
    }

    #[test]
    fn defend_ironclad_gains_five_block() {
        let mut cs = ironclad_combat();
        draw_specific(&mut cs, "DefendIronclad");
        let r = cs.play_card(0, 0, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 5);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.energy, 2);
        assert_eq!(ps.discard.cards[0].id, "DefendIronclad");
    }

    #[test]
    fn upgraded_defend_gains_eight() {
        let mut cs = ironclad_combat();
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            let defend = card_by_id("DefendIronclad").unwrap();
            ps.hand.cards.push(CardInstance::from_card(defend, 1));
        }
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 8); // 5 + 3
    }

    /// All 5 Strike variants share the same OnPlay; confirm each dispatch
    /// arm fires (sanity: the long `|` chain in the match works for each
    /// id without subtle typos).
    #[test]
    fn all_strike_variants_dispatch() {
        for strike in &[
            "StrikeIronclad",
            "StrikeSilent",
            "StrikeDefect",
            "StrikeRegent",
            "StrikeNecrobinder",
        ] {
            let mut cs = ironclad_combat();
            let card = card_by_id(strike).unwrap();
            cs.allies[0]
                .player
                .as_mut()
                .unwrap()
                .hand
                .cards
                .push(CardInstance::from_card(card, 0));
            let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
            let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
            assert_eq!(r, PlayResult::Ok, "{strike} did not dispatch");
        }
    }

    #[test]
    fn all_defend_variants_dispatch() {
        for defend in &[
            "DefendIronclad",
            "DefendSilent",
            "DefendDefect",
            "DefendRegent",
            "DefendNecrobinder",
        ] {
            let mut cs = ironclad_combat();
            let card = card_by_id(defend).unwrap();
            cs.allies[0]
                .player
                .as_mut()
                .unwrap()
                .hand
                .cards
                .push(CardInstance::from_card(card, 0));
            let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
            let r = cs.play_card(0, hand_idx, None);
            assert_eq!(r, PlayResult::Ok, "{defend} did not dispatch");
            assert!(cs.allies[0].block > 0);
        }
    }

    // ---------- Vertical-slice integration test --------------------------

    // ---------- Axebot intent selection tests -----------------------------

    #[test]
    fn axebot_first_turn_is_boot_up() {
        let mut rng = Rng::new(1234, 0);
        let intent = pick_axebot_intent(&mut rng, None);
        assert_eq!(intent, AxebotIntent::BootUp);
    }

    #[test]
    fn axebot_subsequent_intent_is_from_random_set() {
        let mut rng = Rng::new(1234, 0);
        // After BootUp, the next pick must be one of the three random
        // branches.
        let next = pick_axebot_intent(&mut rng, Some(AxebotIntent::BootUp));
        assert!(matches!(
            next,
            AxebotIntent::OneTwo | AxebotIntent::Sharpen | AxebotIntent::HammerUppercut
        ));
    }

    #[test]
    fn axebot_sharpen_cannot_repeat_immediately() {
        // 100 picks following a Sharpen — none should be Sharpen.
        let mut rng = Rng::new(9999, 0);
        for _ in 0..100 {
            let intent = pick_axebot_intent(&mut rng, Some(AxebotIntent::Sharpen));
            assert_ne!(
                intent,
                AxebotIntent::Sharpen,
                "Sharpen should be excluded after just playing Sharpen"
            );
        }
    }

    #[test]
    fn axebot_intent_distribution_matches_weights() {
        // Over many trials following a non-Sharpen intent, expect
        // approximately {OneTwo: 2/5, Sharpen: 1/5, HammerUppercut: 2/5}.
        let mut rng = Rng::new(424242, 0);
        let trials = 10_000;
        let mut one_two = 0;
        let mut sharpen = 0;
        let mut hammer = 0;
        for _ in 0..trials {
            match pick_axebot_intent(&mut rng, Some(AxebotIntent::OneTwo)) {
                AxebotIntent::OneTwo => one_two += 1,
                AxebotIntent::Sharpen => sharpen += 1,
                AxebotIntent::HammerUppercut => hammer += 1,
                AxebotIntent::BootUp => panic!("BootUp shouldn't appear post-first-turn"),
            }
        }
        // Tolerance: 4 standard deviations on a binomial. Reaches 5%
        // tolerance per category at 10k trials.
        let expect_ot = 4000;
        let expect_sh = 2000;
        let expect_hm = 4000;
        let tol = 250;
        assert!(
            (one_two - expect_ot as i32).abs() < tol,
            "OneTwo: {one_two}"
        );
        assert!(
            (sharpen - expect_sh as i32).abs() < tol,
            "Sharpen: {sharpen}"
        );
        assert!(
            (hammer - expect_hm as i32).abs() < tol,
            "HammerUppercut: {hammer}"
        );
    }

    // ---------- Axebot move payload tests ---------------------------------

    #[test]
    fn axebot_boot_up_gains_block_and_strength() {
        let mut cs = ironclad_combat();
        execute_axebot_move(&mut cs, 0, 0, AxebotIntent::BootUp);
        assert_eq!(cs.enemies[0].block, 10);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            1
        );
        // Player unaffected.
        assert_eq!(cs.allies[0].current_hp, 80);
    }

    #[test]
    fn axebot_one_two_hits_player_twice() {
        let mut cs = ironclad_combat();
        execute_axebot_move(&mut cs, 0, 0, AxebotIntent::OneTwo);
        // 2 hits × 5 dmg, no block on player → 10 HP lost.
        assert_eq!(cs.allies[0].current_hp, 80 - 10);
    }

    #[test]
    fn axebot_one_two_block_partial() {
        // Player has 7 block. Hit 1: blocks 5, 2 block remains.
        // Hit 2: blocks 2, 3 HP lost.
        let mut cs = ironclad_combat();
        cs.allies[0].block = 7;
        execute_axebot_move(&mut cs, 0, 0, AxebotIntent::OneTwo);
        assert_eq!(cs.allies[0].block, 0);
        assert_eq!(cs.allies[0].current_hp, 80 - 3);
    }

    #[test]
    fn axebot_sharpen_adds_four_strength() {
        let mut cs = ironclad_combat();
        execute_axebot_move(&mut cs, 0, 0, AxebotIntent::Sharpen);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            4
        );
        // No damage or block effect on player.
        assert_eq!(cs.allies[0].current_hp, 80);
    }

    #[test]
    fn axebot_hammer_uppercut_damages_and_applies_weak_frail() {
        let mut cs = ironclad_combat();
        execute_axebot_move(&mut cs, 0, 0, AxebotIntent::HammerUppercut);
        assert_eq!(cs.allies[0].current_hp, 80 - 8);
        assert_eq!(cs.get_power_amount(CombatSide::Player, 0, "WeakPower"), 1);
        assert_eq!(cs.get_power_amount(CombatSide::Player, 0, "FrailPower"), 1);
    }

    #[test]
    fn axebot_strength_amplifies_one_two_hits() {
        // Bootup gives +1 Strength, then OneTwo: 2 × (5+1) = 12 damage.
        let mut cs = ironclad_combat();
        execute_axebot_move(&mut cs, 0, 0, AxebotIntent::BootUp);
        // Reset block on enemy so the next "turn" effect tests cleanly.
        cs.enemies[0].block = 0;
        execute_axebot_move(&mut cs, 0, 0, AxebotIntent::OneTwo);
        assert_eq!(cs.allies[0].current_hp, 80 - 12);
    }

    /// Realistic round-1 flow: Axebot acts (BootUp), then player acts
    /// (Strike). Validates that monster intent + power application is
    /// orthogonal to the player's card-play pipeline.
    #[test]
    fn round_one_axebot_bootup_then_strike() {
        let mut cs = ironclad_combat();
        // Axebot does BootUp.
        execute_axebot_move(&mut cs, 0, 0, AxebotIntent::BootUp);
        assert_eq!(cs.enemies[0].block, 10);

        // Player strikes the booted Axebot. 6 damage hits 10 block → no HP loss.
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            let strike = card_by_id("StrikeIronclad").unwrap();
            ps.hand.cards.push(CardInstance::from_card(strike, 0));
        }
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].block, 4); // 10 - 6
        assert_eq!(cs.enemies[0].current_hp, cs.enemies[0].max_hp);
    }

    #[test]
    fn axebot_intent_id_strings_match_c_sharp() {
        // String ids match the C# state ids — these get serialized into
        // run logs eventually.
        assert_eq!(AxebotIntent::BootUp.id(), "BOOT_UP_MOVE");
        assert_eq!(AxebotIntent::OneTwo.id(), "ONE_TWO_MOVE");
        assert_eq!(AxebotIntent::Sharpen.id(), "SHARPEN_MOVE");
        assert_eq!(AxebotIntent::HammerUppercut.id(), "HAMMER_UPPERCUT_MOVE");
    }

    /// End-to-end: Ironclad plays Strikes until both Axebots are dead.
    /// Validates state-management + modifier pipeline + OnPlay dispatch +
    /// combat-end detection composed cleanly.
    #[test]
    fn ironclad_kills_axebots_with_strikes() {
        let mut cs = ironclad_combat();
        let mut rng = Rng::new(1, 0);

        // Top up hand with 5 StrikeIroncladS by injecting them directly
        // (sidestepping the shuffle so the test stays deterministic).
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            ps.hand.cards.clear();
            let strike = card_by_id("StrikeIronclad").unwrap();
            for _ in 0..16 {
                ps.hand.cards.push(CardInstance::from_card(strike, 0));
            }
            ps.energy = 99; // Plenty of energy for the test.
        }

        // Axebot has 44 HP. 6 damage/strike → 8 strikes per Axebot, 16 total.
        assert!(cs.is_combat_over().is_none());
        for _ in 0..8 {
            let r = cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
            assert_eq!(r, PlayResult::Ok);
        }
        assert_eq!(cs.enemies[0].current_hp, 0);
        assert!(cs.is_combat_over().is_none(), "second Axebot still alive");

        for _ in 0..8 {
            let r = cs.play_card(0, 0, Some((CombatSide::Enemy, 1)));
            assert_eq!(r, PlayResult::Ok);
        }
        assert_eq!(cs.enemies[1].current_hp, 0);
        assert_eq!(cs.is_combat_over(), Some(CombatResult::Victory));

        // Player should still be alive at max HP (Axebots haven't acted).
        assert_eq!(cs.allies[0].current_hp, 80);

        let _ = rng; // silence unused if compilation is sensitive
    }
}
