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
use serde::{Deserialize, Serialize};

/// Default player energy at the start of each combat turn. (StS1/StS2
/// standard; the actual game lookup includes relic/affliction modifiers that
/// the behavior port will apply.)
pub const DEFAULT_TURN_ENERGY: i32 = 3;

/// C# `CombatSide`. `None` is a sentinel — combat is always Player or Enemy.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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
    /// Relic ids the player has at combat time. Combat hooks (Burning
    /// Blood's AfterCombatVictory, Anchor's BeforeCombatStart, etc.)
    /// dispatch over this list. Mutated only by mid-combat effects that
    /// add/remove relics; usually static for the duration.
    pub relics: Vec<String>,
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
    /// Enchantment attached to this card, if any. Damage / block hooks
    /// read this during the modifier pipeline.
    pub enchantment: Option<EnchantmentInstance>,
}

/// One enchantment attached to a card. `id` matches `EnchantmentData.id`;
/// `amount` is the stack count (Sharp's `EnchantDamageAdditive` returns
/// `Amount`; Corrupted uses a fixed factor and ignores Amount).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnchantmentInstance {
    pub id: String,
    pub amount: i32,
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
            enchantment: None,
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
    /// Append-only event log for combat replay / analysis tooling.
    /// Empty unless `log_enabled` is true (default off in training to
    /// avoid allocation overhead).
    pub combat_log: Vec<CombatEvent>,
    /// When true, mutating methods push their effects to `combat_log`.
    /// Toggle with `set_log_enabled`. Off by default.
    pub log_enabled: bool,
    /// Combat-scoped RNG stream. Used by OnPlay handlers that need
    /// randomness (PommelStrike's draw, Cinder's random hand exhaust,
    /// SwordBoomerang's random target, Juggernaut's random hit, ...).
    /// In C# these route through specific RunState.Rng.* sub-streams
    /// (CombatCardSelection / CombatTargets / ...); we squash them
    /// into one combat-scoped stream for now since bit-exact replay
    /// against a real .run already requires deeper RngSet plumbing
    /// (deferred until corpus combat-replay integration in #72 lands).
    pub rng: Rng,
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
            combat_log: Vec::new(),
            log_enabled: false,
            rng: Rng::new(0, 0),
        }
    }

    /// Toggle the verbose combat log. When true, mutating methods record
    /// their effects in `combat_log`. Off by default to avoid the
    /// allocation overhead during training runs.
    pub fn set_log_enabled(&mut self, enabled: bool) {
        self.log_enabled = enabled;
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
            combat_log: Vec::new(),
            log_enabled: false,
            rng: Rng::new(0, 0),
        }
    }

    // ---------- Turn-loop state machine -----------------------------------
    //
    // The C# CombatManager runs an async turn loop that fires hooks at each
    // boundary (BeforeSideTurnStart, AfterTurnEnd, ...). Those hooks land
    // with the behavior port. The methods below are the pure-state pieces:
    // they shuffle bookkeeping but don't run any model code.

    /// Push a relic-hook-fired log entry. Used by the hook dispatchers.
    fn log_relic_hook(&mut self, hook: &'static str, player_idx: usize, relic_id: &str) {
        if self.log_enabled {
            let round = self.round_number;
            self.combat_log.push(CombatEvent::RelicHookFired {
                round,
                hook,
                player_idx,
                relic_id: relic_id.to_string(),
            });
        }
    }

    /// Player turn → Enemy turn → Player turn. Each Player-side begin is the
    /// start of a new round; we bump `round_number` then. Sets `current_side`.
    pub fn begin_turn(&mut self, side: CombatSide) {
        if side == CombatSide::Player && self.current_side == CombatSide::Enemy {
            self.round_number += 1;
        }
        self.current_side = side;
        if self.log_enabled {
            let round = self.round_number;
            self.combat_log
                .push(CombatEvent::TurnBegan { round, side });
        }
        // Block survives one creature's *own* turn end → wipe at the start
        // of that side's next turn. This matches StS rules: block from
        // Defend persists through enemy attacks, then resets when you play
        // again. We clear on this side's begin, not on end.
        //
        // BarricadePower exception: its `ShouldClearBlock(creature)` C#
        // hook returns false when called on owner, so block is preserved
        // across the owner's turn boundary. We honor this by skipping the
        // clear for any creature that holds BarricadePower.
        match side {
            CombatSide::Player => {
                for ally in self.allies.iter_mut() {
                    if !ally.powers.iter().any(|p| p.id == "BarricadePower") {
                        ally.block = 0;
                    }
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
                    if !enemy.powers.iter().any(|p| p.id == "BarricadePower") {
                        enemy.block = 0;
                    }
                }
            }
            CombatSide::None => {}
        }
        // AfterSideTurnStart hook pass.
        // Hook firing order proper will land in #70; for now powers
        // (Poison / DemonForm) fire first, then relic AfterSideTurnStart
        // hooks (Brimstone). This matches the casual reading of the
        // C# dispatch but isn't formally validated against shipping
        // ordering — adjust when #70 lands.
        self.tick_start_of_turn_powers(side);
        self.fire_after_side_turn_start_hooks(side);
    }

    /// Fire each player's relic `AfterSideTurnStart` hooks. Called from
    /// `begin_turn`. Each relic arm gates internally on whether the
    /// passed-in side equals the owner's side.
    pub fn fire_after_side_turn_start_hooks(&mut self, side: CombatSide) {
        let pairs = self.collect_player_relics();
        for (player_idx, relic_id) in pairs {
            self.log_relic_hook("AfterSideTurnStart", player_idx, &relic_id);
            dispatch_relic_after_side_turn_start(self, player_idx, &relic_id, side);
        }
    }

    /// Generate the rewards earned by clearing this combat. Caller invokes
    /// when `is_combat_over()` returns Victory, then routes the
    /// `CombatRewards` into the strategic-layer RunState (deck additions,
    /// gold accumulation, relic / potion drops).
    ///
    /// Currently models:
    ///   - Gold: range by room type (Monster 10-20, Elite 35-45, Boss
    ///     100). Uses `next_int_range(min, max+1)` per C# exclusive-max
    ///     convention.
    ///   - Card / potion / relic rewards: deferred (need card-pool
    ///     rarity-weighted sampling + drop tables).
    ///
    /// Poverty-ascension gold multiplier deferred until ascension is
    /// plumbed into CombatState.
    pub fn generate_rewards(&self, rng: &mut Rng) -> CombatRewards {
        let (min_gold, max_gold) = gold_reward_range(self.encounter_room_type());
        let gold = if min_gold == max_gold {
            min_gold
        } else if min_gold < max_gold {
            rng.next_int_range(min_gold, max_gold + 1)
        } else {
            0
        };
        CombatRewards {
            gold,
            ..Default::default()
        }
    }

    /// Resolve the encounter's `RoomType` (Monster / Elite / Boss / …)
    /// via the static encounter data table. Returns `None` for ad-hoc
    /// combats that don't reference an EncounterData entry.
    pub fn encounter_room_type(&self) -> Option<&'static str> {
        self.encounter_id.as_ref().and_then(|id| {
            crate::encounter::by_id(id)
                .and_then(|e| e.room_type.as_deref())
                .and_then(|s| ROOM_TYPE_STRS.iter().copied().find(|known| *known == s))
        })
    }

    /// Fire each player's relic `BeforeCombatStart` hooks. Caller invokes
    /// once at the very start of combat (after `start` constructor but
    /// before any draws / turn begins). Used by Anchor (10 block) etc.
    pub fn fire_before_combat_start_hooks(&mut self) {
        let pairs = self.collect_player_relics();
        for (player_idx, relic_id) in pairs {
            self.log_relic_hook("BeforeCombatStart", player_idx, &relic_id);
            dispatch_relic_before_combat_start(self, player_idx, &relic_id);
        }
    }

    /// Fire each player's relic AfterCombatVictory hooks. Caller invokes
    /// when `is_combat_over()` returns Victory. The hook dispatcher
    /// walks each player's `relics` list and runs registered handlers.
    ///
    /// Hook firing order across hook-listening models (powers / relics
    /// / modifiers) lives in #70. For now we only fire relic hooks since
    /// they're the only `AfterCombatVictory` source we've ported.
    pub fn fire_after_combat_victory_hooks(&mut self) {
        let pairs = self.collect_player_relics();
        for (player_idx, relic_id) in pairs {
            self.log_relic_hook("AfterCombatVictory", player_idx, &relic_id);
            dispatch_relic_after_combat_victory(self, player_idx, &relic_id);
        }
    }

    /// Snapshot (player_idx, relic_id) pairs so hook dispatchers can mutate
    /// freely without iterator invalidation. Walks every player's relic
    /// list in canonical order.
    fn collect_player_relics(&self) -> Vec<(usize, String)> {
        let mut pairs: Vec<(usize, String)> = Vec::new();
        for (player_idx, creature) in self.allies.iter().enumerate() {
            if let Some(ps) = creature.player.as_ref() {
                for relic in &ps.relics {
                    pairs.push((player_idx, relic.clone()));
                }
            }
        }
        pairs
    }

    /// Apply each creature's start-of-turn power effects when that
    /// creature's side begins its turn. Currently models:
    ///   - PoisonPower: deal `Amount` damage (Unblockable | Unpowered →
    ///     block-bypassing), then decrement the stack by 1.
    ///   - DemonFormPower: apply StrengthPower(Amount) to owner.
    ///
    /// Snapshots ticks before applying so a death during one tick doesn't
    /// disrupt iteration. Poison uses `lose_hp` (bypasses block) per the
    /// `ValueProp.Unblockable` flag the C# passes.
    pub fn tick_start_of_turn_powers(&mut self, side: CombatSide) {
        let mut poison_ticks: Vec<(usize, i32)> = Vec::new();
        let mut demon_form_grants: Vec<(usize, i32)> = Vec::new();
        let list = match side {
            CombatSide::Player => &self.allies,
            CombatSide::Enemy => &self.enemies,
            CombatSide::None => return,
        };
        for (idx, creature) in list.iter().enumerate() {
            if creature.current_hp == 0 {
                continue;
            }
            for p in &creature.powers {
                match p.id.as_str() {
                    "PoisonPower" if p.amount > 0 => {
                        poison_ticks.push((idx, p.amount));
                    }
                    "DemonFormPower" if p.amount != 0 => {
                        demon_form_grants.push((idx, p.amount));
                    }
                    _ => {}
                }
            }
        }
        for (idx, amount) in poison_ticks {
            self.lose_hp(side, idx, amount);
            self.decrement_power(side, idx, "PoisonPower", 1);
        }
        for (idx, amount) in demon_form_grants {
            self.apply_power(side, idx, "StrengthPower", amount);
        }
    }

    /// Pure end-of-turn bookkeeping for the side just finishing:
    ///   - Player side: discard the hand (StS rule; cards with retain
    ///     keyword stay, but tag-based exemptions land with behavior).
    ///   - Energy refresh for players happens at the *next* `begin_turn`
    ///     after the behavior port wires in modifiers; we leave energy alone
    ///     here so the test surface stays predictable.
    ///   - Hook.AfterTurnEnd dispatch: at end of enemy turn, tick down
    ///     duration debuffs (Frail / Weak / Vulnerable). All three C#
    ///     powers gate on `side == CombatSide.Enemy` regardless of
    ///     owner, so they all tick together on the enemy-turn boundary.
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
        if self.current_side == CombatSide::Enemy {
            self.tick_duration_debuffs();
        }
        // TemporaryStrengthPower (SetupStrikePower extends this) removes
        // its stack at end of owner's turn and subtracts the same amount
        // of StrengthPower. Mirrors C#:
        //   AfterTurnEnd(side): if side == Owner.Side, Remove(this) +
        //   Apply<StrengthPower>(owner, -Sign*Amount).
        // SetupStrikePower has Sign=+1 (IsPositive); negative variants
        // (TemporaryStrengthDown) flip the sign — none ported yet.
        let side = self.current_side;
        self.tick_temporary_strength_powers(side);
        if self.log_enabled {
            let round = self.round_number;
            let side = self.current_side;
            self.combat_log.push(CombatEvent::TurnEnded { round, side });
        }
    }

    /// Fire `AfterTurnEnd` for `TemporaryStrengthPower`-style powers
    /// on the side whose turn just ended. Each known temporary-strength
    /// power id, on creatures whose side matches: remove the stack, and
    /// subtract `sign * amount` from StrengthPower (sign = +1 for
    /// positive variants like SetupStrikePower; negative variants would
    /// use -1, none ported yet).
    fn tick_temporary_strength_powers(&mut self, side: CombatSide) {
        const TEMP_STRENGTH_POWERS: &[(&str, i32)] = &[
            ("SetupStrikePower", 1),
            // ManglePower: IsPositive=false → applies -Amount Strength.
            // On owner-side turn end, removes itself and re-applies
            // +Amount Strength to undo.
            ("ManglePower", -1),
        ];
        let n_allies = self.allies.len();
        let n_enemies = self.enemies.len();
        let mut undo: Vec<(CombatSide, usize, &'static str, i32, i32)> =
            Vec::new();
        for i in 0..n_allies {
            if side != CombatSide::Player {
                continue;
            }
            for (id, sign) in TEMP_STRENGTH_POWERS {
                let amount = self.get_power_amount(CombatSide::Player, i, id);
                if amount != 0 {
                    undo.push((CombatSide::Player, i, id, *sign, amount));
                }
            }
        }
        for i in 0..n_enemies {
            if side != CombatSide::Enemy {
                continue;
            }
            for (id, sign) in TEMP_STRENGTH_POWERS {
                let amount = self.get_power_amount(CombatSide::Enemy, i, id);
                if amount != 0 {
                    undo.push((CombatSide::Enemy, i, id, *sign, amount));
                }
            }
        }
        for (s, idx, id, sign, amount) in undo {
            // Remove the temp-strength stack entirely.
            self.decrement_power(s, idx, id, amount);
            // Subtract sign * amount of StrengthPower (undoing the
            // BeforeApplied silent grant).
            self.apply_power(s, idx, "StrengthPower", -(sign * amount));
        }
    }

    /// Decrement every duration-debuff stack on every creature by 1,
    /// removing the stack on transition to 0. C# `AfterTurnEnd` on
    /// FrailPower / WeakPower / VulnerablePower each call
    /// `PowerCmd.TickDownDuration(this)` when `side == CombatSide.Enemy`,
    /// regardless of who owns the power, so all three tick on the same
    /// boundary.
    fn tick_duration_debuffs(&mut self) {
        const TICKING: &[&str] =
            &["FrailPower", "WeakPower", "VulnerablePower"];
        let n_allies = self.allies.len();
        let n_enemies = self.enemies.len();
        for i in 0..n_allies {
            for power_id in TICKING {
                if self.get_power_amount(CombatSide::Player, i, power_id) > 0 {
                    self.decrement_power(CombatSide::Player, i, power_id, 1);
                }
            }
        }
        for i in 0..n_enemies {
            for power_id in TICKING {
                if self.get_power_amount(CombatSide::Enemy, i, power_id) > 0 {
                    self.decrement_power(CombatSide::Enemy, i, power_id, 1);
                }
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

    /// Convenience wrapper: draw `n` using the combat-scoped `self.rng`.
    /// OnPlay handlers call this rather than threading an external Rng
    /// (which can't co-borrow with `&mut self` here). The temp-swap is
    /// the standard workaround for "method that uses one field on `self`
    /// while another field is also borrowed mutably."
    pub fn draw_cards_self_rng(&mut self, player_idx: usize, n: i32) -> i32 {
        let mut rng = std::mem::replace(&mut self.rng, Rng::new(0, 0));
        let drawn = self.draw_cards(player_idx, n, &mut rng);
        self.rng = rng;
        drawn
    }

    /// Pick one card from the player's hand uniformly at random via
    /// `self.rng` and move it to the exhaust pile. No-op if the hand is
    /// empty. Returns the exhausted card's id (for logging / tests).
    /// Mirrors C# `RunState.Rng.CombatCardSelection.NextItem(hand)
    /// → CardCmd.Exhaust`.
    pub fn exhaust_random_card_in_hand(
        &mut self,
        player_idx: usize,
    ) -> Option<String> {
        let hand_len = self
            .allies
            .get(player_idx)
            .and_then(|c| c.player.as_ref())
            .map(|ps| ps.hand.len())
            .unwrap_or(0);
        if hand_len == 0 {
            return None;
        }
        let idx = self.rng.next_int_range(0, hand_len as i32) as usize;
        let ps = self.allies[player_idx].player.as_mut().unwrap();
        let card = ps.hand.cards.remove(idx);
        let id = card.id.clone();
        ps.exhaust.cards.push(card);
        Some(id)
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
        let outcome = {
            let Some(target) = creature_mut(self, side, target_idx) else {
                return DamageOutcome::default();
            };
            damage_creature(target, amount)
        };
        if self.log_enabled && (outcome.blocked > 0 || outcome.hp_lost > 0) {
            let round = self.round_number;
            self.combat_log.push(CombatEvent::DamageDealt {
                round,
                side,
                target_idx,
                amount,
                outcome,
            });
        }
        outcome
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

    /// Add `amount` block to a creature, threading through the block
    /// modifier pipeline (Dexterity additive, Frail multiplicative). The
    /// default `ValueProp::MOVE` matches card-play and monster-move block —
    /// the contexts where C# `IsPoweredCardOrMonsterMoveBlock` returns true.
    /// Floors at 0 (no negative block).
    pub fn gain_block(
        &mut self,
        side: CombatSide,
        target_idx: usize,
        amount: i32,
    ) -> i32 {
        self.gain_block_with_props(side, target_idx, amount, ValueProp::MOVE)
    }

    /// Like `gain_block`, but lets the caller pass explicit `ValueProp`
    /// flags. Relic / scripted block sources flag `UNPOWERED` so they
    /// bypass Frail/Dexterity (matches C# `ValueProp.Unpowered` on
    /// `BlockVar`).
    pub fn gain_block_with_props(
        &mut self,
        side: CombatSide,
        target_idx: usize,
        amount: i32,
        props: ValueProp,
    ) -> i32 {
        let modified = self.modify_block((side, target_idx), amount, props);
        let actual = {
            let Some(target) = creature_mut(self, side, target_idx) else {
                return 0;
            };
            let actual = modified.max(0);
            target.block += actual;
            actual
        };
        if self.log_enabled && actual > 0 {
            let round = self.round_number;
            self.combat_log.push(CombatEvent::BlockGained {
                round,
                side,
                target_idx,
                amount: actual,
            });
        }
        actual
    }

    /// Compute final integer block after applying every active block
    /// modifier on the gainer (Dexterity additive, Frail multiplicative).
    /// Mirrors C# `Hook.ModifyBlock` / `ModifyBlockInternal` for the
    /// player-self / monster-self block-gain path.
    ///
    /// Both Dexterity (`ModifyBlockAdditive`) and Frail
    /// (`ModifyBlockMultiplicative`) gate on
    /// `props.IsPoweredCardOrMonsterMoveBlock()` — same `Move && !Unpowered`
    /// shape as the attack pipeline's `is_powered_attack`. Status-source
    /// block (Anchor's `Unpowered` flag, etc.) bypasses the pipeline.
    pub fn modify_block(
        &self,
        gainer: (CombatSide, usize),
        raw: i32,
        props: ValueProp,
    ) -> i32 {
        let mut num = raw as f64;
        let powers = creature_powers(self, gainer);
        for power in powers {
            num += power_block_additive(power, props);
        }
        for power in powers {
            num *= power_block_multiplicative(power, props);
        }
        let clamped = num.max(0.0);
        clamped as i32
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
        let result = self.apply_power_inner(side, target_idx, power_id, amount);
        if self.log_enabled {
            let round = self.round_number;
            self.combat_log.push(CombatEvent::PowerApplied {
                round,
                side,
                target_idx,
                power_id: power_id.to_string(),
                delta: amount,
                result_amount: result,
            });
        }
        result
    }

    fn apply_power_inner(
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
        self.modify_damage_with_enchantment(dealer, target, raw, props, None)
    }

    /// Same pipeline as `modify_damage` but threads a card's enchantment
    /// through the pre-power additive + multiplicative phases. C#
    /// `Hook.ModifyDamage` applies `cardSource.Enchantment.EnchantDamage*`
    /// BEFORE iterating per-power hooks.
    pub fn modify_damage_with_enchantment(
        &self,
        dealer: (CombatSide, usize),
        target: (CombatSide, usize),
        raw: i32,
        props: ValueProp,
        enchantment: Option<&EnchantmentInstance>,
    ) -> i32 {
        let mut num = raw as f64;

        if let Some(ench) = enchantment {
            num += enchantment_damage_additive(&ench.id, ench.amount, props);
            num *= enchantment_damage_multiplicative(&ench.id, ench.amount, props);
        }

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

    /// Enchantment-aware variant. Card OnPlay handlers route through this
    /// path so an attached `EnchantmentInstance` participates in the
    /// modifier pipeline.
    pub fn deal_damage_enchanted(
        &mut self,
        dealer: (CombatSide, usize),
        target: (CombatSide, usize),
        raw: i32,
        props: ValueProp,
        enchantment: Option<&EnchantmentInstance>,
    ) -> DamageOutcome {
        let modified =
            self.modify_damage_with_enchantment(dealer, target, raw, props, enchantment);
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
        let x_value;
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
            card_data = data;
            // X-cost cards (Whirlwind): consume all available energy; the
            // resolved X is the integer count of energy spent. Matches
            // C# CardModel.ResolveEnergyXValue / energy-cost-X gating.
            // Non-X cards use the card's printed cost (with the
            // energy_cost_upgrade_delta already applied at CardInstance
            // creation time).
            if data.has_energy_cost_x {
                energy_cost = ps.energy.max(0);
                x_value = energy_cost;
            } else {
                energy_cost = card.current_energy_cost;
                x_value = 0;
            }
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

        // 5. Dispatch OnPlay. The handler may mutate cs freely. The
        //    played card's enchantment (if any) is forwarded for damage
        //    modifier participation.
        let handled = dispatch_on_play(
            self,
            &card_id,
            upgrade_level,
            played_card.enchantment.as_ref(),
            player_idx,
            target,
            x_value,
        );

        // 6. Route the card per its type / keywords. Status/Curse cards
        //    auto-exhaust on play; non-status cards check their
        //    CanonicalKeywords for "Exhaust" (Cinder, MoltenFist,
        //    TrueGrit, ...). Everything else discards.
        let dest = if matches!(card_data.card_type, CardType::Status | CardType::Curse)
            || card_data.keywords.iter().any(|k| k == "Exhaust")
        {
            PileType::Exhaust
        } else {
            PileType::Discard
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
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
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
    enchantment: Option<&EnchantmentInstance>,
    player_idx: usize,
    target: Option<(CombatSide, usize)>,
    x_value: i32,
) -> bool {
    match card_id {
        // All 5 Strike variants: deal Damage to single AnyEnemy target,
        // routed through the modifier pipeline with ValueProp.Move. The
        // played card's enchantment threads through pre-power modifiers.
        "StrikeIronclad" | "StrikeSilent" | "StrikeDefect" | "StrikeRegent"
        | "StrikeNecrobinder" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            true
        }
        // All 5 Defend variants: gain Block on self. `gain_block` routes
        // through `modify_block` with ValueProp::MOVE, so Frail/Dexterity
        // on the player apply automatically.
        "DefendIronclad" | "DefendSilent" | "DefendDefect" | "DefendRegent"
        | "DefendNecrobinder" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let block = canonical_int_value(card, "Block", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            true
        }
        // Bash (Ironclad basic): 8 damage + 2 Vulnerable to single enemy.
        // Upgrade: +2 damage, +1 Vulnerable.
        "Bash" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            let vuln = canonical_int_value(card, "Vulnerable", upgrade_level);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            cs.apply_power(target.0, target.1, "VulnerablePower", vuln);
            true
        }
        // Neutralize (Silent basic): 3 damage + 1 Weak to single enemy.
        // Upgrade: +1 damage, +1 Weak.
        "Neutralize" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            let weak = canonical_int_value(card, "Weak", upgrade_level);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            cs.apply_power(target.0, target.1, "WeakPower", weak);
            true
        }
        // Thunderclap (Ironclad common): 4 damage + 1 Vulnerable to ALL
        // enemies. Upgrade: +3 damage. Each enemy takes the damage
        // independently (block recomputes per target). Dead enemies skip.
        "Thunderclap" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            let vuln = canonical_int_value(card, "Vulnerable", upgrade_level);
            let n = cs.enemies.len();
            for i in 0..n {
                if cs.enemies[i].current_hp == 0 {
                    continue;
                }
                cs.deal_damage_enchanted(
                    (CombatSide::Player, player_idx),
                    (CombatSide::Enemy, i),
                    damage,
                    ValueProp::MOVE,
                    enchantment,
                );
                // C# applies to HittableEnemies (still-alive); skip dead
                // before AND after damage to match.
                if cs.enemies[i].current_hp > 0 {
                    cs.apply_power(CombatSide::Enemy, i, "VulnerablePower", vuln);
                }
            }
            true
        }
        // IronWave (Ironclad common): 5 damage to single enemy + 5 block
        // on self. Upgrade: +2 each. C# order is block-then-damage; we
        // match.
        "IronWave" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            let block = canonical_int_value(card, "Block", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            true
        }
        // TwinStrike (Ironclad common): 5 damage × 2 hits to single
        // enemy. Upgrade: +2 per hit (becomes 7×2). C# uses
        // `.WithHitCount(2)` — each hit goes through modifiers independently.
        "TwinStrike" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            for _ in 0..2 {
                cs.deal_damage_enchanted(
                    (CombatSide::Player, player_idx),
                    target,
                    damage,
                    ValueProp::MOVE,
                    enchantment,
                );
            }
            true
        }
        // Anger (Ironclad common): 6 damage + add a copy of self to
        // discard pile. Upgrade: +2 damage. The clone is `played_card`
        // pre-routing, so we can't directly access it from here —
        // instead instantiate a fresh CardInstance via from_card.
        // Enchantment is NOT inherited by the clone (matches C# —
        // CreateClone strips enchantments from cloned cards).
        "Anger" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            // Append clone to discard at the same upgrade level.
            let clone = CardInstance::from_card(card, upgrade_level);
            if let Some(ps) = cs.allies[player_idx].player.as_mut() {
                ps.discard.cards.push(clone);
            }
            true
        }
        // Inflame (Ironclad uncommon): apply 2 Strength to self.
        // Upgrade: +1 Strength.
        "Inflame" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let strength = canonical_int_value(card, "StrengthPower", upgrade_level);
            cs.apply_power(
                CombatSide::Player,
                player_idx,
                "StrengthPower",
                strength,
            );
            true
        }
        // FiendFire (Ironclad rare Exhaust Attack, 2 cost, AnyEnemy):
        // exhaust the entire remaining hand; deal 7 damage (10
        // upgraded) per exhausted card. Each hit threads through the
        // modifier pipeline independently (Strength composes per hit,
        // matching C# WithHitCount). FiendFire itself is already
        // removed from hand at dispatch time and routes to exhaust via
        // the Exhaust keyword.
        "FiendFire" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            // Drain hand → exhaust, count along the way.
            let count = {
                let ps = cs.allies[player_idx].player.as_mut().unwrap();
                let n = ps.hand.cards.len();
                let drained: Vec<_> = ps.hand.cards.drain(..).collect();
                ps.exhaust.cards.extend(drained);
                n as i32
            };
            for _ in 0..count {
                if cs.enemies[target.1].current_hp == 0 {
                    break;
                }
                cs.deal_damage_enchanted(
                    (CombatSide::Player, player_idx),
                    target,
                    damage,
                    ValueProp::MOVE,
                    enchantment,
                );
            }
            true
        }
        // Mangle (Ironclad rare Attack, 3 cost, AnyEnemy): 15 damage
        // (20 upgraded) + apply ManglePower equal to StrengthLoss (10,
        // 15 upgraded). ManglePower extends TemporaryStrengthPower with
        // IsPositive=false → applies negative Strength on the target
        // for one of its turns. tick_temporary_strength_powers undoes
        // the Strength loss at target-side turn end.
        "Mangle" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            let strength_loss =
                canonical_int_value(card, "StrengthLoss", upgrade_level);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            cs.apply_power(target.0, target.1, "ManglePower", strength_loss);
            cs.apply_power(target.0, target.1, "StrengthPower", -strength_loss);
            true
        }
        // Impervious (Ironclad rare Exhaust Skill, 2 cost, Self): 30
        // block (40 upgraded). Exhaust keyword handles routing.
        "Impervious" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let block = canonical_int_value(card, "Block", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            true
        }
        // SetupStrike (Ironclad common Strike-tagged Attack, 1 cost):
        // 7 damage + 2 SetupStrikePower. SetupStrikePower extends
        // TemporaryStrengthPower → on apply, silently grants the same
        // amount of StrengthPower; at end of owner's turn, removes
        // itself and subtracts the same Strength. We replicate the
        // BeforeApplied side-effect here (apply both stacks together);
        // tick_temporary_strength_powers in end_turn handles the undo.
        "SetupStrike" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            let strength = canonical_int_value(card, "Strength", upgrade_level);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            cs.apply_power(
                CombatSide::Player,
                player_idx,
                "SetupStrikePower",
                strength,
            );
            cs.apply_power(
                CombatSide::Player,
                player_idx,
                "StrengthPower",
                strength,
            );
            true
        }
        // Feed (Ironclad rare Exhaust Attack, 1 cost): 10 damage; if
        // the attack kills, gain 3 max HP permanently. Upgrade: +2
        // damage, +1 max HP. C# uses GainMaxHp which both raises
        // max_hp AND heals current_hp by the same delta (standard StS
        // pattern keeping % HP unchanged), so we do both here.
        //
        // ShouldOwnerDeathTriggerFatal — C# filters out minion/non-
        // meaningful deaths via per-power hooks. Not modeled yet; all
        // enemy deaths count. Reopen with MinionPower port.
        "Feed" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            let max_hp_gain = canonical_int_value(card, "MaxHp", upgrade_level);
            let outcome = cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            if outcome.fatal {
                cs.change_max_hp(CombatSide::Player, player_idx, max_hp_gain);
                cs.heal(CombatSide::Player, player_idx, max_hp_gain);
            }
            true
        }
        // Barricade (Ironclad rare Power, 3 cost, Self): apply 1
        // BarricadePower. Its ShouldClearBlock hook (handled in
        // begin_turn) preserves block across the owner's turn
        // boundary. Single stack — never accumulates.
        "Barricade" => {
            cs.apply_power(
                CombatSide::Player,
                player_idx,
                "BarricadePower",
                1,
            );
            true
        }
        // PerfectedStrike (Ironclad common Attack, 2 cost): deal
        // CalculationBase + ExtraDamage * (count of Strike-tagged cards
        // in player's full combat deck — draw + hand + discard +
        // exhaust). C# uses `PlayerCombatState.AllCards` which is the
        // union of all piles. Upgrade bumps ExtraDamage by 1.
        "PerfectedStrike" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let base_damage = canonical_int_value(card, "CalculationBase", upgrade_level);
            let per_strike = canonical_int_value(card, "ExtraDamage", upgrade_level);
            let strike_count = if let Some(ps) = cs.allies[player_idx].player.as_ref() {
                let count_in = |pile: &CardPile| -> i32 {
                    pile.cards
                        .iter()
                        .filter(|ci| {
                            card_by_id(&ci.id)
                                .map(|d| d.tags.iter().any(|t| t == "Strike"))
                                .unwrap_or(false)
                        })
                        .count() as i32
                };
                count_in(&ps.draw)
                    + count_in(&ps.hand)
                    + count_in(&ps.discard)
                    + count_in(&ps.exhaust)
            } else {
                0
            };
            let damage = base_damage + per_strike * strike_count;
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            true
        }
        // SwordBoomerang (Ironclad common Attack, 1 cost): 3 damage
        // per hit × Repeat hits (3 base, 4 upgraded). Each hit picks a
        // fresh random alive enemy via self.rng; if every enemy dies
        // mid-volley, remaining hits are skipped (matches C#
        // `TargetingRandomOpponents`, which re-samples HittableEnemies
        // each iteration and bails when empty).
        "SwordBoomerang" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            let hits = canonical_int_value(card, "Repeat", upgrade_level);
            for _ in 0..hits {
                let alive: Vec<usize> = cs
                    .enemies
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.current_hp > 0)
                    .map(|(i, _)| i)
                    .collect();
                if alive.is_empty() {
                    break;
                }
                let pick = cs.rng.next_int_range(0, alive.len() as i32) as usize;
                let idx = alive[pick];
                cs.deal_damage_enchanted(
                    (CombatSide::Player, player_idx),
                    (CombatSide::Enemy, idx),
                    damage,
                    ValueProp::MOVE,
                    enchantment,
                );
            }
            true
        }
        // Cinder (Ironclad common Attack, 2 cost): 18 damage (24
        // upgraded) + exhaust a random card from hand. Like TrueGrit
        // the card itself goes to discard — the Exhaust hover-tip is a
        // UI hint for the effect, not a self-exhaust keyword.
        "Cinder" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            cs.exhaust_random_card_in_hand(player_idx);
            true
        }
        // TrueGrit (Ironclad common Skill): 7 block (9 upgraded) +
        // exhaust a random card from hand. The card itself routes to
        // discard normally — the Exhaust hover-tip in C# is a UI hint
        // for the *effect* (the hand-pick gets exhausted), not a
        // CanonicalKeywords entry. C# distinguishes base vs upgraded:
        // base picks randomly, upgraded prompts the player. Until
        // player-choice machinery lands, fall back to random for both.
        // Choice-routing fidelity is a known deviation tracked
        // alongside Headbutt / Cinder etc.
        "TrueGrit" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let block = canonical_int_value(card, "Block", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            cs.exhaust_random_card_in_hand(player_idx);
            true
        }
        // PommelStrike (Ironclad common Attack): 9 damage + draw N
        // (N=1, 2 upgraded). Upgrade bumps both Damage and Cards.
        "PommelStrike" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            let cards = canonical_int_value(card, "Cards", upgrade_level);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            cs.draw_cards_self_rng(player_idx, cards);
            true
        }
        // ShrugItOff (Ironclad common Skill): 8 block (11 upgraded) +
        // draw 1. Block routes through gain_block so Frail/Dex apply.
        "ShrugItOff" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let block = canonical_int_value(card, "Block", upgrade_level);
            let cards = canonical_int_value(card, "Cards", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            cs.draw_cards_self_rng(player_idx, cards);
            true
        }
        // DemonForm (Ironclad rare Power, 3 cost, Self): apply 2
        // DemonFormPower (3 upgraded). DemonFormPower's
        // AfterSideTurnStart hook then applies StrengthPower(Amount) to
        // owner on each player-turn begin (wired into
        // tick_start_of_turn_powers).
        "DemonForm" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let stacks = canonical_int_value(card, "StrengthPower", upgrade_level);
            cs.apply_power(
                CombatSide::Player,
                player_idx,
                "DemonFormPower",
                stacks,
            );
            true
        }
        // Breakthrough (Ironclad common Attack, AoE): lose 1 HP
        // unblockable THEN 9 damage to ALL enemies (13 upgraded). C# is
        // strict on order: HpLoss first, then attack. Dead enemies skip.
        "Breakthrough" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let hp_loss = canonical_int_value(card, "HpLoss", upgrade_level);
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            cs.lose_hp(CombatSide::Player, player_idx, hp_loss);
            let n = cs.enemies.len();
            for i in 0..n {
                if cs.enemies[i].current_hp == 0 {
                    continue;
                }
                cs.deal_damage_enchanted(
                    (CombatSide::Player, player_idx),
                    (CombatSide::Enemy, i),
                    damage,
                    ValueProp::MOVE,
                    enchantment,
                );
            }
            true
        }
        // BloodWall (Ironclad common Skill, 2 cost): lose 2 HP
        // unblockable THEN 16 block (20 upgraded). C# order is
        // HpLoss → GainBlock.
        "BloodWall" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let hp_loss = canonical_int_value(card, "HpLoss", upgrade_level);
            let block = canonical_int_value(card, "Block", upgrade_level);
            cs.lose_hp(CombatSide::Player, player_idx, hp_loss);
            cs.gain_block(CombatSide::Player, player_idx, block);
            true
        }
        // Tremble (Ironclad common Exhaust Skill): apply 3 Vulnerable
        // to single enemy. Upgrade: +1 Vulnerable. Exhausts via the
        // keyword-driven routing.
        "Tremble" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let vuln = canonical_int_value(card, "Vulnerable", upgrade_level);
            cs.apply_power(target.0, target.1, "VulnerablePower", vuln);
            true
        }
        // Apparition (Ancient Skill, Ethereal + Exhaust): apply 1
        // Intangible to self. Upgrade strips Ethereal (handled at card
        // schema level, not here). The IntangiblePower modifier already
        // caps incoming damage at 1 via the existing damage pipeline.
        "Apparition" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            // PowerVar<IntangiblePower> indexes by "IntangiblePower"
            // (matches Inflame's StrengthPower convention, not the
            // suffix-stripped LegSweep "Weak" form). canonical_int_value
            // matches via the generic field.
            let stacks = canonical_int_value(card, "IntangiblePower", upgrade_level);
            cs.apply_power(
                CombatSide::Player,
                player_idx,
                "IntangiblePower",
                stacks,
            );
            true
        }
        // MoltenFist (Ironclad common Exhaust Attack): 10 damage (14
        // upgraded) + if target is alive AND already Vulnerable,
        // re-apply that many stacks. C# samples the count BEFORE the
        // re-apply (so the doubling-each-play behavior is the natural
        // outcome: 2 → 4 → 8 → ...). Exhaust routing is handled by the
        // keyword-driven pile selection above; this dispatcher arm only
        // executes the effect.
        "MoltenFist" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            // Sample Vulnerable AFTER damage (C# uses cardPlay.Target.IsAlive
            // and re-fetches Vulnerable post-damage).
            let still_alive = match target.0 {
                CombatSide::Player => cs.allies.get(target.1).map(|c| c.current_hp > 0),
                CombatSide::Enemy => cs.enemies.get(target.1).map(|c| c.current_hp > 0),
                CombatSide::None => None,
            }
            .unwrap_or(false);
            if still_alive {
                let cur_vuln =
                    cs.get_power_amount(target.0, target.1, "VulnerablePower");
                if cur_vuln > 0 {
                    cs.apply_power(target.0, target.1, "VulnerablePower", cur_vuln);
                }
            }
            true
        }
        // Bloodletting (Ironclad common Skill, 0 cost): lose 3 HP +
        // gain 2 energy (3 upgraded). C# damage call carries
        // Unblockable|Unpowered|Move so it bypasses block AND the
        // modifier pipeline — equivalent to our `lose_hp`. Energy gain
        // is uncapped (matches StS — can exceed max energy mid-turn).
        "Bloodletting" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let hp_loss = canonical_int_value(card, "HpLoss", upgrade_level);
            let energy = canonical_int_value(card, "Energy", upgrade_level);
            cs.lose_hp(CombatSide::Player, player_idx, hp_loss);
            if let Some(ps) = cs.allies[player_idx].player.as_mut() {
                ps.energy += energy;
            }
            true
        }
        // Whirlwind (Ironclad uncommon X-cost Attack): hit ALL enemies
        // X times for 5 damage each (8 upgraded). C# uses
        // `DamageCmd.Attack(...).WithHitCount(num).TargetingAllOpponents`
        // where `num = ResolveEnergyXValue()` (= the energy we spent
        // computing as x_value above). Each hit goes through the modifier
        // pipeline independently; dead enemies skip mid-way through.
        "Whirlwind" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            for _ in 0..x_value {
                if cs.enemies.iter().all(|e| e.current_hp == 0) {
                    break;
                }
                let n = cs.enemies.len();
                for i in 0..n {
                    if cs.enemies[i].current_hp == 0 {
                        continue;
                    }
                    cs.deal_damage_enchanted(
                        (CombatSide::Player, player_idx),
                        (CombatSide::Enemy, i),
                        damage,
                        ValueProp::MOVE,
                        enchantment,
                    );
                }
            }
            true
        }
        // LegSweep (Silent uncommon Skill): gain 11 block + apply 2 Weak
        // to target. Upgrade: +3 block, +1 Weak. Block uses ValueProp.Move
        // so Frail/Dexterity flow through. Both effects run regardless of
        // each other's outcome (matches C# OnPlay sequencing).
        "LegSweep" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let block = canonical_int_value(card, "Block", upgrade_level);
            // Upgrade-delta keys for power vars use the suffix-stripped
            // form ("Weak", not "WeakPower"), matching Neutralize/Bash.
            let weak = canonical_int_value(card, "Weak", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            cs.apply_power(target.0, target.1, "WeakPower", weak);
            true
        }
        // BodySlam (Ironclad common): damage equals caster's current
        // block. C# uses CalculatedDamage = CalculationBase(0) +
        // ExtraDamage(1) * Owner.Creature.Block — i.e. just `block`.
        // Upgrade reduces energy cost by 1 (already in data table).
        "BodySlam" => {
            let Some(target) = target else { return false; };
            let block = cs.allies[player_idx].block;
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                block,
                ValueProp::MOVE,
                enchantment,
            );
            true
        }
        _ => false,
    }
}

/// Resolve the effective integer value of one of a card's canonical vars
/// at a given upgrade level. Sums the base value with any
/// `upgrade_deltas` whose `var_kind` matches, scaled by `upgrade_level`.
///
/// Var-matching rules (mirroring C# dot-accessor / indexer semantics):
///   - exact `v.kind == var_kind` (Damage, Block)
///   - `v.generic == var_kind` (StrengthPower)
///   - `v.generic` stripped of "Power" suffix matches (Vulnerable ↔
///     `PowerVar<VulnerablePower>`)
///   - `v.key == var_kind` for keyed `DynamicVar("key", val)`
fn canonical_int_value(card: &CardData, var_kind: &str, upgrade_level: i32) -> i32 {
    let base = card
        .canonical_vars
        .iter()
        .find(|v| var_matches(v, var_kind))
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

// ---------- Relic combat hook dispatch --------------------------------
//
// Each relic with a combat hook adds an arm to one of the dispatcher
// functions below. Currently only AfterCombatVictory is plumbed; the
// other hook points (BeforeCombatStart, AfterDamageTaken, BeforeSideTurnStart)
// land alongside #70 (hook firing order infrastructure).

/// Dispatch a single relic's `BeforeCombatStart` hook. Used by relics
/// that grant block / draw / energy at combat open.
fn dispatch_relic_before_combat_start(
    cs: &mut CombatState,
    player_idx: usize,
    relic_id: &str,
) {
    match relic_id {
        // Anchor: gain 10 block at combat start. C# uses
        // `BlockVar(10m, ValueProp.Unpowered)` — Unpowered bypasses any
        // Frail-style block modifiers. We pass UNPOWERED so the
        // modify_block pipeline skips Frail/Dexterity.
        "Anchor" => {
            cs.gain_block_with_props(
                CombatSide::Player,
                player_idx,
                10,
                ValueProp::UNPOWERED,
            );
        }
        _ => {}
    }
}

/// Dispatch a single relic's `AfterCombatVictory` hook. Walks per-relic
/// arms; relics with no AfterCombatVictory behavior fall through.
fn dispatch_relic_after_combat_victory(
    cs: &mut CombatState,
    player_idx: usize,
    relic_id: &str,
) {
    match relic_id {
        // BurningBlood: if owner not dead, heal HealVar (6). Matches the
        // C# guard `if (!base.Owner.Creature.IsDead)`.
        "BurningBlood" => {
            if cs
                .allies
                .get(player_idx)
                .map(|c| c.current_hp > 0)
                .unwrap_or(false)
            {
                cs.heal(CombatSide::Player, player_idx, 6);
            }
        }
        _ => {}
    }
}

/// Dispatch a single relic's `AfterSideTurnStart` hook. Fires on every
/// `begin_turn`; per-relic arms gate on side and any other state.
fn dispatch_relic_after_side_turn_start(
    cs: &mut CombatState,
    player_idx: usize,
    relic_id: &str,
    side: CombatSide,
) {
    match relic_id {
        // Brimstone: at start of owner-side turn, +2 Strength to self,
        // +1 Strength to every alive enemy. C# uses keyed PowerVars
        // ("SelfStrength" / "EnemyStrength"); we resolve via
        // relic_var_value.
        "Brimstone" => {
            if side != CombatSide::Player {
                return;
            }
            let self_s = relic_var_value("Brimstone", "SelfStrength").unwrap_or(0);
            let enemy_s = relic_var_value("Brimstone", "EnemyStrength").unwrap_or(0);
            cs.apply_power(CombatSide::Player, player_idx, "StrengthPower", self_s);
            let n = cs.enemies.len();
            for i in 0..n {
                if cs.enemies[i].current_hp == 0 {
                    continue;
                }
                cs.apply_power(CombatSide::Enemy, i, "StrengthPower", enemy_s);
            }
        }
        _ => {}
    }
}

/// Resolve a relic var's integer value by key. Relic vars don't upgrade,
/// so this is just a flat lookup against the canonical_vars table.
fn relic_var_value(relic_id: &str, key: &str) -> Option<i32> {
    let relic = crate::relic::by_id(relic_id)?;
    relic
        .canonical_vars
        .iter()
        .find(|v| v.key.as_deref() == Some(key))
        .and_then(|v| v.base_value)
        .map(|x| x as i32)
}

fn var_matches(v: &crate::card::CardVar, var_kind: &str) -> bool {
    if v.kind == var_kind {
        return true;
    }
    if let Some(g) = &v.generic {
        if g == var_kind {
            return true;
        }
        if let Some(stripped) = g.strip_suffix("Power") {
            if stripped == var_kind {
                return true;
            }
        }
    }
    if let Some(k) = &v.key {
        if k == var_kind {
            return true;
        }
    }
    false
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

/// Per-power `ModifyBlockAdditive` contribution. Returns 0 for
/// non-applicable powers / non-powered block. The "owner == gainer" check
/// is structurally enforced: this is only invoked over the gainer's own
/// power list.
fn power_block_additive(power: &PowerInstance, props: ValueProp) -> f64 {
    if !props.is_powered_attack() {
        return 0.0;
    }
    match power.id.as_str() {
        // DexterityPower.ModifyBlockAdditive: +Amount on powered block
        // gains by the owner. allow_negative=true → Dex can be negative.
        "DexterityPower" => power.amount as f64,
        _ => 0.0,
    }
}

/// Per-power `ModifyBlockMultiplicative` contribution. Returns 1.0 for
/// non-applicable powers / non-powered block.
fn power_block_multiplicative(power: &PowerInstance, props: ValueProp) -> f64 {
    if !props.is_powered_attack() {
        return 1.0;
    }
    match power.id.as_str() {
        // FrailPower.ModifyBlockMultiplicative: ×0.75 on powered block
        // gains by the owner.
        "FrailPower" => 0.75,
        _ => 1.0,
    }
}

/// Per-enchantment `EnchantDamageAdditive` contribution. Returns 0 for
/// non-applicable enchantments / non-powered attacks (matches C# pattern).
fn enchantment_damage_additive(ench_id: &str, amount: i32, props: ValueProp) -> f64 {
    if !props.is_powered_attack() {
        return 0.0;
    }
    match ench_id {
        // Sharp: adds `base.Amount` to damage on powered attacks. Only
        // CanEnchantCardType(Attack) — but that's enforced at attach time,
        // not in the modifier pipeline.
        "Sharp" => amount as f64,
        _ => 0.0,
    }
}

/// Per-enchantment `EnchantDamageMultiplicative` contribution. Returns
/// the identity 1.0 for non-applicable enchantments / non-powered attacks.
fn enchantment_damage_multiplicative(
    ench_id: &str,
    _amount: i32,
    props: ValueProp,
) -> f64 {
    if !props.is_powered_attack() {
        return 1.0;
    }
    match ench_id {
        // Corrupted: fixed ×1.5 on powered attacks. Ignores Amount.
        "Corrupted" => 1.5,
        _ => 1.0,
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
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CombatResult {
    Victory,
    Defeat,
}

/// One event in the combat replay log. Pushed by `CombatState` mutations
/// when `log_enabled` is true. Schema is append-only — adding variants
/// is safe; renaming requires bumping the replay-tooling consumer.
#[derive(Clone, Debug, PartialEq)]
pub enum CombatEvent {
    /// `apply_damage` ran (post-modifier pipeline). Multi-hit attacks
    /// emit one event per hit.
    DamageDealt {
        round: i32,
        side: CombatSide,
        target_idx: usize,
        amount: i32,
        outcome: DamageOutcome,
    },
    /// `gain_block` ran with a positive amount.
    BlockGained {
        round: i32,
        side: CombatSide,
        target_idx: usize,
        amount: i32,
    },
    /// `apply_power` / `decrement_power` ran. `result_amount` is the
    /// resulting stack count.
    PowerApplied {
        round: i32,
        side: CombatSide,
        target_idx: usize,
        power_id: String,
        delta: i32,
        result_amount: i32,
    },
    /// `begin_turn` fired.
    TurnBegan { round: i32, side: CombatSide },
    /// `end_turn` fired.
    TurnEnded { round: i32, side: CombatSide },
    /// A relic combat hook ran (Anchor/BurningBlood/...).
    RelicHookFired {
        round: i32,
        hook: &'static str,
        player_idx: usize,
        relic_id: String,
    },
}

/// End-of-combat rewards. Caller (RunState orchestration layer) routes the
/// fields into deck additions / gold accumulation / relic drops as
/// appropriate.
///
/// Card / potion / relic fields are placeholders for future expansion —
/// currently only `gold` is populated.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatRewards {
    pub gold: i32,
    /// Card-reward choice triplet. Empty until card-reward generation lands.
    pub card_choices: Vec<String>,
    /// Single potion id if a potion dropped.
    pub potion: Option<String>,
    /// Single relic id (elite / boss drop).
    pub relic: Option<String>,
}

/// Known room-type strings the gold table recognizes. Strings come from
/// `EncounterData.room_type`; this list mirrors the C# `RoomType` enum
/// arms checked in `MinGoldReward` / `MaxGoldReward`.
const ROOM_TYPE_STRS: &[&str] = &["Monster", "Elite", "Boss"];

/// Gold reward (min_inclusive, max_inclusive) by room type. From C#
/// `EncounterModel.MinGoldReward` / `MaxGoldReward` at A0 with no Poverty
/// ascension. Unknown room types drop nothing.
fn gold_reward_range(room_type: Option<&str>) -> (i32, i32) {
    match room_type {
        Some("Monster") => (10, 20),
        Some("Elite") => (35, 45),
        Some("Boss") => (100, 100),
        _ => (0, 0),
    }
}

/// Outcome of a single `apply_damage` call. Useful for combat-log replay
/// and for upstream hooks that need to know whether HP actually moved.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Relic ids the player has at combat start. Combat hooks dispatch
    /// over this list.
    pub relics: Vec<String>,
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
                relics: setup.relics,
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
            relics: ironclad.starting_relics.clone(),
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
            relics: ironclad.starting_relics.clone(),
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

    // ---------- Duration debuff tick-down tests --------------------------

    #[test]
    fn vulnerable_on_enemy_ticks_at_end_of_enemy_turn() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 2);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            1
        );
    }

    #[test]
    fn vulnerable_does_not_tick_at_end_of_player_turn() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 2);
        cs.current_side = CombatSide::Player;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            2
        );
    }

    #[test]
    fn frail_on_player_ticks_at_end_of_enemy_turn() {
        // C# FrailPower.AfterTurnEnd fires on `side == Enemy` regardless
        // of owner — even player-owned Frail ticks on the enemy boundary.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 2);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "FrailPower"),
            1
        );
    }

    #[test]
    fn weak_on_enemy_ticks_at_end_of_enemy_turn() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "WeakPower", 3);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "WeakPower"),
            2
        );
    }

    #[test]
    fn duration_debuff_at_one_tick_removes_stack() {
        // Frail/Weak/Vulnerable have allow_negative=false so transition
        // to 0 should drop the PowerInstance entirely (handled by
        // apply_power), not linger at 0.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 1);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "FrailPower"),
            0
        );
        assert!(cs.allies[0]
            .powers
            .iter()
            .all(|p| p.id != "FrailPower"));
    }

    #[test]
    fn tick_handles_all_three_debuffs_at_once() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 2);
        cs.apply_power(CombatSide::Player, 0, "WeakPower", 2);
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 2);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "FrailPower"),
            1
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "WeakPower"),
            1
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            1
        );
    }

    #[test]
    fn non_duration_powers_dont_tick() {
        // Strength/Poison/Intangible/Dexterity are not in the ticking
        // set — they should be untouched at end-of-turn.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", 3);
        cs.apply_power(CombatSide::Player, 0, "DexterityPower", 2);
        cs.apply_power(CombatSide::Enemy, 0, "PoisonPower", 5);
        cs.apply_power(CombatSide::Enemy, 0, "IntangiblePower", 1);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            3
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "DexterityPower"),
            2
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PoisonPower"),
            5
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "IntangiblePower"),
            1
        );
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

    // ---------- Block modifier pipeline tests ----------------------------

    #[test]
    fn modify_block_no_modifiers_passes_through() {
        let cs = ironclad_combat();
        let b = cs.modify_block((CombatSide::Player, 0), 5, ValueProp::MOVE);
        assert_eq!(b, 5);
    }

    #[test]
    fn frail_reduces_block_to_three_quarters() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 1);
        // 5 * 0.75 = 3.75 → trunc → 3
        let b = cs.modify_block((CombatSide::Player, 0), 5, ValueProp::MOVE);
        assert_eq!(b, 3);
    }

    #[test]
    fn frail_only_applies_to_powered_block() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 1);
        // Unpowered source (Anchor-style) bypasses Frail.
        let b = cs.modify_block(
            (CombatSide::Player, 0),
            10,
            ValueProp::UNPOWERED,
        );
        assert_eq!(b, 10);
    }

    #[test]
    fn dexterity_adds_to_block() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "DexterityPower", 2);
        // 5 + 2 = 7
        let b = cs.modify_block((CombatSide::Player, 0), 5, ValueProp::MOVE);
        assert_eq!(b, 7);
    }

    #[test]
    fn negative_dexterity_subtracts_block() {
        // Dexterity.allow_negative=true. -3 Dex on 5 block → 2.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "DexterityPower", -3);
        let b = cs.modify_block((CombatSide::Player, 0), 5, ValueProp::MOVE);
        assert_eq!(b, 2);
    }

    #[test]
    fn frail_and_dexterity_compose_additive_then_multiplicative() {
        // C# order: ModifyBlockAdditive (Dex) THEN
        // ModifyBlockMultiplicative (Frail). (5+2)*0.75 = 5.25 → trunc → 5.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "DexterityPower", 2);
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 1);
        let b = cs.modify_block((CombatSide::Player, 0), 5, ValueProp::MOVE);
        assert_eq!(b, 5);
    }

    #[test]
    fn defend_with_frail_gains_three_block() {
        // Defend = 5 block. With Frail: 5 * 0.75 = 3.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 1);
        let card = card_by_id("DefendIronclad").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        assert_eq!(cs.allies[0].block, 0);
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 3);
    }

    #[test]
    fn defend_with_dexterity_gains_seven_block() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "DexterityPower", 2);
        let card = card_by_id("DefendIronclad").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 7);
    }

    #[test]
    fn anchor_block_bypasses_frail() {
        // Anchor passes UNPOWERED so 10 block lands fully even with Frail.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 1);
        cs.gain_block_with_props(
            CombatSide::Player,
            0,
            10,
            ValueProp::UNPOWERED,
        );
        assert_eq!(cs.allies[0].block, 10);
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

    // ---------- Enchantment pipeline tests ------------------------------

    #[test]
    fn sharp_enchantment_adds_amount_to_attack_damage() {
        // Sharp's EnchantDamageAdditive returns Amount on powered attacks.
        let cs = ironclad_combat();
        let ench = EnchantmentInstance {
            id: "Sharp".to_string(),
            amount: 3,
        };
        let d = cs.modify_damage_with_enchantment(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
            Some(&ench),
        );
        assert_eq!(d, 9); // 6 + 3
    }

    #[test]
    fn corrupted_enchantment_multiplies_attack_damage_by_1_5() {
        let cs = ironclad_combat();
        let ench = EnchantmentInstance {
            id: "Corrupted".to_string(),
            amount: 1, // ignored — Corrupted is a fixed factor
        };
        let d = cs.modify_damage_with_enchantment(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
            Some(&ench),
        );
        assert_eq!(d, 9); // 6 * 1.5
    }

    #[test]
    fn enchantments_skip_on_non_powered_attacks() {
        let cs = ironclad_combat();
        let sharp = EnchantmentInstance { id: "Sharp".to_string(), amount: 5 };
        let d = cs.modify_damage_with_enchantment(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            ValueProp::NONE, // not a powered attack
            Some(&sharp),
        );
        assert_eq!(d, 6); // Sharp contributes 0
    }

    #[test]
    fn enchantment_applied_before_powers() {
        // C# order: enchantment additive then enchantment multiplicative,
        // THEN per-power additive (Strength), then per-power multiplicative.
        // Sharp +3 on Strike 6 = 9. Then Strength +2 → 11. Then Vulnerable
        // ×1.5 → 16. Truncation to i32 = 16.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", 2);
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 1);
        let sharp = EnchantmentInstance { id: "Sharp".to_string(), amount: 3 };
        let d = cs.modify_damage_with_enchantment(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
            Some(&sharp),
        );
        assert_eq!(d, 16);
    }

    #[test]
    fn strike_with_sharp_enchantment_dispatches_through_play_card() {
        // End-to-end through play_card: attach Sharp+2 to a StrikeIronclad,
        // play it, verify damage is 6+2 = 8.
        let mut cs = ironclad_combat();
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            let strike = card_by_id("StrikeIronclad").unwrap();
            let mut card = CardInstance::from_card(strike, 0);
            card.enchantment = Some(EnchantmentInstance {
                id: "Sharp".to_string(),
                amount: 2,
            });
            ps.hand.cards.push(card);
        }
        let axebot_hp = cs.enemies[0].current_hp;
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, axebot_hp - 8);
    }

    #[test]
    fn unknown_enchantment_id_is_noop() {
        let cs = ironclad_combat();
        let ench = EnchantmentInstance {
            id: "NotAnEnchantment".to_string(),
            amount: 99,
        };
        let d = cs.modify_damage_with_enchantment(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            powered_move(),
            Some(&ench),
        );
        assert_eq!(d, 6);
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
        // Survivor isn't dispatched yet (its "discard 1 from hand" branch
        // needs a card-selection prompt that isn't ported). Confirm the
        // "Unhandled but state-changes-still-happen" path: energy spent,
        // card routed to discard.
        let survivor = card_by_id("Survivor").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(survivor, 0));
        let result = cs.play_card(0, 0, None);
        assert_eq!(result, PlayResult::Unhandled);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.energy, 2); // Survivor costs 1.
        assert!(ps.hand.is_empty());
        assert_eq!(ps.discard.cards.iter().any(|c| c.id == "Survivor"), true);
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
    fn bash_deals_damage_and_applies_vulnerable() {
        let mut cs = ironclad_combat();
        let bash = card_by_id("Bash").unwrap();
        cs.allies[0].player.as_mut().unwrap().hand.cards.push(
            CardInstance::from_card(bash, 0),
        );
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 8 damage from Bash; Vulnerable not yet on target during the hit
        // (apply happens after the damage), so no 1.5x amplification on
        // this play.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 8);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            2
        );
    }

    #[test]
    fn upgraded_bash_does_ten_damage_and_three_vulnerable() {
        let mut cs = ironclad_combat();
        let bash = card_by_id("Bash").unwrap();
        cs.allies[0].player.as_mut().unwrap().hand.cards.push(
            CardInstance::from_card(bash, 1),
        );
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 10);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            3
        );
    }

    #[test]
    fn thunderclap_hits_all_enemies_and_applies_vulnerable() {
        let mut cs = ironclad_combat();
        let tc = card_by_id("Thunderclap").unwrap();
        cs.allies[0].player.as_mut().unwrap().hand.cards.push(
            CardInstance::from_card(tc, 0),
        );
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_e0 = cs.enemies[0].current_hp;
        let hp_e1 = cs.enemies[1].current_hp;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_e0 - 4);
        assert_eq!(cs.enemies[1].current_hp, hp_e1 - 4);
        for i in 0..cs.enemies.len() {
            assert_eq!(
                cs.get_power_amount(CombatSide::Enemy, i, "VulnerablePower"),
                1
            );
        }
    }

    #[test]
    fn iron_wave_deals_damage_and_grants_block() {
        let mut cs = ironclad_combat();
        let iw = card_by_id("IronWave").unwrap();
        cs.allies[0].player.as_mut().unwrap().hand.cards.push(
            CardInstance::from_card(iw, 0),
        );
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 5);
        assert_eq!(cs.allies[0].block, 5);
    }

    #[test]
    fn twin_strike_hits_twice() {
        let mut cs = ironclad_combat();
        let ts = card_by_id("TwinStrike").unwrap();
        cs.allies[0].player.as_mut().unwrap().hand.cards.push(
            CardInstance::from_card(ts, 0),
        );
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 5 × 2 = 10, no block.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 10);
    }

    #[test]
    fn upgraded_twin_strike_does_fourteen_damage() {
        let mut cs = ironclad_combat();
        let ts = card_by_id("TwinStrike").unwrap();
        cs.allies[0].player.as_mut().unwrap().hand.cards.push(
            CardInstance::from_card(ts, 1),
        );
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 7 × 2 = 14.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 14);
    }

    #[test]
    fn anger_deals_damage_and_clones_to_discard() {
        let mut cs = ironclad_combat();
        let anger = card_by_id("Anger").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(anger, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let discard_before = cs.allies[0].player.as_ref().unwrap().discard.len();
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 6 damage.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 6);
        // Played card + clone — discard grew by 2.
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.discard.len(), discard_before + 2);
        // Both entries are Anger.
        let n_anger = ps
            .discard
            .cards
            .iter()
            .filter(|c| c.id == "Anger")
            .count();
        assert_eq!(n_anger, 2);
    }

    #[test]
    fn upgraded_anger_does_eight_damage() {
        let mut cs = ironclad_combat();
        let anger = card_by_id("Anger").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(anger, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 8);
    }

    #[test]
    fn inflame_applies_strength_to_self() {
        let mut cs = ironclad_combat();
        let inflame = card_by_id("Inflame").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(inflame, 0));
        // Need enough energy — Inflame costs 1.
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            2
        );
    }

    #[test]
    fn upgraded_inflame_grants_three_strength() {
        let mut cs = ironclad_combat();
        let inflame = card_by_id("Inflame").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(inflame, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            3
        );
    }

    #[test]
    fn body_slam_damage_equals_block() {
        let mut cs = ironclad_combat();
        let bs = card_by_id("BodySlam").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(bs, 0));
        cs.allies[0].block = 17;
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // Damage = 17 (block); player block unchanged by playing
        // BodySlam (block only spends on incoming damage).
        assert_eq!(cs.enemies[0].current_hp, hp_before - 17);
        assert_eq!(cs.allies[0].block, 17);
    }

    #[test]
    fn body_slam_with_zero_block_deals_zero() {
        let mut cs = ironclad_combat();
        let bs = card_by_id("BodySlam").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(bs, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before);
    }

    #[test]
    fn upgraded_body_slam_costs_zero_energy() {
        // BodySlam upgrade reduces cost from 1 -> 0 via energy_cost_upgrade_delta.
        let bs = card_by_id("BodySlam").unwrap();
        let upgraded = CardInstance::from_card(bs, 1);
        assert_eq!(upgraded.current_energy_cost, 0);
    }

    // ---------- FiendFire tests ------------------------------------------

    #[test]
    fn fiend_fire_exhausts_hand_and_hits_per_card() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let strike = card_by_id("StrikeIronclad").unwrap();
        let defend = card_by_id("DefendIronclad").unwrap();
        let ff = card_by_id("FiendFire").unwrap();
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            ps.hand.cards.clear();
            ps.hand.cards.push(CardInstance::from_card(strike, 0));
            ps.hand.cards.push(CardInstance::from_card(defend, 0));
            ps.hand.cards.push(CardInstance::from_card(strike, 0));
            ps.hand.cards.push(CardInstance::from_card(ff, 0));
        }
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, 3, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 3 cards remaining in hand → 3 hits × 7 = 21 damage.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 21);
        let ps = cs.allies[0].player.as_ref().unwrap();
        // Hand empty; all 4 cards (including FiendFire) in exhaust.
        assert_eq!(ps.hand.len(), 0);
        assert_eq!(ps.exhaust.len(), 4);
    }

    #[test]
    fn fiend_fire_with_only_self_in_hand_does_zero_damage() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let ff = card_by_id("FiendFire").unwrap();
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            ps.hand.cards.clear();
            ps.hand.cards.push(CardInstance::from_card(ff, 0));
        }
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // No other cards → 0 hits.
        assert_eq!(cs.enemies[0].current_hp, hp_before);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.exhaust.len(), 1);
    }

    #[test]
    fn upgraded_fiend_fire_does_ten_per_hit() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let strike = card_by_id("StrikeIronclad").unwrap();
        let ff = card_by_id("FiendFire").unwrap();
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            ps.hand.cards.clear();
            ps.hand.cards.push(CardInstance::from_card(strike, 0));
            ps.hand.cards.push(CardInstance::from_card(strike, 0));
            ps.hand.cards.push(CardInstance::from_card(ff, 1));
        }
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, 2, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 2 cards × 10 = 20 damage.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 20);
    }

    #[test]
    fn fiend_fire_strength_composes_per_hit() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", 3);
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let strike = card_by_id("StrikeIronclad").unwrap();
        let ff = card_by_id("FiendFire").unwrap();
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            ps.hand.cards.clear();
            ps.hand.cards.push(CardInstance::from_card(strike, 0));
            ps.hand.cards.push(CardInstance::from_card(strike, 0));
            ps.hand.cards.push(CardInstance::from_card(ff, 0));
        }
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, 2, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 2 hits × (7 + 3 Strength) = 20.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 20);
    }

    // ---------- Impervious tests -----------------------------------------

    #[test]
    fn impervious_grants_thirty_block_and_exhausts() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let card = card_by_id("Impervious").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 30);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert!(ps.exhaust.cards.iter().any(|c| c.id == "Impervious"));
    }

    #[test]
    fn upgraded_impervious_grants_forty_block() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let card = card_by_id("Impervious").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 40);
    }

    // ---------- Mangle tests ---------------------------------------------

    #[test]
    fn mangle_damages_and_temporarily_drops_target_strength() {
        let mut cs = ironclad_combat();
        // Pre-buff enemy with Strength to see the temporary drop.
        cs.apply_power(CombatSide::Enemy, 0, "StrengthPower", 5);
        cs.allies[0].player.as_mut().unwrap().energy = 3;
        let card = card_by_id("Mangle").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 15);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ManglePower"),
            10
        );
        // 5 (pre-existing) - 10 (Mangle) = -5 Strength on enemy now.
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            -5
        );
    }

    #[test]
    fn mangle_strength_loss_clears_at_end_of_enemy_turn() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "StrengthPower", 5);
        cs.allies[0].player.as_mut().unwrap().energy = 3;
        let card = card_by_id("Mangle").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        // Enemy turn passes; end_turn undoes ManglePower.
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ManglePower"),
            0
        );
        // +10 Strength restored → 5 - 10 + 10 = 5.
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            5
        );
    }

    #[test]
    fn upgraded_mangle_does_twenty_damage_and_fifteen_strength_loss() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 3;
        let card = card_by_id("Mangle").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 20);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ManglePower"),
            15
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            -15
        );
    }

    // ---------- SetupStrike tests ----------------------------------------

    #[test]
    fn setup_strike_deals_damage_and_grants_temp_strength() {
        let mut cs = ironclad_combat();
        let card = card_by_id("SetupStrike").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 7);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "SetupStrikePower"),
            2
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            2
        );
    }

    #[test]
    fn temp_strength_clears_at_end_of_player_turn() {
        // Play SetupStrike → temp Strength up by 2. End player turn →
        // both SetupStrikePower and Strength bonus go away.
        let mut cs = ironclad_combat();
        let card = card_by_id("SetupStrike").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "SetupStrikePower"),
            0
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            0
        );
    }

    #[test]
    fn temp_strength_preserves_permanent_strength_from_inflame() {
        // Inflame grants permanent Strength (2). Then SetupStrike adds
        // 2 temp Strength → total 4. End of turn drops 2 (the temp),
        // leaving permanent Strength = 2.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", 2);
        let card = card_by_id("SetupStrike").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            4
        );
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            2
        );
    }

    #[test]
    fn upgraded_setup_strike_does_nine_damage_and_three_strength() {
        let mut cs = ironclad_combat();
        let card = card_by_id("SetupStrike").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // Base 7 damage + 2 strength (applied AFTER damage but before
        // damage is consumed, since the C# applies are sequential and
        // damage runs first). So damage is still 7, then +3 Strength.
        // Actually wait — in our model the strength applies AFTER damage
        // (apply_power is the second call), so damage is computed
        // without the new strength. That matches C#: PowerCmd.Apply
        // runs after DamageCmd.Attack.Execute.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 9);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            3
        );
    }

    // ---------- Feed tests -----------------------------------------------

    #[test]
    fn feed_non_lethal_no_max_hp_gain() {
        let mut cs = ironclad_combat();
        let card = card_by_id("Feed").unwrap();
        let max_before = cs.allies[0].max_hp;
        let cur_before = cs.allies[0].current_hp;
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 10);
        // Target survived → no max HP gain.
        assert_eq!(cs.allies[0].max_hp, max_before);
        assert_eq!(cs.allies[0].current_hp, cur_before);
    }

    #[test]
    fn feed_lethal_grants_three_max_hp_and_heals_three() {
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = 5;
        let card = card_by_id("Feed").unwrap();
        let max_before = cs.allies[0].max_hp;
        cs.allies[0].current_hp = 50;
        let cur_before = cs.allies[0].current_hp;
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, 0);
        // +3 max HP and +3 current HP (% HP preserved style).
        assert_eq!(cs.allies[0].max_hp, max_before + 3);
        assert_eq!(cs.allies[0].current_hp, cur_before + 3);
    }

    #[test]
    fn upgraded_feed_kills_with_twelve_grants_four() {
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = 6;
        let card = card_by_id("Feed").unwrap();
        let max_before = cs.allies[0].max_hp;
        cs.allies[0].current_hp = 40;
        let cur_before = cs.allies[0].current_hp;
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, 0);
        // +4 max HP, +4 current HP.
        assert_eq!(cs.allies[0].max_hp, max_before + 4);
        assert_eq!(cs.allies[0].current_hp, cur_before + 4);
    }

    #[test]
    fn feed_routes_to_exhaust_via_keyword() {
        let mut cs = ironclad_combat();
        let card = card_by_id("Feed").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert!(ps.exhaust.cards.iter().any(|c| c.id == "Feed"));
        assert!(ps.discard.cards.iter().all(|c| c.id != "Feed"));
    }

    // ---------- Barricade tests ------------------------------------------

    #[test]
    fn barricade_applies_single_stack() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 3;
        let card = card_by_id("Barricade").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "BarricadePower"),
            1
        );
    }

    #[test]
    fn barricade_preserves_block_across_player_turn_start() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "BarricadePower", 1);
        cs.allies[0].block = 20;
        cs.begin_turn(CombatSide::Player);
        // Block survives the turn-start clear.
        assert_eq!(cs.allies[0].block, 20);
    }

    #[test]
    fn no_barricade_clears_block_on_turn_start_as_usual() {
        let mut cs = ironclad_combat();
        cs.allies[0].block = 20;
        cs.begin_turn(CombatSide::Player);
        // Baseline: no Barricade → block wipes.
        assert_eq!(cs.allies[0].block, 0);
    }

    #[test]
    fn barricade_on_enemy_preserves_enemy_block_too() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "BarricadePower", 1);
        cs.enemies[0].block = 15;
        cs.begin_turn(CombatSide::Enemy);
        assert_eq!(cs.enemies[0].block, 15);
    }

    // ---------- PerfectedStrike tests ------------------------------------

    #[test]
    fn perfected_strike_baseline_counts_starter_strikes() {
        // Starter Ironclad deck has 5 Strike-tagged cards (StrikeIronclad).
        // PerfectedStrike base = 6 + 2*5 = 16.
        let mut cs = ironclad_combat();
        let card = card_by_id("PerfectedStrike").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 16);
    }

    #[test]
    fn upgraded_perfected_strike_uses_three_per_strike() {
        // 6 + 3*5 = 21.
        let mut cs = ironclad_combat();
        let card = card_by_id("PerfectedStrike").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 21);
    }

    #[test]
    fn perfected_strike_counts_strikes_across_all_piles() {
        // Move some Strikes to discard + a SetupStrike (also Strike-tagged)
        // into hand. Total = 5 + 1 = 6 Strikes. Damage = 6 + 2*6 = 18.
        let mut cs = ironclad_combat();
        let setup_strike = card_by_id("SetupStrike").unwrap();
        let perfected = card_by_id("PerfectedStrike").unwrap();
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            ps.hand
                .cards
                .push(CardInstance::from_card(setup_strike, 0));
            ps.hand
                .cards
                .push(CardInstance::from_card(perfected, 0));
            // Move two StrikeIronclads from draw → discard to verify
            // discard counts too.
            for _ in 0..2 {
                let i = ps
                    .draw
                    .cards
                    .iter()
                    .position(|c| c.id == "StrikeIronclad")
                    .unwrap();
                let c = ps.draw.cards.remove(i);
                ps.discard.cards.push(c);
            }
        }
        // PerfectedStrike is at the last hand index.
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 5 StrikeIronclad + 1 SetupStrike still in piles after play.
        // Damage = 6 + 2*6 = 18.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 18);
    }

    // ---------- SwordBoomerang tests -------------------------------------

    #[test]
    fn sword_boomerang_dispatches_three_hits_total() {
        let mut cs = ironclad_combat();
        cs.rng = Rng::new(42, 0);
        let card = card_by_id("SwordBoomerang").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let total_hp_before: i32 = cs.enemies.iter().map(|e| e.current_hp).sum();
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        let total_hp_after: i32 = cs.enemies.iter().map(|e| e.current_hp).sum();
        // 3 hits × 3 damage = 9 total damage distributed across enemies.
        assert_eq!(total_hp_before - total_hp_after, 9);
    }

    #[test]
    fn upgraded_sword_boomerang_does_four_hits() {
        let mut cs = ironclad_combat();
        cs.rng = Rng::new(42, 0);
        let card = card_by_id("SwordBoomerang").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let total_hp_before: i32 = cs.enemies.iter().map(|e| e.current_hp).sum();
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        let total_hp_after: i32 = cs.enemies.iter().map(|e| e.current_hp).sum();
        // Upgrade adds 1 hit → 4 hits × 3 damage = 12.
        assert_eq!(total_hp_before - total_hp_after, 12);
    }

    #[test]
    fn sword_boomerang_skips_dead_enemies_mid_volley() {
        // Set up: two enemies, both at 3 HP. SwordBoomerang base form
        // does 3×3=9 total damage; with both enemies dropping after one
        // hit each, the third hit has no alive target and is skipped.
        let mut cs = ironclad_combat();
        cs.rng = Rng::new(42, 0);
        cs.enemies[0].current_hp = 3;
        cs.enemies[1].current_hp = 3;
        let card = card_by_id("SwordBoomerang").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        // Both enemies dead. No panic on the would-be-third hit.
        assert_eq!(cs.enemies[0].current_hp, 0);
        assert_eq!(cs.enemies[1].current_hp, 0);
    }

    // ---------- Cinder tests ---------------------------------------------

    #[test]
    fn cinder_deals_eighteen_and_exhausts_one_hand_card() {
        let mut cs = ironclad_combat();
        cs.rng = Rng::new(42, 0);
        let strike = card_by_id("StrikeIronclad").unwrap();
        let cinder = card_by_id("Cinder").unwrap();
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            ps.hand.cards.clear();
            ps.hand.cards.push(CardInstance::from_card(strike, 0));
            ps.hand.cards.push(CardInstance::from_card(cinder, 0));
        }
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, 1, Some((CombatSide::Enemy, 0))); // Cinder
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 18);
        let ps = cs.allies[0].player.as_ref().unwrap();
        // Cinder → discard, the Strike → exhaust.
        assert!(ps.discard.cards.iter().any(|c| c.id == "Cinder"));
        assert_eq!(ps.exhaust.len(), 1);
        assert_eq!(ps.hand.len(), 0);
    }

    #[test]
    fn upgraded_cinder_deals_twentyfour() {
        let mut cs = ironclad_combat();
        let cinder = card_by_id("Cinder").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(cinder, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 24);
    }

    // ---------- TrueGrit (RNG hand exhaust) tests ------------------------

    #[test]
    fn true_grit_grants_block_discards_self_and_exhausts_one_hand_card() {
        let mut cs = ironclad_combat();
        // Deterministic rng so the picked index is reproducible.
        cs.rng = Rng::new(42, 0);
        let strike = card_by_id("StrikeIronclad").unwrap();
        let defend = card_by_id("DefendIronclad").unwrap();
        let truegrit = card_by_id("TrueGrit").unwrap();
        {
            let ps = cs.allies[0].player.as_mut().unwrap();
            ps.hand.cards.clear();
            ps.hand.cards.push(CardInstance::from_card(strike, 0));
            ps.hand.cards.push(CardInstance::from_card(defend, 0));
            ps.hand
                .cards
                .push(CardInstance::from_card(truegrit, 0));
        }
        let r = cs.play_card(0, 2, None); // TrueGrit at index 2
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 7);
        let ps = cs.allies[0].player.as_ref().unwrap();
        // TrueGrit itself → discard (Skill, no CanonicalKeywords Exhaust).
        // One hand card → exhaust. So: hand=1, exhaust=1, discard=1.
        assert_eq!(ps.hand.len(), 1);
        assert_eq!(ps.exhaust.len(), 1);
        assert!(ps.discard.cards.iter().any(|c| c.id == "TrueGrit"));
    }

    #[test]
    fn upgraded_true_grit_grants_nine_block() {
        let mut cs = ironclad_combat();
        let truegrit = card_by_id("TrueGrit").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(truegrit, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        // 7 + 2 = 9.
        assert_eq!(cs.allies[0].block, 9);
    }

    #[test]
    fn true_grit_with_only_self_in_hand_no_extra_exhaust() {
        // No other cards in hand → no second exhaust, just the block.
        // TrueGrit itself goes to discard (not exhaust).
        let mut cs = ironclad_combat();
        let truegrit = card_by_id("TrueGrit").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .clear();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(truegrit, 0));
        let r = cs.play_card(0, 0, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 7);
        let ps = cs.allies[0].player.as_ref().unwrap();
        // Empty exhaust; TrueGrit lives in discard.
        assert!(ps.exhaust.is_empty());
        assert_eq!(ps.discard.len(), 1);
        assert_eq!(ps.discard.cards[0].id, "TrueGrit");
    }

    #[test]
    fn exhaust_random_card_uses_combat_rng_deterministically() {
        // Two combats with same seed → same exhaust pick (both pick
        // the same hand index).
        let strike = card_by_id("StrikeIronclad").unwrap();
        let defend = card_by_id("DefendIronclad").unwrap();
        let make_cs = || {
            let mut cs = ironclad_combat();
            cs.rng = Rng::new(12345, 0);
            let ps = cs.allies[0].player.as_mut().unwrap();
            ps.hand.cards.clear();
            ps.hand.cards.push(CardInstance::from_card(strike, 0));
            ps.hand.cards.push(CardInstance::from_card(defend, 0));
            ps.hand.cards.push(CardInstance::from_card(strike, 0));
            cs
        };
        let mut cs1 = make_cs();
        let mut cs2 = make_cs();
        let id1 = cs1.exhaust_random_card_in_hand(0);
        let id2 = cs2.exhaust_random_card_in_hand(0);
        assert!(id1.is_some());
        assert_eq!(id1, id2);
    }

    // ---------- PommelStrike + ShrugItOff (RNG draw) tests --------------

    fn populate_draw_pile(cs: &mut CombatState, n: usize) {
        let strike = card_by_id("StrikeIronclad").unwrap();
        let ps = cs.allies[0].player.as_mut().unwrap();
        for _ in 0..n {
            ps.draw
                .cards
                .push(CardInstance::from_card(strike, 0));
        }
    }

    #[test]
    fn pommel_strike_deals_nine_and_draws_one() {
        let mut cs = ironclad_combat();
        populate_draw_pile(&mut cs, 3);
        let card = card_by_id("PommelStrike").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let hand_before = cs.allies[0].player.as_ref().unwrap().hand.len();
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 9);
        // PommelStrike consumed (hand−1) then drew 1 (+1). Net hand_before.
        // hand_before counted the PommelStrike itself; after play it's
        // gone but +1 drawn → hand size = hand_before.
        assert_eq!(
            cs.allies[0].player.as_ref().unwrap().hand.len(),
            hand_before
        );
    }

    #[test]
    fn upgraded_pommel_strike_draws_two() {
        let mut cs = ironclad_combat();
        populate_draw_pile(&mut cs, 5);
        let card = card_by_id("PommelStrike").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let hand_before = cs.allies[0].player.as_ref().unwrap().hand.len();
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // Upgrade: damage 9+1=10, draw 1+1=2. Net hand: hand_before-1+2.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 10);
        assert_eq!(
            cs.allies[0].player.as_ref().unwrap().hand.len(),
            hand_before + 1
        );
    }

    #[test]
    fn shrug_it_off_grants_eight_block_and_draws_one() {
        let mut cs = ironclad_combat();
        populate_draw_pile(&mut cs, 3);
        let card = card_by_id("ShrugItOff").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hand_before = cs.allies[0].player.as_ref().unwrap().hand.len();
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 8);
        // ShrugItOff itself consumed, then +1 drawn → net hand size
        // equals hand_before.
        assert_eq!(
            cs.allies[0].player.as_ref().unwrap().hand.len(),
            hand_before
        );
    }

    #[test]
    fn upgraded_shrug_it_off_grants_eleven_block_draws_one() {
        let mut cs = ironclad_combat();
        populate_draw_pile(&mut cs, 3);
        let card = card_by_id("ShrugItOff").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        // 8 + 3 = 11. Upgrade does NOT bump Cards.
        assert_eq!(cs.allies[0].block, 11);
    }

    #[test]
    fn shrug_it_off_with_empty_draw_pile_draws_zero() {
        let mut cs = ironclad_combat();
        let card = card_by_id("ShrugItOff").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        // Block lands; draw is a no-op when both piles are empty.
        assert_eq!(cs.allies[0].block, 8);
    }

    // ---------- Brimstone relic tests -----------------------------------

    fn ironclad_combat_with_relic(relic_id: &str) -> CombatState {
        let mut cs = ironclad_combat();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .relics
            .push(relic_id.to_string());
        cs
    }

    #[test]
    fn brimstone_grants_strength_to_self_and_enemies_on_player_turn() {
        let mut cs = ironclad_combat_with_relic("Brimstone");
        cs.begin_turn(CombatSide::Player);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            2
        );
        for i in 0..cs.enemies.len() {
            assert_eq!(
                cs.get_power_amount(CombatSide::Enemy, i, "StrengthPower"),
                1
            );
        }
    }

    #[test]
    fn brimstone_does_not_fire_on_enemy_turn() {
        let mut cs = ironclad_combat_with_relic("Brimstone");
        cs.begin_turn(CombatSide::Enemy);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            0
        );
        for i in 0..cs.enemies.len() {
            assert_eq!(
                cs.get_power_amount(CombatSide::Enemy, i, "StrengthPower"),
                0
            );
        }
    }

    #[test]
    fn brimstone_skips_dead_enemies() {
        let mut cs = ironclad_combat_with_relic("Brimstone");
        cs.enemies[0].current_hp = 0;
        cs.begin_turn(CombatSide::Player);
        // Dead enemy stays at 0 Strength (no PowerInstance added).
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            0
        );
        assert!(cs.enemies[0]
            .powers
            .iter()
            .all(|p| p.id != "StrengthPower"));
        // Alive enemy still picks up the +1.
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 1, "StrengthPower"),
            1
        );
    }

    #[test]
    fn brimstone_compounds_across_rounds() {
        let mut cs = ironclad_combat_with_relic("Brimstone");
        cs.begin_turn(CombatSide::Player);
        cs.begin_turn(CombatSide::Enemy);
        cs.begin_turn(CombatSide::Player);
        // +2 each player turn → 4 total after two.
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            4
        );
    }

    // ---------- DemonForm tests ------------------------------------------

    #[test]
    fn demon_form_applies_two_demon_form_power() {
        let mut cs = ironclad_combat();
        // Bump energy so we can afford a 3-cost card.
        cs.allies[0].player.as_mut().unwrap().energy = 3;
        let card = card_by_id("DemonForm").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "DemonFormPower"),
            2
        );
    }

    #[test]
    fn upgraded_demon_form_applies_three() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 3;
        let card = card_by_id("DemonForm").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "DemonFormPower"),
            3
        );
    }

    #[test]
    fn demon_form_grants_strength_on_player_turn_start() {
        // After playing DemonForm, the next begin_turn(Player) should
        // apply 2 Strength via tick_start_of_turn_powers.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "DemonFormPower", 2);
        // Initial Strength is 0.
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            0
        );
        cs.begin_turn(CombatSide::Player);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            2
        );
        // Second turn: another +2 → 4 total.
        cs.begin_turn(CombatSide::Enemy);
        cs.begin_turn(CombatSide::Player);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            4
        );
    }

    #[test]
    fn demon_form_does_not_trigger_on_enemy_turn() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "DemonFormPower", 2);
        cs.begin_turn(CombatSide::Enemy);
        // Player's DemonForm should not have fired (wrong side).
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "StrengthPower"),
            0
        );
    }

    // ---------- Breakthrough + BloodWall tests ---------------------------

    #[test]
    fn breakthrough_loses_hp_then_aoe_damages() {
        let mut cs = ironclad_combat();
        let hp_before = cs.allies[0].current_hp;
        let e0 = cs.enemies[0].current_hp;
        let e1 = cs.enemies[1].current_hp;
        let card = card_by_id("Breakthrough").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].current_hp, hp_before - 1);
        assert_eq!(cs.enemies[0].current_hp, e0 - 9);
        assert_eq!(cs.enemies[1].current_hp, e1 - 9);
    }

    #[test]
    fn upgraded_breakthrough_does_thirteen_per_enemy() {
        let mut cs = ironclad_combat();
        let e0 = cs.enemies[0].current_hp;
        let card = card_by_id("Breakthrough").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, e0 - 13);
    }

    #[test]
    fn breakthrough_self_damage_bypasses_block() {
        // HpLoss is Unblockable | Unpowered → block on player unchanged.
        let mut cs = ironclad_combat();
        cs.allies[0].block = 20;
        let hp_before = cs.allies[0].current_hp;
        let card = card_by_id("Breakthrough").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].current_hp, hp_before - 1);
        assert_eq!(cs.allies[0].block, 20);
    }

    #[test]
    fn blood_wall_loses_hp_then_grants_block() {
        let mut cs = ironclad_combat();
        let hp_before = cs.allies[0].current_hp;
        let card = card_by_id("BloodWall").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].current_hp, hp_before - 2);
        assert_eq!(cs.allies[0].block, 16);
    }

    #[test]
    fn upgraded_blood_wall_grants_twenty_block() {
        let mut cs = ironclad_combat();
        let card = card_by_id("BloodWall").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 20);
    }

    #[test]
    fn blood_wall_block_picks_up_frail() {
        // BloodWall threads through gain_block → modify_block, so Frail
        // reduces it: 16 * 0.75 = 12.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 1);
        let card = card_by_id("BloodWall").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 12);
    }

    // ---------- Tremble + Apparition tests -------------------------------

    #[test]
    fn tremble_applies_three_vulnerable_and_exhausts() {
        let mut cs = ironclad_combat();
        let card = card_by_id("Tremble").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            3
        );
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.exhaust.len(), 1);
        assert_eq!(ps.exhaust.cards[0].id, "Tremble");
    }

    #[test]
    fn upgraded_tremble_applies_four_vulnerable() {
        let mut cs = ironclad_combat();
        let card = card_by_id("Tremble").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            4
        );
    }

    #[test]
    fn apparition_grants_intangible_one_and_exhausts() {
        let mut cs = ironclad_combat();
        let card = card_by_id("Apparition").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "IntangiblePower"),
            1
        );
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.exhaust.len(), 1);
        assert_eq!(ps.exhaust.cards[0].id, "Apparition");
    }

    #[test]
    fn apparition_intangible_caps_incoming_damage_at_one() {
        // End-to-end: after Apparition, the existing IntangiblePower
        // damage cap kicks in via the damage pipeline.
        let mut cs = ironclad_combat();
        let card = card_by_id("Apparition").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        cs.play_card(0, hand_idx, None);
        let hp_before = cs.allies[0].current_hp;
        // Big incoming attack should be capped to 1.
        cs.deal_damage(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            50,
            ValueProp::MOVE,
        );
        assert_eq!(cs.allies[0].current_hp, hp_before - 1);
    }

    // ---------- MoltenFist / Exhaust routing tests -----------------------

    #[test]
    fn molten_fist_exhausts_after_play() {
        let mut cs = ironclad_combat();
        let card = card_by_id("MoltenFist").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        let ps = cs.allies[0].player.as_ref().unwrap();
        // Card is in exhaust, not discard.
        assert_eq!(ps.exhaust.len(), 1);
        assert_eq!(ps.exhaust.cards[0].id, "MoltenFist");
        assert!(ps.discard.is_empty());
    }

    #[test]
    fn molten_fist_no_vulnerable_just_damage() {
        let mut cs = ironclad_combat();
        let card = card_by_id("MoltenFist").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 10 damage. Vulnerable wasn't there → no re-apply.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 10);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            0
        );
    }

    #[test]
    fn molten_fist_reapplies_vulnerable_count() {
        // Target has 2 Vulnerable. Damage = 10 * 1.5 = 15. Then reapply
        // 2 stacks → final Vulnerable = 4.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 2);
        let card = card_by_id("MoltenFist").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 15);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            4
        );
    }

    #[test]
    fn molten_fist_no_reapply_if_target_killed() {
        // Set enemy HP to 1; MoltenFist kills it (would have been Vulnerable
        // after, but we skip the reapply on dead targets).
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 2);
        cs.enemies[0].current_hp = 1;
        let card = card_by_id("MoltenFist").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, 0);
        // Vulnerable stack stays at 2 (no reapply on dead target).
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            2
        );
    }

    #[test]
    fn upgraded_molten_fist_does_fourteen_damage() {
        let mut cs = ironclad_combat();
        let card = card_by_id("MoltenFist").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 14);
    }

    // ---------- Bloodletting tests ---------------------------------------

    #[test]
    fn bloodletting_loses_hp_and_gains_energy() {
        let mut cs = ironclad_combat();
        let energy_before = cs.allies[0].player.as_ref().unwrap().energy;
        let hp_before = cs.allies[0].current_hp;
        let card = card_by_id("Bloodletting").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].current_hp, hp_before - 3);
        assert_eq!(
            cs.allies[0].player.as_ref().unwrap().energy,
            energy_before + 2
        );
    }

    #[test]
    fn upgraded_bloodletting_gains_three_energy_same_hp_loss() {
        let mut cs = ironclad_combat();
        let energy_before = cs.allies[0].player.as_ref().unwrap().energy;
        let hp_before = cs.allies[0].current_hp;
        let card = card_by_id("Bloodletting").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        // Upgrade only bumps Energy by +1; HpLoss stays at 3.
        assert_eq!(cs.allies[0].current_hp, hp_before - 3);
        assert_eq!(
            cs.allies[0].player.as_ref().unwrap().energy,
            energy_before + 3
        );
    }

    #[test]
    fn bloodletting_bypasses_block() {
        // Unblockable: HP loss happens even with full block on the
        // caster. Block remains untouched.
        let mut cs = ironclad_combat();
        cs.allies[0].block = 20;
        let hp_before = cs.allies[0].current_hp;
        let card = card_by_id("Bloodletting").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].current_hp, hp_before - 3);
        assert_eq!(cs.allies[0].block, 20);
    }

    // ---------- Whirlwind / X-cost tests ---------------------------------

    #[test]
    fn whirlwind_consumes_all_energy_and_hits_each_enemy_x_times() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 3;
        let card = card_by_id("Whirlwind").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp0_before = cs.enemies[0].current_hp;
        let hp1_before = cs.enemies[1].current_hp;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        // X=3, damage 5 → 15 to each enemy.
        assert_eq!(cs.enemies[0].current_hp, hp0_before - 15);
        assert_eq!(cs.enemies[1].current_hp, hp1_before - 15);
        // All energy consumed.
        assert_eq!(cs.allies[0].player.as_ref().unwrap().energy, 0);
    }

    #[test]
    fn whirlwind_with_zero_energy_is_noop() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 0;
        let card = card_by_id("Whirlwind").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before);
    }

    #[test]
    fn upgraded_whirlwind_does_eight_per_hit() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let card = card_by_id("Whirlwind").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        // X=2, damage 5+3=8 → 16 per enemy.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 16);
    }

    #[test]
    fn whirlwind_picks_up_strength_per_hit() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "StrengthPower", 2);
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let card = card_by_id("Whirlwind").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, None);
        assert_eq!(r, PlayResult::Ok);
        // X=2, damage (5+2)=7 → 14 per enemy.
        assert_eq!(cs.enemies[0].current_hp, hp_before - 14);
    }

    // ---------- LegSweep dispatch tests ----------------------------------

    #[test]
    fn leg_sweep_grants_block_and_applies_weak() {
        let mut cs = ironclad_combat();
        let card = card_by_id("LegSweep").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 11);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "WeakPower"),
            2
        );
    }

    #[test]
    fn upgraded_leg_sweep_grants_more_block_and_weak() {
        let mut cs = ironclad_combat();
        let card = card_by_id("LegSweep").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 1));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        // 11 + 3 = 14 block; 2 + 1 = 3 Weak.
        assert_eq!(cs.allies[0].block, 14);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "WeakPower"),
            3
        );
    }

    #[test]
    fn leg_sweep_block_picks_up_frail_and_dexterity() {
        // End-to-end: Frail + Dex on the caster modify LegSweep's block
        // through the modify_block pipeline. (11+2)*0.75 = 9.75 → 9.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 1);
        cs.apply_power(CombatSide::Player, 0, "DexterityPower", 2);
        let card = card_by_id("LegSweep").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, 0));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.allies[0].block, 9);
    }

    #[test]
    fn neutralize_deals_damage_and_applies_weak() {
        // Inject Neutralize into Ironclad's hand directly (it's a Silent
        // card; the harness doesn't care which character runs the test
        // for OnPlay routing).
        let mut cs = ironclad_combat();
        let n = card_by_id("Neutralize").unwrap();
        cs.allies[0].player.as_mut().unwrap().hand.cards.push(
            CardInstance::from_card(n, 0),
        );
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = cs.enemies[0].current_hp;
        let r = cs.play_card(0, hand_idx, Some((CombatSide::Enemy, 0)));
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp_before - 3);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "WeakPower"),
            1
        );
    }

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

    // ---------- Relic combat hook tests ----------------------------------

    #[test]
    fn burning_blood_heals_six_on_victory() {
        let mut cs = ironclad_combat();
        // Take some damage first.
        cs.allies[0].current_hp = 60;
        // Kill enemies (no rewards / no auto-detection — caller invokes
        // hooks explicitly upon detecting victory).
        for e in cs.enemies.iter_mut() {
            e.current_hp = 0;
        }
        assert_eq!(cs.is_combat_over(), Some(CombatResult::Victory));
        cs.fire_after_combat_victory_hooks();
        // BurningBlood heals 6, saturating at max_hp.
        assert_eq!(cs.allies[0].current_hp, 66);
    }

    #[test]
    fn burning_blood_caps_at_max_hp() {
        let mut cs = ironclad_combat();
        // Already at full HP.
        for e in cs.enemies.iter_mut() {
            e.current_hp = 0;
        }
        cs.fire_after_combat_victory_hooks();
        assert_eq!(cs.allies[0].current_hp, 80);
    }

    #[test]
    fn burning_blood_skips_when_owner_dead() {
        // C# guards "if (!base.Owner.Creature.IsDead)". Mirror that.
        let mut cs = ironclad_combat();
        cs.allies[0].current_hp = 0;
        for e in cs.enemies.iter_mut() {
            e.current_hp = 0;
        }
        cs.fire_after_combat_victory_hooks();
        assert_eq!(cs.allies[0].current_hp, 0);
    }

    // ---------- End-of-combat rewards tests ------------------------------

    #[test]
    fn axebots_normal_gold_reward_is_in_monster_range() {
        // AxebotsNormal -> RoomType=Monster -> 10..=20 gold.
        let cs = ironclad_combat();
        let mut rng = Rng::new(7, 0);
        let r = cs.generate_rewards(&mut rng);
        assert!(
            r.gold >= 10 && r.gold <= 20,
            "Monster gold out of range: {}",
            r.gold
        );
    }

    #[test]
    fn ad_hoc_combat_with_no_encounter_drops_zero_gold() {
        let cs = CombatState::empty();
        let mut rng = Rng::new(7, 0);
        let r = cs.generate_rewards(&mut rng);
        assert_eq!(r.gold, 0);
    }

    #[test]
    fn generate_rewards_card_potion_relic_placeholders_empty() {
        // Until card-reward / potion / relic generation lands, those
        // fields are empty.
        let cs = ironclad_combat();
        let mut rng = Rng::new(7, 0);
        let r = cs.generate_rewards(&mut rng);
        assert!(r.card_choices.is_empty());
        assert!(r.potion.is_none());
        assert!(r.relic.is_none());
    }

    #[test]
    fn gold_range_for_each_room_type() {
        // Sanity-lock the table values against accidental table edits.
        assert_eq!(gold_reward_range(Some("Monster")), (10, 20));
        assert_eq!(gold_reward_range(Some("Elite")), (35, 45));
        assert_eq!(gold_reward_range(Some("Boss")), (100, 100));
        assert_eq!(gold_reward_range(Some("Shop")), (0, 0));
        assert_eq!(gold_reward_range(None), (0, 0));
    }

    #[test]
    fn gold_is_deterministic_for_a_given_seed() {
        let cs = ironclad_combat();
        let mut rng1 = Rng::new(42, 0);
        let mut rng2 = Rng::new(42, 0);
        let g1 = cs.generate_rewards(&mut rng1).gold;
        let g2 = cs.generate_rewards(&mut rng2).gold;
        assert_eq!(g1, g2);
    }

    // ---------- Combat-log tests -----------------------------------------

    #[test]
    fn log_off_by_default_no_events_recorded() {
        let mut cs = ironclad_combat();
        cs.gain_block(CombatSide::Player, 0, 5);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            ValueProp::MOVE,
        );
        assert!(cs.combat_log.is_empty());
    }

    #[test]
    fn log_captures_damage_block_power_events() {
        let mut cs = ironclad_combat();
        cs.set_log_enabled(true);
        cs.gain_block(CombatSide::Player, 0, 5);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            6,
            ValueProp::MOVE,
        );
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 1);

        let kinds: Vec<&str> = cs
            .combat_log
            .iter()
            .map(|e| match e {
                CombatEvent::BlockGained { .. } => "BlockGained",
                CombatEvent::DamageDealt { .. } => "DamageDealt",
                CombatEvent::PowerApplied { .. } => "PowerApplied",
                CombatEvent::TurnBegan { .. } => "TurnBegan",
                CombatEvent::TurnEnded { .. } => "TurnEnded",
                CombatEvent::RelicHookFired { .. } => "RelicHookFired",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["BlockGained", "DamageDealt", "PowerApplied"]
        );
    }

    #[test]
    fn log_captures_turn_and_hook_events() {
        let mut cs = ironclad_combat();
        cs.set_log_enabled(true);
        cs.fire_before_combat_start_hooks();
        cs.begin_turn(CombatSide::Player);
        cs.end_turn();
        cs.fire_after_combat_victory_hooks();

        let hook_count = cs
            .combat_log
            .iter()
            .filter(|e| matches!(e, CombatEvent::RelicHookFired { .. }))
            .count();
        // Ironclad has BurningBlood as the only starter relic; it has
        // both BeforeCombatStart (no-op for BurningBlood — falls through)
        // and AfterCombatVictory (heal). The dispatcher emits a log entry
        // per relic-per-hook regardless of whether the relic has a
        // registered handler, since the log captures dispatch attempts.
        assert!(hook_count >= 2);

        assert!(cs
            .combat_log
            .iter()
            .any(|e| matches!(e, CombatEvent::TurnBegan { .. })));
        assert!(cs
            .combat_log
            .iter()
            .any(|e| matches!(e, CombatEvent::TurnEnded { .. })));
    }

    #[test]
    fn anchor_grants_ten_block_at_combat_start() {
        let mut cs = ironclad_combat();
        // Replace starter relics with Anchor.
        cs.allies[0].player.as_mut().unwrap().relics =
            vec!["Anchor".to_string()];
        assert_eq!(cs.allies[0].block, 0);
        cs.fire_before_combat_start_hooks();
        assert_eq!(cs.allies[0].block, 10);
    }

    #[test]
    fn before_combat_start_skips_unhooked_relics() {
        let mut cs = ironclad_combat();
        // Only BurningBlood (no BeforeCombatStart) — block stays 0.
        assert_eq!(cs.allies[0].player.as_ref().unwrap().relics, vec!["BurningBlood"]);
        cs.fire_before_combat_start_hooks();
        assert_eq!(cs.allies[0].block, 0);
    }

    #[test]
    fn no_relics_means_no_hook_fires() {
        let mut cs = ironclad_combat();
        // Strip relics; fire — nothing happens.
        cs.allies[0].player.as_mut().unwrap().relics.clear();
        cs.allies[0].current_hp = 40;
        for e in cs.enemies.iter_mut() {
            e.current_hp = 0;
        }
        cs.fire_after_combat_victory_hooks();
        assert_eq!(cs.allies[0].current_hp, 40);
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
