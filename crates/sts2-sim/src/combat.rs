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

/// Cards drawn at the start of combat (and every turn-start in C#'s
/// `Hook.ModifyDraw` default). 5 for every character — relics that
/// modify this aren't ported yet.
pub const INITIAL_HAND_SIZE: i32 = 5;

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
    /// Gold accumulated mid-combat by effects like HandOfGreed /
    /// Alchemize / FoulPotion. Folded into `CombatRewards.gold` when
    /// the combat ends. Lives here (not in RunState) because combat is
    /// stateless w.r.t. the strategic layer.
    pub pending_gold: i32,
    /// Stars accumulated mid-combat (StS2 secondary resource — GatherLight,
    /// Watcher-family cards). System not yet wired into card play
    /// gating; tracked here so the data-driven effect path is
    /// future-compatible.
    pub pending_stars: i32,
    /// Defect orb queue. Front (index 0) is the oldest. Channeling
    /// when the queue is full evokes the front first. Mirrors C#
    /// `PlayerCombatState.OrbQueue`.
    pub orb_queue: Vec<OrbInstance>,
    /// Max queue capacity. Default 3 for Defect. ChangeOrbSlots
    /// primitive mutates this. Mirrors C# `PlayerCombatState.OrbCapacity`.
    pub orb_slots: i32,
    /// Pending Forge credits — increments when a card with
    /// `Effect::Forge` is played. Mirrors C# `ForgeCmd.Forge` queue.
    /// Card-upgrade resolution is deferred until a player-choice
    /// mechanism lands.
    pub pending_forge: i32,
    /// Optional Osty companion (Necrobinder). None until SummonOsty
    /// fires. Mirrors C# `PlayerCombatState.Osty: OstyModel?`. C# Osty
    /// extends MonsterModel; we model it as a thin per-summon struct
    /// (HP / Block) since the full creature-with-intent model isn't
    /// needed for the cards that reference Osty.
    pub osty: Option<OstyState>,
    /// Per-relic mutable scalar counters. Used by relics that need
    /// state-across-turns the canonical_vars table can't carry —
    /// e.g. HappyFlower / Pendulum (turns_seen modulo), Kunai / Shuriken /
    /// LetterOpener / IronClub / GamePiece (attacks- or cards-played
    /// count toward a threshold), Pocketwatch (cards-this-turn counter),
    /// CentennialPuzzle (one-shot flag). Keys are short ids the relic's
    /// hook bodies read via `Effect::SetPowerStateField`-style writes.
    pub relic_counters: std::collections::HashMap<String, i32>,
}

/// Companion-creature state. Cards reference Osty.MaxHp (Protector,
/// Sacrifice) and check Osty.IsAlive (DamageFromOsty gates).
#[derive(Clone, Debug)]
pub struct OstyState {
    pub current_hp: i32,
    pub max_hp: i32,
    pub block: i32,
}

/// One orb in the player's queue. Mirrors C# `OrbModel`.
#[derive(Clone, Debug)]
pub struct OrbInstance {
    /// "LightningOrb" / "FrostOrb" / "DarkOrb" / "PlasmaOrb" / "GlassOrb".
    pub id: String,
    /// Per-orb internal value override. DarkOrb uses this to track
    /// charge accumulated by its Passive (`_evokeVal += PassiveVal`).
    /// Other orbs ignore.
    pub evoke_val_bonus: i32,
}

#[derive(Clone, Debug)]
pub struct MonsterState {
    /// Currently-selected move id (matches a key in the monster's move state
    /// machine once that's ported). `None` until intent is resolved.
    pub intent_move: Option<String>,
    /// Computed intent values if known (attack damage × hit count, block,
    /// etc.). Empty until the intent pipeline runs.
    pub intent_values: Vec<IntentValue>,
    /// Per-monster boolean flags that drive state-machine branches but
    /// don't fit cleanly into the Power model. Keyed by short id.
    /// Current users:
    ///   - "is_off_balance": BowlbugRock — flipped on by
    ///     ImbalancedPower's AfterDamageGiven when this monster's
    ///     attack is fully blocked; cleared by its Dizzy move.
    pub flags: std::collections::HashMap<String, bool>,
    /// Per-monster integer counters tied to a specific Power instance
    /// when the Power model needs cross-turn state that the stack
    /// amount alone can't represent. Keyed by short id.
    /// Current users:
    ///   - "hardened_shell_taken": HardenedShellPower —
    ///     `damageReceivedThisTurn` per C#'s Data class. Reset to 0
    ///     at start of the Player's turn (BeforeSideTurnStart).
    pub counters: std::collections::HashMap<String, i32>,
}

impl MonsterState {
    pub fn new() -> Self {
        Self {
            intent_move: None,
            intent_values: Vec::new(),
            flags: std::collections::HashMap::new(),
            counters: std::collections::HashMap::new(),
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        self.flags.get(key).copied().unwrap_or(false)
    }

    pub fn set_flag(&mut self, key: &str, value: bool) {
        self.flags.insert(key.to_string(), value);
    }

    pub fn counter(&self, key: &str) -> i32 {
        self.counters.get(key).copied().unwrap_or(0)
    }

    pub fn set_counter(&mut self, key: &str, value: i32) {
        self.counters.insert(key.to_string(), value);
    }

    pub fn add_counter(&mut self, key: &str, delta: i32) {
        let v = self.counter(key) + delta;
        self.set_counter(key, v);
    }
}

impl Default for MonsterState {
    fn default() -> Self {
        Self::new()
    }
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
    /// Mirrors C# `PowerModel.SkipNextDurationTick`. Set true in
    /// `apply_power` when the target is the player and the power is a
    /// Debuff; the next `tick_duration_debuffs` reads-and-clears the
    /// flag, skipping the decrement. Prevents Weak/Frail/Vulnerable
    /// applied to the player during their own turn from losing a turn
    /// of duration before they get to feel the effect.
    ///
    /// Mirrors `PowerCmd.cs:131` (set) + `PowerCmd.cs:159` (check+clear).
    pub skip_next_duration_tick: bool,
    /// Per-instance scalar state. Used by `Effect::SetPowerStateField`
    /// for powers whose payload depends on a number set at apply time
    /// (TheBomb.Damage, ToricToughness.Block, Monologue's nested vars).
    /// Mirrors C# AbstractPowerWithCounter / per-power "Data" field. Most
    /// powers leave this empty.
    pub state: std::collections::HashMap<String, i32>,
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
    /// Cost override valid until end-of-turn. BulletTime
    /// (`SetToFreeThisTurn`), Enlightenment-non-upgraded
    /// (`SetThisTurnOrUntilPlayed`). Cleared at begin_turn(Player).
    pub cost_override_this_turn: Option<i32>,
    /// Cost override valid until combat end. Enlightenment-upgraded
    /// (`SetThisCombat`), Modded.
    pub cost_override_this_combat: Option<i32>,
    /// Cost override valid until next play of this card.
    /// Enlightenment-non-upgraded's "OrUntilPlayed" half.
    pub cost_override_until_played: Option<i32>,
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
            cost_override_this_turn: None,
            cost_override_this_combat: None,
            cost_override_until_played: None,
        }
    }

    /// Effective energy cost at play time, honoring active overrides.
    /// Priority (highest first): `cost_override_until_played` >
    /// `cost_override_this_turn` > `cost_override_this_combat` >
    /// `current_energy_cost`. Mirrors C# `CardModel.EnergyCost` with
    /// the SetThisTurn / SetThisCombat / SetThisTurnOrUntilPlayed
    /// override priority chain.
    pub fn effective_energy_cost(&self) -> i32 {
        if let Some(c) = self.cost_override_until_played {
            return c.max(0);
        }
        if let Some(c) = self.cost_override_this_turn {
            return c.max(0);
        }
        if let Some(c) = self.cost_override_this_combat {
            return c.max(0);
        }
        self.current_energy_cost.max(0)
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
        // TurnBegan emits unconditionally — history-scan AmountSpecs
        // (CardsPlayedThisTurn etc.) need it to find the start of the
        // current turn even when verbose logging is disabled.
        {
            let round = self.round_number;
            self.combat_log
                .push(CombatEvent::TurnBegan { round, side });
        }
        // BeforeSideTurnStart relic hooks: fire before block-clear /
        // energy-refresh / power ticks. Data-driven only (no legacy
        // match-arm dispatcher at this phase).
        self.fire_before_side_turn_start_hooks(side);
        // Block survives one creature's *own* turn end → wipe at the start
        // of that side's next turn. This matches StS rules: block from
        // Defend persists through enemy attacks, then resets when you play
        // again. We clear on this side's begin, not on end.
        //
        // ShouldClearBlock=false exceptions: BarricadePower and
        // BurrowedPower both return false on owner — block persists
        // across the owner's turn boundary. We skip the clear for any
        // creature that holds either power.
        const BLOCK_PRESERVE_POWERS: &[&str] =
            &["BarricadePower", "BurrowedPower"];
        let preserves = |creature: &Creature| -> bool {
            creature
                .powers
                .iter()
                .any(|p| BLOCK_PRESERVE_POWERS.contains(&p.id.as_str()))
        };
        // Track whether any owner-side creature had non-zero block at
        // the boundary — fires AfterBlockCleared if so (mirrors C# block
        // clear → fire AfterBlockCleared hook).
        let mut block_cleared_player = false;
        let mut block_cleared_enemy = false;
        match side {
            CombatSide::Player => {
                for ally in self.allies.iter_mut() {
                    if !preserves(ally) {
                        if ally.block > 0 {
                            block_cleared_player = true;
                        }
                        ally.block = 0;
                    }
                    if let Some(ps) = ally.player.as_mut() {
                        ps.energy = ps.turn_energy;
                        // Clear per-turn cost overrides (BulletTime /
                        // Enlightenment-non-upgraded ThisTurnOrUntilPlayed).
                        // Mirrors C# CardModel.EnergyCost.ClearThisTurn.
                        for pile in [
                            &mut ps.hand,
                            &mut ps.draw,
                            &mut ps.discard,
                            &mut ps.exhaust,
                        ] {
                            for c in pile.cards.iter_mut() {
                                c.cost_override_this_turn = None;
                            }
                        }
                    }
                }
            }
            CombatSide::Enemy => {
                for enemy in self.enemies.iter_mut() {
                    if !preserves(enemy) {
                        if enemy.block > 0 {
                            block_cleared_enemy = true;
                        }
                        enemy.block = 0;
                    }
                }
            }
            CombatSide::None => {}
        }
        // AfterBlockCleared relic hooks — owner-side firing only (player
        // relics fire on player-side block clear; we don't yet model
        // enemy-owned block-cleared relic equivalents). Captains Wheel
        // is a self-block hook so this matches.
        if block_cleared_player {
            crate::effects::fire_relic_hooks(
                self,
                crate::effects::RelicHookKind::AfterBlockCleared,
                CombatSide::Player,
            );
        }
        let _ = block_cleared_enemy;
        // AfterSideTurnStart hook pass.
        // Hook firing order proper will land in #70; for now powers
        // (Poison / DemonForm) fire first, then relic AfterSideTurnStart
        // hooks (Brimstone). This matches the casual reading of the
        // C# dispatch but isn't formally validated against shipping
        // ordering — adjust when #70 lands.
        self.tick_start_of_turn_powers(side);
        self.fire_after_side_turn_start_hooks(side);
        // Power VM AfterSideTurnStart dispatch — iterates living
        // creature powers and runs any registered AfterSideTurnStart
        // hook bodies. Currently no powers registered at this phase
        // (existing tick paths handle Poison/DemonForm/Ritual via
        // match arms). Future migrations move those into power_effects.
        let started_side = side;
        crate::effects::fire_power_hooks_after_side_turn_start(self, started_side);

        // VitalSparkPower.BeforeSideTurnStart (Enemy side): clear
        // vital_spark_used so the next Player turn re-arms the +1
        // energy grant. Mirrors C# `playersTriggeredThisTurn.Clear()`.
        if side == CombatSide::Enemy {
            for enemy in self.enemies.iter_mut() {
                let has_vs = enemy
                    .powers
                    .iter()
                    .any(|p| p.id == "VitalSparkPower");
                if has_vs {
                    if let Some(ms) = enemy.monster.as_mut() {
                        ms.set_flag("vital_spark_used", false);
                    }
                }
            }
        }
        // VigorPower snapshot/drain moved to fire_before_attack /
        // fire_after_attack (audit fix #178) — matches C# AttackCommand
        // envelope. Was previously begun_turn → tick_vigor_drain.
        // PlatingPower.BeforeSideTurnStart on Player turn start, round 1:
        // each enemy-owned Plating grants Amount unpowered block to its
        // owner. Fires only once per combat — gated on round_number.
        // C# checks `RoundNumber != 1`; we mirror that. The
        // round-end-of-owner-turn block grant lives in `end_turn` so
        // this hook only handles the round-1 prefab.
        if side == CombatSide::Player && self.round_number == 1 {
            let n = self.enemies.len();
            for i in 0..n {
                let plating = self
                    .enemies
                    .get(i)
                    .and_then(|c| {
                        c.powers
                            .iter()
                            .find(|p| p.id == "PlatingPower")
                            .map(|p| p.amount)
                    })
                    .unwrap_or(0);
                if plating > 0 {
                    self.gain_block_with_props(
                        CombatSide::Enemy,
                        i,
                        plating,
                        ValueProp::UNPOWERED,
                    );
                }
            }
        }
        // RampartPower fires on Player-side start regardless of owner.
        // Walks enemies and grants block to their TurretOperator allies.
        if side == CombatSide::Player {
            self.tick_rampart_powers();
            // HardenedShellPower.BeforeSideTurnStart (Player only):
            // reset `damageReceivedThisTurn` to 0 for any monster
            // holding HardenedShell. The counter tracks across the
            // enemy turn — we zero it on the next Player turn.
            for enemy in self.enemies.iter_mut() {
                let has_shell = enemy
                    .powers
                    .iter()
                    .any(|p| p.id == "HardenedShellPower");
                if has_shell {
                    if let Some(ms) = enemy.monster.as_mut() {
                        ms.set_counter("hardened_shell_taken", 0);
                    }
                }
            }
        }
    }

    /// RampartPower.AfterSideTurnStart hook (fires on Player-side
    /// start): every enemy with RampartPower grants `Amount` unpowered
    /// block to every alive enemy teammate whose model_id is
    /// `TurretOperator`. C# filters by `c.Monster is TurretOperator`.
    fn tick_rampart_powers(&mut self) {
        // Snapshot ramparts (owner_idx, amount) and beneficiary indices
        // up-front so the gain_block calls don't disrupt iteration.
        let mut grants: Vec<(usize, i32)> = Vec::new();
        for owner_idx in 0..self.enemies.len() {
            let owner = &self.enemies[owner_idx];
            if owner.current_hp == 0 {
                continue;
            }
            let Some(rampart) = owner.powers.iter().find(|p| p.id == "RampartPower")
            else {
                continue;
            };
            if rampart.amount <= 0 {
                continue;
            }
            let amount = rampart.amount;
            for (idx, ally) in self.enemies.iter().enumerate() {
                if idx == owner_idx {
                    continue;
                }
                if ally.current_hp == 0 {
                    continue;
                }
                if ally.model_id == "TurretOperator" {
                    grants.push((idx, amount));
                }
            }
        }
        for (idx, amount) in grants {
            // C# uses ValueProp.Unpowered → bypasses block modifiers.
            self.gain_block_with_props(
                CombatSide::Enemy,
                idx,
                amount,
                ValueProp::UNPOWERED,
            );
        }
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
        crate::effects::fire_relic_hooks(
            self,
            crate::effects::RelicHookKind::AfterSideTurnStart,
            side,
        );
        // AfterPlayerTurnStart fires on the Player side of begin_turn.
        // Distinct from AfterSideTurnStart only in C# ordering; we
        // co-locate them here.
        if side == CombatSide::Player {
            crate::effects::fire_relic_hooks(
                self,
                crate::effects::RelicHookKind::AfterPlayerTurnStart,
                side,
            );
        }
    }

    /// Fire each player's relic `BeforeSideTurnStart` hooks. New firing
    /// point — invoked at the top of `begin_turn`, before block-clear /
    /// energy-refresh / power ticks. C# BagOfMarbles / RedMask /
    /// TwistedFunnel / CrackedCore land here.
    pub fn fire_before_side_turn_start_hooks(&mut self, side: CombatSide) {
        crate::effects::fire_relic_hooks(
            self,
            crate::effects::RelicHookKind::BeforeSideTurnStart,
            side,
        );
    }

    /// Fire each player's relic `AfterPlayerTurnEnd` hooks. New firing
    /// point — invoked at the end of player-side `end_turn`.
    pub fn fire_after_player_turn_end_hooks(&mut self) {
        crate::effects::fire_relic_hooks(
            self,
            crate::effects::RelicHookKind::AfterPlayerTurnEnd,
            CombatSide::Player,
        );
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
        // Data-driven relic-effect VM dispatch — runs alongside the
        // legacy per-relic match arms above. Relics encoded in
        // `relic_effects` fire here; relics in dispatch_relic_*
        // (Anchor/BurningBlood/Brimstone) keep their hand-coded path.
        crate::effects::fire_relic_hooks(
            self,
            crate::effects::RelicHookKind::BeforeCombatStart,
            CombatSide::Player,
        );
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
        crate::effects::fire_relic_hooks(
            self,
            crate::effects::RelicHookKind::AfterCombatVictory,
            CombatSide::Player,
        );
        // AfterCombatEnd also fires on the victory path. (Defeat path
        // fires it from `fire_after_combat_loss_hooks` if/when it lands.)
        crate::effects::fire_relic_hooks(
            self,
            crate::effects::RelicHookKind::AfterCombatEnd,
            CombatSide::Player,
        );
    }

    /// Fire `AfterCombatLoss` + `AfterCombatEnd` data-driven relic hooks.
    /// Caller invokes when `is_combat_over()` returns Defeat. (Most
    /// relics that branch on outcome use AfterCombatVictory or
    /// AfterCombatEnd; AfterCombatLoss is rare.)
    pub fn fire_after_combat_loss_hooks(&mut self) {
        crate::effects::fire_relic_hooks(
            self,
            crate::effects::RelicHookKind::AfterCombatLoss,
            CombatSide::Player,
        );
        crate::effects::fire_relic_hooks(
            self,
            crate::effects::RelicHookKind::AfterCombatEnd,
            CombatSide::Player,
        );
    }

    /// Snapshot (player_idx, relic_id) pairs so hook dispatchers can mutate
    /// Use a potion: look up its OnUse effect list and run it through the
    /// VM. Returns true if the potion id was found and dispatched, false
    /// if unknown (caller decides whether to charge the slot or no-op).
    /// `target` is honored for AnyEnemy potions (FirePotion etc.).
    pub fn use_potion(
        &mut self,
        player_idx: usize,
        potion_id: &str,
        target: Option<(CombatSide, usize)>,
    ) -> bool {
        let Some(effects) = crate::effects::potion_effects(potion_id) else {
            return false;
        };
        let ctx =
            crate::effects::EffectContext::for_potion_use(player_idx, target, potion_id);
        crate::effects::execute_effects(self, &effects, &ctx);
        true
    }

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
    /// creature's side begins its turn.
    ///
    /// PoisonPower / DemonFormPower migrated to the Power VM —
    /// see `power_effects` in effects.rs. This shell remains for any
    /// future hardcoded start-of-turn power that the data-driven layer
    /// doesn't yet support (currently none).
    pub fn tick_start_of_turn_powers(&mut self, _side: CombatSide) {
        // No-op: Poison + DemonForm now dispatch through
        // crate::effects::fire_power_hooks_after_side_turn_start
        // which begin_turn already calls.
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
        // BeforeTurnEnd relic hooks — fire BEFORE the end-of-turn flush
        // / AfterTurnEnd power ticks. Bookmark / Orichalcum / DiamondDiadem.
        {
            let ending_side = self.current_side;
            crate::effects::fire_relic_hooks(
                self,
                crate::effects::RelicHookKind::BeforeTurnEnd,
                ending_side,
            );
        }
        if self.current_side == CombatSide::Player {
            // Pre-flush: fire OnTurnEndInHand for any status/curse cards
            // still in hand (Burn / Decay / Toxic / Doubt / Shame /
            // BadLuck / Debt / Regret / Infection / Beckon). Mirrors C#
            // CardModel.OnTurnEndInHand iteration in PlayerCombatState
            // before the keyword-routing flush.
            self.fire_turn_end_in_hand_effects();
            // Collect per-player routing decisions so the
            // history-event push + relic-hook fire can happen after
            // the &mut self.allies borrow drops.
            let mut all_exhausted: Vec<(usize, String)> = Vec::new();
            let mut all_discarded: Vec<(usize, String)> = Vec::new();
            for (player_idx, ally) in self.allies.iter_mut().enumerate() {
                let Some(ps) = ally.player.as_mut() else {
                    continue;
                };
                // Per-card routing at end of player turn (C# Flush
                // sequence). Ethereal → auto-exhaust; Retain → keep;
                // otherwise → discard.
                let mut keep_in_hand: Vec<CardInstance> = Vec::new();
                let drained: Vec<CardInstance> =
                    std::mem::take(&mut ps.hand.cards);
                for card in drained {
                    let data = card_by_id(&card.id);
                    let keywords: &[String] = data.map(|d| d.keywords.as_slice()).unwrap_or(&[]);
                    let is_ethereal = keywords.iter().any(|k| k == "Ethereal");
                    let is_retain = keywords.iter().any(|k| k == "Retain");
                    if is_ethereal {
                        all_exhausted.push((player_idx, card.id.clone()));
                        ps.exhaust.cards.push(card);
                    } else if is_retain {
                        keep_in_hand.push(card);
                    } else {
                        all_discarded.push((player_idx, card.id.clone()));
                        ps.discard.cards.push(card);
                    }
                }
                ps.hand.cards = keep_in_hand;
            }
            // History emission + AfterCardExhausted/Discarded relic
            // hooks. C# fires these per-card; we emit the events per-
            // card and fire the hooks once at the end (idempotent — the
            // relic body itself is what reads the per-card history).
            let round = self.round_number;
            for (pid, cid) in &all_exhausted {
                self.combat_log.push(CombatEvent::CardExhausted {
                    round,
                    player_idx: *pid,
                    card_id: cid.clone(),
                });
            }
            for (pid, cid) in &all_discarded {
                self.combat_log.push(CombatEvent::CardDiscarded {
                    round,
                    player_idx: *pid,
                    card_id: cid.clone(),
                });
            }
            if !all_exhausted.is_empty() {
                crate::effects::fire_relic_hooks(
                    self,
                    crate::effects::RelicHookKind::AfterCardExhausted,
                    CombatSide::Player,
                );
            }
            if !all_discarded.is_empty() {
                crate::effects::fire_relic_hooks(
                    self,
                    crate::effects::RelicHookKind::AfterCardDiscarded,
                    CombatSide::Player,
                );
            }
        }
        if self.current_side == CombatSide::Enemy {
            self.tick_duration_debuffs();
        }
        // SkittishPower.AfterTurnEnd (resets on the side OTHER than
        // owner's — i.e., end of Player turn for enemy-owned Skittish):
        // clear skittish_used so the next Player turn can trigger the
        // block grant once again.
        if self.current_side == CombatSide::Player {
            for enemy in self.enemies.iter_mut() {
                let has_skittish =
                    enemy.powers.iter().any(|p| p.id == "SkittishPower");
                if has_skittish {
                    if let Some(ms) = enemy.monster.as_mut() {
                        ms.set_flag("skittish_used", false);
                    }
                }
            }
        }
        // DoomPower.BeforeTurnEnd: if any creature on the side just
        // ending has DoomPower and CurrentHp <= Amount, kill them.
        // Fires before the temp-strength cleanup since the C# uses
        // BeforeTurnEnd (which runs before AfterTurnEnd hooks).
        let side_just_ending = self.current_side;
        self.tick_doom_powers(side_just_ending);
        // TemporaryStrengthPower (SetupStrikePower extends this) removes
        // its stack at end of owner's turn and subtracts the same amount
        // of StrengthPower. Mirrors C#:
        //   AfterTurnEnd(side): if side == Owner.Side, Remove(this) +
        //   Apply<StrengthPower>(owner, -Sign*Amount).
        // SetupStrikePower has Sign=+1 (IsPositive); negative variants
        // (TemporaryStrengthDown) flip the sign — none ported yet.
        let side = self.current_side;
        self.tick_temporary_strength_powers(side);
        // TerritorialPower.AfterTurnEnd: when owner's side ends, apply
        // StrengthPower(Amount) to owner — permanent ramp. Only known
        // user is Byrdonis on spawn (TerritorialPower(1)).
        self.tick_territorial_powers(side);
        // EscapeArtistPower.AfterTurnEnd: decrement on owner side (held
        // at 1 — the C# "now pulsing" warning state). ThievingHopper
        // spawns with EscapeArtistPower(5) — purely a timing signal in
        // C#; gameplay-side this just counts down to align with the
        // intent state machine's Escape step.
        self.tick_escape_artist_powers(side);
        // PlatingPower: at end of owner's turn, gain Amount unpowered
        // block; at end of enemy turn, decrement Amount by 1 (C#
        // BeforeTurnEndEarly + AfterTurnEnd). Order: block grant
        // first (BeforeTurnEndEarly runs earlier), then decrement.
        // Only enemy-owned Plating is in scope for the corpus today.
        if self.current_side == CombatSide::Enemy {
            self.tick_plating_powers();
        }
        // SlumberPower: at end of owner's (enemy) turn, decrement;
        // remove at 0 so the next intent pick reads "no Slumber"
        // and routes the beetle to Rollout. C# stuns + forces
        // ROLL_OUT_MOVE; we just remove the power.
        if self.current_side == CombatSide::Enemy {
            self.tick_slumber_powers();
        }
        // AsleepPower: same shape as Slumber — decrement at owner
        // turn end; at 0 remove Plating too and wake (remove
        // Asleep). LagavulinMatriarch uses this to sleep for 3
        // owner-turns then unconditionally awaken.
        if self.current_side == CombatSide::Enemy {
            self.tick_asleep_powers();
        }
        // VigorPower drain moved to fire_after_attack (audit fix #178)
        // — matches C# AttackCommand envelope, not turn boundary.
        //
        // Power VM AfterTurnEnd dispatch — iterates living creatures'
        // powers and runs any registered effect-list bodies. RegenPower
        // is the first migration; future powers (Poison-on-self,
        // duration debuffs, etc.) will fold in here as they port.
        let ended_side = side;
        crate::effects::fire_power_hooks_after_turn_end(self, ended_side);
        // AfterPlayerTurnEnd relic hooks — fire at end of player-side
        // turn only. Data-driven via relic_effects table.
        if ended_side == CombatSide::Player {
            self.fire_after_player_turn_end_hooks();
        }
        if self.log_enabled {
            let round = self.round_number;
            let side = self.current_side;
            self.combat_log.push(CombatEvent::TurnEnded { round, side });
        }
    }

    /// DoomPower.BeforeTurnEnd: any creature on `side` with DoomPower
    /// dies if CurrentHp <= DoomPower.Amount. Mirrors C# DoomPower's
    /// IsOwnerDoomed check + DoomKill effect; simplified to direct
    /// HP zero-out since we don't model Hook.AfterDiedToDoom or the
    /// special-monster ShouldDie filter yet.
    /// AfterTurnEnd-on-owner-side Strength-ramp powers. On the side
    /// that just ended, each known power applies its `Amount` of
    /// Strength to its owner. Permanent — does not undo.
    ///
    /// Known users:
    ///   - TerritorialPower (Byrdonis spawn): pure ramp.
    ///   - RitualPower (CalcifiedCultist spawn): same shape. C# also
    ///     has a per-instance "skip-first-turn-if-enemy-applied" flag
    ///     (WasJustAppliedByEnemy) — not modeled here, so our Ritual
    ///     fires every turn including the first. Net effect: +1
    ///     Strength tick compared to C# in the worst case. Acceptable
    ///     simplification; reopen if combat-replay diffs surface it.
    fn tick_territorial_powers(&mut self, side: CombatSide) {
        const STRENGTH_RAMP_POWERS: &[&str] =
            &["TerritorialPower", "RitualPower"];
        let list = match side {
            CombatSide::Player => &self.allies,
            CombatSide::Enemy => &self.enemies,
            CombatSide::None => return,
        };
        let mut grants: Vec<(usize, i32)> = Vec::new();
        for (idx, creature) in list.iter().enumerate() {
            if creature.current_hp == 0 {
                continue;
            }
            for power in &creature.powers {
                if STRENGTH_RAMP_POWERS.contains(&power.id.as_str())
                    && power.amount > 0
                {
                    grants.push((idx, power.amount));
                }
            }
        }
        for (idx, amount) in grants {
            self.apply_power(side, idx, "StrengthPower", amount);
        }
    }

    /// EscapeArtistPower.AfterTurnEnd: on owner side, decrement only
    /// while Amount > 1 — so it holds at 1 forever once it lands there.
    /// In C# the "pulse at 1" is a UI cue indicating the monster is
    /// about to escape; gameplay-side it's a no-op past that point,
    /// but the decrement-to-1 timing must still fire so the value the
    /// observation exposes lines up with the real game.
    fn tick_escape_artist_powers(&mut self, side: CombatSide) {
        let list = match side {
            CombatSide::Player => &self.allies,
            CombatSide::Enemy => &self.enemies,
            CombatSide::None => return,
        };
        let mut to_dec: Vec<usize> = Vec::new();
        for (idx, creature) in list.iter().enumerate() {
            if creature.current_hp == 0 {
                continue;
            }
            if let Some(p) = creature
                .powers
                .iter()
                .find(|p| p.id == "EscapeArtistPower")
            {
                if p.amount > 1 {
                    to_dec.push(idx);
                }
            }
        }
        for idx in to_dec {
            self.decrement_power(side, idx, "EscapeArtistPower", 1);
        }
    }

    /// AsleepPower end-of-enemy-turn tick: decrement each enemy's
    /// Asleep by 1; at 0, also remove Plating (Lagavulin spawns with
    /// both, and the C# behavior strips Plating when Asleep clears).
    fn tick_asleep_powers(&mut self) {
        let n = self.enemies.len();
        let mut targets: Vec<usize> = Vec::new();
        for i in 0..n {
            if self.enemies[i].current_hp <= 0 {
                continue;
            }
            let asleep = self.enemies[i]
                .powers
                .iter()
                .find(|p| p.id == "AsleepPower")
                .map(|p| p.amount)
                .unwrap_or(0);
            if asleep > 0 {
                targets.push(i);
            }
        }
        for i in targets {
            self.decrement_power(CombatSide::Enemy, i, "AsleepPower", 1);
            if self.get_power_amount(CombatSide::Enemy, i, "AsleepPower")
                <= 0
            {
                self.remove_power(CombatSide::Enemy, i, "AsleepPower");
                self.remove_power(CombatSide::Enemy, i, "PlatingPower");
            }
        }
    }

    /// SlumberPower end-of-enemy-turn tick: decrement each enemy's
    /// Slumber by 1; remove the power at 0. C# stun-wakes the owner
    /// (forces ROLL_OUT_MOVE) — we skip the stun and let the next
    /// intent pick see no Slumber and route accordingly.
    fn tick_slumber_powers(&mut self) {
        let n = self.enemies.len();
        let mut targets: Vec<usize> = Vec::new();
        for i in 0..n {
            if self.enemies[i].current_hp <= 0 {
                continue;
            }
            let slumber = self.enemies[i]
                .powers
                .iter()
                .find(|p| p.id == "SlumberPower")
                .map(|p| p.amount)
                .unwrap_or(0);
            if slumber > 0 {
                targets.push(i);
            }
        }
        for i in targets {
            self.decrement_power(CombatSide::Enemy, i, "SlumberPower", 1);
            if self.get_power_amount(CombatSide::Enemy, i, "SlumberPower")
                <= 0
            {
                self.remove_power(CombatSide::Enemy, i, "SlumberPower");
            }
        }
    }

    /// PlatingPower end-of-enemy-turn tick: every enemy-owned Plating
    /// grants Amount unpowered block (BeforeTurnEndEarly) then
    /// decrements Amount by 1 (AfterTurnEnd). Stops contributing
    /// block when Amount hits 0.
    fn tick_plating_powers(&mut self) {
        let n = self.enemies.len();
        let mut grants: Vec<(usize, i32)> = Vec::new();
        for i in 0..n {
            if self.enemies[i].current_hp <= 0 {
                continue;
            }
            let plating = self.enemies[i]
                .powers
                .iter()
                .find(|p| p.id == "PlatingPower")
                .map(|p| p.amount)
                .unwrap_or(0);
            if plating > 0 {
                grants.push((i, plating));
            }
        }
        for (i, amount) in grants {
            self.gain_block_with_props(
                CombatSide::Enemy,
                i,
                amount,
                ValueProp::UNPOWERED,
            );
            self.decrement_power(CombatSide::Enemy, i, "PlatingPower", 1);
        }
    }


    fn tick_doom_powers(&mut self, side: CombatSide) {
        let list = match side {
            CombatSide::Player => &self.allies,
            CombatSide::Enemy => &self.enemies,
            CombatSide::None => return,
        };
        let mut doomed: Vec<usize> = Vec::new();
        for (idx, creature) in list.iter().enumerate() {
            if creature.current_hp == 0 {
                continue;
            }
            if let Some(p) = creature.powers.iter().find(|p| p.id == "DoomPower") {
                if creature.current_hp <= p.amount {
                    doomed.push(idx);
                }
            }
        }
        for idx in doomed {
            // lose_hp clamps to 0; pass a value large enough to floor.
            let cur = match side {
                CombatSide::Player => self.allies[idx].current_hp,
                CombatSide::Enemy => self.enemies[idx].current_hp,
                CombatSide::None => continue,
            };
            self.lose_hp(side, idx, cur);
        }
    }

    /// Fire `AfterTurnEnd` for `TemporaryStrengthPower` and
    /// `TemporaryDexterityPower` subclasses on the side whose turn
    /// just ended. Each entry: (temp-power id, sign, target-power id).
    /// On match, remove the temp-power stack and apply -sign*amount
    /// of the target-power (undoing the BeforeApplied silent grant).
    /// sign=+1 for IsPositive subclasses (SetupStrikePower,
    /// AnticipatePower); sign=-1 for IsPositive=false (ManglePower).
    fn tick_temporary_strength_powers(&mut self, side: CombatSide) {
        const TEMP_POWERS: &[(&str, i32, &str)] = &[
            ("SetupStrikePower", 1, "StrengthPower"),
            ("ManglePower", -1, "StrengthPower"),
            ("AnticipatePower", 1, "DexterityPower"),
        ];
        let n_allies = self.allies.len();
        let n_enemies = self.enemies.len();
        let mut undo: Vec<(CombatSide, usize, &'static str, i32, i32, &'static str)> =
            Vec::new();
        for i in 0..n_allies {
            if side != CombatSide::Player {
                continue;
            }
            for (id, sign, target) in TEMP_POWERS {
                let amount = self.get_power_amount(CombatSide::Player, i, id);
                if amount != 0 {
                    undo.push((CombatSide::Player, i, id, *sign, amount, target));
                }
            }
        }
        for i in 0..n_enemies {
            if side != CombatSide::Enemy {
                continue;
            }
            for (id, sign, target) in TEMP_POWERS {
                let amount = self.get_power_amount(CombatSide::Enemy, i, id);
                if amount != 0 {
                    undo.push((CombatSide::Enemy, i, id, *sign, amount, target));
                }
            }
        }
        for (s, idx, id, sign, amount, target) in undo {
            // Remove the temp-power stack entirely.
            self.decrement_power(s, idx, id, amount);
            // Subtract sign * amount of the target power (undoing the
            // BeforeApplied silent grant).
            self.apply_power(s, idx, target, -(sign * amount));
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
                if self.consume_skip_tick(CombatSide::Player, i, power_id) {
                    continue;
                }
                if self.get_power_amount(CombatSide::Player, i, power_id) > 0 {
                    self.decrement_power(CombatSide::Player, i, power_id, 1);
                }
            }
        }
        for i in 0..n_enemies {
            for power_id in TICKING {
                if self.consume_skip_tick(CombatSide::Enemy, i, power_id) {
                    continue;
                }
                if self.get_power_amount(CombatSide::Enemy, i, power_id) > 0 {
                    self.decrement_power(CombatSide::Enemy, i, power_id, 1);
                }
            }
        }
    }

    /// Mirror C# `PowerCmd.TickDownDuration` (line 157): if the
    /// `SkipNextDurationTick` flag is set, clear it and return true
    /// (caller skips the decrement). Otherwise return false.
    fn consume_skip_tick(
        &mut self,
        side: CombatSide,
        target_idx: usize,
        power_id: &str,
    ) -> bool {
        let Some(target) = creature_mut(self, side, target_idx) else {
            return false;
        };
        let Some(power) = target.powers.iter_mut().find(|p| p.id == power_id)
        else {
            return false;
        };
        if power.skip_next_duration_tick {
            power.skip_next_duration_tick = false;
            true
        } else {
            false
        }
    }

    /// Fire `OnTurnEndInHand` for every card in every player's hand
    /// at the end of the Player turn. Mirrors C# CardModel.OnTurnEndInHand
    /// iteration. Cards covered (status / curses):
    ///   - Burn: damage 2 to owner (Unpowered | Move; goes through
    ///     block).
    ///   - Decay / Toxic / Infection: damage Damage-canonical to owner.
    ///   - BadLuck / Beckon: damage HpLoss (Unblockable | Unpowered).
    ///   - Doubt: Weak(1) to owner with SkipNextDurationTick if newly
    ///     applied (handled by apply_power's player-debuff flag).
    ///   - Shame: Frail(1) to owner, same.
    ///   - Debt: lose gold from pending_gold (clamped).
    ///   - Regret: damage = hand size (Unblockable | Unpowered).
    fn fire_turn_end_in_hand_effects(&mut self) {
        let n_allies = self.allies.len();
        for player_idx in 0..n_allies {
            let cards: Vec<(String, i32)> = self
                .allies
                .get(player_idx)
                .and_then(|c| c.player.as_ref())
                .map(|ps| {
                    ps.hand
                        .cards
                        .iter()
                        .map(|c| (c.id.clone(), c.upgrade_level))
                        .collect()
                })
                .unwrap_or_default();
            let hand_size = cards.len() as i32;
            for (cid, upgrade) in &cards {
                let card = card_by_id(cid);
                let Some(card) = card else { continue; };
                match cid.as_str() {
                    "Burn" => {
                        let dmg = canonical_int_value(card, "Damage", *upgrade);
                        // Burn uses normal damage pipeline (Unpowered | Move).
                        // Goes through block; ignores Strength.
                        self.deal_damage(
                            (CombatSide::Player, player_idx),
                            (CombatSide::Player, player_idx),
                            dmg,
                            ValueProp::UNPOWERED,
                        );
                    }
                    "Decay" | "Toxic" | "Infection" => {
                        let dmg = canonical_int_value(card, "Damage", *upgrade);
                        self.deal_damage(
                            (CombatSide::Player, player_idx),
                            (CombatSide::Player, player_idx),
                            dmg,
                            ValueProp::UNPOWERED,
                        );
                    }
                    "BadLuck" | "Beckon" => {
                        let hp = canonical_int_value(card, "HpLoss", *upgrade);
                        // Unblockable | Unpowered → use lose_hp (bypasses block).
                        self.lose_hp(CombatSide::Player, player_idx, hp);
                    }
                    "Doubt" => {
                        let weak = canonical_int_value(card, "Weak", *upgrade);
                        self.apply_power(
                            CombatSide::Player,
                            player_idx,
                            "WeakPower",
                            weak,
                        );
                    }
                    "Shame" => {
                        let frail = canonical_int_value(card, "Frail", *upgrade);
                        self.apply_power(
                            CombatSide::Player,
                            player_idx,
                            "FrailPower",
                            frail,
                        );
                    }
                    "Debt" => {
                        let gold = canonical_int_value(card, "Gold", *upgrade);
                        if let Some(ps) = self
                            .allies
                            .get_mut(player_idx)
                            .and_then(|c| c.player.as_mut())
                        {
                            let lost = gold.min(ps.pending_gold);
                            ps.pending_gold = (ps.pending_gold - lost).max(0);
                        }
                    }
                    "Regret" => {
                        // damage = hand size at this moment. Unblockable.
                        self.lose_hp(CombatSide::Player, player_idx, hand_size);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Move every Innate card from the player's draw pile to the
    /// hand. Returns the number moved. Called once at combat start
    /// before the standard initial draw. Mirrors C# innate-priority
    /// shuffle (PlayerCombatState start-of-combat).
    pub fn move_innate_cards_to_hand(&mut self, player_idx: usize) -> i32 {
        let Some(creature) = self.allies.get_mut(player_idx) else {
            return 0;
        };
        let Some(ps) = creature.player.as_mut() else {
            return 0;
        };
        let mut moved = 0;
        let n = ps.draw.cards.len();
        // Iterate descending so removal doesn't invalidate indices.
        for i in (0..n).rev() {
            let is_innate = card_by_id(&ps.draw.cards[i].id)
                .map(|d| d.keywords.iter().any(|k| k == "Innate"))
                .unwrap_or(false);
            if is_innate {
                let card = ps.draw.cards.remove(i);
                ps.hand.cards.push(card);
                moved += 1;
            }
        }
        moved
    }

    /// Draw up to `n` cards from the first player's draw pile, reshuffling
    /// discard into draw when draw is exhausted. Stops early if both piles
    /// are empty. Uses `rng.shuffle()` (== C# `Rng.Shuffle` Fisher-Yates),
    /// matching `RunState.Rng.Shuffle` semantics. Returns the number drawn.
    pub fn draw_cards(&mut self, player_idx: usize, n: i32, rng: &mut Rng) -> i32 {
        let mut drawn_ids: Vec<String> = Vec::new();
        {
            let Some(creature) = self.allies.get_mut(player_idx) else {
                return 0;
            };
            let Some(ps) = creature.player.as_mut() else {
                return 0;
            };
            for _ in 0..n {
                if ps.draw.is_empty() {
                    if ps.discard.is_empty() {
                        break;
                    }
                    ps.draw.cards.append(&mut ps.discard.cards);
                    rng.shuffle(&mut ps.draw.cards);
                }
                if let Some(card) = ps.draw.cards.pop() {
                    drawn_ids.push(card.id.clone());
                    ps.hand.cards.push(card);
                }
            }
        }
        let drawn = drawn_ids.len() as i32;
        // Emit CardDrawn events unconditionally — history-scan
        // AmountSpecs need them even when verbose logging is disabled.
        let round = self.round_number;
        for card_id in drawn_ids {
            self.combat_log.push(CombatEvent::CardDrawn {
                round,
                player_idx,
                card_id,
            });
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

    /// Append a freshly-instantiated card to the player's hand at the
    /// given upgrade level. Used by OnPlay handlers that conjure Shivs
    /// (CloakAndDagger / LeadingStrike) or temporary cards. Returns
    /// whether the append succeeded (false on bad ids / players).
    pub fn add_card_to_hand(
        &mut self,
        player_idx: usize,
        card_id: &str,
        upgrade_level: i32,
    ) -> bool {
        self.add_card_to_pile(player_idx, card_id, upgrade_level, PileType::Hand)
    }

    /// Append a freshly-instantiated card to the chosen pile (Hand,
    /// Discard, Draw, or Exhaust). Used by OnPlay handlers that
    /// conjure status / token cards into a specific pile — e.g.,
    /// BoostAway dazes into discard, CollisionCourse drops Debris in
    /// hand.
    pub fn add_card_to_pile(
        &mut self,
        player_idx: usize,
        card_id: &str,
        upgrade_level: i32,
        pile: PileType,
    ) -> bool {
        let Some(card) = crate::card::by_id(card_id) else {
            return false;
        };
        let Some(ps) = self.allies.get_mut(player_idx).and_then(|c| c.player.as_mut())
        else {
            return false;
        };
        let instance = CardInstance::from_card(card, upgrade_level);
        match pile {
            PileType::Hand => ps.hand.cards.push(instance),
            PileType::Discard => ps.discard.cards.push(instance),
            PileType::Draw => ps.draw.cards.push(instance),
            PileType::Exhaust => ps.exhaust.cards.push(instance),
            // None / Play / Deck have no in-combat pile representation
            // here; treat as a silent no-op rather than panicking.
            _ => return false,
        }
        true
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
        // HardenedShellPower budget: per-turn HP-loss cap. Computed
        // BEFORE block resolution so block still soaks normally; the
        // cap only clips the residual HP loss. Mirrors C#
        // HardenedShellPower.ModifyHpLostBeforeOstyLate. None if the
        // target doesn't have HardenedShell (the common case).
        let hp_loss_cap = {
            let target = match creature(self, side, target_idx) {
                Some(t) => t,
                None => return DamageOutcome::default(),
            };
            let shell_amount = target
                .powers
                .iter()
                .find(|p| p.id == "HardenedShellPower")
                .map(|p| p.amount);
            shell_amount.map(|amt| {
                let taken = target
                    .monster
                    .as_ref()
                    .map(|m| m.counter("hardened_shell_taken"))
                    .unwrap_or(0);
                (amt - taken).max(0)
            })
        };
        let outcome = {
            let Some(target) = creature_mut(self, side, target_idx) else {
                return DamageOutcome::default();
            };
            damage_creature(target, amount, hp_loss_cap)
        };
        // HardenedShellPower bookkeeping: bump damageReceivedThisTurn
        // by the realized hp_lost. C# AfterDamageReceived adds
        // result.UnblockedDamage (i.e. hp_lost in our model).
        if hp_loss_cap.is_some() && outcome.hp_lost > 0 {
            if let Some(target) = creature_mut(self, side, target_idx) {
                if let Some(ms) = target.monster.as_mut() {
                    ms.add_counter("hardened_shell_taken", outcome.hp_lost);
                }
            }
        }
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
        self.modify_block_with_enchantment(gainer, raw, props, None)
    }

    /// Enchantment-aware block-modifier pipeline. Mirrors C#
    /// `Hook.ModifyBlock` (Hook.cs:1294-1324):
    /// 1. Enchantment additive then multiplicative (pre-power).
    /// 2. Full additive sweep over listener powers (Dexterity).
    /// 3. Full multiplicative sweep over listener powers (Frail).
    /// 4. Clamp at 0.
    ///
    /// Audit fix #6: was missing the enchantment phase, so Nimble /
    /// Goopy and any future block-enchantment didn't participate.
    pub fn modify_block_with_enchantment(
        &self,
        gainer: (CombatSide, usize),
        raw: i32,
        props: ValueProp,
        enchantment: Option<&EnchantmentInstance>,
    ) -> i32 {
        let mut num = raw as f64;
        if let Some(ench) = enchantment {
            num += enchantment_block_additive(&ench.id, ench.amount, props);
            num *= enchantment_block_multiplicative(&ench.id, ench.amount, props);
        }
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
        // C# spec (PowerCmd.cs:90): `if (amount == 0) return;`. We
        // skip the inner mutation entirely on amount=0 to match.
        if amount == 0 {
            return self.get_power_amount(side, target_idx, power_id);
        }
        // Audit fix #5: BeforePowerAmountChanged hook fires once before
        // the apply mutates the stack. Currently a no-op — ArtifactPower
        // and similar modifier-pipeline relics will register here when
        // ported.
        self.fire_before_power_amount_changed(side, target_idx, power_id, amount);
        let result = self.apply_power_inner(side, target_idx, power_id, amount);
        // Mirror C# PowerCmd.cs:129-132 — after the actual stack
        // mutation, if a Debuff lands on a player, flag the next
        // duration tick to skip. Prevents Weak/Frail/Vulnerable applied
        // to the player from losing a turn before they're felt.
        if side == CombatSide::Player && result > 0 {
            if let Some(p) = crate::power::by_id(power_id) {
                if matches!(p.power_type, crate::power::PowerType::Debuff) {
                    if let Some(target) = creature_mut(self, side, target_idx) {
                        if let Some(inst) =
                            target.powers.iter_mut().find(|q| q.id == power_id)
                        {
                            inst.skip_next_duration_tick = true;
                        }
                    }
                }
            }
        }
        // Audit fix #5: AfterPowerAmountChanged hook fires once after
        // mutation. Currently a no-op — placeholder for future power
        // lifecycle hooks (Outbreak.AfterPowerAmountChanged etc.).
        self.fire_after_power_amount_changed(side, target_idx, power_id, amount);
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

    /// Mirror of C# `Hook.BeforePowerAmountChanged` (Hook.cs around
    /// line 1783). Fires before `apply_power_inner` mutates the stack.
    /// Currently a no-op stub. Audit fix #5.
    pub fn fire_before_power_amount_changed(
        &mut self,
        _side: CombatSide,
        _target_idx: usize,
        _power_id: &str,
        _amount: i32,
    ) {
        // No registered listeners yet. Future: ArtifactPower's
        // TryModifyPowerAmountReceived which can return 0 to block
        // a debuff (consuming an Artifact charge).
    }

    /// Mirror of C# `Hook.AfterPowerAmountChanged`. Fires after
    /// mutation. Currently a no-op stub. Audit fix #5.
    pub fn fire_after_power_amount_changed(
        &mut self,
        _side: CombatSide,
        _target_idx: usize,
        _power_id: &str,
        _amount: i32,
    ) {
        // No registered listeners yet. Future: Outbreak (mod-N counter
        // wrap), SwordSage / TempStrength echo-suppression, Shroud
        // self-cancel.
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
                        skip_next_duration_tick: false,
                        state: std::collections::HashMap::new(),
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
                        skip_next_duration_tick: false,
                        state: std::collections::HashMap::new(),
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

    /// Remove a power outright regardless of its stack type. Used by
    /// effects like Verdict that "Remove<SoarPower>" — Single-stack
    /// powers can't be decremented via apply_power (which is a no-op
    /// for Single regardless of negative amount), so unconditional
    /// removal is needed.
    pub fn remove_power(
        &mut self,
        side: CombatSide,
        target_idx: usize,
        power_id: &str,
    ) {
        if let Some(target) = creature_mut(self, side, target_idx) {
            target.powers.retain(|p| p.id != power_id);
        }
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
        // Audit fix #7: C# returns a zero-damage DamageResult immediately
        // if the dealer is dead at entry (so dead-dealer multi-hit attacks
        // halt mid-loop, and Strength etc. don't apply post-mortem).
        if dealer_is_dead(self, dealer) {
            return DamageOutcome::default();
        }
        let modified = self.modify_damage(dealer, target, raw, props);
        let outcome = self.apply_damage(target.0, target.1, modified);
        self.fire_after_damage_given_hooks(dealer, target, &outcome, props);
        self.fire_after_damage_received_hooks(dealer, target, &outcome, props);
        self.fire_thorns_hook(dealer, target, props);
        outcome
    }

    /// Channel an orb (push to the queue). Mirrors C# `OrbCmd.Channel<T>`
    /// + `PlayerCombatState.AddOrbToQueue`. If the queue is full,
    /// evoke the front orb first to make room.
    pub fn channel_orb(&mut self, player_idx: usize, orb_id: &str) {
        let needs_evict = self
            .allies
            .get(player_idx)
            .and_then(|c| c.player.as_ref())
            .map(|ps| ps.orb_queue.len() as i32 >= ps.orb_slots)
            .unwrap_or(false);
        if needs_evict {
            self.evoke_next_orb(player_idx);
        }
        if let Some(ps) = self
            .allies
            .get_mut(player_idx)
            .and_then(|c| c.player.as_mut())
        {
            ps.orb_queue.push(OrbInstance {
                id: orb_id.to_string(),
                evoke_val_bonus: 0,
            });
        }
        // Emit OrbChanneled history event.
        let round = self.round_number;
        self.combat_log.push(CombatEvent::OrbChanneled {
            round,
            player_idx,
            orb_id: orb_id.to_string(),
        });
    }

    /// Evoke the front orb (pop + run its evoke effect). Mirrors C#
    /// `OrbCmd.EvokeNext`.
    pub fn evoke_next_orb(&mut self, player_idx: usize) {
        let orb = {
            let ps = self
                .allies
                .get_mut(player_idx)
                .and_then(|c| c.player.as_mut());
            let Some(ps) = ps else {
                return;
            };
            if ps.orb_queue.is_empty() {
                return;
            }
            ps.orb_queue.remove(0)
        };
        self.run_orb_evoke(player_idx, &orb);
    }

    /// Trigger the passive of every orb in the queue (without
    /// consuming them). Lightning/Frost/Plasma all run this on a
    /// scheduled phase. Card-driven trigger (TriggerOrbPassive
    /// primitive) also lands here.
    pub fn trigger_orb_passives(&mut self, player_idx: usize) {
        let orbs: Vec<OrbInstance> = self
            .allies
            .get(player_idx)
            .and_then(|c| c.player.as_ref())
            .map(|ps| ps.orb_queue.clone())
            .unwrap_or_default();
        for orb in orbs {
            self.run_orb_passive(player_idx, &orb);
        }
    }

    /// Adjust the orb queue capacity by `delta`. Capacitor /
    /// PotionOfCapacity / BulkUp use this.
    pub fn change_orb_slots(&mut self, player_idx: usize, delta: i32) {
        if let Some(ps) = self
            .allies
            .get_mut(player_idx)
            .and_then(|c| c.player.as_mut())
        {
            ps.orb_slots = (ps.orb_slots + delta).max(0);
        }
    }

    /// Per-orb evoke behavior. Mirrors the body of each Orb's
    /// `Evoke()` method in C# `Models/Orbs/`.
    fn run_orb_evoke(&mut self, player_idx: usize, orb: &OrbInstance) {
        match orb.id.as_str() {
            "LightningOrb" => {
                // Damage 1 random alive enemy by 8 (base EvokeVal),
                // unpowered. C# LightningOrb.cs:93.
                let alive: Vec<usize> = self
                    .enemies
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| if e.current_hp > 0 { Some(i) } else { None })
                    .collect();
                if alive.is_empty() {
                    return;
                }
                let pick = self.rng.next_int_range(0, alive.len() as i32) as usize;
                self.deal_damage(
                    (CombatSide::Player, player_idx),
                    (CombatSide::Enemy, alive[pick]),
                    8,
                    ValueProp::UNPOWERED,
                );
            }
            "FrostOrb" => {
                // GainBlock(8, Unpowered) on self. C# FrostOrb.cs.
                self.gain_block_with_props(
                    CombatSide::Player,
                    player_idx,
                    8,
                    ValueProp::UNPOWERED,
                );
            }
            "DarkOrb" => {
                // Damage the lowest-HP alive enemy by (8 + accumulated charge).
                // C# DarkOrb.cs: evoke applies stored _evokeVal.
                let alive: Vec<(usize, i32)> = self
                    .enemies
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| if e.current_hp > 0 { Some((i, e.current_hp)) } else { None })
                    .collect();
                if alive.is_empty() {
                    return;
                }
                let (target_idx, _) = alive
                    .iter()
                    .min_by_key(|(_, hp)| *hp)
                    .copied()
                    .unwrap_or((alive[0].0, alive[0].1));
                self.deal_damage(
                    (CombatSide::Player, player_idx),
                    (CombatSide::Enemy, target_idx),
                    8 + orb.evoke_val_bonus,
                    ValueProp::UNPOWERED,
                );
            }
            "PlasmaOrb" => {
                // GainEnergy(EvokeVal). C# PlasmaOrb.cs.
                if let Some(ps) = self
                    .allies
                    .get_mut(player_idx)
                    .and_then(|c| c.player.as_mut())
                {
                    ps.energy += 2; // PlasmaOrb EvokeVal default = 2
                }
            }
            "GlassOrb" => {
                // GlassOrb: complex; deferred. Currently no-op.
            }
            _ => {}
        }
    }

    /// Per-orb passive behavior. Lightning damages a random enemy
    /// before turn-end; Frost grants block to self; Dark accumulates
    /// charge; Plasma grants 1 energy; Glass deferred.
    fn run_orb_passive(&mut self, player_idx: usize, orb: &OrbInstance) {
        match orb.id.as_str() {
            "LightningOrb" => {
                // PassiveVal = 3.
                let alive: Vec<usize> = self
                    .enemies
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| if e.current_hp > 0 { Some(i) } else { None })
                    .collect();
                if alive.is_empty() {
                    return;
                }
                let pick = self.rng.next_int_range(0, alive.len() as i32) as usize;
                self.deal_damage(
                    (CombatSide::Player, player_idx),
                    (CombatSide::Enemy, alive[pick]),
                    3,
                    ValueProp::UNPOWERED,
                );
            }
            "FrostOrb" => {
                self.gain_block_with_props(
                    CombatSide::Player,
                    player_idx,
                    3,
                    ValueProp::UNPOWERED,
                );
            }
            "DarkOrb" => {
                // Accumulate charge into front-of-queue orb. We need
                // mutable access to the orb instance; find it by id+pos.
                if let Some(ps) = self
                    .allies
                    .get_mut(player_idx)
                    .and_then(|c| c.player.as_mut())
                {
                    for o in ps.orb_queue.iter_mut() {
                        if o.id == "DarkOrb" {
                            o.evoke_val_bonus += 6; // PassiveVal default
                            break;
                        }
                    }
                }
            }
            "PlasmaOrb" => {
                if let Some(ps) = self
                    .allies
                    .get_mut(player_idx)
                    .and_then(|c| c.player.as_mut())
                {
                    ps.energy += 1; // PlasmaOrb PassiveVal default = 1
                }
            }
            _ => {}
        }
    }

    /// Mirror of C# `Hook.BeforeAttack` — called once at the start of
    /// each AttackCommand (a card OnPlay attack chain or a monster
    /// attack move). Powers that snapshot per-attack state (VigorPower)
    /// hook here.
    ///
    /// Caller pairs with `fire_after_attack(dealer)` at the end of the
    /// hit loop. `execute_attack` is the canonical way to bracket a
    /// multi-hit attack with this envelope.
    pub fn fire_before_attack(&mut self, dealer: (CombatSide, usize)) {
        // VigorPower.BeforeAttack: snapshot Amount → counter for the
        // AfterAttack drain. Only enemy-side for now (no player cards
        // currently apply Vigor; if/when they do, this needs a
        // player-side scratch field on Creature).
        if dealer.0 == CombatSide::Enemy {
            let amt = self
                .enemies
                .get(dealer.1)
                .and_then(|c| {
                    c.powers
                        .iter()
                        .find(|p| p.id == "VigorPower")
                        .map(|p| p.amount)
                })
                .unwrap_or(0);
            if amt > 0 {
                if let Some(ms) = self
                    .enemies
                    .get_mut(dealer.1)
                    .and_then(|c| c.monster.as_mut())
                {
                    ms.set_counter("vigor_snapshot", amt);
                }
            }
        }
    }

    /// Mirror of C# `Hook.AfterAttack` — called once at the end of each
    /// AttackCommand. Powers that consume per-attack snapshots hook here.
    pub fn fire_after_attack(&mut self, dealer: (CombatSide, usize)) {
        // VigorPower.AfterAttack: ModifyAmount(-snapshot). Drain only
        // the snapshotted amount, not Vigor applied DURING the attack.
        if dealer.0 == CombatSide::Enemy {
            let snap = self
                .enemies
                .get(dealer.1)
                .and_then(|c| c.monster.as_ref())
                .map(|m| m.counter("vigor_snapshot"))
                .unwrap_or(0);
            if snap > 0 {
                self.decrement_power(CombatSide::Enemy, dealer.1, "VigorPower", snap);
                if let Some(ms) = self
                    .enemies
                    .get_mut(dealer.1)
                    .and_then(|c| c.monster.as_mut())
                {
                    ms.set_counter("vigor_snapshot", 0);
                }
            }
        }
    }

    /// Bracket a multi-hit attack with `fire_before_attack` +
    /// per-hit `deal_damage_enchanted` + `fire_after_attack`.
    /// Canonical entry point for any code path that represents one
    /// C# `AttackCommand`. Card VM dispatches through this; monster
    /// attack moves should migrate to it.
    pub fn execute_attack(
        &mut self,
        dealer: (CombatSide, usize),
        target: (CombatSide, usize),
        raw_per_hit: i32,
        hits: i32,
        props: ValueProp,
        enchantment: Option<&EnchantmentInstance>,
    ) {
        self.fire_before_attack(dealer);
        for _ in 0..hits.max(1) {
            self.deal_damage_enchanted(dealer, target, raw_per_hit, props, enchantment);
        }
        self.fire_after_attack(dealer);
    }

    /// Same as `execute_attack` but the target is re-rolled per hit
    /// (matches `DamageCmd.Attack(...).TargetingRandomOpponents(...,
    /// reroll_dead=true)` — SwordBoomerang).
    pub fn execute_attack_random_target(
        &mut self,
        dealer: (CombatSide, usize),
        raw_per_hit: i32,
        hits: i32,
        props: ValueProp,
        enchantment: Option<&EnchantmentInstance>,
    ) {
        self.fire_before_attack(dealer);
        for _ in 0..hits.max(1) {
            let alive: Vec<usize> = self
                .enemies
                .iter()
                .enumerate()
                .filter_map(|(i, e)| if e.current_hp > 0 { Some(i) } else { None })
                .collect();
            if alive.is_empty() {
                break;
            }
            let pick = self.rng.next_int_range(0, alive.len() as i32) as usize;
            let target = (CombatSide::Enemy, alive[pick]);
            self.deal_damage_enchanted(dealer, target, raw_per_hit, props, enchantment);
        }
        self.fire_after_attack(dealer);
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
        // Audit fix #7: dead-dealer short-circuit.
        if dealer_is_dead(self, dealer) {
            return DamageOutcome::default();
        }
        let modified =
            self.modify_damage_with_enchantment(dealer, target, raw, props, enchantment);
        let outcome = self.apply_damage(target.0, target.1, modified);
        self.fire_after_damage_given_hooks(dealer, target, &outcome, props);
        self.fire_after_damage_received_hooks(dealer, target, &outcome, props);
        self.fire_thorns_hook(dealer, target, props);
        outcome
    }

    /// Fire `AfterDamageReceived` hooks for target-side per-power
    /// listeners. Currently models CurlUpPower: when owner takes any
    /// HP-loss from a powered Player attack, owner gains Amount
    /// unpowered block and CurlUpPower removes itself. C# delays the
    /// block grant to AfterCardPlayed (so multi-hit cards land all
    /// hits before block applies); we trigger eagerly on the first
    /// hit. Net difference: one fewer absorbed hit on multi-hit
    /// cards. Acceptable simplification.
    fn fire_after_damage_received_hooks(
        &mut self,
        dealer: (CombatSide, usize),
        target: (CombatSide, usize),
        outcome: &DamageOutcome,
        props: ValueProp,
    ) {
        if !props.is_powered_attack() {
            return;
        }
        if outcome.hp_lost <= 0 {
            return;
        }
        // CurlUp triggers only on player attacks (the C# check is
        // cardSource != null). Enemy-on-player damage shouldn't
        // trigger a player CurlUp (none today, but be safe).
        if dealer.0 != CombatSide::Player {
            return;
        }
        let target_powers: Vec<(String, i32)> = creature_powers(self, target)
            .iter()
            .map(|p| (p.id.clone(), p.amount))
            .collect();
        for (power_id, amount) in target_powers {
            if amount <= 0 {
                continue;
            }
            if power_id == "CurlUpPower" {
                self.gain_block_with_props(
                    target.0,
                    target.1,
                    amount,
                    ValueProp::UNPOWERED,
                );
                // LouseProgenitor reads Curled in its state machine
                // for animations only; we still set the flag so any
                // future state-machine arm can branch on it.
                if let Some(creature) = creature_mut(self, target.0, target.1) {
                    if let Some(ms) = creature.monster.as_mut() {
                        ms.set_flag("curled", true);
                    }
                }
                self.remove_power(target.0, target.1, "CurlUpPower");
            } else if power_id == "VitalSparkPower" {
                // VitalSparkPower.AfterDamageReceived: when owner takes
                // unblocked Player-attack damage AND that player
                // hasn't triggered VS this Enemy-turn-period, give
                // the player +1 energy. Flag clears at begin_turn
                // (Enemy) — i.e., once per Player turn-cycle.
                // C# uses Amount as the energy gain via `EnergyVar(1)`
                // (always 1 today), and tracks per-player; we
                // simplify to a single flag since the harness is
                // single-player.
                let already = creature(self, target.0, target.1)
                    .and_then(|c| c.monster.as_ref())
                    .map(|m| m.flag("vital_spark_used"))
                    .unwrap_or(false);
                if !already {
                    if let Some(creature) =
                        creature_mut(self, target.0, target.1)
                    {
                        if let Some(ms) = creature.monster.as_mut() {
                            ms.set_flag("vital_spark_used", true);
                        }
                    }
                    if let Some(ps) = self
                        .allies
                        .get_mut(dealer.1)
                        .and_then(|c| c.player.as_mut())
                    {
                        ps.energy += amount;
                    }
                }
            } else if power_id == "SkittishPower" {
                // SkittishPower.AfterAttack: when owner takes unblocked
                // Player-attack damage AND hasn't already gained block
                // this turn, gain Amount unpowered block and flip the
                // skittish_used flag. Flag clears in end_turn(Player).
                // C# additionally gates on `command.ModelSource is
                // CardModel` (only card attacks, not power-tick damage
                // like Poison). All player attacks today are card or
                // direct deal_damage, so the powered+player-side gates
                // approximate this.
                let already_used = creature(self, target.0, target.1)
                    .and_then(|c| c.monster.as_ref())
                    .map(|m| m.flag("skittish_used"))
                    .unwrap_or(false);
                if !already_used {
                    self.gain_block_with_props(
                        target.0,
                        target.1,
                        amount,
                        ValueProp::UNPOWERED,
                    );
                    if let Some(creature) =
                        creature_mut(self, target.0, target.1)
                    {
                        if let Some(ms) = creature.monster.as_mut() {
                            ms.set_flag("skittish_used", true);
                        }
                    }
                }
            } else if power_id == "AsleepPower" {
                // AsleepPower.AfterDamageReceived: any unblocked
                // damage immediately wakes the owner — remove
                // PlatingPower (if present) and remove the
                // AsleepPower stack. C# also stun-wakes the owner
                // and forces SLASH_MOVE; we skip the stun and let
                // the state machine see "no Asleep" on the next
                // intent pick and route to Slash.
                self.remove_power(target.0, target.1, "PlatingPower");
                self.remove_power(target.0, target.1, "AsleepPower");
            } else if power_id == "SlumberPower" {
                // SlumberPower.AfterDamageReceived: any unblocked
                // damage decrements the counter (C# uses
                // result.UnblockedDamage != 0 — same predicate).
                // When Amount hits 0 the C# stuns the owner and
                // forces ROLL_OUT_MOVE; we skip the stun, just
                // remove the power so the state-machine arm reads
                // "no Slumber" on the next intent pick.
                self.decrement_power(target.0, target.1, "SlumberPower", 1);
                if self.get_power_amount(target.0, target.1, "SlumberPower")
                    <= 0
                {
                    self.remove_power(target.0, target.1, "SlumberPower");
                }
            } else if power_id == "ShriekPower" {
                // ShriekPower.AfterDamageReceived: when owner's
                // CurrentHp ≤ Amount AND took unblocked damage, fire
                // the shriek — set the shriek_triggered flag for the
                // state machine to route to TerrorMove on the next
                // enemy turn, then remove the power. C# also stuns
                // the owner via TerrorState — Stun mechanic is
                // deferred so the eel just acts normally next turn
                // and we route directly to Terror.
                let owner_hp = creature(self, target.0, target.1)
                    .map(|c| c.current_hp)
                    .unwrap_or(0);
                if owner_hp <= amount {
                    if let Some(creature) =
                        creature_mut(self, target.0, target.1)
                    {
                        if let Some(ms) = creature.monster.as_mut() {
                            ms.set_flag("shriek_triggered", true);
                        }
                    }
                    self.remove_power(target.0, target.1, "ShriekPower");
                }
            }
        }
    }

    /// ThornsPower.BeforeDamageReceived: when target with ThornsPower is
    /// hit by a powered attack from a living dealer, the dealer takes
    /// `Amount` unpowered damage back. The UNPOWERED flag prevents
    /// recursive Thorns triggers (own check gates on
    /// `is_powered_attack`). Fired post-apply for ordering simplicity:
    /// outcome of the main hit is locked in before reflection.
    fn fire_thorns_hook(
        &mut self,
        dealer: (CombatSide, usize),
        target: (CombatSide, usize),
        props: ValueProp,
    ) {
        if !props.is_powered_attack() {
            return;
        }
        // Self-damage from a creature targeting itself doesn't bounce.
        if dealer == target {
            return;
        }
        let thorns = self.get_power_amount(target.0, target.1, "ThornsPower");
        if thorns <= 0 {
            return;
        }
        let dealer_alive = match dealer.0 {
            CombatSide::Player => self
                .allies
                .get(dealer.1)
                .map(|c| c.current_hp > 0)
                .unwrap_or(false),
            CombatSide::Enemy => self
                .enemies
                .get(dealer.1)
                .map(|c| c.current_hp > 0)
                .unwrap_or(false),
            CombatSide::None => false,
        };
        if !dealer_alive {
            return;
        }
        // Reflect: apply_damage bypasses modify_damage so block on the
        // dealer's side still soaks via apply_damage's block check, but
        // power-pipeline mods like Strength/Vulnerable don't apply to
        // reflected damage. Matches C#: CreatureCmd.Damage with
        // ValueProp.Unpowered.
        self.apply_damage(dealer.0, dealer.1, thorns);
    }

    /// Fire `AfterDamageGiven` hooks for every per-power listener on
    /// the dealer side. Currently models PaperCutsPower: when owner
    /// deals powered-attack damage that gets through block to the
    /// player, reduce player's max_hp by Amount.
    ///
    /// Hook firing order proper lands in #70; for now per-power arms
    /// run inline. Snapshot-then-act pattern so mutations don't disrupt
    /// power iteration.
    fn fire_after_damage_given_hooks(
        &mut self,
        dealer: (CombatSide, usize),
        target: (CombatSide, usize),
        outcome: &DamageOutcome,
        props: ValueProp,
    ) {
        let is_powered = props.is_powered_attack();
        let hp_landed = outcome.hp_lost > 0;
        let fully_blocked = outcome.blocked > 0 && outcome.hp_lost == 0;
        let target_is_player = target.0 == CombatSide::Player;
        // Snapshot (power_id, amount) pairs from dealer-side powers so
        // subsequent mutations don't disrupt iteration.
        let dealer_powers: Vec<(String, i32)> = creature_powers(self, dealer)
            .iter()
            .map(|p| (p.id.clone(), p.amount))
            .collect();
        for (power_id, amount) in dealer_powers {
            if amount <= 0 {
                continue;
            }
            match power_id.as_str() {
                "PaperCutsPower" if is_powered && hp_landed && target_is_player => {
                    self.change_max_hp(target.0, target.1, -amount);
                }
                // ImbalancedPower.AfterDamageGiven: when owner's attack
                // is fully blocked by the target, flag the dealer as
                // off-balance. C# BowlbugRock reads this in its next
                // intent branch. Other monsters with ImbalancedPower
                // would get Stunned in C#; we just set the flag
                // (Stun mechanic not yet ported).
                "ImbalancedPower" if fully_blocked => {
                    if let Some(creature) =
                        creature_mut(self, dealer.0, dealer.1)
                    {
                        if let Some(ms) = creature.monster.as_mut() {
                            ms.set_flag("is_off_balance", true);
                        }
                    }
                }
                _ => {}
            }
        }
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
                energy_cost = card.effective_energy_cost();
                x_value = 0;
            }
            if ps.energy < energy_cost {
                return PlayResult::InsufficientEnergy {
                    available: ps.energy,
                    required: energy_cost,
                };
            }
            // Unplayable keyword gates manual play. Mirrors C#
            // `CardModel.CanPlay -> UnplayableReason.HasUnplayableKeyword`.
            // Status cards (Wound, Slimed) and curses with Unplayable
            // (BadLuck, Burn, Decay, Doubt, Injury, Normality, etc.)
            // reject the play entirely.
            if data.keywords.iter().any(|k| k == "Unplayable") {
                return PlayResult::Unplayable;
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
        let mut played_card = {
            let ps = self.allies[player_idx].player.as_mut().unwrap();
            ps.hand.cards.remove(hand_idx)
        };
        // `cost_override_until_played` consumed by THIS play — clear so
        // the override doesn't survive if the card lands back in hand
        // (e.g., a future "return to hand" effect).
        played_card.cost_override_until_played = None;

        // Emit CardPlayed event for history-scan AmountSpecs and
        // FirstPlayOfSourceCardThisTurn / PlaysThisTurnLt conditions.
        // (Order: emitted BEFORE the OnPlay body executes, so
        // FirstPlayOfSourceCardThisTurn evaluates true on the first play
        // only when the in-flight event is excluded — the resolver
        // tolerates this by treating "0 historical plays" as first.)
        // History events emit unconditionally — log_enabled gates only
        // the verbose damage/block/power events.
        let ethereal = card_data.keywords.iter().any(|k| k == "Ethereal");
        let round = self.round_number;
        self.combat_log.push(CombatEvent::CardPlayed {
            round,
            player_idx,
            card_id: card_id.clone(),
            card_type: card_data.card_type,
            cost: energy_cost,
            ethereal,
        });

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
        // Post-play routing: ONLY the `Exhaust` keyword routes the card
        // to the exhaust pile. CardType::Status / Curse do NOT auto-
        // exhaust -- per the keyword clarification, an Unplayable status
        // is a dead card in hand that interacts like any other card and
        // routes through the normal flush. Cards that should always
        // exhaust have the explicit Exhaust keyword (Debris, AdaptiveStrike,
        // Cinder, MoltenFist, TrueGrit-upgraded, etc.). Ethereal cards
        // routed via end-of-turn flush (handled in end_turn).
        let dest = if card_data.keywords.iter().any(|k| k == "Exhaust") {
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
        // History emission for the routing: CardExhausted or CardDiscarded.
        let round = self.round_number;
        match dest {
            PileType::Discard => self.combat_log.push(CombatEvent::CardDiscarded {
                round,
                player_idx,
                card_id: card_id.clone(),
            }),
            PileType::Exhaust => self.combat_log.push(CombatEvent::CardExhausted {
                round,
                player_idx,
                card_id: card_id.clone(),
            }),
            _ => {}
        }

        // AfterCardPlayed relic-hook firing point. Fires after OnPlay
        // resolves AND the card has routed. Data-driven via relic_effects.
        crate::effects::fire_relic_hooks_after_card_played(
            self,
            player_idx,
            &card_id,
            card_data.card_type,
            &card_data.keywords,
            &card_data.tags,
        );
        // AfterCardExhausted / AfterCardDiscarded relic hooks — fire
        // after the routing event so the relic body sees the post-route
        // state (matches C# AfterCardExhausted / AfterCardDiscarded
        // ordering — fires after the card has moved piles).
        match dest {
            PileType::Discard => crate::effects::fire_relic_hooks(
                self,
                crate::effects::RelicHookKind::AfterCardDiscarded,
                CombatSide::Player,
            ),
            PileType::Exhaust => crate::effects::fire_relic_hooks(
                self,
                crate::effects::RelicHookKind::AfterCardExhausted,
                CombatSide::Player,
            ),
            _ => {}
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
    /// The card has the Unplayable keyword (status cards like Wound /
    /// Slimed, curses like BadLuck / Burn / Doubt / Injury / Normality
    /// / Pride / Regret). Mirrors C#
    /// `UnplayableReason.HasUnplayableKeyword`.
    Unplayable,
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
    // Data-driven path (plan §0.2.6). Cards whose OnPlay is fully
    // expressible as an effect list live in `effects::card_effects` and
    // execute through the VM without a match-arm here.
    if let Some(effects) = crate::effects::card_effects(card_id) {
        let ctx = crate::effects::EffectContext::for_card(
            player_idx,
            target,
            card_id,
            upgrade_level,
            enchantment,
            x_value,
        );
        crate::effects::execute_effects(cs, &effects, &ctx);
        return true;
    }
    match card_id {
        // TwinStrike (Ironclad common): 5 damage × 2 hits to single
        // enemy. Upgrade: +2 per hit (becomes 7×2). C# uses
        // `.WithHitCount(2)` — each hit goes through modifiers independently.
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
        // GraveWarden (Necrobinder common Skill, 1E, Self): 8 block
        // (11 upgraded) + add N Soul tokens to draw pile (N=1; Cards
        // var doesn't upgrade — only Block does). Soul is a Token-pool
        // 0-cost Skill with Exhaust keyword.
        "GraveWarden" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let block = canonical_int_value(card, "Block", upgrade_level);
            let souls = canonical_int_value(card, "Cards", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            for _ in 0..souls {
                cs.add_card_to_pile(player_idx, "Soul", 0, PileType::Draw);
            }
            true
        }
        // BlightStrike (Necrobinder common Strike-tagged Attack, 1E,
        // AnyEnemy): 8 damage (10 upgraded) + apply DoomPower equal to
        // damage dealt (modify_damage output, pre-block-split — matches
        // C# Result.TotalDamage). DoomPower's BeforeTurnEnd hook
        // finishes the target if their HP drops at or below the Doom
        // amount before their turn ends.
        "BlightStrike" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let raw = canonical_int_value(card, "Damage", upgrade_level);
            let modified = cs.modify_damage_with_enchantment(
                (CombatSide::Player, player_idx),
                target,
                raw,
                ValueProp::MOVE,
                enchantment,
            );
            cs.apply_damage(target.0, target.1, modified);
            if modified > 0 {
                cs.apply_power(target.0, target.1, "DoomPower", modified);
            }
            true
        }
        // ---------- Defect / Regent cross-pool commons -----------
        // CollisionCourse (Regent common Attack, 0E, AnyEnemy): 11
        // damage (15 upgraded) + add Debris (status, 1E Exhaust) to
        // hand. Debris OnPlay isn't ported yet — tracking presence
        // suffices for the agent's hand observation.
        "CollisionCourse" => {
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
            cs.add_card_to_pile(player_idx, "Debris", 0, PileType::Hand);
            true
        }
        // BladeDance (Silent common Exhaust Skill, 1E, Self): add N
        // Shivs to hand (N=3 base, 4 upgraded). Exhausts via keyword.
        "BladeDance" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let shivs = canonical_int_value(card, "Cards", upgrade_level);
            for _ in 0..shivs {
                cs.add_card_to_hand(player_idx, "Shiv", 0);
            }
            true
        }
        // Snakebite (Silent common Retain Skill, 2E, AnyEnemy): apply
        // 7 Poison (10 upgraded). Retain keyword handling — keeps the
        // card in hand at end-of-turn discard — is deferred; doesn't
        // affect OnPlay.
        "Snakebite" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let poison = canonical_int_value(card, "Poison", upgrade_level);
            cs.apply_power(target.0, target.1, "PoisonPower", poison);
            true
        }
        // Anticipate (Silent common Skill, 0E, Self): apply 2
        // AnticipatePower (3 upgraded). AnticipatePower extends
        // TemporaryDexterityPower (IsPositive=true) → silently grants
        // matching DexterityPower amount; at end of owner's turn,
        // removes itself + restores Dexterity. tick_temporary_strength_powers
        // handles the cleanup via the (AnticipatePower, +1, DexterityPower)
        // table entry.
        "Anticipate" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let dex = canonical_int_value(card, "Dexterity", upgrade_level);
            cs.apply_power(
                CombatSide::Player,
                player_idx,
                "AnticipatePower",
                dex,
            );
            cs.apply_power(
                CombatSide::Player,
                player_idx,
                "DexterityPower",
                dex,
            );
            true
        }
        // Untouchable (Silent common Skill, 2E, Self): 6 block (8
        // upgraded). Sly keyword is a metadata tag with no effect on
        // OnPlay.
        "Untouchable" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let block = canonical_int_value(card, "Block", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            true
        }
        // FlickFlack (Silent common Attack, 1E, AllEnemies): 6 damage
        // (8 upgraded) to all enemies once. Sly keyword tag only.
        "FlickFlack" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
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
        // Ricochet (Silent common Attack, 2E, RandomEnemy, Sly): 3
        // damage × 4 hits (5 hits upgraded), each picks a fresh random
        // alive enemy. Identical pattern to SwordBoomerang.
        "Ricochet" => {
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
        // Shiv (Token Attack, 0 cost, AnyEnemy): 4 damage (6 upgraded).
        // Exhaust keyword routes the played card to exhaust. Generated
        // in hand by Silent Shiv-creating cards (CloakAndDagger,
        // LeadingStrike, etc.).
        "Shiv" => {
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
        // Backflip (Silent common Skill, 1 cost, Self): 5 block (8
        // upgraded) + draw 2. Cards count doesn't upgrade.
        "Backflip" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let block = canonical_int_value(card, "Block", upgrade_level);
            let cards = canonical_int_value(card, "Cards", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            cs.draw_cards_self_rng(player_idx, cards);
            true
        }
        // CloakAndDagger (Silent common Skill, 1 cost, Self): 6 block
        // + add N Shivs to hand (N=1, 2 upgraded). Block doesn't
        // upgrade.
        "CloakAndDagger" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let block = canonical_int_value(card, "Block", upgrade_level);
            let shivs = canonical_int_value(card, "Cards", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            for _ in 0..shivs {
                cs.add_card_to_hand(player_idx, "Shiv", 0);
            }
            true
        }
        // LeadingStrike (Silent common Strike-tagged Attack, 1 cost,
        // AnyEnemy): 3 damage (6 upgraded) + add 2 Shivs to hand.
        // Damage upgrades, Shiv count doesn't. The keyed CardsVar
        // "Shivs" tracks the count.
        "LeadingStrike" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            let shivs = canonical_int_value(card, "Shivs", upgrade_level);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            for _ in 0..shivs {
                cs.add_card_to_hand(player_idx, "Shiv", 0);
            }
            true
        }
        // ---------- Silent commons batch ---------------------------
        // DaggerThrow (Silent common Attack, 1 cost): 9 damage (12
        // upgraded). Pure damage.
        "DaggerThrow" => {
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
        // Slice (Silent common Attack, 0 cost): 6 damage (9 upgraded).
        "Slice" => {
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
        // Deflect (Silent common Skill, 0 cost, Self): 4 block (7
        // upgraded). Routes through modify_block.
        "Deflect" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let block = canonical_int_value(card, "Block", upgrade_level);
            cs.gain_block(CombatSide::Player, player_idx, block);
            true
        }
        // DaggerSpray (Silent common Attack, 1 cost, AllEnemies): 4
        // damage (6 upgraded) to every enemy, twice. Wait — actually
        // 4 damage once. Let me verify against C#: DaggerSpray hits
        // all enemies twice in C# but vars only show single Damage.
        // Re-checking the source: DaggerSpray uses WithHitCount(2)
        // for the AoE — STS1 classic. Match that here.
        "DaggerSpray" => {
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            for _ in 0..2 {
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
        // SuckerPunch (Silent common Attack, 1 cost): 8 damage (10
        // upgraded) + 1 Weak (2 upgraded).
        "SuckerPunch" => {
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
        // PoisonedStab (Silent common Attack, 1 cost): 6 damage (8
        // upgraded) + 3 Poison (4 upgraded).
        "PoisonedStab" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let damage = canonical_int_value(card, "Damage", upgrade_level);
            let poison = canonical_int_value(card, "Poison", upgrade_level);
            cs.deal_damage_enchanted(
                (CombatSide::Player, player_idx),
                target,
                damage,
                ValueProp::MOVE,
                enchantment,
            );
            cs.apply_power(target.0, target.1, "PoisonPower", poison);
            true
        }
        // DeadlyPoison (Silent common Skill, 1 cost, AnyEnemy): apply
        // 5 Poison (7 upgraded).
        "DeadlyPoison" => {
            let Some(target) = target else { return false; };
            let Some(card) = card_by_id(card_id) else { return false; };
            let poison = canonical_int_value(card, "Poison", upgrade_level);
            cs.apply_power(target.0, target.1, "PoisonPower", poison);
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
pub(crate) fn canonical_int_value(card: &CardData, var_kind: &str, upgrade_level: i32) -> i32 {
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
        // VigorPower.ModifyDamageAdditive: +Amount on powered attacks
        // from the owner. C# only buffs hits belonging to a single
        // AttackCommand and then drains the power; we approximate by
        // snapshotting the amount at the start of the owner's turn
        // and draining by the snapshot at the end of that turn (see
        // tick_vigor_consumption). Net: Vigor applied during turn N
        // doesn't drain at end of N (snapshot was 0), so it boosts
        // turn N+1's attack and then clears.
        "VigorPower" if power.amount > 0 => power.amount as f64,
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
        // ShrinkPower: ×0.70 (=(100-30)/100) on powered attacks from
        // the owner. Amount.sign distinguishes finite (positive,
        // ticks down each owner-side turn end) vs infinite
        // (negative, applied by ShrinkerBeetle's Shrinker move).
        // Either way the multiplier is the same constant when the
        // power is present.
        "ShrinkPower" if power.amount != 0 => 0.70,
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
        // SoarPower: ×0.50 (= DamageDecrease 50 / 100) on powered
        // attacks landing on the owner. OwlMagistrate's flight buff —
        // present while flying, removed on Verdict.
        "SoarPower" if power.amount > 0 => 0.50,
        // FlutterPower: same ×0.50 reduction on owner-incoming powered
        // damage. C# also decrements Flutter on each unblocked hit
        // and Stuns the owner when Amount hits 0 — neither is
        // modeled here (Stun mechanic deferred). Presence-only
        // damage reduction is the playable approximation.
        "FlutterPower" if power.amount > 0 => 0.50,
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
        // HardToKillPower.ModifyDamageCap: caps each incoming hit at
        // `Amount`. Exoskeleton spawns with HardToKill(9).
        "HardToKillPower" if power.amount > 0 => power.amount as f64,
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

/// Per-enchantment `EnchantBlockAdditive` contribution. Audit fix #6:
/// previously missing from the block-modifier pipeline. C# spec
/// (Nimble.cs:28): gates on `IsPoweredCardOrMonsterMoveBlock` (== Move
/// && !Unpowered — same shape as `is_powered_attack`).
fn enchantment_block_additive(ench_id: &str, amount: i32, props: ValueProp) -> f64 {
    if !props.is_powered_attack() {
        return 0.0;
    }
    match ench_id {
        // Nimble: adds `base.Amount` to block on powered card/move-block.
        "Nimble" => amount as f64,
        _ => 0.0,
    }
}

/// Per-enchantment `EnchantBlockMultiplicative` contribution. None of
/// the surveyed C# enchantments use this slot today; placeholder
/// returning identity matches `enchantment_damage_multiplicative`'s shape.
fn enchantment_block_multiplicative(
    _ench_id: &str,
    _amount: i32,
    props: ValueProp,
) -> f64 {
    if !props.is_powered_attack() {
        return 1.0;
    }
    1.0
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

// ---------- Monster intent: Myte ---------------------------------------
//
// Reflects C# `Myte.GenerateMoveStateMachine`:
//   INIT: ConditionalBranchState
//     - if slot == "first":  start at TOXIC_MOVE
//     - if slot == "second": start at SUCK_MOVE
//   Cycle (FollowUpState chain): Toxic → Bite → Suck → Toxic → …
//
// Deterministic (unlike Axebot's weighted random) — no RNG needed for
// intent selection, only for damage modifiers.
//
// A0 values per C# `GetValueIfAscension(level, ascended, fallback)`:
//   - Bite damage: 13 (A0) / 15 (DeadlyEnemies)
//   - Suck damage: 4 (A0) / 6 (DeadlyEnemies)
//   - Suck strength self-gain: 2 (A0) / 3 (DeadlyEnemies)
//   - Toxic count: const 2

const MYTE_BITE_DAMAGE: i32 = 13;
const MYTE_SUCK_DAMAGE: i32 = 4;
const MYTE_SUCK_STRENGTH_GAIN: i32 = 2;
const MYTE_TOXIC_COUNT: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MyteIntent {
    Toxic,
    Bite,
    Suck,
}

impl MyteIntent {
    pub fn id(self) -> &'static str {
        match self {
            MyteIntent::Toxic => "TOXIC_MOVE",
            MyteIntent::Bite => "BITE_MOVE",
            MyteIntent::Suck => "SUCK_MOVE",
        }
    }
}

/// Pick the next Myte intent.
///   - First turn (no `last_intent`): conditional on `slot`. The C#
///     INIT state branches on `Creature.SlotName == "first"` →
///     Toxic, `"second"` → Suck. Any other slot defaults to Toxic
///     (the more common starting branch — C# wouldn't hit this in
///     practice since MytesNormal only uses "first"/"second").
///   - Subsequent turns: FollowUpState chain Toxic → Bite → Suck →
///     Toxic → … (cycle).
pub fn pick_myte_intent(
    last_intent: Option<MyteIntent>,
    slot: &str,
) -> MyteIntent {
    match last_intent {
        None => match slot {
            "second" => MyteIntent::Suck,
            _ => MyteIntent::Toxic,
        },
        Some(MyteIntent::Toxic) => MyteIntent::Bite,
        Some(MyteIntent::Bite) => MyteIntent::Suck,
        Some(MyteIntent::Suck) => MyteIntent::Toxic,
    }
}

/// Execute one Myte move's payload. Mirrors C# Myte's per-move handlers
/// (ToxicMove / BiteMove / SuckMove), minus audio/animation.
pub fn execute_myte_move(
    cs: &mut CombatState,
    myte_idx: usize,
    target_player_idx: usize,
    intent: MyteIntent,
) {
    let attacker = (CombatSide::Enemy, myte_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        MyteIntent::Toxic => {
            // C# uses CardPileCmd.AddToCombatAndPreview<Toxic>(targets,
            // PileType.Hand, 2, …). Each Toxic is a Status card that
            // self-damages 5 at end-of-turn-in-hand (deferred).
            for _ in 0..MYTE_TOXIC_COUNT {
                cs.add_card_to_pile(target_player_idx, "Toxic", 0, PileType::Hand);
            }
        }
        MyteIntent::Bite => {
            cs.deal_damage(attacker, player, MYTE_BITE_DAMAGE, ValueProp::MOVE);
        }
        MyteIntent::Suck => {
            cs.deal_damage(attacker, player, MYTE_SUCK_DAMAGE, ValueProp::MOVE);
            cs.apply_power(
                CombatSide::Enemy,
                myte_idx,
                "StrengthPower",
                MYTE_SUCK_STRENGTH_GAIN,
            );
        }
    }
}

// ---------- Monster intent: Nibbit -------------------------------------
//
// Reflects C# `Nibbit.GenerateMoveStateMachine`:
//   INIT: ConditionalBranchState gated on per-encounter flags
//     `IsAlone` and `IsFront`:
//       - if IsAlone:  start at BUTT_MOVE
//       - else if !IsFront: start at HISS_MOVE
//       - else (IsFront):   start at SLICE_MOVE
//   Cycle (FollowUpState chain, no RNG):
//     Butt → Slice → Hiss → Butt → …
//
// Deterministic — no RNG needed for intent selection. IsAlone /
// IsFront are caller-provided booleans; in C# they're mutable fields
// on the Nibbit monster set by encounter setup. For NibbitsNormal
// (single Nibbit in "back" slot): is_alone=true, is_front=false.
//
// A0 values per `GetValueIfAscension(level, ascended, fallback)`:
//   - Butt damage: 12 (A0) / 13 (DeadlyEnemies)
//   - Slice damage: 6 / 7
//   - Slice block: 5 / 6 (ToughEnemies)
//   - Hiss strength gain: 2 / 3 (DeadlyEnemies)

const NIBBIT_BUTT_DAMAGE: i32 = 12;
const NIBBIT_SLICE_DAMAGE: i32 = 6;
const NIBBIT_SLICE_BLOCK: i32 = 5;
const NIBBIT_HISS_STRENGTH_GAIN: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum NibbitIntent {
    Butt,
    Slice,
    Hiss,
}

impl NibbitIntent {
    pub fn id(self) -> &'static str {
        match self {
            NibbitIntent::Butt => "BUTT_MOVE",
            NibbitIntent::Slice => "SLICE_MOVE",
            NibbitIntent::Hiss => "HISS_MOVE",
        }
    }
}

/// Pick Nibbit's next intent.
///   - First turn (no `last_intent`): use `is_alone` and `is_front`
///     per the C# INIT branch table.
///   - Subsequent turns: deterministic cycle Butt → Slice → Hiss.
pub fn pick_nibbit_intent(
    last_intent: Option<NibbitIntent>,
    is_alone: bool,
    is_front: bool,
) -> NibbitIntent {
    match last_intent {
        None => {
            if is_alone {
                NibbitIntent::Butt
            } else if is_front {
                NibbitIntent::Slice
            } else {
                NibbitIntent::Hiss
            }
        }
        Some(NibbitIntent::Butt) => NibbitIntent::Slice,
        Some(NibbitIntent::Slice) => NibbitIntent::Hiss,
        Some(NibbitIntent::Hiss) => NibbitIntent::Butt,
    }
}

/// Execute one Nibbit move's payload. Mirrors C# Nibbit per-move
/// handlers (ButtMove / SliceMove / HissMove) minus audio/animation.
pub fn execute_nibbit_move(
    cs: &mut CombatState,
    nibbit_idx: usize,
    target_player_idx: usize,
    intent: NibbitIntent,
) {
    let attacker = (CombatSide::Enemy, nibbit_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        NibbitIntent::Butt => {
            cs.deal_damage(attacker, player, NIBBIT_BUTT_DAMAGE, ValueProp::MOVE);
        }
        NibbitIntent::Slice => {
            cs.deal_damage(
                attacker,
                player,
                NIBBIT_SLICE_DAMAGE,
                ValueProp::MOVE,
            );
            cs.gain_block(
                CombatSide::Enemy,
                nibbit_idx,
                NIBBIT_SLICE_BLOCK,
            );
        }
        NibbitIntent::Hiss => {
            cs.apply_power(
                CombatSide::Enemy,
                nibbit_idx,
                "StrengthPower",
                NIBBIT_HISS_STRENGTH_GAIN,
            );
        }
    }
}

// ---------- Monster intent: FlailKnight --------------------------------
//
// Reflects C# `FlailKnight.GenerateMoveStateMachine`:
//   Start state: RAM_MOVE (one-shot init, no INIT ConditionalBranch).
//   Subsequent: RandomBranchState pick across:
//     - WarChant (weight 1, CannotRepeat)
//     - Flail    (weight 2)
//     - Ram      (weight 2)
//   When last_intent == WarChant, WarChant is excluded (CannotRepeat).
//   Pick uses Rng.NextFloat(total) + subtract-and-compare iteration.
//
// A0 payloads:
//   - WarChant: +3 self-Strength (const)
//   - Flail:    9 damage × 2 hits (DeadlyEnemies: 10)
//   - Ram:      15 damage (DeadlyEnemies: 17)

const FLAIL_KNIGHT_WAR_CHANT_STRENGTH: i32 = 3;
const FLAIL_KNIGHT_FLAIL_DAMAGE: i32 = 9;
const FLAIL_KNIGHT_FLAIL_HITS: i32 = 2;
const FLAIL_KNIGHT_RAM_DAMAGE: i32 = 15;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FlailKnightIntent {
    WarChant,
    Flail,
    Ram,
}

impl FlailKnightIntent {
    pub fn id(self) -> &'static str {
        match self {
            FlailKnightIntent::WarChant => "WAR_CHANT",
            FlailKnightIntent::Flail => "FLAIL_MOVE",
            FlailKnightIntent::Ram => "RAM_MOVE",
        }
    }
}

/// Pick FlailKnight's next intent. First turn: Ram. Subsequent:
/// weighted-random across {WarChant 1, Flail 2, Ram 2}, with
/// WarChant excluded when it was the last intent (C# CannotRepeat).
pub fn pick_flail_knight_intent(
    rng: &mut Rng,
    last_intent: Option<FlailKnightIntent>,
) -> FlailKnightIntent {
    if last_intent.is_none() {
        return FlailKnightIntent::Ram;
    }
    let war_chant_blocked = matches!(last_intent, Some(FlailKnightIntent::WarChant));
    let w_war_chant: f32 = if war_chant_blocked { 0.0 } else { 1.0 };
    let w_flail: f32 = 2.0;
    let w_ram: f32 = 2.0;
    let total = w_war_chant + w_flail + w_ram;
    let mut roll = rng.next_float(total);
    // Iteration order matches C#'s RandomBranchState.States list order
    // (WarChant added first, then Flail, then Ram).
    if !war_chant_blocked {
        roll -= w_war_chant;
        if roll <= 0.0 {
            return FlailKnightIntent::WarChant;
        }
    }
    roll -= w_flail;
    if roll <= 0.0 {
        return FlailKnightIntent::Flail;
    }
    // Last branch — math guarantees roll - w_ram <= 0 given total bound.
    FlailKnightIntent::Ram
}

/// Execute one FlailKnight move's payload. Mirrors C# WarChantMove /
/// FlailMove / RamMove minus audio/animation.
pub fn execute_flail_knight_move(
    cs: &mut CombatState,
    knight_idx: usize,
    target_player_idx: usize,
    intent: FlailKnightIntent,
) {
    let attacker = (CombatSide::Enemy, knight_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        FlailKnightIntent::WarChant => {
            cs.apply_power(
                CombatSide::Enemy,
                knight_idx,
                "StrengthPower",
                FLAIL_KNIGHT_WAR_CHANT_STRENGTH,
            );
        }
        FlailKnightIntent::Flail => {
            for _ in 0..FLAIL_KNIGHT_FLAIL_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    FLAIL_KNIGHT_FLAIL_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        FlailKnightIntent::Ram => {
            cs.deal_damage(
                attacker,
                player,
                FLAIL_KNIGHT_RAM_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: OwlMagistrate ------------------------------
//
// Reflects C# `OwlMagistrate.GenerateMoveStateMachine`:
//   Init: MAGISTRATE_SCRUTINY.
//   Cycle: Scrutiny → PeckAssault → JudicialFlight → Verdict →
//          Scrutiny → … (deterministic, no RNG).
//
// IsFlying flag toggles on JudicialFlight, off on Verdict — purely
// animation/sfx in C#. Not tracked here. SoarPower is the gameplay
// effect (×0.50 incoming powered damage), wired into
// power_multiplicative_target.
//
// A0 payloads:
//   - Scrutiny:       16 damage (DeadlyEnemies: 17)
//   - PeckAssault:    4 damage × 6 hits (const)
//   - JudicialFlight: apply SoarPower(1) to self
//   - Verdict:        33 damage (DeadlyEnemies: 36) + 4 Vulnerable
//                     on player; remove SoarPower from self

const OWL_MAGISTRATE_SCRUTINY_DAMAGE: i32 = 16;
const OWL_MAGISTRATE_PECK_DAMAGE: i32 = 4;
const OWL_MAGISTRATE_PECK_HITS: i32 = 6;
const OWL_MAGISTRATE_VERDICT_DAMAGE: i32 = 33;
const OWL_MAGISTRATE_VERDICT_VULN: i32 = 4;
const OWL_MAGISTRATE_SOAR_AMOUNT: i32 = 1;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OwlMagistrateIntent {
    Scrutiny,
    PeckAssault,
    JudicialFlight,
    Verdict,
}

impl OwlMagistrateIntent {
    pub fn id(self) -> &'static str {
        match self {
            OwlMagistrateIntent::Scrutiny => "MAGISTRATE_SCRUTINY",
            OwlMagistrateIntent::PeckAssault => "PECK_ASSAULT",
            OwlMagistrateIntent::JudicialFlight => "JUDICIAL_FLIGHT",
            OwlMagistrateIntent::Verdict => "VERDICT",
        }
    }
}

pub fn pick_owl_magistrate_intent(
    last_intent: Option<OwlMagistrateIntent>,
) -> OwlMagistrateIntent {
    match last_intent {
        None => OwlMagistrateIntent::Scrutiny,
        Some(OwlMagistrateIntent::Scrutiny) => OwlMagistrateIntent::PeckAssault,
        Some(OwlMagistrateIntent::PeckAssault) => OwlMagistrateIntent::JudicialFlight,
        Some(OwlMagistrateIntent::JudicialFlight) => OwlMagistrateIntent::Verdict,
        Some(OwlMagistrateIntent::Verdict) => OwlMagistrateIntent::Scrutiny,
    }
}

pub fn execute_owl_magistrate_move(
    cs: &mut CombatState,
    owl_idx: usize,
    target_player_idx: usize,
    intent: OwlMagistrateIntent,
) {
    let attacker = (CombatSide::Enemy, owl_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        OwlMagistrateIntent::Scrutiny => {
            cs.deal_damage(
                attacker,
                player,
                OWL_MAGISTRATE_SCRUTINY_DAMAGE,
                ValueProp::MOVE,
            );
        }
        OwlMagistrateIntent::PeckAssault => {
            for _ in 0..OWL_MAGISTRATE_PECK_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    OWL_MAGISTRATE_PECK_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        OwlMagistrateIntent::JudicialFlight => {
            cs.apply_power(
                CombatSide::Enemy,
                owl_idx,
                "SoarPower",
                OWL_MAGISTRATE_SOAR_AMOUNT,
            );
        }
        OwlMagistrateIntent::Verdict => {
            cs.deal_damage(
                attacker,
                player,
                OWL_MAGISTRATE_VERDICT_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "VulnerablePower",
                OWL_MAGISTRATE_VERDICT_VULN,
            );
            // Remove SoarPower from self (Single-stack, can't go
            // negative via apply_power; use the explicit remover).
            cs.remove_power(CombatSide::Enemy, owl_idx, "SoarPower");
        }
    }
}

// ---------- Monster intent: WaterfallGiant (boss) ----------------------
//
// Reflects C# `WaterfallGiant.GenerateMoveStateMachine`. Main 6-state
// chain: Pressurize → Stomp → Ram → Siphon → PressureGun → PressureUp
// → Stomp (loop). C# also has an AboutToBlow → Explode death-blow
// path triggered by SteamEruptionPower hitting a threshold — both
// deferred since SteamEruption's accumulation hook isn't ported. Each
// move just deals damage / heals / applies the player debuffs; the
// SteamEruption(3) tick is skipped.
//
// A0 payloads:
//   - Pressurize:  no-op (applies SteamEruption(15) in C# — skipped)
//   - Stomp:       15 dmg + 1 Weak on player (DeadlyEnemies: 16)
//   - Ram:         10 dmg (DeadlyEnemies: 11)
//   - Siphon:      heal 15 * playercount (15 self solo)
//   - PressureGun: starts 20 dmg, +5/use (DeadlyEnemies base: 23)
//   - PressureUp:  13 dmg (DeadlyEnemies: 14)

const WATERFALL_STOMP_DAMAGE: i32 = 15;
const WATERFALL_STOMP_WEAK: i32 = 1;
const WATERFALL_RAM_DAMAGE: i32 = 10;
const WATERFALL_SIPHON_HEAL: i32 = 15;
const WATERFALL_PRESSURE_GUN_BASE: i32 = 20;
const WATERFALL_PRESSURE_GUN_INCREASE: i32 = 5;
const WATERFALL_PRESSURE_UP_DAMAGE: i32 = 13;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WaterfallGiantIntent {
    Pressurize,
    Stomp,
    Ram,
    Siphon,
    PressureGun,
    PressureUp,
}

impl WaterfallGiantIntent {
    pub fn id(self) -> &'static str {
        match self {
            WaterfallGiantIntent::Pressurize => "PRESSURIZE_MOVE",
            WaterfallGiantIntent::Stomp => "STOMP_MOVE",
            WaterfallGiantIntent::Ram => "RAM_MOVE",
            WaterfallGiantIntent::Siphon => "SIPHON_MOVE",
            WaterfallGiantIntent::PressureGun => "PRESSURE_GUN_MOVE",
            WaterfallGiantIntent::PressureUp => "PRESSURE_UP_MOVE",
        }
    }
}

pub fn pick_waterfall_giant_intent(
    last_intent: Option<WaterfallGiantIntent>,
) -> WaterfallGiantIntent {
    match last_intent {
        None => WaterfallGiantIntent::Pressurize,
        Some(WaterfallGiantIntent::Pressurize) => WaterfallGiantIntent::Stomp,
        Some(WaterfallGiantIntent::Stomp) => WaterfallGiantIntent::Ram,
        Some(WaterfallGiantIntent::Ram) => WaterfallGiantIntent::Siphon,
        Some(WaterfallGiantIntent::Siphon) => WaterfallGiantIntent::PressureGun,
        Some(WaterfallGiantIntent::PressureGun) => WaterfallGiantIntent::PressureUp,
        Some(WaterfallGiantIntent::PressureUp) => WaterfallGiantIntent::Stomp,
    }
}

pub fn execute_waterfall_giant_move(
    cs: &mut CombatState,
    giant_idx: usize,
    target_player_idx: usize,
    intent: WaterfallGiantIntent,
) {
    let attacker = (CombatSide::Enemy, giant_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        WaterfallGiantIntent::Pressurize => {
            // No-op — SteamEruptionPower stacking deferred.
        }
        WaterfallGiantIntent::Stomp => {
            cs.deal_damage(
                attacker,
                player,
                WATERFALL_STOMP_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "WeakPower",
                WATERFALL_STOMP_WEAK,
            );
        }
        WaterfallGiantIntent::Ram => {
            cs.deal_damage(
                attacker,
                player,
                WATERFALL_RAM_DAMAGE,
                ValueProp::MOVE,
            );
        }
        WaterfallGiantIntent::Siphon => {
            cs.heal(CombatSide::Enemy, giant_idx, WATERFALL_SIPHON_HEAL);
        }
        WaterfallGiantIntent::PressureGun => {
            // PressureGun damage scales by +5 each use. Tracked per
            // monster via the existing counter map. First use deals
            // 20; second 25; etc.
            let extra = cs.enemies[giant_idx]
                .monster
                .as_ref()
                .map(|m| m.counter("pressure_gun_uses"))
                .unwrap_or(0);
            let dmg = WATERFALL_PRESSURE_GUN_BASE
                + WATERFALL_PRESSURE_GUN_INCREASE * extra;
            cs.deal_damage(attacker, player, dmg, ValueProp::MOVE);
            if let Some(ms) = cs.enemies[giant_idx].monster.as_mut() {
                ms.add_counter("pressure_gun_uses", 1);
            }
        }
        WaterfallGiantIntent::PressureUp => {
            cs.deal_damage(
                attacker,
                player,
                WATERFALL_PRESSURE_UP_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: TwoTailedRat -------------------------------
//
// Reflects C# `TwoTailedRat.GenerateMoveStateMachine`. Init via
// StarterMoveIndex ConditionalBranch (slot 0/1/2 → Scratch/DiseaseBite/
// Screech respectively). Thereafter RandomBranch over the 4 moves
// (Scratch, DiseaseBite, Screech, CallForBackup) with CannotRepeat
// + weight-modifying predicates based on CanSummon and per-rat
// CallForBackupCount.
//
// Simplified port: skip CallForBackup entirely (treat as no-op,
// summon system deferred). Cycle Scratch ↔ DiseaseBite ↔ Screech
// uniformly random with CannotRepeat. Init still slot-derived.
//
// A0 payloads:
//   - Scratch:        8 damage (DeadlyEnemies: 9)
//   - DiseaseBite:    6 damage (DeadlyEnemies: 7); applies a Disease
//                     card affliction in C# — deferred (just damage)
//   - Screech:        applies 1 Weak (DebuffIntent) to player
//   - CallForBackup:  no-op (summon TwoTailedRat — deferred)

const TWO_TAILED_RAT_SCRATCH_DAMAGE: i32 = 8;
const TWO_TAILED_RAT_DISEASE_DAMAGE: i32 = 6;
const TWO_TAILED_RAT_SCREECH_WEAK: i32 = 1;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TwoTailedRatIntent {
    Scratch,
    DiseaseBite,
    Screech,
    CallForBackup,
}

impl TwoTailedRatIntent {
    pub fn id(self) -> &'static str {
        match self {
            TwoTailedRatIntent::Scratch => "SCRATCH_MOVE",
            TwoTailedRatIntent::DiseaseBite => "DISEASE_BITE_MOVE",
            TwoTailedRatIntent::Screech => "SCREECH_MOVE",
            TwoTailedRatIntent::CallForBackup => "CALL_FOR_BACKUP_MOVE",
        }
    }
}

/// `slot` is 0-based (first=0, second=1, third=2). Init routes to
/// Scratch/DiseaseBite/Screech respectively; fourth+ defaults to
/// Scratch. Thereafter pick uniformly from {Scratch, DiseaseBite,
/// Screech} with CannotRepeat — CallForBackup is excluded.
pub fn pick_two_tailed_rat_intent(
    rng: &mut Rng,
    last_intent: Option<TwoTailedRatIntent>,
    slot: u8,
) -> TwoTailedRatIntent {
    if last_intent.is_none() {
        return match slot {
            0 => TwoTailedRatIntent::Scratch,
            1 => TwoTailedRatIntent::DiseaseBite,
            2 => TwoTailedRatIntent::Screech,
            _ => TwoTailedRatIntent::Scratch,
        };
    }
    let allowed: Vec<TwoTailedRatIntent> = [
        TwoTailedRatIntent::Scratch,
        TwoTailedRatIntent::DiseaseBite,
        TwoTailedRatIntent::Screech,
    ]
    .into_iter()
    .filter(|i| Some(*i) != last_intent)
    .collect();
    let pick = rng.next_int_range(0, allowed.len() as i32) as usize;
    allowed[pick]
}

pub fn execute_two_tailed_rat_move(
    cs: &mut CombatState,
    rat_idx: usize,
    target_player_idx: usize,
    intent: TwoTailedRatIntent,
) {
    let attacker = (CombatSide::Enemy, rat_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        TwoTailedRatIntent::Scratch => {
            cs.deal_damage(
                attacker,
                player,
                TWO_TAILED_RAT_SCRATCH_DAMAGE,
                ValueProp::MOVE,
            );
        }
        TwoTailedRatIntent::DiseaseBite => {
            cs.deal_damage(
                attacker,
                player,
                TWO_TAILED_RAT_DISEASE_DAMAGE,
                ValueProp::MOVE,
            );
            // C# additionally afflicts a card with Disease — deferred.
        }
        TwoTailedRatIntent::Screech => {
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "WeakPower",
                TWO_TAILED_RAT_SCREECH_WEAK,
            );
        }
        TwoTailedRatIntent::CallForBackup => {
            // No-op — summon system deferred.
        }
    }
}

// ---------- Monster intent: TheObscura ---------------------------------
//
// Reflects C# `TheObscura.GenerateMoveStateMachine`. Init: Illusion
// (summon — no-op here, summon system deferred). Thereafter
// RandomBranch over {PiercingGaze, Wail, HardeningStrike} with
// CannotRepeat.
//
// A0 payloads:
//   - Illusion:        no-op (would summon Illusion creature in C#)
//   - PiercingGaze:    10 damage (DeadlyEnemies: 11)
//   - Wail:            +3 Strength to teammates. Solo TheObscura
//                      encounter has no teammates → no-op.
//   - HardeningStrike: 6 damage + 6 block (DeadlyEnemies: 7 / 7)

const OBSCURA_PIERCING_GAZE_DAMAGE: i32 = 10;
const OBSCURA_WAIL_STRENGTH: i32 = 3;
const OBSCURA_HARDENING_STRIKE_DAMAGE: i32 = 6;
const OBSCURA_HARDENING_STRIKE_BLOCK: i32 = 6;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TheObscuraIntent {
    Illusion,
    PiercingGaze,
    Wail,
    HardeningStrike,
}

impl TheObscuraIntent {
    pub fn id(self) -> &'static str {
        match self {
            TheObscuraIntent::Illusion => "ILLUSION_MOVE",
            TheObscuraIntent::PiercingGaze => "PIERCING_GAZE_MOVE",
            TheObscuraIntent::Wail => "SAIL_MOVE",
            TheObscuraIntent::HardeningStrike => "HARDENING_STRIKE_MOVE",
        }
    }
}

pub fn pick_the_obscura_intent(
    rng: &mut Rng,
    last_intent: Option<TheObscuraIntent>,
) -> TheObscuraIntent {
    if last_intent.is_none() {
        return TheObscuraIntent::Illusion;
    }
    let allowed: Vec<TheObscuraIntent> = [
        TheObscuraIntent::PiercingGaze,
        TheObscuraIntent::Wail,
        TheObscuraIntent::HardeningStrike,
    ]
    .into_iter()
    .filter(|i| Some(*i) != last_intent)
    .collect();
    let pick = rng.next_int_range(0, allowed.len() as i32) as usize;
    allowed[pick]
}

pub fn execute_the_obscura_move(
    cs: &mut CombatState,
    ob_idx: usize,
    target_player_idx: usize,
    intent: TheObscuraIntent,
) {
    let attacker = (CombatSide::Enemy, ob_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        TheObscuraIntent::Illusion => {
            // No-op — summon system deferred.
        }
        TheObscuraIntent::PiercingGaze => {
            cs.deal_damage(
                attacker,
                player,
                OBSCURA_PIERCING_GAZE_DAMAGE,
                ValueProp::MOVE,
            );
        }
        TheObscuraIntent::Wail => {
            // Apply Strength to every other living enemy
            // (teammates). Solo Obscura encounters have none; the
            // C# call evaluates to empty target set.
            let n = cs.enemies.len();
            for i in 0..n {
                if i == ob_idx {
                    continue;
                }
                if cs.enemies[i].current_hp > 0 {
                    cs.apply_power(
                        CombatSide::Enemy,
                        i,
                        "StrengthPower",
                        OBSCURA_WAIL_STRENGTH,
                    );
                }
            }
        }
        TheObscuraIntent::HardeningStrike => {
            cs.deal_damage(
                attacker,
                player,
                OBSCURA_HARDENING_STRIKE_DAMAGE,
                ValueProp::MOVE,
            );
            cs.gain_block(
                CombatSide::Enemy,
                ob_idx,
                OBSCURA_HARDENING_STRIKE_BLOCK,
            );
        }
    }
}

// ---------- Monster intent: LivingFog ----------------------------------
//
// Reflects C# `LivingFog.GenerateMoveStateMachine`. Init: AdvancedGas.
// Chain AdvancedGas → Bloat → SuperGas → Bloat → SuperGas → … (2-state
// loop after init).
//
// A0 payloads:
//   - AdvancedGas: 8 dmg + apply SmoggyPower(1) on player [marker —
//                  C# behavior afflicts cards / draws extras;
//                  deferred]
//   - Bloat:       5 dmg + summon LivingFog minion (summon deferred,
//                  damage portion ported)
//   - SuperGas:    8 damage

const LIVING_FOG_ADVANCED_GAS_DAMAGE: i32 = 8;
const LIVING_FOG_SMOGGY_AMOUNT: i32 = 1;
const LIVING_FOG_BLOAT_DAMAGE: i32 = 5;
const LIVING_FOG_SUPER_GAS_DAMAGE: i32 = 8;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LivingFogIntent {
    AdvancedGas,
    Bloat,
    SuperGas,
}

impl LivingFogIntent {
    pub fn id(self) -> &'static str {
        match self {
            LivingFogIntent::AdvancedGas => "ADVANCED_GAS_MOVE",
            LivingFogIntent::Bloat => "BLOAT_MOVE",
            LivingFogIntent::SuperGas => "SUPER_GAS_BLAST_MOVE",
        }
    }
}

pub fn pick_living_fog_intent(
    last_intent: Option<LivingFogIntent>,
) -> LivingFogIntent {
    match last_intent {
        None => LivingFogIntent::AdvancedGas,
        Some(LivingFogIntent::AdvancedGas) => LivingFogIntent::Bloat,
        Some(LivingFogIntent::Bloat) => LivingFogIntent::SuperGas,
        Some(LivingFogIntent::SuperGas) => LivingFogIntent::Bloat,
    }
}

pub fn execute_living_fog_move(
    cs: &mut CombatState,
    fog_idx: usize,
    target_player_idx: usize,
    intent: LivingFogIntent,
) {
    let attacker = (CombatSide::Enemy, fog_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        LivingFogIntent::AdvancedGas => {
            cs.deal_damage(
                attacker,
                player,
                LIVING_FOG_ADVANCED_GAS_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "SmoggyPower",
                LIVING_FOG_SMOGGY_AMOUNT,
            );
        }
        LivingFogIntent::Bloat => {
            cs.deal_damage(
                attacker,
                player,
                LIVING_FOG_BLOAT_DAMAGE,
                ValueProp::MOVE,
            );
            // Summon LivingFog minion — deferred.
        }
        LivingFogIntent::SuperGas => {
            cs.deal_damage(
                attacker,
                player,
                LIVING_FOG_SUPER_GAS_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: Fabricator ---------------------------------
//
// Reflects C# `Fabricator.GenerateMoveStateMachine`:
//   Init: ConditionalBranch(CanFabricate → RandomBranch(Fabricate,
//   FabricatingStrike); else Disintegrate). All FollowUps return
//   to the same conditional branch.
//
// CanFabricate = (alive teammates < 4). Without summon system,
// teammate count stays at 0, so CanFabricate is always true and
// Disintegrate is unreachable. Fabricate summons DefensiveBot +
// AggroBot — both deferred (no-op). FabricatingStrike still deals
// damage; its summon is skipped too.
//
// A0 payloads:
//   - Fabricate:         no-op (summon DefensiveBot + AggroBot
//                        deferred)
//   - FabricatingStrike: 18 dmg (DeadlyEnemies: 21); the summon
//                        portion deferred
//   - Disintegrate:      11 dmg (DeadlyEnemies: 13) — unreachable
//                        in current port

const FABRICATOR_FABRICATING_STRIKE_DAMAGE: i32 = 18;
const FABRICATOR_DISINTEGRATE_DAMAGE: i32 = 11;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FabricatorIntent {
    Fabricate,
    FabricatingStrike,
    Disintegrate,
}

impl FabricatorIntent {
    pub fn id(self) -> &'static str {
        match self {
            FabricatorIntent::Fabricate => "FABRICATE_MOVE",
            FabricatorIntent::FabricatingStrike => "FABRICATING_STRIKE_MOVE",
            FabricatorIntent::Disintegrate => "DISINTEGRATE_MOVE",
        }
    }
}

pub fn pick_fabricator_intent(
    rng: &mut Rng,
    last_intent: Option<FabricatorIntent>,
    can_fabricate: bool,
) -> FabricatorIntent {
    let _ = last_intent; // Move repeats are allowed (CanRepeatForever).
    if !can_fabricate {
        return FabricatorIntent::Disintegrate;
    }
    // 50/50 between the two fabricate-class moves.
    if rng.next_int_range(0, 2) == 0 {
        FabricatorIntent::Fabricate
    } else {
        FabricatorIntent::FabricatingStrike
    }
}

pub fn execute_fabricator_move(
    cs: &mut CombatState,
    fab_idx: usize,
    target_player_idx: usize,
    intent: FabricatorIntent,
) {
    let attacker = (CombatSide::Enemy, fab_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        FabricatorIntent::Fabricate => {
            // No-op — would summon DefensiveBot + AggroBot in C#.
        }
        FabricatorIntent::FabricatingStrike => {
            cs.deal_damage(
                attacker,
                player,
                FABRICATOR_FABRICATING_STRIKE_DAMAGE,
                ValueProp::MOVE,
            );
            // Summon AggroBot omitted (deferred).
        }
        FabricatorIntent::Disintegrate => {
            cs.deal_damage(
                attacker,
                player,
                FABRICATOR_DISINTEGRATE_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: Doormaker (boss) ---------------------------
//
// Reflects C# `Doormaker.GenerateMoveStateMachine`. Init:
// DramaticOpen. Chain DramaticOpen → Hunger → Scrutiny → Grasp →
// Hunger (3-state loop after init).
//
// DramaticOpen in C# is the visual "door opens to reveal the
// Doormaker" transformation — restores max HP, removes powers,
// applies HungerPower. Encounter spawns a single Doormaker
// creature directly (.run monster_ids has only "MONSTER.DOORMAKER"),
// so the visual phase swap doesn't need a separate model. The
// per-move HungerPower / ScrutinyPower / GraspPower markers
// (used in C# for visuals + state-tracking) are skipped — the
// state machine reads only last_intent, which is enough.
//
// A0 payloads:
//   - DramaticOpen: no-op (visual reveal in C#)
//   - Hunger:       30 damage (DeadlyEnemies: 35)
//   - Scrutiny:     24 damage (DeadlyEnemies: 26)
//   - Grasp:        10 damage × 2 hits + 3 self-Strength
//                   (DeadlyEnemies: 11 / 4)

const DOORMAKER_HUNGER_DAMAGE: i32 = 30;
const DOORMAKER_SCRUTINY_DAMAGE: i32 = 24;
const DOORMAKER_GRASP_DAMAGE: i32 = 10;
const DOORMAKER_GRASP_HITS: i32 = 2;
const DOORMAKER_GRASP_STRENGTH: i32 = 3;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DoormakerIntent {
    DramaticOpen,
    Hunger,
    Scrutiny,
    Grasp,
}

impl DoormakerIntent {
    pub fn id(self) -> &'static str {
        match self {
            DoormakerIntent::DramaticOpen => "DRAMATIC_OPEN_MOVE",
            DoormakerIntent::Hunger => "HUNGER_MOVE",
            DoormakerIntent::Scrutiny => "SCRUTINY_MOVE",
            DoormakerIntent::Grasp => "GRASP_MOVE",
        }
    }
}

pub fn pick_doormaker_intent(
    last_intent: Option<DoormakerIntent>,
) -> DoormakerIntent {
    match last_intent {
        None => DoormakerIntent::DramaticOpen,
        Some(DoormakerIntent::DramaticOpen) => DoormakerIntent::Hunger,
        Some(DoormakerIntent::Hunger) => DoormakerIntent::Scrutiny,
        Some(DoormakerIntent::Scrutiny) => DoormakerIntent::Grasp,
        Some(DoormakerIntent::Grasp) => DoormakerIntent::Hunger,
    }
}

pub fn execute_doormaker_move(
    cs: &mut CombatState,
    doormaker_idx: usize,
    target_player_idx: usize,
    intent: DoormakerIntent,
) {
    let attacker = (CombatSide::Enemy, doormaker_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        DoormakerIntent::DramaticOpen => {
            // No-op — C# transforms visuals + restores HP + swaps
            // to HungerPower. Encounter spawns Doormaker directly
            // at full HP so the transform is purely cosmetic here.
        }
        DoormakerIntent::Hunger => {
            cs.deal_damage(
                attacker,
                player,
                DOORMAKER_HUNGER_DAMAGE,
                ValueProp::MOVE,
            );
        }
        DoormakerIntent::Scrutiny => {
            cs.deal_damage(
                attacker,
                player,
                DOORMAKER_SCRUTINY_DAMAGE,
                ValueProp::MOVE,
            );
        }
        DoormakerIntent::Grasp => {
            for _ in 0..DOORMAKER_GRASP_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    DOORMAKER_GRASP_DAMAGE,
                    ValueProp::MOVE,
                );
            }
            cs.apply_power(
                CombatSide::Enemy,
                doormaker_idx,
                "StrengthPower",
                DOORMAKER_GRASP_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: LagavulinMatriarch -------------------------
//
// Reflects C# `LagavulinMatriarch.GenerateMoveStateMachine`:
//   Init: Sleep.
//   Sleep → branch:
//     - HasAsleep    → Sleep (loops while asleep)
//     - !HasAsleep   → Slash (woke up via Asleep removal)
//   Slash → Disembowel → Slash2 → SoulSiphon → Slash (loops awake).
//
// Spawn (AfterAddedToRoom): PlatingPower(12), AsleepPower(3).
// AsleepPower wakes either:
//   - on first unblocked-damage hit (fire_after_damage_received
//     hook above)
//   - at end of owner-turn-3 (tick_asleep_powers decrements; at 0
//     also strips Plating). Mirrors C# `BeforeTurnEndVeryEarly +
//     AfterTurnEnd`.
//
// A0 payloads:
//   - Sleep:      no-op
//   - Slash:      19 damage (DeadlyEnemies: 21)
//   - Slash2:     12 damage + 12 block (ToughEnemies: 14 block;
//                 DeadlyEnemies: 14 dmg)
//   - Disembowel: 9 damage × 2 hits (DeadlyEnemies: 10)
//   - SoulSiphon: -2 Strength + -2 Dexterity on player + +2 self
//                 Strength

const LAGAVULIN_PLATING_AMOUNT: i32 = 12;
const LAGAVULIN_ASLEEP_AMOUNT: i32 = 3;
const LAGAVULIN_SLASH_DAMAGE: i32 = 19;
const LAGAVULIN_SLASH2_DAMAGE: i32 = 12;
const LAGAVULIN_SLASH2_BLOCK: i32 = 12;
const LAGAVULIN_DISEMBOWEL_DAMAGE: i32 = 9;
const LAGAVULIN_DISEMBOWEL_HITS: i32 = 2;
const LAGAVULIN_SOUL_SIPHON_DEBUFF: i32 = 2;
const LAGAVULIN_SOUL_SIPHON_STRENGTH: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LagavulinMatriarchIntent {
    Sleep,
    Slash,
    Slash2,
    Disembowel,
    SoulSiphon,
}

impl LagavulinMatriarchIntent {
    pub fn id(self) -> &'static str {
        match self {
            LagavulinMatriarchIntent::Sleep => "SLEEP_MOVE",
            LagavulinMatriarchIntent::Slash => "SLASH_MOVE",
            LagavulinMatriarchIntent::Slash2 => "SLASH2_MOVE",
            LagavulinMatriarchIntent::Disembowel => "DISEMBOWEL_MOVE",
            LagavulinMatriarchIntent::SoulSiphon => "SOUL_SIPHON_MOVE",
        }
    }
}

pub fn lagavulin_matriarch_spawn(cs: &mut CombatState, lag_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        lag_idx,
        "PlatingPower",
        LAGAVULIN_PLATING_AMOUNT,
    );
    cs.apply_power(
        CombatSide::Enemy,
        lag_idx,
        "AsleepPower",
        LAGAVULIN_ASLEEP_AMOUNT,
    );
}

pub fn pick_lagavulin_matriarch_intent(
    last_intent: Option<LagavulinMatriarchIntent>,
    has_asleep: bool,
) -> LagavulinMatriarchIntent {
    if has_asleep {
        return LagavulinMatriarchIntent::Sleep;
    }
    match last_intent {
        // Just woke up (last was Sleep) or first awake move.
        None | Some(LagavulinMatriarchIntent::Sleep) => {
            LagavulinMatriarchIntent::Slash
        }
        Some(LagavulinMatriarchIntent::Slash) => {
            LagavulinMatriarchIntent::Disembowel
        }
        Some(LagavulinMatriarchIntent::Disembowel) => {
            LagavulinMatriarchIntent::Slash2
        }
        Some(LagavulinMatriarchIntent::Slash2) => {
            LagavulinMatriarchIntent::SoulSiphon
        }
        Some(LagavulinMatriarchIntent::SoulSiphon) => {
            LagavulinMatriarchIntent::Slash
        }
    }
}

pub fn execute_lagavulin_matriarch_move(
    cs: &mut CombatState,
    lag_idx: usize,
    target_player_idx: usize,
    intent: LagavulinMatriarchIntent,
) {
    let attacker = (CombatSide::Enemy, lag_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        LagavulinMatriarchIntent::Sleep => {
            // No-op — SleepIntent. The Asleep tick + first-damage
            // hook is what eventually wakes her.
        }
        LagavulinMatriarchIntent::Slash => {
            cs.deal_damage(
                attacker,
                player,
                LAGAVULIN_SLASH_DAMAGE,
                ValueProp::MOVE,
            );
        }
        LagavulinMatriarchIntent::Slash2 => {
            cs.deal_damage(
                attacker,
                player,
                LAGAVULIN_SLASH2_DAMAGE,
                ValueProp::MOVE,
            );
            cs.gain_block(
                CombatSide::Enemy,
                lag_idx,
                LAGAVULIN_SLASH2_BLOCK,
            );
        }
        LagavulinMatriarchIntent::Disembowel => {
            for _ in 0..LAGAVULIN_DISEMBOWEL_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    LAGAVULIN_DISEMBOWEL_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        LagavulinMatriarchIntent::SoulSiphon => {
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "StrengthPower",
                -LAGAVULIN_SOUL_SIPHON_DEBUFF,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "DexterityPower",
                -LAGAVULIN_SOUL_SIPHON_DEBUFF,
            );
            cs.apply_power(
                CombatSide::Enemy,
                lag_idx,
                "StrengthPower",
                LAGAVULIN_SOUL_SIPHON_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: HauntedShip --------------------------------
//
// Reflects C# `HauntedShip.GenerateMoveStateMachine`. Init: Haunt.
// All 4 moves transition to RandomBranch over {Ramming, Swipe,
// Stomp} with `MoveRepeatType.CannotRepeat` AND each branch gated
// on `RoundNumber % 2 != 0`. The round-parity gate is opaque (on
// even rounds NO branches are eligible — what C# does in that
// case isn't clear without runtime). We simplify: every turn after
// the Haunt init, pick uniformly random from the 3 attacks
// excluding the last-played one. Haunt only fires on init.
//
// A0 payloads:
//   - Haunt:        add 5 Dazed cards to player's discard
//   - RammingSpeed: 10 damage + 1 Weak (DeadlyEnemies: 11)
//   - Swipe:        13 damage (DeadlyEnemies: 14)
//   - Stomp:        4 damage × 3 hits (DeadlyEnemies: 5)

const HAUNTED_SHIP_HAUNT_DAZED: i32 = 5;
const HAUNTED_SHIP_RAMMING_DAMAGE: i32 = 10;
const HAUNTED_SHIP_RAMMING_WEAK: i32 = 1;
const HAUNTED_SHIP_SWIPE_DAMAGE: i32 = 13;
const HAUNTED_SHIP_STOMP_DAMAGE: i32 = 4;
const HAUNTED_SHIP_STOMP_HITS: i32 = 3;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum HauntedShipIntent {
    Haunt,
    RammingSpeed,
    Swipe,
    Stomp,
}

impl HauntedShipIntent {
    pub fn id(self) -> &'static str {
        match self {
            HauntedShipIntent::Haunt => "HAUNT_MOVE",
            HauntedShipIntent::RammingSpeed => "RAMMING_SPEED_MOVE",
            HauntedShipIntent::Swipe => "SWIPE_MOVE",
            HauntedShipIntent::Stomp => "STOMP_MOVE",
        }
    }
}

pub fn pick_haunted_ship_intent(
    rng: &mut Rng,
    last_intent: Option<HauntedShipIntent>,
) -> HauntedShipIntent {
    if last_intent.is_none() {
        return HauntedShipIntent::Haunt;
    }
    let allowed: Vec<HauntedShipIntent> = [
        HauntedShipIntent::RammingSpeed,
        HauntedShipIntent::Swipe,
        HauntedShipIntent::Stomp,
    ]
    .into_iter()
    .filter(|i| Some(*i) != last_intent)
    .collect();
    let pick = rng.next_int_range(0, allowed.len() as i32) as usize;
    allowed[pick]
}

pub fn execute_haunted_ship_move(
    cs: &mut CombatState,
    ship_idx: usize,
    target_player_idx: usize,
    intent: HauntedShipIntent,
) {
    let attacker = (CombatSide::Enemy, ship_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        HauntedShipIntent::Haunt => {
            for _ in 0..HAUNTED_SHIP_HAUNT_DAZED {
                cs.add_card_to_pile(
                    target_player_idx,
                    "Dazed",
                    0,
                    PileType::Discard,
                );
            }
        }
        HauntedShipIntent::RammingSpeed => {
            cs.deal_damage(
                attacker,
                player,
                HAUNTED_SHIP_RAMMING_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "WeakPower",
                HAUNTED_SHIP_RAMMING_WEAK,
            );
        }
        HauntedShipIntent::Swipe => {
            cs.deal_damage(
                attacker,
                player,
                HAUNTED_SHIP_SWIPE_DAMAGE,
                ValueProp::MOVE,
            );
        }
        HauntedShipIntent::Stomp => {
            for _ in 0..HAUNTED_SHIP_STOMP_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    HAUNTED_SHIP_STOMP_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
    }
}

// ---------- Monster intent: Queen (boss) -------------------------------
//
// Reflects C# `Queen.GenerateMoveStateMachine`:
//   Init: PuppetStrings.
//   Chain depends on whether the TorchHeadAmalgam teammate has died:
//     - PuppetStrings → YoureMine
//     - YoureMine → branch:
//         amalgam alive → BurnBrightForMe
//         amalgam dead  → OffWithYourHead
//     - BurnBrightForMe → branch:
//         amalgam alive → BurnBrightForMe (loops)
//         amalgam dead  → OffWithYourHead
//     - OffWithYourHead → Execution → Enrage → OffWithYourHead (loop)
//
// A0 payloads:
//   - PuppetStrings:   apply ChainsOfBindingPower(3) [marker] to player
//                      (per-card affliction-on-draw deferred)
//   - YoureMine:       apply 99 Frail + 99 Weak + 99 Vulnerable
//   - BurnBrightForMe: each living teammate gets +1 Strength; Queen
//                      gains 20 block
//   - OffWithYourHead: 3 dmg × 5 hits (DeadlyEnemies: 4)
//   - Execution:       15 damage (DeadlyEnemies: 18)
//   - Enrage:          +2 self-Strength

const QUEEN_CHAINS_AMOUNT: i32 = 3;
const QUEEN_YOURE_MINE_AMOUNT: i32 = 99;
const QUEEN_BURN_BRIGHT_BLOCK: i32 = 20;
const QUEEN_BURN_BRIGHT_TEAMMATE_STRENGTH: i32 = 1;
const QUEEN_OFF_WITH_HEAD_DAMAGE: i32 = 3;
const QUEEN_OFF_WITH_HEAD_HITS: i32 = 5;
const QUEEN_EXECUTION_DAMAGE: i32 = 15;
const QUEEN_ENRAGE_STRENGTH: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum QueenIntent {
    PuppetStrings,
    YoureMine,
    BurnBrightForMe,
    OffWithYourHead,
    Execution,
    Enrage,
}

impl QueenIntent {
    pub fn id(self) -> &'static str {
        match self {
            QueenIntent::PuppetStrings => "PUPPET_STRINGS_MOVE",
            QueenIntent::YoureMine => "YOUR_MINE_MOVE",
            QueenIntent::BurnBrightForMe => "BURN_BRIGHT_FOR_ME_MOVE",
            QueenIntent::OffWithYourHead => "OFF_WITH_YOUR_HEAD_MOVE",
            QueenIntent::Execution => "EXECUTION_MOVE",
            QueenIntent::Enrage => "ENRAGE_MOVE",
        }
    }
}

pub fn pick_queen_intent(
    last_intent: Option<QueenIntent>,
    amalgam_dead: bool,
) -> QueenIntent {
    match last_intent {
        None => QueenIntent::PuppetStrings,
        Some(QueenIntent::PuppetStrings) => QueenIntent::YoureMine,
        Some(QueenIntent::YoureMine) => {
            if amalgam_dead {
                QueenIntent::OffWithYourHead
            } else {
                QueenIntent::BurnBrightForMe
            }
        }
        Some(QueenIntent::BurnBrightForMe) => {
            if amalgam_dead {
                QueenIntent::OffWithYourHead
            } else {
                QueenIntent::BurnBrightForMe
            }
        }
        Some(QueenIntent::OffWithYourHead) => QueenIntent::Execution,
        Some(QueenIntent::Execution) => QueenIntent::Enrage,
        Some(QueenIntent::Enrage) => QueenIntent::OffWithYourHead,
    }
}

pub fn execute_queen_move(
    cs: &mut CombatState,
    queen_idx: usize,
    target_player_idx: usize,
    intent: QueenIntent,
) {
    let attacker = (CombatSide::Enemy, queen_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        QueenIntent::PuppetStrings => {
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "ChainsOfBindingPower",
                QUEEN_CHAINS_AMOUNT,
            );
        }
        QueenIntent::YoureMine => {
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "FrailPower",
                QUEEN_YOURE_MINE_AMOUNT,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "WeakPower",
                QUEEN_YOURE_MINE_AMOUNT,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "VulnerablePower",
                QUEEN_YOURE_MINE_AMOUNT,
            );
        }
        QueenIntent::BurnBrightForMe => {
            // Grant +1 Strength to every living teammate (= other
            // alive enemies). Queen herself doesn't get Strength —
            // C# uses `teammate != base.Creature` filter. Then Queen
            // gains 20 block.
            let n = cs.enemies.len();
            for i in 0..n {
                if i == queen_idx {
                    continue;
                }
                if cs.enemies[i].current_hp > 0 {
                    cs.apply_power(
                        CombatSide::Enemy,
                        i,
                        "StrengthPower",
                        QUEEN_BURN_BRIGHT_TEAMMATE_STRENGTH,
                    );
                }
            }
            cs.gain_block(
                CombatSide::Enemy,
                queen_idx,
                QUEEN_BURN_BRIGHT_BLOCK,
            );
        }
        QueenIntent::OffWithYourHead => {
            for _ in 0..QUEEN_OFF_WITH_HEAD_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    QUEEN_OFF_WITH_HEAD_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        QueenIntent::Execution => {
            cs.deal_damage(
                attacker,
                player,
                QUEEN_EXECUTION_DAMAGE,
                ValueProp::MOVE,
            );
        }
        QueenIntent::Enrage => {
            cs.apply_power(
                CombatSide::Enemy,
                queen_idx,
                "StrengthPower",
                QUEEN_ENRAGE_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: Crusher (KaiserCrabBoss left arm) ----------
//
// Reflects C# `Crusher.GenerateMoveStateMachine`:
//   Init: Thrash. Chain Thrash → EnlargingStrike → BugSting →
//   Adapt → GuardedStrike → Thrash (5-state loop).
//
// Spawn: BackAttackLeftPower(1), CrabRagePower(1). Both marker-only
// in this port — SurroundedPower's 1.5x bonus from back-side
// attackers is deferred (the marker on the player is set by
// Rocket spawn), and CrabRage's AfterDeath rage trigger (gain 6
// Strength + 99 block when teammate dies) is deferred.
//
// A0 payloads:
//   - Thrash:         12 damage (DeadlyEnemies: 14)
//   - EnlargingStrike: 4 damage (DeadlyEnemies: 4)
//   - BugSting:       6 damage × 2 hits + 2 Weak + 2 Frail
//                     (DeadlyEnemies: 7)
//   - Adapt:          +2 self-Strength (DeadlyEnemies: 3)
//   - GuardedStrike:  12 damage + 18 block (DeadlyEnemies: 14)

const CRUSHER_THRASH_DAMAGE: i32 = 12;
const CRUSHER_ENLARGING_DAMAGE: i32 = 4;
const CRUSHER_BUG_STING_DAMAGE: i32 = 6;
const CRUSHER_BUG_STING_HITS: i32 = 2;
const CRUSHER_BUG_STING_WEAK: i32 = 2;
const CRUSHER_BUG_STING_FRAIL: i32 = 2;
const CRUSHER_ADAPT_STRENGTH: i32 = 2;
const CRUSHER_GUARDED_DAMAGE: i32 = 12;
const CRUSHER_GUARDED_BLOCK: i32 = 18;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CrusherIntent {
    Thrash,
    EnlargingStrike,
    BugSting,
    Adapt,
    GuardedStrike,
}

impl CrusherIntent {
    pub fn id(self) -> &'static str {
        match self {
            CrusherIntent::Thrash => "THRASH_MOVE",
            CrusherIntent::EnlargingStrike => "ENLARGING_STRIKE_MOVE",
            CrusherIntent::BugSting => "BUG_STING_MOVE",
            CrusherIntent::Adapt => "ADAPT_MOVE",
            CrusherIntent::GuardedStrike => "GUARDED_STRIKE_MOVE",
        }
    }
}

pub fn pick_crusher_intent(last_intent: Option<CrusherIntent>) -> CrusherIntent {
    match last_intent {
        None => CrusherIntent::Thrash,
        Some(CrusherIntent::Thrash) => CrusherIntent::EnlargingStrike,
        Some(CrusherIntent::EnlargingStrike) => CrusherIntent::BugSting,
        Some(CrusherIntent::BugSting) => CrusherIntent::Adapt,
        Some(CrusherIntent::Adapt) => CrusherIntent::GuardedStrike,
        Some(CrusherIntent::GuardedStrike) => CrusherIntent::Thrash,
    }
}

pub fn execute_crusher_move(
    cs: &mut CombatState,
    crusher_idx: usize,
    target_player_idx: usize,
    intent: CrusherIntent,
) {
    let attacker = (CombatSide::Enemy, crusher_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        CrusherIntent::Thrash => {
            cs.deal_damage(
                attacker,
                player,
                CRUSHER_THRASH_DAMAGE,
                ValueProp::MOVE,
            );
        }
        CrusherIntent::EnlargingStrike => {
            cs.deal_damage(
                attacker,
                player,
                CRUSHER_ENLARGING_DAMAGE,
                ValueProp::MOVE,
            );
        }
        CrusherIntent::BugSting => {
            for _ in 0..CRUSHER_BUG_STING_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    CRUSHER_BUG_STING_DAMAGE,
                    ValueProp::MOVE,
                );
            }
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "WeakPower",
                CRUSHER_BUG_STING_WEAK,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "FrailPower",
                CRUSHER_BUG_STING_FRAIL,
            );
        }
        CrusherIntent::Adapt => {
            cs.apply_power(
                CombatSide::Enemy,
                crusher_idx,
                "StrengthPower",
                CRUSHER_ADAPT_STRENGTH,
            );
        }
        CrusherIntent::GuardedStrike => {
            cs.deal_damage(
                attacker,
                player,
                CRUSHER_GUARDED_DAMAGE,
                ValueProp::MOVE,
            );
            cs.gain_block(
                CombatSide::Enemy,
                crusher_idx,
                CRUSHER_GUARDED_BLOCK,
            );
        }
    }
}

// ---------- Monster intent: Rocket (KaiserCrabBoss right arm) ----------
//
// Reflects C# `Rocket.GenerateMoveStateMachine`:
//   Init: TargetingReticle. Chain TargetingReticle → PrecisionBeam
//   → ChargeUp → Laser → Recharge → TargetingReticle (loop).
//
// Spawn: SurroundedPower(1) on every player, BackAttackRightPower(1),
// CrabRagePower(1). All marker-only in this port (1.5x bonus and
// rage hooks deferred).
//
// A0 payloads:
//   - TargetingReticle: 3 damage (DeadlyEnemies: 4)
//   - PrecisionBeam:    18 damage (DeadlyEnemies: 20)
//   - ChargeUp:         +2 self-Strength (DeadlyEnemies: 3)
//   - Laser:            31 damage (DeadlyEnemies: 35)
//   - Recharge:         no-op (SleepIntent)

const ROCKET_TARGETING_DAMAGE: i32 = 3;
const ROCKET_PRECISION_DAMAGE: i32 = 18;
const ROCKET_CHARGE_STRENGTH: i32 = 2;
const ROCKET_LASER_DAMAGE: i32 = 31;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RocketIntent {
    TargetingReticle,
    PrecisionBeam,
    ChargeUp,
    Laser,
    Recharge,
}

impl RocketIntent {
    pub fn id(self) -> &'static str {
        match self {
            RocketIntent::TargetingReticle => "TARGETING_RETICLE_MOVE",
            RocketIntent::PrecisionBeam => "PRECISION_BEAM_MOVE",
            RocketIntent::ChargeUp => "CHARGE_UP_MOVE",
            RocketIntent::Laser => "LASER_MOVE",
            RocketIntent::Recharge => "RECHARGE_MOVE",
        }
    }
}

pub fn pick_rocket_intent(last_intent: Option<RocketIntent>) -> RocketIntent {
    match last_intent {
        None => RocketIntent::TargetingReticle,
        Some(RocketIntent::TargetingReticle) => RocketIntent::PrecisionBeam,
        Some(RocketIntent::PrecisionBeam) => RocketIntent::ChargeUp,
        Some(RocketIntent::ChargeUp) => RocketIntent::Laser,
        Some(RocketIntent::Laser) => RocketIntent::Recharge,
        Some(RocketIntent::Recharge) => RocketIntent::TargetingReticle,
    }
}

pub fn execute_rocket_move(
    cs: &mut CombatState,
    rocket_idx: usize,
    target_player_idx: usize,
    intent: RocketIntent,
) {
    let attacker = (CombatSide::Enemy, rocket_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        RocketIntent::TargetingReticle => {
            cs.deal_damage(
                attacker,
                player,
                ROCKET_TARGETING_DAMAGE,
                ValueProp::MOVE,
            );
        }
        RocketIntent::PrecisionBeam => {
            cs.deal_damage(
                attacker,
                player,
                ROCKET_PRECISION_DAMAGE,
                ValueProp::MOVE,
            );
        }
        RocketIntent::ChargeUp => {
            cs.apply_power(
                CombatSide::Enemy,
                rocket_idx,
                "StrengthPower",
                ROCKET_CHARGE_STRENGTH,
            );
        }
        RocketIntent::Laser => {
            cs.deal_damage(
                attacker,
                player,
                ROCKET_LASER_DAMAGE,
                ValueProp::MOVE,
            );
        }
        RocketIntent::Recharge => {
            // SleepIntent — Rocket recharges without acting.
        }
    }
}

// ---------- Monster intent: Ovicopter ----------------------------------
//
// Reflects C# `Ovicopter.GenerateMoveStateMachine`:
//   Init: LayEggs.
//   Chain: LayEggs → Smash → Tenderizer → conditional(CanLay →
//          LayEggs, !CanLay → NutritionalPaste) → Smash → … (loop).
//   CanLay = (alive teammates ≤ 3). NutritionalPaste → Smash.
//
// A0 payloads:
//   - LayEggs:          summon 3 ToughEgg minions. Skipped — summon
//                       system isn't ported; the move is a no-op.
//                       Without summoned eggs, CanLay always holds
//                       so the chain stays in LayEggs/Smash/
//                       Tenderizer and never falls to
//                       NutritionalPaste.
//   - Smash:            16 damage (DeadlyEnemies: 17)
//   - Tenderizer:       7 dmg + 2 Vulnerable on player
//                       (DeadlyEnemies: 8)
//   - NutritionalPaste: +3 self-Strength (DeadlyEnemies: 4) —
//                       included for completeness even though
//                       unreachable without summon system.

const OVICOPTER_SMASH_DAMAGE: i32 = 16;
const OVICOPTER_TENDERIZER_DAMAGE: i32 = 7;
const OVICOPTER_TENDERIZER_VULN: i32 = 2;
const OVICOPTER_NUTRITIONAL_STRENGTH: i32 = 3;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OvicopterIntent {
    LayEggs,
    Smash,
    Tenderizer,
    NutritionalPaste,
}

impl OvicopterIntent {
    pub fn id(self) -> &'static str {
        match self {
            OvicopterIntent::LayEggs => "LAY_EGGS_MOVE",
            OvicopterIntent::Smash => "SMASH_MOVE",
            OvicopterIntent::Tenderizer => "TENDERIZER_MOVE",
            OvicopterIntent::NutritionalPaste => "NUTRITIONAL_PASTE_MOVE",
        }
    }
}

/// Pick Ovicopter's next intent. `can_lay` flips the post-Tenderizer
/// branch — true → LayEggs (default since we don't summon, so the
/// alive-teammate count never grows past 0), false → NutritionalPaste.
pub fn pick_ovicopter_intent(
    last_intent: Option<OvicopterIntent>,
    can_lay: bool,
) -> OvicopterIntent {
    match last_intent {
        None => OvicopterIntent::LayEggs,
        Some(OvicopterIntent::LayEggs) => OvicopterIntent::Smash,
        Some(OvicopterIntent::NutritionalPaste) => OvicopterIntent::Smash,
        Some(OvicopterIntent::Smash) => OvicopterIntent::Tenderizer,
        Some(OvicopterIntent::Tenderizer) => {
            if can_lay {
                OvicopterIntent::LayEggs
            } else {
                OvicopterIntent::NutritionalPaste
            }
        }
    }
}

pub fn execute_ovicopter_move(
    cs: &mut CombatState,
    ovi_idx: usize,
    target_player_idx: usize,
    intent: OvicopterIntent,
) {
    let attacker = (CombatSide::Enemy, ovi_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        OvicopterIntent::LayEggs => {
            // No-op — summon system deferred. C# summons 3 ToughEgg
            // minions and applies MinionPower(1) to each.
        }
        OvicopterIntent::Smash => {
            cs.deal_damage(
                attacker,
                player,
                OVICOPTER_SMASH_DAMAGE,
                ValueProp::MOVE,
            );
        }
        OvicopterIntent::Tenderizer => {
            cs.deal_damage(
                attacker,
                player,
                OVICOPTER_TENDERIZER_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "VulnerablePower",
                OVICOPTER_TENDERIZER_VULN,
            );
        }
        OvicopterIntent::NutritionalPaste => {
            cs.apply_power(
                CombatSide::Enemy,
                ovi_idx,
                "StrengthPower",
                OVICOPTER_NUTRITIONAL_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: MagiKnight ---------------------------------
//
// Reflects C# `MagiKnight.GenerateMoveStateMachine`:
//   Init: FirstPowerShield.
//   Chain: PowerShield → Dampen → Spear → Prep → MagicBomb →
//          Spear → Prep → MagicBomb → … (3-state loop).
//
// A0 payloads:
//   - PowerShield: 6 dmg + 5 block (ToughEnemies: 9 block)
//   - Dampen:      apply DampenPower(1) to player (per-card-state
//                  downgrade hook deferred — applied as marker
//                  stack only)
//   - Spear:       10 damage (DeadlyEnemies: 11)
//   - Prep:        5 block (no attack)
//   - MagicBomb:   35 damage (DeadlyEnemies: 40)

const MAGI_KNIGHT_POWER_SHIELD_DAMAGE: i32 = 6;
const MAGI_KNIGHT_POWER_SHIELD_BLOCK: i32 = 5;
const MAGI_KNIGHT_SPEAR_DAMAGE: i32 = 10;
const MAGI_KNIGHT_PREP_BLOCK: i32 = 5;
const MAGI_KNIGHT_BOMB_DAMAGE: i32 = 35;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MagiKnightIntent {
    PowerShield,
    Dampen,
    Spear,
    Prep,
    MagicBomb,
}

impl MagiKnightIntent {
    pub fn id(self) -> &'static str {
        match self {
            MagiKnightIntent::PowerShield => "FIRST_POWER_SHIELD_MOVE",
            MagiKnightIntent::Dampen => "DAMPEN_MOVE",
            MagiKnightIntent::Spear => "RAM_MOVE",
            MagiKnightIntent::Prep => "PREP_MOVE",
            MagiKnightIntent::MagicBomb => "MAGIC_BOMB",
        }
    }
}

pub fn pick_magi_knight_intent(
    last_intent: Option<MagiKnightIntent>,
) -> MagiKnightIntent {
    match last_intent {
        None => MagiKnightIntent::PowerShield,
        Some(MagiKnightIntent::PowerShield) => MagiKnightIntent::Dampen,
        Some(MagiKnightIntent::Dampen) => MagiKnightIntent::Spear,
        Some(MagiKnightIntent::Spear) => MagiKnightIntent::Prep,
        Some(MagiKnightIntent::Prep) => MagiKnightIntent::MagicBomb,
        Some(MagiKnightIntent::MagicBomb) => MagiKnightIntent::Spear,
    }
}

pub fn execute_magi_knight_move(
    cs: &mut CombatState,
    knight_idx: usize,
    target_player_idx: usize,
    intent: MagiKnightIntent,
) {
    let attacker = (CombatSide::Enemy, knight_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        MagiKnightIntent::PowerShield => {
            cs.deal_damage(
                attacker,
                player,
                MAGI_KNIGHT_POWER_SHIELD_DAMAGE,
                ValueProp::MOVE,
            );
            cs.gain_block(
                CombatSide::Enemy,
                knight_idx,
                MAGI_KNIGHT_POWER_SHIELD_BLOCK,
            );
        }
        MagiKnightIntent::Dampen => {
            // C# Dampen downgrades all upgraded player cards and
            // ethereal-keywords them if HexPower is present. We apply
            // DampenPower as a marker stack and skip the card-level
            // mutation (per-card affliction state not modeled).
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "DampenPower",
                1,
            );
        }
        MagiKnightIntent::Spear => {
            cs.deal_damage(
                attacker,
                player,
                MAGI_KNIGHT_SPEAR_DAMAGE,
                ValueProp::MOVE,
            );
        }
        MagiKnightIntent::Prep => {
            cs.gain_block(CombatSide::Enemy, knight_idx, MAGI_KNIGHT_PREP_BLOCK);
        }
        MagiKnightIntent::MagicBomb => {
            cs.deal_damage(
                attacker,
                player,
                MAGI_KNIGHT_BOMB_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: SpectralKnight -----------------------------
//
// Reflects C# `SpectralKnight.GenerateMoveStateMachine`:
//   Init: Hex.
//   Chain: Hex → SoulSlash → RandomBranch(SoulSlash w/2 + SoulFlame
//          CannotRepeat). SoulFlame.FollowUp = RandomBranch.
//
// A0 payloads:
//   - Hex:       apply HexPower(2) to player (afflict-all-cards
//                hook deferred — marker stack only)
//   - SoulSlash: 15 damage (DeadlyEnemies: 17)
//   - SoulFlame: 3 damage × 3 hits (DeadlyEnemies: 4)

const SPECTRAL_KNIGHT_HEX_AMOUNT: i32 = 2;
const SPECTRAL_KNIGHT_SOUL_SLASH_DAMAGE: i32 = 15;
const SPECTRAL_KNIGHT_SOUL_FLAME_DAMAGE: i32 = 3;
const SPECTRAL_KNIGHT_SOUL_FLAME_HITS: i32 = 3;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SpectralKnightIntent {
    Hex,
    SoulSlash,
    SoulFlame,
}

impl SpectralKnightIntent {
    pub fn id(self) -> &'static str {
        match self {
            SpectralKnightIntent::Hex => "HEX",
            SpectralKnightIntent::SoulSlash => "SOUL_SLASH",
            SpectralKnightIntent::SoulFlame => "SOUL_FLAME",
        }
    }
}

pub fn pick_spectral_knight_intent(
    rng: &mut Rng,
    last_intent: Option<SpectralKnightIntent>,
) -> SpectralKnightIntent {
    match last_intent {
        None => SpectralKnightIntent::Hex,
        // Hex → SoulSlash always (per the C# chain — Hex's
        // FollowUpState is SoulSlash).
        Some(SpectralKnightIntent::Hex) => SpectralKnightIntent::SoulSlash,
        // SoulSlash and SoulFlame both → RandomBranch.
        // Branch: SoulSlash weight 2, SoulFlame weight 1 with
        // CannotRepeat. If last was SoulFlame, only SoulSlash is
        // eligible. If last was SoulSlash, both branches are eligible
        // weighted 2/1.
        Some(SpectralKnightIntent::SoulSlash) => {
            // 2:1 weighted pick.
            let w_slash: f32 = 2.0;
            let w_flame: f32 = 1.0;
            let total = w_slash + w_flame;
            let roll = rng.next_float(total);
            if roll < w_slash {
                SpectralKnightIntent::SoulSlash
            } else {
                SpectralKnightIntent::SoulFlame
            }
        }
        Some(SpectralKnightIntent::SoulFlame) => {
            // CannotRepeat means SoulFlame is excluded, leaving only
            // SoulSlash.
            SpectralKnightIntent::SoulSlash
        }
    }
}

pub fn execute_spectral_knight_move(
    cs: &mut CombatState,
    knight_idx: usize,
    target_player_idx: usize,
    intent: SpectralKnightIntent,
) {
    let attacker = (CombatSide::Enemy, knight_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        SpectralKnightIntent::Hex => {
            // C# Hex afflicts every card with Hexed (Ethereal). We
            // apply HexPower(2) as a marker stack and skip the
            // card-level mutation.
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "HexPower",
                SPECTRAL_KNIGHT_HEX_AMOUNT,
            );
        }
        SpectralKnightIntent::SoulSlash => {
            cs.deal_damage(
                attacker,
                player,
                SPECTRAL_KNIGHT_SOUL_SLASH_DAMAGE,
                ValueProp::MOVE,
            );
        }
        SpectralKnightIntent::SoulFlame => {
            for _ in 0..SPECTRAL_KNIGHT_SOUL_FLAME_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    SPECTRAL_KNIGHT_SOUL_FLAME_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
    }
}

// ---------- Monster intent: Tunneler -----------------------------------
//
// Reflects C# `Tunneler.GenerateMoveStateMachine`:
//   Init: Bite. Chain: Bite → Burrow → Below → Below (loop).
//   C# also has a Dizzy (Stun) state reachable via BurrowedPower's
//   AfterBlockBroken hook — when the owner's block runs out, the
//   monster is stunned then routes back to Bite. We skip the stun
//   mechanic; the simulation just leaves Burrowed up forever and
//   keeps Below-looping. Once block is fully eaten by player
//   attacks, subsequent hits land on HP normally.
//
// A0 payloads:
//   - Bite:   13 damage (DeadlyEnemies: 15)
//   - Burrow: apply BurrowedPower(1) + gain 12 block
//   - Below:  23 damage (DeadlyEnemies: 26)
//
// BurrowedPower presence preserves block across the owner's turn
// boundary (wired into begin_turn alongside BarricadePower).

const TUNNELER_BITE_DAMAGE: i32 = 13;
const TUNNELER_BURROW_BLOCK: i32 = 12;
const TUNNELER_BELOW_DAMAGE: i32 = 23;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TunnelerIntent {
    Bite,
    Burrow,
    Below,
}

impl TunnelerIntent {
    pub fn id(self) -> &'static str {
        match self {
            TunnelerIntent::Bite => "BITE_MOVE",
            TunnelerIntent::Burrow => "BURROW_MOVE",
            TunnelerIntent::Below => "BELOW_MOVE_1",
        }
    }
}

pub fn pick_tunneler_intent(
    last_intent: Option<TunnelerIntent>,
) -> TunnelerIntent {
    match last_intent {
        None => TunnelerIntent::Bite,
        Some(TunnelerIntent::Bite) => TunnelerIntent::Burrow,
        Some(TunnelerIntent::Burrow) => TunnelerIntent::Below,
        Some(TunnelerIntent::Below) => TunnelerIntent::Below,
    }
}

pub fn execute_tunneler_move(
    cs: &mut CombatState,
    tun_idx: usize,
    target_player_idx: usize,
    intent: TunnelerIntent,
) {
    let attacker = (CombatSide::Enemy, tun_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        TunnelerIntent::Bite => {
            cs.deal_damage(
                attacker,
                player,
                TUNNELER_BITE_DAMAGE,
                ValueProp::MOVE,
            );
        }
        TunnelerIntent::Burrow => {
            cs.apply_power(CombatSide::Enemy, tun_idx, "BurrowedPower", 1);
            cs.gain_block(CombatSide::Enemy, tun_idx, TUNNELER_BURROW_BLOCK);
        }
        TunnelerIntent::Below => {
            cs.deal_damage(
                attacker,
                player,
                TUNNELER_BELOW_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: TheInsatiable (boss) -----------------------
//
// Reflects C# `TheInsatiable.GenerateMoveStateMachine`:
//   Init: Liquify.
//   Chain: Liquify → Thrash1 → Bite → Salivate → Thrash2 →
//          Thrash1 → Bite → Salivate → Thrash2 → … (4-state loop).
//
// A0 payloads:
//   - Liquify:  add 3 FranticEscape to player's draw pile +
//               3 FranticEscape to discard pile. Skip SandpitPower
//               (per-target-tracked, deferred).
//   - Thrash1/Thrash2: 8 damage × 2 hits (DeadlyEnemies: 9)
//   - Bite:     28 damage (DeadlyEnemies: 31)
//   - Salivate: +2 self-Strength (DeadlyEnemies: 3)

const INSATIABLE_LIQUIFY_DRAW_COUNT: i32 = 3;
const INSATIABLE_LIQUIFY_DISCARD_COUNT: i32 = 3;
const INSATIABLE_THRASH_DAMAGE: i32 = 8;
const INSATIABLE_THRASH_HITS: i32 = 2;
const INSATIABLE_BITE_DAMAGE: i32 = 28;
const INSATIABLE_SALIVATE_STRENGTH: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TheInsatiableIntent {
    Liquify,
    Thrash1,
    Bite,
    Salivate,
    Thrash2,
}

impl TheInsatiableIntent {
    pub fn id(self) -> &'static str {
        match self {
            TheInsatiableIntent::Liquify => "LIQUIFY_GROUND_MOVE",
            TheInsatiableIntent::Thrash1 => "THRASH_MOVE_1",
            TheInsatiableIntent::Bite => "LUNGING_BITE_MOVE",
            TheInsatiableIntent::Salivate => "SALIVATE_MOVE",
            TheInsatiableIntent::Thrash2 => "THRASH_MOVE_2",
        }
    }
}

pub fn pick_the_insatiable_intent(
    last_intent: Option<TheInsatiableIntent>,
) -> TheInsatiableIntent {
    match last_intent {
        None => TheInsatiableIntent::Liquify,
        Some(TheInsatiableIntent::Liquify) => TheInsatiableIntent::Thrash1,
        Some(TheInsatiableIntent::Thrash1) => TheInsatiableIntent::Bite,
        Some(TheInsatiableIntent::Bite) => TheInsatiableIntent::Salivate,
        Some(TheInsatiableIntent::Salivate) => TheInsatiableIntent::Thrash2,
        Some(TheInsatiableIntent::Thrash2) => TheInsatiableIntent::Thrash1,
    }
}

pub fn execute_the_insatiable_move(
    cs: &mut CombatState,
    insatiable_idx: usize,
    target_player_idx: usize,
    intent: TheInsatiableIntent,
) {
    let attacker = (CombatSide::Enemy, insatiable_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        TheInsatiableIntent::Liquify => {
            // SandpitPower skipped (per-target tracking, deferred).
            // 3 FranticEscape into draw pile + 3 into discard pile.
            for _ in 0..INSATIABLE_LIQUIFY_DRAW_COUNT {
                cs.add_card_to_pile(
                    target_player_idx,
                    "FranticEscape",
                    0,
                    PileType::Draw,
                );
            }
            for _ in 0..INSATIABLE_LIQUIFY_DISCARD_COUNT {
                cs.add_card_to_pile(
                    target_player_idx,
                    "FranticEscape",
                    0,
                    PileType::Discard,
                );
            }
        }
        TheInsatiableIntent::Thrash1 | TheInsatiableIntent::Thrash2 => {
            for _ in 0..INSATIABLE_THRASH_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    INSATIABLE_THRASH_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        TheInsatiableIntent::Bite => {
            cs.deal_damage(
                attacker,
                player,
                INSATIABLE_BITE_DAMAGE,
                ValueProp::MOVE,
            );
        }
        TheInsatiableIntent::Salivate => {
            cs.apply_power(
                CombatSide::Enemy,
                insatiable_idx,
                "StrengthPower",
                INSATIABLE_SALIVATE_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: SlumberingBeetle ---------------------------
//
// Reflects C# `SlumberingBeetle.GenerateMoveStateMachine`:
//   Init: Snore.
//   Snore → conditional branch:
//     - HasPower<SlumberPower> → Snore (still asleep, no-op)
//     - else → Rollout (16 dmg + 2 self-Strength), then Rollout loops.
//
// Spawn (AfterAddedToRoom): apply PlatingPower(15), SlumberPower(3).
//
// A0 payloads:
//   - Snore:   no-op
//   - Rollout: 16 damage + 2 self-Strength (DeadlyEnemies: 18)
//
// SlumberPower (counter): decrements per owner-side turn end AND per
// unblocked damage hit; removed at 0. PlatingPower wires the per-turn
// block grant. Both ported alongside this monster.

const SLUMBERING_BEETLE_PLATING_AMOUNT: i32 = 15;
const SLUMBERING_BEETLE_SLUMBER_AMOUNT: i32 = 3;
const SLUMBERING_BEETLE_ROLLOUT_DAMAGE: i32 = 16;
const SLUMBERING_BEETLE_ROLLOUT_STRENGTH: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SlumberingBeetleIntent {
    Snore,
    Rollout,
}

impl SlumberingBeetleIntent {
    pub fn id(self) -> &'static str {
        match self {
            SlumberingBeetleIntent::Snore => "SNORE_MOVE",
            SlumberingBeetleIntent::Rollout => "ROLL_OUT_MOVE",
        }
    }
}

pub fn slumbering_beetle_spawn(cs: &mut CombatState, beetle_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        beetle_idx,
        "PlatingPower",
        SLUMBERING_BEETLE_PLATING_AMOUNT,
    );
    cs.apply_power(
        CombatSide::Enemy,
        beetle_idx,
        "SlumberPower",
        SLUMBERING_BEETLE_SLUMBER_AMOUNT,
    );
}

/// Pick the beetle's next intent. `has_slumber` is read from the
/// owner's powers — when present (and amount > 0), the beetle is
/// still asleep and snores. When Slumber clears it pivots to
/// Rollout and loops.
pub fn pick_slumbering_beetle_intent(
    last_intent: Option<SlumberingBeetleIntent>,
    has_slumber: bool,
) -> SlumberingBeetleIntent {
    if has_slumber {
        return SlumberingBeetleIntent::Snore;
    }
    match last_intent {
        // No slumber → roll out (init case after Slumber drained, or
        // after the wake-via-damage trigger fires).
        _ => SlumberingBeetleIntent::Rollout,
    }
}

pub fn execute_slumbering_beetle_move(
    cs: &mut CombatState,
    beetle_idx: usize,
    target_player_idx: usize,
    intent: SlumberingBeetleIntent,
) {
    let attacker = (CombatSide::Enemy, beetle_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        SlumberingBeetleIntent::Snore => {
            // No-op: SleepIntent in C#, monster doesn't act.
        }
        SlumberingBeetleIntent::Rollout => {
            cs.deal_damage(
                attacker,
                player,
                SLUMBERING_BEETLE_ROLLOUT_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Enemy,
                beetle_idx,
                "StrengthPower",
                SLUMBERING_BEETLE_ROLLOUT_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: DecimillipedeSegment -----------------------
//
// Reflects C# `DecimillipedeSegment.GenerateMoveStateMachine` —
// shared by Front, Middle, Back subclasses (the only diff is
// animation hooks). Init: Constrict.
//   Constrict → Bulk → Writhe → Constrict (loop)
//
// C# also constructs a DeadState + ReattachMove path: when a segment
// dies, its DeadState ticks and ReattachMove revives at max_hp + 2.
// Reattach is wired through `ReattachPower(25)` applied at spawn.
// Both deferred — the port has segments die normally without
// reviving. ReattachPower itself isn't extracted/wired.
//
// A0 payloads:
//   - Constrict: 8 damage + 1 Weak on player (DeadlyEnemies: 9)
//   - Bulk:      6 damage + 2 self-Strength (DeadlyEnemies: 7)
//   - Writhe:    5 damage × 2 hits (DeadlyEnemies: 6)

const DECIMILLIPEDE_CONSTRICT_DAMAGE: i32 = 8;
const DECIMILLIPEDE_CONSTRICT_WEAK: i32 = 1;
const DECIMILLIPEDE_BULK_DAMAGE: i32 = 6;
const DECIMILLIPEDE_BULK_STRENGTH: i32 = 2;
const DECIMILLIPEDE_WRITHE_DAMAGE: i32 = 5;
const DECIMILLIPEDE_WRITHE_HITS: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DecimillipedeSegmentIntent {
    Constrict,
    Bulk,
    Writhe,
}

impl DecimillipedeSegmentIntent {
    pub fn id(self) -> &'static str {
        match self {
            DecimillipedeSegmentIntent::Constrict => "CONSTRICT_MOVE",
            DecimillipedeSegmentIntent::Bulk => "BULK_MOVE",
            DecimillipedeSegmentIntent::Writhe => "WRITHE_MOVE",
        }
    }
}

pub fn pick_decimillipede_segment_intent(
    last_intent: Option<DecimillipedeSegmentIntent>,
) -> DecimillipedeSegmentIntent {
    match last_intent {
        None => DecimillipedeSegmentIntent::Constrict,
        Some(DecimillipedeSegmentIntent::Constrict) => {
            DecimillipedeSegmentIntent::Bulk
        }
        Some(DecimillipedeSegmentIntent::Bulk) => {
            DecimillipedeSegmentIntent::Writhe
        }
        Some(DecimillipedeSegmentIntent::Writhe) => {
            DecimillipedeSegmentIntent::Constrict
        }
    }
}

pub fn execute_decimillipede_segment_move(
    cs: &mut CombatState,
    seg_idx: usize,
    target_player_idx: usize,
    intent: DecimillipedeSegmentIntent,
) {
    let attacker = (CombatSide::Enemy, seg_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        DecimillipedeSegmentIntent::Constrict => {
            cs.deal_damage(
                attacker,
                player,
                DECIMILLIPEDE_CONSTRICT_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "WeakPower",
                DECIMILLIPEDE_CONSTRICT_WEAK,
            );
        }
        DecimillipedeSegmentIntent::Bulk => {
            cs.deal_damage(
                attacker,
                player,
                DECIMILLIPEDE_BULK_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Enemy,
                seg_idx,
                "StrengthPower",
                DECIMILLIPEDE_BULK_STRENGTH,
            );
        }
        DecimillipedeSegmentIntent::Writhe => {
            for _ in 0..DECIMILLIPEDE_WRITHE_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    DECIMILLIPEDE_WRITHE_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
    }
}

// ---------- Monster intent: TorchHeadAmalgam ---------------------------
//
// Reflects C# `TorchHeadAmalgam.GenerateMoveStateMachine`:
//   Init: Tackle1.
//   Chain: Tackle1 → Tackle2 → Beam → Tackle3 → Tackle4 → Beam → ...
//   (3-state loop Beam ↔ Tackle3 ↔ Tackle4 after the opening pair).
//
// Spawn (AfterAddedToRoom): apply MinionPower(1) (cosmetic minion
// identifier in C#; no gameplay hooks today, so the port skips it).
//
// A0 payloads:
//   - Tackle1/Tackle2: 18 dmg each (DeadlyEnemies: 19)
//   - Beam: 8 dmg × 3 hits
//   - Tackle3/Tackle4: 14 dmg each (DeadlyEnemies: 15)

const TORCH_HEAD_TACKLE_DAMAGE: i32 = 18;
const TORCH_HEAD_WEAK_TACKLE_DAMAGE: i32 = 14;
const TORCH_HEAD_BEAM_DAMAGE: i32 = 8;
const TORCH_HEAD_BEAM_HITS: i32 = 3;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TorchHeadAmalgamIntent {
    Tackle1,
    Tackle2,
    Beam,
    Tackle3,
    Tackle4,
}

impl TorchHeadAmalgamIntent {
    pub fn id(self) -> &'static str {
        match self {
            TorchHeadAmalgamIntent::Tackle1 => "TACKLE_1_MOVE",
            TorchHeadAmalgamIntent::Tackle2 => "TACKLE_2_MOVE",
            TorchHeadAmalgamIntent::Beam => "BEAM_MOVE",
            TorchHeadAmalgamIntent::Tackle3 => "TACKLE_3_MOVE",
            TorchHeadAmalgamIntent::Tackle4 => "TACKLE_4_MOVE",
        }
    }
}

pub fn pick_torch_head_amalgam_intent(
    last_intent: Option<TorchHeadAmalgamIntent>,
) -> TorchHeadAmalgamIntent {
    match last_intent {
        None => TorchHeadAmalgamIntent::Tackle1,
        Some(TorchHeadAmalgamIntent::Tackle1) => TorchHeadAmalgamIntent::Tackle2,
        Some(TorchHeadAmalgamIntent::Tackle2) => TorchHeadAmalgamIntent::Beam,
        Some(TorchHeadAmalgamIntent::Beam) => TorchHeadAmalgamIntent::Tackle3,
        Some(TorchHeadAmalgamIntent::Tackle3) => TorchHeadAmalgamIntent::Tackle4,
        Some(TorchHeadAmalgamIntent::Tackle4) => TorchHeadAmalgamIntent::Beam,
    }
}

pub fn execute_torch_head_amalgam_move(
    cs: &mut CombatState,
    torch_idx: usize,
    target_player_idx: usize,
    intent: TorchHeadAmalgamIntent,
) {
    let attacker = (CombatSide::Enemy, torch_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        TorchHeadAmalgamIntent::Tackle1 | TorchHeadAmalgamIntent::Tackle2 => {
            cs.deal_damage(
                attacker,
                player,
                TORCH_HEAD_TACKLE_DAMAGE,
                ValueProp::MOVE,
            );
        }
        TorchHeadAmalgamIntent::Tackle3 | TorchHeadAmalgamIntent::Tackle4 => {
            cs.deal_damage(
                attacker,
                player,
                TORCH_HEAD_WEAK_TACKLE_DAMAGE,
                ValueProp::MOVE,
            );
        }
        TorchHeadAmalgamIntent::Beam => {
            for _ in 0..TORCH_HEAD_BEAM_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    TORCH_HEAD_BEAM_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
    }
}

// ---------- Monster intent: SoulFysh (boss) ----------------------------
//
// Reflects C# `SoulFysh.GenerateMoveStateMachine`:
//   Init: Beckon.
//   Chain: Beckon → DeGas → Gaze → Fade → Scream → Beckon (loop).
//
// A0 payloads:
//   - Beckon: add 2 Beckon status cards to player's discard
//   - DeGas:  16 damage (DeadlyEnemies: 17)
//   - Gaze:   7 dmg + add 1 Beckon status card (DeadlyEnemies: 8)
//   - Fade:   apply IntangiblePower(2) to self
//   - Scream: 11 dmg + apply VulnerablePower(3) to player
//             (DeadlyEnemies: 12)

const SOUL_FYSH_BECKON_COUNT: i32 = 2;
const SOUL_FYSH_DE_GAS_DAMAGE: i32 = 16;
const SOUL_FYSH_GAZE_DAMAGE: i32 = 7;
const SOUL_FYSH_GAZE_BECKON: i32 = 1;
const SOUL_FYSH_FADE_INTANGIBLE: i32 = 2;
const SOUL_FYSH_SCREAM_DAMAGE: i32 = 11;
const SOUL_FYSH_SCREAM_VULN: i32 = 3;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SoulFyshIntent {
    Beckon,
    DeGas,
    Gaze,
    Fade,
    Scream,
}

impl SoulFyshIntent {
    pub fn id(self) -> &'static str {
        match self {
            SoulFyshIntent::Beckon => "BECKON_MOVE",
            SoulFyshIntent::DeGas => "DE_GAS_MOVE",
            SoulFyshIntent::Gaze => "GAZE_MOVE",
            SoulFyshIntent::Fade => "FADE_MOVE",
            SoulFyshIntent::Scream => "SCREAM_MOVE",
        }
    }
}

pub fn pick_soul_fysh_intent(
    last_intent: Option<SoulFyshIntent>,
) -> SoulFyshIntent {
    match last_intent {
        None => SoulFyshIntent::Beckon,
        Some(SoulFyshIntent::Beckon) => SoulFyshIntent::DeGas,
        Some(SoulFyshIntent::DeGas) => SoulFyshIntent::Gaze,
        Some(SoulFyshIntent::Gaze) => SoulFyshIntent::Fade,
        Some(SoulFyshIntent::Fade) => SoulFyshIntent::Scream,
        Some(SoulFyshIntent::Scream) => SoulFyshIntent::Beckon,
    }
}

pub fn execute_soul_fysh_move(
    cs: &mut CombatState,
    fysh_idx: usize,
    target_player_idx: usize,
    intent: SoulFyshIntent,
) {
    let attacker = (CombatSide::Enemy, fysh_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        SoulFyshIntent::Beckon => {
            for _ in 0..SOUL_FYSH_BECKON_COUNT {
                cs.add_card_to_pile(
                    target_player_idx,
                    "Beckon",
                    0,
                    PileType::Discard,
                );
            }
        }
        SoulFyshIntent::DeGas => {
            cs.deal_damage(
                attacker,
                player,
                SOUL_FYSH_DE_GAS_DAMAGE,
                ValueProp::MOVE,
            );
        }
        SoulFyshIntent::Gaze => {
            cs.deal_damage(
                attacker,
                player,
                SOUL_FYSH_GAZE_DAMAGE,
                ValueProp::MOVE,
            );
            for _ in 0..SOUL_FYSH_GAZE_BECKON {
                cs.add_card_to_pile(
                    target_player_idx,
                    "Beckon",
                    0,
                    PileType::Discard,
                );
            }
        }
        SoulFyshIntent::Fade => {
            cs.apply_power(
                CombatSide::Enemy,
                fysh_idx,
                "IntangiblePower",
                SOUL_FYSH_FADE_INTANGIBLE,
            );
        }
        SoulFyshIntent::Scream => {
            cs.deal_damage(
                attacker,
                player,
                SOUL_FYSH_SCREAM_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "VulnerablePower",
                SOUL_FYSH_SCREAM_VULN,
            );
        }
    }
}

// ---------- Monster intent: PhrogParasite ------------------------------
//
// Reflects C# `PhrogParasite.GenerateMoveStateMachine`. Although the
// C# state list also constructs a RandomBranchState, no transition
// leads to it — the actual chain is the simple alternation
// Infect ↔ Lash (moveState.FollowUpState = moveState2 and vice
// versa). Init: Infect.
//
// A0 payloads:
//   - Infect: add 3 Infection status cards to player's discard
//   - Lash:   4 damage × 4 hits (DeadlyEnemies: 5)
//
// No new powers needed.

const PHROG_PARASITE_INFECT_COUNT: i32 = 3;
const PHROG_PARASITE_LASH_DAMAGE: i32 = 4;
const PHROG_PARASITE_LASH_HITS: i32 = 4;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PhrogParasiteIntent {
    Infect,
    Lash,
}

impl PhrogParasiteIntent {
    pub fn id(self) -> &'static str {
        match self {
            PhrogParasiteIntent::Infect => "INFECT_MOVE",
            PhrogParasiteIntent::Lash => "LASH_MOVE",
        }
    }
}

pub fn pick_phrog_parasite_intent(
    last_intent: Option<PhrogParasiteIntent>,
) -> PhrogParasiteIntent {
    match last_intent {
        None => PhrogParasiteIntent::Infect,
        Some(PhrogParasiteIntent::Infect) => PhrogParasiteIntent::Lash,
        Some(PhrogParasiteIntent::Lash) => PhrogParasiteIntent::Infect,
    }
}

pub fn execute_phrog_parasite_move(
    cs: &mut CombatState,
    parasite_idx: usize,
    target_player_idx: usize,
    intent: PhrogParasiteIntent,
) {
    let attacker = (CombatSide::Enemy, parasite_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        PhrogParasiteIntent::Infect => {
            for _ in 0..PHROG_PARASITE_INFECT_COUNT {
                cs.add_card_to_pile(
                    target_player_idx,
                    "Infection",
                    0,
                    PileType::Discard,
                );
            }
        }
        PhrogParasiteIntent::Lash => {
            for _ in 0..PHROG_PARASITE_LASH_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    PHROG_PARASITE_LASH_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
    }
}

// ---------- Monster intent: InfestedPrism (elite) ----------------------
//
// Reflects C# `InfestedPrism.GenerateMoveStateMachine`:
//   Init: Jab. Chain Jab → Radiate → Whirlwind → Pulsate → Jab.
//
// Spawn (AfterAddedToRoom): apply VitalSparkPower(1) to self.
//
// A0 payloads:
//   - Jab:       22 damage (DeadlyEnemies: 24)
//   - Radiate:   16 damage + 16 block (DeadlyEnemies: 18)
//   - Whirlwind: 9 damage × 3 hits (DeadlyEnemies: 10)
//   - Pulsate:   20 block + 4 self-Strength (ToughEnemies: 22 block,
//                DeadlyEnemies: 5 Strength)
//
// VitalSparkPower wires into fire_after_damage_received_hooks: first
// unblocked Player-attack hit per cycle → player gains 1 energy,
// vital_spark_used flag flips. Flag clears in begin_turn(Enemy).

const INFESTED_PRISM_VITAL_SPARK_AMOUNT: i32 = 1;
const INFESTED_PRISM_JAB_DAMAGE: i32 = 22;
const INFESTED_PRISM_RADIATE_DAMAGE: i32 = 16;
const INFESTED_PRISM_RADIATE_BLOCK: i32 = 16;
const INFESTED_PRISM_WHIRLWIND_DAMAGE: i32 = 9;
const INFESTED_PRISM_WHIRLWIND_HITS: i32 = 3;
const INFESTED_PRISM_PULSATE_BLOCK: i32 = 20;
const INFESTED_PRISM_PULSATE_STRENGTH: i32 = 4;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum InfestedPrismIntent {
    Jab,
    Radiate,
    Whirlwind,
    Pulsate,
}

impl InfestedPrismIntent {
    pub fn id(self) -> &'static str {
        match self {
            InfestedPrismIntent::Jab => "JAB_MOVE",
            InfestedPrismIntent::Radiate => "RADIATE_MOVE",
            InfestedPrismIntent::Whirlwind => "WHIRLWIND_MOVE",
            InfestedPrismIntent::Pulsate => "PULSATE_MOVE",
        }
    }
}

pub fn infested_prism_spawn(cs: &mut CombatState, prism_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        prism_idx,
        "VitalSparkPower",
        INFESTED_PRISM_VITAL_SPARK_AMOUNT,
    );
}

pub fn pick_infested_prism_intent(
    last_intent: Option<InfestedPrismIntent>,
) -> InfestedPrismIntent {
    match last_intent {
        None => InfestedPrismIntent::Jab,
        Some(InfestedPrismIntent::Jab) => InfestedPrismIntent::Radiate,
        Some(InfestedPrismIntent::Radiate) => InfestedPrismIntent::Whirlwind,
        Some(InfestedPrismIntent::Whirlwind) => InfestedPrismIntent::Pulsate,
        Some(InfestedPrismIntent::Pulsate) => InfestedPrismIntent::Jab,
    }
}

pub fn execute_infested_prism_move(
    cs: &mut CombatState,
    prism_idx: usize,
    target_player_idx: usize,
    intent: InfestedPrismIntent,
) {
    let attacker = (CombatSide::Enemy, prism_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        InfestedPrismIntent::Jab => {
            cs.deal_damage(
                attacker,
                player,
                INFESTED_PRISM_JAB_DAMAGE,
                ValueProp::MOVE,
            );
        }
        InfestedPrismIntent::Radiate => {
            cs.deal_damage(
                attacker,
                player,
                INFESTED_PRISM_RADIATE_DAMAGE,
                ValueProp::MOVE,
            );
            cs.gain_block(
                CombatSide::Enemy,
                prism_idx,
                INFESTED_PRISM_RADIATE_BLOCK,
            );
        }
        InfestedPrismIntent::Whirlwind => {
            for _ in 0..INFESTED_PRISM_WHIRLWIND_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    INFESTED_PRISM_WHIRLWIND_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        InfestedPrismIntent::Pulsate => {
            cs.gain_block(
                CombatSide::Enemy,
                prism_idx,
                INFESTED_PRISM_PULSATE_BLOCK,
            );
            cs.apply_power(
                CombatSide::Enemy,
                prism_idx,
                "StrengthPower",
                INFESTED_PRISM_PULSATE_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: PhantasmalGardener -------------------------
//
// Reflects C# `PhantasmalGardener.GenerateMoveStateMachine`. Init by
// SlotName: first → Flail, second → Bite, third → Lash, fourth →
// Enlarge. Chain after init: Bite → Lash → Flail → Enlarge → Bite
// (loop).
//
// Spawn (AfterAddedToRoom): apply SkittishPower(6).
//
// A0 payloads (no DeadlyEnemies bump):
//   - Bite:    5 damage
//   - Lash:    7 damage
//   - Flail:   1 damage × 3 hits
//   - Enlarge: +2 self-Strength (DeadlyEnemies: 3)
//
// SkittishPower wires into fire_after_damage_received_hooks: first
// unblocked Player-attack hit per turn → gain Amount unpowered block,
// set skittish_used flag (cleared at end of Player turn).

const PHANTASMAL_GARDENER_SKITTISH_AMOUNT: i32 = 6;
const PHANTASMAL_GARDENER_BITE_DAMAGE: i32 = 5;
const PHANTASMAL_GARDENER_LASH_DAMAGE: i32 = 7;
const PHANTASMAL_GARDENER_FLAIL_DAMAGE: i32 = 1;
const PHANTASMAL_GARDENER_FLAIL_HITS: i32 = 3;
const PHANTASMAL_GARDENER_ENLARGE_STRENGTH: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PhantasmalGardenerIntent {
    Bite,
    Lash,
    Flail,
    Enlarge,
}

impl PhantasmalGardenerIntent {
    pub fn id(self) -> &'static str {
        match self {
            PhantasmalGardenerIntent::Bite => "BITE_MOVE",
            PhantasmalGardenerIntent::Lash => "LASH_MOVE",
            PhantasmalGardenerIntent::Flail => "FLAIL_MOVE",
            PhantasmalGardenerIntent::Enlarge => "ENLARGE_MOVE",
        }
    }
}

pub fn phantasmal_gardener_spawn(cs: &mut CombatState, gardener_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        gardener_idx,
        "SkittishPower",
        PHANTASMAL_GARDENER_SKITTISH_AMOUNT,
    );
}

/// `slot` is 1-based (first..fourth). On init, gates which intent
/// the gardener opens with. After last_intent is set the cycle is
/// deterministic and slot is ignored.
pub fn pick_phantasmal_gardener_intent(
    last_intent: Option<PhantasmalGardenerIntent>,
    slot: u8,
) -> PhantasmalGardenerIntent {
    match last_intent {
        None => match slot {
            1 => PhantasmalGardenerIntent::Flail,
            2 => PhantasmalGardenerIntent::Bite,
            3 => PhantasmalGardenerIntent::Lash,
            4 => PhantasmalGardenerIntent::Enlarge,
            _ => PhantasmalGardenerIntent::Bite,
        },
        Some(PhantasmalGardenerIntent::Bite) => PhantasmalGardenerIntent::Lash,
        Some(PhantasmalGardenerIntent::Lash) => PhantasmalGardenerIntent::Flail,
        Some(PhantasmalGardenerIntent::Flail) => PhantasmalGardenerIntent::Enlarge,
        Some(PhantasmalGardenerIntent::Enlarge) => PhantasmalGardenerIntent::Bite,
    }
}

pub fn execute_phantasmal_gardener_move(
    cs: &mut CombatState,
    gardener_idx: usize,
    target_player_idx: usize,
    intent: PhantasmalGardenerIntent,
) {
    let attacker = (CombatSide::Enemy, gardener_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        PhantasmalGardenerIntent::Bite => {
            cs.deal_damage(
                attacker,
                player,
                PHANTASMAL_GARDENER_BITE_DAMAGE,
                ValueProp::MOVE,
            );
        }
        PhantasmalGardenerIntent::Lash => {
            cs.deal_damage(
                attacker,
                player,
                PHANTASMAL_GARDENER_LASH_DAMAGE,
                ValueProp::MOVE,
            );
        }
        PhantasmalGardenerIntent::Flail => {
            for _ in 0..PHANTASMAL_GARDENER_FLAIL_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    PHANTASMAL_GARDENER_FLAIL_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        PhantasmalGardenerIntent::Enlarge => {
            cs.apply_power(
                CombatSide::Enemy,
                gardener_idx,
                "StrengthPower",
                PHANTASMAL_GARDENER_ENLARGE_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: TerrorEel (elite) --------------------------
//
// Reflects C# `TerrorEel.GenerateMoveStateMachine`:
//   Init: Crash. Chain Crash ↔ Thrash. When ShriekPower fires (HP
//   drops to ≤ ShriekAmount=70), route to Terror on next turn; in
//   C# the eel also stuns the same turn — we skip Stun and just
//   route straight to Terror, dropping the StunMove placeholder.
//
// Spawn (AfterAddedToRoom): apply ShriekPower(70) to self.
//
// A0 payloads:
//   - Crash:   16 damage (DeadlyEnemies: 18)
//   - Thrash:  3 damage × 3 hits + apply VigorPower(6) to self
//              (DeadlyEnemies: 4 damage)
//   - Terror:  apply VulnerablePower(99) to player
//
// VigorPower wires into power_additive_dealer with snapshot/drain
// semantics in begin/end_turn(Enemy). ShriekPower wires into
// fire_after_damage_received_hooks (sets the shriek_triggered flag
// when CurrentHp <= Amount; the next intent pick routes to Terror).

const TERROR_EEL_SHRIEK_AMOUNT: i32 = 70;
const TERROR_EEL_CRASH_DAMAGE: i32 = 16;
const TERROR_EEL_THRASH_DAMAGE: i32 = 3;
const TERROR_EEL_THRASH_HITS: i32 = 3;
const TERROR_EEL_THRASH_VIGOR: i32 = 6;
const TERROR_EEL_TERROR_VULN: i32 = 99;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TerrorEelIntent {
    Crash,
    Thrash,
    Terror,
}

impl TerrorEelIntent {
    pub fn id(self) -> &'static str {
        match self {
            TerrorEelIntent::Crash => "CRASH_MOVE",
            TerrorEelIntent::Thrash => "ThrashMove",
            TerrorEelIntent::Terror => "TERROR_MOVE",
        }
    }
}

pub fn terror_eel_spawn(cs: &mut CombatState, eel_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        eel_idx,
        "ShriekPower",
        TERROR_EEL_SHRIEK_AMOUNT,
    );
}

/// Pick the eel's next intent. `shriek_triggered` is the
/// per-monster flag flipped on by fire_after_damage_received_hooks
/// when the eel's HP drops to ≤ ShriekAmount. When set, the eel
/// routes to Terror once (the flag is cleared inside execute).
pub fn pick_terror_eel_intent(
    last_intent: Option<TerrorEelIntent>,
    shriek_triggered: bool,
) -> TerrorEelIntent {
    if shriek_triggered {
        return TerrorEelIntent::Terror;
    }
    match last_intent {
        None => TerrorEelIntent::Crash,
        Some(TerrorEelIntent::Crash) => TerrorEelIntent::Thrash,
        Some(TerrorEelIntent::Thrash) => TerrorEelIntent::Crash,
        // After Terror resolves, the eel falls back to Crash and
        // continues the normal cycle (C# routes Terror → Crash).
        Some(TerrorEelIntent::Terror) => TerrorEelIntent::Crash,
    }
}

pub fn execute_terror_eel_move(
    cs: &mut CombatState,
    eel_idx: usize,
    target_player_idx: usize,
    intent: TerrorEelIntent,
) {
    let attacker = (CombatSide::Enemy, eel_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        TerrorEelIntent::Crash => {
            // Audit #178: route attack through execute_attack so
            // Vigor snapshot/drain fires per-AttackCommand instead of
            // per-turn-boundary.
            cs.execute_attack(
                attacker,
                player,
                TERROR_EEL_CRASH_DAMAGE,
                1,
                ValueProp::MOVE,
                None,
            );
        }
        TerrorEelIntent::Thrash => {
            // Multi-hit attack as one AttackCommand. Vigor snapshot
            // taken before the first hit; Amount stays constant across
            // all hits (matches C# ModifyDamageAdditive reading live
            // Amount, but the AfterAttack drain only takes the
            // snapshot — so Vigor applied AFTER (next line) is preserved).
            cs.execute_attack(
                attacker,
                player,
                TERROR_EEL_THRASH_DAMAGE,
                TERROR_EEL_THRASH_HITS,
                ValueProp::MOVE,
                None,
            );
            cs.apply_power(
                CombatSide::Enemy,
                eel_idx,
                "VigorPower",
                TERROR_EEL_THRASH_VIGOR,
            );
        }
        TerrorEelIntent::Terror => {
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "VulnerablePower",
                TERROR_EEL_TERROR_VULN,
            );
            // Consume the shriek trigger so subsequent turns return
            // to the Crash/Thrash cycle.
            if let Some(creature) = cs.enemies.get_mut(eel_idx) {
                if let Some(ms) = creature.monster.as_mut() {
                    ms.set_flag("shriek_triggered", false);
                }
            }
        }
    }
}

// ---------- Monster intent: LouseProgenitor ----------------------------
//
// Reflects C# `LouseProgenitor.GenerateMoveStateMachine`:
//   Init: CurlAndGrow.
//   Chain: CurlAndGrow → Pounce → Web → CurlAndGrow (loop).
//
// Spawn (AfterAddedToRoom): apply CurlUpPower(14) to self.
//
// A0 payloads:
//   - CurlAndGrow: 14 block + 5 Strength + set curled flag
//                  (ToughEnemies: 18 block)
//   - Pounce:      14 damage (uncurls); DeadlyEnemies: 16
//   - Web:         9 damage + 2 Frail on player (uncurls);
//                  DeadlyEnemies: 10
//
// CurlUpPower wires via fire_after_damage_received_hooks: when owner
// takes powered Player-attack damage, gain Amount unpowered block
// and remove power. Mirrors C# AfterDamageReceived → AfterCardPlayed
// pipeline (we trigger eagerly — see hook doc for the simplification).

const LOUSE_PROGENITOR_CURL_BLOCK: i32 = 14;
const LOUSE_PROGENITOR_CURL_STRENGTH: i32 = 5;
const LOUSE_PROGENITOR_POUNCE_DAMAGE: i32 = 14;
const LOUSE_PROGENITOR_WEB_DAMAGE: i32 = 9;
const LOUSE_PROGENITOR_WEB_FRAIL: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LouseProgenitorIntent {
    CurlAndGrow,
    Pounce,
    Web,
}

impl LouseProgenitorIntent {
    pub fn id(self) -> &'static str {
        match self {
            LouseProgenitorIntent::CurlAndGrow => "CURL_AND_GROW_MOVE",
            LouseProgenitorIntent::Pounce => "POUNCE_MOVE",
            LouseProgenitorIntent::Web => "WEB_CANNON_MOVE",
        }
    }
}

pub fn louse_progenitor_spawn(cs: &mut CombatState, louse_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        louse_idx,
        "CurlUpPower",
        LOUSE_PROGENITOR_CURL_BLOCK,
    );
}

pub fn pick_louse_progenitor_intent(
    last_intent: Option<LouseProgenitorIntent>,
) -> LouseProgenitorIntent {
    match last_intent {
        None => LouseProgenitorIntent::CurlAndGrow,
        Some(LouseProgenitorIntent::CurlAndGrow) => LouseProgenitorIntent::Pounce,
        Some(LouseProgenitorIntent::Pounce) => LouseProgenitorIntent::Web,
        Some(LouseProgenitorIntent::Web) => LouseProgenitorIntent::CurlAndGrow,
    }
}

pub fn execute_louse_progenitor_move(
    cs: &mut CombatState,
    louse_idx: usize,
    target_player_idx: usize,
    intent: LouseProgenitorIntent,
) {
    let attacker = (CombatSide::Enemy, louse_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        LouseProgenitorIntent::CurlAndGrow => {
            cs.gain_block(
                CombatSide::Enemy,
                louse_idx,
                LOUSE_PROGENITOR_CURL_BLOCK,
            );
            cs.apply_power(
                CombatSide::Enemy,
                louse_idx,
                "StrengthPower",
                LOUSE_PROGENITOR_CURL_STRENGTH,
            );
            if let Some(creature) = cs.enemies.get_mut(louse_idx) {
                if let Some(ms) = creature.monster.as_mut() {
                    ms.set_flag("curled", true);
                }
            }
        }
        LouseProgenitorIntent::Pounce => {
            // Uncurl flag for any state-machine arm; gameplay-side
            // it's animation only in C#.
            if let Some(creature) = cs.enemies.get_mut(louse_idx) {
                if let Some(ms) = creature.monster.as_mut() {
                    ms.set_flag("curled", false);
                }
            }
            cs.deal_damage(
                attacker,
                player,
                LOUSE_PROGENITOR_POUNCE_DAMAGE,
                ValueProp::MOVE,
            );
        }
        LouseProgenitorIntent::Web => {
            if let Some(creature) = cs.enemies.get_mut(louse_idx) {
                if let Some(ms) = creature.monster.as_mut() {
                    ms.set_flag("curled", false);
                }
            }
            cs.deal_damage(
                attacker,
                player,
                LOUSE_PROGENITOR_WEB_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "FrailPower",
                LOUSE_PROGENITOR_WEB_FRAIL,
            );
        }
    }
}

// ---------- Monster intent: SkulkingColony -----------------------------
//
// Reflects C# `SkulkingColony.GenerateMoveStateMachine`:
//   Init: Smash.
//   Chain: Smash → Zoom → Inertia → PiercingStabs → Smash (loop).
//
// Spawn (AfterAddedToRoom): apply HardenedShellPower(15) to self.
//
// A0 payloads:
//   - Smash:         12 damage (DeadlyEnemies: 13)
//   - Zoom:          14 damage + 10 block (DeadlyEnemies: 16, ToughEnemies: 13)
//   - Inertia:       9 damage + 2 self-Strength (DeadlyEnemies: 11, 3)
//   - PiercingStabs: 7 damage × 2 (DeadlyEnemies: 8)

const SKULKING_COLONY_HARDENED_SHELL_AMOUNT: i32 = 15;
const SKULKING_COLONY_SMASH_DAMAGE: i32 = 12;
const SKULKING_COLONY_ZOOM_DAMAGE: i32 = 14;
const SKULKING_COLONY_ZOOM_BLOCK: i32 = 10;
const SKULKING_COLONY_INERTIA_DAMAGE: i32 = 9;
const SKULKING_COLONY_INERTIA_STRENGTH: i32 = 2;
const SKULKING_COLONY_PIERCING_STABS_DAMAGE: i32 = 7;
const SKULKING_COLONY_PIERCING_STABS_HITS: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SkulkingColonyIntent {
    Smash,
    Zoom,
    Inertia,
    PiercingStabs,
}

impl SkulkingColonyIntent {
    pub fn id(self) -> &'static str {
        match self {
            SkulkingColonyIntent::Smash => "SMASH_MOVE",
            SkulkingColonyIntent::Zoom => "ZOOM_MOVE",
            SkulkingColonyIntent::Inertia => "INERTIA_MOVE",
            SkulkingColonyIntent::PiercingStabs => "PIERCING_STABS_MOVE",
        }
    }
}

pub fn skulking_colony_spawn(cs: &mut CombatState, colony_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        colony_idx,
        "HardenedShellPower",
        SKULKING_COLONY_HARDENED_SHELL_AMOUNT,
    );
}

pub fn pick_skulking_colony_intent(
    last_intent: Option<SkulkingColonyIntent>,
) -> SkulkingColonyIntent {
    match last_intent {
        None => SkulkingColonyIntent::Smash,
        Some(SkulkingColonyIntent::Smash) => SkulkingColonyIntent::Zoom,
        Some(SkulkingColonyIntent::Zoom) => SkulkingColonyIntent::Inertia,
        Some(SkulkingColonyIntent::Inertia) => SkulkingColonyIntent::PiercingStabs,
        Some(SkulkingColonyIntent::PiercingStabs) => SkulkingColonyIntent::Smash,
    }
}

pub fn execute_skulking_colony_move(
    cs: &mut CombatState,
    colony_idx: usize,
    target_player_idx: usize,
    intent: SkulkingColonyIntent,
) {
    let attacker = (CombatSide::Enemy, colony_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        SkulkingColonyIntent::Smash => {
            cs.deal_damage(
                attacker,
                player,
                SKULKING_COLONY_SMASH_DAMAGE,
                ValueProp::MOVE,
            );
        }
        SkulkingColonyIntent::Zoom => {
            cs.deal_damage(
                attacker,
                player,
                SKULKING_COLONY_ZOOM_DAMAGE,
                ValueProp::MOVE,
            );
            cs.gain_block(CombatSide::Enemy, colony_idx, SKULKING_COLONY_ZOOM_BLOCK);
        }
        SkulkingColonyIntent::Inertia => {
            cs.deal_damage(
                attacker,
                player,
                SKULKING_COLONY_INERTIA_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Enemy,
                colony_idx,
                "StrengthPower",
                SKULKING_COLONY_INERTIA_STRENGTH,
            );
        }
        SkulkingColonyIntent::PiercingStabs => {
            for _ in 0..SKULKING_COLONY_PIERCING_STABS_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    SKULKING_COLONY_PIERCING_STABS_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
    }
}

// ---------- Monster intent: BygoneEffigy -------------------------------
//
// Reflects C# `BygoneEffigy.GenerateMoveStateMachine`:
//   Init: InitialSleep (no-op + SleepIntent).
//   InitialSleep → Wake → Slash → Slash → ...
//
// A0 payloads:
//   - InitialSleep: no-op (just an intent display)
//   - Wake:         apply StrengthPower(+10) to self
//   - Slash:        13 damage (DeadlyEnemies: 15)
//
// SleepIntent (Sleep mechanic) is just an intent display in C# — the
// monster doesn't act, the player gets a free turn. We model that as
// a no-op execute and rely on the same intent-display semantics.

const BYGONE_EFFIGY_WAKE_STRENGTH: i32 = 10;
const BYGONE_EFFIGY_SLASH_DAMAGE: i32 = 13;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BygoneEffigyIntent {
    InitialSleep,
    Wake,
    Slash,
}

impl BygoneEffigyIntent {
    pub fn id(self) -> &'static str {
        match self {
            BygoneEffigyIntent::InitialSleep => "INITIAL_SLEEP_MOVE",
            BygoneEffigyIntent::Wake => "WAKE_MOVE",
            BygoneEffigyIntent::Slash => "SLASHES_MOVE",
        }
    }
}

pub fn pick_bygone_effigy_intent(
    last_intent: Option<BygoneEffigyIntent>,
) -> BygoneEffigyIntent {
    match last_intent {
        None => BygoneEffigyIntent::InitialSleep,
        Some(BygoneEffigyIntent::InitialSleep) => BygoneEffigyIntent::Wake,
        Some(BygoneEffigyIntent::Wake) => BygoneEffigyIntent::Slash,
        Some(BygoneEffigyIntent::Slash) => BygoneEffigyIntent::Slash,
    }
}

pub fn execute_bygone_effigy_move(
    cs: &mut CombatState,
    effigy_idx: usize,
    target_player_idx: usize,
    intent: BygoneEffigyIntent,
) {
    let attacker = (CombatSide::Enemy, effigy_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        BygoneEffigyIntent::InitialSleep => {
            // No payload — just an intent-only marker.
        }
        BygoneEffigyIntent::Wake => {
            cs.apply_power(
                CombatSide::Enemy,
                effigy_idx,
                "StrengthPower",
                BYGONE_EFFIGY_WAKE_STRENGTH,
            );
        }
        BygoneEffigyIntent::Slash => {
            cs.deal_damage(
                attacker,
                player,
                BYGONE_EFFIGY_SLASH_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: SlimedBerserker ----------------------------
//
// Reflects C# `SlimedBerserker.GenerateMoveStateMachine`:
//   Init: VomitIchor.
//   Chain: VomitIchor → FuriousPummeling → LeechingHug → Smother →
//          VomitIchor (loop).
//
// A0 payloads:
//   - VomitIchor:       add 10 Slimed cards to player's discard
//   - FuriousPummeling: 4 damage × 4 hits (DeadlyEnemies: 5)
//   - LeechingHug:      3 Weak on player + 3 self-Strength
//   - Smother:          30 damage (DeadlyEnemies: 33)

const SLIMED_BERSERKER_VOMIT_COUNT: i32 = 10;
const SLIMED_BERSERKER_PUMMEL_DAMAGE: i32 = 4;
const SLIMED_BERSERKER_PUMMEL_HITS: i32 = 4;
const SLIMED_BERSERKER_HUG_WEAK: i32 = 3;
const SLIMED_BERSERKER_HUG_STRENGTH: i32 = 3;
const SLIMED_BERSERKER_SMOTHER_DAMAGE: i32 = 30;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SlimedBerserkerIntent {
    VomitIchor,
    FuriousPummeling,
    LeechingHug,
    Smother,
}

impl SlimedBerserkerIntent {
    pub fn id(self) -> &'static str {
        match self {
            SlimedBerserkerIntent::VomitIchor => "VOMIT_ICHOR_MOVE",
            SlimedBerserkerIntent::FuriousPummeling => "FURIOUS_PUMMELING_MOVE",
            SlimedBerserkerIntent::LeechingHug => "LEECHING_HUG_MOVE",
            SlimedBerserkerIntent::Smother => "SMOTHER_MOVE",
        }
    }
}

pub fn pick_slimed_berserker_intent(
    last_intent: Option<SlimedBerserkerIntent>,
) -> SlimedBerserkerIntent {
    match last_intent {
        None => SlimedBerserkerIntent::VomitIchor,
        Some(SlimedBerserkerIntent::VomitIchor) => SlimedBerserkerIntent::FuriousPummeling,
        Some(SlimedBerserkerIntent::FuriousPummeling) => SlimedBerserkerIntent::LeechingHug,
        Some(SlimedBerserkerIntent::LeechingHug) => SlimedBerserkerIntent::Smother,
        Some(SlimedBerserkerIntent::Smother) => SlimedBerserkerIntent::VomitIchor,
    }
}

pub fn execute_slimed_berserker_move(
    cs: &mut CombatState,
    slimed_idx: usize,
    target_player_idx: usize,
    intent: SlimedBerserkerIntent,
) {
    let attacker = (CombatSide::Enemy, slimed_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        SlimedBerserkerIntent::VomitIchor => {
            for _ in 0..SLIMED_BERSERKER_VOMIT_COUNT {
                cs.add_card_to_pile(
                    target_player_idx,
                    "Slimed",
                    0,
                    PileType::Discard,
                );
            }
        }
        SlimedBerserkerIntent::FuriousPummeling => {
            for _ in 0..SLIMED_BERSERKER_PUMMEL_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    SLIMED_BERSERKER_PUMMEL_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        SlimedBerserkerIntent::LeechingHug => {
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "WeakPower",
                SLIMED_BERSERKER_HUG_WEAK,
            );
            cs.apply_power(
                CombatSide::Enemy,
                slimed_idx,
                "StrengthPower",
                SLIMED_BERSERKER_HUG_STRENGTH,
            );
        }
        SlimedBerserkerIntent::Smother => {
            cs.deal_damage(
                attacker,
                player,
                SLIMED_BERSERKER_SMOTHER_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: GlobeHead ----------------------------------
//
// Reflects C# `GlobeHead.GenerateMoveStateMachine`:
//   Init: ShockingSlap.
//   Chain: ShockingSlap → ThunderStrike → GalvanicBurst → ShockingSlap.
//
// A0 payloads:
//   - ShockingSlap:  13 damage (DeadlyEnemies: 14) + 2 Frail on player
//   - ThunderStrike: 6 damage × 3 hits (DeadlyEnemies: 7)
//   - GalvanicBurst: 16 damage + 2 self-Strength (DeadlyEnemies: 17)

const GLOBE_HEAD_SLAP_DAMAGE: i32 = 13;
const GLOBE_HEAD_SLAP_FRAIL: i32 = 2;
const GLOBE_HEAD_THUNDER_DAMAGE: i32 = 6;
const GLOBE_HEAD_THUNDER_HITS: i32 = 3;
const GLOBE_HEAD_BURST_DAMAGE: i32 = 16;
const GLOBE_HEAD_BURST_STRENGTH: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GlobeHeadIntent {
    ShockingSlap,
    ThunderStrike,
    GalvanicBurst,
}

impl GlobeHeadIntent {
    pub fn id(self) -> &'static str {
        match self {
            GlobeHeadIntent::ShockingSlap => "SHOCKING_SLAP",
            GlobeHeadIntent::ThunderStrike => "THUNDER_STRIKE",
            GlobeHeadIntent::GalvanicBurst => "GALVANIC_BURST",
        }
    }
}

pub fn pick_globe_head_intent(
    last_intent: Option<GlobeHeadIntent>,
) -> GlobeHeadIntent {
    match last_intent {
        None => GlobeHeadIntent::ShockingSlap,
        Some(GlobeHeadIntent::ShockingSlap) => GlobeHeadIntent::ThunderStrike,
        Some(GlobeHeadIntent::ThunderStrike) => GlobeHeadIntent::GalvanicBurst,
        Some(GlobeHeadIntent::GalvanicBurst) => GlobeHeadIntent::ShockingSlap,
    }
}

pub fn execute_globe_head_move(
    cs: &mut CombatState,
    globe_idx: usize,
    target_player_idx: usize,
    intent: GlobeHeadIntent,
) {
    let attacker = (CombatSide::Enemy, globe_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        GlobeHeadIntent::ShockingSlap => {
            cs.deal_damage(
                attacker,
                player,
                GLOBE_HEAD_SLAP_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "FrailPower",
                GLOBE_HEAD_SLAP_FRAIL,
            );
        }
        GlobeHeadIntent::ThunderStrike => {
            for _ in 0..GLOBE_HEAD_THUNDER_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    GLOBE_HEAD_THUNDER_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        GlobeHeadIntent::GalvanicBurst => {
            cs.deal_damage(
                attacker,
                player,
                GLOBE_HEAD_BURST_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Enemy,
                globe_idx,
                "StrengthPower",
                GLOBE_HEAD_BURST_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: SpinyToad ----------------------------------
//
// Reflects C# `SpinyToad.GenerateMoveStateMachine`:
//   Init: Spikes (apply ThornsPower(5) to self, IsSpiny=true).
//   Chain: Spikes → Explosion → Lash → Spikes (loop).
//
// A0 payloads:
//   - Spikes:    apply ThornsPower(+5) to self
//   - Explosion: 23 damage (DeadlyEnemies: 25) + remove 5 Thorns
//   - Lash:      17 damage (DeadlyEnemies: 19)
//
// IsSpiny flag (true while Thorns is up) is animation-only; gameplay
// is captured by the ThornsPower stack. Skipped here.

const SPINY_TOAD_SPIKES_THORNS: i32 = 5;
const SPINY_TOAD_EXPLOSION_DAMAGE: i32 = 23;
const SPINY_TOAD_LASH_DAMAGE: i32 = 17;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SpinyToadIntent {
    Spikes,
    Explosion,
    Lash,
}

impl SpinyToadIntent {
    pub fn id(self) -> &'static str {
        match self {
            SpinyToadIntent::Spikes => "PROTRUDING_SPIKES_MOVE",
            SpinyToadIntent::Explosion => "SPIKE_EXPLOSION_MOVE",
            SpinyToadIntent::Lash => "TONGUE_LASH_MOVE",
        }
    }
}

pub fn pick_spiny_toad_intent(
    last_intent: Option<SpinyToadIntent>,
) -> SpinyToadIntent {
    match last_intent {
        None => SpinyToadIntent::Spikes,
        Some(SpinyToadIntent::Spikes) => SpinyToadIntent::Explosion,
        Some(SpinyToadIntent::Explosion) => SpinyToadIntent::Lash,
        Some(SpinyToadIntent::Lash) => SpinyToadIntent::Spikes,
    }
}

pub fn execute_spiny_toad_move(
    cs: &mut CombatState,
    toad_idx: usize,
    target_player_idx: usize,
    intent: SpinyToadIntent,
) {
    let attacker = (CombatSide::Enemy, toad_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        SpinyToadIntent::Spikes => {
            cs.apply_power(
                CombatSide::Enemy,
                toad_idx,
                "ThornsPower",
                SPINY_TOAD_SPIKES_THORNS,
            );
        }
        SpinyToadIntent::Explosion => {
            cs.deal_damage(
                attacker,
                player,
                SPINY_TOAD_EXPLOSION_DAMAGE,
                ValueProp::MOVE,
            );
            // Strip the 5 Thorns the Spikes move applied.
            cs.apply_power(
                CombatSide::Enemy,
                toad_idx,
                "ThornsPower",
                -SPINY_TOAD_SPIKES_THORNS,
            );
        }
        SpinyToadIntent::Lash => {
            cs.deal_damage(
                attacker,
                player,
                SPINY_TOAD_LASH_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: Vantom (boss) ------------------------------
//
// Reflects C# `Vantom.GenerateMoveStateMachine`:
//   Init: InkBlot.
//   Chain: InkBlot → InkyLance → Dismember → Prepare → InkBlot (loop).
//
// A0 payloads:
//   - InkBlot:   7 damage (DeadlyEnemies: 8)
//   - InkyLance: 6 damage × 2 (DeadlyEnemies: 7)
//   - Dismember: 27 damage + 3 Wound cards added to player's discard
//                (DeadlyEnemies: 30)
//   - Prepare:   apply StrengthPower(+2) to self

const VANTOM_INK_BLOT_DAMAGE: i32 = 7;
const VANTOM_INKY_LANCE_DAMAGE: i32 = 6;
const VANTOM_INKY_LANCE_HITS: i32 = 2;
const VANTOM_DISMEMBER_DAMAGE: i32 = 27;
const VANTOM_DISMEMBER_WOUND_COUNT: i32 = 3;
const VANTOM_PREPARE_STRENGTH: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VantomIntent {
    InkBlot,
    InkyLance,
    Dismember,
    Prepare,
}

impl VantomIntent {
    pub fn id(self) -> &'static str {
        match self {
            VantomIntent::InkBlot => "INK_BLOT_MOVE",
            VantomIntent::InkyLance => "INKY_LANCE_MOVE",
            VantomIntent::Dismember => "DISMEMBER_MOVE",
            VantomIntent::Prepare => "PREPARE_MOVE",
        }
    }
}

pub fn pick_vantom_intent(
    last_intent: Option<VantomIntent>,
) -> VantomIntent {
    match last_intent {
        None => VantomIntent::InkBlot,
        Some(VantomIntent::InkBlot) => VantomIntent::InkyLance,
        Some(VantomIntent::InkyLance) => VantomIntent::Dismember,
        Some(VantomIntent::Dismember) => VantomIntent::Prepare,
        Some(VantomIntent::Prepare) => VantomIntent::InkBlot,
    }
}

pub fn execute_vantom_move(
    cs: &mut CombatState,
    vantom_idx: usize,
    target_player_idx: usize,
    intent: VantomIntent,
) {
    let attacker = (CombatSide::Enemy, vantom_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        VantomIntent::InkBlot => {
            cs.deal_damage(
                attacker,
                player,
                VANTOM_INK_BLOT_DAMAGE,
                ValueProp::MOVE,
            );
        }
        VantomIntent::InkyLance => {
            for _ in 0..VANTOM_INKY_LANCE_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    VANTOM_INKY_LANCE_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        VantomIntent::Dismember => {
            cs.deal_damage(
                attacker,
                player,
                VANTOM_DISMEMBER_DAMAGE,
                ValueProp::MOVE,
            );
            for _ in 0..VANTOM_DISMEMBER_WOUND_COUNT {
                cs.add_card_to_pile(
                    target_player_idx,
                    "Wound",
                    0,
                    PileType::Discard,
                );
            }
        }
        VantomIntent::Prepare => {
            cs.apply_power(
                CombatSide::Enemy,
                vantom_idx,
                "StrengthPower",
                VANTOM_PREPARE_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: SoulNexus ----------------------------------
//
// Reflects C# `SoulNexus.GenerateMoveStateMachine`:
//   Init: SoulBurn.
//   Thereafter: RandomBranch over all 3 moves, CannotRepeat per branch
//   — picks uniformly from the 2 not-just-played.
//
// A0 payloads:
//   - SoulBurn:  29 damage (DeadlyEnemies: 31)
//   - Maelstrom: 6 damage × 4 (DeadlyEnemies: 7 dmg × 4)
//   - DrainLife: 18 damage + 2 Vulnerable + 2 Weak on player
//                (DeadlyEnemies: 19 dmg)

const SOUL_NEXUS_SOUL_BURN_DAMAGE: i32 = 29;
const SOUL_NEXUS_MAELSTROM_DAMAGE: i32 = 6;
const SOUL_NEXUS_MAELSTROM_HITS: i32 = 4;
const SOUL_NEXUS_DRAIN_LIFE_DAMAGE: i32 = 18;
const SOUL_NEXUS_DRAIN_LIFE_VULN: i32 = 2;
const SOUL_NEXUS_DRAIN_LIFE_WEAK: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SoulNexusIntent {
    SoulBurn,
    Maelstrom,
    DrainLife,
}

impl SoulNexusIntent {
    pub fn id(self) -> &'static str {
        match self {
            SoulNexusIntent::SoulBurn => "SOUL_BURN_MOVE",
            SoulNexusIntent::Maelstrom => "MAELSTROM_MOVE",
            SoulNexusIntent::DrainLife => "DRAIN_LIFE_MOVE",
        }
    }
}

pub fn pick_soul_nexus_intent(
    rng: &mut Rng,
    last_intent: Option<SoulNexusIntent>,
) -> SoulNexusIntent {
    if last_intent.is_none() {
        return SoulNexusIntent::SoulBurn;
    }
    let allowed: Vec<SoulNexusIntent> = [
        SoulNexusIntent::SoulBurn,
        SoulNexusIntent::Maelstrom,
        SoulNexusIntent::DrainLife,
    ]
    .into_iter()
    .filter(|i| Some(*i) != last_intent)
    .collect();
    let pick = rng.next_int_range(0, allowed.len() as i32) as usize;
    allowed[pick]
}

pub fn execute_soul_nexus_move(
    cs: &mut CombatState,
    nexus_idx: usize,
    target_player_idx: usize,
    intent: SoulNexusIntent,
) {
    let attacker = (CombatSide::Enemy, nexus_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        SoulNexusIntent::SoulBurn => {
            cs.deal_damage(
                attacker,
                player,
                SOUL_NEXUS_SOUL_BURN_DAMAGE,
                ValueProp::MOVE,
            );
        }
        SoulNexusIntent::Maelstrom => {
            for _ in 0..SOUL_NEXUS_MAELSTROM_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    SOUL_NEXUS_MAELSTROM_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        SoulNexusIntent::DrainLife => {
            cs.deal_damage(
                attacker,
                player,
                SOUL_NEXUS_DRAIN_LIFE_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "VulnerablePower",
                SOUL_NEXUS_DRAIN_LIFE_VULN,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "WeakPower",
                SOUL_NEXUS_DRAIN_LIFE_WEAK,
            );
        }
    }
}

// ---------- Monster intent: DevotedSculptor ----------------------------
//
// Reflects C# `DevotedSculptor.GenerateMoveStateMachine`:
//   Init: ForbiddenIncantation (apply RitualPower(9) to self).
//   Then: Savage forever.
//
// RitualPower wires into tick_territorial_powers (Strength ramp on
// owner-side turn end). With Ritual(9), Sculptor gains +9 Strength per
// turn after the buff — Savage starts at 12 dmg and grows by 9 each
// turn.
//
// A0 payloads:
//   - ForbiddenIncantation: apply RitualPower(9) to self
//   - Savage: 12 damage (DeadlyEnemies: 15)

const DEVOTED_SCULPTOR_RITUAL_AMOUNT: i32 = 9;
const DEVOTED_SCULPTOR_SAVAGE_DAMAGE: i32 = 12;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DevotedSculptorIntent {
    ForbiddenIncantation,
    Savage,
}

impl DevotedSculptorIntent {
    pub fn id(self) -> &'static str {
        match self {
            DevotedSculptorIntent::ForbiddenIncantation => "FORBIDDEN_INCANTATION_MOVE",
            DevotedSculptorIntent::Savage => "SAVAGE_MOVE",
        }
    }
}

pub fn pick_devoted_sculptor_intent(
    last_intent: Option<DevotedSculptorIntent>,
) -> DevotedSculptorIntent {
    match last_intent {
        None => DevotedSculptorIntent::ForbiddenIncantation,
        Some(_) => DevotedSculptorIntent::Savage,
    }
}

pub fn execute_devoted_sculptor_move(
    cs: &mut CombatState,
    sculptor_idx: usize,
    target_player_idx: usize,
    intent: DevotedSculptorIntent,
) {
    let attacker = (CombatSide::Enemy, sculptor_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        DevotedSculptorIntent::ForbiddenIncantation => {
            cs.apply_power(
                CombatSide::Enemy,
                sculptor_idx,
                "RitualPower",
                DEVOTED_SCULPTOR_RITUAL_AMOUNT,
            );
        }
        DevotedSculptorIntent::Savage => {
            cs.deal_damage(
                attacker,
                player,
                DEVOTED_SCULPTOR_SAVAGE_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: Exoskeleton --------------------------------
//
// Reflects C# `Exoskeleton.GenerateMoveStateMachine`. Slot-driven init:
//   slot 1 (first):  Skitter
//   slot 2 (second): Mandibles
//   slot 3 (third):  Enrage
//   slot 4 (fourth): RandomBranch(Skitter | Mandibles, CannotRepeat)
//
// Chain:
//   Skitter   → RandomBranch (Skitter | Mandibles excluding repeat)
//   Mandibles → Enrage
//   Enrage    → RandomBranch
//
// Spawn (AfterAddedToRoom): apply HardToKillPower(9) to self —
// per-hit damage cap at 9, wired into power_damage_cap_target.
//
// A0 payloads:
//   - Skitter:   1 dmg × 3 hits (DeadlyEnemies: ×4 hits)
//   - Mandibles: 8 dmg (DeadlyEnemies: 9)
//   - Enrage:    apply StrengthPower(+2) to self

const EXOSKELETON_HARD_TO_KILL_AMOUNT: i32 = 9;
const EXOSKELETON_SKITTER_DAMAGE: i32 = 1;
const EXOSKELETON_SKITTER_HITS: i32 = 3;
const EXOSKELETON_MANDIBLES_DAMAGE: i32 = 8;
const EXOSKELETON_ENRAGE_STRENGTH: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ExoskeletonIntent {
    Skitter,
    Mandibles,
    Enrage,
}

impl ExoskeletonIntent {
    pub fn id(self) -> &'static str {
        match self {
            ExoskeletonIntent::Skitter => "SKITTER_MOVE",
            ExoskeletonIntent::Mandibles => "MANDIBLE_MOVE",
            ExoskeletonIntent::Enrage => "ENRAGE_MOVE",
        }
    }
}

/// Spawn payload — caller invokes once per exoskeleton.
pub fn exoskeleton_spawn(cs: &mut CombatState, exo_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        exo_idx,
        "HardToKillPower",
        EXOSKELETON_HARD_TO_KILL_AMOUNT,
    );
}

/// `slot` is 1-based, matching the C# SlotName ("first"=1 etc.). For
/// slot 4 (RandomBranch), `rng` is consulted; pass any `Rng` for
/// slot 1..=3 (it's untouched on those paths).
pub fn pick_exoskeleton_intent(
    rng: &mut Rng,
    last_intent: Option<ExoskeletonIntent>,
    slot: u8,
) -> ExoskeletonIntent {
    match last_intent {
        None => match slot {
            1 => ExoskeletonIntent::Skitter,
            2 => ExoskeletonIntent::Mandibles,
            3 => ExoskeletonIntent::Enrage,
            _ => exoskeleton_random_branch(rng, None),
        },
        Some(ExoskeletonIntent::Mandibles) => ExoskeletonIntent::Enrage,
        Some(prev @ ExoskeletonIntent::Skitter) => {
            exoskeleton_random_branch(rng, Some(prev))
        }
        Some(prev @ ExoskeletonIntent::Enrage) => {
            exoskeleton_random_branch(rng, Some(prev))
        }
    }
}

/// RandomBranch over {Skitter, Mandibles} with CannotRepeat. If the
/// last intent matches one of the branches, that branch is excluded.
/// With one option left, return it directly.
fn exoskeleton_random_branch(
    rng: &mut Rng,
    last_intent: Option<ExoskeletonIntent>,
) -> ExoskeletonIntent {
    let allowed: Vec<ExoskeletonIntent> =
        [ExoskeletonIntent::Skitter, ExoskeletonIntent::Mandibles]
            .into_iter()
            .filter(|i| Some(*i) != last_intent)
            .collect();
    let pick = rng.next_int_range(0, allowed.len() as i32) as usize;
    allowed[pick]
}

pub fn execute_exoskeleton_move(
    cs: &mut CombatState,
    exo_idx: usize,
    target_player_idx: usize,
    intent: ExoskeletonIntent,
) {
    let attacker = (CombatSide::Enemy, exo_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        ExoskeletonIntent::Skitter => {
            for _ in 0..EXOSKELETON_SKITTER_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    EXOSKELETON_SKITTER_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        ExoskeletonIntent::Mandibles => {
            cs.deal_damage(
                attacker,
                player,
                EXOSKELETON_MANDIBLES_DAMAGE,
                ValueProp::MOVE,
            );
        }
        ExoskeletonIntent::Enrage => {
            cs.apply_power(
                CombatSide::Enemy,
                exo_idx,
                "StrengthPower",
                EXOSKELETON_ENRAGE_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: Toadpole -----------------------------------
//
// Reflects C# `Toadpole.GenerateMoveStateMachine`:
//   3-state cycle: SpikeSpit ↔ Whirl ↔ Spiken (triangle).
//   Init depends on `IsFront`:
//     - IsFront=true  → init Spiken
//     - IsFront=false → init Whirl
//   Chain (both entry points walk the same triangle):
//     SpikeSpit → Whirl → Spiken → SpikeSpit → …
//
// A0 payloads:
//   - SpikeSpit: 3 damage × 3 hits (DeadlyEnemies: 4). Also removes
//                Spiken (2) Thorns from self — `Apply<ThornsPower>(-2)`.
//   - Whirl:     7 damage (DeadlyEnemies: 8).
//   - Spiken:    apply ThornsPower(+2) to self.
//
// ThornsPower wires into the deal_damage pipeline (fire_thorns_hook):
// when target with Thorns is hit by a powered attack, dealer takes
// Amount unpowered damage back.

const TOADPOLE_SPIKE_SPIT_DAMAGE: i32 = 3;
const TOADPOLE_SPIKE_SPIT_HITS: i32 = 3;
const TOADPOLE_WHIRL_DAMAGE: i32 = 7;
const TOADPOLE_SPIKEN_AMOUNT: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ToadpoleIntent {
    SpikeSpit,
    Whirl,
    Spiken,
}

impl ToadpoleIntent {
    pub fn id(self) -> &'static str {
        match self {
            ToadpoleIntent::SpikeSpit => "SPIKE_SPIT_MOVE",
            ToadpoleIntent::Whirl => "WHIRL_MOVE",
            ToadpoleIntent::Spiken => "SPIKEN_MOVE",
        }
    }
}

/// Init depends on `is_front`. Subsequent intent walks the cycle.
pub fn pick_toadpole_intent(
    last_intent: Option<ToadpoleIntent>,
    is_front: bool,
) -> ToadpoleIntent {
    match last_intent {
        None if is_front => ToadpoleIntent::Spiken,
        None => ToadpoleIntent::Whirl,
        Some(ToadpoleIntent::SpikeSpit) => ToadpoleIntent::Whirl,
        Some(ToadpoleIntent::Whirl) => ToadpoleIntent::Spiken,
        Some(ToadpoleIntent::Spiken) => ToadpoleIntent::SpikeSpit,
    }
}

pub fn execute_toadpole_move(
    cs: &mut CombatState,
    toadpole_idx: usize,
    target_player_idx: usize,
    intent: ToadpoleIntent,
) {
    let attacker = (CombatSide::Enemy, toadpole_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        ToadpoleIntent::SpikeSpit => {
            // Negative apply to self — strips ThornsPower(2). C# uses
            // PowerCmd.Apply<ThornsPower>(-SpikenAmount).
            cs.apply_power(
                CombatSide::Enemy,
                toadpole_idx,
                "ThornsPower",
                -TOADPOLE_SPIKEN_AMOUNT,
            );
            for _ in 0..TOADPOLE_SPIKE_SPIT_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    TOADPOLE_SPIKE_SPIT_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        ToadpoleIntent::Whirl => {
            cs.deal_damage(
                attacker,
                player,
                TOADPOLE_WHIRL_DAMAGE,
                ValueProp::MOVE,
            );
        }
        ToadpoleIntent::Spiken => {
            cs.apply_power(
                CombatSide::Enemy,
                toadpole_idx,
                "ThornsPower",
                TOADPOLE_SPIKEN_AMOUNT,
            );
        }
    }
}

// ---------- Monster intent: ThievingHopper -----------------------------
//
// Reflects C# `ThievingHopper.GenerateMoveStateMachine`:
//   Init: THIEVERY_MOVE.
//   Chain: Thievery → Flutter → HatTrick → Nab → Escape → Escape (loop).
// Deterministic, no RNG.
//
// Spawn (AfterAddedToRoom): apply EscapeArtistPower(5) to self.
//
// A0 payloads:
//   - Thievery (17 dmg / 19 DeadlyEnemies) + steal a card from each
//     target (lifts it out of combat, applies SwipePower(1) per stolen
//     card on the hopper). Card-steal/Swipe deferred — we deal damage
//     only, matching the SingleAttackIntent piece of the C# move.
//   - Flutter:  apply FlutterPower(5) to self.
//   - HatTrick: 21 dmg (DeadlyEnemies: 23).
//   - Nab:      14 dmg (DeadlyEnemies: 16).
//   - Escape:   leave combat. Modeled as a no-op here — the Escape
//     mechanic (CreatureCmd.Escape) is deferred; the hopper just keeps
//     idling in this state forever, which is enough for replay /
//     readiness scoring.
//
// FlutterPower wires into power_multiplicative_target (×0.50 incoming
// powered damage on owner). C# also decrements Flutter per powered
// hit and stuns the owner when it reaches 0 — both deferred (no Stun
// mechanic yet). Presence-only reduction is the playable approximation.

const THIEVING_HOPPER_THEFT_DAMAGE: i32 = 17;
const THIEVING_HOPPER_HAT_TRICK_DAMAGE: i32 = 21;
const THIEVING_HOPPER_NAB_DAMAGE: i32 = 14;
const THIEVING_HOPPER_FLUTTER_AMOUNT: i32 = 5;
const THIEVING_HOPPER_ESCAPE_ARTIST_AMOUNT: i32 = 5;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ThievingHopperIntent {
    Thievery,
    Flutter,
    HatTrick,
    Nab,
    Escape,
}

impl ThievingHopperIntent {
    pub fn id(self) -> &'static str {
        match self {
            ThievingHopperIntent::Thievery => "THIEVERY_MOVE",
            ThievingHopperIntent::Flutter => "FLUTTER_MOVE",
            ThievingHopperIntent::HatTrick => "HAT_TRICK_MOVE",
            ThievingHopperIntent::Nab => "NAB_MOVE",
            ThievingHopperIntent::Escape => "ESCAPE_MOVE",
        }
    }
}

/// Spawn payload — caller invokes once when the hopper is added to
/// combat. Mirrors C# `AfterAddedToRoom`.
pub fn thieving_hopper_spawn(cs: &mut CombatState, hopper_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        hopper_idx,
        "EscapeArtistPower",
        THIEVING_HOPPER_ESCAPE_ARTIST_AMOUNT,
    );
}

pub fn pick_thieving_hopper_intent(
    last_intent: Option<ThievingHopperIntent>,
) -> ThievingHopperIntent {
    match last_intent {
        None => ThievingHopperIntent::Thievery,
        Some(ThievingHopperIntent::Thievery) => ThievingHopperIntent::Flutter,
        Some(ThievingHopperIntent::Flutter) => ThievingHopperIntent::HatTrick,
        Some(ThievingHopperIntent::HatTrick) => ThievingHopperIntent::Nab,
        Some(ThievingHopperIntent::Nab) => ThievingHopperIntent::Escape,
        Some(ThievingHopperIntent::Escape) => ThievingHopperIntent::Escape,
    }
}

pub fn execute_thieving_hopper_move(
    cs: &mut CombatState,
    hopper_idx: usize,
    target_player_idx: usize,
    intent: ThievingHopperIntent,
) {
    let attacker = (CombatSide::Enemy, hopper_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        ThievingHopperIntent::Thievery => {
            cs.deal_damage(
                attacker,
                player,
                THIEVING_HOPPER_THEFT_DAMAGE,
                ValueProp::MOVE,
            );
        }
        ThievingHopperIntent::Flutter => {
            cs.apply_power(
                CombatSide::Enemy,
                hopper_idx,
                "FlutterPower",
                THIEVING_HOPPER_FLUTTER_AMOUNT,
            );
        }
        ThievingHopperIntent::HatTrick => {
            cs.deal_damage(
                attacker,
                player,
                THIEVING_HOPPER_HAT_TRICK_DAMAGE,
                ValueProp::MOVE,
            );
        }
        ThievingHopperIntent::Nab => {
            cs.deal_damage(
                attacker,
                player,
                THIEVING_HOPPER_NAB_DAMAGE,
                ValueProp::MOVE,
            );
        }
        ThievingHopperIntent::Escape => {
            // No-op — Escape mechanic (CreatureCmd.Escape removes the
            // monster from combat) is deferred. The hopper sits in
            // this state until end-of-combat by player kill.
        }
    }
}

// ---------- Monster intent: CalcifiedCultist ---------------------------
//
// Reflects C# `CalcifiedCultist.GenerateMoveStateMachine`:
//   Init: INCANTATION_MOVE (applies RitualPower(2) to self).
//   Cycle: Incantation → DarkStrike → DarkStrike → DarkStrike → …
//   (Incantation fires once.)
//
// RitualPower is wired into tick_territorial_powers — applies
// Amount Strength to owner each owner-side turn end (with the
// noted "skip first turn if enemy-applied" simplification).
//
// A0 payloads:
//   - Incantation: apply RitualPower(2) to self
//   - DarkStrike:  9 damage (DeadlyEnemies: 11)

const CALCIFIED_CULTIST_INCANTATION_AMOUNT: i32 = 2;
const CALCIFIED_CULTIST_DARK_STRIKE_DAMAGE: i32 = 9;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CalcifiedCultistIntent {
    Incantation,
    DarkStrike,
}

impl CalcifiedCultistIntent {
    pub fn id(self) -> &'static str {
        match self {
            CalcifiedCultistIntent::Incantation => "INCANTATION_MOVE",
            CalcifiedCultistIntent::DarkStrike => "DARK_STRIKE_MOVE",
        }
    }
}

pub fn pick_calcified_cultist_intent(
    last_intent: Option<CalcifiedCultistIntent>,
) -> CalcifiedCultistIntent {
    match last_intent {
        None => CalcifiedCultistIntent::Incantation,
        // After Incantation, DarkStrike forever.
        Some(CalcifiedCultistIntent::Incantation) => {
            CalcifiedCultistIntent::DarkStrike
        }
        Some(CalcifiedCultistIntent::DarkStrike) => {
            CalcifiedCultistIntent::DarkStrike
        }
    }
}

pub fn execute_calcified_cultist_move(
    cs: &mut CombatState,
    cultist_idx: usize,
    target_player_idx: usize,
    intent: CalcifiedCultistIntent,
) {
    let attacker = (CombatSide::Enemy, cultist_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        CalcifiedCultistIntent::Incantation => {
            cs.apply_power(
                CombatSide::Enemy,
                cultist_idx,
                "RitualPower",
                CALCIFIED_CULTIST_INCANTATION_AMOUNT,
            );
        }
        CalcifiedCultistIntent::DarkStrike => {
            cs.deal_damage(
                attacker,
                player,
                CALCIFIED_CULTIST_DARK_STRIKE_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: SludgeSpinner ------------------------------
//
// Reflects C# `SludgeSpinner.GenerateMoveStateMachine`. Init OilSpray;
// subsequent: RandomBranch over all 3 moves with CannotRepeat on
// every branch. So each turn after init picks uniformly between the
// 2 not-just-played.
//
// A0 payloads:
//   - OilSpray: 8 damage + 1 Weak (DeadlyEnemies dmg: 9)
//   - Slam:     11 damage (DeadlyEnemies: 12)
//   - Rage:     6 damage + 3 self-Strength (DeadlyEnemies dmg: 7)

const SLUDGE_SPINNER_OIL_DAMAGE: i32 = 8;
const SLUDGE_SPINNER_OIL_WEAK: i32 = 1;
const SLUDGE_SPINNER_SLAM_DAMAGE: i32 = 11;
const SLUDGE_SPINNER_RAGE_DAMAGE: i32 = 6;
const SLUDGE_SPINNER_RAGE_STRENGTH: i32 = 3;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SludgeSpinnerIntent {
    OilSpray,
    Slam,
    Rage,
}

impl SludgeSpinnerIntent {
    pub fn id(self) -> &'static str {
        match self {
            SludgeSpinnerIntent::OilSpray => "OIL_SPRAY_MOVE",
            SludgeSpinnerIntent::Slam => "SLAM_MOVE",
            SludgeSpinnerIntent::Rage => "RAGE_MOVE",
        }
    }
}

pub fn pick_sludge_spinner_intent(
    rng: &mut Rng,
    last_intent: Option<SludgeSpinnerIntent>,
) -> SludgeSpinnerIntent {
    if last_intent.is_none() {
        return SludgeSpinnerIntent::OilSpray;
    }
    // Each branch weight 1, CannotRepeat: pick uniformly between the
    // 2 branches that weren't just played.
    let allowed: Vec<SludgeSpinnerIntent> = [
        SludgeSpinnerIntent::OilSpray,
        SludgeSpinnerIntent::Slam,
        SludgeSpinnerIntent::Rage,
    ]
    .into_iter()
    .filter(|i| Some(*i) != last_intent)
    .collect();
    let pick = rng.next_int_range(0, allowed.len() as i32) as usize;
    allowed[pick]
}

pub fn execute_sludge_spinner_move(
    cs: &mut CombatState,
    spinner_idx: usize,
    target_player_idx: usize,
    intent: SludgeSpinnerIntent,
) {
    let attacker = (CombatSide::Enemy, spinner_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        SludgeSpinnerIntent::OilSpray => {
            cs.deal_damage(
                attacker,
                player,
                SLUDGE_SPINNER_OIL_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "WeakPower",
                SLUDGE_SPINNER_OIL_WEAK,
            );
        }
        SludgeSpinnerIntent::Slam => {
            cs.deal_damage(
                attacker,
                player,
                SLUDGE_SPINNER_SLAM_DAMAGE,
                ValueProp::MOVE,
            );
        }
        SludgeSpinnerIntent::Rage => {
            cs.deal_damage(
                attacker,
                player,
                SLUDGE_SPINNER_RAGE_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Enemy,
                spinner_idx,
                "StrengthPower",
                SLUDGE_SPINNER_RAGE_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: FuzzyWurmCrawler ---------------------------
//
// Reflects C# `FuzzyWurmCrawler.GenerateMoveStateMachine`. Deterministic
// 3-cycle init FirstAcidGoop:
//   FirstAcidGoop → Inhale → AcidGoop → FirstAcidGoop → …
//
// FirstAcidGoop and AcidGoop share the same payload (separate state
// nodes for chain ordering). Both clear IsPuffed (animation-only —
// not modeled). Inhale grants +7 self-Strength.
//
// A0 payloads:
//   - FirstAcidGoop / AcidGoop: 4 damage (DeadlyEnemies: 6)
//   - Inhale: +7 self-Strength (const)

const FUZZY_WURM_ACID_GOOP_DAMAGE: i32 = 4;
const FUZZY_WURM_INHALE_STRENGTH: i32 = 7;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FuzzyWurmCrawlerIntent {
    FirstAcidGoop,
    Inhale,
    AcidGoop,
}

impl FuzzyWurmCrawlerIntent {
    pub fn id(self) -> &'static str {
        match self {
            FuzzyWurmCrawlerIntent::FirstAcidGoop => "FIRST_ACID_GOOP",
            FuzzyWurmCrawlerIntent::Inhale => "INHALE",
            FuzzyWurmCrawlerIntent::AcidGoop => "ACID_GOOP",
        }
    }
}

pub fn pick_fuzzy_wurm_crawler_intent(
    last_intent: Option<FuzzyWurmCrawlerIntent>,
) -> FuzzyWurmCrawlerIntent {
    match last_intent {
        None => FuzzyWurmCrawlerIntent::FirstAcidGoop,
        Some(FuzzyWurmCrawlerIntent::FirstAcidGoop) => {
            FuzzyWurmCrawlerIntent::Inhale
        }
        Some(FuzzyWurmCrawlerIntent::Inhale) => FuzzyWurmCrawlerIntent::AcidGoop,
        Some(FuzzyWurmCrawlerIntent::AcidGoop) => {
            FuzzyWurmCrawlerIntent::FirstAcidGoop
        }
    }
}

pub fn execute_fuzzy_wurm_crawler_move(
    cs: &mut CombatState,
    wurm_idx: usize,
    target_player_idx: usize,
    intent: FuzzyWurmCrawlerIntent,
) {
    let attacker = (CombatSide::Enemy, wurm_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        FuzzyWurmCrawlerIntent::FirstAcidGoop
        | FuzzyWurmCrawlerIntent::AcidGoop => {
            cs.deal_damage(
                attacker,
                player,
                FUZZY_WURM_ACID_GOOP_DAMAGE,
                ValueProp::MOVE,
            );
        }
        FuzzyWurmCrawlerIntent::Inhale => {
            cs.apply_power(
                CombatSide::Enemy,
                wurm_idx,
                "StrengthPower",
                FUZZY_WURM_INHALE_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: BowlbugRock --------------------------------
//
// Reflects C# `BowlbugRock.GenerateMoveStateMachine`:
//   Init: HEADBUTT_MOVE.
//   After Headbutt: ConditionalBranch(IsOffBalance → Dizzy; else →
//     Headbutt). Dizzy clears IsOffBalance and chains back to Headbutt.
//
// On spawn: ImbalancedPower(1). ImbalancedPower's AfterDamageGiven
// hook (wired in fire_after_damage_given_hooks) flips
// monster.flags["is_off_balance"] = true when this rock's attack is
// fully blocked. The Dizzy move clears the flag.
//
// A0 payloads:
//   - Headbutt: 15 damage (DeadlyEnemies: 16)
//   - Dizzy:    no payload — recovers (clears off-balance flag)

const BOWLBUG_ROCK_HEADBUTT_DAMAGE: i32 = 15;
const BOWLBUG_ROCK_IMBALANCED_AMOUNT: i32 = 1;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BowlbugRockIntent {
    Headbutt,
    Dizzy,
}

impl BowlbugRockIntent {
    pub fn id(self) -> &'static str {
        match self {
            BowlbugRockIntent::Headbutt => "HEADBUTT_MOVE",
            BowlbugRockIntent::Dizzy => "DIZZY_MOVE",
        }
    }
}

pub fn bowlbug_rock_spawn(cs: &mut CombatState, rock_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        rock_idx,
        "ImbalancedPower",
        BOWLBUG_ROCK_IMBALANCED_AMOUNT,
    );
}

/// Pick BowlbugRock's next intent. `is_off_balance` comes from
/// `monster.flags["is_off_balance"]` (set by ImbalancedPower when this
/// rock's last attack was fully blocked).
pub fn pick_bowlbug_rock_intent(
    last_intent: Option<BowlbugRockIntent>,
    is_off_balance: bool,
) -> BowlbugRockIntent {
    match last_intent {
        None => BowlbugRockIntent::Headbutt,
        Some(BowlbugRockIntent::Headbutt) => {
            if is_off_balance {
                BowlbugRockIntent::Dizzy
            } else {
                BowlbugRockIntent::Headbutt
            }
        }
        Some(BowlbugRockIntent::Dizzy) => BowlbugRockIntent::Headbutt,
    }
}

pub fn execute_bowlbug_rock_move(
    cs: &mut CombatState,
    rock_idx: usize,
    target_player_idx: usize,
    intent: BowlbugRockIntent,
) {
    let attacker = (CombatSide::Enemy, rock_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        BowlbugRockIntent::Headbutt => {
            cs.deal_damage(
                attacker,
                player,
                BOWLBUG_ROCK_HEADBUTT_DAMAGE,
                ValueProp::MOVE,
            );
        }
        BowlbugRockIntent::Dizzy => {
            // No damage payload — clear off-balance to recover.
            if let Some(creature) =
                creature_mut(cs, CombatSide::Enemy, rock_idx)
            {
                if let Some(ms) = creature.monster.as_mut() {
                    ms.set_flag("is_off_balance", false);
                }
            }
        }
    }
}

// ---------- Monster intent: MechaKnight --------------------------------
//
// Reflects C# `MechaKnight.GenerateMoveStateMachine`:
//   Init: CHARGE_MOVE.
//   Chain: Charge → Flamethrower → Windup → HeavyCleave →
//          Flamethrower → Windup → HeavyCleave → … (Charge fires once;
//          Flamethrower / Windup / HeavyCleave loop forever).
//
// On spawn: ArtifactPower(3) (presence-only — debuff-absorb behavior
// deferred).
//
// A0 payloads:
//   - Charge:      25 damage (DeadlyEnemies: 30)
//   - Flamethrower: add 4 Burn status cards to player's hand
//   - Windup:      15 self-block + 5 self-Strength (consts)
//   - HeavyCleave: 35 damage (DeadlyEnemies: 40)
//
// IsWoundUp flag (set on Windup, cleared on HeavyCleave) is purely
// animation in C# — no functional effect — so we don't track it.

const MECHA_KNIGHT_CHARGE_DAMAGE: i32 = 25;
const MECHA_KNIGHT_FLAMETHROWER_BURN_COUNT: i32 = 4;
const MECHA_KNIGHT_WINDUP_BLOCK: i32 = 15;
const MECHA_KNIGHT_WINDUP_STRENGTH: i32 = 5;
const MECHA_KNIGHT_HEAVY_CLEAVE_DAMAGE: i32 = 35;
const MECHA_KNIGHT_ARTIFACT_AMOUNT: i32 = 3;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MechaKnightIntent {
    Charge,
    Flamethrower,
    Windup,
    HeavyCleave,
}

impl MechaKnightIntent {
    pub fn id(self) -> &'static str {
        match self {
            MechaKnightIntent::Charge => "CHARGE_MOVE",
            MechaKnightIntent::Flamethrower => "FLAMETHROWER_MOVE",
            MechaKnightIntent::Windup => "WINDUP_MOVE",
            MechaKnightIntent::HeavyCleave => "HEAVY_CLEAVE_MOVE",
        }
    }
}

pub fn mecha_knight_spawn(cs: &mut CombatState, knight_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        knight_idx,
        "ArtifactPower",
        MECHA_KNIGHT_ARTIFACT_AMOUNT,
    );
}

pub fn pick_mecha_knight_intent(
    last_intent: Option<MechaKnightIntent>,
) -> MechaKnightIntent {
    match last_intent {
        None => MechaKnightIntent::Charge,
        Some(MechaKnightIntent::Charge) => MechaKnightIntent::Flamethrower,
        Some(MechaKnightIntent::Flamethrower) => MechaKnightIntent::Windup,
        Some(MechaKnightIntent::Windup) => MechaKnightIntent::HeavyCleave,
        Some(MechaKnightIntent::HeavyCleave) => MechaKnightIntent::Flamethrower,
    }
}

pub fn execute_mecha_knight_move(
    cs: &mut CombatState,
    knight_idx: usize,
    target_player_idx: usize,
    intent: MechaKnightIntent,
) {
    let attacker = (CombatSide::Enemy, knight_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        MechaKnightIntent::Charge => {
            cs.deal_damage(
                attacker,
                player,
                MECHA_KNIGHT_CHARGE_DAMAGE,
                ValueProp::MOVE,
            );
        }
        MechaKnightIntent::Flamethrower => {
            for _ in 0..MECHA_KNIGHT_FLAMETHROWER_BURN_COUNT {
                cs.add_card_to_pile(
                    target_player_idx,
                    "Burn",
                    0,
                    PileType::Hand,
                );
            }
        }
        MechaKnightIntent::Windup => {
            cs.gain_block(
                CombatSide::Enemy,
                knight_idx,
                MECHA_KNIGHT_WINDUP_BLOCK,
            );
            cs.apply_power(
                CombatSide::Enemy,
                knight_idx,
                "StrengthPower",
                MECHA_KNIGHT_WINDUP_STRENGTH,
            );
        }
        MechaKnightIntent::HeavyCleave => {
            cs.deal_damage(
                attacker,
                player,
                MECHA_KNIGHT_HEAVY_CLEAVE_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: Entomancer ---------------------------------
//
// Reflects C# `Entomancer.GenerateMoveStateMachine`:
//   Init: BEES_MOVE.
//   Chain: Bees → Spear → Spit → Bees → … (deterministic, no RNG).
//
// On spawn: PersonalHivePower(1). Acts as a passive counter (no
// per-power hooks); Spit reads it to branch.
//
// A0 payloads:
//   - Bees:  3 damage × 7 hits (DeadlyEnemies count → 8)
//   - Spear: 18 damage (DeadlyEnemies: 20)
//   - Spit:  if PersonalHive < 3 → apply +1 PersonalHive + 1 self-Str;
//            else → +2 self-Strength. (C# constant numbers.)

const ENTOMANCER_BEES_DAMAGE: i32 = 3;
const ENTOMANCER_BEES_HITS: i32 = 7;
const ENTOMANCER_SPEAR_DAMAGE: i32 = 18;
const ENTOMANCER_PERSONAL_HIVE_AMOUNT: i32 = 1;
const ENTOMANCER_SPIT_HIVE_CAP: i32 = 3;
const ENTOMANCER_SPIT_PRE_CAP_HIVE_GAIN: i32 = 1;
const ENTOMANCER_SPIT_PRE_CAP_STRENGTH_GAIN: i32 = 1;
const ENTOMANCER_SPIT_POST_CAP_STRENGTH_GAIN: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EntomancerIntent {
    Bees,
    Spear,
    Spit,
}

impl EntomancerIntent {
    pub fn id(self) -> &'static str {
        match self {
            EntomancerIntent::Bees => "BEES_MOVE",
            EntomancerIntent::Spear => "SPEAR_MOVE",
            EntomancerIntent::Spit => "PHEROMONE_SPIT_MOVE",
        }
    }
}

pub fn entomancer_spawn(cs: &mut CombatState, entomancer_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        entomancer_idx,
        "PersonalHivePower",
        ENTOMANCER_PERSONAL_HIVE_AMOUNT,
    );
}

pub fn pick_entomancer_intent(
    last_intent: Option<EntomancerIntent>,
) -> EntomancerIntent {
    match last_intent {
        None => EntomancerIntent::Bees,
        Some(EntomancerIntent::Bees) => EntomancerIntent::Spear,
        Some(EntomancerIntent::Spear) => EntomancerIntent::Spit,
        Some(EntomancerIntent::Spit) => EntomancerIntent::Bees,
    }
}

pub fn execute_entomancer_move(
    cs: &mut CombatState,
    entomancer_idx: usize,
    target_player_idx: usize,
    intent: EntomancerIntent,
) {
    let attacker = (CombatSide::Enemy, entomancer_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        EntomancerIntent::Bees => {
            for _ in 0..ENTOMANCER_BEES_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    ENTOMANCER_BEES_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        EntomancerIntent::Spear => {
            cs.deal_damage(
                attacker,
                player,
                ENTOMANCER_SPEAR_DAMAGE,
                ValueProp::MOVE,
            );
        }
        EntomancerIntent::Spit => {
            let hive = cs.get_power_amount(
                CombatSide::Enemy,
                entomancer_idx,
                "PersonalHivePower",
            );
            if hive < ENTOMANCER_SPIT_HIVE_CAP {
                cs.apply_power(
                    CombatSide::Enemy,
                    entomancer_idx,
                    "PersonalHivePower",
                    ENTOMANCER_SPIT_PRE_CAP_HIVE_GAIN,
                );
                cs.apply_power(
                    CombatSide::Enemy,
                    entomancer_idx,
                    "StrengthPower",
                    ENTOMANCER_SPIT_PRE_CAP_STRENGTH_GAIN,
                );
            } else {
                cs.apply_power(
                    CombatSide::Enemy,
                    entomancer_idx,
                    "StrengthPower",
                    ENTOMANCER_SPIT_POST_CAP_STRENGTH_GAIN,
                );
            }
        }
    }
}

// ---------- Monster intent: LivingShield -------------------------------
//
// Reflects C# `LivingShield.GenerateMoveStateMachine`:
//   Init: SHIELD_SLAM_MOVE.
//   Chain: ShieldSlam → ConditionalBranch(allies > 0 → ShieldSlam,
//                                          allies == 0 → Smash);
//          Smash → Smash (forever once alone).
//
// On spawn: applies RampartPower(25) — wired into tick_rampart_powers
// in begin_turn (Player-side only).
//
// A0 payloads:
//   - ShieldSlam: 6 damage (const)
//   - Smash:      16 damage (DeadlyEnemies: 18) + 3 self-Strength
//                 (const "EnrageStr")

const LIVING_SHIELD_SLAM_DAMAGE: i32 = 6;
const LIVING_SHIELD_SMASH_DAMAGE: i32 = 16;
const LIVING_SHIELD_ENRAGE_STRENGTH: i32 = 3;
const LIVING_SHIELD_RAMPART_AMOUNT: i32 = 25;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LivingShieldIntent {
    ShieldSlam,
    Smash,
}

impl LivingShieldIntent {
    pub fn id(self) -> &'static str {
        match self {
            LivingShieldIntent::ShieldSlam => "SHIELD_SLAM_MOVE",
            LivingShieldIntent::Smash => "SMASH_MOVE",
        }
    }
}

pub fn living_shield_spawn(cs: &mut CombatState, shield_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        shield_idx,
        "RampartPower",
        LIVING_SHIELD_RAMPART_AMOUNT,
    );
}

/// Pick LivingShield's next intent. `has_alive_allies` is the
/// caller-provided count check: in C# `GetAllyCount() > 0` (excludes
/// self, excludes dead). When alone, Smash forever.
pub fn pick_living_shield_intent(
    last_intent: Option<LivingShieldIntent>,
    has_alive_allies: bool,
) -> LivingShieldIntent {
    match last_intent {
        None => LivingShieldIntent::ShieldSlam,
        Some(LivingShieldIntent::ShieldSlam) => {
            if has_alive_allies {
                LivingShieldIntent::ShieldSlam
            } else {
                LivingShieldIntent::Smash
            }
        }
        Some(LivingShieldIntent::Smash) => LivingShieldIntent::Smash,
    }
}

pub fn execute_living_shield_move(
    cs: &mut CombatState,
    shield_idx: usize,
    target_player_idx: usize,
    intent: LivingShieldIntent,
) {
    let attacker = (CombatSide::Enemy, shield_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        LivingShieldIntent::ShieldSlam => {
            cs.deal_damage(
                attacker,
                player,
                LIVING_SHIELD_SLAM_DAMAGE,
                ValueProp::MOVE,
            );
        }
        LivingShieldIntent::Smash => {
            cs.deal_damage(
                attacker,
                player,
                LIVING_SHIELD_SMASH_DAMAGE,
                ValueProp::MOVE,
            );
            cs.apply_power(
                CombatSide::Enemy,
                shield_idx,
                "StrengthPower",
                LIVING_SHIELD_ENRAGE_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: ShrinkerBeetle -----------------------------
//
// Reflects C# `ShrinkerBeetle.GenerateMoveStateMachine`:
//   Init: SHRINKER_MOVE.
//   Chain: Shrinker → Chomp → Stomp → Chomp → Stomp → … (Chomp↔Stomp
//   forever after Shrinker fires once).
//
// Shrinker applies ShrinkPower(-1) to the player — the negative
// Amount makes it "infinite" (never ticks down). ShrinkPower's
// damage multiplier (×0.70 on owner-side powered attacks) flows
// through power_multiplicative_dealer.
//
// A0 payloads:
//   - Chomp: 7 damage (DeadlyEnemies: 8)
//   - Stomp: 13 damage (DeadlyEnemies: 14)

const SHRINKER_BEETLE_CHOMP_DAMAGE: i32 = 7;
const SHRINKER_BEETLE_STOMP_DAMAGE: i32 = 13;
const SHRINKER_BEETLE_SHRINK_AMOUNT: i32 = -1;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ShrinkerBeetleIntent {
    Shrinker,
    Chomp,
    Stomp,
}

impl ShrinkerBeetleIntent {
    pub fn id(self) -> &'static str {
        match self {
            ShrinkerBeetleIntent::Shrinker => "SHRINKER_MOVE",
            ShrinkerBeetleIntent::Chomp => "CHOMP_MOVE",
            ShrinkerBeetleIntent::Stomp => "STOMP_MOVE",
        }
    }
}

pub fn pick_shrinker_beetle_intent(
    last_intent: Option<ShrinkerBeetleIntent>,
) -> ShrinkerBeetleIntent {
    match last_intent {
        None => ShrinkerBeetleIntent::Shrinker,
        // After Shrinker the chain enters Chomp ↔ Stomp alternation
        // forever (Shrinker FollowUpState = Chomp; Chomp ↔ Stomp).
        Some(ShrinkerBeetleIntent::Shrinker) => ShrinkerBeetleIntent::Chomp,
        Some(ShrinkerBeetleIntent::Chomp) => ShrinkerBeetleIntent::Stomp,
        Some(ShrinkerBeetleIntent::Stomp) => ShrinkerBeetleIntent::Chomp,
    }
}

pub fn execute_shrinker_beetle_move(
    cs: &mut CombatState,
    beetle_idx: usize,
    target_player_idx: usize,
    intent: ShrinkerBeetleIntent,
) {
    let attacker = (CombatSide::Enemy, beetle_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        ShrinkerBeetleIntent::Shrinker => {
            // C# applies ShrinkPower(-1m). Our apply_power supports
            // negative amounts on AllowNegative=true powers (Shrink
            // is AllowNegative=true) — but we also need the power
            // stack to be visible in observation. Use apply_power
            // directly; the negative amount represents "infinite"
            // per ShrinkPower.IsInfinite.
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "ShrinkPower",
                SHRINKER_BEETLE_SHRINK_AMOUNT,
            );
        }
        ShrinkerBeetleIntent::Chomp => {
            cs.deal_damage(
                attacker,
                player,
                SHRINKER_BEETLE_CHOMP_DAMAGE,
                ValueProp::MOVE,
            );
        }
        ShrinkerBeetleIntent::Stomp => {
            cs.deal_damage(
                attacker,
                player,
                SHRINKER_BEETLE_STOMP_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: Byrdonis -----------------------------------
//
// Reflects C# `Byrdonis.GenerateMoveStateMachine`. Two-state strict
// alternation, init Swoop.
//   Init: SWOOP_MOVE
//   Cycle: Swoop ↔ Peck
//
// On spawn: applies TerritorialPower(1). Each owner-side turn end
// then applies StrengthPower(Amount) to itself — permanent ramp
// wired into end_turn via tick_territorial_powers.
//
// A0 payloads:
//   - Peck:  3 damage × 3 hits (DeadlyEnemies: 4)
//   - Swoop: 17 damage (DeadlyEnemies: 19)

const BYRDONIS_PECK_DAMAGE: i32 = 3;
const BYRDONIS_PECK_HITS: i32 = 3;
const BYRDONIS_SWOOP_DAMAGE: i32 = 17;
const BYRDONIS_TERRITORIAL_AMOUNT: i32 = 1;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ByrdonisIntent {
    Peck,
    Swoop,
}

impl ByrdonisIntent {
    pub fn id(self) -> &'static str {
        match self {
            ByrdonisIntent::Peck => "PECK_MOVE",
            ByrdonisIntent::Swoop => "SWOOP_MOVE",
        }
    }
}

/// Byrdonis spawn payload — apply TerritorialPower(1).
pub fn byrdonis_spawn(cs: &mut CombatState, byrdonis_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        byrdonis_idx,
        "TerritorialPower",
        BYRDONIS_TERRITORIAL_AMOUNT,
    );
}

pub fn pick_byrdonis_intent(
    last_intent: Option<ByrdonisIntent>,
) -> ByrdonisIntent {
    match last_intent {
        None => ByrdonisIntent::Swoop,
        Some(ByrdonisIntent::Swoop) => ByrdonisIntent::Peck,
        Some(ByrdonisIntent::Peck) => ByrdonisIntent::Swoop,
    }
}

pub fn execute_byrdonis_move(
    cs: &mut CombatState,
    byrdonis_idx: usize,
    target_player_idx: usize,
    intent: ByrdonisIntent,
) {
    let attacker = (CombatSide::Enemy, byrdonis_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        ByrdonisIntent::Peck => {
            for _ in 0..BYRDONIS_PECK_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    BYRDONIS_PECK_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        ByrdonisIntent::Swoop => {
            cs.deal_damage(
                attacker,
                player,
                BYRDONIS_SWOOP_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: Chomper ------------------------------------
//
// Reflects C# `Chomper.GenerateMoveStateMachine`:
//   On spawn: applies ArtifactPower(2). Power stack tracked but the
//   debuff-absorb behavior (Artifact eats N debuffs before they apply)
//   is deferred — would need an AfterApplied hook on debuffs.
//
//   Init: scream_first flag-based:
//     - scream_first=true → Screech
//     - scream_first=false (default) → Clamp
//   Cycle: Clamp ↔ Screech strict alternation.
//
// A0 payloads:
//   - Clamp: 8 damage × 2 hits (DeadlyEnemies: 9)
//   - Screech: add 3 Dazed status cards to player's discard

const CHOMPER_CLAMP_DAMAGE: i32 = 8;
const CHOMPER_CLAMP_HITS: i32 = 2;
const CHOMPER_SCREECH_DAZED_COUNT: i32 = 3;
const CHOMPER_ARTIFACT_AMOUNT: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ChomperIntent {
    Clamp,
    Screech,
}

impl ChomperIntent {
    pub fn id(self) -> &'static str {
        match self {
            ChomperIntent::Clamp => "CLAMP_MOVE",
            ChomperIntent::Screech => "SCREECH_MOVE",
        }
    }
}

/// The Chomper spawn payload — caller invokes once when the chomper
/// is added to combat. Mirrors C# `AfterAddedToRoom`.
pub fn chomper_spawn(cs: &mut CombatState, chomper_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        chomper_idx,
        "ArtifactPower",
        CHOMPER_ARTIFACT_AMOUNT,
    );
}

/// Pick Chomper's next intent. scream_first=true → init Screech, else
/// init Clamp. Cycle: Clamp ↔ Screech.
pub fn pick_chomper_intent(
    last_intent: Option<ChomperIntent>,
    scream_first: bool,
) -> ChomperIntent {
    match last_intent {
        None if scream_first => ChomperIntent::Screech,
        None => ChomperIntent::Clamp,
        Some(ChomperIntent::Clamp) => ChomperIntent::Screech,
        Some(ChomperIntent::Screech) => ChomperIntent::Clamp,
    }
}

pub fn execute_chomper_move(
    cs: &mut CombatState,
    chomper_idx: usize,
    target_player_idx: usize,
    intent: ChomperIntent,
) {
    let attacker = (CombatSide::Enemy, chomper_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        ChomperIntent::Clamp => {
            for _ in 0..CHOMPER_CLAMP_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    CHOMPER_CLAMP_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        ChomperIntent::Screech => {
            for _ in 0..CHOMPER_SCREECH_DAZED_COUNT {
                cs.add_card_to_pile(
                    target_player_idx,
                    "Dazed",
                    0,
                    PileType::Discard,
                );
            }
        }
    }
}

// ---------- Monster intent: TurretOperator -----------------------------
//
// Reflects C# `TurretOperator.GenerateMoveStateMachine`. Deterministic
// 3-state cycle:
//   Unload1 → Unload2 → Reload → Unload1 → …
// Unload1 and Unload2 have identical payloads (separate C# nodes for
// the chain order). Kept as distinct variants because they live in
// different state positions.
//
// A0 payloads:
//   - Unload (1 or 2): 3 damage × 5 hits (DeadlyEnemies: 4)
//   - Reload: +1 self-Strength

const TURRET_OPERATOR_FIRE_DAMAGE: i32 = 3;
const TURRET_OPERATOR_FIRE_HITS: i32 = 5;
const TURRET_OPERATOR_RELOAD_STRENGTH: i32 = 1;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TurretOperatorIntent {
    Unload1,
    Unload2,
    Reload,
}

impl TurretOperatorIntent {
    pub fn id(self) -> &'static str {
        match self {
            TurretOperatorIntent::Unload1 => "UNLOAD_MOVE_1",
            TurretOperatorIntent::Unload2 => "UNLOAD_MOVE_2",
            TurretOperatorIntent::Reload => "RELOAD_MOVE",
        }
    }
}

pub fn pick_turret_operator_intent(
    last_intent: Option<TurretOperatorIntent>,
) -> TurretOperatorIntent {
    match last_intent {
        None => TurretOperatorIntent::Unload1,
        Some(TurretOperatorIntent::Unload1) => TurretOperatorIntent::Unload2,
        Some(TurretOperatorIntent::Unload2) => TurretOperatorIntent::Reload,
        Some(TurretOperatorIntent::Reload) => TurretOperatorIntent::Unload1,
    }
}

pub fn execute_turret_operator_move(
    cs: &mut CombatState,
    turret_idx: usize,
    target_player_idx: usize,
    intent: TurretOperatorIntent,
) {
    let attacker = (CombatSide::Enemy, turret_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        TurretOperatorIntent::Unload1 | TurretOperatorIntent::Unload2 => {
            for _ in 0..TURRET_OPERATOR_FIRE_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    TURRET_OPERATOR_FIRE_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        TurretOperatorIntent::Reload => {
            cs.apply_power(
                CombatSide::Enemy,
                turret_idx,
                "StrengthPower",
                TURRET_OPERATOR_RELOAD_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: TwigSlimeM ---------------------------------
//
// Reflects C# `TwigSlimeM.GenerateMoveStateMachine`:
//   Init: STICKY_SHOT_MOVE.
//   After Sticky: RandomBranch (Clump weight 2, Sticky CannotRepeat
//     → blocked when last was Sticky). So after Sticky always Clump.
//   After Clump: RandomBranch (Clump weight 2, Sticky weight 1 default
//     → 67/33).
//
// A0 payloads:
//   - ClumpShot: 11 damage (DeadlyEnemies: 12)
//   - StickyShot: add 1 Slimed to discard (const)

const TWIG_SLIME_M_CLUMP_DAMAGE: i32 = 11;
const TWIG_SLIME_M_STICKY_COUNT: i32 = 1;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TwigSlimeMIntent {
    Clump,
    Sticky,
}

impl TwigSlimeMIntent {
    pub fn id(self) -> &'static str {
        match self {
            TwigSlimeMIntent::Clump => "CLUMP_SHOT_MOVE",
            TwigSlimeMIntent::Sticky => "STICKY_SHOT_MOVE",
        }
    }
}

pub fn pick_twig_slime_m_intent(
    rng: &mut Rng,
    last_intent: Option<TwigSlimeMIntent>,
) -> TwigSlimeMIntent {
    match last_intent {
        None => TwigSlimeMIntent::Sticky,
        Some(TwigSlimeMIntent::Sticky) => {
            // Sticky CannotRepeat → Clump wins always.
            TwigSlimeMIntent::Clump
        }
        Some(TwigSlimeMIntent::Clump) => {
            let w_clump: f32 = 2.0;
            let w_sticky: f32 = 1.0;
            let total = w_clump + w_sticky;
            let mut roll = rng.next_float(total);
            roll -= w_clump;
            if roll <= 0.0 {
                return TwigSlimeMIntent::Clump;
            }
            TwigSlimeMIntent::Sticky
        }
    }
}

pub fn execute_twig_slime_m_move(
    cs: &mut CombatState,
    slime_idx: usize,
    target_player_idx: usize,
    intent: TwigSlimeMIntent,
) {
    let attacker = (CombatSide::Enemy, slime_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        TwigSlimeMIntent::Clump => {
            cs.deal_damage(
                attacker,
                player,
                TWIG_SLIME_M_CLUMP_DAMAGE,
                ValueProp::MOVE,
            );
        }
        TwigSlimeMIntent::Sticky => {
            for _ in 0..TWIG_SLIME_M_STICKY_COUNT {
                cs.add_card_to_pile(
                    target_player_idx,
                    "Slimed",
                    0,
                    PileType::Discard,
                );
            }
        }
    }
}

// ---------- Monster intent: LeafSlimeM ---------------------------------
//
// Reflects C# `LeafSlimeM.GenerateMoveStateMachine`. Deterministic
// alternation:
//   Init: STICKY_SHOT.
//   Cycle: Sticky → Clump → Sticky → … (strict alternation).
//
// A0 payloads:
//   - ClumpShot: 8 damage (DeadlyEnemies: 9)
//   - StickyShot: add 2 Slimed to discard (const)

const LEAF_SLIME_M_CLUMP_DAMAGE: i32 = 8;
const LEAF_SLIME_M_STICKY_COUNT: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LeafSlimeMIntent {
    Clump,
    Sticky,
}

impl LeafSlimeMIntent {
    pub fn id(self) -> &'static str {
        match self {
            LeafSlimeMIntent::Clump => "CLUMP_SHOT",
            LeafSlimeMIntent::Sticky => "STICKY_SHOT",
        }
    }
}

pub fn pick_leaf_slime_m_intent(
    last_intent: Option<LeafSlimeMIntent>,
) -> LeafSlimeMIntent {
    match last_intent {
        None => LeafSlimeMIntent::Sticky,
        Some(LeafSlimeMIntent::Sticky) => LeafSlimeMIntent::Clump,
        Some(LeafSlimeMIntent::Clump) => LeafSlimeMIntent::Sticky,
    }
}

pub fn execute_leaf_slime_m_move(
    cs: &mut CombatState,
    slime_idx: usize,
    target_player_idx: usize,
    intent: LeafSlimeMIntent,
) {
    let attacker = (CombatSide::Enemy, slime_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        LeafSlimeMIntent::Clump => {
            cs.deal_damage(
                attacker,
                player,
                LEAF_SLIME_M_CLUMP_DAMAGE,
                ValueProp::MOVE,
            );
        }
        LeafSlimeMIntent::Sticky => {
            for _ in 0..LEAF_SLIME_M_STICKY_COUNT {
                cs.add_card_to_pile(
                    target_player_idx,
                    "Slimed",
                    0,
                    PileType::Discard,
                );
            }
        }
    }
}

// ---------- Monster intent: TwigSlimeS ---------------------------------
//
// Reflects C# `TwigSlimeS.GenerateMoveStateMachine`. Trivial: single
// Butt move that always loops. No state branching, no RNG, no powers.
//
// A0 payload: Butt 4 damage (DeadlyEnemies: 5).

const TWIG_SLIME_S_BUTT_DAMAGE: i32 = 4;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TwigSlimeSIntent {
    Butt,
}

impl TwigSlimeSIntent {
    pub fn id(self) -> &'static str {
        match self {
            TwigSlimeSIntent::Butt => "BUTT_MOVE",
        }
    }
}

pub fn pick_twig_slime_s_intent(
    _last_intent: Option<TwigSlimeSIntent>,
) -> TwigSlimeSIntent {
    TwigSlimeSIntent::Butt
}

pub fn execute_twig_slime_s_move(
    cs: &mut CombatState,
    slime_idx: usize,
    target_player_idx: usize,
    intent: TwigSlimeSIntent,
) {
    let attacker = (CombatSide::Enemy, slime_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        TwigSlimeSIntent::Butt => {
            cs.deal_damage(
                attacker,
                player,
                TWIG_SLIME_S_BUTT_DAMAGE,
                ValueProp::MOVE,
            );
        }
    }
}

// ---------- Monster intent: LeafSlimeS ---------------------------------
//
// Reflects C# `LeafSlimeS.GenerateMoveStateMachine`:
//   Two-move random pick with CannotRepeat on both branches.
//     - Butt (3 damage)
//     - Goop (add 1 Slimed status card to player's discard pile)
//   Init = RandomBranch directly (no fixed first move). Each turn the
//   pick excludes the last-played move.
//
// A0 payloads:
//   - Butt: 3 damage (DeadlyEnemies: 4)
//   - Goop: 1 Slimed to discard (const)

const LEAF_SLIME_S_BUTT_DAMAGE: i32 = 3;
const LEAF_SLIME_S_GOOP_COUNT: i32 = 1;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LeafSlimeSIntent {
    Butt,
    Goop,
}

impl LeafSlimeSIntent {
    pub fn id(self) -> &'static str {
        match self {
            LeafSlimeSIntent::Butt => "BUTT_MOVE",
            LeafSlimeSIntent::Goop => "GOOP_MOVE",
        }
    }
}

/// Pick LeafSlimeS's next intent. Random 50/50 first turn; subsequent
/// turns exclude the last-played move (both branches CannotRepeat).
/// Together this gives a strict alternation after turn 1, but first
/// turn is RNG-determined.
pub fn pick_leaf_slime_s_intent(
    rng: &mut Rng,
    last_intent: Option<LeafSlimeSIntent>,
) -> LeafSlimeSIntent {
    match last_intent {
        None => {
            // 50/50 RandomBranch — both branches weight 1.
            let w_butt: f32 = 1.0;
            let w_goop: f32 = 1.0;
            let total = w_butt + w_goop;
            let mut roll = rng.next_float(total);
            roll -= w_butt;
            if roll <= 0.0 {
                return LeafSlimeSIntent::Butt;
            }
            LeafSlimeSIntent::Goop
        }
        Some(LeafSlimeSIntent::Butt) => LeafSlimeSIntent::Goop,
        Some(LeafSlimeSIntent::Goop) => LeafSlimeSIntent::Butt,
    }
}

pub fn execute_leaf_slime_s_move(
    cs: &mut CombatState,
    slime_idx: usize,
    target_player_idx: usize,
    intent: LeafSlimeSIntent,
) {
    let attacker = (CombatSide::Enemy, slime_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        LeafSlimeSIntent::Butt => {
            cs.deal_damage(
                attacker,
                player,
                LEAF_SLIME_S_BUTT_DAMAGE,
                ValueProp::MOVE,
            );
        }
        LeafSlimeSIntent::Goop => {
            for _ in 0..LEAF_SLIME_S_GOOP_COUNT {
                cs.add_card_to_pile(
                    target_player_idx,
                    "Slimed",
                    0,
                    PileType::Discard,
                );
            }
        }
    }
}

// ---------- Monster intent: Seapunk ------------------------------------
//
// Reflects C# `Seapunk.GenerateMoveStateMachine`:
//   Init: SEA_KICK_MOVE.
//   Cycle: SeaKick → SpinningKick → BubbleBurp → SeaKick → …
//   No RNG.
//
// A0 payloads:
//   - SeaKick: 11 damage (DeadlyEnemies: 13)
//   - SpinningKick: 2 damage × 4 hits (consts)
//   - BubbleBurp: 7 self-block (ToughEnemies: 8) + 1 self-Strength
//       (DeadlyEnemies: 2)

const SEAPUNK_SEA_KICK_DAMAGE: i32 = 11;
const SEAPUNK_SPINNING_KICK_DAMAGE: i32 = 2;
const SEAPUNK_SPINNING_KICK_HITS: i32 = 4;
const SEAPUNK_BUBBLE_BLOCK: i32 = 7;
const SEAPUNK_BUBBLE_STRENGTH: i32 = 1;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SeapunkIntent {
    SeaKick,
    SpinningKick,
    BubbleBurp,
}

impl SeapunkIntent {
    pub fn id(self) -> &'static str {
        match self {
            SeapunkIntent::SeaKick => "SEA_KICK_MOVE",
            SeapunkIntent::SpinningKick => "SPINNING_KICK_MOVE",
            SeapunkIntent::BubbleBurp => "BUBBLE_BURP_MOVE",
        }
    }
}

/// Pick Seapunk's next intent. Init → SeaKick, then deterministic
/// cycle SeaKick → SpinningKick → BubbleBurp → SeaKick.
pub fn pick_seapunk_intent(last_intent: Option<SeapunkIntent>) -> SeapunkIntent {
    match last_intent {
        None => SeapunkIntent::SeaKick,
        Some(SeapunkIntent::SeaKick) => SeapunkIntent::SpinningKick,
        Some(SeapunkIntent::SpinningKick) => SeapunkIntent::BubbleBurp,
        Some(SeapunkIntent::BubbleBurp) => SeapunkIntent::SeaKick,
    }
}

/// Execute one Seapunk move's payload.
pub fn execute_seapunk_move(
    cs: &mut CombatState,
    seapunk_idx: usize,
    target_player_idx: usize,
    intent: SeapunkIntent,
) {
    let attacker = (CombatSide::Enemy, seapunk_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        SeapunkIntent::SeaKick => {
            cs.deal_damage(
                attacker,
                player,
                SEAPUNK_SEA_KICK_DAMAGE,
                ValueProp::MOVE,
            );
        }
        SeapunkIntent::SpinningKick => {
            for _ in 0..SEAPUNK_SPINNING_KICK_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    SEAPUNK_SPINNING_KICK_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        SeapunkIntent::BubbleBurp => {
            cs.gain_block(
                CombatSide::Enemy,
                seapunk_idx,
                SEAPUNK_BUBBLE_BLOCK,
            );
            cs.apply_power(
                CombatSide::Enemy,
                seapunk_idx,
                "StrengthPower",
                SEAPUNK_BUBBLE_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: CorpseSlug ---------------------------------
//
// Reflects C# `CorpseSlug.GenerateMoveStateMachine`:
//   On spawn: applies RavenousPower(4). The full RavenousPower hook
//   (AfterDeath → eat dead teammate → Stun for 1 turn → gain Strength
//   = Amount permanently) is deferred — needs the Stun mechanic and
//   per-monster IsRavenous flag. For now the Power stack is just
//   present on the slug (visible in observation) but doesn't fire on
//   teammate death.
//
//   Init: starter_move_idx % 3:
//     0 → WhipSlap   1 → Glomp   2 (default) → Goop
//
//   Cycle (FollowUpState chain, no RNG):
//     WhipSlap → Glomp → Goop → WhipSlap → …
//
// A0 payloads:
//   - WhipSlap: 3 damage × 2 hits (const)
//   - Glomp:    8 damage (DeadlyEnemies: 9)
//   - Goop:     apply 2 Frail (const) — uses FrailPower which we have
//   - Ravenous strength gain (deferred): +4 (DeadlyEnemies: 5)

const CORPSE_SLUG_WHIP_SLAP_DAMAGE: i32 = 3;
const CORPSE_SLUG_WHIP_SLAP_HITS: i32 = 2;
const CORPSE_SLUG_GLOMP_DAMAGE: i32 = 8;
const CORPSE_SLUG_GOOP_FRAIL: i32 = 2;
const CORPSE_SLUG_RAVENOUS_AMOUNT: i32 = 4;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CorpseSlugIntent {
    WhipSlap,
    Glomp,
    Goop,
}

impl CorpseSlugIntent {
    pub fn id(self) -> &'static str {
        match self {
            CorpseSlugIntent::WhipSlap => "WHIP_SLAP_MOVE",
            CorpseSlugIntent::Glomp => "GLOMP_MOVE",
            CorpseSlugIntent::Goop => "GOOP_MOVE",
        }
    }
}

/// The CorpseSlug spawn payload — caller invokes this once when the
/// slug is added to combat. Equivalent to C# `AfterAddedToRoom`.
pub fn corpse_slug_spawn(cs: &mut CombatState, slug_idx: usize) {
    cs.apply_power(
        CombatSide::Enemy,
        slug_idx,
        "RavenousPower",
        CORPSE_SLUG_RAVENOUS_AMOUNT,
    );
}

/// Pick CorpseSlug's next intent.
///   - First turn: starter_move_idx % 3 → WhipSlap / Glomp / Goop.
///   - Subsequent: deterministic cycle WhipSlap → Glomp → Goop.
pub fn pick_corpse_slug_intent(
    last_intent: Option<CorpseSlugIntent>,
    starter_move_idx: i32,
) -> CorpseSlugIntent {
    match last_intent {
        None => match starter_move_idx.rem_euclid(3) {
            0 => CorpseSlugIntent::WhipSlap,
            1 => CorpseSlugIntent::Glomp,
            _ => CorpseSlugIntent::Goop,
        },
        Some(CorpseSlugIntent::WhipSlap) => CorpseSlugIntent::Glomp,
        Some(CorpseSlugIntent::Glomp) => CorpseSlugIntent::Goop,
        Some(CorpseSlugIntent::Goop) => CorpseSlugIntent::WhipSlap,
    }
}

/// Execute one CorpseSlug move's payload.
pub fn execute_corpse_slug_move(
    cs: &mut CombatState,
    slug_idx: usize,
    target_player_idx: usize,
    intent: CorpseSlugIntent,
) {
    let attacker = (CombatSide::Enemy, slug_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        CorpseSlugIntent::WhipSlap => {
            for _ in 0..CORPSE_SLUG_WHIP_SLAP_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    CORPSE_SLUG_WHIP_SLAP_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        CorpseSlugIntent::Glomp => {
            cs.deal_damage(
                attacker,
                player,
                CORPSE_SLUG_GLOMP_DAMAGE,
                ValueProp::MOVE,
            );
        }
        CorpseSlugIntent::Goop => {
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "FrailPower",
                CORPSE_SLUG_GOOP_FRAIL,
            );
        }
    }
}

// ---------- Monster intent: ScrollOfBiting -----------------------------
//
// Reflects C# `ScrollOfBiting.GenerateMoveStateMachine`:
//   On spawn: applies PaperCutsPower(2) — wired into combat via the
//   AfterDamageGiven hook (deal max_hp loss when owner damages player
//   through block).
//
//   Init: branches on `StarterMoveIdx % 3`:
//     - 0 → Chomp
//     - 1 → Chew
//     - 2 (and default) → MoreTeeth
//
//   FollowUpState chain:
//     Chomp     → MoreTeeth
//     MoreTeeth → Chew
//     Chew      → RandomBranch(Chomp CannotRepeat, Chew weight 2)
//
//   Random pick (after Chew): weights Chomp=1 (CannotRepeat blocks
//   when last was Chomp — but the path here means last was Chew, so
//   Chomp is always allowed; weight 1) and Chew=2. The CannotRepeat
//   guard would only matter if the random branch re-fires with last
//   being a Chomp (Chomp→MoreTeeth, so it never does — kept for
//   fidelity).
//
// A0 payloads:
//   - Chomp: 14 damage (DeadlyEnemies: 16)
//   - Chew: 5 damage × 2 hits (DeadlyEnemies: 6)
//   - MoreTeeth: +2 self-Strength (const)

const SCROLL_OF_BITING_CHOMP_DAMAGE: i32 = 14;
const SCROLL_OF_BITING_CHEW_DAMAGE: i32 = 5;
const SCROLL_OF_BITING_CHEW_HITS: i32 = 2;
const SCROLL_OF_BITING_MORE_TEETH_STRENGTH: i32 = 2;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ScrollOfBitingIntent {
    Chomp,
    Chew,
    MoreTeeth,
}

impl ScrollOfBitingIntent {
    pub fn id(self) -> &'static str {
        match self {
            ScrollOfBitingIntent::Chomp => "CHOMP",
            ScrollOfBitingIntent::Chew => "CHEW",
            ScrollOfBitingIntent::MoreTeeth => "MORE_TEETH",
        }
    }
}

/// Pick ScrollOfBiting's next intent.
///   - First turn: branch on `starter_move_idx % 3`.
///   - Subsequent: deterministic chain Chomp→MoreTeeth→Chew→Random.
///     After Chew the picker rolls RNG: weight Chomp=1, Chew=2.
pub fn pick_scroll_of_biting_intent(
    rng: &mut Rng,
    last_intent: Option<ScrollOfBitingIntent>,
    starter_move_idx: i32,
) -> ScrollOfBitingIntent {
    match last_intent {
        None => match starter_move_idx.rem_euclid(3) {
            0 => ScrollOfBitingIntent::Chomp,
            1 => ScrollOfBitingIntent::Chew,
            _ => ScrollOfBitingIntent::MoreTeeth,
        },
        Some(ScrollOfBitingIntent::Chomp) => ScrollOfBitingIntent::MoreTeeth,
        Some(ScrollOfBitingIntent::MoreTeeth) => ScrollOfBitingIntent::Chew,
        Some(ScrollOfBitingIntent::Chew) => {
            // Random pick: Chomp (1) + Chew (2). CannotRepeat on Chomp
            // would only block if last was Chomp, which the chain
            // forbids here — leave the guard implicit.
            let w_chomp: f32 = 1.0;
            let w_chew: f32 = 2.0;
            let total = w_chomp + w_chew;
            let mut roll = rng.next_float(total);
            roll -= w_chomp;
            if roll <= 0.0 {
                return ScrollOfBitingIntent::Chomp;
            }
            ScrollOfBitingIntent::Chew
        }
    }
}

/// Execute one ScrollOfBiting move's payload.
pub fn execute_scroll_of_biting_move(
    cs: &mut CombatState,
    scroll_idx: usize,
    target_player_idx: usize,
    intent: ScrollOfBitingIntent,
) {
    let attacker = (CombatSide::Enemy, scroll_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        ScrollOfBitingIntent::Chomp => {
            cs.deal_damage(
                attacker,
                player,
                SCROLL_OF_BITING_CHOMP_DAMAGE,
                ValueProp::MOVE,
            );
        }
        ScrollOfBitingIntent::Chew => {
            for _ in 0..SCROLL_OF_BITING_CHEW_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    SCROLL_OF_BITING_CHEW_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        ScrollOfBitingIntent::MoreTeeth => {
            cs.apply_power(
                CombatSide::Enemy,
                scroll_idx,
                "StrengthPower",
                SCROLL_OF_BITING_MORE_TEETH_STRENGTH,
            );
        }
    }
}

// ---------- Monster intent: BowlbugSilk --------------------------------
//
// Reflects C# `BowlbugSilk.GenerateMoveStateMachine`. Two-state
// alternating cycle starting at ToxicSpit:
//   ToxicSpit ↔ Trash (forever)
//
// A0 payloads:
//   - Trash: 4 damage × 2 hits (DeadlyEnemies: 5 per hit)
//   - ToxicSpit: apply 1 Weak to target

const BOWLBUG_SILK_TRASH_DAMAGE: i32 = 4;
const BOWLBUG_SILK_TRASH_HITS: i32 = 2;
const BOWLBUG_SILK_WEAK: i32 = 1;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BowlbugSilkIntent {
    Trash,
    ToxicSpit,
}

impl BowlbugSilkIntent {
    pub fn id(self) -> &'static str {
        match self {
            BowlbugSilkIntent::Trash => "TRASH_MOVE",
            BowlbugSilkIntent::ToxicSpit => "TOXIC_SPIT_MOVE",
        }
    }
}

/// Pick BowlbugSilk's next intent. Init = ToxicSpit, then alternate.
pub fn pick_bowlbug_silk_intent(
    last_intent: Option<BowlbugSilkIntent>,
) -> BowlbugSilkIntent {
    match last_intent {
        None => BowlbugSilkIntent::ToxicSpit,
        Some(BowlbugSilkIntent::ToxicSpit) => BowlbugSilkIntent::Trash,
        Some(BowlbugSilkIntent::Trash) => BowlbugSilkIntent::ToxicSpit,
    }
}

/// Execute BowlbugSilk's move payload.
pub fn execute_bowlbug_silk_move(
    cs: &mut CombatState,
    silk_idx: usize,
    target_player_idx: usize,
    intent: BowlbugSilkIntent,
) {
    let attacker = (CombatSide::Enemy, silk_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        BowlbugSilkIntent::Trash => {
            for _ in 0..BOWLBUG_SILK_TRASH_HITS {
                cs.deal_damage(
                    attacker,
                    player,
                    BOWLBUG_SILK_TRASH_DAMAGE,
                    ValueProp::MOVE,
                );
            }
        }
        BowlbugSilkIntent::ToxicSpit => {
            cs.apply_power(
                CombatSide::Player,
                target_player_idx,
                "WeakPower",
                BOWLBUG_SILK_WEAK,
            );
        }
    }
}

// ---------- Monster intent: BowlbugNectar ------------------------------
//
// Reflects C# `BowlbugNectar.GenerateMoveStateMachine`. Deterministic
// 3-state sequence:
//   Thrash → Buff → Thrash2 → Thrash2 → Thrash2 → … (Thrash2 loops)
//
// Thrash and Thrash2 do the same payload (separate C# nodes for the
// state-machine sequencing). We keep them as distinct variants because
// they live in different state-machine positions — Thrash leads to
// Buff (one-time), Thrash2 self-loops. Collapsing them would re-fire
// Buff on every other turn (incorrect).
//
// A0 payloads:
//   - Thrash / Thrash2: 3 damage (const)
//   - Buff:             +15 self-Strength (DeadlyEnemies: 16)

const BOWLBUG_NECTAR_THRASH_DAMAGE: i32 = 3;
const BOWLBUG_NECTAR_BUFF_STRENGTH_GAIN: i32 = 15;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BowlbugNectarIntent {
    Thrash,
    Buff,
    Thrash2,
}

impl BowlbugNectarIntent {
    pub fn id(self) -> &'static str {
        match self {
            BowlbugNectarIntent::Thrash => "THRASH_MOVE",
            BowlbugNectarIntent::Buff => "BUFF_MOVE",
            BowlbugNectarIntent::Thrash2 => "THRASH2_MOVE",
        }
    }
}

/// Pick BowlbugNectar's next intent. Fully deterministic chain:
///   None    → Thrash
///   Thrash  → Buff (one-time)
///   Buff    → Thrash2
///   Thrash2 → Thrash2 (forever)
pub fn pick_bowlbug_nectar_intent(
    last_intent: Option<BowlbugNectarIntent>,
) -> BowlbugNectarIntent {
    match last_intent {
        None => BowlbugNectarIntent::Thrash,
        Some(BowlbugNectarIntent::Thrash) => BowlbugNectarIntent::Buff,
        Some(BowlbugNectarIntent::Buff) => BowlbugNectarIntent::Thrash2,
        Some(BowlbugNectarIntent::Thrash2) => BowlbugNectarIntent::Thrash2,
    }
}

/// Execute BowlbugNectar's move payload.
pub fn execute_bowlbug_nectar_move(
    cs: &mut CombatState,
    nectar_idx: usize,
    target_player_idx: usize,
    intent: BowlbugNectarIntent,
) {
    let attacker = (CombatSide::Enemy, nectar_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        BowlbugNectarIntent::Thrash | BowlbugNectarIntent::Thrash2 => {
            cs.deal_damage(
                attacker,
                player,
                BOWLBUG_NECTAR_THRASH_DAMAGE,
                ValueProp::MOVE,
            );
        }
        BowlbugNectarIntent::Buff => {
            cs.apply_power(
                CombatSide::Enemy,
                nectar_idx,
                "StrengthPower",
                BOWLBUG_NECTAR_BUFF_STRENGTH_GAIN,
            );
        }
    }
}

// ---------- Monster intent: BowlbugEgg ---------------------------------
//
// Reflects C# `BowlbugEgg.GenerateMoveStateMachine`: a single move
// (Bite) whose FollowUpState points back to itself. No state branching,
// no RNG. Always plays Bite every turn.
//
// A0 payloads:
//   - Bite: 7 damage + 7 self-block (DeadlyEnemies: 8 / 8)

const BOWLBUG_EGG_BITE_DAMAGE: i32 = 7;
const BOWLBUG_EGG_PROTECT_BLOCK: i32 = 7;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BowlbugEggIntent {
    Bite,
}

impl BowlbugEggIntent {
    pub fn id(self) -> &'static str {
        match self {
            BowlbugEggIntent::Bite => "BITE_MOVE",
        }
    }
}

/// Pick BowlbugEgg's next intent. Trivial: always Bite.
pub fn pick_bowlbug_egg_intent(
    _last_intent: Option<BowlbugEggIntent>,
) -> BowlbugEggIntent {
    BowlbugEggIntent::Bite
}

/// Execute BowlbugEgg's move payload. Bite: deal damage + gain block.
pub fn execute_bowlbug_egg_move(
    cs: &mut CombatState,
    egg_idx: usize,
    target_player_idx: usize,
    intent: BowlbugEggIntent,
) {
    let attacker = (CombatSide::Enemy, egg_idx);
    let player = (CombatSide::Player, target_player_idx);
    match intent {
        BowlbugEggIntent::Bite => {
            cs.deal_damage(
                attacker,
                player,
                BOWLBUG_EGG_BITE_DAMAGE,
                ValueProp::MOVE,
            );
            cs.gain_block(
                CombatSide::Enemy,
                egg_idx,
                BOWLBUG_EGG_PROTECT_BLOCK,
            );
        }
    }
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
    /// A card was played by the named player. Emitted by `play_card`
    /// before OnPlay runs. Drives `AmountSpec::CardsPlayedThisTurn` /
    /// `CardsDiscardedThisTurn` / `EnergySpentThisTurn` history scans
    /// and Condition::FirstPlayOfSourceCardThisTurn / PlaysThisTurnLt.
    CardPlayed {
        round: i32,
        player_idx: usize,
        card_id: String,
        card_type: CardType,
        cost: i32,
        ethereal: bool,
    },
    /// A card was drawn into hand (one event per card). Drives
    /// `AmountSpec::CardsDrawnThisTurn`.
    CardDrawn {
        round: i32,
        player_idx: usize,
        card_id: String,
    },
    /// A card was sent from hand to discard (end-of-turn flush + explicit
    /// DiscardCards). Drives `AmountSpec::CardsDiscardedThisTurn`.
    CardDiscarded {
        round: i32,
        player_idx: usize,
        card_id: String,
    },
    /// A card was sent from any pile to exhaust. Drives
    /// `AmountSpec::CardsExhaustedThisTurn` and Condition::OwnerExhaustedCardThisTurn.
    CardExhausted {
        round: i32,
        player_idx: usize,
        card_id: String,
    },
    /// An orb was channeled into the player's queue. Drives
    /// `AmountSpec::OrbsChanneledThisCombat`.
    OrbChanneled {
        round: i32,
        player_idx: usize,
        orb_id: String,
    },
    /// `pending_stars` mutated (positive or negative delta). Drives
    /// `AmountSpec::StarsGainedThisTurnPositive`.
    StarsChanged {
        round: i32,
        player_idx: usize,
        delta: i32,
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

/// Audit fix #7: helper for the dead-dealer short-circuit in
/// `deal_damage` / `deal_damage_enchanted`. Mirrors C# `Damage(...)`
/// returning a zero-damage `DamageResult` immediately when the
/// attacker is dead at entry. Prevents multi-hit attacks from
/// continuing after the dealer dies mid-loop (e.g., to thorns) and
/// prevents Strength etc. from applying post-mortem.
fn dealer_is_dead(cs: &CombatState, dealer: (CombatSide, usize)) -> bool {
    let creature = match dealer.0 {
        CombatSide::Player => cs.allies.get(dealer.1),
        CombatSide::Enemy => cs.enemies.get(dealer.1),
        CombatSide::None => None,
    };
    creature.map(|c| c.current_hp == 0).unwrap_or(true)
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

fn creature(cs: &CombatState, side: CombatSide, idx: usize) -> Option<&Creature> {
    match side {
        CombatSide::Player => cs.allies.get(idx),
        CombatSide::Enemy => cs.enemies.get(idx),
        CombatSide::None => None,
    }
}

/// Apply `amount` damage to one creature. Block soaks first; the
/// remainder hits HP. If `hp_loss_cap` is Some, the post-block HP
/// loss is also clamped to that many — used by HardenedShellPower
/// for the per-turn HP-loss budget.
fn damage_creature(
    target: &mut Creature,
    amount: i32,
    hp_loss_cap: Option<i32>,
) -> DamageOutcome {
    if amount <= 0 {
        return DamageOutcome::default();
    }
    let pre_hit_hp = target.current_hp;
    let blocked = amount.min(target.block);
    target.block -= blocked;
    let mut hp_lost = amount - blocked;
    if let Some(cap) = hp_loss_cap {
        hp_lost = hp_lost.min(cap.max(0));
    }
    if hp_lost > target.current_hp {
        hp_lost = target.current_hp;
    }
    target.current_hp -= hp_lost;
    // C# `WasTargetKilled = (CurrentHp > 0 && amount >= CurrentHp)` —
    // the transition predicate. True iff THIS hit took HP from positive
    // to zero. Hits on an already-dead creature, or hits that don't
    // reach 0, return false. Feed / HandOfGreed / Reaper-style cards
    // gate on this; using post-state "is dead" instead would let an
    // already-dead corpse re-trigger the kill effect on every hit.
    DamageOutcome {
        blocked,
        hp_lost,
        fatal: pre_hit_hp > 0 && target.current_hp == 0,
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
                pending_gold: 0,
                pending_stars: 0,
                orb_queue: Vec::new(),
                orb_slots: 3,
                pending_forge: 0,
                osty: None,
                relic_counters: std::collections::HashMap::new(),
            }),
            monster: None,
        }
    }

    pub fn from_monster_spawn(monster_id: &str, slot: &str) -> Self {
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
            monster: Some(MonsterState::new()),
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
        // Strip the apply-time skip flag so this test exercises just the
        // tick path (skip semantics are covered separately).
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 2);
        for p in cs.allies[0].powers.iter_mut() {
            p.skip_next_duration_tick = false;
        }
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
        // apply_power), not linger at 0. Strip the apply-time skip flag
        // so this test isolates the at-1 → removal path.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "FrailPower", 1);
        for p in cs.allies[0].powers.iter_mut() {
            p.skip_next_duration_tick = false;
        }
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
        // Strip player-side skip flags (C# only sets them on Player +
        // Debuff). VulnerablePower on the enemy does not get the flag.
        for p in cs.allies[0].powers.iter_mut() {
            p.skip_next_duration_tick = false;
        }
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

    /// C# `PowerCmd.cs:129-132` + `159-162`. Debuff applied to a player
    /// sets `SkipNextDurationTick=true`. The next end-of-enemy-turn
    /// tick CLEARS the flag without decrementing. The tick AFTER that
    /// decrements normally. Enemy-applied debuffs (Vulnerable on a
    /// monster) do NOT get the flag.
    #[test]
    fn skip_next_duration_tick_set_on_player_debuff_apply() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "WeakPower", 2);
        let weak = cs.allies[0]
            .powers
            .iter()
            .find(|p| p.id == "WeakPower")
            .unwrap();
        assert!(weak.skip_next_duration_tick);

        // Enemy-side Vulnerable should NOT carry the flag.
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 1);
        let vuln = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "VulnerablePower")
            .unwrap();
        assert!(!vuln.skip_next_duration_tick);
    }

    #[test]
    fn skip_next_duration_tick_consumes_then_decrements_thereafter() {
        // Apply WeakPower(2) to the player — flag set.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "WeakPower", 2);
        cs.current_side = CombatSide::Enemy;

        // First enemy-turn end: skip is consumed, amount stays at 2.
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "WeakPower"),
            2,
            "first tick consumes the skip flag, leaves amount unchanged"
        );
        let weak = cs.allies[0]
            .powers
            .iter()
            .find(|p| p.id == "WeakPower")
            .unwrap();
        assert!(
            !weak.skip_next_duration_tick,
            "skip flag must be cleared after first tick"
        );

        // Begin player turn, end it, begin enemy, end it: second tick
        // decrements normally.
        cs.begin_turn(CombatSide::Player);
        cs.end_turn();
        cs.begin_turn(CombatSide::Enemy);
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "WeakPower"),
            1,
            "second tick decrements normally"
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

    /// Audit fix #6: enchantment additive participates in modify_block.
    /// Nimble enchantment adds Amount block on powered card/move-block.
    #[test]
    fn modify_block_threads_enchantment_additive() {
        let cs = ironclad_combat();
        let raw = 5;
        let ench = EnchantmentInstance {
            id: "Nimble".to_string(),
            amount: 3,
        };
        let with = cs.modify_block_with_enchantment(
            (CombatSide::Player, 0),
            raw,
            powered_move(),
            Some(&ench),
        );
        let without = cs.modify_block_with_enchantment(
            (CombatSide::Player, 0),
            raw,
            powered_move(),
            None,
        );
        assert_eq!(with, without + 3);
    }

    /// Audit fix #7: deal_damage returns zero outcome immediately when
    /// the dealer is dead at entry, matching C# AttackCommand semantics
    /// (`if attacker.IsDead return zero result`). Prevents post-mortem
    /// damage from Strength etc. applying after the dealer dies mid-
    /// multi-hit (e.g., to thorns).
    #[test]
    fn dead_dealer_short_circuits_deal_damage() {
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = 0;
        let player_hp_before = cs.allies[0].current_hp;
        let outcome = cs.deal_damage(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(outcome.hp_lost, 0);
        assert_eq!(outcome.blocked, 0);
        assert_eq!(cs.allies[0].current_hp, player_hp_before);
    }

    /// Audit fix #5: BeforePowerAmountChanged / AfterPowerAmountChanged
    /// hooks fire around the inner apply. No listeners registered yet,
    /// but the hooks must not crash and must not affect output. Also
    /// confirms `amount == 0` short-circuits the apply entirely
    /// (matches C# PowerCmd.cs:90).
    #[test]
    fn apply_power_zero_amount_is_no_op() {
        let mut cs = ironclad_combat();
        let result = cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 0);
        assert_eq!(result, 0);
        // No power instance should be created.
        assert!(!cs.enemies[0]
            .powers
            .iter()
            .any(|p| p.id == "VulnerablePower"));
    }

    /// `DamageOutcome.fatal` is the TRANSITION predicate: true iff this
    /// hit took the target from HP>0 to HP=0. C# spec
    /// `WasTargetKilled = (CurrentHp > 0 && amount >= CurrentHp)`.
    ///
    /// Audit 2026-05-14: prior implementation used post-state (`current_hp == 0`)
    /// which would re-fire kill triggers (Feed / HandOfGreed / Reaper) every
    /// time a corpse got hit by a multi-hit card.
    #[test]
    fn fatal_is_transition_not_post_state() {
        // Case 1: hit that brings HP from positive to 0 → fatal=true.
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = 5;
        let outcome = cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert!(outcome.fatal, "kill hit must report fatal=true");
        assert_eq!(cs.enemies[0].current_hp, 0);

        // Case 2: subsequent hit on the now-dead corpse → fatal=false
        // (no second kill-trigger).
        let outcome2 = cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert!(
            !outcome2.fatal,
            "post-mortem hit must report fatal=false (no re-trigger)"
        );

        // Case 3: hit that doesn't kill → fatal=false.
        let mut cs2 = ironclad_combat();
        let outcome3 = cs2.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            1,
            ValueProp::MOVE,
        );
        assert!(!outcome3.fatal, "non-killing hit must report fatal=false");
        assert!(cs2.enemies[0].current_hp > 0);
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

    /// Status / curse cards with OnTurnEndInHand fire their per-turn
    /// payload when the player turn ends with them in hand.
    /// Burn deals 2 damage (Unpowered) to owner.
    #[test]
    fn burn_in_hand_damages_owner_at_turn_end() {
        let mut cs = ironclad_combat();
        let burn = card_by_id("Burn").expect("Burn exists");
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(burn, 0));
        let hp_before = cs.allies[0].current_hp;
        cs.current_side = CombatSide::Player;
        cs.end_turn();
        assert_eq!(cs.allies[0].current_hp, hp_before - 2);
        // Burn is Unplayable but not Ethereal -- routes to discard at
        // turn end like a normal card. Stays in the deck for the
        // duration of combat, dealing 2 every turn.
        assert!(cs.allies[0]
            .player
            .as_ref()
            .unwrap()
            .discard
            .cards
            .iter()
            .any(|c| c.id == "Burn"));
    }

    /// Doubt applies Weak(1) to the player at turn end. The new debuff's
    /// SkipNextDurationTick prevents it from ticking down on the same
    /// boundary (audit fix #3 applies to player-side Debuffs).
    #[test]
    fn doubt_in_hand_applies_weak_at_turn_end() {
        let mut cs = ironclad_combat();
        let doubt = card_by_id("Doubt").expect("Doubt exists");
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(doubt, 0));
        cs.current_side = CombatSide::Player;
        cs.end_turn();
        let weak = cs.get_power_amount(CombatSide::Player, 0, "WeakPower");
        assert!(weak >= 1);
    }

    /// Unplayable cards (status, curses) cannot be played manually.
    /// Mirrors C# `CardModel.CanPlay -> UnplayableReason.HasUnplayableKeyword`.
    #[test]
    fn unplayable_keyword_rejects_manual_play() {
        let mut cs = ironclad_combat();
        // Wound is a Status with Unplayable.
        let wound = card_by_id("Wound").expect("Wound exists in card data");
        assert!(wound.keywords.iter().any(|k| k == "Unplayable"));
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(wound, 0));
        let energy_before = cs.allies[0].player.as_ref().unwrap().energy;
        let result = cs.play_card(0, 0, None);
        assert_eq!(result, PlayResult::Unplayable);
        // Energy NOT spent, card NOT routed.
        assert_eq!(cs.allies[0].player.as_ref().unwrap().energy, energy_before);
        assert_eq!(cs.allies[0].player.as_ref().unwrap().hand.cards.len(), 1);
    }

    /// Ethereal cards still in hand at end of player turn auto-exhaust
    /// instead of discarding. Mirrors C# `Hook.BeforeFlush` keyword
    /// branch for Ethereal.
    #[test]
    fn ethereal_card_auto_exhausts_at_end_of_player_turn() {
        let mut cs = ironclad_combat();
        // Dazed = status with Ethereal + Unplayable.
        let dazed = card_by_id("Dazed").expect("Dazed exists");
        assert!(dazed.keywords.iter().any(|k| k == "Ethereal"));
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(dazed, 0));
        cs.current_side = CombatSide::Player;
        cs.end_turn();
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert!(ps.hand.cards.iter().all(|c| c.id != "Dazed"));
        assert!(ps.exhaust.cards.iter().any(|c| c.id == "Dazed"));
        assert!(ps.discard.cards.iter().all(|c| c.id != "Dazed"));
    }

    /// Retain cards stay in hand across the player-turn boundary.
    #[test]
    fn retain_card_stays_in_hand_across_turn() {
        let mut cs = ironclad_combat();
        let snake = card_by_id("Snakebite").expect("Snakebite exists");
        // Confirm Snakebite has Retain in its keywords.
        assert!(snake.keywords.iter().any(|k| k == "Retain"));
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(snake, 0));
        cs.current_side = CombatSide::Player;
        cs.end_turn();
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert!(
            ps.hand.cards.iter().any(|c| c.id == "Snakebite"),
            "Retain card must stay in hand"
        );
    }

    /// move_innate_cards_to_hand pulls Innate cards from draw to hand.
    /// Mirrors C# PlayerCombatState start-of-combat innate-priority shuffle.
    #[test]
    fn innate_cards_move_to_hand_before_normal_draw() {
        let mut cs = ironclad_combat();
        // Clear hand to start; ensure draw is fresh
        let ps = cs.allies[0].player.as_mut().unwrap();
        ps.hand.cards.clear();
        // Seed an Innate card into the draw pile.
        let mayhem = card_by_id("Mayhem").expect("Mayhem exists");
        // Mayhem is a Power that has Innate? Actually check the keyword.
        // Inflame doesn't have Innate; let's just look up a known Innate card
        // from card data.
        let _ = mayhem;
        // Pick any card whose data has Innate.
        // Iterate the static table.
        let innate_card = crate::card::ALL_CARDS
            .iter()
            .find(|c| c.keywords.iter().any(|k| k == "Innate"))
            .expect("at least one Innate card exists in the data");
        ps.draw.cards.push(CardInstance::from_card(innate_card, 0));
        // Re-borrow.
        let moved = cs.move_innate_cards_to_hand(0);
        assert!(moved >= 1);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert!(ps.hand.cards.iter().any(|c| c.id == innate_card.id));
        assert!(ps.draw.cards.iter().all(|c| c.id != innate_card.id));
    }

    #[test]
    fn play_card_unhandled_still_spends_energy_and_routes_to_discard() {
        let mut cs = ironclad_combat();
        // Headbutt is one of the cards still SKIPped in the data table —
        // it needs interactive pick-from-discard-then-move-to-draw-top
        // (CardSelectCmd) which isn't expressible in the current
        // Selector vocabulary. Confirm the "Unhandled but state-changes-
        // still-happen" path: energy spent, card routed to discard.
        let headbutt = card_by_id("Headbutt").unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(headbutt, 0));
        let result = cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
        assert_eq!(result, PlayResult::Unhandled);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.energy, 2); // Headbutt costs 1.
        assert!(ps.hand.is_empty());
        assert_eq!(ps.discard.cards.iter().any(|c| c.id == "Headbutt"), true);
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

    // ---------- Defile + Defy + GraveWarden tests ------------------------

    #[test]
    fn defile_deals_thirteen() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "Defile",
            0,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 13);
    }

    #[test]
    fn upgraded_defile_deals_seventeen() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "Defile",
            1,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 17);
    }

    #[test]
    fn defy_grants_six_block_and_one_weak() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "Defy", 0, Some((CombatSide::Enemy, 0)));
        assert_eq!(cs.allies[0].block, 6);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "WeakPower"),
            1
        );
    }

    #[test]
    fn upgraded_defy_grants_nine_block_still_one_weak() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "Defy", 1, Some((CombatSide::Enemy, 0)));
        assert_eq!(cs.allies[0].block, 9);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "WeakPower"),
            1
        );
    }

    #[test]
    fn grave_warden_grants_eight_block_and_adds_soul_to_draw() {
        let mut cs = ironclad_combat();
        let draw_before = cs.allies[0].player.as_ref().unwrap().draw.len();
        inject_card_and_play(&mut cs, "GraveWarden", 0, None);
        assert_eq!(cs.allies[0].block, 8);
        let ps = cs.allies[0].player.as_ref().unwrap();
        // Soul appended to draw pile.
        assert_eq!(ps.draw.len(), draw_before + 1);
        assert!(ps.draw.cards.iter().any(|c| c.id == "Soul"));
    }

    // ---------- BlightStrike + Doom + simple-block ports -----------------

    #[test]
    fn blight_strike_deals_eight_and_applies_doom() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "BlightStrike",
            0,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 8);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "DoomPower"),
            8
        );
    }

    #[test]
    fn doom_kills_when_hp_below_amount_at_turn_end() {
        // Set enemy HP to 5; apply Doom 8 directly. End enemy turn →
        // tick_doom_powers fires, enemy at 0 HP.
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = 5;
        cs.apply_power(CombatSide::Enemy, 0, "DoomPower", 8);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(cs.enemies[0].current_hp, 0);
    }

    #[test]
    fn doom_does_not_kill_when_hp_above_amount() {
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = 20;
        cs.apply_power(CombatSide::Enemy, 0, "DoomPower", 8);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(cs.enemies[0].current_hp, 20);
    }

    #[test]
    fn doom_fires_only_on_owner_side_turn_end() {
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = 5;
        cs.apply_power(CombatSide::Enemy, 0, "DoomPower", 8);
        // Ending player turn — enemy's Doom shouldn't fire yet.
        cs.current_side = CombatSide::Player;
        cs.end_turn();
        assert_eq!(cs.enemies[0].current_hp, 5);
    }

    #[test]
    fn cosmic_indifference_grants_six_block() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "CosmicIndifference", 0, None);
        assert_eq!(cs.allies[0].block, 6);
    }

    #[test]
    fn cloak_of_stars_grants_seven_block() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "CloakOfStars", 0, None);
        assert_eq!(cs.allies[0].block, 7);
    }

    // ---------- Defect/Regent cross-pool commons tests -------------------

    #[test]
    fn beam_cell_deals_three_and_one_vulnerable() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "BeamCell",
            0,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 3);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            1
        );
    }

    #[test]
    fn upgraded_beam_cell_deals_four_two_vulnerable() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "BeamCell",
            1,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 4);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VulnerablePower"),
            2
        );
    }

    #[test]
    fn boost_away_grants_six_block_and_dazes() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "BoostAway", 0, None);
        assert_eq!(cs.allies[0].block, 6);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert!(ps.discard.cards.iter().any(|c| c.id == "Dazed"));
    }

    #[test]
    fn upgraded_boost_away_grants_nine_block() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "BoostAway", 1, None);
        assert_eq!(cs.allies[0].block, 9);
    }

    #[test]
    fn astral_pulse_hits_each_enemy_for_fourteen() {
        let mut cs = ironclad_combat();
        let h0 = cs.enemies[0].current_hp;
        let h1 = cs.enemies[1].current_hp;
        inject_card_and_play(&mut cs, "AstralPulse", 0, None);
        assert_eq!(cs.enemies[0].current_hp, h0 - 14);
        assert_eq!(cs.enemies[1].current_hp, h1 - 14);
    }

    #[test]
    fn upgraded_astral_pulse_does_eighteen() {
        let mut cs = ironclad_combat();
        let h0 = cs.enemies[0].current_hp;
        inject_card_and_play(&mut cs, "AstralPulse", 1, None);
        assert_eq!(cs.enemies[0].current_hp, h0 - 18);
    }

    #[test]
    fn collision_course_deals_eleven_and_adds_debris() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "CollisionCourse",
            0,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 11);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert!(ps.hand.cards.iter().any(|c| c.id == "Debris"));
    }

    #[test]
    fn upgraded_collision_course_deals_fifteen() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "CollisionCourse",
            1,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 15);
    }

    // ---------- BladeDance + Snakebite tests -----------------------------

    #[test]
    fn blade_dance_adds_three_shivs_and_exhausts() {
        let mut cs = ironclad_combat();
        let hand_before = cs.allies[0].player.as_ref().unwrap().hand.len();
        inject_card_and_play(&mut cs, "BladeDance", 0, None);
        let ps = cs.allies[0].player.as_ref().unwrap();
        // +BladeDance (inject) → -BladeDance (play, routes to exhaust)
        // → +3 Shivs. Net delta: +3.
        assert_eq!(ps.hand.len(), hand_before + 3);
        let shivs = ps.hand.cards.iter().filter(|c| c.id == "Shiv").count();
        assert_eq!(shivs, 3);
        assert!(ps.exhaust.cards.iter().any(|c| c.id == "BladeDance"));
    }

    #[test]
    fn upgraded_blade_dance_adds_four_shivs() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "BladeDance", 1, None);
        let ps = cs.allies[0].player.as_ref().unwrap();
        let shivs = ps.hand.cards.iter().filter(|c| c.id == "Shiv").count();
        assert_eq!(shivs, 4);
    }

    #[test]
    fn snakebite_applies_seven_poison() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        inject_card_and_play(
            &mut cs,
            "Snakebite",
            0,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PoisonPower"),
            7
        );
    }

    #[test]
    fn upgraded_snakebite_applies_ten_poison() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        inject_card_and_play(
            &mut cs,
            "Snakebite",
            1,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PoisonPower"),
            10
        );
    }

    // ---------- Anticipate/Untouchable/FlickFlack/Ricochet tests ---------

    #[test]
    fn anticipate_grants_two_temp_dexterity() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "Anticipate", 0, None);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "AnticipatePower"),
            2
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "DexterityPower"),
            2
        );
    }

    #[test]
    fn anticipate_dex_clears_at_end_of_player_turn() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "Anticipate", 0, None);
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "AnticipatePower"),
            0
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "DexterityPower"),
            0
        );
    }

    #[test]
    fn anticipate_preserves_permanent_dex() {
        // Existing permanent Dex(3) + Anticipate(2 temp). After EOT,
        // perma Dex stays at 3.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "DexterityPower", 3);
        inject_card_and_play(&mut cs, "Anticipate", 0, None);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "DexterityPower"),
            5
        );
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "DexterityPower"),
            3
        );
    }

    #[test]
    fn untouchable_grants_six_block() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        inject_card_and_play(&mut cs, "Untouchable", 0, None);
        assert_eq!(cs.allies[0].block, 6);
    }

    #[test]
    fn upgraded_untouchable_grants_eight_block() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        inject_card_and_play(&mut cs, "Untouchable", 1, None);
        assert_eq!(cs.allies[0].block, 8);
    }

    #[test]
    fn flick_flack_hits_each_enemy_once() {
        let mut cs = ironclad_combat();
        let h0 = cs.enemies[0].current_hp;
        let h1 = cs.enemies[1].current_hp;
        inject_card_and_play(&mut cs, "FlickFlack", 0, None);
        assert_eq!(cs.enemies[0].current_hp, h0 - 6);
        assert_eq!(cs.enemies[1].current_hp, h1 - 6);
    }

    #[test]
    fn upgraded_flick_flack_does_eight_damage() {
        let mut cs = ironclad_combat();
        let h0 = cs.enemies[0].current_hp;
        inject_card_and_play(&mut cs, "FlickFlack", 1, None);
        assert_eq!(cs.enemies[0].current_hp, h0 - 8);
    }

    #[test]
    fn ricochet_does_four_hits_three_damage() {
        let mut cs = ironclad_combat();
        cs.rng = Rng::new(42, 0);
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let total_before: i32 = cs.enemies.iter().map(|e| e.current_hp).sum();
        inject_card_and_play(&mut cs, "Ricochet", 0, None);
        let total_after: i32 = cs.enemies.iter().map(|e| e.current_hp).sum();
        // 4 hits × 3 damage = 12.
        assert_eq!(total_before - total_after, 12);
    }

    #[test]
    fn upgraded_ricochet_does_five_hits() {
        let mut cs = ironclad_combat();
        cs.rng = Rng::new(42, 0);
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let total_before: i32 = cs.enemies.iter().map(|e| e.current_hp).sum();
        inject_card_and_play(&mut cs, "Ricochet", 1, None);
        let total_after: i32 = cs.enemies.iter().map(|e| e.current_hp).sum();
        // 5 hits × 3 damage = 15.
        assert_eq!(total_before - total_after, 15);
    }

    // ---------- Shiv-creating cards tests --------------------------------

    fn populate_draw_pile_strikes(cs: &mut CombatState, n: usize) {
        let strike = card_by_id("StrikeIronclad").unwrap();
        let ps = cs.allies[0].player.as_mut().unwrap();
        for _ in 0..n {
            ps.draw.cards.push(CardInstance::from_card(strike, 0));
        }
    }

    #[test]
    fn shiv_deals_four() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(&mut cs, "Shiv", 0, Some((CombatSide::Enemy, 0)));
        assert_eq!(cs.enemies[0].current_hp, hp - 4);
    }

    #[test]
    fn shiv_routes_to_exhaust_via_keyword() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "Shiv", 0, Some((CombatSide::Enemy, 0)));
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert!(ps.exhaust.cards.iter().any(|c| c.id == "Shiv"));
    }

    #[test]
    fn backflip_grants_five_block_and_draws_two() {
        let mut cs = ironclad_combat();
        populate_draw_pile_strikes(&mut cs, 5);
        let hand_before = cs.allies[0].player.as_ref().unwrap().hand.len();
        inject_card_and_play(&mut cs, "Backflip", 0, None);
        assert_eq!(cs.allies[0].block, 5);
        // hand_before + Backflip (+1) → play removes Backflip (-1) →
        // draw 2 (+2). Net delta: +2 vs starting hand_before.
        assert_eq!(
            cs.allies[0].player.as_ref().unwrap().hand.len(),
            hand_before + 2
        );
    }

    #[test]
    fn upgraded_backflip_grants_eight_block_two_cards() {
        let mut cs = ironclad_combat();
        populate_draw_pile_strikes(&mut cs, 5);
        inject_card_and_play(&mut cs, "Backflip", 1, None);
        assert_eq!(cs.allies[0].block, 8);
    }

    #[test]
    fn cloak_and_dagger_grants_six_block_and_adds_shiv() {
        let mut cs = ironclad_combat();
        let hand_before = cs.allies[0].player.as_ref().unwrap().hand.len();
        inject_card_and_play(&mut cs, "CloakAndDagger", 0, None);
        assert_eq!(cs.allies[0].block, 6);
        // hand_before + Cloak (+1) → play removes Cloak (-1) → add
        // 1 Shiv (+1). Net delta: +1 vs starting hand_before.
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.hand.len(), hand_before + 1);
        assert!(ps.hand.cards.iter().any(|c| c.id == "Shiv"));
    }

    #[test]
    fn upgraded_cloak_and_dagger_adds_two_shivs() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "CloakAndDagger", 1, None);
        assert_eq!(cs.allies[0].block, 6);
        let ps = cs.allies[0].player.as_ref().unwrap();
        let shivs = ps.hand.cards.iter().filter(|c| c.id == "Shiv").count();
        assert_eq!(shivs, 2);
    }

    #[test]
    fn leading_strike_deals_three_and_adds_two_shivs() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "LeadingStrike",
            0,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 3);
        let ps = cs.allies[0].player.as_ref().unwrap();
        let shivs = ps.hand.cards.iter().filter(|c| c.id == "Shiv").count();
        assert_eq!(shivs, 2);
    }

    #[test]
    fn upgraded_leading_strike_deals_six_still_two_shivs() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "LeadingStrike",
            1,
            Some((CombatSide::Enemy, 0)),
        );
        // Damage upgrades, Shiv count doesn't.
        assert_eq!(cs.enemies[0].current_hp, hp - 6);
        let ps = cs.allies[0].player.as_ref().unwrap();
        let shivs = ps.hand.cards.iter().filter(|c| c.id == "Shiv").count();
        assert_eq!(shivs, 2);
    }

    // ---------- Silent commons batch tests -------------------------------

    fn inject_card_and_play(
        cs: &mut CombatState,
        card_id: &str,
        upgrade: i32,
        target: Option<(CombatSide, usize)>,
    ) -> PlayResult {
        let card = card_by_id(card_id).unwrap();
        cs.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(card, upgrade));
        let hand_idx = cs.allies[0].player.as_ref().unwrap().hand.len() - 1;
        cs.play_card(0, hand_idx, target)
    }

    #[test]
    fn dagger_throw_deals_nine() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        let r = inject_card_and_play(
            &mut cs,
            "DaggerThrow",
            0,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(r, PlayResult::Ok);
        assert_eq!(cs.enemies[0].current_hp, hp - 9);
    }

    #[test]
    fn upgraded_dagger_throw_deals_twelve() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "DaggerThrow",
            1,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 12);
    }

    #[test]
    fn slice_deals_six_zero_cost() {
        let mut cs = ironclad_combat();
        let energy_before = cs.allies[0].player.as_ref().unwrap().energy;
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(&mut cs, "Slice", 0, Some((CombatSide::Enemy, 0)));
        assert_eq!(cs.enemies[0].current_hp, hp - 6);
        // 0-cost: energy unchanged.
        assert_eq!(
            cs.allies[0].player.as_ref().unwrap().energy,
            energy_before
        );
    }

    #[test]
    fn deflect_grants_four_block() {
        let mut cs = ironclad_combat();
        inject_card_and_play(&mut cs, "Deflect", 0, None);
        assert_eq!(cs.allies[0].block, 4);
    }

    #[test]
    fn dagger_spray_hits_each_enemy_twice() {
        let mut cs = ironclad_combat();
        let h0 = cs.enemies[0].current_hp;
        let h1 = cs.enemies[1].current_hp;
        inject_card_and_play(&mut cs, "DaggerSpray", 0, None);
        // 2 hits × 4 damage = 8 per enemy.
        assert_eq!(cs.enemies[0].current_hp, h0 - 8);
        assert_eq!(cs.enemies[1].current_hp, h1 - 8);
    }

    #[test]
    fn upgraded_dagger_spray_hits_for_six_per_hit() {
        let mut cs = ironclad_combat();
        let h0 = cs.enemies[0].current_hp;
        inject_card_and_play(&mut cs, "DaggerSpray", 1, None);
        // 2 × 6 = 12 per enemy.
        assert_eq!(cs.enemies[0].current_hp, h0 - 12);
    }

    #[test]
    fn sucker_punch_damage_and_weak() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "SuckerPunch",
            0,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 8);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "WeakPower"),
            1
        );
    }

    #[test]
    fn poisoned_stab_damage_and_poison() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "PoisonedStab",
            0,
            Some((CombatSide::Enemy, 0)),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 6);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PoisonPower"),
            3
        );
    }

    #[test]
    fn deadly_poison_applies_five_poison() {
        let mut cs = ironclad_combat();
        let hp = cs.enemies[0].current_hp;
        inject_card_and_play(
            &mut cs,
            "DeadlyPoison",
            0,
            Some((CombatSide::Enemy, 0)),
        );
        // Skill — no damage.
        assert_eq!(cs.enemies[0].current_hp, hp);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PoisonPower"),
            5
        );
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

    // ---------- Relic VM (data-driven hook dispatch) tests ----------------

    #[test]
    fn akabeko_grants_vigor_on_first_player_turn() {
        let mut cs = ironclad_combat_with_relic("Akabeko");
        cs.begin_turn(CombatSide::Player);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "VigorPower"),
            8
        );
    }

    #[test]
    fn akabeko_does_not_fire_on_enemy_side() {
        let mut cs = ironclad_combat_with_relic("Akabeko");
        cs.begin_turn(CombatSide::Enemy);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "VigorPower"),
            0
        );
    }

    #[test]
    fn akabeko_does_not_fire_on_round_two() {
        let mut cs = ironclad_combat_with_relic("Akabeko");
        cs.begin_turn(CombatSide::Player);
        cs.begin_turn(CombatSide::Enemy);
        // Strip the round-1 grant so we can distinguish round-2 firing.
        cs.allies[0].powers.retain(|p| p.id != "VigorPower");
        cs.begin_turn(CombatSide::Player);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "VigorPower"),
            0
        );
    }

    #[test]
    fn bag_of_marbles_applies_vulnerable_to_all_enemies() {
        let mut cs = ironclad_combat_with_relic("BagOfMarbles");
        cs.begin_turn(CombatSide::Player);
        for i in 0..cs.enemies.len() {
            assert_eq!(
                cs.get_power_amount(CombatSide::Enemy, i, "VulnerablePower"),
                1,
                "enemy {i} should have 1 Vulnerable"
            );
        }
    }

    #[test]
    fn lantern_grants_one_extra_energy_round_one() {
        let mut cs = ironclad_combat_with_relic("Lantern");
        let starting = cs.allies[0].player.as_ref().unwrap().energy;
        cs.begin_turn(CombatSide::Player);
        let after = cs.allies[0].player.as_ref().unwrap().energy;
        // Energy refresh happens before AfterSideTurnStart in begin_turn,
        // so Lantern adds on top of the refilled turn_energy.
        let turn_energy = cs.allies[0].player.as_ref().unwrap().turn_energy;
        assert_eq!(after, turn_energy + 1, "starting was {starting}");
    }

    // ---------- Potion VM (data-driven OnUse dispatch) tests --------------

    #[test]
    fn block_potion_gains_twelve_block() {
        let mut cs = ironclad_combat();
        let ok = cs.use_potion(0, "BlockPotion", None);
        assert!(ok);
        assert_eq!(cs.allies[0].block, 12);
    }

    #[test]
    fn energy_potion_grants_two_energy() {
        let mut cs = ironclad_combat();
        let start = cs.allies[0].player.as_ref().unwrap().energy;
        let ok = cs.use_potion(0, "EnergyPotion", None);
        assert!(ok);
        let after = cs.allies[0].player.as_ref().unwrap().energy;
        assert_eq!(after - start, 2);
    }

    #[test]
    fn fire_potion_damages_chosen_enemy() {
        let mut cs = ironclad_combat();
        let target_hp_before = cs.enemies[0].current_hp;
        let ok = cs.use_potion(0, "FirePotion", Some((CombatSide::Enemy, 0)));
        assert!(ok);
        let target_hp_after = cs.enemies[0].current_hp;
        // FirePotion: 20 damage (Unpowered). Other enemies untouched.
        assert!(target_hp_before - target_hp_after >= 1);
    }

    #[test]
    fn dexterity_potion_applies_dex_to_self() {
        let mut cs = ironclad_combat();
        let ok = cs.use_potion(0, "DexterityPotion", None);
        assert!(ok);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "DexterityPower"),
            2
        );
    }

    #[test]
    fn use_potion_unknown_returns_false() {
        let mut cs = ironclad_combat();
        let ok = cs.use_potion(0, "NotARealPotion", None);
        assert!(!ok);
    }

    // ---------- end Potion VM tests ---------------------------------------

    #[test]
    fn black_blood_heals_on_victory() {
        let mut cs = ironclad_combat();
        // Swap BurningBlood (default starter) for BlackBlood.
        cs.allies[0].player.as_mut().unwrap().relics =
            vec!["BlackBlood".to_string()];
        // Damage the player so we can verify the heal.
        cs.allies[0].current_hp = cs.allies[0].max_hp - 20;
        let before = cs.allies[0].current_hp;
        cs.fire_after_combat_victory_hooks();
        let after = cs.allies[0].current_hp;
        // BlackBlood's HealVar(12) — same shape as BurningBlood's 6.
        assert_eq!(after - before, 12);
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
                CombatEvent::CardPlayed { .. } => "CardPlayed",
                CombatEvent::CardDrawn { .. } => "CardDrawn",
                CombatEvent::CardDiscarded { .. } => "CardDiscarded",
                CombatEvent::CardExhausted { .. } => "CardExhausted",
                CombatEvent::OrbChanneled { .. } => "OrbChanneled",
                CombatEvent::StarsChanged { .. } => "StarsChanged",
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

    // ---------- OwlMagistrate + SoarPower tests ---------------------------

    #[test]
    fn owl_magistrate_chain_scrutiny_peck_flight_verdict() {
        assert_eq!(
            pick_owl_magistrate_intent(None),
            OwlMagistrateIntent::Scrutiny
        );
        assert_eq!(
            pick_owl_magistrate_intent(Some(OwlMagistrateIntent::Scrutiny)),
            OwlMagistrateIntent::PeckAssault
        );
        assert_eq!(
            pick_owl_magistrate_intent(Some(OwlMagistrateIntent::PeckAssault)),
            OwlMagistrateIntent::JudicialFlight
        );
        assert_eq!(
            pick_owl_magistrate_intent(Some(
                OwlMagistrateIntent::JudicialFlight
            )),
            OwlMagistrateIntent::Verdict
        );
        assert_eq!(
            pick_owl_magistrate_intent(Some(OwlMagistrateIntent::Verdict)),
            OwlMagistrateIntent::Scrutiny
        );
    }

    #[test]
    fn owl_magistrate_judicial_flight_applies_soar() {
        let mut cs = ironclad_combat();
        execute_owl_magistrate_move(
            &mut cs,
            0,
            0,
            OwlMagistrateIntent::JudicialFlight,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "SoarPower"),
            1
        );
    }

    #[test]
    fn owl_magistrate_verdict_payload_and_removes_soar() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "SoarPower", 1);
        let hp = cs.allies[0].current_hp;
        execute_owl_magistrate_move(
            &mut cs,
            0,
            0,
            OwlMagistrateIntent::Verdict,
        );
        // 33 damage halved by SoarPower? NO — SoarPower is on the
        // attacker (target=Owner), reducing incoming damage to Owner.
        // Verdict attacks the PLAYER, who doesn't have Soar. Full 33.
        assert_eq!(cs.allies[0].current_hp, hp - 33);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "VulnerablePower"),
            4
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "SoarPower"),
            0
        );
    }

    #[test]
    fn soar_halves_incoming_powered_damage() {
        // Apply Soar to player; enemy deals 10 damage to player → 5.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "SoarPower", 1);
        let hp = cs.allies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 5);
    }

    #[test]
    fn soar_does_not_affect_unpowered_damage() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "SoarPower", 1);
        let hp = cs.allies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            10,
            ValueProp::UNPOWERED.with(ValueProp::MOVE),
        );
        assert_eq!(cs.allies[0].current_hp, hp - 10);
    }

    // ---------- CalcifiedCultist + RitualPower tests ----------------------

    #[test]
    fn calcified_cultist_chain_incantation_then_dark_strikes_forever() {
        assert_eq!(
            pick_calcified_cultist_intent(None),
            CalcifiedCultistIntent::Incantation
        );
        assert_eq!(
            pick_calcified_cultist_intent(Some(
                CalcifiedCultistIntent::Incantation
            )),
            CalcifiedCultistIntent::DarkStrike
        );
        // DarkStrike self-loops.
        for _ in 0..5 {
            assert_eq!(
                pick_calcified_cultist_intent(Some(
                    CalcifiedCultistIntent::DarkStrike
                )),
                CalcifiedCultistIntent::DarkStrike
            );
        }
    }

    #[test]
    fn calcified_cultist_incantation_applies_ritual_two() {
        let mut cs = ironclad_combat();
        execute_calcified_cultist_move(
            &mut cs,
            0,
            0,
            CalcifiedCultistIntent::Incantation,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "RitualPower"),
            2
        );
    }

    #[test]
    fn calcified_cultist_dark_strike_deals_nine() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_calcified_cultist_move(
            &mut cs,
            0,
            0,
            CalcifiedCultistIntent::DarkStrike,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 9);
    }

    #[test]
    fn ritual_grants_strength_on_owner_side_turn_end() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "RitualPower", 2);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    // ---------- ThievingHopper + FlutterPower tests -----------------------

    #[test]
    fn thieving_hopper_chain_thievery_flutter_hattrick_nab_escape() {
        assert_eq!(
            pick_thieving_hopper_intent(None),
            ThievingHopperIntent::Thievery
        );
        assert_eq!(
            pick_thieving_hopper_intent(Some(ThievingHopperIntent::Thievery)),
            ThievingHopperIntent::Flutter
        );
        assert_eq!(
            pick_thieving_hopper_intent(Some(ThievingHopperIntent::Flutter)),
            ThievingHopperIntent::HatTrick
        );
        assert_eq!(
            pick_thieving_hopper_intent(Some(ThievingHopperIntent::HatTrick)),
            ThievingHopperIntent::Nab
        );
        assert_eq!(
            pick_thieving_hopper_intent(Some(ThievingHopperIntent::Nab)),
            ThievingHopperIntent::Escape
        );
        // Escape self-loops.
        for _ in 0..5 {
            assert_eq!(
                pick_thieving_hopper_intent(Some(ThievingHopperIntent::Escape)),
                ThievingHopperIntent::Escape
            );
        }
    }

    #[test]
    fn thieving_hopper_spawn_applies_escape_artist_five() {
        let mut cs = ironclad_combat();
        thieving_hopper_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "EscapeArtistPower"),
            5
        );
    }

    #[test]
    fn thieving_hopper_thievery_deals_seventeen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_thieving_hopper_move(
            &mut cs,
            0,
            0,
            ThievingHopperIntent::Thievery,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 17);
    }

    #[test]
    fn thieving_hopper_flutter_applies_flutter_five() {
        let mut cs = ironclad_combat();
        execute_thieving_hopper_move(
            &mut cs,
            0,
            0,
            ThievingHopperIntent::Flutter,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "FlutterPower"),
            5
        );
    }

    #[test]
    fn thieving_hopper_hat_trick_deals_twentyone() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_thieving_hopper_move(
            &mut cs,
            0,
            0,
            ThievingHopperIntent::HatTrick,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 21);
    }

    #[test]
    fn thieving_hopper_nab_deals_fourteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_thieving_hopper_move(
            &mut cs,
            0,
            0,
            ThievingHopperIntent::Nab,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 14);
    }

    #[test]
    fn thieving_hopper_escape_is_noop() {
        // Escape is a placeholder until the Escape mechanic ports.
        // Until then it must not damage the player or modify state.
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_thieving_hopper_move(
            &mut cs,
            0,
            0,
            ThievingHopperIntent::Escape,
        );
        assert_eq!(cs.allies[0].current_hp, hp);
    }

    #[test]
    fn flutter_halves_incoming_powered_damage_on_owner() {
        // Flutter on enemy halves damage targeting enemy. Apply via
        // spawn pattern: 5 stacks.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "FlutterPower", 5);
        let hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 5);
    }

    #[test]
    fn flutter_does_not_affect_unpowered_damage() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "FlutterPower", 5);
        let hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::UNPOWERED.with(ValueProp::MOVE),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 10);
    }

    #[test]
    fn escape_artist_decrements_on_owner_turn_end_holds_at_one() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "EscapeArtistPower", 3);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "EscapeArtistPower"),
            2
        );
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "EscapeArtistPower"),
            1
        );
        // Now holds at 1 — won't decrement further.
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "EscapeArtistPower"),
            1
        );
    }

    #[test]
    fn escape_artist_does_not_tick_on_other_side_turn_end() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "EscapeArtistPower", 3);
        cs.current_side = CombatSide::Player;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "EscapeArtistPower"),
            3
        );
    }

    // ---------- Toadpole + ThornsPower tests ------------------------------

    #[test]
    fn toadpole_front_starts_spiken_back_starts_whirl() {
        assert_eq!(
            pick_toadpole_intent(None, true),
            ToadpoleIntent::Spiken
        );
        assert_eq!(
            pick_toadpole_intent(None, false),
            ToadpoleIntent::Whirl
        );
    }

    #[test]
    fn toadpole_walks_triangle() {
        assert_eq!(
            pick_toadpole_intent(Some(ToadpoleIntent::SpikeSpit), true),
            ToadpoleIntent::Whirl
        );
        assert_eq!(
            pick_toadpole_intent(Some(ToadpoleIntent::Whirl), true),
            ToadpoleIntent::Spiken
        );
        assert_eq!(
            pick_toadpole_intent(Some(ToadpoleIntent::Spiken), true),
            ToadpoleIntent::SpikeSpit
        );
        // is_front flag doesn't matter once last_intent is set.
        assert_eq!(
            pick_toadpole_intent(Some(ToadpoleIntent::Spiken), false),
            ToadpoleIntent::SpikeSpit
        );
    }

    #[test]
    fn toadpole_spiken_applies_thorns_two() {
        let mut cs = ironclad_combat();
        execute_toadpole_move(&mut cs, 0, 0, ToadpoleIntent::Spiken);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ThornsPower"),
            2
        );
    }

    #[test]
    fn toadpole_spike_spit_strips_two_thorns_and_hits_thrice() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "ThornsPower", 4);
        let hp = cs.allies[0].current_hp;
        execute_toadpole_move(&mut cs, 0, 0, ToadpoleIntent::SpikeSpit);
        // 3 hits of 3 damage each (no Strength, no Vuln). Pre-SpikeSpit
        // strips 2 Thorns from self — the player then takes 3 hits, each
        // bouncing remaining 2 Thorns back on the toadpole.
        assert_eq!(cs.allies[0].current_hp, hp - 9);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ThornsPower"),
            2
        );
    }

    #[test]
    fn toadpole_whirl_deals_seven() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_toadpole_move(&mut cs, 0, 0, ToadpoleIntent::Whirl);
        assert_eq!(cs.allies[0].current_hp, hp - 7);
    }

    #[test]
    fn thorns_reflects_damage_on_powered_hit() {
        // Player attacks enemy with ThornsPower. Player should take
        // back the thorns amount (unpowered).
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "ThornsPower", 4);
        let player_hp = cs.allies[0].current_hp;
        let enemy_hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(cs.allies[0].current_hp, player_hp - 4);
        assert_eq!(cs.enemies[0].current_hp, enemy_hp - 10);
    }

    #[test]
    fn thorns_ignores_unpowered_attack() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "ThornsPower", 4);
        let player_hp = cs.allies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::UNPOWERED.with(ValueProp::MOVE),
        );
        assert_eq!(cs.allies[0].current_hp, player_hp);
    }

    // ---------- TheObscura tests -------------------------------------------

    #[test]
    fn obscura_init_is_illusion() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_the_obscura_intent(&mut rng, None),
            TheObscuraIntent::Illusion
        );
    }

    #[test]
    fn obscura_post_illusion_no_repeat() {
        let mut rng = Rng::new(42, 0);
        for _ in 0..30 {
            for &start in &[
                TheObscuraIntent::Illusion,
                TheObscuraIntent::PiercingGaze,
                TheObscuraIntent::Wail,
                TheObscuraIntent::HardeningStrike,
            ] {
                let next = pick_the_obscura_intent(&mut rng, Some(start));
                assert_ne!(next, start);
                assert_ne!(next, TheObscuraIntent::Illusion);
            }
        }
    }

    #[test]
    fn obscura_piercing_gaze_deals_ten() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_the_obscura_move(
            &mut cs,
            0,
            0,
            TheObscuraIntent::PiercingGaze,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 10);
    }

    #[test]
    fn obscura_hardening_strike_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_the_obscura_move(
            &mut cs,
            0,
            0,
            TheObscuraIntent::HardeningStrike,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 6);
        assert_eq!(cs.enemies[0].block, 6);
    }

    // ---------- LivingFog tests --------------------------------------------

    #[test]
    fn living_fog_walks_chain() {
        assert_eq!(
            pick_living_fog_intent(None),
            LivingFogIntent::AdvancedGas
        );
        assert_eq!(
            pick_living_fog_intent(Some(LivingFogIntent::AdvancedGas)),
            LivingFogIntent::Bloat
        );
        assert_eq!(
            pick_living_fog_intent(Some(LivingFogIntent::Bloat)),
            LivingFogIntent::SuperGas
        );
        assert_eq!(
            pick_living_fog_intent(Some(LivingFogIntent::SuperGas)),
            LivingFogIntent::Bloat
        );
    }

    #[test]
    fn living_fog_advanced_gas_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_living_fog_move(&mut cs, 0, 0, LivingFogIntent::AdvancedGas);
        assert_eq!(cs.allies[0].current_hp, hp - 8);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "SmoggyPower"),
            1
        );
    }

    #[test]
    fn living_fog_bloat_deals_five() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_living_fog_move(&mut cs, 0, 0, LivingFogIntent::Bloat);
        assert_eq!(cs.allies[0].current_hp, hp - 5);
    }

    #[test]
    fn living_fog_super_gas_deals_eight() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_living_fog_move(&mut cs, 0, 0, LivingFogIntent::SuperGas);
        assert_eq!(cs.allies[0].current_hp, hp - 8);
    }

    // ---------- Fabricator tests -------------------------------------------

    #[test]
    fn fabricator_can_fabricate_picks_one_of_two() {
        let mut rng = Rng::new(1, 0);
        for _ in 0..20 {
            let intent = pick_fabricator_intent(&mut rng, None, true);
            assert!(matches!(
                intent,
                FabricatorIntent::Fabricate | FabricatorIntent::FabricatingStrike
            ));
        }
    }

    #[test]
    fn fabricator_cannot_fabricate_picks_disintegrate() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_fabricator_intent(&mut rng, None, false),
            FabricatorIntent::Disintegrate
        );
    }

    #[test]
    fn fabricator_fabricate_is_noop() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_fabricator_move(&mut cs, 0, 0, FabricatorIntent::Fabricate);
        assert_eq!(cs.allies[0].current_hp, hp);
    }

    #[test]
    fn fabricator_fabricating_strike_deals_eighteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_fabricator_move(
            &mut cs,
            0,
            0,
            FabricatorIntent::FabricatingStrike,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 18);
    }

    #[test]
    fn fabricator_disintegrate_deals_eleven() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_fabricator_move(&mut cs, 0, 0, FabricatorIntent::Disintegrate);
        assert_eq!(cs.allies[0].current_hp, hp - 11);
    }

    // ---------- Doormaker tests --------------------------------------------

    #[test]
    fn doormaker_walks_chain() {
        assert_eq!(
            pick_doormaker_intent(None),
            DoormakerIntent::DramaticOpen
        );
        assert_eq!(
            pick_doormaker_intent(Some(DoormakerIntent::DramaticOpen)),
            DoormakerIntent::Hunger
        );
        assert_eq!(
            pick_doormaker_intent(Some(DoormakerIntent::Hunger)),
            DoormakerIntent::Scrutiny
        );
        assert_eq!(
            pick_doormaker_intent(Some(DoormakerIntent::Scrutiny)),
            DoormakerIntent::Grasp
        );
        assert_eq!(
            pick_doormaker_intent(Some(DoormakerIntent::Grasp)),
            DoormakerIntent::Hunger
        );
    }

    #[test]
    fn doormaker_dramatic_open_is_noop() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_doormaker_move(&mut cs, 0, 0, DoormakerIntent::DramaticOpen);
        assert_eq!(cs.allies[0].current_hp, hp);
    }

    #[test]
    fn doormaker_hunger_deals_thirty() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_doormaker_move(&mut cs, 0, 0, DoormakerIntent::Hunger);
        assert_eq!(cs.allies[0].current_hp, hp - 30);
    }

    #[test]
    fn doormaker_scrutiny_deals_twenty_four() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_doormaker_move(&mut cs, 0, 0, DoormakerIntent::Scrutiny);
        assert_eq!(cs.allies[0].current_hp, hp - 24);
    }

    #[test]
    fn doormaker_grasp_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_doormaker_move(&mut cs, 0, 0, DoormakerIntent::Grasp);
        assert_eq!(cs.allies[0].current_hp, hp - 20);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            3
        );
    }

    // ---------- LagavulinMatriarch + AsleepPower tests ---------------------

    #[test]
    fn lagavulin_sleeps_while_asleep_up() {
        assert_eq!(
            pick_lagavulin_matriarch_intent(None, true),
            LagavulinMatriarchIntent::Sleep
        );
        assert_eq!(
            pick_lagavulin_matriarch_intent(
                Some(LagavulinMatriarchIntent::Sleep),
                true
            ),
            LagavulinMatriarchIntent::Sleep
        );
    }

    #[test]
    fn lagavulin_wakes_into_slash_then_loops() {
        assert_eq!(
            pick_lagavulin_matriarch_intent(
                Some(LagavulinMatriarchIntent::Sleep),
                false
            ),
            LagavulinMatriarchIntent::Slash
        );
        assert_eq!(
            pick_lagavulin_matriarch_intent(
                Some(LagavulinMatriarchIntent::Slash),
                false
            ),
            LagavulinMatriarchIntent::Disembowel
        );
        assert_eq!(
            pick_lagavulin_matriarch_intent(
                Some(LagavulinMatriarchIntent::Disembowel),
                false
            ),
            LagavulinMatriarchIntent::Slash2
        );
        assert_eq!(
            pick_lagavulin_matriarch_intent(
                Some(LagavulinMatriarchIntent::Slash2),
                false
            ),
            LagavulinMatriarchIntent::SoulSiphon
        );
        assert_eq!(
            pick_lagavulin_matriarch_intent(
                Some(LagavulinMatriarchIntent::SoulSiphon),
                false
            ),
            LagavulinMatriarchIntent::Slash
        );
    }

    #[test]
    fn lagavulin_spawn_applies_plating_and_asleep() {
        let mut cs = ironclad_combat();
        lagavulin_matriarch_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PlatingPower"),
            12
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "AsleepPower"),
            3
        );
    }

    #[test]
    fn asleep_wakes_on_first_unblocked_damage_removes_plating() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PlatingPower", 12);
        cs.apply_power(CombatSide::Enemy, 0, "AsleepPower", 3);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            5,
            ValueProp::MOVE,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "AsleepPower"),
            0
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PlatingPower"),
            0
        );
    }

    #[test]
    fn asleep_does_not_wake_when_fully_blocked() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PlatingPower", 12);
        cs.apply_power(CombatSide::Enemy, 0, "AsleepPower", 3);
        cs.gain_block(CombatSide::Enemy, 0, 100);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            5,
            ValueProp::MOVE,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "AsleepPower"),
            3
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PlatingPower"),
            12
        );
    }

    #[test]
    fn asleep_decrements_at_enemy_turn_end() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "AsleepPower", 3);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "AsleepPower"),
            2
        );
    }

    #[test]
    fn asleep_natural_wake_strips_plating_at_zero() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PlatingPower", 12);
        cs.apply_power(CombatSide::Enemy, 0, "AsleepPower", 1);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "AsleepPower"),
            0
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PlatingPower"),
            0
        );
    }

    // ---------- HauntedShip tests ------------------------------------------

    #[test]
    fn haunted_ship_init_is_haunt() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_haunted_ship_intent(&mut rng, None),
            HauntedShipIntent::Haunt
        );
    }

    #[test]
    fn haunted_ship_post_haunt_never_repeats() {
        let mut rng = Rng::new(42, 0);
        for _ in 0..30 {
            for &start in &[
                HauntedShipIntent::Haunt,
                HauntedShipIntent::RammingSpeed,
                HauntedShipIntent::Swipe,
                HauntedShipIntent::Stomp,
            ] {
                let next = pick_haunted_ship_intent(&mut rng, Some(start));
                assert_ne!(next, start);
                assert_ne!(next, HauntedShipIntent::Haunt);
            }
        }
    }

    #[test]
    fn haunted_ship_haunt_adds_five_dazed() {
        let mut cs = ironclad_combat();
        execute_haunted_ship_move(&mut cs, 0, 0, HauntedShipIntent::Haunt);
        let dazed = cs.allies[0]
            .player
            .as_ref()
            .map(|p| p.discard.cards.iter().filter(|c| c.id == "Dazed").count())
            .unwrap_or(0);
        assert_eq!(dazed, 5);
    }

    #[test]
    fn haunted_ship_ramming_speed_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_haunted_ship_move(
            &mut cs,
            0,
            0,
            HauntedShipIntent::RammingSpeed,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 10);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "WeakPower"),
            1
        );
    }

    #[test]
    fn haunted_ship_swipe_deals_thirteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_haunted_ship_move(&mut cs, 0, 0, HauntedShipIntent::Swipe);
        assert_eq!(cs.allies[0].current_hp, hp - 13);
    }

    #[test]
    fn haunted_ship_stomp_four_times_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_haunted_ship_move(&mut cs, 0, 0, HauntedShipIntent::Stomp);
        assert_eq!(cs.allies[0].current_hp, hp - 12);
    }

    // ---------- Queen tests ------------------------------------------------

    #[test]
    fn queen_walks_alive_amalgam_path() {
        // alive amalgam → PuppetStrings → YoureMine → BurnBright
        // (loops).
        assert_eq!(
            pick_queen_intent(None, false),
            QueenIntent::PuppetStrings
        );
        assert_eq!(
            pick_queen_intent(Some(QueenIntent::PuppetStrings), false),
            QueenIntent::YoureMine
        );
        assert_eq!(
            pick_queen_intent(Some(QueenIntent::YoureMine), false),
            QueenIntent::BurnBrightForMe
        );
        for _ in 0..3 {
            assert_eq!(
                pick_queen_intent(Some(QueenIntent::BurnBrightForMe), false),
                QueenIntent::BurnBrightForMe
            );
        }
    }

    #[test]
    fn queen_pivots_when_amalgam_dies() {
        // YoureMine → OffWithYourHead → Execution → Enrage →
        // OffWithYourHead (loop).
        assert_eq!(
            pick_queen_intent(Some(QueenIntent::YoureMine), true),
            QueenIntent::OffWithYourHead
        );
        assert_eq!(
            pick_queen_intent(Some(QueenIntent::OffWithYourHead), true),
            QueenIntent::Execution
        );
        assert_eq!(
            pick_queen_intent(Some(QueenIntent::Execution), true),
            QueenIntent::Enrage
        );
        assert_eq!(
            pick_queen_intent(Some(QueenIntent::Enrage), true),
            QueenIntent::OffWithYourHead
        );
        // BurnBrightForMe with amalgam dead also pivots.
        assert_eq!(
            pick_queen_intent(Some(QueenIntent::BurnBrightForMe), true),
            QueenIntent::OffWithYourHead
        );
    }

    #[test]
    fn queen_youre_mine_applies_99_each() {
        let mut cs = ironclad_combat();
        execute_queen_move(&mut cs, 0, 0, QueenIntent::YoureMine);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "FrailPower"),
            99
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "WeakPower"),
            99
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "VulnerablePower"),
            99
        );
    }

    #[test]
    fn queen_off_with_head_3_times_5() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_queen_move(&mut cs, 0, 0, QueenIntent::OffWithYourHead);
        assert_eq!(cs.allies[0].current_hp, hp - 15);
    }

    #[test]
    fn queen_execution_deals_fifteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_queen_move(&mut cs, 0, 0, QueenIntent::Execution);
        assert_eq!(cs.allies[0].current_hp, hp - 15);
    }

    #[test]
    fn queen_enrage_grants_two_strength() {
        let mut cs = ironclad_combat();
        execute_queen_move(&mut cs, 0, 0, QueenIntent::Enrage);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    // ---------- Crusher + Rocket tests -------------------------------------

    #[test]
    fn crusher_walks_five_state_chain() {
        assert_eq!(pick_crusher_intent(None), CrusherIntent::Thrash);
        assert_eq!(
            pick_crusher_intent(Some(CrusherIntent::Thrash)),
            CrusherIntent::EnlargingStrike
        );
        assert_eq!(
            pick_crusher_intent(Some(CrusherIntent::EnlargingStrike)),
            CrusherIntent::BugSting
        );
        assert_eq!(
            pick_crusher_intent(Some(CrusherIntent::BugSting)),
            CrusherIntent::Adapt
        );
        assert_eq!(
            pick_crusher_intent(Some(CrusherIntent::Adapt)),
            CrusherIntent::GuardedStrike
        );
        assert_eq!(
            pick_crusher_intent(Some(CrusherIntent::GuardedStrike)),
            CrusherIntent::Thrash
        );
    }

    #[test]
    fn crusher_thrash_deals_twelve() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_crusher_move(&mut cs, 0, 0, CrusherIntent::Thrash);
        assert_eq!(cs.allies[0].current_hp, hp - 12);
    }

    #[test]
    fn crusher_bug_sting_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_crusher_move(&mut cs, 0, 0, CrusherIntent::BugSting);
        assert_eq!(cs.allies[0].current_hp, hp - 12);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "WeakPower"),
            2
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "FrailPower"),
            2
        );
    }

    #[test]
    fn crusher_guarded_strike_damage_plus_block() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_crusher_move(&mut cs, 0, 0, CrusherIntent::GuardedStrike);
        assert_eq!(cs.allies[0].current_hp, hp - 12);
        assert_eq!(cs.enemies[0].block, 18);
    }

    #[test]
    fn rocket_walks_five_state_chain() {
        assert_eq!(pick_rocket_intent(None), RocketIntent::TargetingReticle);
        assert_eq!(
            pick_rocket_intent(Some(RocketIntent::TargetingReticle)),
            RocketIntent::PrecisionBeam
        );
        assert_eq!(
            pick_rocket_intent(Some(RocketIntent::PrecisionBeam)),
            RocketIntent::ChargeUp
        );
        assert_eq!(
            pick_rocket_intent(Some(RocketIntent::ChargeUp)),
            RocketIntent::Laser
        );
        assert_eq!(
            pick_rocket_intent(Some(RocketIntent::Laser)),
            RocketIntent::Recharge
        );
        assert_eq!(
            pick_rocket_intent(Some(RocketIntent::Recharge)),
            RocketIntent::TargetingReticle
        );
    }

    #[test]
    fn rocket_laser_deals_thirty_one() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_rocket_move(&mut cs, 0, 0, RocketIntent::Laser);
        assert_eq!(cs.allies[0].current_hp, hp - 31);
    }

    #[test]
    fn rocket_recharge_is_noop() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_rocket_move(&mut cs, 0, 0, RocketIntent::Recharge);
        assert_eq!(cs.allies[0].current_hp, hp);
    }

    // ---------- Ovicopter tests --------------------------------------------

    #[test]
    fn ovicopter_walks_chain_with_can_lay() {
        assert_eq!(pick_ovicopter_intent(None, true), OvicopterIntent::LayEggs);
        assert_eq!(
            pick_ovicopter_intent(Some(OvicopterIntent::LayEggs), true),
            OvicopterIntent::Smash
        );
        assert_eq!(
            pick_ovicopter_intent(Some(OvicopterIntent::Smash), true),
            OvicopterIntent::Tenderizer
        );
        // CanLay=true after Tenderizer → back to LayEggs.
        assert_eq!(
            pick_ovicopter_intent(Some(OvicopterIntent::Tenderizer), true),
            OvicopterIntent::LayEggs
        );
        // CanLay=false after Tenderizer → NutritionalPaste.
        assert_eq!(
            pick_ovicopter_intent(Some(OvicopterIntent::Tenderizer), false),
            OvicopterIntent::NutritionalPaste
        );
        assert_eq!(
            pick_ovicopter_intent(Some(OvicopterIntent::NutritionalPaste), false),
            OvicopterIntent::Smash
        );
    }

    #[test]
    fn ovicopter_lay_eggs_is_noop() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_ovicopter_move(&mut cs, 0, 0, OvicopterIntent::LayEggs);
        assert_eq!(cs.allies[0].current_hp, hp);
    }

    #[test]
    fn ovicopter_smash_deals_sixteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_ovicopter_move(&mut cs, 0, 0, OvicopterIntent::Smash);
        assert_eq!(cs.allies[0].current_hp, hp - 16);
    }

    #[test]
    fn ovicopter_tenderizer_seven_dmg_plus_vuln() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_ovicopter_move(&mut cs, 0, 0, OvicopterIntent::Tenderizer);
        assert_eq!(cs.allies[0].current_hp, hp - 7);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "VulnerablePower"),
            2
        );
    }

    #[test]
    fn ovicopter_nutritional_paste_grants_three_strength() {
        let mut cs = ironclad_combat();
        execute_ovicopter_move(&mut cs, 0, 0, OvicopterIntent::NutritionalPaste);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            3
        );
    }

    // ---------- MagiKnight + SpectralKnight tests --------------------------

    #[test]
    fn magi_knight_walks_chain() {
        assert_eq!(
            pick_magi_knight_intent(None),
            MagiKnightIntent::PowerShield
        );
        assert_eq!(
            pick_magi_knight_intent(Some(MagiKnightIntent::PowerShield)),
            MagiKnightIntent::Dampen
        );
        assert_eq!(
            pick_magi_knight_intent(Some(MagiKnightIntent::Dampen)),
            MagiKnightIntent::Spear
        );
        assert_eq!(
            pick_magi_knight_intent(Some(MagiKnightIntent::Spear)),
            MagiKnightIntent::Prep
        );
        assert_eq!(
            pick_magi_knight_intent(Some(MagiKnightIntent::Prep)),
            MagiKnightIntent::MagicBomb
        );
        assert_eq!(
            pick_magi_knight_intent(Some(MagiKnightIntent::MagicBomb)),
            MagiKnightIntent::Spear
        );
    }

    #[test]
    fn magi_knight_power_shield_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_magi_knight_move(&mut cs, 0, 0, MagiKnightIntent::PowerShield);
        assert_eq!(cs.allies[0].current_hp, hp - 6);
        assert_eq!(cs.enemies[0].block, 5);
    }

    #[test]
    fn magi_knight_spear_deals_ten() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_magi_knight_move(&mut cs, 0, 0, MagiKnightIntent::Spear);
        assert_eq!(cs.allies[0].current_hp, hp - 10);
    }

    #[test]
    fn magi_knight_magic_bomb_deals_thirty_five() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_magi_knight_move(&mut cs, 0, 0, MagiKnightIntent::MagicBomb);
        assert_eq!(cs.allies[0].current_hp, hp - 35);
    }

    #[test]
    fn magi_knight_dampen_applies_marker_stack() {
        let mut cs = ironclad_combat();
        execute_magi_knight_move(&mut cs, 0, 0, MagiKnightIntent::Dampen);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "DampenPower"),
            1
        );
    }

    #[test]
    fn spectral_knight_init_is_hex() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_spectral_knight_intent(&mut rng, None),
            SpectralKnightIntent::Hex
        );
    }

    #[test]
    fn spectral_knight_after_hex_is_soul_slash() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_spectral_knight_intent(&mut rng, Some(SpectralKnightIntent::Hex)),
            SpectralKnightIntent::SoulSlash
        );
    }

    #[test]
    fn spectral_knight_after_flame_must_slash() {
        let mut rng = Rng::new(1, 0);
        for _ in 0..20 {
            assert_eq!(
                pick_spectral_knight_intent(
                    &mut rng,
                    Some(SpectralKnightIntent::SoulFlame)
                ),
                SpectralKnightIntent::SoulSlash
            );
        }
    }

    #[test]
    fn spectral_knight_after_slash_weighted_pick() {
        // 2:1 weighting — over many trials, SoulSlash should
        // dominate but SoulFlame appears too.
        let mut rng = Rng::new(123, 0);
        let mut slash = 0;
        let mut flame = 0;
        for _ in 0..3000 {
            match pick_spectral_knight_intent(
                &mut rng,
                Some(SpectralKnightIntent::SoulSlash),
            ) {
                SpectralKnightIntent::SoulSlash => slash += 1,
                SpectralKnightIntent::SoulFlame => flame += 1,
                _ => panic!("unexpected"),
            }
        }
        // expect roughly 2000/1000 split.
        assert!((slash - 2000_i32).abs() < 200, "slash={slash}");
        assert!((flame - 1000_i32).abs() < 200, "flame={flame}");
    }

    #[test]
    fn spectral_knight_hex_applies_marker_stack() {
        // HexPower is Single stack in C# (and our table) — the C#
        // Apply<HexPower>(2) sets it present; our Single handling
        // clamps to 1, which is the same "presence" semantic.
        let mut cs = ironclad_combat();
        execute_spectral_knight_move(&mut cs, 0, 0, SpectralKnightIntent::Hex);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "HexPower"),
            1
        );
    }

    #[test]
    fn spectral_knight_soul_slash_deals_fifteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_spectral_knight_move(
            &mut cs,
            0,
            0,
            SpectralKnightIntent::SoulSlash,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 15);
    }

    #[test]
    fn spectral_knight_soul_flame_three_times_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_spectral_knight_move(
            &mut cs,
            0,
            0,
            SpectralKnightIntent::SoulFlame,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 9);
    }

    // ---------- Tunneler + BurrowedPower tests -----------------------------

    #[test]
    fn tunneler_walks_chain_bite_burrow_below_loop() {
        assert_eq!(pick_tunneler_intent(None), TunnelerIntent::Bite);
        assert_eq!(
            pick_tunneler_intent(Some(TunnelerIntent::Bite)),
            TunnelerIntent::Burrow
        );
        assert_eq!(
            pick_tunneler_intent(Some(TunnelerIntent::Burrow)),
            TunnelerIntent::Below
        );
        // Below loops.
        for _ in 0..5 {
            assert_eq!(
                pick_tunneler_intent(Some(TunnelerIntent::Below)),
                TunnelerIntent::Below
            );
        }
    }

    #[test]
    fn tunneler_bite_deals_thirteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_tunneler_move(&mut cs, 0, 0, TunnelerIntent::Bite);
        assert_eq!(cs.allies[0].current_hp, hp - 13);
    }

    #[test]
    fn tunneler_burrow_applies_power_and_block() {
        let mut cs = ironclad_combat();
        execute_tunneler_move(&mut cs, 0, 0, TunnelerIntent::Burrow);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "BurrowedPower"),
            1
        );
        assert_eq!(cs.enemies[0].block, 12);
    }

    #[test]
    fn tunneler_below_deals_twenty_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_tunneler_move(&mut cs, 0, 0, TunnelerIntent::Below);
        assert_eq!(cs.allies[0].current_hp, hp - 23);
    }

    #[test]
    fn burrowed_preserves_block_across_owner_turn_start() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "BurrowedPower", 1);
        cs.enemies[0].block = 12;
        // Simulate enemy turn start — block should persist.
        cs.current_side = CombatSide::Player;
        cs.begin_turn(CombatSide::Enemy);
        assert_eq!(cs.enemies[0].block, 12);
    }

    #[test]
    fn burrowed_does_not_preserve_block_on_player_turn_start() {
        // Burrowed is on enemy. Player turn start clears player block,
        // which is unrelated — verify behavior isn't unintentionally
        // spilled to allies.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "BurrowedPower", 1);
        cs.allies[0].block = 5;
        cs.current_side = CombatSide::Enemy;
        cs.begin_turn(CombatSide::Player);
        assert_eq!(cs.allies[0].block, 0);
    }

    // ---------- TheInsatiable tests ----------------------------------------

    #[test]
    fn insatiable_walks_five_state_chain() {
        assert_eq!(
            pick_the_insatiable_intent(None),
            TheInsatiableIntent::Liquify
        );
        assert_eq!(
            pick_the_insatiable_intent(Some(TheInsatiableIntent::Liquify)),
            TheInsatiableIntent::Thrash1
        );
        assert_eq!(
            pick_the_insatiable_intent(Some(TheInsatiableIntent::Thrash1)),
            TheInsatiableIntent::Bite
        );
        assert_eq!(
            pick_the_insatiable_intent(Some(TheInsatiableIntent::Bite)),
            TheInsatiableIntent::Salivate
        );
        assert_eq!(
            pick_the_insatiable_intent(Some(TheInsatiableIntent::Salivate)),
            TheInsatiableIntent::Thrash2
        );
        assert_eq!(
            pick_the_insatiable_intent(Some(TheInsatiableIntent::Thrash2)),
            TheInsatiableIntent::Thrash1
        );
    }

    #[test]
    fn insatiable_liquify_adds_six_franticescape() {
        let mut cs = ironclad_combat();
        execute_the_insatiable_move(&mut cs, 0, 0, TheInsatiableIntent::Liquify);
        let draw_fe = cs.allies[0]
            .player
            .as_ref()
            .map(|p| {
                p.draw
                    .cards
                    .iter()
                    .filter(|c| c.id == "FranticEscape")
                    .count()
            })
            .unwrap_or(0);
        let disc_fe = cs.allies[0]
            .player
            .as_ref()
            .map(|p| {
                p.discard
                    .cards
                    .iter()
                    .filter(|c| c.id == "FranticEscape")
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(draw_fe, 3);
        assert_eq!(disc_fe, 3);
    }

    #[test]
    fn insatiable_thrash_eight_times_two() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_the_insatiable_move(&mut cs, 0, 0, TheInsatiableIntent::Thrash1);
        assert_eq!(cs.allies[0].current_hp, hp - 16);
    }

    #[test]
    fn insatiable_bite_deals_twenty_eight() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_the_insatiable_move(&mut cs, 0, 0, TheInsatiableIntent::Bite);
        assert_eq!(cs.allies[0].current_hp, hp - 28);
    }

    #[test]
    fn insatiable_salivate_grants_two_strength() {
        let mut cs = ironclad_combat();
        execute_the_insatiable_move(&mut cs, 0, 0, TheInsatiableIntent::Salivate);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    // ---------- SlumberingBeetle + SlumberPower tests ----------------------

    #[test]
    fn slumbering_beetle_snores_while_slumber_present() {
        assert_eq!(
            pick_slumbering_beetle_intent(None, true),
            SlumberingBeetleIntent::Snore
        );
        assert_eq!(
            pick_slumbering_beetle_intent(
                Some(SlumberingBeetleIntent::Snore),
                true
            ),
            SlumberingBeetleIntent::Snore
        );
    }

    #[test]
    fn slumbering_beetle_rolls_out_when_slumber_clears() {
        assert_eq!(
            pick_slumbering_beetle_intent(None, false),
            SlumberingBeetleIntent::Rollout
        );
        assert_eq!(
            pick_slumbering_beetle_intent(
                Some(SlumberingBeetleIntent::Snore),
                false
            ),
            SlumberingBeetleIntent::Rollout
        );
    }

    #[test]
    fn slumbering_beetle_spawn_applies_plating_and_slumber() {
        let mut cs = ironclad_combat();
        slumbering_beetle_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PlatingPower"),
            15
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "SlumberPower"),
            3
        );
    }

    #[test]
    fn slumbering_beetle_rollout_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_slumbering_beetle_move(
            &mut cs,
            0,
            0,
            SlumberingBeetleIntent::Rollout,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 16);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    #[test]
    fn slumber_decrements_each_enemy_turn_end() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "SlumberPower", 3);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "SlumberPower"),
            2
        );
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "SlumberPower"),
            1
        );
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "SlumberPower"),
            0
        );
    }

    #[test]
    fn slumber_decrements_on_unblocked_damage() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "SlumberPower", 3);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "SlumberPower"),
            2
        );
    }

    #[test]
    fn slumber_not_decremented_when_fully_blocked() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "SlumberPower", 3);
        cs.gain_block(CombatSide::Enemy, 0, 100);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "SlumberPower"),
            3
        );
    }

    // ---------- PlatingPower tests -----------------------------------------

    #[test]
    fn plating_round_one_player_start_grants_block_once() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PlatingPower", 6);
        // Stage current_side=Player so begin_turn(Player) doesn't
        // bump round_number. Start round_number at 1.
        cs.current_side = CombatSide::Player;
        cs.round_number = 1;
        cs.begin_turn(CombatSide::Player);
        assert_eq!(cs.enemies[0].block, 6);
        // Round 2 player start — no second one-shot.
        cs.enemies[0].block = 0;
        cs.round_number = 2;
        cs.begin_turn(CombatSide::Player);
        assert_eq!(cs.enemies[0].block, 0);
    }

    #[test]
    fn plating_enemy_turn_end_grants_then_decrements() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PlatingPower", 6);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(cs.enemies[0].block, 6);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PlatingPower"),
            5
        );
        // Reset block (begin_turn would clear it), simulate again.
        cs.enemies[0].block = 0;
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(cs.enemies[0].block, 5);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PlatingPower"),
            4
        );
    }

    #[test]
    fn plating_stops_granting_at_zero() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PlatingPower", 1);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(cs.enemies[0].block, 1);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PlatingPower"),
            0
        );
        cs.enemies[0].block = 0;
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(cs.enemies[0].block, 0);
    }

    // ---------- DecimillipedeSegment tests ---------------------------------

    #[test]
    fn decimillipede_segment_walks_three_state_chain() {
        assert_eq!(
            pick_decimillipede_segment_intent(None),
            DecimillipedeSegmentIntent::Constrict
        );
        assert_eq!(
            pick_decimillipede_segment_intent(Some(
                DecimillipedeSegmentIntent::Constrict
            )),
            DecimillipedeSegmentIntent::Bulk
        );
        assert_eq!(
            pick_decimillipede_segment_intent(Some(
                DecimillipedeSegmentIntent::Bulk
            )),
            DecimillipedeSegmentIntent::Writhe
        );
        assert_eq!(
            pick_decimillipede_segment_intent(Some(
                DecimillipedeSegmentIntent::Writhe
            )),
            DecimillipedeSegmentIntent::Constrict
        );
    }

    #[test]
    fn decimillipede_segment_constrict_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_decimillipede_segment_move(
            &mut cs,
            0,
            0,
            DecimillipedeSegmentIntent::Constrict,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 8);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "WeakPower"),
            1
        );
    }

    #[test]
    fn decimillipede_segment_bulk_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_decimillipede_segment_move(
            &mut cs,
            0,
            0,
            DecimillipedeSegmentIntent::Bulk,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 6);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    #[test]
    fn decimillipede_segment_writhe_five_times_two() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_decimillipede_segment_move(
            &mut cs,
            0,
            0,
            DecimillipedeSegmentIntent::Writhe,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 10);
    }

    // ---------- TorchHeadAmalgam tests -------------------------------------

    #[test]
    fn torch_head_walks_five_state_chain() {
        assert_eq!(
            pick_torch_head_amalgam_intent(None),
            TorchHeadAmalgamIntent::Tackle1
        );
        assert_eq!(
            pick_torch_head_amalgam_intent(Some(TorchHeadAmalgamIntent::Tackle1)),
            TorchHeadAmalgamIntent::Tackle2
        );
        assert_eq!(
            pick_torch_head_amalgam_intent(Some(TorchHeadAmalgamIntent::Tackle2)),
            TorchHeadAmalgamIntent::Beam
        );
        assert_eq!(
            pick_torch_head_amalgam_intent(Some(TorchHeadAmalgamIntent::Beam)),
            TorchHeadAmalgamIntent::Tackle3
        );
        assert_eq!(
            pick_torch_head_amalgam_intent(Some(TorchHeadAmalgamIntent::Tackle3)),
            TorchHeadAmalgamIntent::Tackle4
        );
        assert_eq!(
            pick_torch_head_amalgam_intent(Some(TorchHeadAmalgamIntent::Tackle4)),
            TorchHeadAmalgamIntent::Beam
        );
    }

    #[test]
    fn torch_head_tackle1_2_deal_eighteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_torch_head_amalgam_move(
            &mut cs,
            0,
            0,
            TorchHeadAmalgamIntent::Tackle1,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 18);
        let hp2 = cs.allies[0].current_hp;
        execute_torch_head_amalgam_move(
            &mut cs,
            0,
            0,
            TorchHeadAmalgamIntent::Tackle2,
        );
        assert_eq!(cs.allies[0].current_hp, hp2 - 18);
    }

    #[test]
    fn torch_head_tackle3_4_deal_fourteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_torch_head_amalgam_move(
            &mut cs,
            0,
            0,
            TorchHeadAmalgamIntent::Tackle3,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 14);
    }

    #[test]
    fn torch_head_beam_eight_times_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_torch_head_amalgam_move(
            &mut cs,
            0,
            0,
            TorchHeadAmalgamIntent::Beam,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 24);
    }

    // ---------- SoulFysh tests ---------------------------------------------

    #[test]
    fn soul_fysh_walks_five_state_chain() {
        assert_eq!(pick_soul_fysh_intent(None), SoulFyshIntent::Beckon);
        assert_eq!(
            pick_soul_fysh_intent(Some(SoulFyshIntent::Beckon)),
            SoulFyshIntent::DeGas
        );
        assert_eq!(
            pick_soul_fysh_intent(Some(SoulFyshIntent::DeGas)),
            SoulFyshIntent::Gaze
        );
        assert_eq!(
            pick_soul_fysh_intent(Some(SoulFyshIntent::Gaze)),
            SoulFyshIntent::Fade
        );
        assert_eq!(
            pick_soul_fysh_intent(Some(SoulFyshIntent::Fade)),
            SoulFyshIntent::Scream
        );
        assert_eq!(
            pick_soul_fysh_intent(Some(SoulFyshIntent::Scream)),
            SoulFyshIntent::Beckon
        );
    }

    #[test]
    fn soul_fysh_beckon_adds_two_beckons() {
        let mut cs = ironclad_combat();
        execute_soul_fysh_move(&mut cs, 0, 0, SoulFyshIntent::Beckon);
        let beckons = cs.allies[0]
            .player
            .as_ref()
            .map(|p| {
                p.discard
                    .cards
                    .iter()
                    .filter(|c| c.id == "Beckon")
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(beckons, 2);
    }

    #[test]
    fn soul_fysh_de_gas_deals_sixteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_soul_fysh_move(&mut cs, 0, 0, SoulFyshIntent::DeGas);
        assert_eq!(cs.allies[0].current_hp, hp - 16);
    }

    #[test]
    fn soul_fysh_gaze_seven_damage_plus_beckon() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_soul_fysh_move(&mut cs, 0, 0, SoulFyshIntent::Gaze);
        assert_eq!(cs.allies[0].current_hp, hp - 7);
        let beckons = cs.allies[0]
            .player
            .as_ref()
            .map(|p| {
                p.discard
                    .cards
                    .iter()
                    .filter(|c| c.id == "Beckon")
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(beckons, 1);
    }

    #[test]
    fn soul_fysh_fade_applies_intangible_two() {
        let mut cs = ironclad_combat();
        execute_soul_fysh_move(&mut cs, 0, 0, SoulFyshIntent::Fade);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "IntangiblePower"),
            2
        );
    }

    #[test]
    fn soul_fysh_scream_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_soul_fysh_move(&mut cs, 0, 0, SoulFyshIntent::Scream);
        assert_eq!(cs.allies[0].current_hp, hp - 11);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "VulnerablePower"),
            3
        );
    }

    // ---------- PhrogParasite tests ----------------------------------------

    #[test]
    fn phrog_parasite_alternates_infect_lash() {
        assert_eq!(
            pick_phrog_parasite_intent(None),
            PhrogParasiteIntent::Infect
        );
        assert_eq!(
            pick_phrog_parasite_intent(Some(PhrogParasiteIntent::Infect)),
            PhrogParasiteIntent::Lash
        );
        assert_eq!(
            pick_phrog_parasite_intent(Some(PhrogParasiteIntent::Lash)),
            PhrogParasiteIntent::Infect
        );
    }

    #[test]
    fn phrog_parasite_infect_adds_three_infections() {
        let mut cs = ironclad_combat();
        execute_phrog_parasite_move(&mut cs, 0, 0, PhrogParasiteIntent::Infect);
        let infections = cs.allies[0]
            .player
            .as_ref()
            .map(|p| {
                p.discard
                    .cards
                    .iter()
                    .filter(|c| c.id == "Infection")
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(infections, 3);
    }

    #[test]
    fn phrog_parasite_lash_four_times_four() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_phrog_parasite_move(&mut cs, 0, 0, PhrogParasiteIntent::Lash);
        assert_eq!(cs.allies[0].current_hp, hp - 16);
    }

    // ---------- InfestedPrism + VitalSparkPower tests ----------------------

    #[test]
    fn infested_prism_walks_four_state_chain() {
        assert_eq!(
            pick_infested_prism_intent(None),
            InfestedPrismIntent::Jab
        );
        assert_eq!(
            pick_infested_prism_intent(Some(InfestedPrismIntent::Jab)),
            InfestedPrismIntent::Radiate
        );
        assert_eq!(
            pick_infested_prism_intent(Some(InfestedPrismIntent::Radiate)),
            InfestedPrismIntent::Whirlwind
        );
        assert_eq!(
            pick_infested_prism_intent(Some(InfestedPrismIntent::Whirlwind)),
            InfestedPrismIntent::Pulsate
        );
        assert_eq!(
            pick_infested_prism_intent(Some(InfestedPrismIntent::Pulsate)),
            InfestedPrismIntent::Jab
        );
    }

    #[test]
    fn infested_prism_spawn_applies_vital_spark_one() {
        let mut cs = ironclad_combat();
        infested_prism_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VitalSparkPower"),
            1
        );
    }

    #[test]
    fn infested_prism_jab_deals_twenty_two() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_infested_prism_move(&mut cs, 0, 0, InfestedPrismIntent::Jab);
        assert_eq!(cs.allies[0].current_hp, hp - 22);
    }

    #[test]
    fn infested_prism_radiate_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_infested_prism_move(
            &mut cs,
            0,
            0,
            InfestedPrismIntent::Radiate,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 16);
        assert_eq!(cs.enemies[0].block, 16);
    }

    #[test]
    fn infested_prism_whirlwind_nine_times_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_infested_prism_move(
            &mut cs,
            0,
            0,
            InfestedPrismIntent::Whirlwind,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 27);
    }

    #[test]
    fn infested_prism_pulsate_payload() {
        let mut cs = ironclad_combat();
        execute_infested_prism_move(
            &mut cs,
            0,
            0,
            InfestedPrismIntent::Pulsate,
        );
        assert_eq!(cs.enemies[0].block, 20);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            4
        );
    }

    #[test]
    fn vital_spark_grants_energy_on_first_unblocked_hit() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VitalSparkPower", 1);
        let energy_before = cs.allies[0]
            .player
            .as_ref()
            .map(|p| p.energy)
            .unwrap_or(0);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        let energy_after = cs.allies[0]
            .player
            .as_ref()
            .map(|p| p.energy)
            .unwrap_or(0);
        assert_eq!(energy_after, energy_before + 1);
        // Second hit same turn — no extra energy.
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        let energy_after_2 = cs.allies[0]
            .player
            .as_ref()
            .map(|p| p.energy)
            .unwrap_or(0);
        assert_eq!(energy_after_2, energy_before + 1);
    }

    #[test]
    fn vital_spark_does_not_grant_when_fully_blocked() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VitalSparkPower", 1);
        cs.gain_block(CombatSide::Enemy, 0, 100);
        let energy_before = cs.allies[0]
            .player
            .as_ref()
            .map(|p| p.energy)
            .unwrap_or(0);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        let energy_after = cs.allies[0]
            .player
            .as_ref()
            .map(|p| p.energy)
            .unwrap_or(0);
        assert_eq!(energy_after, energy_before);
    }

    #[test]
    fn vital_spark_re_arms_after_enemy_turn_start() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VitalSparkPower", 1);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        // begin_turn(Enemy) clears the flag.
        cs.current_side = CombatSide::Player;
        cs.begin_turn(CombatSide::Enemy);
        assert!(!cs.enemies[0]
            .monster
            .as_ref()
            .map(|m| m.flag("vital_spark_used"))
            .unwrap_or(true));
    }

    // ---------- PhantasmalGardener + SkittishPower tests -------------------

    #[test]
    fn phantasmal_gardener_init_by_slot() {
        assert_eq!(
            pick_phantasmal_gardener_intent(None, 1),
            PhantasmalGardenerIntent::Flail
        );
        assert_eq!(
            pick_phantasmal_gardener_intent(None, 2),
            PhantasmalGardenerIntent::Bite
        );
        assert_eq!(
            pick_phantasmal_gardener_intent(None, 3),
            PhantasmalGardenerIntent::Lash
        );
        assert_eq!(
            pick_phantasmal_gardener_intent(None, 4),
            PhantasmalGardenerIntent::Enlarge
        );
    }

    #[test]
    fn phantasmal_gardener_walks_four_state_cycle() {
        for slot in 1..=4 {
            assert_eq!(
                pick_phantasmal_gardener_intent(
                    Some(PhantasmalGardenerIntent::Bite),
                    slot
                ),
                PhantasmalGardenerIntent::Lash
            );
        }
        assert_eq!(
            pick_phantasmal_gardener_intent(
                Some(PhantasmalGardenerIntent::Lash),
                1
            ),
            PhantasmalGardenerIntent::Flail
        );
        assert_eq!(
            pick_phantasmal_gardener_intent(
                Some(PhantasmalGardenerIntent::Flail),
                1
            ),
            PhantasmalGardenerIntent::Enlarge
        );
        assert_eq!(
            pick_phantasmal_gardener_intent(
                Some(PhantasmalGardenerIntent::Enlarge),
                1
            ),
            PhantasmalGardenerIntent::Bite
        );
    }

    #[test]
    fn phantasmal_gardener_spawn_applies_skittish_six() {
        let mut cs = ironclad_combat();
        phantasmal_gardener_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "SkittishPower"),
            6
        );
    }

    #[test]
    fn phantasmal_gardener_bite_deals_five() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_phantasmal_gardener_move(
            &mut cs,
            0,
            0,
            PhantasmalGardenerIntent::Bite,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 5);
    }

    #[test]
    fn phantasmal_gardener_lash_deals_seven() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_phantasmal_gardener_move(
            &mut cs,
            0,
            0,
            PhantasmalGardenerIntent::Lash,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 7);
    }

    #[test]
    fn phantasmal_gardener_flail_one_times_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_phantasmal_gardener_move(
            &mut cs,
            0,
            0,
            PhantasmalGardenerIntent::Flail,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 3);
    }

    #[test]
    fn phantasmal_gardener_enlarge_grants_two_strength() {
        let mut cs = ironclad_combat();
        execute_phantasmal_gardener_move(
            &mut cs,
            0,
            0,
            PhantasmalGardenerIntent::Enlarge,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    #[test]
    fn skittish_triggers_once_per_turn_on_unblocked_player_attack() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "SkittishPower", 6);
        let hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 10);
        assert_eq!(cs.enemies[0].block, 6);
        // SkittishPower stays — not consumed, just flagged.
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "SkittishPower"),
            6
        );
        // Second hit same turn — no second block grant.
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        // Block soaks 6, then 4 of the 10 hits HP.
        assert_eq!(cs.enemies[0].current_hp, hp - 14);
        assert_eq!(cs.enemies[0].block, 0);
    }

    #[test]
    fn skittish_flag_clears_at_player_turn_end() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "SkittishPower", 6);
        // First Player turn — trigger once.
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert!(cs.enemies[0]
            .monster
            .as_ref()
            .map(|m| m.flag("skittish_used"))
            .unwrap_or(false));
        // End the Player turn — flag must clear.
        cs.current_side = CombatSide::Player;
        cs.end_turn();
        assert!(!cs.enemies[0]
            .monster
            .as_ref()
            .map(|m| m.flag("skittish_used"))
            .unwrap_or(true));
    }

    // ---------- TerrorEel + VigorPower + ShriekPower tests -----------------

    #[test]
    fn terror_eel_default_chain_crash_thrash() {
        assert_eq!(
            pick_terror_eel_intent(None, false),
            TerrorEelIntent::Crash
        );
        assert_eq!(
            pick_terror_eel_intent(Some(TerrorEelIntent::Crash), false),
            TerrorEelIntent::Thrash
        );
        assert_eq!(
            pick_terror_eel_intent(Some(TerrorEelIntent::Thrash), false),
            TerrorEelIntent::Crash
        );
    }

    #[test]
    fn terror_eel_shriek_routes_to_terror() {
        // Regardless of last intent, shriek_triggered forces Terror.
        assert_eq!(
            pick_terror_eel_intent(Some(TerrorEelIntent::Crash), true),
            TerrorEelIntent::Terror
        );
        assert_eq!(pick_terror_eel_intent(None, true), TerrorEelIntent::Terror);
    }

    #[test]
    fn terror_eel_spawn_applies_shriek_seventy() {
        let mut cs = ironclad_combat();
        terror_eel_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ShriekPower"),
            70
        );
    }

    #[test]
    fn terror_eel_crash_deals_sixteen_baseline() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_terror_eel_move(&mut cs, 0, 0, TerrorEelIntent::Crash);
        assert_eq!(cs.allies[0].current_hp, hp - 16);
    }

    #[test]
    fn terror_eel_thrash_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_terror_eel_move(&mut cs, 0, 0, TerrorEelIntent::Thrash);
        assert_eq!(cs.allies[0].current_hp, hp - 9);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VigorPower"),
            6
        );
    }

    #[test]
    fn terror_eel_terror_applies_vulnerable_and_clears_shriek_flag() {
        let mut cs = ironclad_combat();
        if let Some(ms) = cs.enemies[0].monster.as_mut() {
            ms.set_flag("shriek_triggered", true);
        }
        execute_terror_eel_move(&mut cs, 0, 0, TerrorEelIntent::Terror);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "VulnerablePower"),
            99
        );
        assert!(!cs.enemies[0]
            .monster
            .as_ref()
            .map(|m| m.flag("shriek_triggered"))
            .unwrap_or(true));
    }

    #[test]
    fn vigor_adds_to_dealer_damage() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VigorPower", 6);
        let hp = cs.allies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 16);
    }

    #[test]
    fn vigor_persists_when_applied_during_turn() {
        // Eel applies Vigor on Thrash. At end of THIS turn, Vigor was
        // 0 at the snapshot — so it stays 6 for next turn.
        let mut cs = ironclad_combat();
        cs.current_side = CombatSide::Player;
        cs.begin_turn(CombatSide::Enemy);
        // Mid-turn: Thrash applies Vigor.
        execute_terror_eel_move(&mut cs, 0, 0, TerrorEelIntent::Thrash);
        cs.end_turn();
        // Vigor not drained — it was 0 at snapshot.
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VigorPower"),
            6
        );
    }

    #[test]
    fn vigor_drains_after_attack() {
        // Audit fix #178: Vigor drains per-AttackCommand, not per-turn.
        // C# AttackCommand.Execute envelope: BeforeAttack snapshots Amount,
        // AfterAttack drains by snapshot. An enemy turn that contains
        // no attacks doesn't drain Vigor.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VigorPower", 6);
        // Run one attack — fire_before/after wrap deal_damage.
        cs.execute_attack(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            10,
            1,
            ValueProp::MOVE,
            None,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VigorPower"),
            0,
            "Vigor drains after the attack completes"
        );
    }

    #[test]
    fn vigor_does_not_drain_on_turn_boundary_without_attack() {
        // Audit fix #178: with the envelope-based fix, a turn that
        // contains no attacks leaves Vigor untouched. Matches C#:
        // AfterAttack only fires when an AttackCommand happens.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VigorPower", 6);
        cs.current_side = CombatSide::Player;
        cs.begin_turn(CombatSide::Enemy);
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VigorPower"),
            6,
            "no attack happened — Vigor persists"
        );
    }

    #[test]
    fn vigor_buffs_all_hits_of_a_multi_hit_attack() {
        // Audit fix #178: ModifyDamageAdditive reads live Amount, but
        // since Vigor isn't modified during the hits, all hits get the
        // same +Amount boost. C# VigorPower.ModifyDamageAdditive
        // returns base.Amount; AfterAttack drains the SNAPSHOT amount.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VigorPower", 4);
        let hp_before = cs.allies[0].current_hp;
        // 3-hit attack of base 2 damage. Each hit: 2+4=6. Total = 18.
        cs.execute_attack(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            2,
            3,
            ValueProp::MOVE,
            None,
        );
        // Block defaults to 0, no Frail/Dex; raw 18 damage hits HP.
        assert_eq!(cs.allies[0].current_hp, hp_before - 18);
        // Vigor drains by 4 after the attack.
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "VigorPower"),
            0
        );
    }

    #[test]
    fn shriek_fires_when_owner_drops_below_threshold() {
        // Enemy max_hp 50, Shriek(40). Player hits for damage that
        // takes HP to 30. Shriek fires, flag set, power removed.
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = 50;
        cs.enemies[0].max_hp = 50;
        cs.apply_power(CombatSide::Enemy, 0, "ShriekPower", 40);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            20,
            ValueProp::MOVE,
        );
        // HP now 30 ≤ 40 → Shriek fires.
        assert_eq!(cs.enemies[0].current_hp, 30);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ShriekPower"),
            0
        );
        assert!(cs.enemies[0]
            .monster
            .as_ref()
            .map(|m| m.flag("shriek_triggered"))
            .unwrap_or(false));
    }

    #[test]
    fn shriek_does_not_fire_above_threshold() {
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = 100;
        cs.enemies[0].max_hp = 100;
        cs.apply_power(CombatSide::Enemy, 0, "ShriekPower", 40);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ShriekPower"),
            40
        );
        assert!(!cs.enemies[0]
            .monster
            .as_ref()
            .map(|m| m.flag("shriek_triggered"))
            .unwrap_or(true));
    }

    // ---------- LouseProgenitor + CurlUpPower tests ------------------------

    #[test]
    fn louse_progenitor_walks_three_state_chain() {
        assert_eq!(
            pick_louse_progenitor_intent(None),
            LouseProgenitorIntent::CurlAndGrow
        );
        assert_eq!(
            pick_louse_progenitor_intent(Some(LouseProgenitorIntent::CurlAndGrow)),
            LouseProgenitorIntent::Pounce
        );
        assert_eq!(
            pick_louse_progenitor_intent(Some(LouseProgenitorIntent::Pounce)),
            LouseProgenitorIntent::Web
        );
        assert_eq!(
            pick_louse_progenitor_intent(Some(LouseProgenitorIntent::Web)),
            LouseProgenitorIntent::CurlAndGrow
        );
    }

    #[test]
    fn louse_progenitor_spawn_applies_curl_up_fourteen() {
        let mut cs = ironclad_combat();
        louse_progenitor_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "CurlUpPower"),
            14
        );
    }

    #[test]
    fn louse_progenitor_curl_and_grow_payload() {
        let mut cs = ironclad_combat();
        execute_louse_progenitor_move(
            &mut cs,
            0,
            0,
            LouseProgenitorIntent::CurlAndGrow,
        );
        assert_eq!(cs.enemies[0].block, 14);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            5
        );
        assert!(cs.enemies[0]
            .monster
            .as_ref()
            .map(|m| m.flag("curled"))
            .unwrap_or(false));
    }

    #[test]
    fn louse_progenitor_pounce_deals_fourteen_and_uncurls() {
        let mut cs = ironclad_combat();
        if let Some(ms) = cs.enemies[0].monster.as_mut() {
            ms.set_flag("curled", true);
        }
        let hp = cs.allies[0].current_hp;
        execute_louse_progenitor_move(
            &mut cs,
            0,
            0,
            LouseProgenitorIntent::Pounce,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 14);
        assert!(!cs.enemies[0]
            .monster
            .as_ref()
            .map(|m| m.flag("curled"))
            .unwrap_or(true));
    }

    #[test]
    fn louse_progenitor_web_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_louse_progenitor_move(&mut cs, 0, 0, LouseProgenitorIntent::Web);
        assert_eq!(cs.allies[0].current_hp, hp - 9);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "FrailPower"),
            2
        );
    }

    #[test]
    fn curl_up_triggers_on_first_player_powered_hp_hit() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "CurlUpPower", 14);
        let hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 10);
        assert_eq!(cs.enemies[0].block, 14);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "CurlUpPower"),
            0
        );
        assert!(cs.enemies[0]
            .monster
            .as_ref()
            .map(|m| m.flag("curled"))
            .unwrap_or(false));
    }

    #[test]
    fn curl_up_does_not_trigger_when_fully_blocked() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "CurlUpPower", 14);
        cs.gain_block(CombatSide::Enemy, 0, 100);
        let hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp);
        // CurlUp NOT consumed; player attack hit only block.
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "CurlUpPower"),
            14
        );
    }

    #[test]
    fn curl_up_does_not_trigger_on_unpowered_damage() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "CurlUpPower", 14);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::UNPOWERED.with(ValueProp::MOVE),
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "CurlUpPower"),
            14
        );
    }

    // ---------- SkulkingColony + HardenedShellPower tests ------------------

    #[test]
    fn skulking_colony_walks_four_state_chain() {
        assert_eq!(
            pick_skulking_colony_intent(None),
            SkulkingColonyIntent::Smash
        );
        assert_eq!(
            pick_skulking_colony_intent(Some(SkulkingColonyIntent::Smash)),
            SkulkingColonyIntent::Zoom
        );
        assert_eq!(
            pick_skulking_colony_intent(Some(SkulkingColonyIntent::Zoom)),
            SkulkingColonyIntent::Inertia
        );
        assert_eq!(
            pick_skulking_colony_intent(Some(SkulkingColonyIntent::Inertia)),
            SkulkingColonyIntent::PiercingStabs
        );
        assert_eq!(
            pick_skulking_colony_intent(Some(SkulkingColonyIntent::PiercingStabs)),
            SkulkingColonyIntent::Smash
        );
    }

    #[test]
    fn skulking_colony_spawn_applies_hardened_shell_fifteen() {
        let mut cs = ironclad_combat();
        skulking_colony_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "HardenedShellPower"),
            15
        );
    }

    #[test]
    fn skulking_colony_smash_deals_twelve() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_skulking_colony_move(
            &mut cs,
            0,
            0,
            SkulkingColonyIntent::Smash,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 12);
    }

    #[test]
    fn skulking_colony_zoom_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_skulking_colony_move(&mut cs, 0, 0, SkulkingColonyIntent::Zoom);
        assert_eq!(cs.allies[0].current_hp, hp - 14);
        assert_eq!(cs.enemies[0].block, 10);
    }

    #[test]
    fn skulking_colony_inertia_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_skulking_colony_move(&mut cs, 0, 0, SkulkingColonyIntent::Inertia);
        assert_eq!(cs.allies[0].current_hp, hp - 9);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    #[test]
    fn skulking_colony_piercing_stabs_seven_times_two() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_skulking_colony_move(
            &mut cs,
            0,
            0,
            SkulkingColonyIntent::PiercingStabs,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 14);
    }

    #[test]
    fn hardened_shell_caps_cumulative_hp_loss_per_turn() {
        // 100 hp, HardenedShell(15). Two 50-damage hits same turn:
        // first lands 15, second 0 (budget exhausted).
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "HardenedShellPower", 15);
        let hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            50,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 15);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            50,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 15);
    }

    #[test]
    fn hardened_shell_resets_on_player_turn_start() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "HardenedShellPower", 15);
        let hp = cs.enemies[0].current_hp;
        // Exhaust the budget this turn.
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            50,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 15);
        // Next player turn — counter resets, budget restored.
        cs.current_side = CombatSide::Enemy;
        cs.begin_turn(CombatSide::Player);
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            50,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 30);
    }

    #[test]
    fn hardened_shell_lets_block_soak_first() {
        // Enemy with 100 block + HardenedShell(15). 50 damage hit:
        // block absorbs all 50, no HP loss.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "HardenedShellPower", 15);
        cs.gain_block(CombatSide::Enemy, 0, 100);
        let hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            50,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp);
        assert_eq!(cs.enemies[0].block, 50);
    }

    // ---------- BygoneEffigy tests -----------------------------------------

    #[test]
    fn bygone_effigy_init_sleep_then_wake_then_slash_forever() {
        assert_eq!(
            pick_bygone_effigy_intent(None),
            BygoneEffigyIntent::InitialSleep
        );
        assert_eq!(
            pick_bygone_effigy_intent(Some(BygoneEffigyIntent::InitialSleep)),
            BygoneEffigyIntent::Wake
        );
        assert_eq!(
            pick_bygone_effigy_intent(Some(BygoneEffigyIntent::Wake)),
            BygoneEffigyIntent::Slash
        );
        for _ in 0..5 {
            assert_eq!(
                pick_bygone_effigy_intent(Some(BygoneEffigyIntent::Slash)),
                BygoneEffigyIntent::Slash
            );
        }
    }

    #[test]
    fn bygone_effigy_initial_sleep_is_noop() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_bygone_effigy_move(
            &mut cs,
            0,
            0,
            BygoneEffigyIntent::InitialSleep,
        );
        assert_eq!(cs.allies[0].current_hp, hp);
    }

    #[test]
    fn bygone_effigy_wake_applies_ten_strength() {
        let mut cs = ironclad_combat();
        execute_bygone_effigy_move(&mut cs, 0, 0, BygoneEffigyIntent::Wake);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            10
        );
    }

    #[test]
    fn bygone_effigy_slash_deals_thirteen_baseline() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_bygone_effigy_move(&mut cs, 0, 0, BygoneEffigyIntent::Slash);
        assert_eq!(cs.allies[0].current_hp, hp - 13);
    }

    // ---------- SlimedBerserker tests --------------------------------------

    #[test]
    fn slimed_berserker_walks_four_state_chain() {
        assert_eq!(
            pick_slimed_berserker_intent(None),
            SlimedBerserkerIntent::VomitIchor
        );
        assert_eq!(
            pick_slimed_berserker_intent(Some(SlimedBerserkerIntent::VomitIchor)),
            SlimedBerserkerIntent::FuriousPummeling
        );
        assert_eq!(
            pick_slimed_berserker_intent(Some(
                SlimedBerserkerIntent::FuriousPummeling
            )),
            SlimedBerserkerIntent::LeechingHug
        );
        assert_eq!(
            pick_slimed_berserker_intent(Some(SlimedBerserkerIntent::LeechingHug)),
            SlimedBerserkerIntent::Smother
        );
        assert_eq!(
            pick_slimed_berserker_intent(Some(SlimedBerserkerIntent::Smother)),
            SlimedBerserkerIntent::VomitIchor
        );
    }

    #[test]
    fn slimed_berserker_vomit_ichor_adds_ten_slimed() {
        let mut cs = ironclad_combat();
        let before = cs.allies[0]
            .player
            .as_ref()
            .map(|p| p.discard.cards.len())
            .unwrap_or(0);
        execute_slimed_berserker_move(
            &mut cs,
            0,
            0,
            SlimedBerserkerIntent::VomitIchor,
        );
        let after = cs.allies[0]
            .player
            .as_ref()
            .map(|p| p.discard.cards.len())
            .unwrap_or(0);
        assert_eq!(after - before, 10);
        let slimed = cs.allies[0]
            .player
            .as_ref()
            .map(|p| {
                p.discard
                    .cards
                    .iter()
                    .filter(|c| c.id == "Slimed")
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(slimed, 10);
    }

    #[test]
    fn slimed_berserker_furious_pummeling_four_times_four() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_slimed_berserker_move(
            &mut cs,
            0,
            0,
            SlimedBerserkerIntent::FuriousPummeling,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 16);
    }

    #[test]
    fn slimed_berserker_leeching_hug_payload() {
        let mut cs = ironclad_combat();
        execute_slimed_berserker_move(
            &mut cs,
            0,
            0,
            SlimedBerserkerIntent::LeechingHug,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "WeakPower"),
            3
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            3
        );
    }

    #[test]
    fn slimed_berserker_smother_deals_thirty() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_slimed_berserker_move(
            &mut cs,
            0,
            0,
            SlimedBerserkerIntent::Smother,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 30);
    }

    // ---------- GlobeHead tests --------------------------------------------

    #[test]
    fn globe_head_walks_three_state_chain() {
        assert_eq!(pick_globe_head_intent(None), GlobeHeadIntent::ShockingSlap);
        assert_eq!(
            pick_globe_head_intent(Some(GlobeHeadIntent::ShockingSlap)),
            GlobeHeadIntent::ThunderStrike
        );
        assert_eq!(
            pick_globe_head_intent(Some(GlobeHeadIntent::ThunderStrike)),
            GlobeHeadIntent::GalvanicBurst
        );
        assert_eq!(
            pick_globe_head_intent(Some(GlobeHeadIntent::GalvanicBurst)),
            GlobeHeadIntent::ShockingSlap
        );
    }

    #[test]
    fn globe_head_shocking_slap_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_globe_head_move(&mut cs, 0, 0, GlobeHeadIntent::ShockingSlap);
        assert_eq!(cs.allies[0].current_hp, hp - 13);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "FrailPower"),
            2
        );
    }

    #[test]
    fn globe_head_thunder_strike_six_times_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_globe_head_move(&mut cs, 0, 0, GlobeHeadIntent::ThunderStrike);
        assert_eq!(cs.allies[0].current_hp, hp - 18);
    }

    #[test]
    fn globe_head_galvanic_burst_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_globe_head_move(&mut cs, 0, 0, GlobeHeadIntent::GalvanicBurst);
        assert_eq!(cs.allies[0].current_hp, hp - 16);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    // ---------- SpinyToad tests --------------------------------------------

    #[test]
    fn spiny_toad_walks_three_state_chain() {
        assert_eq!(pick_spiny_toad_intent(None), SpinyToadIntent::Spikes);
        assert_eq!(
            pick_spiny_toad_intent(Some(SpinyToadIntent::Spikes)),
            SpinyToadIntent::Explosion
        );
        assert_eq!(
            pick_spiny_toad_intent(Some(SpinyToadIntent::Explosion)),
            SpinyToadIntent::Lash
        );
        assert_eq!(
            pick_spiny_toad_intent(Some(SpinyToadIntent::Lash)),
            SpinyToadIntent::Spikes
        );
    }

    #[test]
    fn spiny_toad_spikes_applies_five_thorns() {
        let mut cs = ironclad_combat();
        execute_spiny_toad_move(&mut cs, 0, 0, SpinyToadIntent::Spikes);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ThornsPower"),
            5
        );
    }

    #[test]
    fn spiny_toad_explosion_deals_23_and_strips_thorns() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "ThornsPower", 5);
        let hp = cs.allies[0].current_hp;
        execute_spiny_toad_move(&mut cs, 0, 0, SpinyToadIntent::Explosion);
        assert_eq!(cs.allies[0].current_hp, hp - 23);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ThornsPower"),
            0
        );
    }

    #[test]
    fn spiny_toad_lash_deals_seventeen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_spiny_toad_move(&mut cs, 0, 0, SpinyToadIntent::Lash);
        assert_eq!(cs.allies[0].current_hp, hp - 17);
    }

    // ---------- Vantom tests -----------------------------------------------

    #[test]
    fn vantom_walks_four_state_chain() {
        assert_eq!(pick_vantom_intent(None), VantomIntent::InkBlot);
        assert_eq!(
            pick_vantom_intent(Some(VantomIntent::InkBlot)),
            VantomIntent::InkyLance
        );
        assert_eq!(
            pick_vantom_intent(Some(VantomIntent::InkyLance)),
            VantomIntent::Dismember
        );
        assert_eq!(
            pick_vantom_intent(Some(VantomIntent::Dismember)),
            VantomIntent::Prepare
        );
        assert_eq!(
            pick_vantom_intent(Some(VantomIntent::Prepare)),
            VantomIntent::InkBlot
        );
    }

    #[test]
    fn vantom_ink_blot_deals_seven() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_vantom_move(&mut cs, 0, 0, VantomIntent::InkBlot);
        assert_eq!(cs.allies[0].current_hp, hp - 7);
    }

    #[test]
    fn vantom_inky_lance_six_times_two() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_vantom_move(&mut cs, 0, 0, VantomIntent::InkyLance);
        assert_eq!(cs.allies[0].current_hp, hp - 12);
    }

    #[test]
    fn vantom_dismember_27_damage_and_three_wounds() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        let discard_before = cs.allies[0]
            .player
            .as_ref()
            .map(|p| p.discard.cards.len())
            .unwrap_or(0);
        execute_vantom_move(&mut cs, 0, 0, VantomIntent::Dismember);
        assert_eq!(cs.allies[0].current_hp, hp - 27);
        let discard_after = cs.allies[0]
            .player
            .as_ref()
            .map(|p| p.discard.cards.len())
            .unwrap_or(0);
        assert_eq!(discard_after - discard_before, 3);
        let wounds = cs.allies[0]
            .player
            .as_ref()
            .map(|p| {
                p.discard
                    .cards
                    .iter()
                    .filter(|c| c.id == "Wound")
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(wounds, 3);
    }

    #[test]
    fn vantom_prepare_grants_two_strength() {
        let mut cs = ironclad_combat();
        execute_vantom_move(&mut cs, 0, 0, VantomIntent::Prepare);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    // ---------- SoulNexus tests --------------------------------------------

    #[test]
    fn soul_nexus_first_turn_is_soul_burn() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_soul_nexus_intent(&mut rng, None),
            SoulNexusIntent::SoulBurn
        );
    }

    #[test]
    fn soul_nexus_cannot_repeat() {
        let mut rng = Rng::new(42, 0);
        for _ in 0..40 {
            for &start in &[
                SoulNexusIntent::SoulBurn,
                SoulNexusIntent::Maelstrom,
                SoulNexusIntent::DrainLife,
            ] {
                let next = pick_soul_nexus_intent(&mut rng, Some(start));
                assert_ne!(next, start);
            }
        }
    }

    #[test]
    fn soul_nexus_soul_burn_deals_twentynine() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_soul_nexus_move(&mut cs, 0, 0, SoulNexusIntent::SoulBurn);
        assert_eq!(cs.allies[0].current_hp, hp - 29);
    }

    #[test]
    fn soul_nexus_maelstrom_six_times_four() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_soul_nexus_move(&mut cs, 0, 0, SoulNexusIntent::Maelstrom);
        assert_eq!(cs.allies[0].current_hp, hp - 24);
    }

    #[test]
    fn soul_nexus_drain_life_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_soul_nexus_move(&mut cs, 0, 0, SoulNexusIntent::DrainLife);
        assert_eq!(cs.allies[0].current_hp, hp - 18);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "VulnerablePower"),
            2
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "WeakPower"),
            2
        );
    }

    // ---------- DevotedSculptor + Exoskeleton tests ------------------------

    #[test]
    fn devoted_sculptor_init_incants_then_savage_forever() {
        assert_eq!(
            pick_devoted_sculptor_intent(None),
            DevotedSculptorIntent::ForbiddenIncantation
        );
        assert_eq!(
            pick_devoted_sculptor_intent(Some(
                DevotedSculptorIntent::ForbiddenIncantation
            )),
            DevotedSculptorIntent::Savage
        );
        for _ in 0..5 {
            assert_eq!(
                pick_devoted_sculptor_intent(Some(DevotedSculptorIntent::Savage)),
                DevotedSculptorIntent::Savage
            );
        }
    }

    #[test]
    fn devoted_sculptor_forbidden_incantation_applies_ritual_nine() {
        let mut cs = ironclad_combat();
        execute_devoted_sculptor_move(
            &mut cs,
            0,
            0,
            DevotedSculptorIntent::ForbiddenIncantation,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "RitualPower"),
            9
        );
    }

    #[test]
    fn devoted_sculptor_savage_deals_twelve() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_devoted_sculptor_move(
            &mut cs,
            0,
            0,
            DevotedSculptorIntent::Savage,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 12);
    }

    #[test]
    fn exoskeleton_init_by_slot() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_exoskeleton_intent(&mut rng, None, 1),
            ExoskeletonIntent::Skitter
        );
        assert_eq!(
            pick_exoskeleton_intent(&mut rng, None, 2),
            ExoskeletonIntent::Mandibles
        );
        assert_eq!(
            pick_exoskeleton_intent(&mut rng, None, 3),
            ExoskeletonIntent::Enrage
        );
        // Slot 4 routes to RandomBranch — must be one of Skitter | Mandibles.
        let s4 = pick_exoskeleton_intent(&mut rng, None, 4);
        assert!(matches!(
            s4,
            ExoskeletonIntent::Skitter | ExoskeletonIntent::Mandibles
        ));
    }

    #[test]
    fn exoskeleton_mandibles_always_to_enrage() {
        let mut rng = Rng::new(1, 0);
        for _ in 0..10 {
            assert_eq!(
                pick_exoskeleton_intent(
                    &mut rng,
                    Some(ExoskeletonIntent::Mandibles),
                    1
                ),
                ExoskeletonIntent::Enrage
            );
        }
    }

    #[test]
    fn exoskeleton_skitter_cannot_repeat_into_skitter() {
        // After Skitter → RandomBranch with CannotRepeat: must yield
        // Mandibles every time.
        let mut rng = Rng::new(42, 0);
        for _ in 0..20 {
            assert_eq!(
                pick_exoskeleton_intent(
                    &mut rng,
                    Some(ExoskeletonIntent::Skitter),
                    1
                ),
                ExoskeletonIntent::Mandibles
            );
        }
    }

    #[test]
    fn exoskeleton_enrage_random_branch_picks_skitter_or_mandibles() {
        let mut rng = Rng::new(1, 0);
        for _ in 0..20 {
            let next = pick_exoskeleton_intent(
                &mut rng,
                Some(ExoskeletonIntent::Enrage),
                1,
            );
            assert!(matches!(
                next,
                ExoskeletonIntent::Skitter | ExoskeletonIntent::Mandibles
            ));
        }
    }

    #[test]
    fn exoskeleton_spawn_applies_hard_to_kill_nine() {
        let mut cs = ironclad_combat();
        exoskeleton_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "HardToKillPower"),
            9
        );
    }

    #[test]
    fn exoskeleton_skitter_3x1() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_exoskeleton_move(&mut cs, 0, 0, ExoskeletonIntent::Skitter);
        assert_eq!(cs.allies[0].current_hp, hp - 3);
    }

    #[test]
    fn exoskeleton_mandibles_eight() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_exoskeleton_move(&mut cs, 0, 0, ExoskeletonIntent::Mandibles);
        assert_eq!(cs.allies[0].current_hp, hp - 8);
    }

    #[test]
    fn exoskeleton_enrage_strengths_two() {
        let mut cs = ironclad_combat();
        execute_exoskeleton_move(&mut cs, 0, 0, ExoskeletonIntent::Enrage);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    #[test]
    fn hard_to_kill_caps_per_hit_damage() {
        // Enemy with HardToKill(9) takes a 50-damage hit. After cap: 9.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "HardToKillPower", 9);
        let hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            50,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 9);
    }

    #[test]
    fn hard_to_kill_does_not_increase_small_hits() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "HardToKillPower", 9);
        let hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            3,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 3);
    }

    #[test]
    fn thorns_does_not_recurse_when_both_sides_have_it() {
        // Both player and enemy have Thorns. Player attacks enemy.
        // Enemy reflects unpowered → must NOT trigger player's Thorns
        // back at enemy (recursion). Net: player loses Thorns amount,
        // enemy loses raw damage, no further bounces.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "ThornsPower", 5);
        cs.apply_power(CombatSide::Enemy, 0, "ThornsPower", 3);
        let php = cs.allies[0].current_hp;
        let ehp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(cs.allies[0].current_hp, php - 3);
        assert_eq!(cs.enemies[0].current_hp, ehp - 10);
    }

    // ---------- SludgeSpinner + FuzzyWurmCrawler tests --------------------

    #[test]
    fn sludge_spinner_first_turn_is_oil_spray() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_sludge_spinner_intent(&mut rng, None),
            SludgeSpinnerIntent::OilSpray
        );
    }

    #[test]
    fn sludge_spinner_cannot_repeat_after_any_move() {
        let mut rng = Rng::new(42, 0);
        for _ in 0..50 {
            for &start in &[
                SludgeSpinnerIntent::OilSpray,
                SludgeSpinnerIntent::Slam,
                SludgeSpinnerIntent::Rage,
            ] {
                let next = pick_sludge_spinner_intent(&mut rng, Some(start));
                assert_ne!(next, start);
            }
        }
    }

    #[test]
    fn sludge_spinner_oil_spray_payload() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_sludge_spinner_move(&mut cs, 0, 0, SludgeSpinnerIntent::OilSpray);
        assert_eq!(cs.allies[0].current_hp, hp - 8);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "WeakPower"),
            1
        );
    }

    #[test]
    fn sludge_spinner_slam_deals_eleven() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_sludge_spinner_move(&mut cs, 0, 0, SludgeSpinnerIntent::Slam);
        assert_eq!(cs.allies[0].current_hp, hp - 11);
    }

    #[test]
    fn sludge_spinner_rage_dmg_and_strength() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_sludge_spinner_move(&mut cs, 0, 0, SludgeSpinnerIntent::Rage);
        assert_eq!(cs.allies[0].current_hp, hp - 6);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            3
        );
    }

    #[test]
    fn fuzzy_wurm_chain_first_acid_inhale_acid() {
        assert_eq!(
            pick_fuzzy_wurm_crawler_intent(None),
            FuzzyWurmCrawlerIntent::FirstAcidGoop
        );
        assert_eq!(
            pick_fuzzy_wurm_crawler_intent(Some(
                FuzzyWurmCrawlerIntent::FirstAcidGoop
            )),
            FuzzyWurmCrawlerIntent::Inhale
        );
        assert_eq!(
            pick_fuzzy_wurm_crawler_intent(Some(FuzzyWurmCrawlerIntent::Inhale)),
            FuzzyWurmCrawlerIntent::AcidGoop
        );
        assert_eq!(
            pick_fuzzy_wurm_crawler_intent(Some(
                FuzzyWurmCrawlerIntent::AcidGoop
            )),
            FuzzyWurmCrawlerIntent::FirstAcidGoop
        );
    }

    #[test]
    fn fuzzy_wurm_acid_goop_variants_share_damage() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_fuzzy_wurm_crawler_move(
            &mut cs,
            0,
            0,
            FuzzyWurmCrawlerIntent::FirstAcidGoop,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 4);
        let hp2 = cs.allies[0].current_hp;
        execute_fuzzy_wurm_crawler_move(
            &mut cs,
            0,
            0,
            FuzzyWurmCrawlerIntent::AcidGoop,
        );
        assert_eq!(cs.allies[0].current_hp, hp2 - 4);
    }

    #[test]
    fn fuzzy_wurm_inhale_gains_seven_strength() {
        let mut cs = ironclad_combat();
        execute_fuzzy_wurm_crawler_move(
            &mut cs,
            0,
            0,
            FuzzyWurmCrawlerIntent::Inhale,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            7
        );
    }

    // ---------- BowlbugRock + ImbalancedPower tests -----------------------

    #[test]
    fn bowlbug_rock_first_turn_is_headbutt() {
        assert_eq!(
            pick_bowlbug_rock_intent(None, false),
            BowlbugRockIntent::Headbutt
        );
    }

    #[test]
    fn bowlbug_rock_balanced_keeps_headbutting() {
        assert_eq!(
            pick_bowlbug_rock_intent(Some(BowlbugRockIntent::Headbutt), false),
            BowlbugRockIntent::Headbutt
        );
    }

    #[test]
    fn bowlbug_rock_off_balance_dizzies() {
        assert_eq!(
            pick_bowlbug_rock_intent(Some(BowlbugRockIntent::Headbutt), true),
            BowlbugRockIntent::Dizzy
        );
        // Dizzy → Headbutt always.
        assert_eq!(
            pick_bowlbug_rock_intent(Some(BowlbugRockIntent::Dizzy), true),
            BowlbugRockIntent::Headbutt
        );
    }

    #[test]
    fn bowlbug_rock_headbutt_deals_fifteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_bowlbug_rock_move(&mut cs, 0, 0, BowlbugRockIntent::Headbutt);
        assert_eq!(cs.allies[0].current_hp, hp - 15);
    }

    #[test]
    fn bowlbug_rock_dizzy_clears_off_balance_flag() {
        let mut cs = ironclad_combat();
        // Pre-set the flag.
        cs.enemies[0]
            .monster
            .as_mut()
            .unwrap()
            .set_flag("is_off_balance", true);
        execute_bowlbug_rock_move(&mut cs, 0, 0, BowlbugRockIntent::Dizzy);
        assert!(!cs.enemies[0].monster.as_ref().unwrap().flag("is_off_balance"));
    }

    #[test]
    fn bowlbug_rock_spawn_applies_imbalanced() {
        let mut cs = ironclad_combat();
        bowlbug_rock_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ImbalancedPower"),
            1
        );
    }

    #[test]
    fn imbalanced_sets_off_balance_when_fully_blocked() {
        // Set up: rock attacks player who has full block coverage.
        let mut cs = ironclad_combat();
        bowlbug_rock_spawn(&mut cs, 0);
        cs.allies[0].block = 100;
        cs.deal_damage(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            15,
            ValueProp::MOVE,
        );
        // Damage fully absorbed → off_balance flipped on.
        assert!(
            cs.enemies[0].monster.as_ref().unwrap().flag("is_off_balance"),
            "expected off_balance after fully-blocked attack"
        );
    }

    #[test]
    fn imbalanced_does_not_trigger_when_damage_lands() {
        let mut cs = ironclad_combat();
        bowlbug_rock_spawn(&mut cs, 0);
        // No block — damage lands.
        cs.deal_damage(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            15,
            ValueProp::MOVE,
        );
        assert!(
            !cs.enemies[0].monster.as_ref().unwrap().flag("is_off_balance"),
            "off_balance should not be set when damage gets through"
        );
    }

    // ---------- MechaKnight tests -----------------------------------------

    #[test]
    fn mecha_knight_chain_charge_flame_windup_cleave_then_loops() {
        assert_eq!(
            pick_mecha_knight_intent(None),
            MechaKnightIntent::Charge
        );
        assert_eq!(
            pick_mecha_knight_intent(Some(MechaKnightIntent::Charge)),
            MechaKnightIntent::Flamethrower
        );
        assert_eq!(
            pick_mecha_knight_intent(Some(MechaKnightIntent::Flamethrower)),
            MechaKnightIntent::Windup
        );
        assert_eq!(
            pick_mecha_knight_intent(Some(MechaKnightIntent::Windup)),
            MechaKnightIntent::HeavyCleave
        );
        // HeavyCleave loops back to Flamethrower (not Charge — Charge
        // fires once only).
        assert_eq!(
            pick_mecha_knight_intent(Some(MechaKnightIntent::HeavyCleave)),
            MechaKnightIntent::Flamethrower
        );
    }

    #[test]
    fn mecha_knight_charge_deals_twenty_five() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_mecha_knight_move(&mut cs, 0, 0, MechaKnightIntent::Charge);
        assert_eq!(cs.allies[0].current_hp, hp - 25);
    }

    #[test]
    fn mecha_knight_flamethrower_adds_four_burns() {
        let mut cs = ironclad_combat();
        execute_mecha_knight_move(
            &mut cs,
            0,
            0,
            MechaKnightIntent::Flamethrower,
        );
        let ps = cs.allies[0].player.as_ref().unwrap();
        let burns = ps.hand.cards.iter().filter(|c| c.id == "Burn").count();
        assert_eq!(burns, 4);
    }

    #[test]
    fn mecha_knight_windup_gains_fifteen_block_five_strength() {
        let mut cs = ironclad_combat();
        execute_mecha_knight_move(&mut cs, 0, 0, MechaKnightIntent::Windup);
        assert_eq!(cs.enemies[0].block, 15);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            5
        );
    }

    #[test]
    fn mecha_knight_heavy_cleave_deals_thirty_five() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_mecha_knight_move(&mut cs, 0, 0, MechaKnightIntent::HeavyCleave);
        assert_eq!(cs.allies[0].current_hp, hp - 35);
    }

    #[test]
    fn mecha_knight_spawn_applies_artifact_three() {
        let mut cs = ironclad_combat();
        mecha_knight_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ArtifactPower"),
            3
        );
    }

    // ---------- Entomancer tests ------------------------------------------

    #[test]
    fn entomancer_first_turn_is_bees() {
        assert_eq!(pick_entomancer_intent(None), EntomancerIntent::Bees);
    }

    #[test]
    fn entomancer_cycle_bees_spear_spit() {
        assert_eq!(
            pick_entomancer_intent(Some(EntomancerIntent::Bees)),
            EntomancerIntent::Spear
        );
        assert_eq!(
            pick_entomancer_intent(Some(EntomancerIntent::Spear)),
            EntomancerIntent::Spit
        );
        assert_eq!(
            pick_entomancer_intent(Some(EntomancerIntent::Spit)),
            EntomancerIntent::Bees
        );
    }

    #[test]
    fn entomancer_bees_hits_seven_times_for_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_entomancer_move(&mut cs, 0, 0, EntomancerIntent::Bees);
        assert_eq!(cs.allies[0].current_hp, hp - 21);
    }

    #[test]
    fn entomancer_spear_deals_eighteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_entomancer_move(&mut cs, 0, 0, EntomancerIntent::Spear);
        assert_eq!(cs.allies[0].current_hp, hp - 18);
    }

    #[test]
    fn entomancer_spawn_applies_personal_hive_one() {
        let mut cs = ironclad_combat();
        entomancer_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PersonalHivePower"),
            1
        );
    }

    #[test]
    fn entomancer_spit_pre_cap_grows_hive_and_strength() {
        let mut cs = ironclad_combat();
        entomancer_spawn(&mut cs, 0);
        // PersonalHive = 1, < cap 3 → branch grows both.
        execute_entomancer_move(&mut cs, 0, 0, EntomancerIntent::Spit);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PersonalHivePower"),
            2
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            1
        );
    }

    #[test]
    fn entomancer_spit_post_cap_only_grows_strength_by_two() {
        let mut cs = ironclad_combat();
        // Pre-load PersonalHive to cap.
        cs.apply_power(CombatSide::Enemy, 0, "PersonalHivePower", 3);
        execute_entomancer_move(&mut cs, 0, 0, EntomancerIntent::Spit);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "PersonalHivePower"),
            3
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    // ---------- LivingShield + RampartPower tests -------------------------

    fn add_enemy(cs: &mut CombatState, model_id: &str, hp: i32) {
        // Helper to inject a fake enemy for ally-count tests. Mirrors
        // Creature::from_monster_spawn minus the data-table lookup.
        cs.enemies.push(Creature {
            kind: CreatureKind::Monster,
            model_id: model_id.to_string(),
            slot: String::new(),
            current_hp: hp,
            max_hp: hp,
            block: 0,
            powers: Vec::new(),
            afflictions: Vec::new(),
            player: None,
            monster: None,
        });
    }

    #[test]
    fn living_shield_first_turn_is_shield_slam() {
        assert_eq!(
            pick_living_shield_intent(None, false),
            LivingShieldIntent::ShieldSlam
        );
        assert_eq!(
            pick_living_shield_intent(None, true),
            LivingShieldIntent::ShieldSlam
        );
    }

    #[test]
    fn living_shield_with_allies_stays_shield_slam() {
        assert_eq!(
            pick_living_shield_intent(
                Some(LivingShieldIntent::ShieldSlam),
                true,
            ),
            LivingShieldIntent::ShieldSlam
        );
    }

    #[test]
    fn living_shield_alone_smashes_forever() {
        assert_eq!(
            pick_living_shield_intent(
                Some(LivingShieldIntent::ShieldSlam),
                false,
            ),
            LivingShieldIntent::Smash
        );
        // Smash self-loops.
        assert_eq!(
            pick_living_shield_intent(Some(LivingShieldIntent::Smash), true),
            LivingShieldIntent::Smash
        );
        assert_eq!(
            pick_living_shield_intent(Some(LivingShieldIntent::Smash), false),
            LivingShieldIntent::Smash
        );
    }

    #[test]
    fn living_shield_smash_damage_and_strength_gain() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_living_shield_move(&mut cs, 0, 0, LivingShieldIntent::Smash);
        assert_eq!(cs.allies[0].current_hp, hp - 16);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            3
        );
    }

    #[test]
    fn living_shield_spawn_applies_rampart_25() {
        let mut cs = ironclad_combat();
        living_shield_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "RampartPower"),
            25
        );
    }

    #[test]
    fn rampart_grants_block_to_turret_operator_at_player_turn_start() {
        let mut cs = CombatState::empty();
        // Player + 2 enemies (LivingShield in slot 0, TurretOperator
        // in slot 1).
        cs.allies.push(Creature {
            kind: CreatureKind::Player,
            model_id: "Ironclad".to_string(),
            slot: String::new(),
            current_hp: 80,
            max_hp: 80,
            block: 0,
            powers: Vec::new(),
            afflictions: Vec::new(),
            player: None,
            monster: None,
        });
        add_enemy(&mut cs, "LivingShield", 55);
        add_enemy(&mut cs, "TurretOperator", 41);
        cs.apply_power(CombatSide::Enemy, 0, "RampartPower", 25);
        cs.current_side = CombatSide::Enemy;
        cs.begin_turn(CombatSide::Player);
        assert_eq!(cs.enemies[1].block, 25);
        // LivingShield itself doesn't get block.
        assert_eq!(cs.enemies[0].block, 0);
    }

    #[test]
    fn rampart_does_not_grant_block_to_non_turret_teammates() {
        let mut cs = CombatState::empty();
        cs.allies.push(Creature {
            kind: CreatureKind::Player,
            model_id: "Ironclad".to_string(),
            slot: String::new(),
            current_hp: 80,
            max_hp: 80,
            block: 0,
            powers: Vec::new(),
            afflictions: Vec::new(),
            player: None,
            monster: None,
        });
        add_enemy(&mut cs, "LivingShield", 55);
        add_enemy(&mut cs, "Axebot", 42); // not a TurretOperator
        cs.apply_power(CombatSide::Enemy, 0, "RampartPower", 25);
        cs.current_side = CombatSide::Enemy;
        cs.begin_turn(CombatSide::Player);
        assert_eq!(cs.enemies[1].block, 0);
    }

    #[test]
    fn rampart_only_fires_on_player_turn_start() {
        let mut cs = CombatState::empty();
        cs.allies.push(Creature {
            kind: CreatureKind::Player,
            model_id: "Ironclad".to_string(),
            slot: String::new(),
            current_hp: 80,
            max_hp: 80,
            block: 0,
            powers: Vec::new(),
            afflictions: Vec::new(),
            player: None,
            monster: None,
        });
        add_enemy(&mut cs, "LivingShield", 55);
        add_enemy(&mut cs, "TurretOperator", 41);
        cs.apply_power(CombatSide::Enemy, 0, "RampartPower", 25);
        cs.current_side = CombatSide::Player;
        cs.begin_turn(CombatSide::Enemy);
        assert_eq!(cs.enemies[1].block, 0);
    }

    // ---------- ShrinkerBeetle + ShrinkPower tests ------------------------

    #[test]
    fn shrinker_beetle_first_turn_is_shrinker() {
        assert_eq!(
            pick_shrinker_beetle_intent(None),
            ShrinkerBeetleIntent::Shrinker
        );
    }

    #[test]
    fn shrinker_beetle_after_shrinker_alternates_chomp_stomp() {
        assert_eq!(
            pick_shrinker_beetle_intent(Some(ShrinkerBeetleIntent::Shrinker)),
            ShrinkerBeetleIntent::Chomp
        );
        assert_eq!(
            pick_shrinker_beetle_intent(Some(ShrinkerBeetleIntent::Chomp)),
            ShrinkerBeetleIntent::Stomp
        );
        assert_eq!(
            pick_shrinker_beetle_intent(Some(ShrinkerBeetleIntent::Stomp)),
            ShrinkerBeetleIntent::Chomp
        );
    }

    #[test]
    fn shrinker_beetle_shrinker_applies_negative_shrink() {
        let mut cs = ironclad_combat();
        execute_shrinker_beetle_move(
            &mut cs,
            0,
            0,
            ShrinkerBeetleIntent::Shrinker,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "ShrinkPower"),
            -1
        );
    }

    #[test]
    fn shrinker_beetle_chomp_deals_seven() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_shrinker_beetle_move(&mut cs, 0, 0, ShrinkerBeetleIntent::Chomp);
        assert_eq!(cs.allies[0].current_hp, hp - 7);
    }

    #[test]
    fn shrinker_beetle_stomp_deals_thirteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_shrinker_beetle_move(&mut cs, 0, 0, ShrinkerBeetleIntent::Stomp);
        assert_eq!(cs.allies[0].current_hp, hp - 13);
    }

    #[test]
    fn shrink_reduces_owner_powered_damage_by_thirty_percent() {
        // Apply Shrink to player; player deals 10 damage to enemy →
        // 10 * 0.70 = 7.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "ShrinkPower", -1);
        let hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 7);
    }

    #[test]
    fn shrink_does_not_reduce_unpowered_damage() {
        // Unpowered damage (e.g. Bloodletting self-damage) is
        // unaffected by Shrink.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Player, 0, "ShrinkPower", -1);
        let hp = cs.enemies[0].current_hp;
        cs.deal_damage(
            (CombatSide::Player, 0),
            (CombatSide::Enemy, 0),
            10,
            ValueProp::UNPOWERED.with(ValueProp::MOVE),
        );
        assert_eq!(cs.enemies[0].current_hp, hp - 10);
    }

    // ---------- Byrdonis + TerritorialPower tests -------------------------

    #[test]
    fn byrdonis_first_turn_is_swoop() {
        assert_eq!(pick_byrdonis_intent(None), ByrdonisIntent::Swoop);
    }

    #[test]
    fn byrdonis_alternates() {
        assert_eq!(
            pick_byrdonis_intent(Some(ByrdonisIntent::Swoop)),
            ByrdonisIntent::Peck
        );
        assert_eq!(
            pick_byrdonis_intent(Some(ByrdonisIntent::Peck)),
            ByrdonisIntent::Swoop
        );
    }

    #[test]
    fn byrdonis_swoop_deals_seventeen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_byrdonis_move(&mut cs, 0, 0, ByrdonisIntent::Swoop);
        assert_eq!(cs.allies[0].current_hp, hp - 17);
    }

    #[test]
    fn byrdonis_peck_hits_three_times_for_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_byrdonis_move(&mut cs, 0, 0, ByrdonisIntent::Peck);
        assert_eq!(cs.allies[0].current_hp, hp - 9);
    }

    #[test]
    fn byrdonis_spawn_applies_territorial_one() {
        let mut cs = ironclad_combat();
        byrdonis_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "TerritorialPower"),
            1
        );
    }

    #[test]
    fn territorial_grants_strength_on_owner_side_turn_end() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "TerritorialPower", 1);
        cs.current_side = CombatSide::Enemy;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            1
        );
    }

    #[test]
    fn territorial_does_not_fire_on_other_side_turn_end() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "TerritorialPower", 1);
        cs.current_side = CombatSide::Player;
        cs.end_turn();
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            0
        );
    }

    #[test]
    fn territorial_compounds_across_turns() {
        // Three enemy turn ends → Strength ramps to 3.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "TerritorialPower", 1);
        for _ in 0..3 {
            cs.current_side = CombatSide::Enemy;
            cs.end_turn();
        }
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            3
        );
    }

    // ---------- Chomper tests ---------------------------------------------

    #[test]
    fn chomper_default_init_is_clamp() {
        assert_eq!(pick_chomper_intent(None, false), ChomperIntent::Clamp);
    }

    #[test]
    fn chomper_scream_first_init_is_screech() {
        assert_eq!(pick_chomper_intent(None, true), ChomperIntent::Screech);
    }

    #[test]
    fn chomper_alternates() {
        assert_eq!(
            pick_chomper_intent(Some(ChomperIntent::Clamp), false),
            ChomperIntent::Screech
        );
        assert_eq!(
            pick_chomper_intent(Some(ChomperIntent::Screech), false),
            ChomperIntent::Clamp
        );
    }

    #[test]
    fn chomper_spawn_applies_artifact_two() {
        let mut cs = ironclad_combat();
        chomper_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "ArtifactPower"),
            2
        );
    }

    #[test]
    fn chomper_clamp_hits_twice_for_eight() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_chomper_move(&mut cs, 0, 0, ChomperIntent::Clamp);
        assert_eq!(cs.allies[0].current_hp, hp - 16);
    }

    #[test]
    fn chomper_screech_adds_three_dazed() {
        let mut cs = ironclad_combat();
        execute_chomper_move(&mut cs, 0, 0, ChomperIntent::Screech);
        let ps = cs.allies[0].player.as_ref().unwrap();
        let dazed = ps.discard.cards.iter().filter(|c| c.id == "Dazed").count();
        assert_eq!(dazed, 3);
    }

    // ---------- TurretOperator tests --------------------------------------

    #[test]
    fn turret_operator_chain_unload1_unload2_reload() {
        assert_eq!(
            pick_turret_operator_intent(None),
            TurretOperatorIntent::Unload1
        );
        assert_eq!(
            pick_turret_operator_intent(Some(TurretOperatorIntent::Unload1)),
            TurretOperatorIntent::Unload2
        );
        assert_eq!(
            pick_turret_operator_intent(Some(TurretOperatorIntent::Unload2)),
            TurretOperatorIntent::Reload
        );
        assert_eq!(
            pick_turret_operator_intent(Some(TurretOperatorIntent::Reload)),
            TurretOperatorIntent::Unload1
        );
    }

    #[test]
    fn turret_operator_unload_hits_five_times_for_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_turret_operator_move(&mut cs, 0, 0, TurretOperatorIntent::Unload1);
        assert_eq!(cs.allies[0].current_hp, hp - 15);
    }

    #[test]
    fn turret_operator_unload1_unload2_share_payload() {
        let mut cs = ironclad_combat();
        let hp1_before = cs.enemies[0].current_hp;
        let _ = hp1_before;
        let p1 = cs.allies[0].current_hp;
        execute_turret_operator_move(&mut cs, 0, 0, TurretOperatorIntent::Unload2);
        assert_eq!(cs.allies[0].current_hp, p1 - 15);
    }

    #[test]
    fn turret_operator_reload_gains_one_strength() {
        let mut cs = ironclad_combat();
        execute_turret_operator_move(&mut cs, 0, 0, TurretOperatorIntent::Reload);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            1
        );
    }

    // ---------- TwigSlimeM + LeafSlimeM tests -----------------------------

    #[test]
    fn twig_slime_m_first_turn_is_sticky() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_twig_slime_m_intent(&mut rng, None),
            TwigSlimeMIntent::Sticky
        );
    }

    #[test]
    fn twig_slime_m_after_sticky_always_clumps() {
        let mut rng = Rng::new(99, 0);
        for _ in 0..50 {
            assert_eq!(
                pick_twig_slime_m_intent(&mut rng, Some(TwigSlimeMIntent::Sticky)),
                TwigSlimeMIntent::Clump
            );
        }
    }

    #[test]
    fn twig_slime_m_after_clump_67_33_distribution() {
        // Clump weight 2, Sticky weight 1 (default) → 2/3, 1/3.
        let mut rng = Rng::new(1234, 0);
        let mut clump = 0;
        let mut sticky = 0;
        for _ in 0..10_000 {
            match pick_twig_slime_m_intent(&mut rng, Some(TwigSlimeMIntent::Clump)) {
                TwigSlimeMIntent::Clump => clump += 1,
                TwigSlimeMIntent::Sticky => sticky += 1,
            }
        }
        let tol = 200;
        assert!((clump - 6667_i32).abs() < tol, "Clump: {clump}");
        assert!((sticky - 3333_i32).abs() < tol, "Sticky: {sticky}");
    }

    #[test]
    fn twig_slime_m_clump_deals_eleven() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_twig_slime_m_move(&mut cs, 0, 0, TwigSlimeMIntent::Clump);
        assert_eq!(cs.allies[0].current_hp, hp - 11);
    }

    #[test]
    fn twig_slime_m_sticky_adds_one_slimed() {
        let mut cs = ironclad_combat();
        execute_twig_slime_m_move(&mut cs, 0, 0, TwigSlimeMIntent::Sticky);
        let ps = cs.allies[0].player.as_ref().unwrap();
        let count = ps.discard.cards.iter().filter(|c| c.id == "Slimed").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn leaf_slime_m_alternates_starting_with_sticky() {
        assert_eq!(pick_leaf_slime_m_intent(None), LeafSlimeMIntent::Sticky);
        assert_eq!(
            pick_leaf_slime_m_intent(Some(LeafSlimeMIntent::Sticky)),
            LeafSlimeMIntent::Clump
        );
        assert_eq!(
            pick_leaf_slime_m_intent(Some(LeafSlimeMIntent::Clump)),
            LeafSlimeMIntent::Sticky
        );
    }

    #[test]
    fn leaf_slime_m_clump_deals_eight() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_leaf_slime_m_move(&mut cs, 0, 0, LeafSlimeMIntent::Clump);
        assert_eq!(cs.allies[0].current_hp, hp - 8);
    }

    #[test]
    fn leaf_slime_m_sticky_adds_two_slimed() {
        let mut cs = ironclad_combat();
        execute_leaf_slime_m_move(&mut cs, 0, 0, LeafSlimeMIntent::Sticky);
        let ps = cs.allies[0].player.as_ref().unwrap();
        let count = ps.discard.cards.iter().filter(|c| c.id == "Slimed").count();
        assert_eq!(count, 2);
    }

    // ---------- TwigSlimeS + LeafSlimeS tests -----------------------------

    #[test]
    fn twig_slime_s_always_butts() {
        assert_eq!(
            pick_twig_slime_s_intent(None),
            TwigSlimeSIntent::Butt
        );
        assert_eq!(
            pick_twig_slime_s_intent(Some(TwigSlimeSIntent::Butt)),
            TwigSlimeSIntent::Butt
        );
    }

    #[test]
    fn twig_slime_s_butt_deals_four() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_twig_slime_s_move(&mut cs, 0, 0, TwigSlimeSIntent::Butt);
        assert_eq!(cs.allies[0].current_hp, hp - 4);
    }

    #[test]
    fn leaf_slime_s_alternates_after_init() {
        // After init the cycle is strict alternation (both branches
        // CannotRepeat).
        let mut rng = Rng::new(1, 0);
        let last = pick_leaf_slime_s_intent(&mut rng, None);
        for _ in 0..20 {
            let next = pick_leaf_slime_s_intent(&mut rng, Some(last));
            assert_ne!(next, last);
        }
    }

    #[test]
    fn leaf_slime_s_init_picks_50_50() {
        let mut rng = Rng::new(1234, 0);
        let mut butt = 0;
        let mut goop = 0;
        for _ in 0..10_000 {
            match pick_leaf_slime_s_intent(&mut rng, None) {
                LeafSlimeSIntent::Butt => butt += 1,
                LeafSlimeSIntent::Goop => goop += 1,
            }
        }
        let tol = 200;
        assert!((butt - 5000_i32).abs() < tol, "Butt: {butt}");
        assert!((goop - 5000_i32).abs() < tol, "Goop: {goop}");
    }

    #[test]
    fn leaf_slime_s_butt_deals_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_leaf_slime_s_move(&mut cs, 0, 0, LeafSlimeSIntent::Butt);
        assert_eq!(cs.allies[0].current_hp, hp - 3);
    }

    #[test]
    fn leaf_slime_s_goop_adds_slimed_to_discard() {
        let mut cs = ironclad_combat();
        execute_leaf_slime_s_move(&mut cs, 0, 0, LeafSlimeSIntent::Goop);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert!(ps.discard.cards.iter().any(|c| c.id == "Slimed"));
    }

    // ---------- Seapunk tests ---------------------------------------------

    #[test]
    fn seapunk_first_turn_is_sea_kick() {
        assert_eq!(pick_seapunk_intent(None), SeapunkIntent::SeaKick);
    }

    #[test]
    fn seapunk_cycle_sea_kick_spinning_bubble() {
        assert_eq!(
            pick_seapunk_intent(Some(SeapunkIntent::SeaKick)),
            SeapunkIntent::SpinningKick
        );
        assert_eq!(
            pick_seapunk_intent(Some(SeapunkIntent::SpinningKick)),
            SeapunkIntent::BubbleBurp
        );
        assert_eq!(
            pick_seapunk_intent(Some(SeapunkIntent::BubbleBurp)),
            SeapunkIntent::SeaKick
        );
    }

    #[test]
    fn seapunk_sea_kick_deals_eleven() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_seapunk_move(&mut cs, 0, 0, SeapunkIntent::SeaKick);
        assert_eq!(cs.allies[0].current_hp, hp - 11);
    }

    #[test]
    fn seapunk_spinning_kick_hits_four_times_for_two() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_seapunk_move(&mut cs, 0, 0, SeapunkIntent::SpinningKick);
        assert_eq!(cs.allies[0].current_hp, hp - 8);
    }

    #[test]
    fn seapunk_bubble_burp_gains_block_and_strength() {
        let mut cs = ironclad_combat();
        execute_seapunk_move(&mut cs, 0, 0, SeapunkIntent::BubbleBurp);
        assert_eq!(cs.enemies[0].block, 7);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            1
        );
    }

    // ---------- CorpseSlug tests ------------------------------------------

    #[test]
    fn corpse_slug_starter_zero_is_whip_slap() {
        assert_eq!(
            pick_corpse_slug_intent(None, 0),
            CorpseSlugIntent::WhipSlap
        );
        assert_eq!(
            pick_corpse_slug_intent(None, 3),
            CorpseSlugIntent::WhipSlap
        );
    }

    #[test]
    fn corpse_slug_starter_one_is_glomp() {
        assert_eq!(
            pick_corpse_slug_intent(None, 1),
            CorpseSlugIntent::Glomp
        );
    }

    #[test]
    fn corpse_slug_starter_two_is_goop() {
        assert_eq!(
            pick_corpse_slug_intent(None, 2),
            CorpseSlugIntent::Goop
        );
    }

    #[test]
    fn corpse_slug_cycle_whipslap_glomp_goop() {
        assert_eq!(
            pick_corpse_slug_intent(Some(CorpseSlugIntent::WhipSlap), 0),
            CorpseSlugIntent::Glomp
        );
        assert_eq!(
            pick_corpse_slug_intent(Some(CorpseSlugIntent::Glomp), 0),
            CorpseSlugIntent::Goop
        );
        assert_eq!(
            pick_corpse_slug_intent(Some(CorpseSlugIntent::Goop), 0),
            CorpseSlugIntent::WhipSlap
        );
    }

    #[test]
    fn corpse_slug_whip_slap_hits_player_twice_for_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_corpse_slug_move(&mut cs, 0, 0, CorpseSlugIntent::WhipSlap);
        assert_eq!(cs.allies[0].current_hp, hp - 6);
    }

    #[test]
    fn corpse_slug_glomp_deals_eight() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_corpse_slug_move(&mut cs, 0, 0, CorpseSlugIntent::Glomp);
        assert_eq!(cs.allies[0].current_hp, hp - 8);
    }

    #[test]
    fn corpse_slug_goop_applies_two_frail() {
        let mut cs = ironclad_combat();
        execute_corpse_slug_move(&mut cs, 0, 0, CorpseSlugIntent::Goop);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "FrailPower"),
            2
        );
    }

    #[test]
    fn corpse_slug_spawn_applies_ravenous() {
        let mut cs = ironclad_combat();
        corpse_slug_spawn(&mut cs, 0);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "RavenousPower"),
            4
        );
    }

    // ---------- ScrollOfBiting + PaperCutsPower tests ---------------------

    #[test]
    fn scroll_of_biting_starter_zero_is_chomp() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_scroll_of_biting_intent(&mut rng, None, 0),
            ScrollOfBitingIntent::Chomp
        );
        assert_eq!(
            pick_scroll_of_biting_intent(&mut rng, None, 3),
            ScrollOfBitingIntent::Chomp
        );
    }

    #[test]
    fn scroll_of_biting_starter_one_is_chew() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_scroll_of_biting_intent(&mut rng, None, 1),
            ScrollOfBitingIntent::Chew
        );
    }

    #[test]
    fn scroll_of_biting_starter_two_is_more_teeth() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_scroll_of_biting_intent(&mut rng, None, 2),
            ScrollOfBitingIntent::MoreTeeth
        );
    }

    #[test]
    fn scroll_of_biting_chain_chomp_moreteeth_chew() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_scroll_of_biting_intent(
                &mut rng,
                Some(ScrollOfBitingIntent::Chomp),
                0,
            ),
            ScrollOfBitingIntent::MoreTeeth
        );
        assert_eq!(
            pick_scroll_of_biting_intent(
                &mut rng,
                Some(ScrollOfBitingIntent::MoreTeeth),
                0,
            ),
            ScrollOfBitingIntent::Chew
        );
    }

    #[test]
    fn scroll_of_biting_chew_random_distribution_1_2() {
        // After Chew: Chomp weight 1, Chew weight 2 → 33/67 distribution.
        let mut rng = Rng::new(1234, 0);
        let mut chomp = 0;
        let mut chew = 0;
        for _ in 0..10_000 {
            match pick_scroll_of_biting_intent(
                &mut rng,
                Some(ScrollOfBitingIntent::Chew),
                0,
            ) {
                ScrollOfBitingIntent::Chomp => chomp += 1,
                ScrollOfBitingIntent::Chew => chew += 1,
                ScrollOfBitingIntent::MoreTeeth => {
                    panic!("MoreTeeth shouldn't appear after Chew");
                }
            }
        }
        // 4 SD tolerance.
        let tol = 200;
        assert!((chomp - 3333_i32).abs() < tol, "Chomp count: {chomp}");
        assert!((chew - 6667_i32).abs() < tol, "Chew count: {chew}");
    }

    #[test]
    fn scroll_of_biting_chomp_deals_fourteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_scroll_of_biting_move(&mut cs, 0, 0, ScrollOfBitingIntent::Chomp);
        assert_eq!(cs.allies[0].current_hp, hp - 14);
    }

    #[test]
    fn scroll_of_biting_chew_hits_player_twice_for_five() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_scroll_of_biting_move(&mut cs, 0, 0, ScrollOfBitingIntent::Chew);
        assert_eq!(cs.allies[0].current_hp, hp - 10);
    }

    #[test]
    fn scroll_of_biting_more_teeth_gains_two_strength() {
        let mut cs = ironclad_combat();
        execute_scroll_of_biting_move(
            &mut cs,
            0,
            0,
            ScrollOfBitingIntent::MoreTeeth,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    #[test]
    fn paper_cuts_drops_player_max_hp_on_unblocked_damage() {
        // Enemy 0 holds PaperCutsPower(2). Direct damage to player
        // through 0 block → max_hp drops by 2.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PaperCutsPower", 2);
        let max_before = cs.allies[0].max_hp;
        cs.deal_damage(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            10,
            ValueProp::MOVE,
        );
        assert_eq!(cs.allies[0].max_hp, max_before - 2);
    }

    #[test]
    fn paper_cuts_no_max_hp_loss_when_damage_blocked() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PaperCutsPower", 2);
        cs.allies[0].block = 50;
        let max_before = cs.allies[0].max_hp;
        cs.deal_damage(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            10,
            ValueProp::MOVE,
        );
        // Damage all absorbed by block → no max_hp loss.
        assert_eq!(cs.allies[0].max_hp, max_before);
    }

    #[test]
    fn paper_cuts_only_fires_on_powered_attacks() {
        // Unpowered damage (e.g. Poison tick): PaperCuts doesn't fire.
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "PaperCutsPower", 2);
        let max_before = cs.allies[0].max_hp;
        cs.deal_damage(
            (CombatSide::Enemy, 0),
            (CombatSide::Player, 0),
            10,
            ValueProp::UNPOWERED.with(ValueProp::MOVE),
        );
        assert_eq!(cs.allies[0].max_hp, max_before);
    }

    // ---------- BowlbugSilk intent + move payload tests -------------------

    #[test]
    fn bowlbug_silk_first_turn_is_toxic_spit() {
        assert_eq!(
            pick_bowlbug_silk_intent(None),
            BowlbugSilkIntent::ToxicSpit
        );
    }

    #[test]
    fn bowlbug_silk_alternates_forever() {
        let mut last = pick_bowlbug_silk_intent(None);
        assert_eq!(last, BowlbugSilkIntent::ToxicSpit);
        for _ in 0..10 {
            let next = pick_bowlbug_silk_intent(Some(last));
            assert_ne!(next, last);
            last = next;
        }
    }

    #[test]
    fn bowlbug_silk_trash_hits_twice_for_four() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_bowlbug_silk_move(&mut cs, 0, 0, BowlbugSilkIntent::Trash);
        assert_eq!(cs.allies[0].current_hp, hp - 8);
    }

    #[test]
    fn bowlbug_silk_toxic_spit_applies_one_weak() {
        let mut cs = ironclad_combat();
        execute_bowlbug_silk_move(&mut cs, 0, 0, BowlbugSilkIntent::ToxicSpit);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "WeakPower"),
            1
        );
    }

    // ---------- BowlbugNectar intent + move payload tests -----------------

    #[test]
    fn bowlbug_nectar_first_turn_is_thrash() {
        assert_eq!(
            pick_bowlbug_nectar_intent(None),
            BowlbugNectarIntent::Thrash
        );
    }

    #[test]
    fn bowlbug_nectar_thrash_buff_thrash2_self_loop() {
        // Sequence: None → Thrash → Buff → Thrash2 → Thrash2 → …
        assert_eq!(
            pick_bowlbug_nectar_intent(Some(BowlbugNectarIntent::Thrash)),
            BowlbugNectarIntent::Buff
        );
        assert_eq!(
            pick_bowlbug_nectar_intent(Some(BowlbugNectarIntent::Buff)),
            BowlbugNectarIntent::Thrash2
        );
        // Thrash2 self-loops forever.
        for _ in 0..20 {
            assert_eq!(
                pick_bowlbug_nectar_intent(Some(BowlbugNectarIntent::Thrash2)),
                BowlbugNectarIntent::Thrash2
            );
        }
    }

    #[test]
    fn bowlbug_nectar_thrash2_payload_matches_thrash() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_bowlbug_nectar_move(
            &mut cs,
            0,
            0,
            BowlbugNectarIntent::Thrash2,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 3);
    }

    #[test]
    fn bowlbug_nectar_thrash_deals_three() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_bowlbug_nectar_move(
            &mut cs,
            0,
            0,
            BowlbugNectarIntent::Thrash,
        );
        assert_eq!(cs.allies[0].current_hp, hp - 3);
    }

    #[test]
    fn bowlbug_nectar_buff_gains_fifteen_strength() {
        let mut cs = ironclad_combat();
        execute_bowlbug_nectar_move(
            &mut cs,
            0,
            0,
            BowlbugNectarIntent::Buff,
        );
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            15
        );
    }

    // ---------- BowlbugEgg intent + move payload tests --------------------

    #[test]
    fn bowlbug_egg_always_bites() {
        assert_eq!(pick_bowlbug_egg_intent(None), BowlbugEggIntent::Bite);
        assert_eq!(
            pick_bowlbug_egg_intent(Some(BowlbugEggIntent::Bite)),
            BowlbugEggIntent::Bite
        );
    }

    #[test]
    fn bowlbug_egg_bite_does_damage_and_block() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_bowlbug_egg_move(&mut cs, 0, 0, BowlbugEggIntent::Bite);
        assert_eq!(cs.allies[0].current_hp, hp - 7);
        assert_eq!(cs.enemies[0].block, 7);
    }

    // ---------- FlailKnight intent + move payload tests -------------------

    #[test]
    fn flail_knight_first_turn_is_ram() {
        let mut rng = Rng::new(1, 0);
        assert_eq!(
            pick_flail_knight_intent(&mut rng, None),
            FlailKnightIntent::Ram
        );
    }

    #[test]
    fn flail_knight_subsequent_picks_from_set() {
        let mut rng = Rng::new(42, 0);
        for _ in 0..100 {
            let intent = pick_flail_knight_intent(
                &mut rng,
                Some(FlailKnightIntent::Ram),
            );
            assert!(matches!(
                intent,
                FlailKnightIntent::WarChant
                    | FlailKnightIntent::Flail
                    | FlailKnightIntent::Ram
            ));
        }
    }

    #[test]
    fn flail_knight_war_chant_cannot_repeat() {
        let mut rng = Rng::new(7, 0);
        for _ in 0..200 {
            let intent = pick_flail_knight_intent(
                &mut rng,
                Some(FlailKnightIntent::WarChant),
            );
            assert!(matches!(
                intent,
                FlailKnightIntent::Flail | FlailKnightIntent::Ram
            ));
        }
    }

    #[test]
    fn flail_knight_weighted_distribution_after_non_war_chant() {
        // From Ram: WarChant=1, Flail=2, Ram=2 → 20%/40%/40% of 5.
        let mut rng = Rng::new(1234, 0);
        let mut wc = 0;
        let mut fl = 0;
        let mut rm = 0;
        let trials = 10_000;
        for _ in 0..trials {
            match pick_flail_knight_intent(&mut rng, Some(FlailKnightIntent::Ram)) {
                FlailKnightIntent::WarChant => wc += 1,
                FlailKnightIntent::Flail => fl += 1,
                FlailKnightIntent::Ram => rm += 1,
            }
        }
        // 4 SD tolerance on a binomial.
        let tol = 250;
        assert!((wc - 2000_i32).abs() < tol, "WarChant: {wc}");
        assert!((fl - 4000_i32).abs() < tol, "Flail: {fl}");
        assert!((rm - 4000_i32).abs() < tol, "Ram: {rm}");
    }

    #[test]
    fn flail_knight_war_chant_gains_three_strength() {
        let mut cs = ironclad_combat();
        execute_flail_knight_move(&mut cs, 0, 0, FlailKnightIntent::WarChant);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            3
        );
    }

    #[test]
    fn flail_knight_flail_hits_player_twice_for_nine() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_flail_knight_move(&mut cs, 0, 0, FlailKnightIntent::Flail);
        assert_eq!(cs.allies[0].current_hp, hp - 18);
    }

    #[test]
    fn flail_knight_ram_deals_fifteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_flail_knight_move(&mut cs, 0, 0, FlailKnightIntent::Ram);
        assert_eq!(cs.allies[0].current_hp, hp - 15);
    }

    // ---------- Nibbit intent + move payload tests ------------------------

    #[test]
    fn nibbit_alone_first_turn_is_butt() {
        assert_eq!(
            pick_nibbit_intent(None, true, false),
            NibbitIntent::Butt
        );
        assert_eq!(
            pick_nibbit_intent(None, true, true),
            NibbitIntent::Butt
        );
    }

    #[test]
    fn nibbit_pair_front_first_turn_is_slice() {
        assert_eq!(
            pick_nibbit_intent(None, false, true),
            NibbitIntent::Slice
        );
    }

    #[test]
    fn nibbit_pair_back_first_turn_is_hiss() {
        assert_eq!(
            pick_nibbit_intent(None, false, false),
            NibbitIntent::Hiss
        );
    }

    #[test]
    fn nibbit_cycle_butt_slice_hiss() {
        assert_eq!(
            pick_nibbit_intent(Some(NibbitIntent::Butt), true, false),
            NibbitIntent::Slice
        );
        assert_eq!(
            pick_nibbit_intent(Some(NibbitIntent::Slice), true, false),
            NibbitIntent::Hiss
        );
        assert_eq!(
            pick_nibbit_intent(Some(NibbitIntent::Hiss), true, false),
            NibbitIntent::Butt
        );
    }

    #[test]
    fn nibbit_butt_deals_twelve() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_nibbit_move(&mut cs, 0, 0, NibbitIntent::Butt);
        assert_eq!(cs.allies[0].current_hp, hp - 12);
    }

    #[test]
    fn nibbit_slice_deals_six_and_gains_five_block() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_nibbit_move(&mut cs, 0, 0, NibbitIntent::Slice);
        assert_eq!(cs.allies[0].current_hp, hp - 6);
        assert_eq!(cs.enemies[0].block, 5);
    }

    #[test]
    fn nibbit_hiss_gains_two_strength() {
        let mut cs = ironclad_combat();
        execute_nibbit_move(&mut cs, 0, 0, NibbitIntent::Hiss);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
        );
    }

    // ---------- Myte intent + move payload tests --------------------------

    #[test]
    fn myte_first_turn_first_slot_is_toxic() {
        assert_eq!(pick_myte_intent(None, "first"), MyteIntent::Toxic);
    }

    #[test]
    fn myte_first_turn_second_slot_is_suck() {
        assert_eq!(pick_myte_intent(None, "second"), MyteIntent::Suck);
    }

    #[test]
    fn myte_cycle_toxic_bite_suck_toxic() {
        assert_eq!(
            pick_myte_intent(Some(MyteIntent::Toxic), "first"),
            MyteIntent::Bite
        );
        assert_eq!(
            pick_myte_intent(Some(MyteIntent::Bite), "first"),
            MyteIntent::Suck
        );
        assert_eq!(
            pick_myte_intent(Some(MyteIntent::Suck), "first"),
            MyteIntent::Toxic
        );
    }

    #[test]
    fn myte_toxic_adds_two_toxic_cards_to_player_hand() {
        let mut cs = ironclad_combat();
        let hand_before = cs.allies[0].player.as_ref().unwrap().hand.len();
        execute_myte_move(&mut cs, 0, 0, MyteIntent::Toxic);
        let ps = cs.allies[0].player.as_ref().unwrap();
        assert_eq!(ps.hand.len(), hand_before + 2);
        let toxics = ps.hand.cards.iter().filter(|c| c.id == "Toxic").count();
        assert_eq!(toxics, 2);
    }

    #[test]
    fn myte_bite_deals_thirteen() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_myte_move(&mut cs, 0, 0, MyteIntent::Bite);
        assert_eq!(cs.allies[0].current_hp, hp - 13);
    }

    #[test]
    fn myte_suck_deals_four_and_gains_two_strength() {
        let mut cs = ironclad_combat();
        let hp = cs.allies[0].current_hp;
        execute_myte_move(&mut cs, 0, 0, MyteIntent::Suck);
        assert_eq!(cs.allies[0].current_hp, hp - 4);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "StrengthPower"),
            2
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
