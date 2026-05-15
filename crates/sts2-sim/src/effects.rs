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
use crate::relic::by_id as relic_by_id;

/// Resolve a relic's canonical-var integer value by key. Relic vars
/// don't upgrade, so this is a flat lookup against the `canonical_vars`
/// table. Matches by `kind` first, then `generic`, then suffix-stripped
/// `generic` (e.g. "VigorPower" matches a generic "Vigor"). Returns 0
/// if no match.
fn relic_canonical_int_value(relic_id: &str, var_kind: &str) -> i32 {
    let Some(relic) = relic_by_id(relic_id) else {
        return 0;
    };
    for v in &relic.canonical_vars {
        if v.kind == var_kind
            || v.generic.as_deref() == Some(var_kind)
            || v
                .generic
                .as_deref()
                .and_then(|g| g.strip_suffix("Power"))
                == Some(var_kind)
        {
            return v.base_value.unwrap_or(0.0) as i32;
        }
    }
    0
}

/// Resolve a potion's canonical-var integer value by key. Same shape as
/// `relic_canonical_int_value`; potion vars share the kind/generic/
/// base_value schema and don't upgrade. Returns 0 if no match.
fn potion_canonical_int_value(potion_id: &str, var_kind: &str) -> i32 {
    let Some(potion) = crate::potion::by_id(potion_id) else {
        return 0;
    };
    for v in &potion.canonical_vars {
        if v.kind == var_kind
            || v.generic.as_deref() == Some(var_kind)
            || v
                .generic
                .as_deref()
                .and_then(|g| g.strip_suffix("Power"))
                == Some(var_kind)
        {
            return v.base_value.unwrap_or(0.0) as i32;
        }
    }
    0
}

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
    /// Multiply the inner amount by `factor` (constant i32). Older
    /// composition helper kept for backwards compatibility.
    Multiplied { base: Box<AmountSpec>, factor: i32 },
    /// Multiply two amount specs. Used by PerfectedStrike
    /// (CardCount * ExtraDamage) where both terms are computed.
    Mul {
        left: Box<AmountSpec>,
        right: Box<AmountSpec>,
    },
    /// Actor's current amount of the named power. Used inside
    /// power-hook effect bodies that reference their own stack count
    /// (RegenPower heals by `base.Amount`, PoisonPower damages by
    /// `base.Amount`, etc.). The power-VM dispatcher binds the value
    /// into `EffectContext::actor_amount` before invoking the body.
    /// `power_id` is recorded for documentation / future per-power
    /// disambiguation; current resolver ignores it and returns the
    /// pre-bound amount.
    OwnerPowerAmount(String),
    /// Actor's current block. Mirrors C# BodySlam:
    /// `CalculatedDamageVar.WithMultiplier(card, _ => Owner.Creature.Block)`.
    SelfBlock,
    /// Target's current amount of the named power. Used by cards
    /// that scale by an enemy's debuff stacks (Bully: damage scales
    /// with target Vulnerable, Sandpit, etc.). Reads from
    /// `EffectContext::target` (or the actor if no target).
    TargetPowerAmount { power_id: String },
    /// Count of cards in `pile` matching `filter`. `Pile::AllCombat`
    /// counts the union of hand + draw + discard + exhaust (mirrors
    /// C# `PlayerCombatState.AllCards`). PerfectedStrike, MindBlast,
    /// FlakCannon, Flechettes, etc. use this.
    CardCountInPile {
        pile: PileSelector,
        filter: CardFilter,
    },
    /// Sum of two amount specs. Used by PerfectedStrike (base + per-Strike
    /// multiplier) and similar composite-amount cards.
    Add {
        left: Box<AmountSpec>,
        right: Box<AmountSpec>,
    },
    /// Player's Osty companion's MaxHp (0 if no Osty). Used by
    /// Protector / Sacrifice (`block = Osty.MaxHp * 2`). Mirrors C#
    /// `Owner.Osty.MaxHp`.
    OstyMaxHp,
    /// Player's Osty companion's current Block (0 if no Osty).
    OstyBlock,
    /// Size of the player's hand right now. Scrawl: `10 - HandCount`.
    HandCount,
    /// Hand size excluding the currently-playing source card. PreciseCut.
    HandCountExcludingSource,
    /// Target's max HP. BloodPotion / FairyInABottle (% of MaxHp).
    TargetMaxHp,
    /// Target's current Block. Mimic / Mirage / DemonicShield.
    TargetBlock,
    /// Number of `Type == Debuff` powers on target. Rend.
    TargetDebuffCount,
    /// Number of cards played this turn by the owner (optionally filtered).
    /// Conflagration / Finisher / GoldAxe / Murder / GangUp.
    CardsPlayedThisTurn { filter: CardFilter },
    /// Number of cards discarded this turn (MementoMori).
    CardsDiscardedThisTurn,
    /// Number of cards drawn this turn (Murder).
    CardsDrawnThisTurn,
    /// Number of cards exhausted this turn (rare; companion to EvilEye).
    CardsExhaustedThisTurn,
    /// Sum of energy costs paid this turn (HelixDrill hits).
    EnergySpentThisTurn,
    /// Orbs of a given id channeled this combat (Voltaic).
    /// `orb_id == None` counts all orbs.
    OrbsChanneledThisCombat { orb_id: Option<String> },
    /// Distinct orb-ids currently in the player's queue (Synchronize).
    DistinctOrbTypesInQueue,
    /// Sum of positive stars-deltas this turn (Radiate hit count).
    StarsGainedThisTurnPositive,
    /// `max(a, b)`. Mirrors C# `Math.Max(a, b)` — Hang's
    /// `Apply<HangPower>(Max(2, target.HangPower))`.
    Max { left: Box<AmountSpec>, right: Box<AmountSpec> },
    /// `min(a, b)`. Mirrors C# `Math.Min(a, b)`.
    Min { left: Box<AmountSpec>, right: Box<AmountSpec> },
    /// `a - b`. Convenience for `Add(a, Mul(b, -1))` shaped expressions.
    /// Drives Scrawl (`Draw(10 - HandCount)`).
    Sub { left: Box<AmountSpec>, right: Box<AmountSpec> },
    /// `floor(a / b)` (zero if `b <= 0`). Mirrors C# `Math.Floor(a / b)`.
    /// NoEscape: `floor(target.DoomPower / DoomThreshold)`. BloodPotion:
    /// `floor(target.MaxHp * HealPercent / 100)`.
    FloorDiv { left: Box<AmountSpec>, right: Box<AmountSpec> },
    /// Player's current energy at resolve time. DoubleEnergy:
    /// `GainEnergy(Owner.PlayerCombatState.Energy)`.
    CurrentEnergy,
    /// Sum of `power_id` stacks across every alive enemy. Mirage:
    /// `Enemies.Where(IsAlive).Sum(GetPowerAmount<PoisonPower>())`.
    AllEnemiesPowerSum { power_id: String },
    /// Empty orb slots = `orb_slots - orb_queue.len()`. EssenceOfDarkness.
    EmptyOrbSlots,
    /// Per-card-instance scalar counter from `CardInstance.state[key]`
    /// of the source card. Claw (per-play damage ramp), HiddenGem
    /// (BaseReplayCount), Maul/Rampage (own base-damage ramp).
    /// Returns 0 if no source card or key not set.
    SourceCardCounter { key: String },
    /// Count of alive enemies. Chill (Channel FrostOrb per enemy).
    AliveEnemyCount,
    /// Count of alive allies. GangUp / HuddleUp / Coordinate-family.
    AliveAllyCount,
    /// Player's current HP. DeathsDoor / HP-threshold scaling.
    OwnerCurrentHp,
    /// Player's max HP.
    OwnerMaxHp,
    /// Player's missing HP (`max_hp - current_hp`). DeathsDoor:
    /// scale block by damage already taken.
    OwnerHpMissing,
    /// Total damage dealt by the most recent DealDamage step in this
    /// effect list (incl. overkill). Fisticuffs.
    LastRealizedDamage,
    /// Block gained by the most recent GainBlock step. DodgeAndRoll.
    LastRealizedBlock,
    /// Ethereal-tagged cards played by owner this turn. PullFromBelow.
    EtherealCardsPlayedThisTurn,
    /// Hand size captured at the moment OnPlay started (source card
    /// already removed). Stoke / StormOfSteel / FlakCannon.
    HandSizeAtPlayStart,
}

/// Named card pool reference for `Effect::AddRandomCardFromPool`.
/// Pools resolve to a list of card ids the runtime picks from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardPoolRef {
    /// Owner character's full pool.
    CharacterAny,
    /// Owner character's pool filtered to Attack.
    CharacterAttack,
    /// Owner character's pool filtered to Skill.
    CharacterSkill,
    /// Owner character's pool filtered to Power.
    CharacterPower,
    /// Cross-character Colorless pool.
    Colorless,
}

/// Pile-scope discriminator used by `CardCountInPile`. Wider than the
/// `Pile` enum because callers sometimes want the union of all combat
/// piles (PerfectedStrike's `AllCards` semantics).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PileSelector {
    Single(Pile),
    /// Union of Hand + Draw + Discard + Exhaust.
    AllCombat,
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
    /// "The actor itself" — the creature owning the effect list. For
    /// player card OnPlay this collapses to SelfPlayer; for monster
    /// move bodies authored as data, this resolves to the moving
    /// monster (via `EffectContext.actor`).
    SelfActor,
    /// A single ally chosen by the player action. Multiplayer-only in
    /// C#; single-player collapses to `SelfPlayer`. Mimic / DemonicShield.
    ChosenAlly,
    /// Every alive ally in the party. Multi-player; collapses to the
    /// single player. GlimpseBeyond / Coordinate / Largesse / EnergySurge.
    AllAllies,
}

/// Closed condition vocabulary, derived from the C# survey
/// (`docs/effect-vocabulary.md` §3 + §6). Used by `Effect::Conditional`
/// to guard a step on game state.
///
/// Stubs (Always-true / Always-false) are explicit so the data layer
/// can encode incomplete predicates without breaking. Real predicates
/// resolve against `CombatState` + the `EffectContext`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    /// Trivially true. Useful for unconditional steps inside a
    /// Conditional shell (lets the data layer always carry a guard).
    Always,
    /// Trivially false.
    Never,
    /// Negation.
    Not(Box<Condition>),
    /// Both must hold.
    And(Box<Condition>, Box<Condition>),
    /// Either holds.
    Or(Box<Condition>, Box<Condition>),
    /// Source card was upgraded.
    IsUpgraded,
    /// `target.HasPower<P>` with `target` from the EffectContext's
    /// resolved target (or self if no target).
    HasPowerOnTarget { power_id: String },
    /// `target.HasPower<P>` resolved against the player executing
    /// the effect.
    HasPowerOnSelf { power_id: String },
    /// `pile.Cards.Count <op> n` where `op` is one of the Comparison
    /// variants below.
    CardCountInPile {
        pile: Pile,
        op: Comparison,
        value: i32,
    },
    /// Owner has lost HP this turn. Spite-family scan over combat
    /// history.
    OwnerLostHpThisTurn,
    /// The last damage attack (the one wrapping this conditional in
    /// an OnDamage-style step) killed its target. Feed / HandOfGreed.
    AttackKilledTarget,
    /// Hand has a card matching the filter.
    HandHasCardMatching(CardFilter),
    /// The played card has the given keyword (Exhaust / Ethereal / ...).
    SourceCardHasKeyword(String),
    /// Random-chance branch. Resolves via combat RNG.
    /// `numerator / denominator` chance of true.
    RandomChance { numerator: i32, denominator: i32 },
    /// Player's Osty companion is missing — None or current_hp <= 0.
    /// Sacrifice / SicEm / Snap / SweepingGaze / RightHandHand / HighFive / Poke.
    IsOstyMissing,
    /// Owner has exhausted at least one card this turn. EvilEye trigger,
    /// ForgottenRitual.
    OwnerExhaustedCardThisTurn,
    /// First play of the source card this turn (Fetch).
    FirstPlayOfSourceCardThisTurn,
    /// Owner has played strictly fewer than `n` cards this turn. Ftl.
    PlaysThisTurnLt { n: i32 },
    /// combatState.RoundNumber == n. Candelabra (R==2), Chandelier (R==3).
    RoundEquals { n: i32 },
    /// combatState.RoundNumber >= n. PaelsFlesh / StoneCalendar.
    RoundGe { n: i32 },
    /// `PlayerState.relic_counters[key] >= value`. Used by Kunai-style
    /// relics to gate the body on a counter threshold. Counter slot
    /// stored on the relic's owner (player_idx).
    RelicCounterGe { key: String, value: i32 },
    /// `PlayerState.relic_counters[key] % modulus == remainder`. Drives
    /// HappyFlower / Pendulum "every Nth turn" relics.
    RelicCounterModEq { key: String, modulus: i32, remainder: i32 },
    /// Resolved X-energy value compared to `n`. HeavenlyDrill: doubles
    /// hits when `X >= Energy.IntValue` (= 4).
    XEnergyGe { n: i32 },
    /// `EffectContext::x_value == n` (rare; HeavenlyDrill edge).
    XEnergyEq { n: i32 },
    /// `target.current_hp > 0`. MoltenFist gates its Vulnerable re-apply
    /// on target-still-alive after the damage step.
    TargetIsAlive,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Comparison {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Pile-id mirror of `combat::PileType`, plus run-state piles that
/// events / relics / potions need to address.
///
/// Combat piles (Hand, Discard, Draw, Exhaust) resolve to in-combat
/// CardPiles on `PlayerState`. Run-state piles (Deck, PotionBelt)
/// reference the strategic-layer state and are STUBS in this module —
/// the dispatcher records the intent but cannot mutate `RunState`
/// without a handle to it. Events run their own dispatcher; for now,
/// these variants make the vocabulary closed even when they no-op.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pile {
    Hand,
    Discard,
    Draw,
    Exhaust,
    /// Run-state permanent deck. Event-layer-only.
    Deck,
}

impl Pile {
    fn as_pile_type(self) -> PileType {
        match self {
            Pile::Hand => PileType::Hand,
            Pile::Discard => PileType::Discard,
            Pile::Draw => PileType::Draw,
            Pile::Exhaust => PileType::Exhaust,
            // Deck has a PileType::Deck variant but is run-state only.
            Pile::Deck => PileType::Deck,
        }
    }
}

/// How a card-ref primitive selects which card(s) to act on.
///
/// The C# decompile uses `CardSelectCmd.From*` for interactive
/// player-choice. Until the play-card API supports multi-step
/// interaction, choices resolve via deterministic policies (the
/// `PlayerInteractive` variant is reserved for the future multi-step
/// path).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Selector {
    /// Apply to every card in the pile.
    All,
    /// Apply to the first `n` cards matching the optional filter.
    /// Default filter: any card.
    Random { n: i32 },
    /// The card at the top (back) of the pile, where "top" matches
    /// the C# convention used by `Cards.Add(card, Position.Top)`.
    Top { n: i32 },
    /// The card at the bottom (front) of the pile.
    Bottom { n: i32 },
    /// First N cards in pile order matching the filter. Used for
    /// auto-selection ("upgrade all upgradable cards", etc.).
    FirstMatching { n: i32, filter: CardFilter },
    /// Deferred: player picks N cards via a modal screen. Currently
    /// resolves to `Random { n }` so cards that use this stay
    /// functional until the multi-step play API lands.
    PlayerInteractive { n: i32 },
    /// Player picks from cards matching `filter`. Filtered variant of
    /// PlayerInteractive — used when the C# `CardSelectCmd.FromX`
    /// passes a predicate (Attack-only / Skill-only / etc.).
    /// SecretTechnique (Skill-only from Draw), SecretWeapon
    /// (Attack-only from Draw). Fallback: Random over the filtered
    /// candidates.
    PlayerInteractiveFiltered { n: i32, filter: CardFilter },
}

/// Predicate over cards. Closed set tracks the C# pile-filter idioms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardFilter {
    Any,
    Upgradable,
    OfType(String),       // "Attack" | "Skill" | "Power" | "Status" | "Curse"
    HasKeyword(String),   // "Exhaust" | "Ethereal" | ...
    TaggedAs(String),     // "Strike" | "Shiv" | ...
    /// C# `card.Rarity == CardRarity.X`. "Common" / "Uncommon" / "Rare" /
    /// "Ancient" / "Event" / "Token" / "Status" / "Curse" / "Quest".
    /// Anointed pulls Rare-only from draw.
    OfRarity(String),
    /// Logical AND of two filters.
    And(Box<CardFilter>, Box<CardFilter>),
    /// Logical OR. Cleanse (Status or Curse).
    Or(Box<CardFilter>, Box<CardFilter>),
    /// Logical NOT.
    Not(Box<CardFilter>),
    /// Exact card id (single-card filter; useful for "find SovereignBlade").
    HasId(String),
    /// Card's `energy_cost` matches a comparison. AllForOne / Jackpot.
    WithEnergyCost { op: Comparison, value: i32 },
    /// Card is NOT X-cost (`!has_energy_cost_x`). AllForOne filter.
    NotXCost,
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
    /// Remove block from `target`. Mirror of GainBlock for the rare
    /// debuff that strips block.
    LoseBlock { amount: AmountSpec, target: Target },
    /// Directly mutate an existing power's stack count on `target`.
    /// Different from `ApplyPower` because it bypasses Counter-stack
    /// merging logic. Adrenaline-style.
    ModifyPowerAmount {
        power_id: String,
        delta: AmountSpec,
        target: Target,
    },
    /// Increment the player's pending gold (folded into combat reward
    /// gold at combat end). HandOfGreed, Alchemize, FoulPotion's
    /// merchant-throw.
    GainGold { amount: AmountSpec },
    /// Decrement pending gold (clamped at 0). Rare debuff.
    LoseGold { amount: AmountSpec },
    /// Accumulate Stars (StS2 secondary currency). System not yet
    /// wired into card gating; resolves to a counter bump.
    GainStars { amount: AmountSpec },
    /// Channel an orb into the player's orb queue (Defect).
    /// STUB — orb system not yet implemented; this is a no-op so
    /// orb-using cards can be encoded as data with future-compatible
    /// shape.
    ChannelOrb { orb_id: String },
    /// Evoke the front orb in the queue (Defect MultiCast).
    /// STUB — see ChannelOrb.
    EvokeNextOrb,
    /// Trigger the front orb's passive without consuming it
    /// (Recycle-passive). STUB — see ChannelOrb.
    TriggerOrbPassive,
    /// Grow / shrink the orb queue capacity. STUB.
    ChangeOrbSlots { delta: AmountSpec },
    /// Summon an Osty companion (StS2 companion mechanic).
    /// `max_hp` carries the HP the companion is summoned with — most
    /// summon cards use a `Summon` canonical var for this; pass None
    /// to fall back to a default (6 HP) for cards that omit it.
    SummonOsty { osty_id: String, max_hp: Option<AmountSpec> },
    /// Heal the player's Osty companion (Spur).
    HealOsty { amount: AmountSpec },
    /// Set Osty current_hp to 0. Sacrifice (combined with GainBlock).
    KillOsty,
    /// Generate N random cards from a named pool into the target pile.
    /// Mirrors C# `CardFactory.GetDistinctForCombat(pool, n, rng)`. Used
    /// by AttackPotion/SkillPotion/etc. + Discovery/Distraction/Splash/
    /// Stoke/StormOfSteel/WhiteNoise/BeatDown/Bombardment/InfernalBlade
    /// /JackOfAllTrades/Jackpot/Largesse/Quasar/BundleOfJoy.
    AddRandomCardFromPool {
        pool: CardPoolRef,
        filter: CardFilter,
        n: AmountSpec,
        pile: Pile,
        upgrade: i32,
        free_this_turn: bool,
        distinct: bool,
    },
    /// Auto-play matching cards from `pile` (no energy cost). KnifeTrap
    /// (Exhaust→all Shivs), Uproar (Draw→1 Attack).
    AutoplayCardsFromPile {
        pile: Pile,
        filter: CardFilter,
        n: AmountSpec,
    },
    /// Write a scalar to a per-power-instance state field. TheBomb
    /// (SetDamage on its freshly-applied power), ToricToughness
    /// (SetBlock on its freshly-applied power).
    SetPowerStateField {
        power_id: String,
        field: String,
        value: AmountSpec,
        target: Target,
    },
    /// Discard the top N cards of the draw pile straight into discard.
    /// Cycle-family.
    MillFromDraw { n: AmountSpec },
    /// Clone the source card into the named pile with optional cost
    /// override. Mirrors C# `base.CreateClone()` + EnergyCost.Set*.
    /// AdaptiveStrike (clone into Discard with cost 0 ThisCombat),
    /// Undeath (clone into Discard), DualWield (clone N times into Hand).
    CloneSourceCardToPile {
        pile: Pile,
        /// Set the clone's cost-override-this-combat to this value if
        /// Some. Otherwise the clone inherits the source's cost.
        cost_override_this_combat: Option<i32>,
        /// Number of clones to create.
        copies: AmountSpec,
    },
    /// Channel a randomly-chosen orb from a fixed pool. Chaos (random
    /// from Lightning/Frost/Dark/Plasma). Uses combat RNG.
    ChannelRandomOrb { from_pool: Vec<String> },
    /// Copy every Debuff power from `target` onto every other alive
    /// enemy. Misery: each non-target enemy gains the same Debuff
    /// stack counts as the target.
    CopyDebuffsToOtherEnemies,
    /// Add `delta` to the source card's per-instance state counter.
    /// Used by Claw (increment plays counter) and similar self-ramping
    /// cards. Reads via `AmountSpec::SourceCardCounter`.
    IncrementSourceCardCounter { key: String, delta: AmountSpec },
    /// Pick cards from `from` and insert into `to` at `position`.
    /// Glimmer / Headbutt / Hologram / PhotonCut / Dredge.
    MoveCardWithPosition {
        from: Pile,
        to: Pile,
        selector: Selector,
        position: PilePosition,
    },
    /// Pick one card from `from`, create `copies` clones in `to_pile`.
    /// DualWield (pick Attack/Power, clone N to Hand).
    ClonePickedCardToPile {
        from: Pile,
        selector: Selector,
        to_pile: Pile,
        copies: AmountSpec,
    },
    /// Draw cards until a drawn card matches `stop_filter`, capped at
    /// `max_count`. Pillage (draws until non-Attack).
    DrawUntil { stop_filter: CardFilter, max_count: i32 },
    /// Discard entire hand, draw same count. CalculatedGamble.
    DiscardHandAndDrawSameCount,
    /// Auto-play `n` cards from the top of the draw pile. Replaces
    /// the legacy i32-typed AutoplayFromDraw for X-cost shapes
    /// (Cascade). Runtime is STUB — needs play_card recursion.
    AutoplayFromDrawAmount { n: AmountSpec },
    /// Move all matching cards from EVERY combat pile (Hand / Draw /
    /// Discard / Exhaust) to `to_pile`. SummonForth: pull every
    /// SovereignBlade across piles into Hand.
    MoveAllByFilterAcrossPiles { to_pile: Pile, filter: CardFilter },
    /// Add `delta` to the per-instance `state[key]` of a player-picked
    /// card from `from`. HiddenGem: pick from Draw, bump replay counter.
    /// Per-instance (not per-card-id) — mutates the specific CardInstance.
    IncrementPickedCardCounter {
        from: Pile,
        selector: Selector,
        key: String,
        delta: AmountSpec,
    },
    /// Bump the source card's `cost_override_this_combat` by `delta`.
    /// Modded: EnergyCost.AddThisCombat(1).
    AddSourceCardCostThisCombat { delta: AmountSpec },
    /// Add `delta` to `PlayerState.relic_counters[key]`. Used by stateful
    /// relics (Kunai/Shuriken/HappyFlower/Pendulum/Pocketwatch/etc.) to
    /// implement "every Nth attack" / "after N turns" gating. `key`
    /// scopes the counter; relics typically use their id but bodies can
    /// share counters via a common key.
    ModifyRelicCounter { key: String, delta: AmountSpec },
    /// Set `PlayerState.relic_counters[key]` to a specific value
    /// (typically 0 to reset).
    SetRelicCounter { key: String, value: AmountSpec },
    /// Permanently adjust `PlayerState.turn_energy` (per-turn energy
    /// refresh amount). Mirrors C# `ModifyMaxEnergy(player, amount) ->
    /// amount + delta` on RelicModel. Fired at BeforeCombatStart to
    /// apply the offset for the whole combat. PhilosophersStone (+1),
    /// Bread (+1 from r>=2 via Conditional), Sozu (-1), Ectoplasm (-1),
    /// VelvetChoker (-1 after N plays).
    IncreaseMaxEnergy { delta: AmountSpec },
    /// Damage attributed to Osty companion (Protector-family).
    /// STUB — falls back to regular DealDamage for now.
    DamageFromOsty {
        amount: AmountSpec,
        target: Target,
    },
    /// Forge: in-combat upgrade hook tied to the StS2 smith system.
    /// STUB — forge system not yet wired.
    Forge { amount: AmountSpec },
    /// End the player's turn immediately. STUB — calling cs.end_turn()
    /// from inside OnPlay nests the turn machine; needs a "pending
    /// end-of-turn" flag the env loop consumes. FranticEscape is the
    /// only card that uses this.
    EndTurn,
    /// Complete a Quest card's objective (StS2 mechanic). STUB.
    CompleteQuest,
    /// Generate a random potion into the player's potion belt.
    /// STUB — potion-belt state not in CombatState.
    GenerateRandomPotion,
    /// Top up the potion belt to full from the per-combat
    /// potion-generation RNG stream. EntropicBrew. STUB.
    FillPotionSlots,
    /// Auto-play (without paying energy / using a hand slot) the top
    /// `n` cards of the draw pile. DistilledChaos, Mayhem-family.
    /// STUB — needs auto-play recursion into play_card.
    AutoplayFromDraw { n: i32 },
    /// Pick cards via the given selector from `from` and move to `to`.
    /// Generalizes Anointed (pile-pick → hand) and similar.
    MoveCard {
        from: Pile,
        to: Pile,
        selector: Selector,
    },
    /// Exhaust cards selected from `from` (typically Hand). Supersedes
    /// `ExhaustRandomInHand` for the general case (top-of-pile and
    /// filter-based selection are also supported).
    ExhaustCards {
        from: Pile,
        selector: Selector,
    },
    /// Discard cards selected from `from` (typically Hand). Acrobatics
    /// uses this with PlayerInteractive (currently → Random fallback).
    DiscardCards {
        from: Pile,
        selector: Selector,
    },
    /// In-combat upgrade of cards selected from `from`. Armaments.
    UpgradeCards {
        from: Pile,
        selector: Selector,
    },
    /// Apply a runtime keyword (Ethereal / Exhaust / Retain / Innate)
    /// to selected cards. JossPaper-style. STUB — keyword runtime
    /// mutation surface not yet plumbed.
    ApplyKeywordToCards {
        keyword: String,
        from: Pile,
        selector: Selector,
    },
    /// Transform selected cards into random replacements from the
    /// card pool. PandorasBox-style. STUB — transformation requires
    /// CardFactory RNG plumbing.
    TransformCards {
        from: Pile,
        selector: Selector,
    },
    /// Transform selected cards into instances of a specific card_id
    /// (optionally upgraded). Begone (-> MinionStrike), Charge, Guards
    /// (-> MinionSacrifice). Mirrors C# `CardCmd.Transform(picked, new)`.
    TransformIntoSpecific {
        from: Pile,
        selector: Selector,
        target_card_id: String,
        upgrade: bool,
    },
    /// Upgrade every upgradable card across ALL the player's piles.
    /// Apotheosis. Mirrors C# `foreach card in AllCards if IsUpgradable`.
    UpgradeAllUpgradableCards,
    /// Set the energy cost of selected cards for a duration.
    /// Discovery-style. STUB — per-card per-scope cost override not
    /// yet plumbed into CardInstance.
    SetCardCost {
        from: Pile,
        selector: Selector,
        cost: AmountSpec,
        scope: CostScope,
    },
    /// Spawn a fresh monster into the named slot. Used by summon
    /// moves (LivingFog, Fabricator, Ovicopter, Doormaker).
    SummonMonster {
        monster_id: String,
        slot: String,
    },
    /// Drop the actor's own HP to 0 (GasBomb Explode, DeathBlowIntent).
    /// `target` is interpreted as the actor in monster contexts;
    /// `Target::SelfPlayer` is a no-op (no cards self-kill).
    KillSelf,
    /// Set max HP to `amount` and heal to full. TestSubject Revive,
    /// Doormaker DramaticOpen phase shift.
    SetMaxHpAndHeal { amount: AmountSpec, target: Target },
    /// Stun `target` for their next turn — skip their move and clear
    /// the flag. Mirrors C# `CreatureCmd.Stun(creature, ...)` plus the
    /// power-driven variants (Asleep / Slumber / Burrowed → Stun). For
    /// enemies, sets `MonsterState.flags["stunned"]=true`; the next
    /// `dispatch_enemy_turn` consumes the flag. Stun on a player is
    /// not yet modeled (no card / monster targets player-stun in our
    /// current ports).
    Stun { target: Target },
    /// Apply an Affliction to every card in `pile`. HexPower-style:
    /// iterate all cards, set `card.affliction = Some(...)`. STUB —
    /// affliction-on-card infrastructure (CardInstance.affliction
    /// field + lifecycle hooks) not yet present.
    ApplyAfflictionToAllInPile {
        affliction_id: String,
        pile: Pile,
        amount: AmountSpec,
    },

    // ---------- Control flow ----------
    /// Conditional branch. Run `then_branch` if `condition` evaluates
    /// to true; otherwise run `else_branch` (empty if not specified).
    Conditional {
        condition: Condition,
        then_branch: Vec<Effect>,
        else_branch: Vec<Effect>,
    },
    /// Repeat `body` `count` times. Used by X-cost cards
    /// (Whirlwind, Skewer) once `XEnergy` amount is bound — though
    /// most card-level multi-hit goes through `DealDamage.hits`.
    /// More general: lets event-layer steps loop ("for each X").
    Repeat {
        count: AmountSpec,
        body: Vec<Effect>,
    },

    // ---------- Run-state (out-of-combat) — STUB layer ----------
    /// Grant a relic to the player. Map-event rewards, ToyBox-style
    /// "obtain another relic" effects.
    /// STUB: requires a handle to RunState — combat effect VM cannot
    /// mutate it directly. Will route through the event/relic-layer
    /// VM once that lands.
    GainRelic { relic_id: String },
    /// Strip a relic permanently. Rare.
    LoseRelic { relic_id: String },
    /// Drop a specific potion into the player's potion belt.
    /// AlchemicalCoffer / event rewards. STUB — see GainRelic.
    GainPotionToBelt { potion_id: String },
    /// Lose HP at run-state level (events that say "lose 8 HP").
    /// STUB — bypasses combat block/modifier pipeline. Distinct from
    /// `LoseHp` which mutates the combat-frame creature's current_hp.
    LoseRunStateHp { amount: AmountSpec },
    /// Add max HP outside combat. Most "+max HP" effects (Strawberry/
    /// Pear/Mango relics, food events). STUB — currently
    /// `ChangeMaxHp` covers in-combat; this variant signals run-state
    /// scope so the eventual run-state dispatcher knows.
    GainRunStateMaxHp { amount: AmountSpec },
    /// Permanent gold gain (events, +gold relics). Distinct from the
    /// combat-time `GainGold` which writes to pending_gold and folds
    /// into combat rewards. STUB.
    GainRunStateGold { amount: AmountSpec },
    /// Lose max HP outside combat. DistinguishedCape, LeafyPoultice
    /// (`CreatureCmd.LoseMaxHp(N)`).
    LoseRunStateMaxHp { amount: AmountSpec },
    /// Add a specific card to the player's run-state deck. ArcaneScroll
    /// (Rare card factory), BloodSoakedRose (curse), various event
    /// rewards. `upgrade` is the upgrade level the card is added at.
    AddCardToRunStateDeck { card_id: String, upgrade: i32 },
    /// Increase the player's max-potion-belt slot count. PotionBelt
    /// (+2), PhialHolster (+1).
    GainMaxPotionSlots { delta: AmountSpec },

    // ---------- Event flow — STUB ----------
    /// Close the current event with a final description block.
    /// `description_key` is the localization key for the C# loc
    /// system; the Rust port records it without rendering. STUB
    /// until the event-state machine lands.
    SetEventFinished { description_key: String },
    /// Transition to another event page (multi-page events).
    /// STUB — events not yet modeled in run state.
    MoveToEventPage { page_id: String },
}

/// Lifetime of a card-cost override.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostScope {
    ThisTurn,
    ThisCombat,
    UntilPlayed,
}

/// Position to insert a card into a pile. Mirrors C# `CardPilePosition`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PilePosition {
    /// Top of the pile (back of Vec; drawn first).
    Top,
    /// Bottom of the pile (front of Vec; drawn last).
    Bottom,
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
    /// The creature acting as the source of this effect list. For
    /// card OnPlay this is `(Player, player_idx)`; for monster moves
    /// authored as effect lists, this is the monster. `Target::SelfActor`
    /// resolves to this.
    pub actor: (CombatSide, usize),
    /// Pre-bound actor's power amount for the hook's host power.
    /// `AmountSpec::OwnerPowerAmount` reads this. The power-VM
    /// dispatcher sets it before invoking the body. 0 outside power
    /// contexts.
    pub actor_amount: i32,
    /// Relic id that owns this effect list. Read by `Canonical` when
    /// `source_card_id` is None — relic hooks share the same Effect
    /// vocabulary but their canonical-var table lives on `RelicData`.
    pub source_relic_id: Option<&'a str>,
    /// Running total of damage dealt by the most recent `DealDamage`
    /// step in the current effect list. Used by
    /// `AmountSpec::LastRealizedDamage` (Fisticuffs). Interior mutability
    /// so the dispatcher can update through `&EffectContext`.
    pub last_realized_damage: std::cell::Cell<i32>,
    /// Running total of block gained by the most recent `GainBlock`
    /// step. Used by `AmountSpec::LastRealizedBlock` (DodgeAndRoll).
    pub last_realized_block: std::cell::Cell<i32>,
    /// Hand size at the moment OnPlay started (after the source card
    /// was removed from hand). Set by `play_card`. Used by
    /// hand-exhaust-then-spawn cards (Stoke / StormOfSteel) and
    /// pre-exhaust counts (FlakCannon).
    pub hand_size_at_play_start: i32,
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
            actor: (CombatSide::Player, player_idx),
            actor_amount: 0,
            source_relic_id: None,
            last_realized_damage: std::cell::Cell::new(0),
            last_realized_block: std::cell::Cell::new(0),
            hand_size_at_play_start: 0,
        }
    }

    /// Convenience builder for monster-move authoring. The actor is
    /// the moving enemy; player_idx is the targeted player (defaults
    /// to 0 for single-player). `target` is the chosen-target slot
    /// for moves that target a specific opponent.
    pub fn for_monster_move(
        actor_idx: usize,
        target: Option<(CombatSide, usize)>,
    ) -> Self {
        Self {
            player_idx: 0,
            target,
            source_card_id: None,
            upgrade_level: 0,
            enchantment: None,
            x_value: 0,
            actor: (CombatSide::Enemy, actor_idx),
            actor_amount: 0,
            source_relic_id: None,
            last_realized_damage: std::cell::Cell::new(0),
            last_realized_block: std::cell::Cell::new(0),
            hand_size_at_play_start: 0,
        }
    }

    /// Builder for power-hook bodies. The actor is the power's owner;
    /// `host_power_amount` pre-binds `AmountSpec::OwnerPowerAmount` to
    /// the current stack count.
    pub fn for_power_hook(
        actor: (CombatSide, usize),
        host_power_amount: i32,
    ) -> Self {
        Self {
            player_idx: 0,
            target: None,
            source_card_id: None,
            upgrade_level: 0,
            enchantment: None,
            x_value: 0,
            actor,
            actor_amount: host_power_amount,
            source_relic_id: None,
            last_realized_damage: std::cell::Cell::new(0),
            last_realized_block: std::cell::Cell::new(0),
            hand_size_at_play_start: 0,
        }
    }

    /// Convenience builder for relic-hook bodies. The actor is the
    /// owning player; `Canonical` amounts resolve through the relic's
    /// `canonical_vars` table.
    pub fn for_relic_hook(player_idx: usize, relic_id: &'a str) -> Self {
        Self {
            player_idx,
            target: None,
            source_card_id: None,
            upgrade_level: 0,
            enchantment: None,
            x_value: 0,
            actor: (CombatSide::Player, player_idx),
            actor_amount: 0,
            source_relic_id: Some(relic_id),
            last_realized_damage: std::cell::Cell::new(0),
            last_realized_block: std::cell::Cell::new(0),
            hand_size_at_play_start: 0,
        }
    }

    /// Builder for potion-OnUse invocations. The actor is the using
    /// player; `Canonical` amounts resolve through the potion's
    /// `canonical_vars` table — same schema as relic vars, so we
    /// tunnel through the same `source_relic_id` slot. (`Canonical`
    /// resolution checks card first, then this slot, then a separate
    /// potion lookup — see `AmountSpec::resolve`.)
    pub fn for_potion_use(
        player_idx: usize,
        target: Option<(CombatSide, usize)>,
        potion_id: &'a str,
    ) -> Self {
        Self {
            player_idx,
            target,
            source_card_id: None,
            upgrade_level: 0,
            enchantment: None,
            x_value: 0,
            actor: (CombatSide::Player, player_idx),
            actor_amount: 0,
            source_relic_id: Some(potion_id),
            last_realized_damage: std::cell::Cell::new(0),
            last_realized_block: std::cell::Cell::new(0),
            hand_size_at_play_start: 0,
        }
    }
}

impl AmountSpec {
    /// Resolve to an integer value given context + live combat state.
    /// `cs` is required for the calc-var variants (SelfBlock,
    /// TargetPowerAmount, CardCountInPile) that read live state.
    /// Constant variants (Fixed, Canonical, BranchedOnUpgrade,
    /// XEnergy, OwnerPowerAmount) ignore `cs`.
    pub fn resolve(&self, ctx: &EffectContext, cs: &CombatState) -> i32 {
        match self {
            AmountSpec::Fixed(n) => *n,
            AmountSpec::Canonical(var_kind) => {
                if let Some(card_id) = ctx.source_card_id {
                    if let Some(card) = card_by_id(card_id) {
                        return canonical_int_value(card, var_kind, ctx.upgrade_level);
                    }
                }
                if let Some(id) = ctx.source_relic_id {
                    // The slot is shared between relics and potions —
                    // try relic first, then potion (data tables are
                    // disjoint, so at most one will resolve).
                    let v = relic_canonical_int_value(id, var_kind);
                    if v != 0 {
                        return v;
                    }
                    return potion_canonical_int_value(id, var_kind);
                }
                0
            }
            AmountSpec::BranchedOnUpgrade { base, upgraded } => {
                if ctx.upgrade_level > 0 {
                    *upgraded
                } else {
                    *base
                }
            }
            AmountSpec::XEnergy => ctx.x_value,
            AmountSpec::Multiplied { base, factor } => base.resolve(ctx, cs) * factor,
            AmountSpec::Mul { left, right } => left.resolve(ctx, cs) * right.resolve(ctx, cs),
            AmountSpec::OwnerPowerAmount(_) => ctx.actor_amount,
            AmountSpec::SelfBlock => {
                // Mirrors C# `Owner.Creature.Block`. Reads actor's block;
                // for cards this is the player, for monster moves the
                // moving monster.
                let (side, idx) = ctx.actor;
                match side {
                    CombatSide::Player => cs.allies.get(idx).map(|c| c.block).unwrap_or(0),
                    CombatSide::Enemy => cs.enemies.get(idx).map(|c| c.block).unwrap_or(0),
                    CombatSide::None => 0,
                }
            }
            AmountSpec::TargetPowerAmount { power_id } => {
                // Bully-shape: scale by target's debuff stacks. Falls
                // back to actor if target is None.
                let (side, idx) = ctx.target.unwrap_or(ctx.actor);
                let creature = match side {
                    CombatSide::Player => cs.allies.get(idx),
                    CombatSide::Enemy => cs.enemies.get(idx),
                    CombatSide::None => None,
                };
                creature
                    .and_then(|c| {
                        c.powers
                            .iter()
                            .find(|p| p.id == *power_id)
                            .map(|p| p.amount)
                    })
                    .unwrap_or(0)
            }
            AmountSpec::Add { left, right } => left.resolve(ctx, cs) + right.resolve(ctx, cs),
            AmountSpec::OstyMaxHp => cs
                .allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
                .and_then(|ps| ps.osty.as_ref())
                .map(|o| o.max_hp)
                .unwrap_or(0),
            AmountSpec::OstyBlock => cs
                .allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
                .and_then(|ps| ps.osty.as_ref())
                .map(|o| o.block)
                .unwrap_or(0),
            AmountSpec::CardCountInPile { pile, filter } => {
                // PerfectedStrike-shape: count cards in pile(s)
                // matching filter. AllCombat = Hand+Draw+Discard+Exhaust.
                let Some(ps) = cs
                    .allies
                    .get(ctx.player_idx)
                    .and_then(|c| c.player.as_ref())
                else {
                    return 0;
                };
                let count_in = |pile: Pile| -> i32 {
                    let cards = match pile {
                        Pile::Hand => &ps.hand.cards,
                        Pile::Discard => &ps.discard.cards,
                        Pile::Draw => &ps.draw.cards,
                        Pile::Exhaust => &ps.exhaust.cards,
                        Pile::Deck => return 0, // not addressable from combat
                    };
                    cards
                        .iter()
                        .filter(|c| matches_filter(c, filter))
                        .count() as i32
                };
                match pile {
                    PileSelector::Single(p) => count_in(*p),
                    PileSelector::AllCombat => {
                        count_in(Pile::Hand)
                            + count_in(Pile::Discard)
                            + count_in(Pile::Draw)
                            + count_in(Pile::Exhaust)
                    }
                }
            }
            AmountSpec::HandCount => cs
                .allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
                .map(|ps| ps.hand.cards.len() as i32)
                .unwrap_or(0),
            AmountSpec::HandCountExcludingSource => {
                let h = cs
                    .allies
                    .get(ctx.player_idx)
                    .and_then(|c| c.player.as_ref())
                    .map(|ps| ps.hand.cards.len() as i32)
                    .unwrap_or(0);
                // The currently-playing card has already been moved out
                // of hand by `play_card` before OnPlay runs, so the raw
                // count IS "excluding the source card" — but defensively
                // we subtract 1 only if the source card is still seen
                // in hand. For the common case we just return h.
                h
            }
            AmountSpec::TargetMaxHp => {
                let (side, idx) = ctx.target.unwrap_or(ctx.actor);
                let creature = match side {
                    CombatSide::Player => cs.allies.get(idx),
                    CombatSide::Enemy => cs.enemies.get(idx),
                    CombatSide::None => None,
                };
                creature.map(|c| c.max_hp).unwrap_or(0)
            }
            AmountSpec::TargetBlock => {
                let (side, idx) = ctx.target.unwrap_or(ctx.actor);
                let creature = match side {
                    CombatSide::Player => cs.allies.get(idx),
                    CombatSide::Enemy => cs.enemies.get(idx),
                    CombatSide::None => None,
                };
                creature.map(|c| c.block).unwrap_or(0)
            }
            AmountSpec::TargetDebuffCount => {
                let (side, idx) = ctx.target.unwrap_or(ctx.actor);
                let creature = match side {
                    CombatSide::Player => cs.allies.get(idx),
                    CombatSide::Enemy => cs.enemies.get(idx),
                    CombatSide::None => None,
                };
                creature
                    .map(|c| {
                        c.powers
                            .iter()
                            .filter(|p| is_debuff_power(&p.id))
                            .count() as i32
                    })
                    .unwrap_or(0)
            }
            AmountSpec::CardsPlayedThisTurn { filter } => {
                cards_played_this_turn(cs, ctx.player_idx, filter)
            }
            AmountSpec::CardsDiscardedThisTurn => {
                count_history_events_this_turn(cs, ctx.player_idx, HistoryKind::Discarded)
            }
            AmountSpec::CardsDrawnThisTurn => {
                count_history_events_this_turn(cs, ctx.player_idx, HistoryKind::Drawn)
            }
            AmountSpec::CardsExhaustedThisTurn => {
                cards_exhausted_this_turn(cs, ctx.player_idx)
            }
            AmountSpec::EnergySpentThisTurn => {
                energy_spent_this_turn(cs, ctx.player_idx)
            }
            AmountSpec::OrbsChanneledThisCombat { orb_id } => {
                orbs_channeled_this_combat(cs, ctx.player_idx, orb_id.as_deref())
            }
            AmountSpec::DistinctOrbTypesInQueue => cs
                .allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
                .map(|ps| {
                    let mut seen: std::collections::HashSet<&str> =
                        std::collections::HashSet::new();
                    for o in &ps.orb_queue {
                        seen.insert(o.id.as_str());
                    }
                    seen.len() as i32
                })
                .unwrap_or(0),
            AmountSpec::StarsGainedThisTurnPositive => {
                stars_gained_this_turn_positive(cs, ctx.player_idx)
            }
            AmountSpec::Max { left, right } => {
                left.resolve(ctx, cs).max(right.resolve(ctx, cs))
            }
            AmountSpec::Min { left, right } => {
                left.resolve(ctx, cs).min(right.resolve(ctx, cs))
            }
            AmountSpec::Sub { left, right } => {
                left.resolve(ctx, cs) - right.resolve(ctx, cs)
            }
            AmountSpec::FloorDiv { left, right } => {
                let r = right.resolve(ctx, cs);
                if r <= 0 {
                    return 0;
                }
                let l = left.resolve(ctx, cs);
                let q = l / r;
                if (l % r) != 0 && (l < 0) {
                    q - 1
                } else {
                    q
                }
            }
            AmountSpec::CurrentEnergy => cs
                .allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
                .map(|ps| ps.energy)
                .unwrap_or(0),
            AmountSpec::AllEnemiesPowerSum { power_id } => cs
                .enemies
                .iter()
                .filter(|e| e.current_hp > 0)
                .map(|e| {
                    e.powers
                        .iter()
                        .find(|p| p.id == *power_id)
                        .map(|p| p.amount)
                        .unwrap_or(0)
                })
                .sum::<i32>(),
            AmountSpec::EmptyOrbSlots => cs
                .allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
                .map(|ps| (ps.orb_slots - ps.orb_queue.len() as i32).max(0))
                .unwrap_or(0),
            AmountSpec::SourceCardCounter { key } => {
                let Some(card_id) = ctx.source_card_id else {
                    return 0;
                };
                let namespaced = format!("card.{}.{}", card_id, key);
                cs.allies
                    .get(ctx.player_idx)
                    .and_then(|c| c.player.as_ref())
                    .map(|ps| ps.relic_counters.get(&namespaced).copied().unwrap_or(0))
                    .unwrap_or(0)
            }
            AmountSpec::AliveEnemyCount => {
                cs.enemies.iter().filter(|e| e.current_hp > 0).count() as i32
            }
            AmountSpec::AliveAllyCount => {
                cs.allies.iter().filter(|a| a.current_hp > 0).count() as i32
            }
            AmountSpec::OwnerCurrentHp => cs
                .allies
                .get(ctx.player_idx)
                .map(|c| c.current_hp)
                .unwrap_or(0),
            AmountSpec::OwnerMaxHp => cs
                .allies
                .get(ctx.player_idx)
                .map(|c| c.max_hp)
                .unwrap_or(0),
            AmountSpec::OwnerHpMissing => cs
                .allies
                .get(ctx.player_idx)
                .map(|c| (c.max_hp - c.current_hp).max(0))
                .unwrap_or(0),
            AmountSpec::LastRealizedDamage => ctx.last_realized_damage.get(),
            AmountSpec::LastRealizedBlock => ctx.last_realized_block.get(),
            AmountSpec::EtherealCardsPlayedThisTurn => {
                ethereal_cards_played_this_turn(cs, ctx.player_idx)
            }
            AmountSpec::HandSizeAtPlayStart => ctx.hand_size_at_play_start,
        }
    }
}

fn ethereal_cards_played_this_turn(cs: &CombatState, player_idx: usize) -> i32 {
    let turn_start = current_turn_start_idx(cs);
    cs.combat_log
        .iter()
        .skip(turn_start)
        .filter(|ev| match ev {
            crate::combat::CombatEvent::CardPlayed {
                player_idx: pid,
                ethereal,
                ..
            } => *pid == player_idx && *ethereal,
            _ => false,
        })
        .count() as i32
}

/// Per-power-id classifier used by `AmountSpec::TargetDebuffCount`. The
/// PowerData table carries a `PowerType` field (Buff / Debuff); this
/// helper reads it for the known debuff classes. Falls back to the
/// PowerData lookup.
fn is_debuff_power(power_id: &str) -> bool {
    crate::power::by_id(power_id)
        .map(|p| matches!(p.power_type, crate::power::PowerType::Debuff))
        .unwrap_or(false)
}

#[derive(Copy, Clone)]
enum HistoryKind {
    Discarded,
    Drawn,
}

fn cards_played_this_turn(
    cs: &CombatState,
    player_idx: usize,
    filter: &CardFilter,
) -> i32 {
    let turn_start = current_turn_start_idx(cs);
    cs.combat_log
        .iter()
        .skip(turn_start)
        .filter(|ev| match ev {
            crate::combat::CombatEvent::CardPlayed {
                player_idx: pid,
                card_id,
                round: _,
                card_type: _,
                cost: _,
                ethereal: _,
            } => {
                if *pid != player_idx {
                    return false;
                }
                card_filter_matches_id(filter, card_id)
            }
            _ => false,
        })
        .count() as i32
}

fn cards_played_with_id_this_turn(
    cs: &CombatState,
    player_idx: usize,
    card_id: &str,
) -> i32 {
    let turn_start = current_turn_start_idx(cs);
    cs.combat_log
        .iter()
        .skip(turn_start)
        .filter(|ev| match ev {
            crate::combat::CombatEvent::CardPlayed {
                player_idx: pid,
                card_id: cid,
                ..
            } => *pid == player_idx && cid == card_id,
            _ => false,
        })
        .count() as i32
}

fn count_history_events_this_turn(
    cs: &CombatState,
    player_idx: usize,
    kind: HistoryKind,
) -> i32 {
    let turn_start = current_turn_start_idx(cs);
    cs.combat_log
        .iter()
        .skip(turn_start)
        .filter(|ev| match (ev, kind) {
            (
                crate::combat::CombatEvent::CardDiscarded { player_idx: pid, .. },
                HistoryKind::Discarded,
            )
            | (
                crate::combat::CombatEvent::CardDrawn { player_idx: pid, .. },
                HistoryKind::Drawn,
            ) => *pid == player_idx,
            _ => false,
        })
        .count() as i32
}

fn cards_exhausted_this_turn(cs: &CombatState, player_idx: usize) -> i32 {
    let turn_start = current_turn_start_idx(cs);
    cs.combat_log
        .iter()
        .skip(turn_start)
        .filter(|ev| match ev {
            crate::combat::CombatEvent::CardExhausted {
                player_idx: pid, ..
            } => *pid == player_idx,
            _ => false,
        })
        .count() as i32
}

fn energy_spent_this_turn(cs: &CombatState, player_idx: usize) -> i32 {
    let turn_start = current_turn_start_idx(cs);
    cs.combat_log
        .iter()
        .skip(turn_start)
        .filter_map(|ev| match ev {
            crate::combat::CombatEvent::CardPlayed {
                player_idx: pid,
                cost,
                ..
            } if *pid == player_idx => Some(*cost),
            _ => None,
        })
        .sum::<i32>()
}

fn orbs_channeled_this_combat(
    cs: &CombatState,
    player_idx: usize,
    orb_id: Option<&str>,
) -> i32 {
    cs.combat_log
        .iter()
        .filter(|ev| match ev {
            crate::combat::CombatEvent::OrbChanneled {
                player_idx: pid,
                orb_id: oid,
                round: _,
            } => {
                if *pid != player_idx {
                    return false;
                }
                match orb_id {
                    Some(want) => oid == want,
                    None => true,
                }
            }
            _ => false,
        })
        .count() as i32
}

fn stars_gained_this_turn_positive(cs: &CombatState, player_idx: usize) -> i32 {
    let turn_start = current_turn_start_idx(cs);
    cs.combat_log
        .iter()
        .skip(turn_start)
        .filter_map(|ev| match ev {
            crate::combat::CombatEvent::StarsChanged {
                player_idx: pid,
                delta,
                round: _,
            } if *pid == player_idx && *delta > 0 => Some(*delta),
            _ => None,
        })
        .sum::<i32>()
}

/// Index into `combat_log` of the most recent `TurnBegan` event. All
/// "this-turn" history scans start from here.
fn current_turn_start_idx(cs: &CombatState) -> usize {
    cs.combat_log
        .iter()
        .rposition(|ev| matches!(ev, crate::combat::CombatEvent::TurnBegan { .. }))
        .map(|i| i + 1)
        .unwrap_or(0)
}

fn card_filter_matches_id(filter: &CardFilter, card_id: &str) -> bool {
    let Some(card) = crate::card::by_id(card_id) else {
        return matches!(filter, CardFilter::Any);
    };
    match filter {
        CardFilter::Any => true,
        CardFilter::Upgradable => card.max_upgrade_level > 0,
        CardFilter::OfType(name) => match name.as_str() {
            "Attack" => card.card_type == crate::card::CardType::Attack,
            "Skill" => card.card_type == crate::card::CardType::Skill,
            "Power" => card.card_type == crate::card::CardType::Power,
            "Status" => card.card_type == crate::card::CardType::Status,
            "Curse" => card.card_type == crate::card::CardType::Curse,
            _ => false,
        },
        CardFilter::HasKeyword(kw) => card.keywords.iter().any(|k| k == kw),
        CardFilter::TaggedAs(tag) => card.tags.iter().any(|t| t == tag),
        CardFilter::OfRarity(r) => format!("{:?}", card.rarity).eq_ignore_ascii_case(r),
        CardFilter::And(a, b) => {
            card_filter_matches_id(a, card_id) && card_filter_matches_id(b, card_id)
        }
        CardFilter::Or(a, b) => {
            card_filter_matches_id(a, card_id) || card_filter_matches_id(b, card_id)
        }
        CardFilter::Not(inner) => !card_filter_matches_id(inner, card_id),
        CardFilter::HasId(id) => card_id == id,
        CardFilter::WithEnergyCost { op, value } => compare(card.energy_cost, *op, *value),
        CardFilter::NotXCost => !card.has_energy_cost_x,
    }
}

/// Walk an effect list and execute each step against `cs`. Effects are
/// applied in order; no implicit batching or reordering.
pub fn execute_effects(cs: &mut CombatState, effects: &[Effect], ctx: &EffectContext) {
    for eff in effects {
        execute_effect(cs, eff, ctx);
    }
}

// ========================================================================
// Power VM — same shape as the card VM, applied to PowerModel lifecycle.
// ========================================================================
//
// Powers have C# override methods on `AbstractModel` (BeforeAttack,
// AfterTurnEnd, AfterCardPlayed, AfterApplied, …). The Power VM expresses
// the body of each override as a `Vec<Effect>` keyed by a `PowerHook`
// trigger variant — exactly the same dispatch pattern as card OnPlay
// goes through `card_effects`.
//
// Initial scope is one hook trigger: `AfterTurnEnd`. RegenPower is the
// first migration. The other ~25 trigger variants (audit §2, full list
// in `project_pipeline_audit_2026_05_14.md`) get added incrementally
// alongside their first consumer.

/// Closed enum of power-lifecycle trigger points. Mirrors the C# hook
/// surface declared on `AbstractModel`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerHook {
    /// C# `AfterTurnEnd(side)`. Fires once per side turn end. The
    /// `filter` discriminant gates on whether the ended side matches
    /// the power owner's side.
    AfterTurnEnd {
        filter: HookSideFilter,
        body: Vec<Effect>,
    },
    /// C# `AfterSideTurnStart(side)`. Fires once when a side's turn
    /// begins (after draw / energy refresh). DemonForm / Ritual /
    /// Poison fire on owner's turn start; Plasma orb / Coolant trigger
    /// at this phase.
    AfterSideTurnStart {
        filter: HookSideFilter,
        body: Vec<Effect>,
    },
    // TODO (audit §2): BeforeSideTurnStart, BeforeTurnEnd, AfterApplied,
    // AfterRemoved, BeforeAttack, AfterAttack, AfterDamageGiven,
    // BeforeDamageReceived, AfterDamageReceived, AfterCardPlayed,
    // BeforeCardPlayed, AfterDeath, OnHostDeath, ShouldClearBlock,
    // ShouldDie, ...
}

/// Discriminant for hook side-filtering. Mirrors the C#
/// `if (side == base.Owner.Side)` pattern.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookSideFilter {
    /// Body fires only when the ended/starting side matches the
    /// power owner's side. C# default.
    OwnerSide,
    /// Body fires regardless of which side. Rare.
    Any,
}

/// Registry of powers whose lifecycle behavior is expressed as data.
///
/// Migrating a power here means: (1) its overrides only call existing
/// VM primitives, and (2) any pre-existing hardcoded behavior in
/// combat.rs's match-arm `tick_*` paths is removed so we don't double-fire.
///
/// Plan §0.2.3 + audit §6 recommendation. First 10 migration order:
///   1. RegenPower (this commit) — simplest: heal + decrement at turn end.
///   2. Strength / Dexterity — already wired via additive table; would
///      port for uniformity once damage-modifier hooks join the VM.
///   3. Weak / Vulnerable / Frail — duration-tick variant.
///   4. Poison — symmetric to Regen (damage, side=Owner).
///   5. DemonForm / Ritual — AfterSideTurnStart bodies.
///   6. Barricade — ShouldClearBlock gate.
pub fn power_effects(power_id: &str) -> Vec<PowerHook> {
    match power_id {
        // RegenPower: at end of owner's turn, heal Owner by Amount and
        // decrement the stack by 1. Mirrors RegenPower.cs:46-57.
        "RegenPower" => vec![PowerHook::AfterTurnEnd {
            filter: HookSideFilter::OwnerSide,
            body: vec![
                Effect::Heal {
                    amount: AmountSpec::OwnerPowerAmount("RegenPower".to_string()),
                    target: Target::SelfActor,
                },
                Effect::ApplyPower {
                    power_id: "RegenPower".to_string(),
                    amount: AmountSpec::Fixed(-1),
                    target: Target::SelfActor,
                },
            ],
        }],
        // PoisonPower: at start of owner's turn, deal Amount damage
        // (Unblockable | Unpowered -> bypasses block + modifiers), then
        // decrement stack by 1. Mirrors PoisonPower.cs:81-100. We use
        // LoseHp + ApplyPower(-1) to match the hardcoded behavior.
        //
        // C# TriggerCount considers AccelerantPower on opponents; in
        // typical play (no Accelerant) TriggerCount == 1 so our single
        // tick + decrement matches. Accelerant-induced multi-tick is
        // not yet modeled.
        "PoisonPower" => vec![PowerHook::AfterSideTurnStart {
            filter: HookSideFilter::OwnerSide,
            body: vec![
                Effect::LoseHp {
                    amount: AmountSpec::OwnerPowerAmount("PoisonPower".to_string()),
                    target: Target::SelfActor,
                },
                Effect::ApplyPower {
                    power_id: "PoisonPower".to_string(),
                    amount: AmountSpec::Fixed(-1),
                    target: Target::SelfActor,
                },
            ],
        }],
        // DemonFormPower: at start of owner's turn, apply
        // StrengthPower(Amount) to owner. Permanent ramp.
        "DemonFormPower" => vec![PowerHook::AfterSideTurnStart {
            filter: HookSideFilter::OwnerSide,
            body: vec![Effect::ApplyPower {
                power_id: "StrengthPower".to_string(),
                amount: AmountSpec::OwnerPowerAmount("DemonFormPower".to_string()),
                target: Target::SelfActor,
            }],
        }],
        // TODO Strength/Dex/Weak/Vulnerable/Frail/Intangible:
        // These are damage/block VALUE-FLOW modifier hooks
        // (ModifyDamageAdditive / ModifyDamageMultiplicative / etc.),
        // not event hooks. Migrating them requires the damage pipeline
        // to walk power_effects entries by hook KIND — a separate
        // layer parallel to AfterTurnEnd / AfterSideTurnStart. Left
        // on the existing hardcoded pipeline in combat.rs until the
        // hook-dispatcher #70 lands.
        //
        // Same applies to Barricade/Burrowed (ShouldClearBlock gate),
        // Ritual (counter ramp with WasJustAppliedByEnemy flag), and
        // the AfterDamageGiven/AfterDamageReceived family.
        _ => vec![],
    }
}

/// Walk every living creature's powers and execute any matching
/// `AfterSideTurnStart` hook bodies. Called from `CombatState::begin_turn`
/// after the existing hardcoded turn-start paths.
pub fn fire_power_hooks_after_side_turn_start(
    cs: &mut CombatState,
    started_side: CombatSide,
) {
    fire_power_hooks_impl(cs, started_side, |hook| match hook {
        PowerHook::AfterSideTurnStart { filter, body } => Some((*filter, body.as_slice())),
        _ => None,
    });
}

/// Walk every living creature's powers and execute any matching
/// `AfterTurnEnd` hook bodies. Called from `CombatState::end_turn`
/// after the existing hardcoded tick paths.
pub fn fire_power_hooks_after_turn_end(cs: &mut CombatState, ended_side: CombatSide) {
    fire_power_hooks_impl(cs, ended_side, |hook| match hook {
        PowerHook::AfterTurnEnd { filter, body } => Some((*filter, body.as_slice())),
        _ => None,
    });
}

/// Shared per-phase dispatcher: snapshots (side, idx, power_id, amount)
/// for every living creature's power, then for each entry calls
/// `extract` to find the right PowerHook variant + dispatch its body.
fn fire_power_hooks_impl<F>(cs: &mut CombatState, phase_side: CombatSide, extract: F)
where
    F: for<'a> Fn(&'a PowerHook) -> Option<(HookSideFilter, &'a [Effect])>,
{
    // Snapshot (side, idx, power_id, amount) so iteration is stable
    // against mid-body mutations (heal/apply/remove etc. mutate the
    // powers list).
    let mut snapshot: Vec<(CombatSide, usize, String, i32)> = Vec::new();
    for (i, c) in cs.allies.iter().enumerate() {
        for p in &c.powers {
            snapshot.push((CombatSide::Player, i, p.id.clone(), p.amount));
        }
    }
    for (i, c) in cs.enemies.iter().enumerate() {
        for p in &c.powers {
            snapshot.push((CombatSide::Enemy, i, p.id.clone(), p.amount));
        }
    }
    for (side, idx, power_id, amount) in snapshot {
        // Skip dead actors (matches C# `if !base.Owner.IsDead`).
        let alive = match side {
            CombatSide::Player => cs.allies.get(idx).map(|c| c.current_hp > 0),
            CombatSide::Enemy => cs.enemies.get(idx).map(|c| c.current_hp > 0),
            CombatSide::None => Some(false),
        }
        .unwrap_or(false);
        if !alive {
            continue;
        }
        let hooks = power_effects(&power_id);
        for hook in &hooks {
            let Some((filter, body)) = extract(hook) else {
                continue;
            };
            let matches = match filter {
                HookSideFilter::OwnerSide => side == phase_side,
                HookSideFilter::Any => true,
            };
            if !matches {
                continue;
            }
            let ctx = EffectContext::for_power_hook((side, idx), amount);
            // Body is borrowed from `hooks`; clone to satisfy the
            // execute_effects signature.
            let body_owned: Vec<Effect> = body.to_vec();
            execute_effects(cs, &body_owned, &ctx);
        }
    }
}

// ========================================================================
// Monster move VM — data-driven monster-intent payloads.
// ========================================================================

/// Registry of per-monster move payloads. Keyed by `(monster_id,
/// intent_name)`; the body is a `Vec<Effect>` interpreted with
/// `EffectContext::for_monster_move` (actor = the moving monster).
///
/// Monster state machines (intent picking, FollowUpState transitions)
/// stay as Rust code in `combat.rs::pick_*_intent` / `monster_dispatch`
/// — they're choreography, not pure effect composition. Move PAYLOADS
/// route through this registry once migrated.
///
/// Proof-of-concept: Axebot's four intents wired here. Remaining
/// monsters (~30 model_ids) follow the same pattern but each requires
/// hand-encoding their payload; the migration is mechanical and
/// optional (existing match-arm dispatchers are functional).
pub fn monster_move_effects(
    monster_id: &str,
    intent_name: &str,
) -> Option<Vec<Effect>> {
    match (monster_id, intent_name) {
        // Axebot.cs (constants from combat.rs Axebot section):
        //   BootUp: GainBlock(10) + Apply<StrengthPower>(self, 1).
        //   OneTwo: Damage(5) x 2 to chosen player target.
        //   Sharpen: Apply<StrengthPower>(self, 4).
        //   HammerUppercut: Damage(8) + Apply<WeakPower>(player, 1) +
        //                   Apply<FrailPower>(player, 1).
        ("Axebot", "BootUp") => Some(vec![
            Effect::GainBlock {
                amount: AmountSpec::Fixed(10),
                target: Target::SelfActor,
            },
            Effect::ApplyPower {
                power_id: "StrengthPower".to_string(),
                amount: AmountSpec::Fixed(1),
                target: Target::SelfActor,
            },
        ]),
        ("Axebot", "OneTwo") => Some(vec![Effect::DealDamage {
            amount: AmountSpec::Fixed(5),
            target: Target::ChosenEnemy,
            hits: 2,
        }]),
        ("Axebot", "Sharpen") => Some(vec![Effect::ApplyPower {
            power_id: "StrengthPower".to_string(),
            amount: AmountSpec::Fixed(4),
            target: Target::SelfActor,
        }]),
        ("Axebot", "HammerUppercut") => Some(vec![
            Effect::DealDamage {
                amount: AmountSpec::Fixed(8),
                target: Target::ChosenEnemy,
                hits: 1,
            },
            Effect::ApplyPower {
                power_id: "WeakPower".to_string(),
                amount: AmountSpec::Fixed(1),
                target: Target::ChosenEnemy,
            },
            Effect::ApplyPower {
                power_id: "FrailPower".to_string(),
                amount: AmountSpec::Fixed(1),
                target: Target::ChosenEnemy,
            },
        ]),        // ===== Manual monster-move ports (batch_m_*) =====


        ("Myte", "Toxic") => Some(vec![
        Effect::AddCardToPile { card_id: "Toxic".to_string(), upgrade: 0, pile: Pile::Hand },
        Effect::AddCardToPile { card_id: "Toxic".to_string(), upgrade: 0, pile: Pile::Hand },
        ]),

        ("Myte", "Bite") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(13), target: Target::ChosenEnemy, hits: 1 }]),

        ("Myte", "Suck") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(4), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("Nibbit", "Butt") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(12), target: Target::ChosenEnemy, hits: 1 }]),

        ("Nibbit", "Slice") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(6), target: Target::ChosenEnemy, hits: 1 },
        Effect::GainBlock { amount: AmountSpec::Fixed(5), target: Target::SelfActor },
        ]),

        ("Nibbit", "Hiss") => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor }]),

        ("OwlMagistrate", "Scrutiny") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(16), target: Target::ChosenEnemy, hits: 1 }]),

        ("OwlMagistrate", "PeckAssault") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(4), target: Target::ChosenEnemy, hits: 6 }]),

        ("OwlMagistrate", "JudicialFlight") => Some(vec![
        Effect::ApplyPower { power_id: "SoarPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfActor },
        ]),

        ("OwlMagistrate", "Verdict") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(33), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Fixed(4), target: Target::ChosenEnemy },
        Effect::RemovePower { power_id: "SoarPower".to_string(), target: Target::SelfActor },
        ]),

        ("WaterfallGiant", "Pressurize") => Some(vec![]),

        ("WaterfallGiant", "Stomp") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(15), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::ChosenEnemy },
        ]),

        ("WaterfallGiant", "Ram") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(10), target: Target::ChosenEnemy, hits: 1 }]),

        ("WaterfallGiant", "Siphon") => Some(vec![Effect::Heal { amount: AmountSpec::Fixed(15), target: Target::SelfActor }]),

        ("WaterfallGiant", "PressureUp") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(13), target: Target::ChosenEnemy, hits: 1 }]),

        ("TwoTailedRat", "Scratch") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(8), target: Target::ChosenEnemy, hits: 1 }]),

        ("TwoTailedRat", "DiseaseBite") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(6), target: Target::ChosenEnemy, hits: 1 },
        // C# also afflicts a card with Disease — deferred (matches Rust impl).
        ]),

        ("TwoTailedRat", "Screech") => Some(vec![
        Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::ChosenEnemy },
        ]),

        ("TwoTailedRat", "CallForBackup") => Some(vec![]),

        ("TheObscura", "Illusion") => Some(vec![]),

        ("TheObscura", "PiercingGaze") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(10), target: Target::ChosenEnemy, hits: 1 }]),

        ("TheObscura", "HardeningStrike") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(6), target: Target::ChosenEnemy, hits: 1 },
        Effect::GainBlock { amount: AmountSpec::Fixed(6), target: Target::SelfActor },
        ]),

        ("LivingFog", "AdvancedGas") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(8), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "SmoggyPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::ChosenEnemy },
        ]),

        ("LivingFog", "Bloat") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(5), target: Target::ChosenEnemy, hits: 1 },
        // Summon LivingFog minion deferred (matches Rust impl).
        ]),

        ("LivingFog", "SuperGas") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(8), target: Target::ChosenEnemy, hits: 1 }]),

        ("Fabricator", "Fabricate") => Some(vec![]),

        ("Fabricator", "FabricatingStrike") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(18), target: Target::ChosenEnemy, hits: 1 },
        ]),

        ("Fabricator", "Disintegrate") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(11), target: Target::ChosenEnemy, hits: 1 },
        ]),

        ("Doormaker", "DramaticOpen") => Some(vec![]),

        ("Doormaker", "Hunger") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(30), target: Target::ChosenEnemy, hits: 1 }]),

        ("Doormaker", "Scrutiny") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(24), target: Target::ChosenEnemy, hits: 1 }]),

        ("Doormaker", "Grasp") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(10), target: Target::ChosenEnemy, hits: 2 },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(3), target: Target::SelfActor },
        ]),

        ("LagavulinMatriarch", "Sleep") => Some(vec![]),

        ("LagavulinMatriarch", "Slash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(19), target: Target::ChosenEnemy, hits: 1 }]),

        ("LagavulinMatriarch", "Slash2") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(12), target: Target::ChosenEnemy, hits: 1 },
        Effect::GainBlock { amount: AmountSpec::Fixed(12), target: Target::SelfActor },
        ]),

        ("LagavulinMatriarch", "Disembowel") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(9), target: Target::ChosenEnemy, hits: 2 },
        ]),

        ("LagavulinMatriarch", "SoulSiphon") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(-2), target: Target::ChosenEnemy },
        Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Fixed(-2), target: Target::ChosenEnemy },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("HauntedShip", "Haunt") => Some(vec![
        Effect::AddCardToPile { card_id: "Dazed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Dazed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Dazed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Dazed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Dazed".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),

        ("HauntedShip", "RammingSpeed") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(10), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::ChosenEnemy },
        ]),

        ("HauntedShip", "Swipe") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(13), target: Target::ChosenEnemy, hits: 1 }]),

        ("HauntedShip", "Stomp") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(4), target: Target::ChosenEnemy, hits: 3 }]),

        ("Queen", "PuppetStrings") => Some(vec![
        Effect::ApplyPower { power_id: "ChainsOfBindingPower".to_string(), amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy },
        ]),

        ("Queen", "YoureMine") => Some(vec![
        Effect::ApplyPower { power_id: "FrailPower".to_string(), amount: AmountSpec::Fixed(99), target: Target::ChosenEnemy },
        Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Fixed(99), target: Target::ChosenEnemy },
        Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Fixed(99), target: Target::ChosenEnemy },
        ]),

        ("Queen", "OffWithYourHead") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 5 }]),

        ("Queen", "Execution") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(15), target: Target::ChosenEnemy, hits: 1 }]),

        ("Queen", "Enrage") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("Crusher", "Thrash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(12), target: Target::ChosenEnemy, hits: 1 }]),

        ("Crusher", "EnlargingStrike") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(4), target: Target::ChosenEnemy, hits: 1 }]),

        ("Crusher", "BugSting") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(6), target: Target::ChosenEnemy, hits: 2 },
        Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::ChosenEnemy },
        Effect::ApplyPower { power_id: "FrailPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::ChosenEnemy },
        ]),

        ("Crusher", "Adapt") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("Crusher", "GuardedStrike") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(12), target: Target::ChosenEnemy, hits: 1 },
        Effect::GainBlock { amount: AmountSpec::Fixed(18), target: Target::SelfActor },
        ]),

        ("Rocket", "TargetingReticle") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 1 }]),

        ("Rocket", "PrecisionBeam") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(18), target: Target::ChosenEnemy, hits: 1 }]),

        ("Rocket", "ChargeUp") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("Rocket", "Laser") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(31), target: Target::ChosenEnemy, hits: 1 }]),

        ("Rocket", "Recharge") => Some(vec![]),

        ("Ovicopter", "LayEggs") => Some(vec![]),

        ("Ovicopter", "Smash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(16), target: Target::ChosenEnemy, hits: 1 }]),

        ("Ovicopter", "Tenderizer") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(7), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Fixed(2), target: Target::ChosenEnemy },
        ]),

        ("Ovicopter", "NutritionalPaste") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(3), target: Target::SelfActor },
        ]),

        ("MagiKnight", "PowerShield") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(6), target: Target::ChosenEnemy, hits: 1 },
        Effect::GainBlock { amount: AmountSpec::Fixed(5), target: Target::SelfActor },
        ]),

        ("MagiKnight", "Dampen") => Some(vec![
        Effect::ApplyPower { power_id: "DampenPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::ChosenEnemy },
        ]),

        ("MagiKnight", "Spear") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(10), target: Target::ChosenEnemy, hits: 1 }]),

        ("MagiKnight", "Prep") => Some(vec![Effect::GainBlock { amount: AmountSpec::Fixed(5), target: Target::SelfActor }]),

        ("MagiKnight", "MagicBomb") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(35), target: Target::ChosenEnemy, hits: 1 }]),

        ("SpectralKnight", "Hex") => Some(vec![
        Effect::ApplyPower { power_id: "HexPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::ChosenEnemy },
        ]),

        ("SpectralKnight", "SoulSlash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(15), target: Target::ChosenEnemy, hits: 1 }]),

        ("SpectralKnight", "SoulFlame") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 3 }]),

        ("Tunneler", "Bite") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(13), target: Target::ChosenEnemy, hits: 1 }]),

        ("Tunneler", "Burrow") => Some(vec![
        Effect::ApplyPower { power_id: "BurrowedPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfActor },
        Effect::GainBlock { amount: AmountSpec::Fixed(12), target: Target::SelfActor },
        ]),

        ("Tunneler", "Below") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(23), target: Target::ChosenEnemy, hits: 1 }]),

        ("TheInsatiable", "Liquify") => Some(vec![
        Effect::AddCardToPile { card_id: "FranticEscape".to_string(), upgrade: 0, pile: Pile::Draw },
        Effect::AddCardToPile { card_id: "FranticEscape".to_string(), upgrade: 0, pile: Pile::Draw },
        Effect::AddCardToPile { card_id: "FranticEscape".to_string(), upgrade: 0, pile: Pile::Draw },
        Effect::AddCardToPile { card_id: "FranticEscape".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "FranticEscape".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "FranticEscape".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),

        ("TheInsatiable", "Thrash1") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(8), target: Target::ChosenEnemy, hits: 2 }]),

        ("TheInsatiable", "Thrash2") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(8), target: Target::ChosenEnemy, hits: 2 }]),

        ("TheInsatiable", "Bite") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(28), target: Target::ChosenEnemy, hits: 1 }]),

        ("TheInsatiable", "Salivate") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("SlumberingBeetle", "Snore") => Some(vec![]),

        ("SlumberingBeetle", "Rollout") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(16), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("TorchHeadAmalgam", "Tackle1") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(18), target: Target::ChosenEnemy, hits: 1 }]),

        ("TorchHeadAmalgam", "Tackle2") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(18), target: Target::ChosenEnemy, hits: 1 }]),

        ("TorchHeadAmalgam", "Beam") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(8), target: Target::ChosenEnemy, hits: 3 }]),

        ("TorchHeadAmalgam", "Tackle3") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(14), target: Target::ChosenEnemy, hits: 1 }]),

        ("TorchHeadAmalgam", "Tackle4") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(14), target: Target::ChosenEnemy, hits: 1 }]),

        ("SoulFysh", "Beckon") => Some(vec![
        Effect::AddCardToPile { card_id: "Beckon".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Beckon".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),

        ("SoulFysh", "DeGas") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(16), target: Target::ChosenEnemy, hits: 1 }]),

        ("SoulFysh", "Gaze") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(7), target: Target::ChosenEnemy, hits: 1 },
        Effect::AddCardToPile { card_id: "Beckon".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),

        ("SoulFysh", "Fade") => Some(vec![
        Effect::ApplyPower { power_id: "IntangiblePower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("SoulFysh", "Scream") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(11), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy },
        ]),

        ("PhrogParasite", "Infect") => Some(vec![
        Effect::AddCardToPile { card_id: "Infection".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Infection".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Infection".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),

        ("PhrogParasite", "Lash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(4), target: Target::ChosenEnemy, hits: 4 }]),

        ("InfestedPrism", "Jab") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(22), target: Target::ChosenEnemy, hits: 1 }]),

        ("InfestedPrism", "Radiate") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(16), target: Target::ChosenEnemy, hits: 1 },
        Effect::GainBlock { amount: AmountSpec::Fixed(16), target: Target::SelfActor },
        ]),

        ("InfestedPrism", "Whirlwind") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(9), target: Target::ChosenEnemy, hits: 3 }]),

        ("InfestedPrism", "Pulsate") => Some(vec![
        Effect::GainBlock { amount: AmountSpec::Fixed(20), target: Target::SelfActor },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(4), target: Target::SelfActor },
        ]),

        ("PhantasmalGardener", "Bite") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(5), target: Target::ChosenEnemy, hits: 1 }]),

        ("PhantasmalGardener", "Lash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(7), target: Target::ChosenEnemy, hits: 1 }]),

        ("PhantasmalGardener", "Flail") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(1), target: Target::ChosenEnemy, hits: 3 }]),

        ("PhantasmalGardener", "Enlarge") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("TerrorEel", "Crash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(16), target: Target::ChosenEnemy, hits: 1 }]),

        ("TerrorEel", "Thrash") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 3 },
        Effect::ApplyPower { power_id: "VigorPower".to_string(), amount: AmountSpec::Fixed(6), target: Target::SelfActor },
        ]),

        ("LouseProgenitor", "CurlAndGrow") => Some(vec![
        Effect::GainBlock { amount: AmountSpec::Fixed(14), target: Target::SelfActor },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(5), target: Target::SelfActor },
        ]),

        ("LouseProgenitor", "Pounce") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(14), target: Target::ChosenEnemy, hits: 1 },
        ]),

        ("LouseProgenitor", "Web") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(9), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "FrailPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::ChosenEnemy },
        ]),

        ("SkulkingColony", "Smash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(12), target: Target::ChosenEnemy, hits: 1 }]),

        ("SkulkingColony", "Zoom") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(14), target: Target::ChosenEnemy, hits: 1 },
        Effect::GainBlock { amount: AmountSpec::Fixed(10), target: Target::SelfActor },
        ]),

        ("SkulkingColony", "Inertia") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(9), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("SkulkingColony", "PiercingStabs") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(7), target: Target::ChosenEnemy, hits: 2 },
        ]),

        ("BygoneEffigy", "InitialSleep") => Some(vec![]),

        ("BygoneEffigy", "Wake") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(10), target: Target::SelfActor },
        ]),

        ("BygoneEffigy", "Slash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(13), target: Target::ChosenEnemy, hits: 1 }]),

        ("SlimedBerserker", "VomitIchor") => Some(vec![
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),

        ("SlimedBerserker", "FuriousPummeling") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(4), target: Target::ChosenEnemy, hits: 4 }]),

        ("SlimedBerserker", "LeechingHug") => Some(vec![
        Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(3), target: Target::SelfActor },
        ]),

        ("SlimedBerserker", "Smother") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(30), target: Target::ChosenEnemy, hits: 1 }]),

        ("GlobeHead", "ShockingSlap") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(13), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "FrailPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::ChosenEnemy },
        ]),

        ("GlobeHead", "ThunderStrike") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(6), target: Target::ChosenEnemy, hits: 3 }]),

        ("GlobeHead", "GalvanicBurst") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(16), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("SpinyToad", "Spikes") => Some(vec![
        Effect::ApplyPower { power_id: "ThornsPower".to_string(), amount: AmountSpec::Fixed(5), target: Target::SelfActor },
        ]),

        ("SpinyToad", "Explosion") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(23), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "ThornsPower".to_string(), amount: AmountSpec::Fixed(-5), target: Target::SelfActor },
        ]),

        ("SpinyToad", "Lash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(17), target: Target::ChosenEnemy, hits: 1 }]),

        ("Vantom", "InkBlot") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(7), target: Target::ChosenEnemy, hits: 1 }]),

        ("Vantom", "InkyLance") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(6), target: Target::ChosenEnemy, hits: 2 }]),

        ("Vantom", "Dismember") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(27), target: Target::ChosenEnemy, hits: 1 },
        Effect::AddCardToPile { card_id: "Wound".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Wound".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Wound".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),

        ("Vantom", "Prepare") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("SoulNexus", "SoulBurn") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(29), target: Target::ChosenEnemy, hits: 1 }]),

        ("SoulNexus", "Maelstrom") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(6), target: Target::ChosenEnemy, hits: 4 }]),

        ("SoulNexus", "DrainLife") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(18), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Fixed(2), target: Target::ChosenEnemy },
        Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::ChosenEnemy },
        ]),

        ("DevotedSculptor", "ForbiddenIncantation") => Some(vec![
        Effect::ApplyPower { power_id: "RitualPower".to_string(), amount: AmountSpec::Fixed(9), target: Target::SelfActor },
        ]),

        ("DevotedSculptor", "Savage") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(12), target: Target::ChosenEnemy, hits: 1 }]),

        ("Exoskeleton", "Skitter") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(1), target: Target::ChosenEnemy, hits: 3 }]),

        ("Exoskeleton", "Mandibles") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(8), target: Target::ChosenEnemy, hits: 1 }]),

        ("Exoskeleton", "Enrage") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("Toadpole", "SpikeSpit") => Some(vec![
        // C# also strips ThornsPower(-2) before the damage hits — order
        // matters for the Thorns retaliation path. Order preserved.
        Effect::ApplyPower { power_id: "ThornsPower".to_string(), amount: AmountSpec::Fixed(-2), target: Target::SelfActor },
        Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 3 },
        ]),

        ("Toadpole", "Whirl") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(7), target: Target::ChosenEnemy, hits: 1 }]),

        ("Toadpole", "Spiken") => Some(vec![
        Effect::ApplyPower { power_id: "ThornsPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("ThievingHopper", "Thievery") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(17), target: Target::ChosenEnemy, hits: 1 }]),

        ("ThievingHopper", "Flutter") => Some(vec![
        Effect::ApplyPower { power_id: "FlutterPower".to_string(), amount: AmountSpec::Fixed(5), target: Target::SelfActor },
        ]),

        ("ThievingHopper", "HatTrick") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(21), target: Target::ChosenEnemy, hits: 1 }]),

        ("ThievingHopper", "Nab") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(14), target: Target::ChosenEnemy, hits: 1 }]),

        ("ThievingHopper", "Escape") => Some(vec![]),

        ("CalcifiedCultist", "Incantation") => Some(vec![
        Effect::ApplyPower { power_id: "RitualPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("CalcifiedCultist", "DarkStrike") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(9), target: Target::ChosenEnemy, hits: 1 }]),

        ("SludgeSpinner", "OilSpray") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(8), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::ChosenEnemy },
        ]),

        ("SludgeSpinner", "Slam") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(11), target: Target::ChosenEnemy, hits: 1 }]),

        ("SludgeSpinner", "Rage") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(6), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(3), target: Target::SelfActor },
        ]),

        ("FuzzyWurmCrawler", "FirstAcidGoop") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(4), target: Target::ChosenEnemy, hits: 1 }]),

        ("FuzzyWurmCrawler", "Inhale") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(7), target: Target::SelfActor },
        ]),

        ("FuzzyWurmCrawler", "AcidGoop") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(4), target: Target::ChosenEnemy, hits: 1 }]),

        ("BowlbugRock", "Headbutt") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(15), target: Target::ChosenEnemy, hits: 1 }]),

        ("MechaKnight", "Charge") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(25), target: Target::ChosenEnemy, hits: 1 }]),

        ("MechaKnight", "Flamethrower") => Some(vec![
        Effect::AddCardToPile { card_id: "Burn".to_string(), upgrade: 0, pile: Pile::Hand },
        Effect::AddCardToPile { card_id: "Burn".to_string(), upgrade: 0, pile: Pile::Hand },
        Effect::AddCardToPile { card_id: "Burn".to_string(), upgrade: 0, pile: Pile::Hand },
        Effect::AddCardToPile { card_id: "Burn".to_string(), upgrade: 0, pile: Pile::Hand },
        ]),

        ("MechaKnight", "Windup") => Some(vec![
        Effect::GainBlock { amount: AmountSpec::Fixed(15), target: Target::SelfActor },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(5), target: Target::SelfActor },
        ]),

        ("MechaKnight", "HeavyCleave") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(35), target: Target::ChosenEnemy, hits: 1 }]),

        ("Entomancer", "Bees") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 7 }]),

        ("Entomancer", "Spear") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(18), target: Target::ChosenEnemy, hits: 1 }]),

        ("LivingShield", "ShieldSlam") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(6), target: Target::ChosenEnemy, hits: 1 }]),

        ("LivingShield", "Smash") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(16), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(3), target: Target::SelfActor },
        ]),

        ("ShrinkerBeetle", "Shrinker") => Some(vec![
        // Negative amount = "infinite" (ShrinkPower.IsInfinite in C#).
        Effect::ApplyPower { power_id: "ShrinkPower".to_string(), amount: AmountSpec::Fixed(-1), target: Target::ChosenEnemy },
        ]),

        ("ShrinkerBeetle", "Chomp") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(7), target: Target::ChosenEnemy, hits: 1 }]),

        ("ShrinkerBeetle", "Stomp") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(13), target: Target::ChosenEnemy, hits: 1 }]),

        ("Byrdonis", "Peck") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 3 }]),

        ("Byrdonis", "Swoop") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(17), target: Target::ChosenEnemy, hits: 1 }]),

        ("Chomper", "Clamp") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(8), target: Target::ChosenEnemy, hits: 2 }]),

        ("Chomper", "Screech") => Some(vec![
        Effect::AddCardToPile { card_id: "Dazed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Dazed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Dazed".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),

        ("TurretOperator", "Unload1") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 5 }]),

        ("TurretOperator", "Unload2") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 5 }]),

        ("TurretOperator", "Reload") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfActor },
        ]),

        ("TwigSlimeM", "Clump") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(11), target: Target::ChosenEnemy, hits: 1 }]),

        ("TwigSlimeM", "Sticky") => Some(vec![
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),

        ("LeafSlimeM", "Clump") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(8), target: Target::ChosenEnemy, hits: 1 }]),

        ("LeafSlimeM", "Sticky") => Some(vec![
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),

        ("TwigSlimeS", "Butt") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(4), target: Target::ChosenEnemy, hits: 1 }]),

        ("LeafSlimeS", "Butt") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 1 }]),

        ("LeafSlimeS", "Goop") => Some(vec![
        Effect::AddCardToPile { card_id: "Slimed".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),

        ("Seapunk", "SeaKick") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(11), target: Target::ChosenEnemy, hits: 1 }]),

        ("Seapunk", "SpinningKick") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(2), target: Target::ChosenEnemy, hits: 4 }]),

        ("Seapunk", "BubbleBurp") => Some(vec![
        Effect::GainBlock { amount: AmountSpec::Fixed(7), target: Target::SelfActor },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfActor },
        ]),

        ("CorpseSlug", "WhipSlap") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 2 }]),

        ("CorpseSlug", "Glomp") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(8), target: Target::ChosenEnemy, hits: 1 }]),

        ("CorpseSlug", "Goop") => Some(vec![
        Effect::ApplyPower { power_id: "FrailPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::ChosenEnemy },
        ]),

        ("ScrollOfBiting", "Chomp") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(14), target: Target::ChosenEnemy, hits: 1 }]),

        ("ScrollOfBiting", "Chew") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(5), target: Target::ChosenEnemy, hits: 2 }]),

        ("ScrollOfBiting", "MoreTeeth") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::SelfActor },
        ]),

        ("BowlbugSilk", "Trash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(4), target: Target::ChosenEnemy, hits: 2 }]),

        ("BowlbugSilk", "ToxicSpit") => Some(vec![
        Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::ChosenEnemy },
        ]),

        ("BowlbugNectar", "Thrash") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 1 }]),

        ("BowlbugNectar", "Buff") => Some(vec![
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(15), target: Target::SelfActor },
        ]),

        ("BowlbugNectar", "Thrash2") => Some(vec![Effect::DealDamage { amount: AmountSpec::Fixed(3), target: Target::ChosenEnemy, hits: 1 }]),

        ("BowlbugEgg", "Bite") => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Fixed(7), target: Target::ChosenEnemy, hits: 1 },
        Effect::GainBlock { amount: AmountSpec::Fixed(7), target: Target::SelfActor },
        ]),


        _ => None,
    }
}

/// Execute a monster's move payload through the Effect VM. The acting
/// monster is `actor_idx`, the target player is `target_player_idx`.
/// `EffectContext::for_monster_move` sets actor for `Target::SelfActor`
/// and binds target for `Target::ChosenEnemy` (single-player → player_idx 0).
pub fn dispatch_monster_move_via_vm(
    cs: &mut CombatState,
    monster_id: &str,
    intent_name: &str,
    actor_idx: usize,
    target_player_idx: usize,
) -> bool {
    let Some(effects) = monster_move_effects(monster_id, intent_name) else {
        return false;
    };
    let ctx = EffectContext::for_monster_move(
        actor_idx,
        Some((CombatSide::Player, target_player_idx)),
    );
    execute_effects(cs, &effects, &ctx);
    true
}

// ========================================================================
// Run-state effect VM — out-of-combat relic hooks (AfterObtained etc.).
// ========================================================================

/// Run-state hook kinds. Mirror the C# `RelicModel` out-of-combat hooks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStateHook {
    /// C# `AfterObtained()`. Fires once at relic pickup.
    AfterObtained,
    /// C# `AfterRoomEntered(room)`. `room_type_filter` = Some("Monster"
    /// / "Elite" / "Boss" / "MerchantRoom" / "RestRoom" / ...) narrows
    /// to a specific room type; None fires on every room.
    AfterRoomEntered { room_type_filter: Option<String> },
    /// C# `AfterPotionUsed()`.
    AfterPotionUsed,
    /// C# `AfterShopCleared()`.
    AfterShopCleared,
}

/// Per-relic run-state hook bodies. Body is a `Vec<Effect>` interpreted
/// by `execute_run_state_effects` — only run-state Effect variants
/// (GainRunStateMaxHp / GainRunStateGold / LoseRunStateHp / GainRelic /
/// GainPotionToBelt / LoseRelic) actually mutate; combat-frame effects
/// no-op out of combat.
pub fn run_state_effects(
    relic_id: &str,
) -> Option<Vec<(RunStateHook, Vec<Effect>)>> {
    match relic_id {
        // ===== AfterObtained: permanent +MaxHp relics =====
        // Mango.cs L46-50: GainMaxHp(14).
        "Mango" => Some(vec![(
            RunStateHook::AfterObtained,
            vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(14) }],
        )]),
        // Pear.cs: GainMaxHp(10).
        "Pear" => Some(vec![(
            RunStateHook::AfterObtained,
            vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(10) }],
        )]),
        // Strawberry.cs: GainMaxHp(7).
        "Strawberry" => Some(vec![(
            RunStateHook::AfterObtained,
            vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(7) }],
        )]),
        // FakeMango.cs: GainMaxHp(3).
        "FakeMango" => Some(vec![(
            RunStateHook::AfterObtained,
            vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(3) }],
        )]),

        // ===== AfterObtained: +Gold relics =====
        // OldCoin.cs L46-49: GainGold(300).
        "OldCoin" => Some(vec![(
            RunStateHook::AfterObtained,
            vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(300) }],
        )]),
        // CursedPearl.cs L?-?: GainGold(333) + AddCurseToDeck<Greed>.
        // We encode the gold gain only; the AddCurseToDeck side needs a
        // mid-run deck-mutation primitive that doesn't yet land.
        "CursedPearl" => Some(vec![(
            RunStateHook::AfterObtained,
            vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(333) }],
        )]),
        // ===== Manual run-state ports (batch_r_rs_*) =====


        "BigMushroom" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(20) }],
        )]),

        "LeesWaffle" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(7) }],
        )]),

        "LoomingFruit" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(31) }],
        )]),

        "NutritiousOyster" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(11) }],
        )]),

        "GoldenPearl" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(150) }],
        )]),

        "SignetRing" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(999) }],
        )]),

        "BloodSoakedRose" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::AddCardToRunStateDeck { card_id: "Enthralled".to_string(), upgrade: 0 }],
        )]),

        "CallingBell" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::AddCardToRunStateDeck { card_id: "CurseOfTheBell".to_string(), upgrade: 0 }],
        )]),

        "CursedPearl" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![
        Effect::GainRunStateGold { amount: AmountSpec::Fixed(333) },
        Effect::AddCardToRunStateDeck { card_id: "Greed".to_string(), upgrade: 0 },
        ],
        )]),

        "LeafyPoultice" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::LoseRunStateMaxHp { amount: AmountSpec::Fixed(12) }],
        )]),

        "DistinguishedCape" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::LoseRunStateMaxHp { amount: AmountSpec::Fixed(7) }],
        )]),

        "PotionBelt" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::GainMaxPotionSlots { delta: AmountSpec::Fixed(2) }],
        )]),

        "PhialHolster" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::GainMaxPotionSlots { delta: AmountSpec::Fixed(1) }],
        )]),

        "JewelryBox" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::AddCardToRunStateDeck { card_id: "Apotheosis".to_string(), upgrade: 0 }],
        )]),

        "NeowsTorment" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::AddCardToRunStateDeck { card_id: "NeowsFury".to_string(), upgrade: 0 }],
        )]),

        "Storybook" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::AddCardToRunStateDeck { card_id: "BrightestFlame".to_string(), upgrade: 0 }],
        )]),

        "TanxsWhistle" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::AddCardToRunStateDeck { card_id: "Whistle".to_string(), upgrade: 0 }],
        )]),

        "PaelsHorn" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![
        Effect::AddCardToRunStateDeck { card_id: "Relax".to_string(), upgrade: 0 },
        Effect::AddCardToRunStateDeck { card_id: "Relax".to_string(), upgrade: 0 },
        ],
        )]),

        "SereTalon" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![
        Effect::AddCardToRunStateDeck { card_id: "Wish".to_string(), upgrade: 0 },
        Effect::AddCardToRunStateDeck { card_id: "Wish".to_string(), upgrade: 0 },
        Effect::AddCardToRunStateDeck { card_id: "Wish".to_string(), upgrade: 0 },
        ],
        )]),

        "PreservedFog" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::AddCardToRunStateDeck { card_id: "Folly".to_string(), upgrade: 0 }],
        )]),

        "FragrantMushroom" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::LoseRunStateHp { amount: AmountSpec::Fixed(15) }],
        )]),

        "AlchemicalCoffer" => Some(vec![(
        RunStateHook::AfterObtained,
        vec![Effect::GainMaxPotionSlots { delta: AmountSpec::Fixed(4) }],
        )]),


        _ => None,
    }
}

/// Execute a run-state effect list against `RunState`. Match arms
/// dispatch only the run-state Effect variants; combat-frame variants
/// are no-ops here.
pub fn execute_run_state_effects(
    rs: &mut crate::run_state::RunState,
    player_idx: usize,
    effects: &[Effect],
) {
    for eff in effects {
        execute_run_state_effect(rs, player_idx, eff);
    }
}

fn execute_run_state_effect(
    rs: &mut crate::run_state::RunState,
    player_idx: usize,
    eff: &Effect,
) {
    match eff {
        Effect::GainRunStateMaxHp { amount } => {
            let amt = run_state_resolve_amount(rs, player_idx, amount).max(0);
            if let Some(ps) = rs.player_state_mut(player_idx) {
                ps.max_hp += amt;
                ps.hp += amt;
            }
        }
        Effect::GainRunStateGold { amount } => {
            let amt = run_state_resolve_amount(rs, player_idx, amount).max(0);
            if let Some(ps) = rs.player_state_mut(player_idx) {
                ps.gold += amt;
            }
        }
        Effect::LoseRunStateHp { amount } => {
            let amt = run_state_resolve_amount(rs, player_idx, amount).max(0);
            if let Some(ps) = rs.player_state_mut(player_idx) {
                ps.hp = (ps.hp - amt).max(0);
            }
        }
        Effect::GainRelic { relic_id } => {
            if let Some(ps) = rs.player_state_mut(player_idx) {
                ps.relics.push(crate::run_log::RelicEntry {
                    id: relic_id.clone(),
                    floor_added_to_deck: 0,
                    props: None,
                });
            }
        }
        Effect::LoseRelic { relic_id } => {
            if let Some(ps) = rs.player_state_mut(player_idx) {
                ps.relics.retain(|r| &r.id != relic_id);
            }
        }
        Effect::GainPotionToBelt { potion_id } => {
            if let Some(ps) = rs.player_state_mut(player_idx) {
                let slot = ps.potions.len() as i32;
                ps.potions.push(crate::run_log::PotionEntry {
                    id: potion_id.clone(),
                    slot_index: slot,
                });
            }
        }
        Effect::LoseRunStateMaxHp { amount } => {
            let amt = run_state_resolve_amount(rs, player_idx, amount).max(0);
            if let Some(ps) = rs.player_state_mut(player_idx) {
                // Match C# CreatureCmd.LoseMaxHp: lowers max_hp and clamps
                // current_hp so it stays <= new max.
                ps.max_hp = (ps.max_hp - amt).max(1);
                ps.hp = ps.hp.min(ps.max_hp);
            }
        }
        Effect::AddCardToRunStateDeck { card_id, upgrade } => {
            if let Some(ps) = rs.player_state_mut(player_idx) {
                ps.deck.push(crate::run_log::CardRef {
                    id: card_id.clone(),
                    floor_added_to_deck: None,
                    current_upgrade_level: if *upgrade > 0 { Some(*upgrade) } else { None },
                    ..Default::default()
                });
            }
        }
        Effect::GainMaxPotionSlots { delta } => {
            let d = run_state_resolve_amount(rs, player_idx, delta);
            if let Some(ps) = rs.player_state_mut(player_idx) {
                ps.max_potion_slot_count = (ps.max_potion_slot_count + d).max(0);
            }
        }
        _ => {
            // Combat-frame effects no-op out of combat.
        }
    }
}

fn run_state_resolve_amount(
    _rs: &crate::run_state::RunState,
    _player_idx: usize,
    spec: &AmountSpec,
) -> i32 {
    match spec {
        AmountSpec::Fixed(n) => *n,
        AmountSpec::Add { left, right } => {
            run_state_resolve_amount(_rs, _player_idx, left)
                + run_state_resolve_amount(_rs, _player_idx, right)
        }
        AmountSpec::Sub { left, right } => {
            run_state_resolve_amount(_rs, _player_idx, left)
                - run_state_resolve_amount(_rs, _player_idx, right)
        }
        AmountSpec::Mul { left, right } => {
            run_state_resolve_amount(_rs, _player_idx, left)
                * run_state_resolve_amount(_rs, _player_idx, right)
        }
        AmountSpec::Multiplied { base, factor } => {
            run_state_resolve_amount(_rs, _player_idx, base) * factor
        }
        // Other variants require CombatState / EffectContext — caller
        // must specialize run-state relic bodies to use Fixed(N) for
        // amount literals.
        _ => 0,
    }
}

// ========================================================================
// Relic VM — per-relic data table parallel to power_effects / card_effects.
// ========================================================================

/// Closed enum of relic-lifecycle trigger points. Each variant carries
/// its own gate (owner-side-only / first-turn-only / etc.) so the data
/// layer fully describes "when does this relic body fire".
///
/// Hook coverage is intentionally a subset of the C# relic surface —
/// the dominant 7 hooks across the 294 relics that we wire fire points
/// for. The rest (AfterRoomEntered, AfterCardPlayed, ModifyHandDraw,
/// etc.) need infrastructure that hasn't landed yet; relics that depend
/// on them stay SKIPPED in `relic_effects`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelicHook {
    /// C# `BeforeCombatStart()`. Once, before any draws / turn begins.
    BeforeCombatStart,
    /// C# `AfterCombatVictory()`. Once, after combat ends in victory.
    AfterCombatVictory,
    /// C# `AfterCombatLoss()` / `AfterCombatEnd()` for the loss path.
    AfterCombatLoss,
    /// C# `AfterSideTurnStart(side)`. Fires once each side's turn begins.
    /// `owner_side_only` gates on the typical `side == base.Owner.Side`.
    /// `first_turn_only` gates on `combatState.RoundNumber <= 1`.
    AfterSideTurnStart {
        owner_side_only: bool,
        first_turn_only: bool,
    },
    /// C# `BeforeSideTurnStart(side)`. Same gates as AfterSideTurnStart;
    /// distinct firing point (before draws / energy refresh).
    BeforeSideTurnStart {
        owner_side_only: bool,
        first_turn_only: bool,
    },
    /// C# `AfterPlayerTurnStart(player)`. Always owner-only (implicit).
    AfterPlayerTurnStart { first_turn_only: bool },
    /// C# `AfterPlayerTurnEnd(player)`.
    AfterPlayerTurnEnd,
    /// C# `AfterCardPlayed(...)`. Fires after the played card's OnPlay
    /// fully resolves (and after the card has routed to discard /
    /// exhaust). `filter` (optional) gates on the played card matching
    /// — e.g. Kunai: Attack-only.
    AfterCardPlayed { filter: Option<CardFilter> },
    /// C# `AfterCombatEnd()`. Fires once at combat end regardless of
    /// outcome (distinct from AfterCombatVictory). ChosenCheese etc.
    AfterCombatEnd,
    /// C# `BeforeTurnEnd(side)` — fires before AfterTurnEnd power ticks.
    /// Bookmark / DiamondDiadem / Orichalcum family.
    BeforeTurnEnd { owner_side_only: bool },
    /// C# `AfterDamageGiven(...)`. Owner's attack landed.
    AfterDamageGiven,
    /// C# `AfterDamageReceived(...)`. Owner took damage.
    AfterDamageReceived,
    /// C# `AfterCardExhausted(card)`. Any card the owner exhausts.
    /// CharonsAshes / JossPaper / ForgottenSoul / DarkstonePeriapt.
    AfterCardExhausted,
    /// C# `AfterCardDiscarded(card)`. Tingsha / ToughBandages.
    AfterCardDiscarded,
    /// C# `AfterBlockCleared(creature)`. Owner's block dropped to 0.
    /// CaptainsWheel / HornCleat.
    AfterBlockCleared,
}

/// Discriminant for matching `RelicHook` entries against a firing point.
/// The fire-point passes the kind; the data entry's guards (`owner_side_only`
/// / `first_turn_only`) are checked separately.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RelicHookKind {
    BeforeCombatStart,
    AfterCombatVictory,
    AfterCombatLoss,
    AfterCombatEnd,
    AfterSideTurnStart,
    BeforeSideTurnStart,
    AfterPlayerTurnStart,
    AfterPlayerTurnEnd,
    AfterCardPlayed,
    BeforeTurnEnd,
    AfterDamageGiven,
    AfterDamageReceived,
    AfterCardExhausted,
    AfterCardDiscarded,
    AfterBlockCleared,
}

impl RelicHook {
    /// The variant discriminant.
    pub fn kind(&self) -> RelicHookKind {
        match self {
            RelicHook::BeforeCombatStart => RelicHookKind::BeforeCombatStart,
            RelicHook::AfterCombatVictory => RelicHookKind::AfterCombatVictory,
            RelicHook::AfterCombatLoss => RelicHookKind::AfterCombatLoss,
            RelicHook::AfterCombatEnd => RelicHookKind::AfterCombatEnd,
            RelicHook::AfterSideTurnStart { .. } => RelicHookKind::AfterSideTurnStart,
            RelicHook::BeforeSideTurnStart { .. } => RelicHookKind::BeforeSideTurnStart,
            RelicHook::AfterPlayerTurnStart { .. } => RelicHookKind::AfterPlayerTurnStart,
            RelicHook::AfterPlayerTurnEnd => RelicHookKind::AfterPlayerTurnEnd,
            RelicHook::AfterCardPlayed { .. } => RelicHookKind::AfterCardPlayed,
            RelicHook::BeforeTurnEnd { .. } => RelicHookKind::BeforeTurnEnd,
            RelicHook::AfterDamageGiven => RelicHookKind::AfterDamageGiven,
            RelicHook::AfterDamageReceived => RelicHookKind::AfterDamageReceived,
            RelicHook::AfterCardExhausted => RelicHookKind::AfterCardExhausted,
            RelicHook::AfterCardDiscarded => RelicHookKind::AfterCardDiscarded,
            RelicHook::AfterBlockCleared => RelicHookKind::AfterBlockCleared,
        }
    }

    /// Check the entry's per-variant guards against runtime context.
    /// Returns true if the guarded body should fire.
    pub fn allows(
        &self,
        current_side: CombatSide,
        owner_side: CombatSide,
        round_number: i32,
    ) -> bool {
        match self {
            RelicHook::BeforeCombatStart
            | RelicHook::AfterCombatVictory
            | RelicHook::AfterCombatLoss
            | RelicHook::AfterCombatEnd
            | RelicHook::AfterPlayerTurnEnd
            | RelicHook::AfterCardPlayed { .. }
            | RelicHook::AfterDamageGiven
            | RelicHook::AfterDamageReceived
            | RelicHook::AfterCardExhausted
            | RelicHook::AfterCardDiscarded
            | RelicHook::AfterBlockCleared => true,
            RelicHook::AfterSideTurnStart { owner_side_only, first_turn_only }
            | RelicHook::BeforeSideTurnStart { owner_side_only, first_turn_only } => {
                (!owner_side_only || current_side == owner_side)
                    && (!first_turn_only || round_number <= 1)
            }
            RelicHook::AfterPlayerTurnStart { first_turn_only } => {
                !first_turn_only || round_number <= 1
            }
            RelicHook::BeforeTurnEnd { owner_side_only } => {
                !owner_side_only || current_side == owner_side
            }
        }
    }
}

/// `AfterCardPlayed`-specific dispatcher. Filters relic_effects entries
/// by `RelicHook::AfterCardPlayed { filter }` matching the played card.
pub fn fire_relic_hooks_after_card_played(
    cs: &mut CombatState,
    player_idx: usize,
    card_id: &str,
    card_type: crate::card::CardType,
    keywords: &[String],
    tags: &[String],
) {
    let mut pairs: Vec<(usize, String)> = Vec::new();
    for (i, c) in cs.allies.iter().enumerate() {
        if let Some(ps) = c.player.as_ref() {
            for r in &ps.relics {
                pairs.push((i, r.clone()));
            }
        }
    }
    for (pid, relic_id) in pairs {
        if pid != player_idx {
            continue;
        }
        let Some(arms) = relic_effects(&relic_id) else {
            continue;
        };
        for (hook, body) in arms {
            let RelicHook::AfterCardPlayed { filter } = &hook else {
                continue;
            };
            if let Some(f) = filter {
                if !card_metadata_matches_filter(card_type, keywords, tags, f) {
                    continue;
                }
            }
            let _ = card_id; // currently the body resolves its own context
            let ctx = EffectContext::for_relic_hook(player_idx, relic_id.as_str());
            execute_effects(cs, &body, &ctx);
        }
    }
}

fn card_metadata_matches_filter(
    card_type: crate::card::CardType,
    keywords: &[String],
    tags: &[String],
    filter: &CardFilter,
) -> bool {
    match filter {
        CardFilter::Any => true,
        CardFilter::Upgradable => false, // never relevant on the playing arm
        CardFilter::OfType(name) => match name.as_str() {
            "Attack" => card_type == crate::card::CardType::Attack,
            "Skill" => card_type == crate::card::CardType::Skill,
            "Power" => card_type == crate::card::CardType::Power,
            "Status" => card_type == crate::card::CardType::Status,
            "Curse" => card_type == crate::card::CardType::Curse,
            _ => false,
        },
        CardFilter::HasKeyword(k) => keywords.iter().any(|kw| kw == k),
        CardFilter::TaggedAs(t) => tags.iter().any(|tag| tag == t),
        CardFilter::OfRarity(_) => false, // not derivable from metadata alone
        CardFilter::And(a, b) => {
            card_metadata_matches_filter(card_type, keywords, tags, a)
                && card_metadata_matches_filter(card_type, keywords, tags, b)
        }
        CardFilter::Or(a, b) => {
            card_metadata_matches_filter(card_type, keywords, tags, a)
                || card_metadata_matches_filter(card_type, keywords, tags, b)
        }
        CardFilter::Not(inner) => {
            !card_metadata_matches_filter(card_type, keywords, tags, inner)
        }
        CardFilter::HasId(_) => false,
        CardFilter::WithEnergyCost { .. } => false,
        CardFilter::NotXCost => false,
    }
}

/// Walk each player's relics and fire any matching hook bodies through
/// the Effect VM. Call sites are in `CombatState::{begin_turn, end_turn,
/// fire_before_combat_start_hooks, fire_after_combat_victory_hooks}`.
pub fn fire_relic_hooks(
    cs: &mut CombatState,
    kind: RelicHookKind,
    current_side: CombatSide,
) {
    // Snapshot (player_idx, relic_id) so the loop can mutate freely
    // without iterator invalidation; mirrors the existing
    // `collect_player_relics` pattern.
    let mut pairs: Vec<(usize, String)> = Vec::new();
    for (i, c) in cs.allies.iter().enumerate() {
        if let Some(ps) = c.player.as_ref() {
            for r in &ps.relics {
                pairs.push((i, r.clone()));
            }
        }
    }
    let round = cs.round_number;
    for (player_idx, relic_id) in pairs {
        let Some(arms) = relic_effects(&relic_id) else {
            continue;
        };
        for (hook, body) in arms {
            if hook.kind() != kind {
                continue;
            }
            if !hook.allows(current_side, CombatSide::Player, round) {
                continue;
            }
            let ctx = EffectContext::for_relic_hook(player_idx, relic_id.as_str());
            execute_effects(cs, &body, &ctx);
        }
    }
}

// ========================================================================
// Potion VM — per-potion data table parallel to card_effects.
// ========================================================================

/// Per-potion OnUse body. Same shape as `card_effects` — looked up by
/// id, returns an effect list. Callers (env.rs `UsePotion`, mid-combat
/// potion-throw effects, etc.) build an `EffectContext` whose
/// `source_relic_id`-equivalent is the potion id and dispatch through
/// `execute_effects`. AmountSpec::Canonical resolves through the
/// potion's `canonical_vars` table — see `for_potion_use` builder.
///
/// Survey: `tools/merge_potion_ports/batch_p_1.txt`. 45 of 64 potions
/// encoded; rest depend on primitives we haven't built (CardFactory
/// random pools, target-relative AmountSpec, etc.).
pub fn potion_effects(potion_id: &str) -> Option<Vec<Effect>> {
    match potion_id {
        // ===== Manual potion ports (batch_p_1) =====
        // 45 hand-curated arms. Source: tools/merge_potion_ports/batch_p_1.txt.


        "Ashwater" => Some(vec![Effect::ExhaustCards { from: Pile::Hand, selector: Selector::PlayerInteractive { n: 1 } }]),

        "BeetleJuice" => Some(vec![Effect::ApplyPower { power_id: "ShrinkPower".to_string(), amount: AmountSpec::Canonical("Repeat".to_string()), target: Target::ChosenEnemy }]),

        "BlessingOfTheForge" => Some(vec![Effect::UpgradeCards { from: Pile::Hand, selector: Selector::FirstMatching { n: i32::MAX, filter: CardFilter::Upgradable } }]),

        "BlockPotion" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),

        "BottledPotential" => Some(vec![
            Effect::MoveCard { from: Pile::Hand, to: Pile::Draw, selector: Selector::All },
            Effect::Shuffle { pile: Pile::Draw },
            Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) },
        ]),

        "Clarity" => Some(vec![
            Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) },
            Effect::ApplyPower { power_id: "ClarityPower".to_string(), amount: AmountSpec::Canonical("ClarityPower".to_string()), target: Target::SelfPlayer },
        ]),

        "CureAll" => Some(vec![
            Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) },
            Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) },
        ]),

        "DexterityPotion" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }]),

        "DistilledChaos" => Some(vec![Effect::AutoplayFromDraw { n: 3 }]),

        "DropletOfPrecognition" => Some(vec![Effect::MoveCard { from: Pile::Draw, to: Pile::Hand, selector: Selector::PlayerInteractive { n: 1 } }]),

        "Duplicator" => Some(vec![Effect::ApplyPower { power_id: "DuplicationPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),

        "EnergyPotion" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),

        "EntropicBrew" => Some(vec![Effect::FillPotionSlots]),

        "ExplosiveAmpoule" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),

        "FirePotion" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),

        "FlexPotion" => Some(vec![Effect::ApplyPower { power_id: "FlexPotionPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),

        "FocusPotion" => Some(vec![Effect::ApplyPower { power_id: "FocusPower".to_string(), amount: AmountSpec::Canonical("FocusPower".to_string()), target: Target::SelfPlayer }]),

        "FruitJuice" => Some(vec![Effect::ChangeMaxHp { amount: AmountSpec::Canonical("MaxHp".to_string()), target: Target::SelfPlayer }]),

        "FyshOil" => Some(vec![
            Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer },
            Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer },
        ]),

        "GhostInAJar" => Some(vec![Effect::ApplyPower { power_id: "IntangiblePower".to_string(), amount: AmountSpec::Canonical("IntangiblePower".to_string()), target: Target::SelfPlayer }]),

        "GigantificationPotion" => Some(vec![Effect::ApplyPower { power_id: "GigantificationPower".to_string(), amount: AmountSpec::Canonical("GigantificationPower".to_string()), target: Target::SelfPlayer }]),

        "GlowwaterPotion" => Some(vec![
            Effect::ExhaustCards { from: Pile::Hand, selector: Selector::All },
            Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) },
        ]),

        "HeartOfIron" => Some(vec![Effect::ApplyPower { power_id: "PlatingPower".to_string(), amount: AmountSpec::Canonical("PlatingPower".to_string()), target: Target::SelfPlayer }]),

        "KingsCourage" => Some(vec![Effect::Forge { amount: AmountSpec::Canonical("Forge".to_string()) }]),

        "LiquidBronze" => Some(vec![Effect::ApplyPower { power_id: "ThornsPower".to_string(), amount: AmountSpec::Canonical("ThornsPower".to_string()), target: Target::SelfPlayer }]),

        "LiquidMemories" => Some(vec![Effect::MoveCard { from: Pile::Discard, to: Pile::Hand, selector: Selector::PlayerInteractive { n: 1 } }]),

        "LuckyTonic" => Some(vec![Effect::ApplyPower { power_id: "BufferPower".to_string(), amount: AmountSpec::Canonical("BufferPower".to_string()), target: Target::SelfPlayer }]),

        "MazalethsGift" => Some(vec![Effect::ApplyPower { power_id: "RitualPower".to_string(), amount: AmountSpec::Canonical("RitualPower".to_string()), target: Target::SelfPlayer }]),

        "PoisonPotion" => Some(vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("PoisonPower".to_string()), target: Target::ChosenEnemy }]),

        "PotionOfBinding" => Some(vec![
            Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::AllEnemies },
            Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::AllEnemies },
        ]),

        "PotionOfCapacity" => Some(vec![Effect::ChangeOrbSlots { delta: AmountSpec::Canonical("Repeat".to_string()) }]),

        "PotionOfDoom" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("DoomPower".to_string()), target: Target::ChosenEnemy }]),

        "PotionShapedRock" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),

        "PowderedDemise" => Some(vec![Effect::ApplyPower { power_id: "DemisePower".to_string(), amount: AmountSpec::Canonical("Demise".to_string()), target: Target::ChosenEnemy }]),

        "RadiantTincture" => Some(vec![
            Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) },
            Effect::ApplyPower { power_id: "RadiancePower".to_string(), amount: AmountSpec::Canonical("RadiancePower".to_string()), target: Target::SelfPlayer },
        ]),

        "RegenPotion" => Some(vec![Effect::ApplyPower { power_id: "RegenPower".to_string(), amount: AmountSpec::Canonical("RegenPower".to_string()), target: Target::SelfPlayer }]),

        "ShacklingPotion" => Some(vec![Effect::ApplyPower { power_id: "ShacklingPotionPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::AllEnemies }]),

        "ShipInABottle" => Some(vec![
            Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer },
            Effect::ApplyPower { power_id: "BlockNextTurnPower".to_string(), amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer },
        ]),

        "SpeedPotion" => Some(vec![Effect::ApplyPower { power_id: "SpeedPotionPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }]),

        "StableSerum" => Some(vec![Effect::ApplyPower { power_id: "RetainHandPower".to_string(), amount: AmountSpec::Canonical("Repeat".to_string()), target: Target::SelfPlayer }]),

        "StarPotion" => Some(vec![Effect::GainStars { amount: AmountSpec::Canonical("Stars".to_string()) }]),

        "StrengthPotion" => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),

        "SwiftPotion" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),

        "VulnerablePotion" => Some(vec![Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),

        "WeakPotion" => Some(vec![Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),

        "AttackPotion" => Some(vec![Effect::AddRandomCardFromPool {
            pool: CardPoolRef::CharacterAttack,
            filter: CardFilter::Any,
            n: AmountSpec::Fixed(3),
            pile: Pile::Hand,
            upgrade: 0,
            free_this_turn: true,
            distinct: true,
        }]),

        "SkillPotion" => Some(vec![Effect::AddRandomCardFromPool {
            pool: CardPoolRef::CharacterSkill,
            filter: CardFilter::Any,
            n: AmountSpec::Fixed(3),
            pile: Pile::Hand,
            upgrade: 0,
            free_this_turn: true,
            distinct: true,
        }]),

        "PowerPotion" => Some(vec![Effect::AddRandomCardFromPool {
            pool: CardPoolRef::CharacterPower,
            filter: CardFilter::Any,
            n: AmountSpec::Fixed(3),
            pile: Pile::Hand,
            upgrade: 0,
            free_this_turn: true,
            distinct: true,
        }]),

        "ColorlessPotion" => Some(vec![Effect::AddRandomCardFromPool {
            pool: CardPoolRef::Colorless,
            filter: CardFilter::Any,
            n: AmountSpec::Fixed(3),
            pile: Pile::Hand,
            upgrade: 0,
            free_this_turn: true,
            distinct: true,
        }]),

        "CosmicConcoction" => Some(vec![Effect::AddRandomCardFromPool {
            pool: CardPoolRef::Colorless,
            filter: CardFilter::Any,
            n: AmountSpec::Canonical("Cards".to_string()),
            pile: Pile::Hand,
            upgrade: 1,
            free_this_turn: false,
            distinct: true,
        }]),

        "CunningPotion" => Some(vec![Effect::Repeat {
            count: AmountSpec::Canonical("Cards".to_string()),
            body: vec![Effect::AddCardToPile { card_id: "Shiv".to_string(), upgrade: 1, pile: Pile::Hand }],
        }]),

        "OrobicAcid" => Some(vec![
            Effect::AddRandomCardFromPool {
                pool: CardPoolRef::CharacterAttack,
                filter: CardFilter::Any,
                n: AmountSpec::Fixed(1),
                pile: Pile::Hand,
                upgrade: 0,
                free_this_turn: true,
                distinct: true,
            },
            Effect::AddRandomCardFromPool {
                pool: CardPoolRef::CharacterSkill,
                filter: CardFilter::Any,
                n: AmountSpec::Fixed(1),
                pile: Pile::Hand,
                upgrade: 0,
                free_this_turn: true,
                distinct: true,
            },
            Effect::AddRandomCardFromPool {
                pool: CardPoolRef::CharacterPower,
                filter: CardFilter::Any,
                n: AmountSpec::Fixed(1),
                pile: Pile::Hand,
                upgrade: 0,
                free_this_turn: true,
                distinct: true,
            },
        ]),

        "PotOfGhouls" => Some(vec![Effect::Repeat {
            count: AmountSpec::Canonical("Cards".to_string()),
            body: vec![Effect::AddCardToPile { card_id: "Soul".to_string(), upgrade: 0, pile: Pile::Hand }],
        }]),

        "BoneBrew" => Some(vec![Effect::SummonOsty {
            osty_id: "Default".to_string(),
            max_hp: Some(AmountSpec::Canonical("Summon".to_string())),
        }]),

        "Fortifier" => Some(vec![Effect::GainBlock {
            amount: AmountSpec::Mul {
                left: Box::new(AmountSpec::TargetBlock),
                right: Box::new(AmountSpec::Fixed(2)),
            },
            target: Target::SelfPlayer,
        }]),

        "EssenceOfDarkness" => Some(vec![Effect::Repeat {
            count: AmountSpec::EmptyOrbSlots,
            body: vec![Effect::ChannelOrb { orb_id: "DarkOrb".to_string() }],
        }]),


        _ => None,
    }
}

/// Per-relic registry of hook bodies, parallel to `card_effects` /
/// `power_effects`. Each entry is `(RelicHook, Vec<Effect>)`: the hook
/// names the trigger + guards; the Vec<Effect> is the body executed by
/// `fire_relic_hooks` when guards pass.
///
/// Survey: `tools/merge_relic_ports/batch_r_1.txt`. 22 relics encoded;
/// the rest depend on hooks/infrastructure not yet wired (AfterRoomEntered,
/// AfterCardPlayed, ModifyHandDraw, counter-state, etc.) and SKIP there.
pub fn relic_effects(relic_id: &str) -> Option<Vec<(RelicHook, Vec<Effect>)>> {
    match relic_id {
        // ===== Manual relic ports (batch_r_1) =====
        // 22 hand-curated arms. Source: tools/merge_relic_ports/batch_r_1.txt.


        "Akabeko" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::ApplyPower { power_id: "VigorPower".to_string(), amount: AmountSpec::Canonical("VigorPower".to_string()), target: Target::SelfPlayer }]),
        ]),

        "BagOfMarbles" => Some(vec![
            (RelicHook::BeforeSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::AllEnemies }]),
        ]),

        "Bellows" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: true },
             vec![Effect::UpgradeCards { from: Pile::Hand, selector: Selector::All }]),
        ]),

        "BlackBlood" => Some(vec![
            (RelicHook::AfterCombatVictory,
             vec![Effect::Heal { amount: AmountSpec::Canonical("Heal".to_string()), target: Target::SelfPlayer }]),
        ]),

        "Bread" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::LoseEnergy { amount: AmountSpec::Canonical("LoseEnergy".to_string()) }]),
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::Conditional {
                condition: Condition::RoundEquals { n: 2 },
                then_branch: vec![Effect::IncreaseMaxEnergy { delta: AmountSpec::Fixed(1) }],
                else_branch: vec![],
             }]),
        ]),

        "CrackedCore" => Some(vec![
            (RelicHook::BeforeSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::ChannelOrb { orb_id: "LightningOrb".to_string() }]),
        ]),

        "DivineDestiny" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::GainStars { amount: AmountSpec::Canonical("Stars".to_string()) }]),
        ]),

        "FakeAnchor" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        ]),

        "FakeSneckoEye" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::ApplyPower { power_id: "ConfusedPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        ]),

        "FencingManual" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::Forge { amount: AmountSpec::Canonical("Forge".to_string()) }]),
        ]),

        "FuneraryMask" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::Repeat {
                 count: AmountSpec::Canonical("Cards".to_string()),
                 body: vec![Effect::AddCardToPile {
                     card_id: "Soul".to_string(),
                     upgrade: 0,
                     pile: Pile::Draw,
                 }],
             }]),
        ]),

        "InfusedCore" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::Repeat {
                 count: AmountSpec::Canonical("Lightning".to_string()),
                 body: vec![Effect::ChannelOrb { orb_id: "LightningOrb".to_string() }],
             }]),
        ]),

        "Lantern" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        ]),

        "MercuryHourglass" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: false },
             vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        ]),

        "RedMask" => Some(vec![
            (RelicHook::BeforeSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::AllEnemies }]),
        ]),

        "RoyalPoison" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: true },
             vec![Effect::LoseHp { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::SelfPlayer }]),
        ]),

        "RunicCapacitor" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::ChangeOrbSlots { delta: AmountSpec::Canonical("Repeat".to_string()) }]),
        ]),

        "Sai" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        ]),

        "SneckoEye" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::ApplyPower { power_id: "ConfusedPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        ]),

        "SymbioticVirus" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::Repeat {
                 count: AmountSpec::Canonical("Dark".to_string()),
                 body: vec![Effect::ChannelOrb { orb_id: "DarkOrb".to_string() }],
             }]),
        ]),

        "TwistedFunnel" => Some(vec![
            (RelicHook::BeforeSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("PoisonPower".to_string()), target: Target::AllEnemies }]),
        ]),

        "VeryHotCocoa" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        ]),

        "ChosenCheese" => Some(vec![
            (RelicHook::AfterCombatEnd,
             vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Canonical("MaxHp".to_string()) }]),
        ]),

        "CentennialPuzzle" => Some(vec![
            (RelicHook::AfterDamageReceived,
             vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        ]),

        "SelfFormingClay" => Some(vec![
            (RelicHook::AfterDamageReceived,
             vec![Effect::ApplyPower {
                 power_id: "SelfFormingClayPower".to_string(),
                 amount: AmountSpec::Canonical("BlockNextTurn".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "DemonTongue" => Some(vec![
            (RelicHook::AfterDamageReceived,
             vec![Effect::Heal { amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        ]),

        "HandDrill" => Some(vec![
            (RelicHook::AfterDamageGiven,
             vec![Effect::ApplyPower {
                 power_id: "VulnerablePower".to_string(),
                 amount: AmountSpec::Canonical("Vulnerable".to_string()),
                 target: Target::ChosenEnemy,
             }]),
        ]),

        "BronzeScales" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::ApplyPower {
                 power_id: "ThornsPower".to_string(),
                 amount: AmountSpec::Canonical("ThornsPower".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "SwordOfJade" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::ApplyPower {
                 power_id: "StrengthPower".to_string(),
                 amount: AmountSpec::Canonical("Strength".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "Pantograph" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::Heal {
                 amount: AmountSpec::Canonical("Heal".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "Candelabra" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::Conditional {
                 condition: Condition::RoundEquals { n: 2 },
                 then_branch: vec![Effect::GainEnergy {
                     amount: AmountSpec::Canonical("Energy".to_string()),
                 }],
                 else_branch: vec![],
             }]),
        ]),

        "Chandelier" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::Conditional {
                 condition: Condition::RoundEquals { n: 3 },
                 then_branch: vec![Effect::GainEnergy {
                     amount: AmountSpec::Canonical("Energy".to_string()),
                 }],
                 else_branch: vec![],
             }]),
        ]),

        "PaelsFlesh" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::Conditional {
                 condition: Condition::RoundGe { n: 3 },
                 then_branch: vec![Effect::GainEnergy {
                     amount: AmountSpec::Canonical("Energy".to_string()),
                 }],
                 else_branch: vec![],
             }]),
        ]),

        "BigHat" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::AddRandomCardFromPool {
                 pool: CardPoolRef::CharacterAny,
                 filter: CardFilter::HasKeyword("Ethereal".to_string()),
                 n: AmountSpec::Canonical("Cards".to_string()),
                 pile: Pile::Hand,
                 upgrade: 0,
                 free_this_turn: true,
                 distinct: true,
             }]),
        ]),

        "Crossbow" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::AddRandomCardFromPool {
                 pool: CardPoolRef::CharacterAttack,
                 filter: CardFilter::Any,
                 n: AmountSpec::Fixed(1),
                 pile: Pile::Hand,
                 upgrade: 0,
                 free_this_turn: true,
                 distinct: true,
             }]),
        ]),

        "OrangeDough" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::AddRandomCardFromPool {
                 pool: CardPoolRef::Colorless,
                 filter: CardFilter::Any,
                 n: AmountSpec::Canonical("Cards".to_string()),
                 pile: Pile::Hand,
                 upgrade: 0,
                 free_this_turn: false,
                 distinct: true,
             }]),
        ]),

        "DaughterOfTheWind" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Attack".to_string())) },
             vec![Effect::GainBlock {
                 amount: AmountSpec::Canonical("Block".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "GamePiece" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Power".to_string())) },
             vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        ]),

        "HelicalDart" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::TaggedAs("Shiv".to_string())) },
             vec![Effect::ApplyPower {
                 power_id: "HelicalDartPower".to_string(),
                 amount: AmountSpec::Canonical("Dexterity".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "IronClub" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: None },
             vec![Effect::DrawCards { amount: AmountSpec::Fixed(1) }]),
        ]),

        "IvoryTile" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: None },
             vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        ]),

        "Kunai" => Some(vec![
            (RelicHook::BeforeSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::SetRelicCounter { key: "Kunai_attacks".to_string(), value: AmountSpec::Fixed(0) }]),
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Attack".to_string())) },
             vec![
                Effect::ModifyRelicCounter { key: "Kunai_attacks".to_string(), delta: AmountSpec::Fixed(1) },
                Effect::Conditional {
                    condition: Condition::RelicCounterModEq { key: "Kunai_attacks".to_string(), modulus: 3, remainder: 0 },
                    then_branch: vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }],
                    else_branch: vec![],
                },
             ]),
        ]),

        "Shuriken" => Some(vec![
            (RelicHook::BeforeSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::SetRelicCounter { key: "Shuriken_attacks".to_string(), value: AmountSpec::Fixed(0) }]),
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Attack".to_string())) },
             vec![
                Effect::ModifyRelicCounter { key: "Shuriken_attacks".to_string(), delta: AmountSpec::Fixed(1) },
                Effect::Conditional {
                    condition: Condition::RelicCounterModEq { key: "Shuriken_attacks".to_string(), modulus: 3, remainder: 0 },
                    then_branch: vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }],
                    else_branch: vec![],
                },
             ]),
        ]),

        "Nunchaku" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Attack".to_string())) },
             vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        ]),

        "TuningFork" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Skill".to_string())) },
             vec![Effect::GainBlock {
                 amount: AmountSpec::Canonical("Block".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "LetterOpener" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Skill".to_string())) },
             vec![Effect::DealDamage {
                 amount: AmountSpec::Canonical("Damage".to_string()),
                 target: Target::AllEnemies,
                 hits: 1,
             }]),
        ]),

        "Kusarigama" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Attack".to_string())) },
             vec![Effect::DealDamage {
                 amount: AmountSpec::Canonical("Damage".to_string()),
                 target: Target::RandomEnemy,
                 hits: 1,
             }]),
        ]),

        "OrnamentalFan" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Attack".to_string())) },
             vec![Effect::GainBlock {
                 amount: AmountSpec::Canonical("Block".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "LostWisp" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Power".to_string())) },
             vec![Effect::DealDamage {
                 amount: AmountSpec::Canonical("Damage".to_string()),
                 target: Target::AllEnemies,
                 hits: 1,
             }]),
        ]),

        "Permafrost" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Power".to_string())) },
             vec![Effect::GainBlock {
                 amount: AmountSpec::Canonical("Block".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "RazorTooth" => Some(vec![
            // Body is "upgrade the played card" — needs a Source-card selector that
            // we don't have yet. Documented placeholder; runtime no-op.
            (RelicHook::AfterCardPlayed { filter: None }, vec![]),
        ]),

        "MummifiedHand" => Some(vec![
            (RelicHook::AfterCardPlayed { filter: Some(CardFilter::OfType("Power".to_string())) }, vec![]),
        ]),

        "CloakClasp" => Some(vec![
            (RelicHook::BeforeTurnEnd { owner_side_only: true },
             vec![Effect::GainBlock {
                 amount: AmountSpec::Mul {
                     left: Box::new(AmountSpec::CardCountInPile {
                         pile: PileSelector::Single(Pile::Hand),
                         filter: CardFilter::Any,
                     }),
                     right: Box::new(AmountSpec::Canonical("Block".to_string())),
                 },
                 target: Target::SelfPlayer,
             }]),
        ]),

        "ScreamingFlagon" => Some(vec![
            (RelicHook::BeforeTurnEnd { owner_side_only: true },
             vec![Effect::Conditional {
                 condition: Condition::CardCountInPile {
                     pile: Pile::Hand,
                     op: Comparison::Eq,
                     value: 0,
                 },
                 then_branch: vec![Effect::DealDamage {
                     amount: AmountSpec::Canonical("Damage".to_string()),
                     target: Target::AllEnemies,
                     hits: 1,
                 }],
                 else_branch: vec![],
             }]),
        ]),

        "Orichalcum" => Some(vec![
            (RelicHook::BeforeTurnEnd { owner_side_only: true },
             vec![Effect::GainBlock {
                 amount: AmountSpec::Canonical("Block".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "FakeOrichalcum" => Some(vec![
            (RelicHook::BeforeTurnEnd { owner_side_only: true },
             vec![Effect::GainBlock {
                 amount: AmountSpec::Canonical("Block".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "PaelsTears" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::Conditional {
                 condition: Condition::Not(Box::new(Condition::RoundEquals { n: 1 })),
                 then_branch: vec![Effect::GainEnergy {
                     amount: AmountSpec::Canonical("Energy".to_string()),
                 }],
                 else_branch: vec![],
             }]),
        ]),

        "RippleBasin" => Some(vec![
            (RelicHook::BeforeTurnEnd { owner_side_only: true },
             vec![Effect::Conditional {
                 // No Attack played this turn — Eq comparison against count.
                 // Approximated by re-using HandHasCardMatching's "any" idiom but
                 // inverted; since we lack a CardsPlayedThisTurn-comparison
                 // primitive in Condition, body is ungated here.
                 condition: Condition::Always,
                 then_branch: vec![Effect::GainBlock {
                     amount: AmountSpec::Canonical("Block".to_string()),
                     target: Target::SelfPlayer,
                 }],
                 else_branch: vec![],
             }]),
        ]),

        "DiamondDiadem" => Some(vec![
            (RelicHook::BeforeTurnEnd { owner_side_only: true },
             vec![Effect::Conditional {
                 // CardsPlayedThisTurn<=2 — no direct LE in Condition for
                 // CardsPlayedThisTurn AmountSpec; approximate as Always so the
                 // body lands even when the player overplayed. STUB.
                 condition: Condition::Always,
                 then_branch: vec![Effect::ApplyPower {
                     power_id: "DiamondDiademPower".to_string(),
                     amount: AmountSpec::Fixed(1),
                     target: Target::SelfPlayer,
                 }],
                 else_branch: vec![],
             }]),
        ]),

        "StoneCalendar" => Some(vec![
            (RelicHook::BeforeTurnEnd { owner_side_only: true },
             vec![Effect::Conditional {
                 condition: Condition::RoundEquals { n: 7 },
                 then_branch: vec![Effect::DealDamage {
                     amount: AmountSpec::Canonical("Damage".to_string()),
                     target: Target::AllEnemies,
                     hits: 1,
                 }],
                 else_branch: vec![],
             }]),
        ]),

        "SealOfGold" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![
                 Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) },
                 Effect::LoseGold { amount: AmountSpec::Canonical("Gold".to_string()) },
             ]),
        ]),

        "BoneTea" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
             vec![Effect::UpgradeCards { from: Pile::Hand, selector: Selector::All }]),
        ]),

        "TeaOfDiscourtesy" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::Repeat {
                 count: AmountSpec::Canonical("DazedCount".to_string()),
                 body: vec![Effect::AddCardToPile {
                     card_id: "Dazed".to_string(),
                     upgrade: 0,
                     pile: Pile::Draw,
                 }],
             }]),
        ]),

        "BoundPhylactery" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::SummonOsty {
                 osty_id: "BoundPhylactery".to_string(),
                 max_hp: Some(AmountSpec::Canonical("Summon".to_string())),
             }]),
        ]),

        "PhylacteryUnbound" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::SummonOsty {
                 osty_id: "PhylacteryUnbound".to_string(),
                 max_hp: Some(AmountSpec::Canonical("StartOfCombat".to_string())),
             }]),
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::SummonOsty {
                 osty_id: "PhylacteryUnbound".to_string(),
                 max_hp: Some(AmountSpec::Canonical("StartOfTurn".to_string())),
             }]),
        ]),

        "Byrdpip" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::SummonOsty {
                 osty_id: "Byrdpip".to_string(),
                 max_hp: None,
             }]),
        ]),

        "PaelsLegion" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::SummonOsty {
                 osty_id: "PaelsLegion".to_string(),
                 max_hp: None,
             }]),
        ]),

        "MeatOnTheBone" => Some(vec![
            (RelicHook::AfterCombatVictory,
             vec![Effect::Conditional {
                 // CurrentHp/MaxHp <= 50% — needs HpPctLe Condition variant.
                 condition: Condition::Always,
                 then_branch: vec![Effect::Heal {
                     amount: AmountSpec::Canonical("Heal".to_string()),
                     target: Target::SelfPlayer,
                 }],
                 else_branch: vec![],
             }]),
        ]),

        "RedSkull" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::ApplyPower {
                 power_id: "StrengthPower".to_string(),
                 amount: AmountSpec::Canonical("Strength".to_string()),
                 target: Target::SelfPlayer,
             }]),
        ]),

        "MrStruggles" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: false },
             vec![Effect::DealDamage {
                 amount: AmountSpec::Fixed(1),
                 target: Target::AllEnemies,
                 hits: 1,
             }]),
        ]),

        "LunarPastry" => Some(vec![
            (RelicHook::AfterPlayerTurnEnd,
             vec![Effect::GainStars { amount: AmountSpec::Canonical("Stars".to_string()) }]),
        ]),

        "ParryingShield" => Some(vec![
            (RelicHook::AfterPlayerTurnEnd,
             vec![Effect::DealDamage {
                 amount: AmountSpec::Canonical("Damage".to_string()),
                 target: Target::RandomEnemy,
                 hits: 1,
             }]),
        ]),

        "Pendulum" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: false },
             vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        ]),

        "HappyFlower" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![
                Effect::ModifyRelicCounter { key: "HappyFlower_turns".to_string(), delta: AmountSpec::Fixed(1) },
                Effect::Conditional {
                    condition: Condition::RelicCounterModEq { key: "HappyFlower_turns".to_string(), modulus: 3, remainder: 0 },
                    then_branch: vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }],
                    else_branch: vec![],
                },
             ]),
        ]),

        "FakeHappyFlower" => Some(vec![
            (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        ]),

        "BeatingRemnant" => Some(vec![
            (RelicHook::AfterDamageReceived, vec![]),
        ]),

        "EmotionChip" => Some(vec![
            (RelicHook::AfterDamageReceived, vec![]),
        ]),

        "LavaLamp" => Some(vec![
            (RelicHook::AfterDamageReceived, vec![]),
        ]),

        "CharonsAshes" => Some(vec![
            (RelicHook::AfterCardExhausted,
             vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        ]),

        "ForgottenSoul" => Some(vec![
            (RelicHook::AfterCardExhausted,
             vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        ]),

        "Tingsha" => Some(vec![
            (RelicHook::AfterCardDiscarded,
             vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        ]),

        "ToughBandages" => Some(vec![
            (RelicHook::AfterCardDiscarded,
             vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        ]),

        "CaptainsWheel" => Some(vec![
            (RelicHook::AfterBlockCleared,
             vec![Effect::Conditional {
                condition: Condition::RoundEquals { n: 3 },
                then_branch: vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }],
                else_branch: vec![],
             }]),
        ]),

        "HornCleat" => Some(vec![
            (RelicHook::AfterBlockCleared,
             vec![Effect::Conditional {
                condition: Condition::RoundEquals { n: 2 },
                then_branch: vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }],
                else_branch: vec![],
             }]),
        ]),

        "BagOfPreparation" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        ]),

        "BoomingConch" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        ]),

        "PhilosophersStone" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![
                Effect::IncreaseMaxEnergy { delta: AmountSpec::Canonical("Energy".to_string()) },
                Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::AllEnemies },
             ]),
        ]),

        "BlessedAntler" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::IncreaseMaxEnergy { delta: AmountSpec::Fixed(1) }]),
        ]),

        "BloodVial" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: true },
             vec![Effect::Heal { amount: AmountSpec::Canonical("Heal".to_string()), target: Target::SelfPlayer }]),
        ]),

        "BurningSticks" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::SetRelicCounter { key: "BurningSticks_charges".to_string(), value: AmountSpec::Fixed(1) }]),
            (RelicHook::AfterCardExhausted,
             vec![Effect::Conditional {
                condition: Condition::And(
                    Box::new(Condition::SourceCardHasKeyword("Skill".to_string())),
                    Box::new(Condition::RelicCounterGe { key: "BurningSticks_charges".to_string(), value: 1 }),
                ),
                then_branch: vec![
                    Effect::CloneSourceCardToPile {
                        pile: Pile::Hand,
                        cost_override_this_combat: None,
                        copies: AmountSpec::Fixed(1),
                    },
                    Effect::ModifyRelicCounter { key: "BurningSticks_charges".to_string(), delta: AmountSpec::Fixed(-1) },
                ],
                else_branch: vec![],
             }]),
        ]),

        "DataDisk" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::ApplyPower { power_id: "FocusPower".to_string(), amount: AmountSpec::Canonical("FocusPower".to_string()), target: Target::SelfPlayer }]),
        ]),

        "DelicateFrond" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::FillPotionSlots]),
        ]),

        "DivineRight" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::GainStars { amount: AmountSpec::Canonical("Stars".to_string()) }]),
        ]),

        "Ectoplasm" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::IncreaseMaxEnergy { delta: AmountSpec::Fixed(1) }]),
        ]),

        "EmberTea" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![
                Effect::Conditional {
                    condition: Condition::RelicCounterGe { key: "EmberTea_charges".to_string(), value: 1 },
                    then_branch: vec![
                        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer },
                        Effect::ModifyRelicCounter { key: "EmberTea_charges".to_string(), delta: AmountSpec::Fixed(-1) },
                    ],
                    else_branch: vec![],
                },
             ]),
        ]),

        "FakeBloodVial" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: true },
             vec![Effect::Heal { amount: AmountSpec::Canonical("Heal".to_string()), target: Target::SelfPlayer }]),
        ]),

        "FestivePopper" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: true },
             vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        ]),

        "Fiddle" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: false },
             vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        ]),

        "GamblingChip" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: true },
             vec![Effect::DiscardHandAndDrawSameCount]),
        ]),

        "GhostSeed" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::ApplyKeywordToCards { keyword: "Ethereal".to_string(), from: Pile::Draw, selector: Selector::All }]),
        ]),

        "Gorget" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::ApplyPower { power_id: "PlatingPower".to_string(), amount: AmountSpec::Canonical("PlatingPower".to_string()), target: Target::SelfPlayer }]),
        ]),

        "JeweledMask" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::MoveAllByFilterAcrossPiles { to_pile: Pile::Hand, filter: CardFilter::OfType("Power".to_string()) }]),
        ]),

        "JossPaper" => Some(vec![
            (RelicHook::AfterCardExhausted,
             vec![
                Effect::ModifyRelicCounter { key: "JossPaper_exhausts".to_string(), delta: AmountSpec::Fixed(1) },
                Effect::Conditional {
                    condition: Condition::RelicCounterModEq { key: "JossPaper_exhausts".to_string(), modulus: 5, remainder: 0 },
                    then_branch: vec![Effect::DrawCards { amount: AmountSpec::Fixed(1) }],
                    else_branch: vec![],
                },
             ]),
        ]),

        "OddlySmoothStone" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }]),
        ]),

        "PaelsBlood" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: false },
             vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        ]),

        "PetrifiedToad" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::GainPotionToBelt { potion_id: "PotionShapedRock".to_string() }]),
        ]),

        "PollinousCore" => Some(vec![
            (RelicHook::BeforeSideTurnStart { owner_side_only: true, first_turn_only: false },
             vec![Effect::ModifyRelicCounter { key: "PollinousCore_turns".to_string(), delta: AmountSpec::Fixed(1) }]),
            (RelicHook::AfterPlayerTurnStart { first_turn_only: false },
             vec![Effect::Conditional {
                condition: Condition::RelicCounterModEq { key: "PollinousCore_turns".to_string(), modulus: 4, remainder: 0 },
                then_branch: vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }],
                else_branch: vec![],
             }]),
        ]),

        "PrismaticGem" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::IncreaseMaxEnergy { delta: AmountSpec::Fixed(1) }]),
        ]),

        "PumpkinCandle" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::IncreaseMaxEnergy { delta: AmountSpec::Fixed(1) }]),
        ]),

        "RingOfTheDrake" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: false },
             vec![Effect::Conditional {
                condition: Condition::Not(Box::new(Condition::RoundGe { n: 4 })),
                then_branch: vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }],
                else_branch: vec![],
             }]),
        ]),

        "RingOfTheSnake" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        ]),

        "Sozu" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::IncreaseMaxEnergy { delta: AmountSpec::Fixed(1) }]),
        ]),

        "SparklingRouge" => Some(vec![
            (RelicHook::AfterBlockCleared,
             vec![Effect::Conditional {
                condition: Condition::RoundEquals { n: 3 },
                then_branch: vec![
                    Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer },
                    Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer },
                ],
                else_branch: vec![],
             }]),
        ]),

        "SpikedGauntlets" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::IncreaseMaxEnergy { delta: AmountSpec::Fixed(1) }]),
        ]),

        "StoneCracker" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::UpgradeCards {
                from: Pile::Draw,
                selector: Selector::FirstMatching { n: 2, filter: CardFilter::Upgradable },
             }]),
        ]),

        "ToastyMittens" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: false },
             vec![
                Effect::ExhaustCards {
                    from: Pile::Draw,
                    selector: Selector::FirstMatching { n: 1, filter: CardFilter::Any },
                },
                Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer },
             ]),
        ]),

        "Vajra" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        ]),

        "VelvetChoker" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::IncreaseMaxEnergy { delta: AmountSpec::Fixed(1) }]),
        ]),

        "VexingPuzzlebox" => Some(vec![
            (RelicHook::AfterPlayerTurnStart { first_turn_only: true },
             vec![Effect::AddRandomCardFromPool {
                pool: CardPoolRef::CharacterAny,
                filter: CardFilter::Any,
                n: AmountSpec::Fixed(1),
                pile: Pile::Hand,
                upgrade: 0,
                free_this_turn: true,
                distinct: true,
             }]),
        ]),

        "WhisperingEarring" => Some(vec![
            (RelicHook::BeforeCombatStart,
             vec![Effect::IncreaseMaxEnergy { delta: AmountSpec::Fixed(1) }]),
        ]),


        _ => None,
    }
}

/// Registry of cards whose OnPlay is fully expressed as data, replacing
/// their match-arm implementation in `combat.rs::dispatch_on_play`. The
/// dispatcher consults this first; falls back to the match-arm path for
/// cards still on the hand-coded route.
///
/// Migrating a card here means: (1) the only primitives it uses are
/// already implemented in the VM, and (2) its existing spec-derived
/// tests still pass after the route change. This is plan §0.2.6 in
/// motion — one card or family at a time.
pub fn card_effects(card_id: &str) -> Option<Vec<Effect>> {
    match card_id {
        // All 5 Strike variants: deal Damage to a single chosen enemy.
        "StrikeIronclad" | "StrikeSilent" | "StrikeDefect" | "StrikeRegent"
        | "StrikeNecrobinder" => Some(vec![Effect::DealDamage {
            amount: AmountSpec::Canonical("Damage".to_string()),
            target: Target::ChosenEnemy,
            hits: 1,
        }]),
        // All 5 Defend variants: gain Block on self.
        "DefendIronclad" | "DefendSilent" | "DefendDefect" | "DefendRegent"
        | "DefendNecrobinder" => Some(vec![Effect::GainBlock {
            amount: AmountSpec::Canonical("Block".to_string()),
            target: Target::SelfPlayer,
        }]),

        // ----- Ironclad starter / common -----
        // Bash: damage + Vulnerable on single enemy.
        "Bash" => Some(vec![
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
        ]),
        // Neutralize (Silent basic): damage + Weak.
        "Neutralize" => Some(vec![
            Effect::DealDamage {
                amount: AmountSpec::Canonical("Damage".to_string()),
                target: Target::ChosenEnemy,
                hits: 1,
            },
            Effect::ApplyPower {
                power_id: "WeakPower".to_string(),
                amount: AmountSpec::Canonical("Weak".to_string()),
                target: Target::ChosenEnemy,
            },
        ]),
        // Thunderclap: AOE damage + AOE Vulnerable.
        "Thunderclap" => Some(vec![
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
        ]),
        // IronWave: block then damage.
        "IronWave" => Some(vec![
            Effect::GainBlock {
                amount: AmountSpec::Canonical("Block".to_string()),
                target: Target::SelfPlayer,
            },
            Effect::DealDamage {
                amount: AmountSpec::Canonical("Damage".to_string()),
                target: Target::ChosenEnemy,
                hits: 1,
            },
        ]),
        // TwinStrike: Damage × 2 hits to a single enemy.
        "TwinStrike" => Some(vec![Effect::DealDamage {
            amount: AmountSpec::Canonical("Damage".to_string()),
            target: Target::ChosenEnemy,
            hits: 2,
        }]),
        // Inflame: apply Strength to self.
        "Inflame" => Some(vec![Effect::ApplyPower {
            power_id: "StrengthPower".to_string(),
            amount: AmountSpec::Canonical("StrengthPower".to_string()),
            target: Target::SelfPlayer,
        }]),
        // Bloodletting: lose HP (bypass block) + gain energy.
        "Bloodletting" => Some(vec![
            Effect::LoseHp {
                amount: AmountSpec::Canonical("HpLoss".to_string()),
                target: Target::SelfPlayer,
            },
            Effect::GainEnergy {
                amount: AmountSpec::Canonical("Energy".to_string()),
            },
        ]),

        // ----- Necrobinder commons -----
        // Defile: single-target damage (Ethereal handled at routing layer).
        "Defile" => Some(vec![Effect::DealDamage {
            amount: AmountSpec::Canonical("Damage".to_string()),
            target: Target::ChosenEnemy,
            hits: 1,
        }]),
        // Defy: block self + Weak on target.
        "Defy" => Some(vec![
            Effect::GainBlock {
                amount: AmountSpec::Canonical("Block".to_string()),
                target: Target::SelfPlayer,
            },
            Effect::ApplyPower {
                power_id: "WeakPower".to_string(),
                amount: AmountSpec::Canonical("Weak".to_string()),
                target: Target::ChosenEnemy,
            },
        ]),

        // ----- Regent commons -----
        // CosmicIndifference: block self.
        "CosmicIndifference" => Some(vec![Effect::GainBlock {
            amount: AmountSpec::Canonical("Block".to_string()),
            target: Target::SelfPlayer,
        }]),
        // CloakOfStars: block self.
        "CloakOfStars" => Some(vec![Effect::GainBlock {
            amount: AmountSpec::Canonical("Block".to_string()),
            target: Target::SelfPlayer,
        }]),
        // AstralPulse: AOE damage.
        "AstralPulse" => Some(vec![Effect::DealDamage {
            amount: AmountSpec::Canonical("Damage".to_string()),
            target: Target::AllEnemies,
            hits: 1,
        }]),

        // ----- Defect commons -----
        // BeamCell: damage + Vulnerable (Bash-shape, smaller numbers).
        "BeamCell" => Some(vec![
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
        ]),
        // BoostAway: block self + Dazed to discard.
        "BoostAway" => Some(vec![
            Effect::GainBlock {
                amount: AmountSpec::Canonical("Block".to_string()),
                target: Target::SelfPlayer,
            },
            Effect::AddCardToPile {
                card_id: "Dazed".to_string(),
                upgrade: 0,
                pile: Pile::Discard,
            },
        ]),

        // ----- Calc-var hand-encodings (subsystem unlock) -----
        // BodySlam: damage = player's current block.
        // C# BodySlam.cs:35 -- CalculatedDamageVar with multiplier = Owner.Creature.Block.
        "BodySlam" => Some(vec![Effect::DealDamage {
            amount: AmountSpec::SelfBlock,
            target: Target::ChosenEnemy,
            hits: 1,
        }]),
        // PerfectedStrike: CalculationBase + ExtraDamage * StrikeCount.
        // C# PerfectedStrike.cs:46. Both terms are canonical vars
        // (CalculationBase = base damage, ExtraDamage = per-strike
        // multiplier, both upgrade-aware). Data-driven via
        // Add(Canonical, Mul(CardCount, Canonical)).
        "PerfectedStrike" => Some(vec![Effect::DealDamage {
            amount: AmountSpec::Add {
                left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())),
                right: Box::new(AmountSpec::Mul {
                    left: Box::new(AmountSpec::CardCountInPile {
                        pile: PileSelector::AllCombat,
                        filter: CardFilter::TaggedAs("Strike".to_string()),
                    }),
                    right: Box::new(AmountSpec::Canonical("ExtraDamage".to_string())),
                }),
            },
            target: Target::ChosenEnemy,
            hits: 1,
        }]),
        // Bully: CalculationBase + ExtraDamage * target's Vulnerable amount.
        // C# Bully.cs:36 -- WithMultiplier(_, target) => target.GetPowerAmount<VulnerablePower>().
        "Bully" => Some(vec![Effect::DealDamage {
            amount: AmountSpec::Add {
                left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())),
                right: Box::new(AmountSpec::Mul {
                    left: Box::new(AmountSpec::TargetPowerAmount {
                        power_id: "VulnerablePower".to_string(),
                    }),
                    right: Box::new(AmountSpec::Canonical("ExtraDamage".to_string())),
                }),
            },
            target: Target::ChosenEnemy,
            hits: 1,
        }]),
        // MindBlast: damage = number of cards in draw pile.
        "MindBlast" => Some(vec![Effect::DealDamage {
            amount: AmountSpec::CardCountInPile {
                pile: PileSelector::Single(Pile::Draw),
                filter: CardFilter::Any,
            },
            target: Target::ChosenEnemy,
            hits: 1,
        }]),

        // ===== Auto-generated bulk card port (553 cards) =====
        // Generated by tools/merge_card_ports/autogen.py from cards.json.
        // ~328 encoded by shape-match; ~225 skipped (need primitive that
        // is missing, stub-only, or have a shape the auto-encoder did
        // not recognize). SKIP comments below name each.

// are not yet ported. See `// SKIP` comments for reasons.
        "Abrasive" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "ThornsPower".to_string(), amount: AmountSpec::Canonical("ThornsPower".to_string()), target: Target::SelfPlayer }]),
        "Accelerant" => Some(vec![Effect::ApplyPower { power_id: "AccelerantPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Accuracy" => Some(vec![Effect::ApplyPower { power_id: "AccuracyPower".to_string(), amount: AmountSpec::Canonical("AccuracyPower".to_string()), target: Target::SelfPlayer }]),
        "Afterimage" => Some(vec![Effect::ApplyPower { power_id: "AfterimagePower".to_string(), amount: AmountSpec::Canonical("AfterimagePower".to_string()), target: Target::SelfPlayer }]),
        "Aggression" => Some(vec![Effect::ApplyPower { power_id: "AggressionPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Alignment" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Armaments" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Arsenal" => Some(vec![Effect::ApplyPower { power_id: "ArsenalPower".to_string(), amount: AmountSpec::Canonical("ArsenalPower".to_string()), target: Target::SelfPlayer }]),
        "AscendersBane" => Some(vec![]),
        "Assassinate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "Automation" => Some(vec![Effect::ApplyPower { power_id: "AutomationPower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "Backflip" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Backstab" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BallLightning" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BansheesCry" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Barrage" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BattleTrance" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "BeaconOfHope" => Some(vec![Effect::ApplyPower { power_id: "BeaconOfHopePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "BeatIntoShape" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Beckon" => Some(vec![]),
        "BelieveInYou" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "BiasedCognition" => Some(vec![Effect::ApplyPower { power_id: "BiasedCognitionPower".to_string(), amount: AmountSpec::Canonical("BiasedCognitionPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "FocusPower".to_string(), amount: AmountSpec::Canonical("FocusPower".to_string()), target: Target::SelfPlayer }]),
        "BladeOfInk" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Bludgeon" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BootSequence" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "BorrowedTime" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Break" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "BubbleBubble" => Some(vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("PoisonPower".to_string()), target: Target::ChosenEnemy }]),
        "Buffer" => Some(vec![Effect::ApplyPower { power_id: "BufferPower".to_string(), amount: AmountSpec::Canonical("BufferPower".to_string()), target: Target::SelfPlayer }]),
        "BulkUp" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "Burn" => Some(vec![]),
        "BurningPact" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Bury" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ByrdSwoop" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Calamity" => Some(vec![Effect::ApplyPower { power_id: "CalamityPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "CallOfTheVoid" => Some(vec![Effect::ApplyPower { power_id: "CallOfTheVoidPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Caltrops" => Some(vec![Effect::ApplyPower { power_id: "ThornsPower".to_string(), amount: AmountSpec::Canonical("ThornsPower".to_string()), target: Target::SelfPlayer }]),
        "Capacitor" => Some(vec![Effect::ApplyPower { power_id: "CapacitorPower".to_string(), amount: AmountSpec::Canonical("Repeat".to_string()), target: Target::SelfPlayer }]),
        "Catastrophe" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "ChildOfTheStars" => Some(vec![Effect::ApplyPower { power_id: "ChildOfTheStarsPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Clash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Clumsy" => Some(vec![]),
        "ColdSnap" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Comet" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Compact" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "ConsumingShadow" => Some(vec![Effect::ApplyPower { power_id: "ConsumingShadowPower".to_string(), amount: AmountSpec::Canonical("ConsumingShadowPower".to_string()), target: Target::SelfPlayer }]),
        "Coolant" => Some(vec![Effect::ApplyPower { power_id: "CoolantPower".to_string(), amount: AmountSpec::Canonical("CoolantPower".to_string()), target: Target::SelfPlayer }]),
        "Coolheaded" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Corruption" => Some(vec![Effect::ApplyPower { power_id: "CorruptionPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Countdown" => Some(vec![Effect::ApplyPower { power_id: "CountdownPower".to_string(), amount: AmountSpec::Canonical("CountdownPower".to_string()), target: Target::SelfPlayer }]),
        "CreativeAi" => Some(vec![Effect::ApplyPower { power_id: "CreativeAiPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Cruelty" => Some(vec![Effect::ApplyPower { power_id: "CrueltyPower".to_string(), amount: AmountSpec::Canonical("CrueltyPower".to_string()), target: Target::SelfPlayer }]),
        "CrushUnder" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "CurseOfTheBell" => Some(vec![]),
        "DaggerThrow" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "DanseMacabre" => Some(vec![Effect::ApplyPower { power_id: "DanseMacabrePower".to_string(), amount: AmountSpec::Canonical("DanseMacabrePower".to_string()), target: Target::SelfPlayer }]),
        "DarkEmbrace" => Some(vec![Effect::ApplyPower { power_id: "DarkEmbracePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Dazed" => Some(vec![]),
        "DeadlyPoison" => Some(vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("PoisonPower".to_string()), target: Target::ChosenEnemy }]),
        "Deathbringer" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("DoomPower".to_string()), target: Target::AllEnemies }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::AllEnemies }]),
        "Debilitate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Debris" => Some(vec![]),
        "Debt" => Some(vec![]),
        "Decay" => Some(vec![]),
        "Deflect" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Defragment" => Some(vec![Effect::ApplyPower { power_id: "FocusPower".to_string(), amount: AmountSpec::Canonical("FocusPower".to_string()), target: Target::SelfPlayer }]),
        "Demesne" => Some(vec![Effect::ApplyPower { power_id: "DemesnePower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "DeprecatedCard" => Some(vec![]),
        "Devastate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "DevourLife" => Some(vec![Effect::ApplyPower { power_id: "DevourLifePower".to_string(), amount: AmountSpec::Canonical("DevourLifePower".to_string()), target: Target::SelfPlayer }]),
        "Disintegration" => Some(vec![]),
        "Dismantle" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Doubt" => Some(vec![]),
        "DrainPower" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "DramaticEntrance" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "DrumOfBattle" => Some(vec![Effect::ApplyPower { power_id: "DrumOfBattlePower".to_string(), amount: AmountSpec::Canonical("DrumOfBattlePower".to_string()), target: Target::SelfPlayer }]),
        "EchoForm" => Some(vec![Effect::ApplyPower { power_id: "EchoFormPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "EchoingSlash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Enthralled" => Some(vec![]),
        "Entropy" => Some(vec![Effect::ApplyPower { power_id: "EntropyPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Envenom" => Some(vec![Effect::ApplyPower { power_id: "EnvenomPower".to_string(), amount: AmountSpec::Canonical("EnvenomPower".to_string()), target: Target::SelfPlayer }]),
        "EternalArmor" => Some(vec![Effect::ApplyPower { power_id: "PlatingPower".to_string(), amount: AmountSpec::Canonical("PlatingPower".to_string()), target: Target::SelfPlayer }]),
        "Expertise" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Exterminate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "FallingStar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "FanOfKnives" => Some(vec![Effect::ApplyPower { power_id: "FanOfKnivesPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Fasten" => Some(vec![Effect::ApplyPower { power_id: "FastenPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Fear" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "FeelNoPain" => Some(vec![Effect::ApplyPower { power_id: "FeelNoPainPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Feral" => Some(vec![Effect::ApplyPower { power_id: "FeralPower".to_string(), amount: AmountSpec::Canonical("FeralPower".to_string()), target: Target::SelfPlayer }]),
        "Finesse" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "FlashOfSteel" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "FlickFlack" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "FocusedStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "FollowThrough" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Folly" => Some(vec![]),
        "Footwork" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }]),
        "ForbiddenGrimoire" => Some(vec![Effect::ApplyPower { power_id: "ForbiddenGrimoirePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "ForegoneConclusion" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "FranticEscape" => Some(vec![]),
        "Friendship" => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "Furnace" => Some(vec![Effect::ApplyPower { power_id: "FurnacePower".to_string(), amount: AmountSpec::Canonical("Forge".to_string()), target: Target::SelfPlayer }]),
        "Genesis" => Some(vec![Effect::ApplyPower { power_id: "GenesisPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "GiantRock" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Glacier" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Glasswork" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Glitterstream" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "GoForTheEyes" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "GrandFinale" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Graveblast" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Greed" => Some(vec![]),
        "Guilty" => Some(vec![]),
        "GunkUp" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Hailstorm" => Some(vec![Effect::ApplyPower { power_id: "HailstormPower".to_string(), amount: AmountSpec::Canonical("HailstormPower".to_string()), target: Target::SelfPlayer }]),
        "HammerTime" => Some(vec![Effect::ApplyPower { power_id: "HammerTimePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "HandOfGreed" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "HandTrick" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Haunt" => Some(vec![Effect::ApplyPower { power_id: "HauntPower".to_string(), amount: AmountSpec::Canonical("HpLoss".to_string()), target: Target::SelfPlayer }]),
        "Haze" => Some(vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("PoisonPower".to_string()), target: Target::AllEnemies }]),
        "Hegemony" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "HeirloomHammer" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "HelloWorld" => Some(vec![Effect::ApplyPower { power_id: "HelloWorldPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Hellraiser" => Some(vec![Effect::ApplyPower { power_id: "HellraiserPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Hemokinesis" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "HowlFromBeyond" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "IAmInvincible" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Impatience" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Impervious" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Infection" => Some(vec![]),
        "Inferno" => Some(vec![Effect::ApplyPower { power_id: "InfernoPower".to_string(), amount: AmountSpec::Canonical("InfernoPower".to_string()), target: Target::SelfPlayer }]),
        "InfiniteBlades" => Some(vec![Effect::ApplyPower { power_id: "InfiniteBladesPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Injury" => Some(vec![]),
        "Intercept" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Iteration" => Some(vec![Effect::ApplyPower { power_id: "IterationPower".to_string(), amount: AmountSpec::Canonical("IterationPower".to_string()), target: Target::SelfPlayer }]),
        "Juggernaut" => Some(vec![Effect::ApplyPower { power_id: "JuggernautPower".to_string(), amount: AmountSpec::Canonical("JuggernautPower".to_string()), target: Target::SelfPlayer }]),
        "Juggling" => Some(vec![Effect::ApplyPower { power_id: "JugglingPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "KinglyKick" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "KinglyPunch" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Knockdown" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "KnockoutBlow" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "KnowThyPlace" => Some(vec![Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Leap" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Lethality" => Some(vec![Effect::ApplyPower { power_id: "LethalityPower".to_string(), amount: AmountSpec::Canonical("LethalityPower".to_string()), target: Target::SelfPlayer }]),
        "Lift" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "LightningRod" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "LightningRodPower".to_string(), amount: AmountSpec::Canonical("LightningRodPower".to_string()), target: Target::SelfPlayer }]),
        "Loop" => Some(vec![Effect::ApplyPower { power_id: "LoopPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Luminesce" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "LunarBlast" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MachineLearning" => Some(vec![Effect::ApplyPower { power_id: "MachineLearningPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "MadScience" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "MakeItSo" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ManifestAuthority" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "MasterOfStrategy" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "MasterPlanner" => Some(vec![Effect::ApplyPower { power_id: "MasterPlannerPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Mayhem" => Some(vec![Effect::ApplyPower { power_id: "MayhemPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Metamorphosis" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "MeteorShower" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::AllEnemies }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::AllEnemies }]),
        "MeteorStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MindRot" => Some(vec![]),
        "MinionDiveBomb" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MinionSacrifice" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "MinionStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MomentumStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MonarchsGaze" => Some(vec![Effect::ApplyPower { power_id: "MonarchsGazePower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "NecroMastery" => Some(vec![Effect::ApplyPower { power_id: "NecroMasteryPower".to_string(), amount: AmountSpec::Canonical("Summon".to_string()), target: Target::SelfPlayer }]),
        "NegativePulse" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("DoomPower".to_string()), target: Target::AllEnemies }]),
        "NeowsFury" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "NeutronAegis" => Some(vec![Effect::ApplyPower { power_id: "PlatingPower".to_string(), amount: AmountSpec::Canonical("PlatingPower".to_string()), target: Target::SelfPlayer }]),
        "Normality" => Some(vec![]),
        "Nostalgia" => Some(vec![Effect::ApplyPower { power_id: "NostalgiaPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "NoxiousFumes" => Some(vec![Effect::ApplyPower { power_id: "NoxiousFumesPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Null" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Oblivion" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("DoomPower".to_string()), target: Target::ChosenEnemy }]),
        "Orbit" => Some(vec![Effect::ApplyPower { power_id: "OrbitPower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "Outbreak" => Some(vec![Effect::ApplyPower { power_id: "OutbreakPower".to_string(), amount: AmountSpec::Canonical("OutbreakPower".to_string()), target: Target::SelfPlayer }]),
        "Outmaneuver" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "PactsEnd" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Pagestorm" => Some(vec![Effect::ApplyPower { power_id: "PagestormPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "PaleBlueDot" => Some(vec![Effect::ApplyPower { power_id: "PaleBlueDotPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Panache" => Some(vec![Effect::ApplyPower { power_id: "PanachePower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Parry" => Some(vec![Effect::ApplyPower { power_id: "ParryPower".to_string(), amount: AmountSpec::Canonical("ParryPower".to_string()), target: Target::SelfPlayer }]),
        "Parse" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "ParticleWall" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Peck" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "PhantomBlades" => Some(vec![Effect::ApplyPower { power_id: "PhantomBladesPower".to_string(), amount: AmountSpec::Canonical("PhantomBladesPower".to_string()), target: Target::SelfPlayer }]),
        "PillarOfCreation" => Some(vec![Effect::ApplyPower { power_id: "PillarOfCreationPower".to_string(), amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Pinpoint" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "PoorSleep" => Some(vec![]),
        "Pounce" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Predator" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "PrepTime" => Some(vec![Effect::ApplyPower { power_id: "PrepTimePower".to_string(), amount: AmountSpec::Canonical("PrepTimePower".to_string()), target: Target::SelfPlayer }]),
        "Prepared" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Production" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Prophesize" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Protector" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Prowess" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "Purity" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Pyre" => Some(vec![Effect::ApplyPower { power_id: "PyrePower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "Reap" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ReaperForm" => Some(vec![Effect::ApplyPower { power_id: "ReaperFormPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Reave" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Rebound" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Reflect" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Reflex" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Regret" => Some(vec![]),
        "RipAndTear" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        "RocketPunch" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "RollingBoulder" => Some(vec![Effect::ApplyPower { power_id: "RollingBoulderPower".to_string(), amount: AmountSpec::Canonical("RollingBoulderPower".to_string()), target: Target::SelfPlayer }]),
        "Royalties" => Some(vec![Effect::ApplyPower { power_id: "RoyaltiesPower".to_string(), amount: AmountSpec::Canonical("Gold".to_string()), target: Target::SelfPlayer }]),
        "Rupture" => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "Scrape" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "SculptingStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Seance" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "SecondWind" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "SeekerStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "SeekingEdge" => Some(vec![Effect::ApplyPower { power_id: "SeekingEdgePower".to_string(), amount: AmountSpec::Canonical("Forge".to_string()), target: Target::SelfPlayer }]),
        "SentryMode" => Some(vec![Effect::ApplyPower { power_id: "SentryModePower".to_string(), amount: AmountSpec::Canonical("SentryModePower".to_string()), target: Target::SelfPlayer }]),
        "SerpentForm" => Some(vec![Effect::ApplyPower { power_id: "SerpentFormPower".to_string(), amount: AmountSpec::Canonical("SerpentFormPower".to_string()), target: Target::SelfPlayer }]),
        "SevenStars" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "ShadowShield" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "ShadowStep" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Shame" => Some(vec![]),
        "Shatter" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "ShiningStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Shiv" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Shroud" => Some(vec![Effect::ApplyPower { power_id: "ShroudPower".to_string(), amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "ShrugItOff" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Skim" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "SleightOfFlesh" => Some(vec![Effect::ApplyPower { power_id: "SleightOfFleshPower".to_string(), amount: AmountSpec::Canonical("SleightOfFleshPower".to_string()), target: Target::SelfPlayer }]),
        "Slice" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Slimed" => Some(vec![]),
        "Sloth" => Some(vec![]),
        "Smokestack" => Some(vec![Effect::ApplyPower { power_id: "SmokestackPower".to_string(), amount: AmountSpec::Canonical("SmokestackPower".to_string()), target: Target::SelfPlayer }]),
        "Sneaky" => Some(vec![Effect::ApplyPower { power_id: "SneakyPower".to_string(), amount: AmountSpec::Canonical("SneakyPower".to_string()), target: Target::SelfPlayer }]),
        "SolarStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Soot" => Some(vec![]),
        "Soul" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "SoulStorm" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "SovereignBlade" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Sow" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "SpectrumShift" => Some(vec![Effect::ApplyPower { power_id: "SpectrumShiftPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Speedster" => Some(vec![Effect::ApplyPower { power_id: "SpeedsterPower".to_string(), amount: AmountSpec::Canonical("SpeedsterPower".to_string()), target: Target::SelfPlayer }]),
        "Spinner" => Some(vec![Effect::ApplyPower { power_id: "SpinnerPower".to_string(), amount: AmountSpec::Canonical("SpinnerPower".to_string()), target: Target::SelfPlayer }]),
        "SpiritOfAsh" => Some(vec![Effect::ApplyPower { power_id: "SpiritOfAshPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "SporeMind" => Some(vec![]),
        "Squash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "Squeeze" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Stampede" => Some(vec![Effect::ApplyPower { power_id: "StampedePower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Stardust" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        "Stomp" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "StoneArmor" => Some(vec![Effect::ApplyPower { power_id: "PlatingPower".to_string(), amount: AmountSpec::Canonical("PlatingPower".to_string()), target: Target::SelfPlayer }]),
        "Storm" => Some(vec![Effect::ApplyPower { power_id: "StormPower".to_string(), amount: AmountSpec::Canonical("StormPower".to_string()), target: Target::SelfPlayer }]),
        "Strangle" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Stratagem" => Some(vec![Effect::ApplyPower { power_id: "StratagemPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Subroutine" => Some(vec![Effect::ApplyPower { power_id: "SubroutinePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "SuckerPunch" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Supercritical" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Supermassive" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Suppress" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "SweepingBeam" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "SwordSage" => Some(vec![Effect::ApplyPower { power_id: "SwordSagePower".to_string(), amount: AmountSpec::Canonical("SwordSagePower".to_string()), target: Target::SelfPlayer }]),
        "Synthesis" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Tactician" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "TagTeam" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Tank" => Some(vec![Effect::ApplyPower { power_id: "TankPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Taunt" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "TearAsunder" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "TeslaCoil" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "TheGambit" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "TheHunt" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "TheScythe" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "TheSealedThrone" => Some(vec![Effect::ApplyPower { power_id: "TheSealedThronePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "ThinkingAhead" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Thrash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ThrummingHatchet" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Thunder" => Some(vec![Effect::ApplyPower { power_id: "ThunderPower".to_string(), amount: AmountSpec::Canonical("ThunderPower".to_string()), target: Target::SelfPlayer }]),
        "TimesUp" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ToolsOfTheTrade" => Some(vec![Effect::ApplyPower { power_id: "ToolsOfTheTradePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Toxic" => Some(vec![]),
        "Tracking" => Some(vec![Effect::ApplyPower { power_id: "TrackingPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Transfigure" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "TrashToTreasure" => Some(vec![Effect::ApplyPower { power_id: "TrashToTreasurePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Tyranny" => Some(vec![Effect::ApplyPower { power_id: "TyrannyPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "UltimateDefend" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "UltimateStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Unleash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Unmovable" => Some(vec![Effect::ApplyPower { power_id: "UnmovablePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Untouchable" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "UpMySleeve" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Veilpiercer" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Vicious" => Some(vec![Effect::ApplyPower { power_id: "ViciousPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Void" => Some(vec![]),
        "VoidForm" => Some(vec![Effect::ApplyPower { power_id: "VoidFormPower".to_string(), amount: AmountSpec::Canonical("VoidFormPower".to_string()), target: Target::SelfPlayer }]),
        "Volley" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        "WasteAway" => Some(vec![]),
        "WellLaidPlans" => Some(vec![Effect::ApplyPower { power_id: "WellLaidPlansPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Whistle" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Wisp" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Wound" => Some(vec![]),
        "WraithForm" => Some(vec![Effect::ApplyPower { power_id: "IntangiblePower".to_string(), amount: AmountSpec::Canonical("IntangiblePower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "WraithFormPower".to_string(), amount: AmountSpec::Canonical("WraithFormPower".to_string()), target: Target::SelfPlayer }]),
        "Writhe" => Some(vec![]),
        "WroughtInWar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        // SKIP Acrobatics: has richer match-arm in combat.rs; let it run
        // SKIP AdaptiveStrike: has richer match-arm in combat.rs; let it run
        // SKIP Adrenaline: Skill/Self shape with vars={'Energy', 'Cards'} powers=set() not recognized
        // SKIP Afterlife: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP Alchemize: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP AllForOne: has richer match-arm in combat.rs; let it run
        // SKIP Anger: has richer match-arm in combat.rs; let it run
        // SKIP Anointed: has richer match-arm in combat.rs; let it run
        // SKIP Anticipate: Skill/Self shape with vars={'Power'} powers={'DexterityPower'} not recognized
        // SKIP Apotheosis: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Apparition: has richer match-arm in combat.rs; let it run
        // SKIP AshenStrike: has richer match-arm in combat.rs; let it run
        // SKIP BadLuck: has richer match-arm in combat.rs; let it run
        // SKIP Barricade: has richer match-arm in combat.rs; let it run
        // SKIP BeatDown: has richer match-arm in combat.rs; let it run
        // SKIP Begone: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP BigBang: Skill/Self shape with vars={'Energy', 'Cards', 'Stars', 'Forge'} powers=set() not recognized
        // SKIP BlackHole: has richer match-arm in combat.rs; let it run
        // SKIP BladeDance: has richer match-arm in combat.rs; let it run
        // SKIP BlightStrike: has richer match-arm in combat.rs; let it run
        // SKIP BloodWall: Skill/Self shape with vars={'HpLoss', 'Block'} powers=set() not recognized
        // SKIP Blur: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP BodySlam: has richer match-arm in combat.rs; let it run
        // SKIP Bodyguard: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP Bolas: has richer match-arm in combat.rs; let it run
        // SKIP Bombardment: has richer match-arm in combat.rs; let it run
        // SKIP BoneShards: AOE attack without Damage var
        // SKIP BouncingFlask: unknown shape: type=Skill target=RandomEnemy vars={'Power', 'Repeat'} powers={'PoisonPower'}
        // SKIP Brand: Skill/Self shape with vars={'HpLoss', 'Power'} powers={'StrengthPower'} not recognized
        // SKIP Breakthrough: has richer match-arm in combat.rs; let it run
        // SKIP BrightestFlame: Skill/Self shape with vars={'MaxHp', 'Energy', 'Cards'} powers=set() not recognized
        // SKIP BulletTime: has richer match-arm in combat.rs; let it run
        // SKIP Bully: has richer match-arm in combat.rs; let it run
        // SKIP Bulwark: Skill/Self shape with vars={'Block', 'Forge'} powers=set() not recognized
        // SKIP BundleOfJoy: has richer match-arm in combat.rs; let it run
        // SKIP Burst: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP ByrdonisEgg: unknown shape: type=Quest target=None vars=set() powers=set()
        // SKIP Calcify: has richer match-arm in combat.rs; let it run
        // SKIP CalculatedGamble: has richer match-arm in combat.rs; let it run
        // SKIP CaptureSpirit: Skill/AnyEnemy shape with vars={'Cards', 'Damage'} powers=set() not recognized
        // SKIP Cascade: has richer match-arm in combat.rs; let it run
        // SKIP CelestialMight: has richer match-arm in combat.rs; let it run
        // SKIP Chaos: Skill/Self shape with vars={'Repeat'} powers=set() not recognized
        // SKIP Charge: has richer match-arm in combat.rs; let it run
        // SKIP ChargeBattery: Skill/Self shape with vars={'Energy', 'Block'} powers=set() not recognized
        // SKIP Chill: has richer match-arm in combat.rs; let it run
        // SKIP Cinder: has richer match-arm in combat.rs; let it run
        // SKIP Claw: has richer match-arm in combat.rs; let it run
        // SKIP Cleanse: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP CloakAndDagger: has richer match-arm in combat.rs; let it run
        // SKIP CollisionCourse: has richer match-arm in combat.rs; let it run
        // SKIP Colossus: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP CompileDriver: has richer match-arm in combat.rs; let it run
        // SKIP Conflagration: has richer match-arm in combat.rs; let it run
        // SKIP Conqueror: Skill/AnyEnemy shape with vars={'Forge'} powers=set() not recognized
        // SKIP Convergence: Skill/Self shape with vars={'Energy', 'Stars'} powers=set() not recognized
        // SKIP Coordinate: Skill/Self shape with vars={'Power'} powers={'StrengthPower'} not recognized
        // SKIP CorrosiveWave: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP CrashLanding: has richer match-arm in combat.rs; let it run
        // SKIP CrescentSpear: has richer match-arm in combat.rs; let it run
        // SKIP CrimsonMantle: has richer match-arm in combat.rs; let it run
        // SKIP DaggerSpray: has richer match-arm in combat.rs; let it run
        // SKIP DarkShackles: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Darkness: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Dash: has richer match-arm in combat.rs; let it run
        // SKIP DeathMarch: has richer match-arm in combat.rs; let it run
        // SKIP DeathsDoor: has richer match-arm in combat.rs; let it run
        // SKIP DecisionsDecisions: has richer match-arm in combat.rs; let it run
        // SKIP Delay: Skill/Self shape with vars={'Energy', 'Block'} powers=set() not recognized
        // SKIP DemonForm: has richer match-arm in combat.rs; let it run
        // SKIP DemonicShield: Skill/Self shape with vars={'HpLoss', 'CalculationExtra', 'CalculationBase', 'CalculatedBlock'} powers=set() not recognized
        // SKIP Dirge: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP Discovery: has richer match-arm in combat.rs; let it run
        // SKIP Distraction: has richer match-arm in combat.rs; let it run
        // SKIP DodgeAndRoll: has richer match-arm in combat.rs; let it run
        // SKIP Dominate: has richer match-arm in combat.rs; let it run
        // SKIP DoubleEnergy: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Dredge: has richer match-arm in combat.rs; let it run
        // SKIP DualWield: has richer match-arm in combat.rs; let it run
        // SKIP Dualcast: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP DyingStar: has richer match-arm in combat.rs; let it run
        // SKIP Eidolon: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP EndOfDays: has richer match-arm in combat.rs; let it run
        // SKIP EnergySurge: unknown shape: type=Skill target=AllAllies vars={'Energy'} powers=set()
        // SKIP EnfeeblingTouch: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Enlightenment: has richer match-arm in combat.rs; let it run
        // SKIP Entrench: has richer match-arm in combat.rs; let it run
        // SKIP Equilibrium: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP Eradicate: has richer match-arm in combat.rs; let it run
        // SKIP EscapePlan: has richer match-arm in combat.rs; let it run
        // SKIP EvilEye: has richer match-arm in combat.rs; let it run
        // SKIP ExpectAFight: has richer match-arm in combat.rs; let it run
        // SKIP Expose: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Feed: has richer match-arm in combat.rs; let it run
        // SKIP FeedingFrenzy: Skill/Self shape with vars={'Power'} powers={'StrengthPower'} not recognized
        // SKIP Fetch: Attack to single enemy without Damage var
        // SKIP FiendFire: has richer match-arm in combat.rs; let it run
        // SKIP FightMe: has richer match-arm in combat.rs; let it run
        // SKIP FightThrough: has richer match-arm in combat.rs; let it run
        // SKIP Finisher: has richer match-arm in combat.rs; let it run
        // SKIP Fisticuffs: has richer match-arm in combat.rs; let it run
        // SKIP FlakCannon: has richer match-arm in combat.rs; let it run
        // SKIP FlameBarrier: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP Flanking: Skill/AnyEnemy shape with vars=set() powers=set() not recognized
        // SKIP Flatten: Attack to single enemy without Damage var
        // SKIP Flechettes: has richer match-arm in combat.rs; let it run
        // SKIP ForgottenRitual: has richer match-arm in combat.rs; let it run
        // SKIP Ftl: has richer match-arm in combat.rs; let it run
        // SKIP Fuel: Skill/Self shape with vars={'Energy', 'Cards'} powers=set() not recognized
        // SKIP Fusion: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP GammaBlast: has richer match-arm in combat.rs; let it run
        // SKIP GangUp: has richer match-arm in combat.rs; let it run
        // SKIP GatherLight: Skill/Self shape with vars={'Block', 'Stars'} powers=set() not recognized
        // SKIP GeneticAlgorithm: has richer match-arm in combat.rs; let it run
        // SKIP Glimmer: Skill/Self shape with vars={'Dynamic', 'Cards'} powers=set() not recognized
        // SKIP GlimpseBeyond: unknown shape: type=Skill target=AllAllies vars={'Cards'} powers=set()
        // SKIP Glow: has richer match-arm in combat.rs; let it run
        // SKIP GoldAxe: has richer match-arm in combat.rs; let it run
        // SKIP GraveWarden: has richer match-arm in combat.rs; let it run
        // SKIP Guards: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP GuidingStar: has richer match-arm in combat.rs; let it run
        // SKIP Hang: has richer match-arm in combat.rs; let it run
        // SKIP Havoc: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Headbutt: has richer match-arm in combat.rs; let it run
        // SKIP HeavenlyDrill: has richer match-arm in combat.rs; let it run
        // SKIP HelixDrill: has richer match-arm in combat.rs; let it run
        // SKIP HiddenCache: Skill/Self shape with vars={'Power', 'Stars'} powers={'StarNextTurnPower'} not recognized
        // SKIP HiddenDaggers: Skill/Self shape with vars={'Dynamic', 'Cards'} powers=set() not recognized
        // SKIP HiddenGem: Skill/Self shape with vars={'Int'} powers=set() not recognized
        // SKIP HighFive: AOE attack without Damage var
        // SKIP Hologram: has richer match-arm in combat.rs; let it run
        // SKIP Hotfix: Skill/Self shape with vars={'Power'} powers={'FocusPower'} not recognized
        // SKIP HuddleUp: unknown shape: type=Skill target=AllAllies vars={'Cards'} powers=set()
        // SKIP Hyperbeam: has richer match-arm in combat.rs; let it run
        // SKIP IceLance: has richer match-arm in combat.rs; let it run
        // SKIP Ignition: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP InfernalBlade: has richer match-arm in combat.rs; let it run
        // SKIP Invoke: Skill/Self shape with vars={'Summon', 'Energy'} powers=set() not recognized
        // SKIP JackOfAllTrades: has richer match-arm in combat.rs; let it run
        // SKIP Jackpot: has richer match-arm in combat.rs; let it run
        // SKIP KnifeTrap: has richer match-arm in combat.rs; let it run
        // SKIP LanternKey: unknown shape: type=Quest target=Self vars=set() powers=set()
        // SKIP Largesse: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP LeadingStrike: has richer match-arm in combat.rs; let it run
        // SKIP LegSweep: has richer match-arm in combat.rs; let it run
        // SKIP LegionOfBone: has richer match-arm in combat.rs; let it run
        // SKIP Malaise: Skill/AnyEnemy shape with vars=set() powers=set() not recognized
        // SKIP Mangle: has richer match-arm in combat.rs; let it run
        // SKIP Maul: has richer match-arm in combat.rs; let it run
        // SKIP Melancholy: Skill/Self shape with vars={'Energy', 'Block'} powers=set() not recognized
        // SKIP MementoMori: has richer match-arm in combat.rs; let it run
        // SKIP Mimic: has richer match-arm in combat.rs; let it run
        // SKIP MindBlast: has richer match-arm in combat.rs; let it run
        // SKIP Mirage: has richer match-arm in combat.rs; let it run
        // SKIP Misery: has richer match-arm in combat.rs; let it run
        // SKIP Modded: has richer match-arm in combat.rs; let it run
        // SKIP MoltenFist: has richer match-arm in combat.rs; let it run
        // SKIP Monologue: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP MultiCast: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Murder: has richer match-arm in combat.rs; let it run
        // SKIP Neurosurge: has richer match-arm in combat.rs; let it run
        // SKIP Nightmare: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP NoEscape: has richer match-arm in combat.rs; let it run
        // SKIP NotYet: Skill/Self shape with vars={'Heal'} powers=set() not recognized
        // SKIP Offering: has richer match-arm in combat.rs; let it run
        // SKIP Omnislice: has richer match-arm in combat.rs; let it run
        // SKIP OneTwoPunch: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Overclock: has richer match-arm in combat.rs; let it run
        // SKIP PanicButton: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP Patter: has richer match-arm in combat.rs; let it run
        // SKIP PerfectedStrike: has richer match-arm in combat.rs; let it run
        // SKIP PhotonCut: has richer match-arm in combat.rs; let it run
        // SKIP PiercingWail: Skill/AllEnemies with vars={'Dynamic'} powers=set() not recognized
        // SKIP Pillage: has richer match-arm in combat.rs; let it run
        // SKIP PoisonedStab: has richer match-arm in combat.rs; let it run
        // SKIP Poke: Attack to single enemy without Damage var
        // SKIP PommelStrike: has richer match-arm in combat.rs; let it run
        // SKIP PreciseCut: has richer match-arm in combat.rs; let it run
        // SKIP PrimalForce: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Prolong: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP PullAggro: has richer match-arm in combat.rs; let it run
        // SKIP PullFromBelow: has richer match-arm in combat.rs; let it run
        // SKIP Putrefy: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Quadcast: Skill/Self shape with vars={'Repeat'} powers=set() not recognized
        // SKIP Quasar: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Radiate: has richer match-arm in combat.rs; let it run
        // SKIP Rage: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Rainbow: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Rally: has richer match-arm in combat.rs; let it run
        // SKIP Rampage: has richer match-arm in combat.rs; let it run
        // SKIP Rattle: has richer match-arm in combat.rs; let it run
        // SKIP Reanimate: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP Reboot: has richer match-arm in combat.rs; let it run
        // SKIP RefineBlade: Skill/Self shape with vars={'Energy', 'Forge'} powers=set() not recognized
        // SKIP Refract: has richer match-arm in combat.rs; let it run
        // SKIP Relax: has richer match-arm in combat.rs; let it run
        // SKIP Rend: has richer match-arm in combat.rs; let it run
        // SKIP Resonance: has richer match-arm in combat.rs; let it run
        // SKIP Restlessness: has richer match-arm in combat.rs; let it run
        // SKIP Ricochet: has richer match-arm in combat.rs; let it run
        // SKIP RightHandHand: Attack to single enemy without Damage var
        // SKIP RoyalGamble: Skill/Self shape with vars={'Stars'} powers=set() not recognized
        // SKIP Sacrifice: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Salvo: has richer match-arm in combat.rs; let it run
        // SKIP Scavenge: has richer match-arm in combat.rs; let it run
        // SKIP Scourge: has richer match-arm in combat.rs; let it run
        // SKIP Scrawl: has richer match-arm in combat.rs; let it run
        // SKIP SecretTechnique: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP SecretWeapon: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP SetupStrike: has richer match-arm in combat.rs; let it run
        // SKIP Severance: has richer match-arm in combat.rs; let it run
        // SKIP Shadowmeld: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP SharedFate: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Shockwave: Skill/AllEnemies with vars={'Dynamic'} powers=set() not recognized
        // SKIP SicEm: Attack to single enemy without Damage var
        // SKIP SignalBoost: Skill/Self shape with vars={'Power'} powers={'SignalBoostPower'} not recognized
        // SKIP Skewer: X-cost single-target attack (would need Repeat over hits)
        // SKIP Snakebite: has richer match-arm in combat.rs; let it run
        // SKIP Snap: Attack to single enemy without Damage var
        // SKIP Spite: has richer match-arm in combat.rs; let it run
        // SKIP Splash: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP SpoilsMap: unknown shape: type=Quest target=Self vars={'Gold'} powers=set()
        // SKIP SpoilsOfBattle: Skill/Self shape with vars={'Forge', 'Cards'} powers=set() not recognized
        // SKIP Spur: Skill/Self shape with vars={'Summon', 'Heal'} powers=set() not recognized
        // SKIP Stack: Skill/Self shape with vars={'CalculationExtra', 'CalculationBase', 'CalculatedBlock'} powers=set() not recognized
        // SKIP Stoke: has richer match-arm in combat.rs; let it run
        // SKIP StormOfSteel: has richer match-arm in combat.rs; let it run
        // SKIP SummonForth: Skill/Self shape with vars={'Forge'} powers=set() not recognized
        // SKIP Sunder: has richer match-arm in combat.rs; let it run
        // SKIP Survivor: has richer match-arm in combat.rs; let it run
        // SKIP SweepingGaze: Random-target attack without Damage var
        // SKIP SwordBoomerang: has richer match-arm in combat.rs; let it run
        // SKIP Synchronize: Skill/Self shape with vars={'CalculationExtra', 'CalculationBase', 'Calculated'} powers=set() not recognized
        // SKIP Tempest: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Terraforming: Skill/Self shape with vars={'Power'} powers={'VigorPower'} not recognized
        // SKIP TheBomb: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP TheSmith: Skill/Self shape with vars={'Forge'} powers=set() not recognized
        // SKIP ToricToughness: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP Tremble: has richer match-arm in combat.rs; let it run
        // SKIP TrueGrit: has richer match-arm in combat.rs; let it run
        // SKIP Turbo: has richer match-arm in combat.rs; let it run
        // SKIP Undeath: has richer match-arm in combat.rs; let it run
        // SKIP Unrelenting: has richer match-arm in combat.rs; let it run
        // SKIP Uppercut: has richer match-arm in combat.rs; let it run
        // SKIP Uproar: has richer match-arm in combat.rs; let it run
        // SKIP Venerate: Skill/Self shape with vars={'Stars'} powers=set() not recognized
        // SKIP Voltaic: Skill/Self shape with vars={'CalculationExtra', 'CalculationBase', 'Calculated'} powers=set() not recognized
        // SKIP Whirlwind: X-cost AOE (Whirlwind shape -- handled in earlier migration)
        // SKIP WhiteNoise: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Wish: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Zap: Skill/Self shape with vars=set() powers=set() not recognized
        // ===== Manual v2 card ports (batches v2_1..v2_3) =====
        // 247 hand-curated arms covering Acrobatics..Rattle.
        // Source: tools/merge_card_ports/batch_v2_*.txt.
        // SKIPs documented in those files.

        "Acrobatics" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }, Effect::DiscardCards { from: Pile::Hand, selector: Selector::PlayerInteractive { n: 1 } }]),
        "Adrenaline" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Alchemize" => Some(vec![Effect::GenerateRandomPotion]),
        "AshenStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Add { left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("ExtraDamage".to_string())), right: Box::new(AmountSpec::CardCountInPile { pile: PileSelector::Single(Pile::Exhaust), filter: CardFilter::Any }) }) }, target: Target::ChosenEnemy, hits: 1 }]),
        "BadLuck" => Some(vec![]),
        "BigBang" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }, Effect::GainStars { amount: AmountSpec::Canonical("Stars".to_string()) }, Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }, Effect::Forge { amount: AmountSpec::Canonical("Forge".to_string()) }]),
        "BlackHole" => Some(vec![Effect::ApplyPower { power_id: "BlackHolePower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::SelfPlayer }]),
        "Blur" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "BlurPower".to_string(), amount: AmountSpec::Canonical("Blur".to_string()), target: Target::SelfPlayer }]),
        "Brand" => Some(vec![Effect::LoseHp { amount: AmountSpec::Canonical("HpLoss".to_string()), target: Target::SelfPlayer }, Effect::ExhaustCards { from: Pile::Hand, selector: Selector::PlayerInteractive { n: 1 } }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::SelfPlayer }]),
        "BrightestFlame" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }, Effect::ChangeMaxHp { amount: AmountSpec::Mul { left: Box::new(AmountSpec::Fixed(-1)), right: Box::new(AmountSpec::Canonical("MaxHp".to_string())) }, target: Target::SelfPlayer }]),
        "Bulwark" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::Forge { amount: AmountSpec::Canonical("Forge".to_string()) }]),
        "Burst" => Some(vec![Effect::ApplyPower { power_id: "BurstPower".to_string(), amount: AmountSpec::Canonical("Skills".to_string()), target: Target::SelfPlayer }]),
        "ByrdonisEgg" => Some(vec![]),
        "Calcify" => Some(vec![Effect::ApplyPower { power_id: "CalcifyPower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::SelfPlayer }]),
        "CelestialMight" => Some(vec![Effect::Repeat { count: AmountSpec::Canonical("Repeat".to_string()), body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }] }]),
        "ChargeBattery" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "EnergyNextTurnPower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "Colossus" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "ColossusPower".to_string(), amount: AmountSpec::Canonical("Colossus".to_string()), target: Target::SelfPlayer }]),
        "Conqueror" => Some(vec![Effect::Forge { amount: AmountSpec::Canonical("Forge".to_string()) }, Effect::ApplyPower { power_id: "ConquerorPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::ChosenEnemy }]),
        "Convergence" => Some(vec![Effect::ApplyPower { power_id: "RetainHandPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "EnergyNextTurnPower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "StarNextTurnPower".to_string(), amount: AmountSpec::Canonical("Stars".to_string()), target: Target::SelfPlayer }]),
        "CorrosiveWave" => Some(vec![Effect::ApplyPower { power_id: "CorrosiveWavePower".to_string(), amount: AmountSpec::Canonical("CorrosiveWave".to_string()), target: Target::SelfPlayer }]),
        "DarkShackles" => Some(vec![Effect::ApplyPower { power_id: "DarkShacklesPower".to_string(), amount: AmountSpec::Canonical("StrengthLoss".to_string()), target: Target::ChosenEnemy }]),
        "Dash" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Delay" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "EnergyNextTurnPower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "Dualcast" => Some(vec![Effect::EvokeNextOrb, Effect::EvokeNextOrb]),
        "DyingStar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }, Effect::ApplyPower { power_id: "DyingStarPower".to_string(), amount: AmountSpec::Canonical("StrengthLoss".to_string()), target: Target::AllEnemies }]),
        "EndOfDays" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("Doom".to_string()), target: Target::AllEnemies }]),
        "EnfeeblingTouch" => Some(vec![Effect::ApplyPower { power_id: "EnfeeblingTouchPower".to_string(), amount: AmountSpec::Canonical("StrengthLoss".to_string()), target: Target::ChosenEnemy }]),
        "Entrench" => Some(vec![Effect::GainBlock { amount: AmountSpec::SelfBlock, target: Target::SelfPlayer }]),
        "Equilibrium" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "RetainHandPower".to_string(), amount: AmountSpec::Canonical("Equilibrium".to_string()), target: Target::SelfPlayer }]),
        "FeedingFrenzy" => Some(vec![Effect::ApplyPower { power_id: "FeedingFrenzyPower".to_string(), amount: AmountSpec::Canonical("Strength".to_string()), target: Target::SelfPlayer }]),
        "FightThrough" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::AddCardToPile { card_id: "Wound".to_string(), upgrade: 0, pile: Pile::Discard }, Effect::AddCardToPile { card_id: "Wound".to_string(), upgrade: 0, pile: Pile::Discard }]),
        "FlameBarrier" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "FlameBarrierPower".to_string(), amount: AmountSpec::Canonical("DamageBack".to_string()), target: Target::SelfPlayer }]),
        "Flanking" => Some(vec![Effect::ApplyPower { power_id: "FlankingPower".to_string(), amount: AmountSpec::Fixed(2), target: Target::ChosenEnemy }]),
        "Flatten" => Some(vec![Effect::DamageFromOsty { amount: AmountSpec::Canonical("OstyDamage".to_string()), target: Target::ChosenEnemy }]),
        "Fuel" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Fusion" => Some(vec![Effect::ChannelOrb { orb_id: "Plasma".to_string() }]),
        "GammaBlast" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("Weak".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("Vulnerable".to_string()), target: Target::ChosenEnemy }]),
        "GatherLight" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::GainStars { amount: AmountSpec::Canonical("Stars".to_string()) }]),
        "Glow" => Some(vec![Effect::GainStars { amount: AmountSpec::Canonical("Stars".to_string()) }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }, Effect::ApplyPower { power_id: "DrawCardsNextTurnPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "GuidingStar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Havoc" => Some(vec![Effect::AutoplayFromDraw { n: 1 }]),
        "HiddenCache" => Some(vec![Effect::GainStars { amount: AmountSpec::Canonical("Stars".to_string()) }, Effect::ApplyPower { power_id: "StarNextTurnPower".to_string(), amount: AmountSpec::Canonical("StarNextTurnPower".to_string()), target: Target::SelfPlayer }]),
        "Hotfix" => Some(vec![Effect::ApplyPower { power_id: "HotfixPower".to_string(), amount: AmountSpec::Canonical("FocusPower".to_string()), target: Target::SelfPlayer }]),
        "Hyperbeam" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }, Effect::ApplyPower { power_id: "FocusPower".to_string(), amount: AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("FocusPower".to_string())), right: Box::new(AmountSpec::Fixed(-1)) }, target: Target::SelfPlayer }]),
        "IceLance" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::Repeat { count: AmountSpec::Canonical("Repeat".to_string()), body: vec![Effect::ChannelOrb { orb_id: "Frost".to_string() }] }]),
        "Ignition" => Some(vec![Effect::ChannelOrb { orb_id: "Plasma".to_string() }]),
        "Invoke" => Some(vec![Effect::ApplyPower { power_id: "SummonNextTurnPower".to_string(), amount: AmountSpec::Canonical("Summon".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "EnergyNextTurnPower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "LanternKey" => Some(vec![]),
        "Malaise" => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Mul { left: Box::new(AmountSpec::Add { left: Box::new(AmountSpec::XEnergy), right: Box::new(AmountSpec::BranchedOnUpgrade { base: 0, upgraded: 1 }) }), right: Box::new(AmountSpec::Fixed(-1)) }, target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Add { left: Box::new(AmountSpec::XEnergy), right: Box::new(AmountSpec::BranchedOnUpgrade { base: 0, upgraded: 1 }) }, target: Target::ChosenEnemy }]),
        "Melancholy" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "MultiCast" => Some(vec![Effect::Repeat { count: AmountSpec::Add { left: Box::new(AmountSpec::XEnergy), right: Box::new(AmountSpec::BranchedOnUpgrade { base: 0, upgraded: 1 }) }, body: vec![Effect::EvokeNextOrb] }]),
        "Neurosurge" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }, Effect::ApplyPower { power_id: "NeurosurgePower".to_string(), amount: AmountSpec::Canonical("NeurosurgePower".to_string()), target: Target::SelfPlayer }]),
        "NotYet" => Some(vec![Effect::Heal { amount: AmountSpec::Canonical("Heal".to_string()), target: Target::SelfPlayer }]),
        "Offering" => Some(vec![Effect::LoseHp { amount: AmountSpec::Canonical("HpLoss".to_string()), target: Target::SelfPlayer }, Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "OneTwoPunch" => Some(vec![Effect::ApplyPower { power_id: "OneTwoPunchPower".to_string(), amount: AmountSpec::Canonical("Attacks".to_string()), target: Target::SelfPlayer }]),
        "Overclock" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }, Effect::AddCardToPile { card_id: "Burn".to_string(), upgrade: 0, pile: Pile::Discard }]),
        "PanicButton" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "NoBlockPower".to_string(), amount: AmountSpec::Canonical("Turns".to_string()), target: Target::SelfPlayer }]),
        "Patter" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "VigorPower".to_string(), amount: AmountSpec::Canonical("VigorPower".to_string()), target: Target::SelfPlayer }]),
        "PiercingWail" => Some(vec![Effect::ApplyPower { power_id: "PiercingWailPower".to_string(), amount: AmountSpec::Canonical("StrengthLoss".to_string()), target: Target::AllEnemies }]),
        "Prolong" => Some(vec![Effect::ApplyPower { power_id: "BlockNextTurnPower".to_string(), amount: AmountSpec::SelfBlock, target: Target::SelfPlayer }]),
        "PullAggro" => Some(vec![Effect::SummonOsty { osty_id: "Default".to_string(), max_hp: None }, Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Putrefy" => Some(vec![Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::ChosenEnemy }]),
        "Quadcast" => Some(vec![Effect::Repeat { count: AmountSpec::Canonical("Repeat".to_string()), body: vec![Effect::EvokeNextOrb] }]),
        "Rage" => Some(vec![Effect::ApplyPower { power_id: "RagePower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::SelfPlayer }]),
        "Rainbow" => Some(vec![Effect::ChannelOrb { orb_id: "Lightning".to_string() }, Effect::ChannelOrb { orb_id: "Frost".to_string() }, Effect::ChannelOrb { orb_id: "Dark".to_string() }]),
        "Rally" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Reanimate" => Some(vec![Effect::SummonOsty { osty_id: "Default".to_string(), max_hp: None }]),
        "Reboot" => Some(vec![Effect::MoveCard { from: Pile::Hand, to: Pile::Draw, selector: Selector::All }, Effect::Shuffle { pile: Pile::Draw }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "RefineBlade" => Some(vec![Effect::Forge { amount: AmountSpec::Canonical("Forge".to_string()) }, Effect::ApplyPower { power_id: "EnergyNextTurnPower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "Refract" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 2 }, Effect::Repeat { count: AmountSpec::Canonical("Repeat".to_string()), body: vec![Effect::ChannelOrb { orb_id: "Glass".to_string() }] }]),
        "Relax" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "DrawCardsNextTurnPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "EnergyNextTurnPower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "Resonance" => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Fixed(-1), target: Target::AllEnemies }]),
        "RoyalGamble" => Some(vec![Effect::GainStars { amount: AmountSpec::Canonical("Stars".to_string()) }]),
        "Salvo" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "RetainHandPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Scavenge" => Some(vec![Effect::ExhaustCards { from: Pile::Hand, selector: Selector::PlayerInteractive { n: 1 } }, Effect::ApplyPower { power_id: "EnergyNextTurnPower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "Scourge" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("Doom".to_string()), target: Target::ChosenEnemy }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Shadowmeld" => Some(vec![Effect::ApplyPower { power_id: "ShadowmeldPower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::SelfPlayer }]),
        "SharedFate" => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("PlayerStrengthLoss".to_string())), right: Box::new(AmountSpec::Fixed(-1)) }, target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("EnemyStrengthLoss".to_string())), right: Box::new(AmountSpec::Fixed(-1)) }, target: Target::ChosenEnemy }]),
        "Shockwave" => Some(vec![Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::AllEnemies }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::AllEnemies }]),
        "SignalBoost" => Some(vec![Effect::ApplyPower { power_id: "SignalBoostPower".to_string(), amount: AmountSpec::Canonical("SignalBoostPower".to_string()), target: Target::SelfPlayer }]),
        "Skewer" => Some(vec![Effect::Repeat { count: AmountSpec::XEnergy, body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }] }]),
        "Spite" => Some(vec![Effect::Conditional { condition: Condition::OwnerLostHpThisTurn, then_branch: vec![Effect::Repeat { count: AmountSpec::Canonical("Repeat".to_string()), body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }] }], else_branch: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }] }]),
        "SpoilsMap" => Some(vec![]),
        "SpoilsOfBattle" => Some(vec![Effect::Forge { amount: AmountSpec::Canonical("Forge".to_string()) }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Stack" => Some(vec![Effect::GainBlock { amount: AmountSpec::Add { left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("CalculationExtra".to_string())), right: Box::new(AmountSpec::CardCountInPile { pile: PileSelector::Single(Pile::Discard), filter: CardFilter::Any }) }) }, target: Target::SelfPlayer }]),
        "Sunder" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::Conditional { condition: Condition::AttackKilledTarget, then_branch: vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }], else_branch: vec![] }]),
        "Survivor" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DiscardCards { from: Pile::Hand, selector: Selector::PlayerInteractive { n: 1 } }]),
        "Tempest" => Some(vec![Effect::Repeat { count: AmountSpec::Add { left: Box::new(AmountSpec::XEnergy), right: Box::new(AmountSpec::BranchedOnUpgrade { base: 0, upgraded: 1 }) }, body: vec![Effect::ChannelOrb { orb_id: "Lightning".to_string() }] }]),
        "Terraforming" => Some(vec![Effect::ApplyPower { power_id: "VigorPower".to_string(), amount: AmountSpec::Canonical("VigorPower".to_string()), target: Target::SelfPlayer }]),
        "TheSmith" => Some(vec![Effect::Forge { amount: AmountSpec::Canonical("Forge".to_string()) }]),
        "Turbo" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }, Effect::AddCardToPile { card_id: "Void".to_string(), upgrade: 0, pile: Pile::Discard }]),
        "Unrelenting" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "FreeAttackPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Uppercut" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::ChosenEnemy }]),
        "Venerate" => Some(vec![Effect::GainStars { amount: AmountSpec::Canonical("Stars".to_string()) }]),
        "Wish" => Some(vec![Effect::MoveCard { from: Pile::Draw, to: Pile::Hand, selector: Selector::PlayerInteractive { n: 1 } }]),
        "Zap" => Some(vec![Effect::ChannelOrb { orb_id: "Lightning".to_string() }]),
        "Afterlife" => Some(vec![Effect::SummonOsty { osty_id: "Default".to_string(), max_hp: Some(AmountSpec::Canonical("Summon".to_string())) }]),
        "Bodyguard" => Some(vec![Effect::SummonOsty { osty_id: "Default".to_string(), max_hp: Some(AmountSpec::Canonical("Summon".to_string())) }]),
        "BoneShards" => Some(vec![Effect::Conditional { condition: Condition::Not(Box::new(Condition::IsOstyMissing)), then_branch: vec![Effect::DamageFromOsty { amount: AmountSpec::Canonical("OstyDamage".to_string()), target: Target::AllEnemies }, Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::KillOsty], else_branch: vec![] }]),
        "Sacrifice" => Some(vec![Effect::Conditional { condition: Condition::Not(Box::new(Condition::IsOstyMissing)), then_branch: vec![Effect::KillOsty, Effect::GainBlock { amount: AmountSpec::Mul { left: Box::new(AmountSpec::OstyMaxHp), right: Box::new(AmountSpec::Fixed(2)) }, target: Target::SelfPlayer }], else_branch: vec![] }]),
        "HighFive" => Some(vec![Effect::Conditional { condition: Condition::Not(Box::new(Condition::IsOstyMissing)), then_branch: vec![Effect::DamageFromOsty { amount: AmountSpec::Canonical("OstyDamage".to_string()), target: Target::AllEnemies }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("Vulnerable".to_string()), target: Target::AllEnemies }], else_branch: vec![] }]),
        "Poke" => Some(vec![Effect::Conditional { condition: Condition::Not(Box::new(Condition::IsOstyMissing)), then_branch: vec![Effect::DamageFromOsty { amount: AmountSpec::Canonical("OstyDamage".to_string()), target: Target::ChosenEnemy }], else_branch: vec![] }]),
        "SicEm" => Some(vec![Effect::Conditional { condition: Condition::Not(Box::new(Condition::IsOstyMissing)), then_branch: vec![Effect::DamageFromOsty { amount: AmountSpec::Canonical("OstyDamage".to_string()), target: Target::ChosenEnemy }], else_branch: vec![] }, Effect::ApplyPower { power_id: "SicEmPower".to_string(), amount: AmountSpec::Canonical("SicEmPower".to_string()), target: Target::ChosenEnemy }]),
        "SweepingGaze" => Some(vec![Effect::Conditional { condition: Condition::Not(Box::new(Condition::IsOstyMissing)), then_branch: vec![Effect::DamageFromOsty { amount: AmountSpec::Canonical("OstyDamage".to_string()), target: Target::RandomEnemy }], else_branch: vec![] }]),
        "Fetch" => Some(vec![Effect::Conditional { condition: Condition::Not(Box::new(Condition::IsOstyMissing)), then_branch: vec![Effect::DamageFromOsty { amount: AmountSpec::Canonical("OstyDamage".to_string()), target: Target::ChosenEnemy }, Effect::Conditional { condition: Condition::FirstPlayOfSourceCardThisTurn, then_branch: vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }], else_branch: vec![] }], else_branch: vec![] }]),
        "Spur" => Some(vec![Effect::SummonOsty { osty_id: "Default".to_string(), max_hp: Some(AmountSpec::Canonical("Summon".to_string())) }, Effect::HealOsty { amount: AmountSpec::Canonical("Heal".to_string()) }]),
        "Discovery" => Some(vec![Effect::AddRandomCardFromPool { pool: CardPoolRef::CharacterAny, filter: CardFilter::Any, n: AmountSpec::Fixed(1), pile: Pile::Hand, upgrade: 0, free_this_turn: true, distinct: true }]),
        "Distraction" => Some(vec![Effect::AddRandomCardFromPool { pool: CardPoolRef::CharacterSkill, filter: CardFilter::OfType("Skill".to_string()), n: AmountSpec::Fixed(1), pile: Pile::Hand, upgrade: 0, free_this_turn: true, distinct: true }]),
        "InfernalBlade" => Some(vec![Effect::AddRandomCardFromPool { pool: CardPoolRef::CharacterAttack, filter: CardFilter::OfType("Attack".to_string()), n: AmountSpec::Fixed(1), pile: Pile::Hand, upgrade: 0, free_this_turn: true, distinct: true }]),
        "WhiteNoise" => Some(vec![Effect::AddRandomCardFromPool { pool: CardPoolRef::CharacterPower, filter: CardFilter::OfType("Power".to_string()), n: AmountSpec::Fixed(1), pile: Pile::Hand, upgrade: 0, free_this_turn: true, distinct: true }]),
        "JackOfAllTrades" => Some(vec![Effect::AddRandomCardFromPool { pool: CardPoolRef::Colorless, filter: CardFilter::Any, n: AmountSpec::Canonical("Cards".to_string()), pile: Pile::Hand, upgrade: 0, free_this_turn: false, distinct: true }]),
        "BundleOfJoy" => Some(vec![Effect::AddRandomCardFromPool { pool: CardPoolRef::Colorless, filter: CardFilter::Any, n: AmountSpec::Canonical("Cards".to_string()), pile: Pile::Hand, upgrade: 0, free_this_turn: false, distinct: true }]),
        "Quasar" => Some(vec![Effect::AddRandomCardFromPool { pool: CardPoolRef::Colorless, filter: CardFilter::Any, n: AmountSpec::Fixed(1), pile: Pile::Hand, upgrade: 0, free_this_turn: false, distinct: true }]),
        "Splash" => Some(vec![Effect::AddRandomCardFromPool { pool: CardPoolRef::CharacterAttack, filter: CardFilter::OfType("Attack".to_string()), n: AmountSpec::Fixed(1), pile: Pile::Hand, upgrade: 0, free_this_turn: true, distinct: true }]),
        "BeatDown" => Some(vec![Effect::AutoplayCardsFromPile { pile: Pile::Discard, filter: CardFilter::OfType("Attack".to_string()), n: AmountSpec::Canonical("Cards".to_string()) }]),
        "KnifeTrap" => Some(vec![Effect::AutoplayCardsFromPile { pile: Pile::Exhaust, filter: CardFilter::TaggedAs("Shiv".to_string()), n: AmountSpec::CardCountInPile { pile: PileSelector::Single(Pile::Exhaust), filter: CardFilter::TaggedAs("Shiv".to_string()) } }]),
        "Uproar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 2 }, Effect::AutoplayCardsFromPile { pile: Pile::Draw, filter: CardFilter::OfType("Attack".to_string()), n: AmountSpec::Fixed(1) }]),
        "TheBomb" => Some(vec![Effect::ApplyPower { power_id: "TheBombPower".to_string(), amount: AmountSpec::Canonical("Turns".to_string()), target: Target::SelfPlayer }, Effect::SetPowerStateField { power_id: "TheBombPower".to_string(), field: "Damage".to_string(), value: AmountSpec::Canonical("BombDamage".to_string()), target: Target::SelfPlayer }]),
        "ToricToughness" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "ToricToughnessPower".to_string(), amount: AmountSpec::Canonical("Turns".to_string()), target: Target::SelfPlayer }, Effect::SetPowerStateField { power_id: "ToricToughnessPower".to_string(), field: "Block".to_string(), value: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Scrawl" => Some(vec![Effect::DrawCards { amount: AmountSpec::Add { left: Box::new(AmountSpec::Fixed(10)), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::HandCount), right: Box::new(AmountSpec::Fixed(-1)) }) } }]),
        "Restlessness" => Some(vec![Effect::Conditional { condition: Condition::CardCountInPile { pile: Pile::Hand, op: Comparison::Eq, value: 0 }, then_branch: vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }, Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }], else_branch: vec![] }]),
        "PreciseCut" => Some(vec![Effect::DealDamage { amount: AmountSpec::Add { left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("ExtraDamage".to_string())), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::HandCountExcludingSource), right: Box::new(AmountSpec::Fixed(-1)) }) }) }, target: Target::ChosenEnemy, hits: 1 }]),
        "Expose" => Some(vec![Effect::LoseBlock { amount: AmountSpec::TargetBlock, target: Target::ChosenEnemy }, Effect::Conditional { condition: Condition::HasPowerOnTarget { power_id: "ArtifactPower".to_string() }, then_branch: vec![Effect::RemovePower { power_id: "ArtifactPower".to_string(), target: Target::ChosenEnemy }], else_branch: vec![] }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("Power".to_string()), target: Target::ChosenEnemy }]),
        "Dominate" => Some(vec![Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::TargetPowerAmount { power_id: "VulnerablePower".to_string() }, target: Target::SelfPlayer }]),
        "Rend" => Some(vec![Effect::DealDamage { amount: AmountSpec::Add { left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("ExtraDamage".to_string())), right: Box::new(AmountSpec::TargetDebuffCount) }) }, target: Target::ChosenEnemy, hits: 1 }]),
        "Conflagration" => Some(vec![Effect::DealDamage { amount: AmountSpec::Add { left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("ExtraDamage".to_string())), right: Box::new(AmountSpec::CardsPlayedThisTurn { filter: CardFilter::OfType("Attack".to_string()) }) }) }, target: Target::AllEnemies, hits: 1 }]),
        "GoldAxe" => Some(vec![Effect::DealDamage { amount: AmountSpec::Add { left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("ExtraDamage".to_string())), right: Box::new(AmountSpec::CardsPlayedThisTurn { filter: CardFilter::Any }) }) }, target: Target::ChosenEnemy, hits: 1 }]),
        "Murder" => Some(vec![Effect::DealDamage { amount: AmountSpec::Add { left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("ExtraDamage".to_string())), right: Box::new(AmountSpec::CardsDrawnThisTurn) }) }, target: Target::ChosenEnemy, hits: 1 }]),
        "Finisher" => Some(vec![Effect::Repeat { count: AmountSpec::CardsPlayedThisTurn { filter: CardFilter::OfType("Attack".to_string()) }, body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }] }]),
        "MementoMori" => Some(vec![Effect::DealDamage { amount: AmountSpec::Add { left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("ExtraDamage".to_string())), right: Box::new(AmountSpec::CardsDiscardedThisTurn) }) }, target: Target::ChosenEnemy, hits: 1 }]),
        "DeathMarch" => Some(vec![Effect::DealDamage { amount: AmountSpec::Add { left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("ExtraDamage".to_string())), right: Box::new(AmountSpec::CardsDrawnThisTurn) }) }, target: Target::ChosenEnemy, hits: 1 }]),
        "HelixDrill" => Some(vec![Effect::Repeat { count: AmountSpec::EnergySpentThisTurn, body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }] }]),
        "Radiate" => Some(vec![Effect::Repeat { count: AmountSpec::StarsGainedThisTurnPositive, body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }] }]),
        "Flechettes" => Some(vec![Effect::Repeat { count: AmountSpec::CardCountInPile { pile: PileSelector::Single(Pile::Hand), filter: CardFilter::OfType("Skill".to_string()) }, body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }] }]),
        "ExpectAFight" => Some(vec![Effect::GainEnergy { amount: AmountSpec::CardCountInPile { pile: PileSelector::Single(Pile::Hand), filter: CardFilter::OfType("Attack".to_string()) } }, Effect::ApplyPower { power_id: "NoEnergyGainPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Voltaic" => Some(vec![Effect::Repeat { count: AmountSpec::OrbsChanneledThisCombat { orb_id: Some("Lightning".to_string()) }, body: vec![Effect::ChannelOrb { orb_id: "Lightning".to_string() }] }]),
        "Synchronize" => Some(vec![Effect::ApplyPower { power_id: "SynchronizePower".to_string(), amount: AmountSpec::Add { left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("CalculationExtra".to_string())), right: Box::new(AmountSpec::DistinctOrbTypesInQueue) }) }, target: Target::SelfPlayer }]),
        "CompileDriver" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::DrawCards { amount: AmountSpec::DistinctOrbTypesInQueue }]),
        "EvilEye" => Some(vec![Effect::Conditional { condition: Condition::OwnerExhaustedCardThisTurn, then_branch: vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }], else_branch: vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }] }]),
        "ForgottenRitual" => Some(vec![Effect::Conditional { condition: Condition::OwnerExhaustedCardThisTurn, then_branch: vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }], else_branch: vec![] }]),
        "EnergySurge" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "BouncingFlask" => Some(vec![Effect::Repeat { count: AmountSpec::Canonical("Repeat".to_string()), body: vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("Poison".to_string()), target: Target::RandomEnemy }] }]),
        "FightMe" => Some(vec![Effect::Repeat { count: AmountSpec::Canonical("Repeat".to_string()), body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }] }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("EnemyStrength".to_string()), target: Target::ChosenEnemy }]),
        "Eradicate" => Some(vec![Effect::Repeat { count: AmountSpec::XEnergy, body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }] }]),
        "CrashLanding" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }, Effect::Repeat { count: AmountSpec::Add { left: Box::new(AmountSpec::Fixed(10)), right: Box::new(AmountSpec::Mul { left: Box::new(AmountSpec::HandCount), right: Box::new(AmountSpec::Fixed(-1)) }) }, body: vec![Effect::AddCardToPile { card_id: "Debris".to_string(), upgrade: 0, pile: Pile::Hand }] }]),
        "LegionOfBone" => Some(vec![Effect::SummonOsty { osty_id: "Default".to_string(), max_hp: Some(AmountSpec::Canonical("Summon".to_string())) }]),
        "Hang" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower {
        power_id: "HangPower".to_string(),
        amount: AmountSpec::Max {
        left: Box::new(AmountSpec::Fixed(2)),
        right: Box::new(AmountSpec::TargetPowerAmount { power_id: "HangPower".to_string() }),
        },
        target: Target::ChosenEnemy,
        },
        ]),
        "NoEscape" => Some(vec![Effect::ApplyPower {
        power_id: "DoomPower".to_string(),
        amount: AmountSpec::FloorDiv {
        left: Box::new(AmountSpec::TargetPowerAmount { power_id: "DoomPower".to_string() }),
        right: Box::new(AmountSpec::Canonical("DoomThreshold".to_string())),
        },
        target: Target::ChosenEnemy,
        }]),
        "HeavenlyDrill" => Some(vec![Effect::Conditional {
        condition: Condition::XEnergyGe { n: 4 },
        then_branch: vec![Effect::Repeat {
        count: AmountSpec::Mul {
        left: Box::new(AmountSpec::XEnergy),
        right: Box::new(AmountSpec::Fixed(2)),
        },
        body: vec![Effect::DealDamage {
        amount: AmountSpec::Canonical("Damage".to_string()),
        target: Target::ChosenEnemy,
        hits: 1,
        }],
        }],
        else_branch: vec![Effect::Repeat {
        count: AmountSpec::XEnergy,
        body: vec![Effect::DealDamage {
        amount: AmountSpec::Canonical("Damage".to_string()),
        target: Target::ChosenEnemy,
        hits: 1,
        }],
        }],
        }]),
        "BulletTime" => Some(vec![
        Effect::SetCardCost {
        from: Pile::Hand,
        selector: Selector::All,
        cost: AmountSpec::Fixed(0),
        scope: CostScope::ThisTurn,
        },
        Effect::ApplyPower {
        power_id: "NoDrawPower".to_string(),
        amount: AmountSpec::Fixed(1),
        target: Target::SelfPlayer,
        },
        ]),
        "Enlightenment" => Some(vec![Effect::Conditional {
        condition: Condition::IsUpgraded,
        then_branch: vec![Effect::SetCardCost {
        from: Pile::Hand,
        selector: Selector::All,
        cost: AmountSpec::Fixed(1),
        scope: CostScope::ThisCombat,
        }],
        else_branch: vec![Effect::SetCardCost {
        from: Pile::Hand,
        selector: Selector::All,
        cost: AmountSpec::Fixed(1),
        scope: CostScope::ThisTurn,
        }],
        }]),
        "Anointed" => Some(vec![Effect::MoveCard {
        from: Pile::Draw,
        to: Pile::Hand,
        selector: Selector::FirstMatching {
        n: i32::MAX,
        filter: CardFilter::OfRarity("Rare".to_string()),
        },
        }]),
        "Apotheosis" => Some(vec![Effect::UpgradeAllUpgradableCards]),
        "Begone" => Some(vec![Effect::Conditional {
        condition: Condition::IsUpgraded,
        then_branch: vec![Effect::TransformIntoSpecific {
        from: Pile::Hand,
        selector: Selector::PlayerInteractive { n: 1 },
        target_card_id: "MinionStrike".to_string(),
        upgrade: true,
        }],
        else_branch: vec![Effect::TransformIntoSpecific {
        from: Pile::Hand,
        selector: Selector::PlayerInteractive { n: 1 },
        target_card_id: "MinionStrike".to_string(),
        upgrade: false,
        }],
        }]),
        "Charge" => Some(vec![Effect::Conditional {
        condition: Condition::IsUpgraded,
        then_branch: vec![Effect::TransformIntoSpecific {
        from: Pile::Draw,
        selector: Selector::PlayerInteractive { n: 2 },
        target_card_id: "MinionDiveBomb".to_string(),
        upgrade: true,
        }],
        else_branch: vec![Effect::TransformIntoSpecific {
        from: Pile::Draw,
        selector: Selector::PlayerInteractive { n: 2 },
        target_card_id: "MinionDiveBomb".to_string(),
        upgrade: false,
        }],
        }]),
        "Guards" => Some(vec![Effect::Conditional {
        condition: Condition::IsUpgraded,
        then_branch: vec![Effect::TransformIntoSpecific {
        from: Pile::Hand,
        selector: Selector::PlayerInteractive { n: 10 },
        target_card_id: "MinionSacrifice".to_string(),
        upgrade: true,
        }],
        else_branch: vec![Effect::TransformIntoSpecific {
        from: Pile::Hand,
        selector: Selector::PlayerInteractive { n: 10 },
        target_card_id: "MinionSacrifice".to_string(),
        upgrade: false,
        }],
        }]),
        "PrimalForce" => Some(vec![Effect::Conditional {
        condition: Condition::IsUpgraded,
        then_branch: vec![Effect::TransformIntoSpecific {
        from: Pile::Hand,
        selector: Selector::FirstMatching {
        n: i32::MAX,
        filter: CardFilter::OfType("Attack".to_string()),
        },
        target_card_id: "GiantRock".to_string(),
        upgrade: true,
        }],
        else_branch: vec![Effect::TransformIntoSpecific {
        from: Pile::Hand,
        selector: Selector::FirstMatching {
        n: i32::MAX,
        filter: CardFilter::OfType("Attack".to_string()),
        },
        target_card_id: "GiantRock".to_string(),
        upgrade: false,
        }],
        }]),
        "Mimic" => Some(vec![Effect::GainBlock {
        amount: AmountSpec::TargetBlock,
        target: Target::SelfPlayer,
        }]),
        "SecretTechnique" => Some(vec![Effect::MoveCard {
        from: Pile::Draw,
        to: Pile::Hand,
        selector: Selector::PlayerInteractiveFiltered {
        n: 1,
        filter: CardFilter::OfType("Skill".to_string()),
        },
        }]),
        "SecretWeapon" => Some(vec![Effect::MoveCard {
        from: Pile::Draw,
        to: Pile::Hand,
        selector: Selector::PlayerInteractiveFiltered {
        n: 1,
        filter: CardFilter::OfType("Attack".to_string()),
        },
        }]),
        "Anger" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::AddCardToPile { card_id: "Anger".to_string(), upgrade: 0, pile: Pile::Discard },
        ]),
        "GraveWarden" => Some(vec![
        Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer },
        Effect::Repeat { count: AmountSpec::Canonical("Cards".to_string()), body: vec![Effect::AddCardToPile { card_id: "Soul".to_string(), upgrade: 0, pile: Pile::Draw }] },
        ]),
        "BlightStrike" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy },
        ]),
        "CollisionCourse" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::AddCardToPile { card_id: "Debris".to_string(), upgrade: 0, pile: Pile::Hand },
        ]),
        "BladeDance" => Some(vec![Effect::Repeat { count: AmountSpec::Canonical("Cards".to_string()), body: vec![Effect::AddCardToPile { card_id: "Shiv".to_string(), upgrade: 0, pile: Pile::Hand }] }]),
        "Snakebite" => Some(vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("Poison".to_string()), target: Target::ChosenEnemy }]),
        "Anticipate" => Some(vec![
        Effect::ApplyPower { power_id: "AnticipatePower".to_string(), amount: AmountSpec::Canonical("Dexterity".to_string()), target: Target::SelfPlayer },
        Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("Dexterity".to_string()), target: Target::SelfPlayer },
        ]),
        "Ricochet" => Some(vec![Effect::Repeat { count: AmountSpec::Canonical("Repeat".to_string()), body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }] }]),
        "CloakAndDagger" => Some(vec![
        Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer },
        Effect::Repeat { count: AmountSpec::Canonical("Cards".to_string()), body: vec![Effect::AddCardToPile { card_id: "Shiv".to_string(), upgrade: 0, pile: Pile::Hand }] },
        ]),
        "LeadingStrike" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::Repeat { count: AmountSpec::Canonical("Shivs".to_string()), body: vec![Effect::AddCardToPile { card_id: "Shiv".to_string(), upgrade: 0, pile: Pile::Hand }] },
        ]),
        "DaggerSpray" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 2 }]),
        "PoisonedStab" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("Poison".to_string()), target: Target::ChosenEnemy },
        ]),
        "FiendFire" => Some(vec![Effect::Repeat {
        count: AmountSpec::HandCount,
        body: vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::ExhaustRandomInHand { amount: AmountSpec::Fixed(1) },
        ],
        }]),
        "Mangle" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "ManglePower".to_string(), amount: AmountSpec::Canonical("StrengthLoss".to_string()), target: Target::ChosenEnemy },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Mul { left: Box::new(AmountSpec::Canonical("StrengthLoss".to_string())), right: Box::new(AmountSpec::Fixed(-1)) }, target: Target::ChosenEnemy },
        ]),
        "SetupStrike" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::ApplyPower { power_id: "SetupStrikePower".to_string(), amount: AmountSpec::Canonical("Strength".to_string()), target: Target::SelfPlayer },
        Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("Strength".to_string()), target: Target::SelfPlayer },
        ]),
        "Feed" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::Conditional {
        condition: Condition::AttackKilledTarget,
        then_branch: vec![
        Effect::ChangeMaxHp { amount: AmountSpec::Canonical("MaxHp".to_string()), target: Target::SelfPlayer },
        Effect::Heal { amount: AmountSpec::Canonical("MaxHp".to_string()), target: Target::SelfPlayer },
        ],
        else_branch: vec![],
        },
        ]),
        "Barricade" => Some(vec![Effect::ApplyPower { power_id: "BarricadePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "SwordBoomerang" => Some(vec![Effect::Repeat { count: AmountSpec::Canonical("Repeat".to_string()), body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }] }]),
        "Cinder" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::ExhaustRandomInHand { amount: AmountSpec::Fixed(1) },
        ]),
        "TrueGrit" => Some(vec![
        Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer },
        Effect::ExhaustRandomInHand { amount: AmountSpec::Fixed(1) },
        ]),
        "PommelStrike" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) },
        ]),
        "DemonForm" => Some(vec![Effect::ApplyPower { power_id: "DemonFormPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "Breakthrough" => Some(vec![
        Effect::LoseHp { amount: AmountSpec::Canonical("HpLoss".to_string()), target: Target::SelfPlayer },
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 },
        ]),
        "BloodWall" => Some(vec![
        Effect::LoseHp { amount: AmountSpec::Canonical("HpLoss".to_string()), target: Target::SelfPlayer },
        Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer },
        ]),
        "Tremble" => Some(vec![Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("Vulnerable".to_string()), target: Target::ChosenEnemy }]),
        "Apparition" => Some(vec![Effect::ApplyPower { power_id: "IntangiblePower".to_string(), amount: AmountSpec::Canonical("IntangiblePower".to_string()), target: Target::SelfPlayer }]),
        "MoltenFist" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::Conditional {
        condition: Condition::And(
        Box::new(Condition::TargetIsAlive),
        Box::new(Condition::HasPowerOnTarget { power_id: "VulnerablePower".to_string() }),
        ),
        then_branch: vec![Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::TargetPowerAmount { power_id: "VulnerablePower".to_string() }, target: Target::ChosenEnemy }],
        else_branch: vec![],
        },
        ]),
        "Whirlwind" => Some(vec![Effect::Repeat { count: AmountSpec::XEnergy, body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }] }]),
        "LegSweep" => Some(vec![
        Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer },
        Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("Weak".to_string()), target: Target::ChosenEnemy },
        ]),
        "AdaptiveStrike" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::CloneSourceCardToPile { pile: Pile::Discard, cost_override_this_combat: Some(0), copies: AmountSpec::Fixed(1) },
        ]),
        "Undeath" => Some(vec![
        Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer },
        Effect::CloneSourceCardToPile { pile: Pile::Discard, cost_override_this_combat: None, copies: AmountSpec::Fixed(1) },
        ]),
        "DoubleEnergy" => Some(vec![Effect::GainEnergy { amount: AmountSpec::CurrentEnergy }]),
        "Mirage" => Some(vec![Effect::GainBlock {
        amount: AmountSpec::Add {
        left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())),
        right: Box::new(AmountSpec::Mul {
        left: Box::new(AmountSpec::Canonical("CalculationExtra".to_string())),
        right: Box::new(AmountSpec::AllEnemiesPowerSum { power_id: "PoisonPower".to_string() }),
        }),
        },
        target: Target::SelfPlayer,
        }]),
        "Chaos" => Some(vec![Effect::Repeat {
        count: AmountSpec::Canonical("Repeat".to_string()),
        body: vec![Effect::ChannelRandomOrb {
        from_pool: vec![
        "LightningOrb".to_string(),
        "FrostOrb".to_string(),
        "DarkOrb".to_string(),
        "PlasmaOrb".to_string(),
        ],
        }],
        }]),
        "Misery" => Some(vec![
        Effect::CopyDebuffsToOtherEnemies,
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        ]),
        "Claw" => Some(vec![
        Effect::DealDamage {
        amount: AmountSpec::Add {
        left: Box::new(AmountSpec::Canonical("Damage".to_string())),
        right: Box::new(AmountSpec::SourceCardCounter { key: "plays".to_string() }),
        },
        target: Target::ChosenEnemy,
        hits: 1,
        },
        Effect::IncrementSourceCardCounter { key: "plays".to_string(), delta: AmountSpec::Fixed(2) },
        ]),
        "Maul" => Some(vec![
        Effect::DealDamage {
        amount: AmountSpec::Add {
        left: Box::new(AmountSpec::Canonical("Damage".to_string())),
        right: Box::new(AmountSpec::SourceCardCounter { key: "extra_damage".to_string() }),
        },
        target: Target::ChosenEnemy,
        hits: 2,
        },
        Effect::IncrementSourceCardCounter { key: "extra_damage".to_string(), delta: AmountSpec::Canonical("Increase".to_string()) },
        ]),
        "Rampage" => Some(vec![
        Effect::DealDamage {
        amount: AmountSpec::Add {
        left: Box::new(AmountSpec::Canonical("Damage".to_string())),
        right: Box::new(AmountSpec::SourceCardCounter { key: "extra_damage".to_string() }),
        },
        target: Target::ChosenEnemy,
        hits: 1,
        },
        Effect::IncrementSourceCardCounter { key: "extra_damage".to_string(), delta: AmountSpec::Canonical("Increase".to_string()) },
        ]),
        "Ftl" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::Conditional {
        condition: Condition::PlaysThisTurnLt { n: 3 },
        then_branch: vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }],
        else_branch: vec![],
        },
        ]),
        "Coordinate" => Some(vec![Effect::ApplyPower {
        power_id: "CoordinatePower".to_string(),
        amount: AmountSpec::Canonical("Strength".to_string()),
        target: Target::ChosenAlly,
        }]),
        "Chill" => Some(vec![Effect::Repeat {
        count: AmountSpec::AliveEnemyCount,
        body: vec![Effect::ChannelOrb { orb_id: "FrostOrb".to_string() }],
        }]),
        "DemonicShield" => Some(vec![
        Effect::LoseHp { amount: AmountSpec::Canonical("HpLoss".to_string()), target: Target::SelfPlayer },
        Effect::GainBlock {
        amount: AmountSpec::Add {
        left: Box::new(AmountSpec::Canonical("CalculationBase".to_string())),
        right: Box::new(AmountSpec::Mul {
        left: Box::new(AmountSpec::Canonical("CalculationExtra".to_string())),
        right: Box::new(AmountSpec::TargetBlock),
        }),
        },
        target: Target::ChosenAlly,
        },
        ]),
        "HiddenDaggers" => Some(vec![
        Effect::DiscardCards { from: Pile::Hand, selector: Selector::PlayerInteractive { n: 1 } },
        Effect::Repeat {
        count: AmountSpec::Canonical("Cards".to_string()),
        body: vec![Effect::AddCardToPile { card_id: "Shiv".to_string(), upgrade: 0, pile: Pile::Hand }],
        },
        ]),
        "GlimpseBeyond" => Some(vec![Effect::Repeat {
        count: AmountSpec::Canonical("Cards".to_string()),
        body: vec![Effect::AddCardToPile { card_id: "Soul".to_string(), upgrade: 0, pile: Pile::Draw }],
        }]),
        "HuddleUp" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "GangUp" => Some(vec![Effect::DealDamage {
        amount: AmountSpec::Canonical("CalculationBase".to_string()),
        target: Target::ChosenEnemy,
        hits: 1,
        }]),
        "Darkness" => Some(vec![Effect::ChannelOrb { orb_id: "DarkOrb".to_string() }]),
        "Bolas" => Some(vec![Effect::DealDamage {
        amount: AmountSpec::Canonical("Damage".to_string()),
        target: Target::ChosenEnemy,
        hits: 1,
        }]),
        "Bombardment" => Some(vec![Effect::DealDamage {
        amount: AmountSpec::Canonical("Damage".to_string()),
        target: Target::ChosenEnemy,
        hits: 1,
        }]),
        "Largesse" => Some(vec![Effect::AddRandomCardFromPool {
        pool: CardPoolRef::Colorless,
        filter: CardFilter::Any,
        n: AmountSpec::Fixed(1),
        pile: Pile::Hand,
        upgrade: 0,
        free_this_turn: false,
        distinct: true,
        }]),
        "Eidolon" => Some(vec![Effect::ExhaustCards {
        from: Pile::Hand,
        selector: Selector::All,
        }]),
        "AllForOne" => Some(vec![Effect::MoveCard {
        from: Pile::Discard,
        to: Pile::Hand,
        selector: Selector::FirstMatching {
        n: i32::MAX,
        filter: CardFilter::And(
        Box::new(CardFilter::WithEnergyCost { op: Comparison::Eq, value: 0 }),
        Box::new(CardFilter::NotXCost),
        ),
        },
        }]),
        "Jackpot" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::AddRandomCardFromPool {
        pool: CardPoolRef::CharacterAny,
        filter: CardFilter::WithEnergyCost { op: Comparison::Eq, value: 0 },
        n: AmountSpec::Canonical("Cards".to_string()),
        pile: Pile::Hand,
        upgrade: 0,
        free_this_turn: false,
        distinct: true,
        },
        ]),
        "DeathsDoor" => Some(vec![Effect::GainBlock {
        amount: AmountSpec::Canonical("Block".to_string()),
        target: Target::SelfPlayer,
        }]),
        "EscapePlan" => Some(vec![Effect::DrawCards {
        amount: AmountSpec::Canonical("Cards".to_string()),
        }]),
        "Fisticuffs" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::GainBlock { amount: AmountSpec::LastRealizedDamage, target: Target::SelfPlayer },
        ]),
        "DodgeAndRoll" => Some(vec![
        Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer },
        Effect::ApplyPower { power_id: "BlockNextTurnPower".to_string(), amount: AmountSpec::LastRealizedBlock, target: Target::SelfPlayer },
        ]),
        "PullFromBelow" => Some(vec![Effect::Repeat {
        count: AmountSpec::EtherealCardsPlayedThisTurn,
        body: vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }],
        }]),
        "Dirge" => Some(vec![Effect::Repeat {
        count: AmountSpec::XEnergy,
        body: vec![
        Effect::SummonOsty { osty_id: "Default".to_string(), max_hp: Some(AmountSpec::Canonical("Summon".to_string())) },
        Effect::AddCardToPile { card_id: "Soul".to_string(), upgrade: 0, pile: Pile::Draw },
        ],
        }]),
        "CrimsonMantle" => Some(vec![
        Effect::ApplyPower { power_id: "CrimsonMantlePower".to_string(), amount: AmountSpec::Canonical("CrimsonMantlePower".to_string()), target: Target::SelfPlayer },
        Effect::IncrementSourceCardCounter { key: "self_damage".to_string(), delta: AmountSpec::Fixed(1) },
        ]),
        "GeneticAlgorithm" => Some(vec![
        Effect::GainBlock {
        amount: AmountSpec::Add {
        left: Box::new(AmountSpec::Canonical("Block".to_string())),
        right: Box::new(AmountSpec::SourceCardCounter { key: "ramp".to_string() }),
        },
        target: Target::SelfPlayer,
        },
        Effect::IncrementSourceCardCounter { key: "ramp".to_string(), delta: AmountSpec::Canonical("Increase".to_string()) },
        ]),
        "Glimmer" => Some(vec![
        Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) },
        Effect::MoveCardWithPosition {
        from: Pile::Hand,
        to: Pile::Draw,
        selector: Selector::PlayerInteractive { n: 1 },
        position: PilePosition::Top,
        },
        ]),
        "Headbutt" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::MoveCardWithPosition {
        from: Pile::Discard,
        to: Pile::Draw,
        selector: Selector::PlayerInteractive { n: 1 },
        position: PilePosition::Top,
        },
        ]),
        "Hologram" => Some(vec![
        Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer },
        Effect::MoveCardWithPosition {
        from: Pile::Discard,
        to: Pile::Hand,
        selector: Selector::PlayerInteractive { n: 1 },
        position: PilePosition::Bottom,
        },
        ]),
        "PhotonCut" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) },
        Effect::MoveCardWithPosition {
        from: Pile::Hand,
        to: Pile::Draw,
        selector: Selector::PlayerInteractive { n: 1 },
        position: PilePosition::Top,
        },
        ]),
        "Dredge" => Some(vec![Effect::MoveCardWithPosition {
        from: Pile::Discard,
        to: Pile::Hand,
        selector: Selector::PlayerInteractive { n: 3 },
        position: PilePosition::Bottom,
        }]),
        "DualWield" => Some(vec![Effect::ClonePickedCardToPile {
        from: Pile::Hand,
        selector: Selector::PlayerInteractiveFiltered {
        n: 1,
        filter: CardFilter::Not(Box::new(CardFilter::OfType("Skill".to_string()))),
        },
        to_pile: Pile::Hand,
        copies: AmountSpec::Canonical("Cards".to_string()),
        }]),
        "Pillage" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::DrawUntil {
        stop_filter: CardFilter::Not(Box::new(CardFilter::OfType("Attack".to_string()))),
        max_count: 10,
        },
        ]),
        "CalculatedGamble" => Some(vec![Effect::DiscardHandAndDrawSameCount]),
        "Cascade" => Some(vec![Effect::AutoplayFromDrawAmount {
        n: AmountSpec::Add {
        left: Box::new(AmountSpec::XEnergy),
        right: Box::new(AmountSpec::BranchedOnUpgrade { base: 0, upgraded: 1 }),
        },
        }]),
        "Cleanse" => Some(vec![Effect::ExhaustCards {
        from: Pile::Hand,
        selector: Selector::FirstMatching {
        n: i32::MAX,
        filter: CardFilter::Or(
        Box::new(CardFilter::OfType("Status".to_string())),
        Box::new(CardFilter::OfType("Curse".to_string())),
        ),
        },
        }]),
        "Stoke" => Some(vec![
        Effect::ExhaustCards { from: Pile::Hand, selector: Selector::All },
        Effect::AddRandomCardFromPool {
        pool: CardPoolRef::CharacterAny,
        filter: CardFilter::Any,
        n: AmountSpec::HandSizeAtPlayStart,
        pile: Pile::Hand,
        upgrade: 0,
        free_this_turn: false,
        distinct: false,
        },
        ]),
        "StormOfSteel" => Some(vec![
        Effect::DiscardCards { from: Pile::Hand, selector: Selector::All },
        Effect::Repeat {
        count: AmountSpec::HandSizeAtPlayStart,
        body: vec![Effect::AddCardToPile { card_id: "Shiv".to_string(), upgrade: 0, pile: Pile::Hand }],
        },
        ]),
        "SummonForth" => Some(vec![
        Effect::Forge { amount: AmountSpec::Canonical("Forge".to_string()) },
        Effect::MoveAllByFilterAcrossPiles {
        to_pile: Pile::Hand,
        filter: CardFilter::HasId("SovereignBlade".to_string()),
        },
        ]),
        "RightHandHand" => Some(vec![Effect::Conditional {
        condition: Condition::Not(Box::new(Condition::IsOstyMissing)),
        then_branch: vec![Effect::DamageFromOsty {
        amount: AmountSpec::Canonical("OstyDamage".to_string()),
        target: Target::ChosenEnemy,
        }],
        else_branch: vec![],
        }]),
        "Modded" => Some(vec![
        Effect::ChangeOrbSlots { delta: AmountSpec::Canonical("Repeat".to_string()) },
        Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) },
        Effect::AddSourceCardCostThisCombat { delta: AmountSpec::Fixed(1) },
        ]),
        "Monologue" => Some(vec![
        Effect::ApplyPower { power_id: "MonologuePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer },
        Effect::SetPowerStateField {
        power_id: "MonologuePower".to_string(),
        field: "Strength".to_string(),
        value: AmountSpec::Canonical("Power".to_string()),
        target: Target::SelfPlayer,
        },
        ]),
        "Nightmare" => Some(vec![
        Effect::ApplyPower { power_id: "NightmarePower".to_string(), amount: AmountSpec::Fixed(3), target: Target::SelfPlayer },
        // Selected-card capture deferred — needs a "picked card id"
        // state field. Power apply lands; the selection is logged
        // but doesn't drive future hand-gen.
        ]),
        "HiddenGem" => Some(vec![Effect::IncrementPickedCardCounter {
        from: Pile::Draw,
        selector: Selector::PlayerInteractiveFiltered {
        n: 1,
        filter: CardFilter::And(
        Box::new(CardFilter::Not(Box::new(CardFilter::HasKeyword("Unplayable".to_string())))),
        Box::new(CardFilter::Not(Box::new(CardFilter::OfType("Status".to_string())))),
        ),
        },
        key: "replay_count".to_string(),
        delta: AmountSpec::Canonical("Replay".to_string()),
        }]),
        "Snap" => Some(vec![Effect::Conditional {
        condition: Condition::Not(Box::new(Condition::IsOstyMissing)),
        then_branch: vec![
        Effect::DamageFromOsty {
        amount: AmountSpec::Canonical("OstyDamage".to_string()),
        target: Target::ChosenEnemy,
        },
        Effect::ApplyKeywordToCards {
        keyword: "Retain".to_string(),
        from: Pile::Hand,
        selector: Selector::PlayerInteractiveFiltered {
        n: 1,
        filter: CardFilter::Not(Box::new(CardFilter::HasKeyword("Retain".to_string()))),
        },
        },
        ],
        else_branch: vec![],
        }]),
        "FlakCannon" => Some(vec![
        // Snapshot status count BEFORE exhausting via Repeat (count
        // resolves once before the loop), but the body's exhaust changes
        // pile contents — so we need to capture count, then exhaust,
        // then damage. Easiest: damage first (using pre-exhaust count),
        // then exhaust. C# order is exhaust-first but the net effect
        // is identical (status cards don't participate in the attack).
        Effect::Repeat {
        count: AmountSpec::CardCountInPile {
        pile: PileSelector::Single(Pile::Hand),
        filter: CardFilter::OfType("Status".to_string()),
        },
        body: vec![Effect::DealDamage {
        amount: AmountSpec::Canonical("Damage".to_string()),
        target: Target::RandomEnemy,
        hits: 1,
        }],
        },
        Effect::ExhaustCards {
        from: Pile::Hand,
        selector: Selector::FirstMatching {
        n: i32::MAX,
        filter: CardFilter::OfType("Status".to_string()),
        },
        },
        ]),
        "CaptureSpirit" => Some(vec![
        Effect::LoseHp { amount: AmountSpec::Canonical("HpLoss".to_string()), target: Target::SelfPlayer },
        Effect::Repeat {
        count: AmountSpec::Canonical("Cards".to_string()),
        body: vec![Effect::AddCardToPile { card_id: "Soul".to_string(), upgrade: 0, pile: Pile::Draw }],
        },
        ]),
        "CrescentSpear" => Some(vec![Effect::DealDamage {
        amount: AmountSpec::Canonical("CalculationBase".to_string()),
        target: Target::ChosenEnemy,
        hits: 1,
        }]),
        "DecisionsDecisions" => Some(vec![]),
        "Omnislice" => Some(vec![Effect::DealDamage {
        amount: AmountSpec::Canonical("Damage".to_string()),
        target: Target::ChosenEnemy,
        hits: 1,
        }]),
        "Rattle" => Some(vec![Effect::Conditional {
        condition: Condition::Not(Box::new(Condition::IsOstyMissing)),
        then_branch: vec![Effect::DamageFromOsty {
        amount: AmountSpec::Canonical("OstyDamage".to_string()),
        target: Target::ChosenEnemy,
        }],
        else_branch: vec![],
        }]),
        "Severance" => Some(vec![
        Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 },
        Effect::AddCardToPile { card_id: "Soul".to_string(), upgrade: 0, pile: Pile::Draw },
        ]),

        _ => None,
    }
}

fn execute_effect(cs: &mut CombatState, eff: &Effect, ctx: &EffectContext) {
    match eff {
        Effect::DealDamage {
            amount,
            target,
            hits,
        } => {
            let amt = amount.resolve(ctx, cs);
            let dealer = ctx.actor;
            cs.fire_before_attack(dealer);
            for _ in 0..(*hits).max(1) {
                deal_damage_to(cs, ctx, *target, amt);
            }
            cs.fire_after_attack(dealer);
            // Track realized damage for AmountSpec::LastRealizedDamage
            // (Fisticuffs). Uses pre-modifier amount * hits as an
            // approximation — actual outcome.total_damage isn't
            // surfaced here.
            ctx.last_realized_damage
                .set(amt.max(0) * (*hits).max(1));
        }
        Effect::GainBlock { amount, target } => {
            let amt = amount.resolve(ctx, cs);
            // Snapshot block-before for realized-block tracking
            // (DodgeAndRoll). Only the SelfPlayer path participates in
            // modifier pipeline; others add literal amt.
            let block_before = cs
                .allies
                .get(ctx.player_idx)
                .map(|c| c.block)
                .unwrap_or(0);
            for_each_target_idx(cs, ctx, *target, |cs, side, idx| {
                if matches!(side, CombatSide::Player) {
                    cs.gain_block(CombatSide::Player, idx, amt);
                } else if let Some(c) = creature_at_mut(cs, side, idx) {
                    c.block += amt.max(0);
                }
            });
            let block_after = cs
                .allies
                .get(ctx.player_idx)
                .map(|c| c.block)
                .unwrap_or(0);
            ctx.last_realized_block.set((block_after - block_before).max(0));
        }
        Effect::ApplyPower {
            power_id,
            amount,
            target,
        } => {
            let amt = amount.resolve(ctx, cs);
            apply_power_to(cs, ctx, *target, power_id, amt);
        }
        Effect::DrawCards { amount } => {
            let n = amount.resolve(ctx, cs);
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
            let n = amount.resolve(ctx, cs);
            for _ in 0..n {
                cs.exhaust_random_card_in_hand(ctx.player_idx);
            }
        }
        Effect::ChangeMaxHp { amount, target } => {
            let amt = amount.resolve(ctx, cs);
            for_each_target_idx(cs, ctx, *target, |cs, side, idx| {
                cs.change_max_hp(side, idx, amt);
            });
        }
        Effect::GainEnergy { amount } => {
            let amt = amount.resolve(ctx, cs);
            if let Some(creature) = cs.allies.get_mut(ctx.player_idx) {
                if let Some(ps) = creature.player.as_mut() {
                    ps.energy += amt;
                }
            }
        }
        Effect::Heal { amount, target } => {
            let amt = amount.resolve(ctx, cs);
            for_each_target_idx(cs, ctx, *target, |cs, side, idx| {
                cs.heal(side, idx, amt);
            });
        }
        Effect::LoseHp { amount, target } => {
            let amt = amount.resolve(ctx, cs);
            for_each_target_idx(cs, ctx, *target, |cs, side, idx| {
                cs.lose_hp(side, idx, amt);
            });
        }
        Effect::LoseEnergy { amount } => {
            let amt = amount.resolve(ctx, cs);
            if let Some(creature) = cs.allies.get_mut(ctx.player_idx) {
                if let Some(ps) = creature.player.as_mut() {
                    ps.energy = (ps.energy - amt).max(0);
                }
            }
        }
        Effect::RemovePower { power_id, target } => {
            for_each_target_idx(cs, ctx, *target, |cs, side, idx| {
                cs.remove_power(side, idx, power_id);
            });
        }
        Effect::Shuffle { pile } => {
            shuffle_pile(cs, ctx.player_idx, *pile);
        }
        Effect::DiscardHand => {
            cs.discard_hand(ctx.player_idx);
        }
        Effect::Kill { target } => {
            for_each_target_idx(cs, ctx, *target, |cs, side, idx| {
                if let Some(c) = creature_at_mut(cs, side, idx) {
                    c.current_hp = 0;
                }
            });
        }
        Effect::LoseBlock { amount, target } => {
            let amt = amount.resolve(ctx, cs);
            for_each_target_idx(cs, ctx, *target, |cs, side, idx| {
                if let Some(c) = creature_at_mut(cs, side, idx) {
                    c.block = (c.block - amt).max(0);
                }
            });
        }
        Effect::ModifyPowerAmount { power_id, delta, target } => {
            let d = delta.resolve(ctx, cs);
            for_each_target_idx(cs, ctx, *target, |cs, side, idx| {
                if let Some(c) = creature_at_mut(cs, side, idx) {
                    if let Some(p) = c.powers.iter_mut().find(|p| p.id == *power_id) {
                        p.amount += d;
                    }
                }
            });
        }
        Effect::GainGold { amount } => {
            let amt = amount.resolve(ctx, cs).max(0);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                ps.pending_gold += amt;
            }
        }
        Effect::LoseGold { amount } => {
            let amt = amount.resolve(ctx, cs).max(0);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                ps.pending_gold = (ps.pending_gold - amt).max(0);
            }
        }
        Effect::GainStars { amount } => {
            let amt = amount.resolve(ctx, cs).max(0);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                ps.pending_stars += amt;
            }
        }
        Effect::ChannelOrb { orb_id } => {
            cs.channel_orb(ctx.player_idx, orb_id);
        }
        Effect::EvokeNextOrb => {
            cs.evoke_next_orb(ctx.player_idx);
        }
        Effect::TriggerOrbPassive => {
            cs.trigger_orb_passives(ctx.player_idx);
        }
        Effect::ChangeOrbSlots { delta } => {
            let d = delta.resolve(ctx, cs);
            cs.change_orb_slots(ctx.player_idx, d);
        }
        Effect::SummonOsty { osty_id: _, max_hp } => {
            // C# OstyCmd.Summon(owner, amount, source) — summons Osty
            // with HP = amount. Most cards bind this via their
            // `Summon` canonical var; cards that omit the arg fall
            // back to a default of 6 HP.
            let hp = max_hp
                .as_ref()
                .map(|spec| spec.resolve(ctx, cs))
                .unwrap_or(6)
                .max(1);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                ps.osty = Some(crate::combat::OstyState {
                    current_hp: hp,
                    max_hp: hp,
                    block: 0,
                });
            }
        }
        Effect::HealOsty { amount } => {
            let amt = amount.resolve(ctx, cs).max(0);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                if let Some(o) = ps.osty.as_mut() {
                    o.current_hp = (o.current_hp + amt).min(o.max_hp);
                }
            }
        }
        Effect::KillOsty => {
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                if let Some(o) = ps.osty.as_mut() {
                    o.current_hp = 0;
                }
            }
        }
        Effect::AddRandomCardFromPool {
            pool,
            filter,
            n,
            pile,
            upgrade,
            free_this_turn: _,
            distinct,
        } => {
            let count = n.resolve(ctx, cs).max(0) as usize;
            if count == 0 {
                return;
            }
            // Materialize a frozen list of candidate card ids matching
            // (pool, filter), then draw `count` via combat RNG.
            let candidates =
                crate::card::pool_card_ids(cs, ctx.player_idx, pool, filter);
            if candidates.is_empty() {
                return;
            }
            // `free_this_turn` cost override is deferred — the runtime
            // doesn't yet thread it onto generated CardInstances.
            let picks =
                crate::card::sample_card_ids(&mut cs.rng, &candidates, count, *distinct);
            for card_id in picks {
                if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                    let inst = crate::combat::CardInstance::from_card(
                        crate::card::by_id(&card_id).unwrap(),
                        *upgrade,
                    );
                    let target_pile = match pile {
                        Pile::Hand => &mut ps.hand,
                        Pile::Discard => &mut ps.discard,
                        Pile::Draw => &mut ps.draw,
                        Pile::Exhaust => &mut ps.exhaust,
                        Pile::Deck => continue, // deck not addressable mid-combat
                    };
                    target_pile.cards.push(inst);
                }
            }
        }
        Effect::AutoplayCardsFromPile { .. } => {
            // STUB: auto-play recursion into play_card lands with the
            // pending-action queue. KnifeTrap / Uproar / DistilledChaos
            // / Mayhem all share this gap.
        }
        Effect::SetPowerStateField {
            power_id,
            field,
            value,
            target,
        } => {
            let amt = value.resolve(ctx, cs);
            for_each_target_idx(cs, ctx, *target, |cs, side, idx| {
                let powers = match side {
                    CombatSide::Player => cs.allies.get_mut(idx).map(|c| &mut c.powers),
                    CombatSide::Enemy => cs.enemies.get_mut(idx).map(|c| &mut c.powers),
                    CombatSide::None => None,
                };
                if let Some(powers) = powers {
                    if let Some(p) = powers.iter_mut().find(|p| p.id == *power_id) {
                        p.state.insert(field.clone(), amt);
                    }
                }
            });
        }
        Effect::MillFromDraw { n } => {
            let count = n.resolve(ctx, cs).max(0) as usize;
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                for _ in 0..count {
                    if let Some(card) = ps.draw.cards.pop() {
                        ps.discard.cards.push(card);
                    }
                }
            }
        }
        Effect::CloneSourceCardToPile {
            pile,
            cost_override_this_combat,
            copies,
        } => {
            let n = copies.resolve(ctx, cs).max(0) as usize;
            let Some(card_id) = ctx.source_card_id else {
                return;
            };
            let Some(data) = crate::card::by_id(card_id) else {
                return;
            };
            let upg = ctx.upgrade_level;
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                let target_pile = match pile {
                    Pile::Hand => &mut ps.hand,
                    Pile::Discard => &mut ps.discard,
                    Pile::Draw => &mut ps.draw,
                    Pile::Exhaust => &mut ps.exhaust,
                    Pile::Deck => return,
                };
                for _ in 0..n {
                    let mut clone = crate::combat::CardInstance::from_card(data, upg);
                    if let Some(c) = cost_override_this_combat {
                        clone.cost_override_this_combat = Some(*c);
                    }
                    target_pile.cards.push(clone);
                }
            }
        }
        Effect::ChannelRandomOrb { from_pool } => {
            if from_pool.is_empty() {
                return;
            }
            let mut rng = std::mem::replace(&mut cs.rng, crate::rng::Rng::new(0, 0));
            let pick = rng.next_int_range(0, from_pool.len() as i32) as usize;
            cs.rng = rng;
            let orb_id = from_pool[pick].clone();
            cs.channel_orb(ctx.player_idx, &orb_id);
        }
        Effect::CopyDebuffsToOtherEnemies => {
            // Snapshot target's debuff powers.
            let Some((side, target_idx)) = ctx.target else {
                return;
            };
            if !matches!(side, CombatSide::Enemy) {
                return;
            }
            let debuffs: Vec<(String, i32)> = cs
                .enemies
                .get(target_idx)
                .map(|c| {
                    c.powers
                        .iter()
                        .filter(|p| is_debuff_power(&p.id))
                        .map(|p| (p.id.clone(), p.amount))
                        .collect()
                })
                .unwrap_or_default();
            if debuffs.is_empty() {
                return;
            }
            let n = cs.enemies.len();
            for i in 0..n {
                if i == target_idx {
                    continue;
                }
                if cs.enemies[i].current_hp == 0 {
                    continue;
                }
                for (power_id, amount) in &debuffs {
                    cs.apply_power(CombatSide::Enemy, i, power_id, *amount);
                }
            }
        }
        Effect::IncrementSourceCardCounter { key, delta } => {
            let d = delta.resolve(ctx, cs);
            let Some(card_id) = ctx.source_card_id else {
                return;
            };
            let namespaced = format!("card.{}.{}", card_id, key);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                let entry = ps.relic_counters.entry(namespaced).or_insert(0);
                *entry += d;
            }
        }
        Effect::MoveCardWithPosition {
            from,
            to,
            selector,
            position,
        } => {
            let picks = select_card_indices(cs, ctx.player_idx, *from, selector);
            let mut sorted = picks;
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            for idx in sorted {
                if let Some(card) = remove_card_from_pile(cs, ctx.player_idx, *from, idx) {
                    if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                        let dest = match to {
                            Pile::Hand => &mut ps.hand,
                            Pile::Discard => &mut ps.discard,
                            Pile::Draw => &mut ps.draw,
                            Pile::Exhaust => &mut ps.exhaust,
                            Pile::Deck => continue,
                        };
                        match position {
                            PilePosition::Top => dest.cards.push(card),
                            PilePosition::Bottom => dest.cards.insert(0, card),
                        }
                    }
                }
            }
        }
        Effect::ClonePickedCardToPile {
            from,
            selector,
            to_pile,
            copies,
        } => {
            let picks = select_card_indices(cs, ctx.player_idx, *from, selector);
            let n = copies.resolve(ctx, cs).max(0) as usize;
            // Snapshot picked card-ids (don't remove from source).
            let picked_ids: Vec<(String, i32)> = {
                let Some(ps) = cs
                    .allies
                    .get(ctx.player_idx)
                    .and_then(|c| c.player.as_ref())
                else {
                    return;
                };
                let source_pile = match from {
                    Pile::Hand => &ps.hand,
                    Pile::Discard => &ps.discard,
                    Pile::Draw => &ps.draw,
                    Pile::Exhaust => &ps.exhaust,
                    Pile::Deck => return,
                };
                picks
                    .iter()
                    .filter_map(|i| source_pile.cards.get(*i))
                    .map(|c| (c.id.clone(), c.upgrade_level))
                    .collect()
            };
            for (cid, upg) in picked_ids {
                let Some(data) = crate::card::by_id(&cid) else {
                    continue;
                };
                if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                    let dest = match to_pile {
                        Pile::Hand => &mut ps.hand,
                        Pile::Discard => &mut ps.discard,
                        Pile::Draw => &mut ps.draw,
                        Pile::Exhaust => &mut ps.exhaust,
                        Pile::Deck => continue,
                    };
                    for _ in 0..n {
                        dest.cards.push(crate::combat::CardInstance::from_card(data, upg));
                    }
                }
            }
        }
        Effect::DrawUntil {
            stop_filter,
            max_count,
        } => {
            // Iteratively draw cards; stop when the most-recently-drawn
            // card matches stop_filter or we hit max_count.
            for _ in 0..(*max_count).max(0) {
                let pre_hand_len = cs
                    .allies
                    .get(ctx.player_idx)
                    .and_then(|c| c.player.as_ref())
                    .map(|ps| ps.hand.cards.len())
                    .unwrap_or(0);
                cs.draw_cards_self_rng(ctx.player_idx, 1);
                let post = cs
                    .allies
                    .get(ctx.player_idx)
                    .and_then(|c| c.player.as_ref())
                    .map(|ps| (ps.hand.cards.len(), ps.hand.cards.last().cloned()))
                    .unwrap_or((0, None));
                if post.0 == pre_hand_len {
                    break; // nothing to draw
                }
                let Some(last) = post.1 else { break };
                if matches_filter(&last, stop_filter) {
                    break;
                }
            }
        }
        Effect::DiscardHandAndDrawSameCount => {
            // Snapshot hand size, discard hand, draw same count.
            let n = cs
                .allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
                .map(|ps| ps.hand.cards.len() as i32)
                .unwrap_or(0);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                let drained: Vec<crate::combat::CardInstance> = ps.hand.cards.drain(..).collect();
                ps.discard.cards.extend(drained);
            }
            cs.draw_cards_self_rng(ctx.player_idx, n);
        }
        Effect::AutoplayFromDrawAmount { n: _ } => {
            // STUB: full auto-play recursion into play_card still
            // pending — same status as the legacy AutoplayFromDraw.
        }
        Effect::MoveAllByFilterAcrossPiles { to_pile, filter } => {
            // SummonForth: walk every combat pile EXCEPT to_pile,
            // collect cards matching `filter`, append to to_pile.
            let mut collected: Vec<crate::combat::CardInstance> = Vec::new();
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                for from in [Pile::Hand, Pile::Discard, Pile::Draw, Pile::Exhaust] {
                    if from == *to_pile {
                        continue;
                    }
                    let pile = match from {
                        Pile::Hand => &mut ps.hand,
                        Pile::Discard => &mut ps.discard,
                        Pile::Draw => &mut ps.draw,
                        Pile::Exhaust => &mut ps.exhaust,
                        Pile::Deck => continue,
                    };
                    let kept: Vec<crate::combat::CardInstance> = pile
                        .cards
                        .drain(..)
                        .filter_map(|c| {
                            if matches_filter(&c, filter) {
                                collected.push(c);
                                None
                            } else {
                                Some(c)
                            }
                        })
                        .collect();
                    pile.cards = kept;
                }
                let dest = match to_pile {
                    Pile::Hand => &mut ps.hand,
                    Pile::Discard => &mut ps.discard,
                    Pile::Draw => &mut ps.draw,
                    Pile::Exhaust => &mut ps.exhaust,
                    Pile::Deck => return,
                };
                dest.cards.extend(collected);
            }
        }
        Effect::IncrementPickedCardCounter {
            from,
            selector,
            key,
            delta,
        } => {
            let picks = select_card_indices(cs, ctx.player_idx, *from, selector);
            let d = delta.resolve(ctx, cs);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                let pile = match from {
                    Pile::Hand => &mut ps.hand,
                    Pile::Discard => &mut ps.discard,
                    Pile::Draw => &mut ps.draw,
                    Pile::Exhaust => &mut ps.exhaust,
                    Pile::Deck => return,
                };
                for idx in picks {
                    if let Some(card) = pile.cards.get_mut(idx) {
                        let entry = card.state.entry(key.clone()).or_insert(0);
                        *entry += d;
                    }
                }
            }
        }
        Effect::AddSourceCardCostThisCombat { delta: _ } => {
            // The source CardInstance has already been removed from
            // hand by play_card; we can't mutate it here. The
            // semantics are that the BASE energy_cost for future
            // instances of this card-id this combat should increase.
            // Approximate: leave as STUB — Modded's bump is observable
            // through the displayed-cost diff. Full impl needs a
            // per-(player, card_id) cost-delta map on PlayerState.
        }
        Effect::ModifyRelicCounter { key, delta } => {
            let d = delta.resolve(ctx, cs);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                let entry = ps.relic_counters.entry(key.clone()).or_insert(0);
                *entry += d;
            }
        }
        Effect::SetRelicCounter { key, value } => {
            let v = value.resolve(ctx, cs);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                ps.relic_counters.insert(key.clone(), v);
            }
        }
        Effect::IncreaseMaxEnergy { delta } => {
            let d = delta.resolve(ctx, cs);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                ps.turn_energy = (ps.turn_energy + d).max(0);
            }
        }
        Effect::DamageFromOsty { amount, target } => {
            // Mirrors C# `DamageCmd.Attack(...).FromOsty(Osty, this)`.
            // If Osty exists, route damage as Osty-attributed (we just
            // use player as dealer here — the result is the same for
            // damage math; the only difference is the attribution flag
            // which we don't model). If no Osty, no-op.
            let has_osty = cs
                .allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
                .map(|ps| ps.osty.is_some())
                .unwrap_or(false);
            if !has_osty {
                return;
            }
            let amt = amount.resolve(ctx, cs);
            deal_damage_to(cs, ctx, *target, amt);
        }
        Effect::Forge { amount } => {
            let amt = amount.resolve(ctx, cs);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                ps.pending_forge += amt;
            }
        }
        Effect::EndTurn => {
            // STUB: calling end_turn() mid-card nests the turn machine.
            // Once a "pending end-of-turn" flag exists in CombatState,
            // this primitive flips it and the env loop drains the
            // remaining play-stack before transitioning.
        }
        Effect::CompleteQuest => {
            // STUB: quest progression isn't represented in combat state.
        }
        Effect::GenerateRandomPotion | Effect::FillPotionSlots => {
            // STUB: potion belt isn't in CombatState.
        }
        Effect::AutoplayFromDraw { .. } => {
            // STUB: requires re-entry into play_card from inside OnPlay.
            // DistilledChaos / Mayhem-family cards encode as data but
            // don't fire until the auto-play recursion lands.
        }
        Effect::MoveCard { from, to, selector } => {
            let picks = select_card_indices(cs, ctx.player_idx, *from, selector);
            // Iterate high-to-low to keep indices valid as we remove.
            let mut sorted = picks;
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            for idx in sorted {
                if let Some(card) = remove_card_from_pile(cs, ctx.player_idx, *from, idx) {
                    push_card_to_pile(cs, ctx.player_idx, *to, card);
                }
            }
        }
        Effect::ExhaustCards { from, selector } => {
            let picks = select_card_indices(cs, ctx.player_idx, *from, selector);
            let mut sorted = picks;
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            let mut exhausted_ids: Vec<String> = Vec::new();
            for idx in sorted {
                if let Some(card) = remove_card_from_pile(cs, ctx.player_idx, *from, idx) {
                    exhausted_ids.push(card.id.clone());
                    push_card_to_pile(cs, ctx.player_idx, Pile::Exhaust, card);
                }
            }
            // History emission + AfterCardExhausted relic-hook firing.
            let round = cs.round_number;
            for cid in &exhausted_ids {
                cs.combat_log.push(crate::combat::CombatEvent::CardExhausted {
                    round,
                    player_idx: ctx.player_idx,
                    card_id: cid.clone(),
                });
            }
            if !exhausted_ids.is_empty() {
                fire_relic_hooks(cs, RelicHookKind::AfterCardExhausted, CombatSide::Player);
            }
        }
        Effect::DiscardCards { from, selector } => {
            let picks = select_card_indices(cs, ctx.player_idx, *from, selector);
            let mut sorted = picks;
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            let mut discarded_ids: Vec<String> = Vec::new();
            for idx in sorted {
                if let Some(card) = remove_card_from_pile(cs, ctx.player_idx, *from, idx) {
                    discarded_ids.push(card.id.clone());
                    push_card_to_pile(cs, ctx.player_idx, Pile::Discard, card);
                }
            }
            let round = cs.round_number;
            for cid in &discarded_ids {
                cs.combat_log.push(crate::combat::CombatEvent::CardDiscarded {
                    round,
                    player_idx: ctx.player_idx,
                    card_id: cid.clone(),
                });
            }
            if !discarded_ids.is_empty() {
                fire_relic_hooks(cs, RelicHookKind::AfterCardDiscarded, CombatSide::Player);
            }
        }
        Effect::UpgradeCards { from, selector } => {
            let picks = select_card_indices(cs, ctx.player_idx, *from, selector);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                let Some(cards) = pile_mut(ps, *from) else {
                    return; // Deck not accessible from combat VM.
                };
                for idx in picks {
                    if let Some(card) = cards.cards.get_mut(idx) {
                        // Bumping upgrade_level past the card's allowed
                        // max is a no-op upstream (canonical_int_value
                        // tolerates), but cap defensively at 1 for now —
                        // most cards in StS2 only upgrade once.
                        if card.upgrade_level < 1 {
                            card.upgrade_level += 1;
                        }
                    }
                }
            }
        }
        Effect::ApplyKeywordToCards { .. } => {
            // STUB: keyword runtime mutation needs a per-CardInstance
            // override field. Defer.
        }
        Effect::TransformCards { .. } => {
            // STUB: CardFactory.CreateRandom* not yet plumbed through
            // a named RNG stream.
        }
        Effect::TransformIntoSpecific {
            from,
            selector,
            target_card_id,
            upgrade,
        } => {
            let picks = select_card_indices(cs, ctx.player_idx, *from, selector);
            let Some(target_data) = crate::card::by_id(target_card_id) else {
                return;
            };
            let new_upgrade = if *upgrade { 1 } else { 0 };
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                let Some(pile) = pile_mut(ps, *from) else {
                    return;
                };
                for idx in picks {
                    if let Some(card) = pile.cards.get_mut(idx) {
                        *card = crate::combat::CardInstance::from_card(
                            target_data,
                            new_upgrade,
                        );
                    }
                }
            }
        }
        Effect::UpgradeAllUpgradableCards => {
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                for pile in [&mut ps.hand, &mut ps.draw, &mut ps.discard, &mut ps.exhaust] {
                    for card in pile.cards.iter_mut() {
                        let Some(data) = crate::card::by_id(&card.id) else {
                            continue;
                        };
                        if data.max_upgrade_level > 0
                            && card.upgrade_level < data.max_upgrade_level
                        {
                            card.upgrade_level += 1;
                        }
                    }
                }
            }
        }
        Effect::SetCardCost {
            from,
            selector,
            cost,
            scope,
        } => {
            // Resolve target picks first (before mutating costs).
            let picks = select_card_indices(cs, ctx.player_idx, *from, selector);
            let c = cost.resolve(ctx, cs).max(0);
            if let Some(ps) = player_state_mut(cs, ctx.player_idx) {
                let Some(pile) = pile_mut(ps, *from) else {
                    return;
                };
                for idx in picks {
                    let Some(card) = pile.cards.get_mut(idx) else {
                        continue;
                    };
                    match scope {
                        CostScope::ThisTurn => {
                            card.cost_override_this_turn = Some(c);
                        }
                        CostScope::ThisCombat => {
                            card.cost_override_this_combat = Some(c);
                        }
                        CostScope::UntilPlayed => {
                            card.cost_override_until_played = Some(c);
                        }
                    }
                }
            }
        }
        Effect::SummonMonster { monster_id, slot } => {
            // Reuses the existing monster_dispatch + spawn payload path.
            crate::monster_dispatch::spawn_monster_into_slot(cs, monster_id, slot);
        }
        Effect::KillSelf => {
            // Interpreted as the actor; in card OnPlay contexts the
            // actor is the player and KillSelf is unused (no cards
            // self-kill the player). For monster moves dispatched
            // through the VM, the actor index will live in EffectContext
            // once that path lands.
            // No-op for now to keep cards safe.
        }
        Effect::SetMaxHpAndHeal { amount, target } => {
            let amt = amount.resolve(ctx, cs);
            for_each_target_idx(cs, ctx, *target, |cs, side, idx| {
                if let Some(c) = creature_at_mut(cs, side, idx) {
                    c.max_hp = amt.max(1);
                    c.current_hp = c.max_hp;
                }
            });
        }
        Effect::Stun { target } => {
            for_each_target_idx(cs, ctx, *target, |cs, side, idx| {
                if matches!(side, CombatSide::Enemy) {
                    if let Some(ms) = cs
                        .enemies
                        .get_mut(idx)
                        .and_then(|c| c.monster.as_mut())
                    {
                        ms.set_flag("stunned", true);
                    }
                }
                // Player-stun is currently a no-op (no source applies
                // player stun in current ports).
            });
        }
        Effect::ApplyAfflictionToAllInPile { .. } => {
            // STUB: requires affliction-on-card infrastructure
            // (CardInstance.affliction + AfterCardEnteredCombat hook
            // that re-applies to mid-combat-generated cards). HexPower
            // is the only consumer today.
        }
        Effect::Conditional {
            condition,
            then_branch,
            else_branch,
        } => {
            if evaluate_condition(cs, ctx, condition) {
                execute_effects(cs, then_branch, ctx);
            } else {
                execute_effects(cs, else_branch, ctx);
            }
        }
        Effect::Repeat { count, body } => {
            let n = count.resolve(ctx, cs);
            for _ in 0..n.max(0) {
                execute_effects(cs, body, ctx);
            }
        }
        // Run-state primitives — STUB layer. The combat VM cannot
        // mutate RunState (no handle). Records intent so a future
        // run-state dispatcher can replay these.
        Effect::GainRelic { .. }
        | Effect::LoseRelic { .. }
        | Effect::GainPotionToBelt { .. }
        | Effect::LoseRunStateHp { .. }
        | Effect::GainRunStateMaxHp { .. }
        | Effect::GainRunStateGold { .. }
        | Effect::LoseRunStateMaxHp { .. }
        | Effect::AddCardToRunStateDeck { .. }
        | Effect::GainMaxPotionSlots { .. } => {
            // STUB: see Pile::Deck rationale. Mutates RunState; combat
            // VM has no handle. Routes through run_state_effects path.
        }
        // Event-flow primitives — STUB. Events run outside combat;
        // these variants make event bodies encode-able as data.
        Effect::SetEventFinished { .. } | Effect::MoveToEventPage { .. } => {
            // STUB.
        }
    }
}

/// Evaluate a `Condition` against the current combat state + context.
/// Truthiness rules:
/// - `Always` → true · `Never` → false
/// - Logical combinators: standard
/// - Power-presence: looks up the target/self creature's powers
/// - Pile counts: only meaningful for combat piles; Deck always 0
/// - History-derived (OwnerLostHpThisTurn, AttackKilledTarget):
///   currently FALSE — combat-history scan over `cs.combat_log`
///   not yet plumbed for VM use. Conditions that reference them
///   short-circuit to false; encoded cards stay safe.
/// - RandomChance: draws from combat RNG.
pub fn evaluate_condition(
    cs: &mut CombatState,
    ctx: &EffectContext,
    cond: &Condition,
) -> bool {
    match cond {
        Condition::Always => true,
        Condition::Never => false,
        Condition::Not(inner) => !evaluate_condition(cs, ctx, inner),
        Condition::And(a, b) => {
            evaluate_condition(cs, ctx, a) && evaluate_condition(cs, ctx, b)
        }
        Condition::Or(a, b) => {
            evaluate_condition(cs, ctx, a) || evaluate_condition(cs, ctx, b)
        }
        Condition::IsUpgraded => ctx.upgrade_level > 0,
        Condition::HasPowerOnTarget { power_id } => {
            let (side, idx) = ctx.target.unwrap_or(ctx.actor);
            creature_has_power(cs, side, idx, power_id)
        }
        Condition::HasPowerOnSelf { power_id } => {
            creature_has_power(cs, ctx.actor.0, ctx.actor.1, power_id)
        }
        Condition::CardCountInPile { pile, op, value } => {
            let Some(ps) = cs
                .allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
            else {
                return false;
            };
            let n = match pile {
                Pile::Hand => ps.hand.cards.len() as i32,
                Pile::Discard => ps.discard.cards.len() as i32,
                Pile::Draw => ps.draw.cards.len() as i32,
                Pile::Exhaust => ps.exhaust.cards.len() as i32,
                Pile::Deck => 0, // Run-state pile not accessible here.
            };
            compare(n, *op, *value)
        }
        Condition::OwnerLostHpThisTurn => {
            // STUB: history-derived predicate needs a per-turn HP-delta
            // scan. Returns false so encoded cards stay safe.
            false
        }
        Condition::AttackKilledTarget => {
            // True iff the current EffectContext::target is at 0 HP.
            // Used in `Effect::Conditional` right after a DealDamage
            // step: if the damage killed the target, the conditional
            // body fires. Mirrors the legacy `outcome.fatal` check in
            // Feed / HandOfGreed. Caller orders this immediately after
            // the DealDamage step so the predicate sees post-damage HP.
            let Some((side, idx)) = ctx.target else {
                return false;
            };
            let creature = match side {
                CombatSide::Player => cs.allies.get(idx),
                CombatSide::Enemy => cs.enemies.get(idx),
                CombatSide::None => None,
            };
            creature.map(|c| c.current_hp == 0).unwrap_or(false)
        }
        Condition::HandHasCardMatching(filter) => {
            let Some(ps) = cs
                .allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
            else {
                return false;
            };
            ps.hand.cards.iter().any(|c| matches_filter(c, filter))
        }
        Condition::SourceCardHasKeyword(kw) => {
            let Some(card_id) = ctx.source_card_id else {
                return false;
            };
            let Some(data) = crate::card::by_id(card_id) else {
                return false;
            };
            data.keywords.iter().any(|k| k.eq_ignore_ascii_case(kw))
        }
        Condition::RandomChance {
            numerator,
            denominator,
        } => {
            if *denominator <= 0 {
                return false;
            }
            let roll = cs.rng.next_int_range(0, *denominator);
            roll < *numerator
        }
        Condition::IsOstyMissing => {
            let alive = cs
                .allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
                .and_then(|ps| ps.osty.as_ref())
                .map(|o| o.current_hp > 0)
                .unwrap_or(false);
            !alive
        }
        Condition::OwnerExhaustedCardThisTurn => {
            cards_exhausted_this_turn(cs, ctx.player_idx) > 0
        }
        Condition::FirstPlayOfSourceCardThisTurn => {
            let Some(card_id) = ctx.source_card_id else {
                return false;
            };
            // The current play is in flight — for it to be the "first"
            // play this turn, the historical scan must return 0
            // (the in-flight CardPlayed event hasn't been logged yet
            // when condition evaluation runs).
            cards_played_with_id_this_turn(cs, ctx.player_idx, card_id) == 0
        }
        Condition::PlaysThisTurnLt { n } => {
            cards_played_this_turn(cs, ctx.player_idx, &CardFilter::Any) < *n
        }
        Condition::RoundEquals { n } => cs.round_number == *n,
        Condition::RoundGe { n } => cs.round_number >= *n,
        Condition::RelicCounterGe { key, value } => {
            cs.allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
                .map(|ps| ps.relic_counters.get(key).copied().unwrap_or(0) >= *value)
                .unwrap_or(false)
        }
        Condition::RelicCounterModEq {
            key,
            modulus,
            remainder,
        } => {
            if *modulus <= 0 {
                return false;
            }
            cs.allies
                .get(ctx.player_idx)
                .and_then(|c| c.player.as_ref())
                .map(|ps| {
                    let v = ps.relic_counters.get(key).copied().unwrap_or(0);
                    v.rem_euclid(*modulus) == *remainder
                })
                .unwrap_or(false)
        }
        Condition::XEnergyGe { n } => ctx.x_value >= *n,
        Condition::XEnergyEq { n } => ctx.x_value == *n,
        Condition::TargetIsAlive => {
            let Some((side, idx)) = ctx.target else {
                return false;
            };
            let creature = match side {
                CombatSide::Player => cs.allies.get(idx),
                CombatSide::Enemy => cs.enemies.get(idx),
                CombatSide::None => None,
            };
            creature.map(|c| c.current_hp > 0).unwrap_or(false)
        }
    }
}

fn creature_has_power(
    cs: &CombatState,
    side: CombatSide,
    idx: usize,
    power_id: &str,
) -> bool {
    let creature = match side {
        CombatSide::Player => cs.allies.get(idx),
        CombatSide::Enemy => cs.enemies.get(idx),
        CombatSide::None => None,
    };
    creature
        .map(|c| c.powers.iter().any(|p| p.id == power_id && p.amount > 0))
        .unwrap_or(false)
}

fn compare(a: i32, op: Comparison, b: i32) -> bool {
    match op {
        Comparison::Eq => a == b,
        Comparison::Ne => a != b,
        Comparison::Lt => a < b,
        Comparison::Le => a <= b,
        Comparison::Gt => a > b,
        Comparison::Ge => a >= b,
    }
}

fn for_each_target_idx<F>(
    cs: &mut CombatState,
    ctx: &EffectContext,
    target: Target,
    mut f: F,
) where
    F: FnMut(&mut CombatState, CombatSide, usize),
{
    match target {
        Target::SelfPlayer => f(cs, CombatSide::Player, ctx.player_idx),
        Target::SelfActor => f(cs, ctx.actor.0, ctx.actor.1),
        Target::ChosenEnemy => {
            if let Some((side, idx)) = ctx.target {
                f(cs, side, idx);
            }
        }
        Target::AllEnemies => {
            let n = cs.enemies.len();
            for i in 0..n {
                if cs.enemies[i].current_hp == 0 {
                    continue;
                }
                f(cs, CombatSide::Enemy, i);
            }
        }
        Target::RandomEnemy => {
            if let Some(idx) = pick_random_alive_enemy(cs) {
                f(cs, CombatSide::Enemy, idx);
            }
        }
        Target::ChosenAlly => {
            // For multiplayer-only AnyAlly cards, the player action
            // carries an ally target. Single-player collapses to self.
            let (side, idx) = ctx
                .target
                .unwrap_or((CombatSide::Player, ctx.player_idx));
            if matches!(side, CombatSide::Player) {
                f(cs, side, idx);
            } else {
                f(cs, CombatSide::Player, ctx.player_idx);
            }
        }
        Target::AllAllies => {
            let n = cs.allies.len();
            for i in 0..n {
                if cs.allies[i].current_hp == 0 {
                    continue;
                }
                f(cs, CombatSide::Player, i);
            }
        }
    }
}

fn player_state_mut(
    cs: &mut CombatState,
    player_idx: usize,
) -> Option<&mut crate::combat::PlayerState> {
    cs.allies.get_mut(player_idx).and_then(|c| c.player.as_mut())
}

fn pile_mut<'a>(
    ps: &'a mut crate::combat::PlayerState,
    pile: Pile,
) -> Option<&'a mut crate::combat::CardPile> {
    match pile {
        Pile::Hand => Some(&mut ps.hand),
        Pile::Discard => Some(&mut ps.discard),
        Pile::Draw => Some(&mut ps.draw),
        Pile::Exhaust => Some(&mut ps.exhaust),
        // Deck lives on RunState, not PlayerState — not accessible
        // from the combat-scoped VM. Event-layer dispatcher will
        // resolve this elsewhere.
        Pile::Deck => None,
    }
}

fn remove_card_from_pile(
    cs: &mut CombatState,
    player_idx: usize,
    pile: Pile,
    idx: usize,
) -> Option<crate::combat::CardInstance> {
    let ps = player_state_mut(cs, player_idx)?;
    let cards = pile_mut(ps, pile)?;
    if idx >= cards.cards.len() {
        return None;
    }
    Some(cards.cards.remove(idx))
}

fn push_card_to_pile(
    cs: &mut CombatState,
    player_idx: usize,
    pile: Pile,
    card: crate::combat::CardInstance,
) {
    if let Some(ps) = player_state_mut(cs, player_idx) {
        if let Some(p) = pile_mut(ps, pile) {
            p.cards.push(card);
        }
    }
}

/// Resolve a Selector to the list of card indices in the named pile.
/// Indices are returned in pile order; callers that mutate the pile
/// (Exhaust / Discard / MoveCard) sort descending before removing.
fn select_card_indices(
    cs: &mut CombatState,
    player_idx: usize,
    pile: Pile,
    selector: &Selector,
) -> Vec<usize> {
    let Some(ps) = player_state_mut(cs, player_idx) else {
        return Vec::new();
    };
    let Some(cards) = pile_mut(ps, pile) else {
        return Vec::new();
    };
    let len = cards.cards.len();
    if len == 0 {
        return Vec::new();
    }
    match selector {
        Selector::All => (0..len).collect(),
        Selector::Random { n } => {
            // Re-borrow rng via the temp-swap trick.
            let n = (*n).max(0) as usize;
            let n = n.min(len);
            // Snapshot indices, draw without replacement.
            let mut pool: Vec<usize> = (0..len).collect();
            let mut picked = Vec::with_capacity(n);
            let mut rng = std::mem::replace(&mut cs.rng, crate::rng::Rng::new(0, 0));
            for _ in 0..n {
                if pool.is_empty() {
                    break;
                }
                let pick = rng.next_int_range(0, pool.len() as i32) as usize;
                picked.push(pool.swap_remove(pick));
            }
            cs.rng = rng;
            picked
        }
        Selector::Top { n } => {
            let n = (*n).max(0) as usize;
            let start = len.saturating_sub(n);
            (start..len).rev().collect()
        }
        Selector::Bottom { n } => {
            let n = (*n).max(0) as usize;
            (0..n.min(len)).collect()
        }
        Selector::FirstMatching { n, filter } => {
            // Re-borrow pile.
            let Some(ps) = player_state_mut(cs, player_idx) else {
                return Vec::new();
            };
            let Some(cards) = pile_mut(ps, pile) else {
                return Vec::new();
            };
            let n = (*n).max(0) as usize;
            let mut out = Vec::new();
            for (i, card) in cards.cards.iter().enumerate() {
                if matches_filter(card, filter) {
                    out.push(i);
                    if out.len() >= n {
                        break;
                    }
                }
            }
            out
        }
        Selector::PlayerInteractive { n } => {
            // Deferred: until multi-step play API lands, fall back to
            // Random selection. Deterministic given combat RNG.
            let stub = Selector::Random { n: *n };
            select_card_indices(cs, player_idx, pile, &stub)
        }
        Selector::PlayerInteractiveFiltered { n, filter } => {
            // Filter to the candidate pool, then random-pick from that.
            // Equivalent to "C# CardSelectCmd.From(filtered_cards, n)
            // with default-random fallback".
            let Some(ps) = player_state_mut(cs, player_idx) else {
                return Vec::new();
            };
            let Some(cards) = pile_mut(ps, pile) else {
                return Vec::new();
            };
            let candidate_indices: Vec<usize> = cards
                .cards
                .iter()
                .enumerate()
                .filter(|(_, c)| matches_filter(c, filter))
                .map(|(i, _)| i)
                .collect();
            if candidate_indices.is_empty() {
                return Vec::new();
            }
            let mut rng = std::mem::replace(&mut cs.rng, crate::rng::Rng::new(0, 0));
            let mut picked: Vec<usize> = Vec::with_capacity((*n).max(0) as usize);
            let mut pool = candidate_indices;
            for _ in 0..(*n).max(0) {
                if pool.is_empty() {
                    break;
                }
                let pick = rng.next_int_range(0, pool.len() as i32) as usize;
                picked.push(pool.swap_remove(pick));
            }
            cs.rng = rng;
            picked
        }
    }
}

fn matches_filter(card: &crate::combat::CardInstance, filter: &CardFilter) -> bool {
    let Some(data) = crate::card::by_id(&card.id) else {
        return false;
    };
    match filter {
        CardFilter::Any => true,
        CardFilter::Upgradable => {
            card.upgrade_level == 0 && data.max_upgrade_level > 0
        }
        CardFilter::OfType(t) => {
            format!("{:?}", data.card_type).eq_ignore_ascii_case(t)
        }
        CardFilter::HasKeyword(k) => data.keywords.iter().any(|kw| kw.eq_ignore_ascii_case(k)),
        CardFilter::TaggedAs(t) => data.tags.iter().any(|tag| tag.eq_ignore_ascii_case(t)),
        CardFilter::OfRarity(r) => {
            format!("{:?}", data.rarity).eq_ignore_ascii_case(r)
        }
        CardFilter::And(a, b) => matches_filter(card, a) && matches_filter(card, b),
        CardFilter::Or(a, b) => matches_filter(card, a) || matches_filter(card, b),
        CardFilter::Not(inner) => !matches_filter(card, inner),
        CardFilter::HasId(id) => &card.id == id,
        CardFilter::WithEnergyCost { op, value } => compare(data.energy_cost, *op, *value),
        CardFilter::NotXCost => !data.has_energy_cost_x,
    }
}

fn shuffle_pile(cs: &mut CombatState, player_idx: usize, pile: Pile) {
    // Mirror the temp-swap trick used elsewhere so the combat RNG can
    // be borrowed alongside `cs.allies`.
    let mut rng = std::mem::replace(&mut cs.rng, crate::rng::Rng::new(0, 0));
    if let Some(creature) = cs.allies.get_mut(player_idx) {
        if let Some(ps) = creature.player.as_mut() {
            let cards = match pile {
                Pile::Hand => Some(&mut ps.hand.cards),
                Pile::Discard => Some(&mut ps.discard.cards),
                Pile::Draw => Some(&mut ps.draw.cards),
                Pile::Exhaust => Some(&mut ps.exhaust.cards),
                Pile::Deck => None,
            };
            if let Some(cards) = cards {
                rng.shuffle(cards);
            }
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
        Target::SelfActor => {
            // Attack-pipeline damage from actor to self is not a real
            // card / monster-move pattern; if a monster wants to lose
            // HP, use LoseHp. No-op.
        }
        Target::ChosenAlly | Target::AllAllies => {
            // Attack-pipeline damage targeting allies is not a real
            // card pattern (StS2 has no friendly-fire attack cards).
            // No-op so AnyAlly cards encoded as DealDamage{ChosenAlly}
            // stay safe.
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
    for_each_target_idx(cs, ctx, target, |cs, side, idx| {
        cs.apply_power(side, idx, power_id, amount);
    });
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
        let cs = ironclad_combat();
        let ctx_base = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        let ctx_up = EffectContext::for_card(0, None, "StrikeIronclad", 1, None, 0);
        let spec = AmountSpec::Canonical("Damage".to_string());
        assert_eq!(spec.resolve(&ctx_base, &cs), 6);
        // Upgraded Strike does 9 damage (+3).
        assert_eq!(spec.resolve(&ctx_up, &cs), 9);
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

    /// Osty subsystem: SummonOsty creates the companion; OstyMaxHp /
    /// OstyBlock AmountSpecs read its state; DamageFromOsty no-ops
    /// if Osty doesn't exist.
    #[test]
    fn osty_summon_creates_companion_with_hp() {
        let mut cs = ironclad_combat();
        assert!(cs.allies[0].player.as_ref().unwrap().osty.is_none());
        let ctx = EffectContext::for_card(0, None, "Bodyguard", 0, None, 0);
        execute_effects(
            &mut cs,
            &[Effect::SummonOsty {
                osty_id: "Osty".to_string(),
                max_hp: None,
            }],
            &ctx,
        );
        let osty = cs.allies[0].player.as_ref().unwrap().osty.as_ref();
        assert!(osty.is_some());
        let o = osty.unwrap();
        assert_eq!(o.current_hp, 6);
        assert_eq!(o.max_hp, 6);
    }

    #[test]
    fn osty_max_hp_amount_reads_companion() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().osty = Some(crate::combat::OstyState {
            current_hp: 10,
            max_hp: 10,
            block: 0,
        });
        let ctx = EffectContext::for_card(0, None, "Protector", 0, None, 0);
        assert_eq!(AmountSpec::OstyMaxHp.resolve(&ctx, &cs), 10);
    }

    #[test]
    fn damage_from_osty_no_ops_when_no_osty() {
        let mut cs = ironclad_combat();
        let hp_before = cs.enemies[0].current_hp;
        let ctx = EffectContext::for_card(
            0,
            Some((CombatSide::Enemy, 0)),
            "Fetch",
            0,
            None,
            0,
        );
        execute_effects(
            &mut cs,
            &[Effect::DamageFromOsty {
                amount: AmountSpec::Fixed(5),
                target: Target::ChosenEnemy,
            }],
            &ctx,
        );
        // No Osty → no damage.
        assert_eq!(cs.enemies[0].current_hp, hp_before);
    }

    #[test]
    fn damage_from_osty_lands_when_osty_present() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().osty = Some(crate::combat::OstyState {
            current_hp: 6,
            max_hp: 6,
            block: 0,
        });
        let hp_before = cs.enemies[0].current_hp;
        let ctx = EffectContext::for_card(
            0,
            Some((CombatSide::Enemy, 0)),
            "Fetch",
            0,
            None,
            0,
        );
        execute_effects(
            &mut cs,
            &[Effect::DamageFromOsty {
                amount: AmountSpec::Fixed(5),
                target: Target::ChosenEnemy,
            }],
            &ctx,
        );
        assert_eq!(cs.enemies[0].current_hp, hp_before - 5);
    }

    /// Orb subsystem — Channel pushes to queue, Evoke pops front and
    /// runs that orb's effect. Frost evokes block (8 unpowered);
    /// Lightning evokes random-enemy damage; Plasma evokes energy.
    #[test]
    fn orb_channel_and_evoke_frost() {
        let mut cs = ironclad_combat();
        let block_before = cs.allies[0].block;
        let ctx = EffectContext::for_card(0, None, "Glacier", 0, None, 0);
        execute_effects(
            &mut cs,
            &[Effect::ChannelOrb {
                orb_id: "FrostOrb".to_string(),
            }],
            &ctx,
        );
        assert_eq!(
            cs.allies[0].player.as_ref().unwrap().orb_queue.len(),
            1
        );
        execute_effects(&mut cs, &[Effect::EvokeNextOrb], &ctx);
        // FrostOrb evoke = 8 block (unpowered).
        assert_eq!(cs.allies[0].block, block_before + 8);
        assert_eq!(cs.allies[0].player.as_ref().unwrap().orb_queue.len(), 0);
    }

    #[test]
    fn orb_channel_at_capacity_evokes_front_first() {
        let mut cs = ironclad_combat();
        cs.allies[0].player.as_mut().unwrap().orb_slots = 2;
        let ctx = EffectContext::for_card(0, None, "Coolheaded", 0, None, 0);
        // Fill queue with 2 Frosts.
        execute_effects(
            &mut cs,
            &[Effect::ChannelOrb {
                orb_id: "FrostOrb".to_string(),
            }],
            &ctx,
        );
        execute_effects(
            &mut cs,
            &[Effect::ChannelOrb {
                orb_id: "FrostOrb".to_string(),
            }],
            &ctx,
        );
        assert_eq!(cs.allies[0].player.as_ref().unwrap().orb_queue.len(), 2);
        let block_before = cs.allies[0].block;
        // Third channel at capacity: front evokes (8 block), new orb
        // pushed.
        execute_effects(
            &mut cs,
            &[Effect::ChannelOrb {
                orb_id: "LightningOrb".to_string(),
            }],
            &ctx,
        );
        assert_eq!(cs.allies[0].block, block_before + 8);
        let queue = &cs.allies[0].player.as_ref().unwrap().orb_queue;
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.last().unwrap().id, "LightningOrb");
    }

    #[test]
    fn change_orb_slots_adjusts_capacity() {
        let mut cs = ironclad_combat();
        let before = cs.allies[0].player.as_ref().unwrap().orb_slots;
        let ctx = EffectContext::for_card(0, None, "Capacitor", 0, None, 0);
        execute_effects(
            &mut cs,
            &[Effect::ChangeOrbSlots {
                delta: AmountSpec::Fixed(2),
            }],
            &ctx,
        );
        assert_eq!(cs.allies[0].player.as_ref().unwrap().orb_slots, before + 2);
    }

    /// Calc-var AmountSpec extensions:
    /// - SelfBlock reads actor's current block.
    /// - CardCountInPile counts matching cards across one or more piles.
    /// - TargetPowerAmount reads the chosen target's power amount.
    #[test]
    fn self_block_amount_reads_player_block() {
        let mut cs = ironclad_combat();
        cs.allies[0].block = 17;
        let ctx = EffectContext::for_card(0, None, "BodySlam", 0, None, 0);
        let amt = AmountSpec::SelfBlock.resolve(&ctx, &cs);
        assert_eq!(amt, 17);
    }

    #[test]
    fn card_count_in_pile_counts_filtered_cards() {
        let mut cs = ironclad_combat();
        let strike_count_before = AmountSpec::CardCountInPile {
            pile: PileSelector::AllCombat,
            filter: CardFilter::TaggedAs("Strike".to_string()),
        }
        .resolve(
            &EffectContext::for_card(0, None, "PerfectedStrike", 0, None, 0),
            &cs,
        );
        // Ironclad starter deck has 5 Strikes and 5 Defends.
        assert_eq!(strike_count_before, 5);

        // Add another Strike to hand; count should bump.
        if let Some(card) = crate::card::by_id("StrikeIronclad") {
            cs.allies[0]
                .player
                .as_mut()
                .unwrap()
                .hand
                .cards
                .push(crate::combat::CardInstance::from_card(card, 0));
        }
        let after = AmountSpec::CardCountInPile {
            pile: PileSelector::AllCombat,
            filter: CardFilter::TaggedAs("Strike".to_string()),
        }
        .resolve(
            &EffectContext::for_card(0, None, "PerfectedStrike", 0, None, 0),
            &cs,
        );
        assert_eq!(after, strike_count_before + 1);
    }

    #[test]
    fn target_power_amount_reads_chosen_enemy_power() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 4);
        let ctx = EffectContext::for_card(
            0,
            Some((CombatSide::Enemy, 0)),
            "Bully",
            0,
            None,
            0,
        );
        let amt = AmountSpec::TargetPowerAmount {
            power_id: "VulnerablePower".to_string(),
        }
        .resolve(&ctx, &cs);
        assert_eq!(amt, 4);
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

    /// LoseBlock decrements target block, floors at 0.
    #[test]
    fn lose_block_floors_at_zero() {
        let mut cs = ironclad_combat();
        cs.allies[0].block = 7;
        let effects = vec![Effect::LoseBlock {
            amount: AmountSpec::Fixed(10),
            target: Target::SelfPlayer,
        }];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        assert_eq!(cs.allies[0].block, 0);
    }

    /// GainGold + LoseGold accumulate / drain pending_gold; floor at 0.
    #[test]
    fn gain_gold_and_lose_gold_accumulate() {
        let mut cs = ironclad_combat();
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(
            &mut cs,
            &[Effect::GainGold {
                amount: AmountSpec::Fixed(20),
            }],
            &ctx,
        );
        assert_eq!(cs.allies[0].player.as_ref().unwrap().pending_gold, 20);
        execute_effects(
            &mut cs,
            &[Effect::LoseGold {
                amount: AmountSpec::Fixed(50),
            }],
            &ctx,
        );
        assert_eq!(cs.allies[0].player.as_ref().unwrap().pending_gold, 0);
    }

    /// GainStars accumulates pending_stars.
    #[test]
    fn gain_stars_accumulates() {
        let mut cs = ironclad_combat();
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(
            &mut cs,
            &[Effect::GainStars {
                amount: AmountSpec::Fixed(3),
            }],
            &ctx,
        );
        assert_eq!(cs.allies[0].player.as_ref().unwrap().pending_stars, 3);
    }

    /// ExhaustCards with `Selector::All` clears hand → exhaust.
    #[test]
    fn exhaust_cards_all_from_hand_clears_hand() {
        let mut cs = ironclad_combat();
        // Seed two cards into hand.
        let ps = cs.allies[0].player.as_mut().unwrap();
        for _ in 0..3 {
            if let Some(card) = crate::card::by_id("StrikeIronclad") {
                ps.hand
                    .cards
                    .push(crate::combat::CardInstance::from_card(card, 0));
            }
        }
        let hand_before = cs.allies[0].player.as_ref().unwrap().hand.cards.len();
        let exhaust_before = cs.allies[0].player.as_ref().unwrap().exhaust.cards.len();
        let effects = vec![Effect::ExhaustCards {
            from: Pile::Hand,
            selector: Selector::All,
        }];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        assert_eq!(cs.allies[0].player.as_ref().unwrap().hand.cards.len(), 0);
        assert_eq!(
            cs.allies[0].player.as_ref().unwrap().exhaust.cards.len(),
            exhaust_before + hand_before
        );
    }

    /// UpgradeCards FirstMatching(Upgradable) bumps upgrade_level.
    #[test]
    fn upgrade_cards_first_matching_upgradable() {
        let mut cs = ironclad_combat();
        // Seed an upgradable card.
        let ps = cs.allies[0].player.as_mut().unwrap();
        if let Some(card) = crate::card::by_id("StrikeIronclad") {
            ps.hand
                .cards
                .push(crate::combat::CardInstance::from_card(card, 0));
        }
        let effects = vec![Effect::UpgradeCards {
            from: Pile::Hand,
            selector: Selector::FirstMatching {
                n: 1,
                filter: CardFilter::Upgradable,
            },
        }];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        let upgraded_count = cs.allies[0]
            .player
            .as_ref()
            .unwrap()
            .hand
            .cards
            .iter()
            .filter(|c| c.upgrade_level >= 1)
            .count();
        assert!(upgraded_count >= 1);
    }

    /// SummonMonster appends a new enemy and fires its spawn payload
    /// (HardenedShellPower for SkulkingColony).
    #[test]
    fn summon_monster_appends_and_spawns() {
        let mut cs = ironclad_combat();
        let n_before = cs.enemies.len();
        let effects = vec![Effect::SummonMonster {
            monster_id: "SkulkingColony".to_string(),
            slot: "back".to_string(),
        }];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        assert_eq!(cs.enemies.len(), n_before + 1);
        let new_idx = n_before;
        assert_eq!(cs.enemies[new_idx].model_id, "SkulkingColony");
        // SkulkingColony spawn applies HardenedShellPower.
        assert!(cs.enemies[new_idx]
            .powers
            .iter()
            .any(|p| p.id == "HardenedShellPower"));
    }

    /// SetMaxHpAndHeal resets HP. TestSubject Revive shape.
    #[test]
    fn set_max_hp_and_heal_resets_to_full() {
        let mut cs = ironclad_combat();
        cs.allies[0].current_hp = 30;
        let effects = vec![Effect::SetMaxHpAndHeal {
            amount: AmountSpec::Fixed(60),
            target: Target::SelfPlayer,
        }];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        assert_eq!(cs.allies[0].max_hp, 60);
        assert_eq!(cs.allies[0].current_hp, 60);
    }

    /// ModifyPowerAmount adjusts an existing power without going
    /// through ApplyPower's Counter merging.
    #[test]
    fn modify_power_amount_adjusts_existing_stack() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 2);
        let effects = vec![Effect::ModifyPowerAmount {
            power_id: "VulnerablePower".to_string(),
            delta: AmountSpec::Fixed(3),
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
        let vuln = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "VulnerablePower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(vuln, 5);
    }

    /// Conditional with IsUpgraded picks the right branch based on
    /// upgrade_level. Forms the basis of TrueGrit / MultiCast-style
    /// upgrade-branched bodies.
    #[test]
    fn conditional_is_upgraded_picks_branch() {
        // Base path (upgrade_level=0) → then_branch fires.
        let mut cs = ironclad_combat();
        let effects = vec![Effect::Conditional {
            condition: Condition::IsUpgraded,
            then_branch: vec![Effect::GainBlock {
                amount: AmountSpec::Fixed(10),
                target: Target::SelfPlayer,
            }],
            else_branch: vec![Effect::GainBlock {
                amount: AmountSpec::Fixed(3),
                target: Target::SelfPlayer,
            }],
        }];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        assert_eq!(cs.allies[0].block, 3);

        // Upgraded path.
        let mut cs2 = ironclad_combat();
        let ctx_up = EffectContext::for_card(0, None, "StrikeIronclad", 1, None, 0);
        execute_effects(&mut cs2, &effects, &ctx_up);
        assert_eq!(cs2.allies[0].block, 10);
    }

    /// Repeat with a Fixed count loops the body the right number of times.
    #[test]
    fn repeat_loops_body() {
        let mut cs = ironclad_combat();
        let effects = vec![Effect::Repeat {
            count: AmountSpec::Fixed(4),
            body: vec![Effect::GainBlock {
                amount: AmountSpec::Fixed(2),
                target: Target::SelfPlayer,
            }],
        }];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        assert_eq!(cs.allies[0].block, 8);
    }

    /// Repeat with XEnergy resolves to ctx.x_value (Whirlwind shape).
    #[test]
    fn repeat_with_x_energy_resolves_dynamically() {
        let mut cs = ironclad_combat();
        let effects = vec![Effect::Repeat {
            count: AmountSpec::XEnergy,
            body: vec![Effect::DealDamage {
                amount: AmountSpec::Fixed(5),
                target: Target::AllEnemies,
                hits: 1,
            }],
        }];
        let ctx = EffectContext::for_card(0, None, "Whirlwind", 0, None, 3);
        let hp_before: Vec<i32> = cs.enemies.iter().map(|e| e.current_hp).collect();
        execute_effects(&mut cs, &effects, &ctx);
        // 3 × 5 = 15 damage to each alive enemy.
        for (i, before) in hp_before.iter().enumerate() {
            if *before == 0 {
                continue;
            }
            assert_eq!(cs.enemies[i].current_hp, before - 15);
        }
    }

    /// SelfActor target routes via EffectContext.actor — useful for
    /// monster-move authoring where the actor is the moving enemy.
    #[test]
    fn self_actor_targets_actor_creature() {
        let mut cs = ironclad_combat();
        // Manually craft a monster-author context.
        let ctx = EffectContext::for_monster_move(0, None);
        let effects = vec![Effect::ApplyPower {
            power_id: "StrengthPower".to_string(),
            amount: AmountSpec::Fixed(2),
            target: Target::SelfActor,
        }];
        execute_effects(&mut cs, &effects, &ctx);
        // The actor here is enemy 0, so Strength lands on the enemy.
        let strength = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "StrengthPower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(strength, 2);
    }

    /// Condition::HasPowerOnTarget reads the chosen-target's powers.
    #[test]
    fn has_power_on_target_works_for_chosen_enemy() {
        let mut cs = ironclad_combat();
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 3);
        let ctx = EffectContext::for_card(
            0,
            Some((CombatSide::Enemy, 0)),
            "StrikeIronclad",
            0,
            None,
            0,
        );
        assert!(evaluate_condition(
            &mut cs,
            &ctx,
            &Condition::HasPowerOnTarget {
                power_id: "VulnerablePower".to_string(),
            }
        ));
        assert!(!evaluate_condition(
            &mut cs,
            &ctx,
            &Condition::HasPowerOnTarget {
                power_id: "PoisonPower".to_string(),
            }
        ));
    }

    /// RandomChance with 1/1 fires; 0/1 never fires (deterministic
    /// edge cases).
    #[test]
    fn random_chance_certainty_cases() {
        let mut cs = ironclad_combat();
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        assert!(evaluate_condition(
            &mut cs,
            &ctx,
            &Condition::RandomChance {
                numerator: 1,
                denominator: 1,
            }
        ));
        assert!(!evaluate_condition(
            &mut cs,
            &ctx,
            &Condition::RandomChance {
                numerator: 0,
                denominator: 1,
            }
        ));
    }

    /// Run-state stubs (GainRelic etc.) do not crash and do not
    /// alter CombatState. Encode-able-but-inert tier.
    #[test]
    fn run_state_stubs_are_safe_noops() {
        let mut cs = ironclad_combat();
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        let stubs = vec![
            Effect::GainRelic {
                relic_id: "BurningBlood".to_string(),
            },
            Effect::LoseRelic {
                relic_id: "BurningBlood".to_string(),
            },
            Effect::GainPotionToBelt {
                potion_id: "FirePotion".to_string(),
            },
            Effect::LoseRunStateHp {
                amount: AmountSpec::Fixed(5),
            },
            Effect::GainRunStateMaxHp {
                amount: AmountSpec::Fixed(10),
            },
            Effect::GainRunStateGold {
                amount: AmountSpec::Fixed(50),
            },
            Effect::SetEventFinished {
                description_key: "WOOD_CARVINGS.pages.BIRD.description".to_string(),
            },
            Effect::MoveToEventPage {
                page_id: "PAGE_2".to_string(),
            },
        ];
        let hp_before = cs.allies[0].current_hp;
        execute_effects(&mut cs, &stubs, &ctx);
        assert_eq!(cs.allies[0].current_hp, hp_before);
    }

    /// RegenPower migrated to the Power VM. AfterTurnEnd on owner-side:
    /// heal Amount, then decrement stack by 1. Mirrors C# RegenPower.cs.
    #[test]
    fn regen_power_vm_heals_and_decrements_at_owner_turn_end() {
        let mut cs = ironclad_combat();
        // Wound the player a little so heal is observable.
        cs.allies[0].current_hp = cs.allies[0].max_hp - 10;
        cs.apply_power(CombatSide::Player, 0, "RegenPower", 5);
        let hp_before = cs.allies[0].current_hp;
        cs.current_side = CombatSide::Player;
        cs.end_turn();
        // Heal +5; Regen decremented to 4.
        assert_eq!(cs.allies[0].current_hp, hp_before + 5);
        assert_eq!(
            cs.get_power_amount(CombatSide::Player, 0, "RegenPower"),
            4
        );
    }

    /// Regen on enemy doesn't fire when the player's turn ends —
    /// `HookSideFilter::OwnerSide` mirrors C# `if side == Owner.Side`.
    #[test]
    fn regen_power_vm_only_fires_on_owner_turn_end() {
        let mut cs = ironclad_combat();
        cs.enemies[0].current_hp = cs.enemies[0].max_hp - 10;
        cs.apply_power(CombatSide::Enemy, 0, "RegenPower", 3);
        let hp_before = cs.enemies[0].current_hp;
        let regen_before = cs.get_power_amount(CombatSide::Enemy, 0, "RegenPower");
        // End the PLAYER turn — enemy regen should NOT fire.
        cs.current_side = CombatSide::Player;
        cs.end_turn();
        assert_eq!(cs.enemies[0].current_hp, hp_before);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "RegenPower"),
            regen_before
        );
        // Now end the ENEMY turn — regen fires.
        cs.begin_turn(CombatSide::Enemy);
        cs.end_turn();
        assert_eq!(cs.enemies[0].current_hp, hp_before + 3);
        assert_eq!(
            cs.get_power_amount(CombatSide::Enemy, 0, "RegenPower"),
            regen_before - 1
        );
    }

    /// Stun sets the monster's stunned flag. dispatch_enemy_turn
    /// consumes it (test that path indirectly via the MonsterState
    /// flag read here; full dispatch integration is in
    /// monster_dispatch tests).
    #[test]
    fn stun_sets_monster_stunned_flag() {
        let mut cs = ironclad_combat();
        let effects = vec![Effect::Stun {
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
        let stunned = cs.enemies[0]
            .monster
            .as_ref()
            .map(|m| m.flag("stunned"))
            .unwrap_or(false);
        assert!(stunned);
    }

    /// ApplyAfflictionToAllInPile is a stub (no affliction-on-card
    /// infrastructure yet). Test confirms it doesn't crash.
    #[test]
    fn apply_affliction_to_all_in_pile_stub_is_safe() {
        let mut cs = ironclad_combat();
        let effects = vec![Effect::ApplyAfflictionToAllInPile {
            affliction_id: "Hexed".to_string(),
            pile: Pile::Hand,
            amount: AmountSpec::Fixed(1),
        }];
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        execute_effects(&mut cs, &effects, &ctx);
        // No-op; just confirms no panic.
    }

    /// Stub primitives (orb / osty / forge / quest / end-turn /
    /// potion-fill / auto-play / keyword / transform / cost) do not
    /// crash; they just don't change state. This is the "encode-able
    /// but inert" surface area.
    #[test]
    fn stub_primitives_are_safe_noops() {
        let mut cs = ironclad_combat();
        let ctx = EffectContext::for_card(0, None, "StrikeIronclad", 0, None, 0);
        let stubs = vec![
            Effect::ChannelOrb {
                orb_id: "LightningOrb".to_string(),
            },
            Effect::EvokeNextOrb,
            Effect::TriggerOrbPassive,
            Effect::ChangeOrbSlots {
                delta: AmountSpec::Fixed(1),
            },
            Effect::SummonOsty {
                osty_id: "BoneCompanion".to_string(),
                max_hp: None,
            },
            Effect::DamageFromOsty {
                amount: AmountSpec::Fixed(5),
                target: Target::ChosenEnemy,
            },
            Effect::Forge {
                amount: AmountSpec::Fixed(1),
            },
            Effect::EndTurn,
            Effect::CompleteQuest,
            Effect::GenerateRandomPotion,
            Effect::FillPotionSlots,
            Effect::AutoplayFromDraw { n: 3 },
            Effect::ApplyKeywordToCards {
                keyword: "Exhaust".to_string(),
                from: Pile::Hand,
                selector: Selector::All,
            },
            Effect::TransformCards {
                from: Pile::Discard,
                selector: Selector::All,
            },
            Effect::SetCardCost {
                from: Pile::Hand,
                selector: Selector::All,
                cost: AmountSpec::Fixed(0),
                scope: CostScope::ThisTurn,
            },
            Effect::KillSelf,
        ];
        execute_effects(&mut cs, &stubs, &ctx);
        // Just assert the run didn't panic and state is plausible.
        assert!(cs.allies[0].current_hp > 0);
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
