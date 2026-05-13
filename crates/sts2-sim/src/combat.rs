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

use crate::card::{by_id as card_by_id, CardData};
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
                }
            }
            CombatSide::Enemy => {
                for enemy in self.enemies.iter_mut() {
                    enemy.block = 0;
                }
            }
            CombatSide::None => {}
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
}
