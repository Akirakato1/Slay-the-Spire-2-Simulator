//! Phase 1.1 — card feature encoding.
//!
//! Fixed-size feature vector per card, derived from `CardData` (static
//! table) and optionally `CardInstance` (runtime upgrade level /
//! enchantment). The agent's card-embedding MLP consumes these vectors
//! through `sts2-sim-py` to produce dense embeddings shared across
//! decisions.
//!
//! ## Schema versioning
//!
//! `OBSERVATION_SCHEMA_VERSION` bumps when this layout changes. A
//! trained agent reads the version it was trained against; the
//! simulator emits the current version. Mismatch is a hard error so
//! stale checkpoints can't load against a new featurizer.
//!
//! ## Why derived, not config-driven (for now)
//!
//! The project plan suggests externalizing feature tables to JSON/YAML
//! for hot-reload during agent iteration. We start derived for two
//! reasons: (1) the canonical `CardData` already encodes the underlying
//! C# spec, so deriving here gives one source of truth; (2) per-card
//! feature override needs are unknown until agent training surfaces
//! them. When iteration hurts, externalize to `data/card_features.json`
//! and keep the schema version in lockstep.
//!
//! ## What's in v1
//!
//! 45 floats:
//!   - 1: energy_cost (raw, may need normalization downstream)
//!   - 7: CardType one-hot (None / Attack / Skill / Power / Status / Curse / Quest)
//!   - 11: CardRarity one-hot
//!   - 10: TargetType one-hot
//!   - 1: upgrade_level (0/1/2…)
//!   - 1: max_upgrade_level
//!   - 1: has_energy_cost_x (X-cost cards)
//!   - 2: has_damage, damage_amount
//!   - 2: has_block, block_amount
//!   - 3: applies StrengthPower / VulnerablePower / WeakPower (counts)
//!   - 2: has_card_draw, draw_amount
//!   - 1: has_enchantment
//!
//! Reserved space for v2 expansion: more power-apply slots, hover-tip
//! flags, conditional-trigger flags (exhaust, retain, innate), tag
//! one-hots (Strike, Defend, Curse, …).

use crate::card::{CardData, CardRarity, CardType, TargetType};
use crate::combat::{CardInstance, Creature, CreatureKind, PowerInstance};
use crate::relic::{RelicData, RelicRarity};
use serde::{ser::SerializeSeq, Serialize, Serializer};

// `serde` derive can't generate a `Serialize` impl for `[T; N]` when
// `N > 32` (large-array support requires a separate crate or a manual
// impl). We hand-write impls below that emit a flat JSON array. The
// derived shape is a single `[…]` per feature struct — agents reading
// the JSON treat it as the feature row directly.
fn serialize_f32_slice<S: Serializer>(values: &[f32], s: S) -> Result<S::Ok, S::Error> {
    let mut seq = s.serialize_seq(Some(values.len()))?;
    for v in values {
        seq.serialize_element(v)?;
    }
    seq.end()
}

/// Schema version. Bump when the feature layout below changes.
pub const OBSERVATION_SCHEMA_VERSION: u32 = 1;

/// Fixed-size feature vector dimension. Const so callers can size
/// tensors at compile time.
pub const CARD_FEATURE_DIM: usize = 45;

/// One card's feature vector. Indices documented below; the layout is
/// stable for a given `OBSERVATION_SCHEMA_VERSION`.
#[derive(Clone, Copy, Debug)]
pub struct CardFeatures {
    pub values: [f32; CARD_FEATURE_DIM],
}

impl CardFeatures {
    pub fn as_slice(&self) -> &[f32] {
        &self.values
    }
}

impl Serialize for CardFeatures {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serialize_f32_slice(&self.values, s)
    }
}

// ---------- Index map (stable for v1) ----------
const IDX_ENERGY_COST: usize = 0;
const IDX_CARD_TYPE_BASE: usize = 1; // 7 slots
const IDX_RARITY_BASE: usize = 8; // 11 slots
const IDX_TARGET_BASE: usize = 19; // 10 slots
const IDX_UPGRADE_LEVEL: usize = 29;
const IDX_MAX_UPGRADE_LEVEL: usize = 30;
const IDX_HAS_ENERGY_COST_X: usize = 31;
const IDX_HAS_DAMAGE: usize = 32;
const IDX_DAMAGE_AMOUNT: usize = 33;
const IDX_HAS_BLOCK: usize = 34;
const IDX_BLOCK_AMOUNT: usize = 35;
const IDX_APPLIES_STRENGTH: usize = 36;
const IDX_APPLIES_VULNERABLE: usize = 37;
const IDX_APPLIES_WEAK: usize = 38;
const IDX_HAS_CARD_DRAW: usize = 39;
const IDX_CARD_DRAW_AMOUNT: usize = 40;
const IDX_HAS_ENCHANTMENT: usize = 41;
// Indices 42-44 reserved for v2 expansion. Zeroed out for v1.

/// Featurize one card. `instance` is the optional runtime state —
/// upgrade level and enchantment come from it. When `instance` is
/// `None`, the card is featurized at upgrade level 0 with no
/// enchantment.
pub fn card_features(card: &CardData, instance: Option<&CardInstance>) -> CardFeatures {
    let mut v = [0.0f32; CARD_FEATURE_DIM];

    let upgrade_level = instance.map(|i| i.upgrade_level).unwrap_or(0);

    v[IDX_ENERGY_COST] = effective_energy_cost(card, upgrade_level) as f32;

    one_hot(&mut v, IDX_CARD_TYPE_BASE, card_type_index(card.card_type));
    one_hot(&mut v, IDX_RARITY_BASE, rarity_index(card.rarity));
    one_hot(&mut v, IDX_TARGET_BASE, target_type_index(card.target_type));

    v[IDX_UPGRADE_LEVEL] = upgrade_level as f32;
    v[IDX_MAX_UPGRADE_LEVEL] = card.max_upgrade_level as f32;
    v[IDX_HAS_ENERGY_COST_X] = bool_f32(card.has_energy_cost_x);

    let damage = effective_var(card, "Damage", upgrade_level);
    v[IDX_HAS_DAMAGE] = bool_f32(damage.is_some());
    v[IDX_DAMAGE_AMOUNT] = damage.unwrap_or(0.0) as f32;

    let block = effective_var(card, "Block", upgrade_level);
    v[IDX_HAS_BLOCK] = bool_f32(block.is_some());
    v[IDX_BLOCK_AMOUNT] = block.unwrap_or(0.0) as f32;

    // Power-apply features — read the resolved value (base + upgrade
    // delta scaled by upgrade level). `effective_var` already does
    // var-name resolution against generic/key/strip-Power-suffix.
    v[IDX_APPLIES_STRENGTH] =
        effective_var(card, "Strength", upgrade_level).unwrap_or(0.0) as f32;
    v[IDX_APPLIES_VULNERABLE] =
        effective_var(card, "Vulnerable", upgrade_level).unwrap_or(0.0) as f32;
    v[IDX_APPLIES_WEAK] =
        effective_var(card, "Weak", upgrade_level).unwrap_or(0.0) as f32;

    let draw = effective_var(card, "Cards", upgrade_level);
    v[IDX_HAS_CARD_DRAW] = bool_f32(draw.is_some());
    v[IDX_CARD_DRAW_AMOUNT] = draw.unwrap_or(0.0) as f32;

    v[IDX_HAS_ENCHANTMENT] = bool_f32(
        instance
            .map(|i| i.enchantment.is_some())
            .unwrap_or(false),
    );

    CardFeatures { values: v }
}

// ---------- Helpers ----------

fn bool_f32(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

fn one_hot(buf: &mut [f32], base: usize, idx: usize) {
    buf[base + idx] = 1.0;
}

fn card_type_index(t: CardType) -> usize {
    match t {
        CardType::None => 0,
        CardType::Attack => 1,
        CardType::Skill => 2,
        CardType::Power => 3,
        CardType::Status => 4,
        CardType::Curse => 5,
        CardType::Quest => 6,
    }
}

fn rarity_index(r: CardRarity) -> usize {
    match r {
        CardRarity::None => 0,
        CardRarity::Basic => 1,
        CardRarity::Common => 2,
        CardRarity::Uncommon => 3,
        CardRarity::Rare => 4,
        CardRarity::Ancient => 5,
        CardRarity::Event => 6,
        CardRarity::Token => 7,
        CardRarity::Status => 8,
        CardRarity::Curse => 9,
        CardRarity::Quest => 10,
    }
}

fn target_type_index(t: TargetType) -> usize {
    match t {
        TargetType::None => 0,
        TargetType::SelfTarget => 1,
        TargetType::AnyEnemy => 2,
        TargetType::AllEnemies => 3,
        TargetType::RandomEnemy => 4,
        TargetType::AnyPlayer => 5,
        TargetType::AnyAlly => 6,
        TargetType::AllAllies => 7,
        TargetType::TargetedNoCreature => 8,
        TargetType::Osty => 9,
    }
}

fn effective_energy_cost(card: &CardData, upgrade_level: i32) -> i32 {
    if upgrade_level > 0 {
        (card.energy_cost + card.energy_cost_upgrade_delta).max(0)
    } else {
        card.energy_cost
    }
}

/// Resolve a canonical var by name (matching kind / generic / generic
/// minus "Power" suffix / key) at a given upgrade level. Returns the
/// base value plus accumulated upgrade deltas, or None if the card has
/// no such var.
fn effective_var(card: &CardData, var_kind: &str, upgrade_level: i32) -> Option<f64> {
    let base = card
        .canonical_vars
        .iter()
        .find(|v| var_matches(v, var_kind))?
        .base_value?;
    let delta_sum: f64 = card
        .upgrade_deltas
        .iter()
        .filter(|d| d.var_kind == var_kind)
        .map(|d| d.delta)
        .sum();
    Some(base + delta_sum * upgrade_level as f64)
}

// ---------- Combat observation bundle (Phase 1.4) --------------------

/// One complete combat observation. The agent's transformer trunk
/// processes the variable-length card / enemy / relic lists; the fixed
/// scalars become a small dense head.
///
/// Per-pile card feature vectors include the card's current instance
/// state (upgrade level, enchantment) — the agent never has to look
/// the runtime info up separately.
#[derive(Clone, Debug, Serialize)]
pub struct CombatObservation {
    /// Pinned to `OBSERVATION_SCHEMA_VERSION` at observation time.
    /// Agents reject observations whose version doesn't match their
    /// training-time version.
    pub schema_version: u32,

    // ----- Global combat context -----
    /// `encounter_id`, mostly for telemetry / logging.
    pub encounter_id: Option<String>,
    pub round_number: i32,
    /// Encoded as 0 = None, 1 = Player, 2 = Enemy.
    pub current_side: u8,

    // ----- Player state -----
    /// Player creature features. For single-player runs there's just
    /// one entry; the vec exists to support coop.
    pub players: Vec<CreatureStateFeatures>,
    /// Per-player energy state: (current_energy, turn_energy). Same
    /// order as `players`.
    pub player_energy: Vec<(i32, i32)>,
    /// Relic feature vectors per player. Outer index matches `players`.
    pub player_relics: Vec<Vec<RelicFeatures>>,

    // ----- Card piles -----
    /// Player-indexed card-pile features. Each inner Vec is one pile
    /// in order: [Draw, Hand, Discard, Exhaust]. Cards inside a pile
    /// keep their natural ordering (top-of-pile first).
    pub player_piles: Vec<PilesFeatures>,

    // ----- Enemies -----
    pub enemies: Vec<CreatureStateFeatures>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PilesFeatures {
    pub draw: Vec<CardFeatures>,
    pub hand: Vec<CardFeatures>,
    pub discard: Vec<CardFeatures>,
    pub exhaust: Vec<CardFeatures>,
}

/// Produce a `CombatObservation` from the live combat state. Cheap —
/// the agent can call this every step. Allocates fresh Vecs per
/// observation; if profiling reveals churn during training, swap to a
/// reusable buffer.
pub fn observe_combat(cs: &crate::combat::CombatState) -> CombatObservation {
    let players_vec: Vec<CreatureStateFeatures> = cs
        .allies
        .iter()
        .filter(|c| c.kind == CreatureKind::Player)
        .map(creature_state_features)
        .collect();

    let player_energy: Vec<(i32, i32)> = cs
        .allies
        .iter()
        .filter(|c| c.kind == CreatureKind::Player)
        .map(|c| {
            let ps = c.player.as_ref();
            (
                ps.map(|p| p.energy).unwrap_or(0),
                ps.map(|p| p.turn_energy).unwrap_or(0),
            )
        })
        .collect();

    let player_relics: Vec<Vec<RelicFeatures>> = cs
        .allies
        .iter()
        .filter(|c| c.kind == CreatureKind::Player)
        .map(|c| {
            let ps = c.player.as_ref();
            ps.map(|p| {
                p.relics
                    .iter()
                    .filter_map(|id| crate::relic::by_id(id))
                    .map(relic_features)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        })
        .collect();

    let player_piles: Vec<PilesFeatures> = cs
        .allies
        .iter()
        .filter(|c| c.kind == CreatureKind::Player)
        .map(|c| {
            let ps = c.player.as_ref();
            match ps {
                Some(p) => PilesFeatures {
                    draw: cards_to_features(&p.draw.cards),
                    hand: cards_to_features(&p.hand.cards),
                    discard: cards_to_features(&p.discard.cards),
                    exhaust: cards_to_features(&p.exhaust.cards),
                },
                None => PilesFeatures {
                    draw: Vec::new(),
                    hand: Vec::new(),
                    discard: Vec::new(),
                    exhaust: Vec::new(),
                },
            }
        })
        .collect();

    let enemies: Vec<CreatureStateFeatures> =
        cs.enemies.iter().map(creature_state_features).collect();

    let current_side = match cs.current_side {
        crate::combat::CombatSide::None => 0u8,
        crate::combat::CombatSide::Player => 1u8,
        crate::combat::CombatSide::Enemy => 2u8,
    };

    CombatObservation {
        schema_version: OBSERVATION_SCHEMA_VERSION,
        encounter_id: cs.encounter_id.clone(),
        round_number: cs.round_number,
        current_side,
        players: players_vec,
        player_energy,
        player_relics,
        player_piles,
        enemies,
    }
}

fn cards_to_features(cards: &[CardInstance]) -> Vec<CardFeatures> {
    cards
        .iter()
        .filter_map(|inst| {
            let data = crate::card::by_id(&inst.id)?;
            Some(card_features(data, Some(inst)))
        })
        .collect()
}

// ---------- Relic features (Phase 1.2) --------------------------------

/// Relic feature vector size. Smaller than cards — no target/upgrade.
pub const RELIC_FEATURE_DIM: usize = 21;

#[derive(Clone, Copy, Debug)]
pub struct RelicFeatures {
    pub values: [f32; RELIC_FEATURE_DIM],
}

impl RelicFeatures {
    pub fn as_slice(&self) -> &[f32] {
        &self.values
    }
}

impl Serialize for RelicFeatures {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serialize_f32_slice(&self.values, s)
    }
}

// Layout:
//   0..8:  RelicRarity one-hot (None / Starter / Common / Uncommon / Rare /
//          Shop / Event / Ancient)
//   8..17: Pool one-hot (Ironclad / Silent / Defect / Regent / Necrobinder /
//          Shared / Event / Deprecated / Fallback). Multi-pool relics
//          (LastingCandy ∈ Event ∩ Shared) set all that apply.
//   17:    has_canonical_vars
//   18:    canonical_var_count (raw)
//   19:    first_var_base_value (heuristic; many relics have one numeric
//          knob like BurningBlood's Heal=6)
//   20:    reserved for v2
const IDX_RELIC_RARITY_BASE: usize = 0; // 8 slots
const IDX_RELIC_POOL_BASE: usize = 8; // 9 slots
const IDX_RELIC_HAS_VARS: usize = 17;
const IDX_RELIC_VAR_COUNT: usize = 18;
const IDX_RELIC_FIRST_VAR: usize = 19;
// Index 20 reserved.

pub fn relic_features(relic: &RelicData) -> RelicFeatures {
    let mut v = [0.0f32; RELIC_FEATURE_DIM];

    one_hot(&mut v, IDX_RELIC_RARITY_BASE, relic_rarity_index(relic.rarity));

    // Pool multi-hot — relics can span pools.
    for pool in &relic.pools {
        if let Some(idx) = pool_index(pool) {
            v[IDX_RELIC_POOL_BASE + idx] = 1.0;
        }
    }

    v[IDX_RELIC_HAS_VARS] = bool_f32(!relic.canonical_vars.is_empty());
    v[IDX_RELIC_VAR_COUNT] = relic.canonical_vars.len() as f32;
    if let Some(first) = relic.canonical_vars.first().and_then(|cv| cv.base_value) {
        v[IDX_RELIC_FIRST_VAR] = first as f32;
    }

    RelicFeatures { values: v }
}

fn relic_rarity_index(r: RelicRarity) -> usize {
    match r {
        RelicRarity::None => 0,
        RelicRarity::Starter => 1,
        RelicRarity::Common => 2,
        RelicRarity::Uncommon => 3,
        RelicRarity::Rare => 4,
        RelicRarity::Shop => 5,
        RelicRarity::Event => 6,
        RelicRarity::Ancient => 7,
    }
}

fn pool_index(pool: &str) -> Option<usize> {
    match pool {
        "Ironclad" => Some(0),
        "Silent" => Some(1),
        "Defect" => Some(2),
        "Regent" => Some(3),
        "Necrobinder" => Some(4),
        "Shared" => Some(5),
        "Event" => Some(6),
        "Deprecated" => Some(7),
        "Fallback" => Some(8),
        _ => None,
    }
}

// ---------- Enemy / creature state features (Phase 1.3) --------------

/// Per-creature state feature vector. Used for enemies (and player
/// creatures for the value head's hp-preservation reward).
pub const CREATURE_STATE_FEATURE_DIM: usize = 14;

#[derive(Clone, Copy, Debug)]
pub struct CreatureStateFeatures {
    pub values: [f32; CREATURE_STATE_FEATURE_DIM],
}

impl CreatureStateFeatures {
    pub fn as_slice(&self) -> &[f32] {
        &self.values
    }
}

impl Serialize for CreatureStateFeatures {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serialize_f32_slice(&self.values, s)
    }
}

// Layout:
//   0: is_player (1.0 if CreatureKind::Player, else 0.0)
//   1: alive (current_hp > 0)
//   2: current_hp / max(1, max_hp) — normalized
//   3: max_hp (raw — agent learns its own scale)
//   4: block (raw)
//   5: strength_amount (StrengthPower, signed)
//   6: vulnerable_stack (VulnerablePower)
//   7: weak_stack (WeakPower)
//   8: poison_stack (PoisonPower)
//   9: frail_stack (FrailPower)
//  10: intangible_stack
//  11: dexterity_stack
//  12: artifact_stack
//  13: power_count_total (cardinality of `powers` for ones we don't
//      have a dedicated slot for; reserved for v2 expansion)
const IDX_CREATURE_IS_PLAYER: usize = 0;
const IDX_CREATURE_ALIVE: usize = 1;
const IDX_CREATURE_HP_FRAC: usize = 2;
const IDX_CREATURE_MAX_HP: usize = 3;
const IDX_CREATURE_BLOCK: usize = 4;
const IDX_CREATURE_STRENGTH: usize = 5;
const IDX_CREATURE_VULNERABLE: usize = 6;
const IDX_CREATURE_WEAK: usize = 7;
const IDX_CREATURE_POISON: usize = 8;
const IDX_CREATURE_FRAIL: usize = 9;
const IDX_CREATURE_INTANGIBLE: usize = 10;
const IDX_CREATURE_DEXTERITY: usize = 11;
const IDX_CREATURE_ARTIFACT: usize = 12;
const IDX_CREATURE_POWER_COUNT: usize = 13;

pub fn creature_state_features(c: &Creature) -> CreatureStateFeatures {
    let mut v = [0.0f32; CREATURE_STATE_FEATURE_DIM];
    v[IDX_CREATURE_IS_PLAYER] = bool_f32(c.kind == CreatureKind::Player);
    v[IDX_CREATURE_ALIVE] = bool_f32(c.current_hp > 0);
    let max_hp = c.max_hp.max(1) as f32;
    v[IDX_CREATURE_HP_FRAC] = c.current_hp as f32 / max_hp;
    v[IDX_CREATURE_MAX_HP] = c.max_hp as f32;
    v[IDX_CREATURE_BLOCK] = c.block as f32;
    v[IDX_CREATURE_STRENGTH] = power_amount(&c.powers, "StrengthPower") as f32;
    v[IDX_CREATURE_VULNERABLE] = power_amount(&c.powers, "VulnerablePower") as f32;
    v[IDX_CREATURE_WEAK] = power_amount(&c.powers, "WeakPower") as f32;
    v[IDX_CREATURE_POISON] = power_amount(&c.powers, "PoisonPower") as f32;
    v[IDX_CREATURE_FRAIL] = power_amount(&c.powers, "FrailPower") as f32;
    v[IDX_CREATURE_INTANGIBLE] = power_amount(&c.powers, "IntangiblePower") as f32;
    v[IDX_CREATURE_DEXTERITY] = power_amount(&c.powers, "DexterityPower") as f32;
    v[IDX_CREATURE_ARTIFACT] = power_amount(&c.powers, "ArtifactPower") as f32;
    v[IDX_CREATURE_POWER_COUNT] = c.powers.len() as f32;
    CreatureStateFeatures { values: v }
}

fn power_amount(powers: &[PowerInstance], id: &str) -> i32 {
    powers.iter().find(|p| p.id == id).map(|p| p.amount).unwrap_or(0)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::by_id as card_by_id;

    #[test]
    fn schema_version_v1() {
        assert_eq!(OBSERVATION_SCHEMA_VERSION, 1);
        assert_eq!(CARD_FEATURE_DIM, 45);
    }

    #[test]
    fn strike_ironclad_features() {
        let strike = card_by_id("StrikeIronclad").unwrap();
        let f = card_features(strike, None);
        assert_eq!(f.values[IDX_ENERGY_COST], 1.0);
        // CardType::Attack one-hot.
        assert_eq!(f.values[IDX_CARD_TYPE_BASE + 1], 1.0);
        // CardRarity::Basic one-hot.
        assert_eq!(f.values[IDX_RARITY_BASE + 1], 1.0);
        // TargetType::AnyEnemy one-hot.
        assert_eq!(f.values[IDX_TARGET_BASE + 2], 1.0);
        assert_eq!(f.values[IDX_HAS_DAMAGE], 1.0);
        assert_eq!(f.values[IDX_DAMAGE_AMOUNT], 6.0);
        assert_eq!(f.values[IDX_HAS_BLOCK], 0.0);
        assert_eq!(f.values[IDX_UPGRADE_LEVEL], 0.0);
    }

    #[test]
    fn upgraded_strike_damage_increases() {
        let strike = card_by_id("StrikeIronclad").unwrap();
        let inst = CardInstance::from_card(strike, 1);
        let f = card_features(strike, Some(&inst));
        // 6 base + 3 upgrade delta = 9.
        assert_eq!(f.values[IDX_DAMAGE_AMOUNT], 9.0);
        assert_eq!(f.values[IDX_UPGRADE_LEVEL], 1.0);
    }

    #[test]
    fn defend_ironclad_features() {
        let defend = card_by_id("DefendIronclad").unwrap();
        let f = card_features(defend, None);
        assert_eq!(f.values[IDX_ENERGY_COST], 1.0);
        // CardType::Skill.
        assert_eq!(f.values[IDX_CARD_TYPE_BASE + 2], 1.0);
        // TargetType::SelfTarget.
        assert_eq!(f.values[IDX_TARGET_BASE + 1], 1.0);
        assert_eq!(f.values[IDX_HAS_DAMAGE], 0.0);
        assert_eq!(f.values[IDX_HAS_BLOCK], 1.0);
        assert_eq!(f.values[IDX_BLOCK_AMOUNT], 5.0);
    }

    #[test]
    fn bash_applies_vulnerable() {
        let bash = card_by_id("Bash").unwrap();
        let f = card_features(bash, None);
        assert_eq!(f.values[IDX_ENERGY_COST], 2.0);
        assert_eq!(f.values[IDX_DAMAGE_AMOUNT], 8.0);
        // Bash applies 2 Vulnerable; canonical_vars carries
        // PowerVar<VulnerablePower>(2).
        assert_eq!(f.values[IDX_APPLIES_VULNERABLE], 2.0);
    }

    #[test]
    fn neutralize_applies_weak() {
        let n = card_by_id("Neutralize").unwrap();
        let f = card_features(n, None);
        assert_eq!(f.values[IDX_ENERGY_COST], 0.0);
        assert_eq!(f.values[IDX_DAMAGE_AMOUNT], 3.0);
        assert_eq!(f.values[IDX_APPLIES_WEAK], 1.0);
    }

    #[test]
    fn acrobatics_card_draw() {
        let a = card_by_id("Acrobatics").unwrap();
        let f = card_features(a, None);
        assert_eq!(f.values[IDX_HAS_CARD_DRAW], 1.0);
        assert_eq!(f.values[IDX_CARD_DRAW_AMOUNT], 3.0);
    }

    #[test]
    fn whirlwind_x_cost() {
        let ww = card_by_id("Whirlwind").unwrap();
        let f = card_features(ww, None);
        assert_eq!(f.values[IDX_HAS_ENERGY_COST_X], 1.0);
        // TargetType::AllEnemies.
        assert_eq!(f.values[IDX_TARGET_BASE + 3], 1.0);
    }

    #[test]
    fn unupgradable_status_features() {
        let wound = card_by_id("Wound").unwrap();
        let f = card_features(wound, None);
        // CardType::Status.
        assert_eq!(f.values[IDX_CARD_TYPE_BASE + 4], 1.0);
        // CardRarity::Status.
        assert_eq!(f.values[IDX_RARITY_BASE + 8], 1.0);
        // max_upgrade_level = 0.
        assert_eq!(f.values[IDX_MAX_UPGRADE_LEVEL], 0.0);
    }

    #[test]
    fn enchantment_flag_set_when_instance_carries_one() {
        use crate::combat::EnchantmentInstance;
        let strike = card_by_id("StrikeIronclad").unwrap();
        let mut inst = CardInstance::from_card(strike, 0);
        inst.enchantment = Some(EnchantmentInstance {
            id: "Sharp".to_string(),
            amount: 2,
        });
        let f = card_features(strike, Some(&inst));
        assert_eq!(f.values[IDX_HAS_ENCHANTMENT], 1.0);
        // No-enchantment baseline:
        let inst2 = CardInstance::from_card(strike, 0);
        let f2 = card_features(strike, Some(&inst2));
        assert_eq!(f2.values[IDX_HAS_ENCHANTMENT], 0.0);
    }

    // ---------- Relic feature tests -----------------------------------

    #[test]
    fn relic_feature_dim_is_21() {
        assert_eq!(RELIC_FEATURE_DIM, 21);
    }

    #[test]
    fn burning_blood_features() {
        let bb = crate::relic::by_id("BurningBlood").unwrap();
        let f = relic_features(bb);
        // Starter rarity (index 1).
        assert_eq!(f.values[IDX_RELIC_RARITY_BASE + 1], 1.0);
        // Pool = Ironclad (index 0 within pool block).
        assert_eq!(f.values[IDX_RELIC_POOL_BASE + 0], 1.0);
        // Has canonical vars; first var is Heal=6.
        assert_eq!(f.values[IDX_RELIC_HAS_VARS], 1.0);
        assert_eq!(f.values[IDX_RELIC_VAR_COUNT], 1.0);
        assert_eq!(f.values[IDX_RELIC_FIRST_VAR], 6.0);
    }

    #[test]
    fn anchor_pool_is_shared() {
        let a = crate::relic::by_id("Anchor").unwrap();
        let f = relic_features(a);
        // Common rarity (index 2).
        assert_eq!(f.values[IDX_RELIC_RARITY_BASE + 2], 1.0);
        // Pool = Shared (index 5).
        assert_eq!(f.values[IDX_RELIC_POOL_BASE + 5], 1.0);
        // Block 10 is the canonical var.
        assert_eq!(f.values[IDX_RELIC_FIRST_VAR], 10.0);
    }

    #[test]
    fn lasting_candy_is_multi_pool() {
        let lc = crate::relic::by_id("LastingCandy").unwrap();
        let f = relic_features(lc);
        // Event AND Shared both set.
        assert_eq!(f.values[IDX_RELIC_POOL_BASE + 5], 1.0); // Shared
        assert_eq!(f.values[IDX_RELIC_POOL_BASE + 6], 1.0); // Event
    }

    #[test]
    fn every_relic_produces_finite_feature_vector() {
        for relic in crate::relic::ALL_RELICS.iter() {
            let f = relic_features(relic);
            for (i, x) in f.values.iter().enumerate() {
                assert!(
                    x.is_finite(),
                    "relic {} index {} is not finite: {}",
                    relic.id, i, x
                );
            }
        }
    }

    // ---------- Creature state feature tests --------------------------

    #[test]
    fn creature_state_feature_dim_is_14() {
        assert_eq!(CREATURE_STATE_FEATURE_DIM, 14);
    }

    #[test]
    fn fresh_axebot_state_features() {
        use crate::combat::{
            deck_from_ids, CombatSide, CombatState, PlayerSetup,
        };
        use crate::{character, encounter};
        let ironclad = character::by_id("Ironclad").unwrap();
        let enc = encounter::by_id("AxebotsNormal").unwrap();
        let deck = deck_from_ids(&ironclad.starting_deck);
        let setup = PlayerSetup {
            character: ironclad,
            current_hp: 80,
            max_hp: 80,
            deck,
            relics: ironclad.starting_relics.clone(),
        };
        let cs = CombatState::start(enc, vec![setup], Vec::new());
        let axebot = &cs.enemies[0];
        let f = creature_state_features(axebot);
        assert_eq!(f.values[IDX_CREATURE_IS_PLAYER], 0.0);
        assert_eq!(f.values[IDX_CREATURE_ALIVE], 1.0);
        // Fresh axebot at full HP → hp_frac = 1.0.
        assert!((f.values[IDX_CREATURE_HP_FRAC] - 1.0).abs() < 1e-6);
        assert_eq!(f.values[IDX_CREATURE_MAX_HP], 44.0);
        assert_eq!(f.values[IDX_CREATURE_BLOCK], 0.0);
        assert_eq!(f.values[IDX_CREATURE_POWER_COUNT], 0.0);
        // Suppress unused
        let _ = CombatSide::Player;
    }

    #[test]
    fn creature_powers_populate_dedicated_slots() {
        use crate::combat::{
            deck_from_ids, CombatSide, CombatState, PlayerSetup,
        };
        use crate::{character, encounter};
        let ironclad = character::by_id("Ironclad").unwrap();
        let enc = encounter::by_id("AxebotsNormal").unwrap();
        let deck = deck_from_ids(&ironclad.starting_deck);
        let setup = PlayerSetup {
            character: ironclad,
            current_hp: 80,
            max_hp: 80,
            deck,
            relics: ironclad.starting_relics.clone(),
        };
        let mut cs = CombatState::start(enc, vec![setup], Vec::new());
        cs.apply_power(CombatSide::Enemy, 0, "StrengthPower", 3);
        cs.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 2);
        cs.apply_power(CombatSide::Enemy, 0, "PoisonPower", 5);

        let f = creature_state_features(&cs.enemies[0]);
        assert_eq!(f.values[IDX_CREATURE_STRENGTH], 3.0);
        assert_eq!(f.values[IDX_CREATURE_VULNERABLE], 2.0);
        assert_eq!(f.values[IDX_CREATURE_POISON], 5.0);
        assert_eq!(f.values[IDX_CREATURE_POWER_COUNT], 3.0);
        // Powers we don't track in dedicated slots → zero.
        assert_eq!(f.values[IDX_CREATURE_FRAIL], 0.0);
        assert_eq!(f.values[IDX_CREATURE_INTANGIBLE], 0.0);
    }

    // ---------- Observation bundle tests ------------------------------

    fn build_ironclad_state() -> crate::combat::CombatState {
        use crate::combat::{deck_from_ids, CombatState, PlayerSetup};
        use crate::{character, encounter};
        let ironclad = character::by_id("Ironclad").unwrap();
        let enc = encounter::by_id("AxebotsNormal").unwrap();
        let deck = deck_from_ids(&ironclad.starting_deck);
        let setup = PlayerSetup {
            character: ironclad,
            current_hp: 80,
            max_hp: 80,
            deck,
            relics: ironclad.starting_relics.clone(),
        };
        CombatState::start(enc, vec![setup], Vec::new())
    }

    #[test]
    fn observation_carries_schema_version() {
        let cs = build_ironclad_state();
        let obs = observe_combat(&cs);
        assert_eq!(obs.schema_version, OBSERVATION_SCHEMA_VERSION);
    }

    #[test]
    fn observation_has_one_player_two_enemies() {
        let cs = build_ironclad_state();
        let obs = observe_combat(&cs);
        assert_eq!(obs.players.len(), 1);
        assert_eq!(obs.enemies.len(), 2);
        assert_eq!(obs.player_energy.len(), 1);
        assert_eq!(obs.player_relics.len(), 1);
        assert_eq!(obs.player_piles.len(), 1);
    }

    #[test]
    fn observation_player_energy_matches_state() {
        let mut cs = build_ironclad_state();
        cs.allies[0].player.as_mut().unwrap().energy = 2;
        let obs = observe_combat(&cs);
        assert_eq!(obs.player_energy[0], (2, 3));
    }

    #[test]
    fn observation_pile_sizes_match_state() {
        let cs = build_ironclad_state();
        let obs = observe_combat(&cs);
        // 10-card Ironclad starter deck, all in draw, none drawn yet.
        assert_eq!(obs.player_piles[0].draw.len(), 10);
        assert_eq!(obs.player_piles[0].hand.len(), 0);
        assert_eq!(obs.player_piles[0].discard.len(), 0);
        assert_eq!(obs.player_piles[0].exhaust.len(), 0);
    }

    #[test]
    fn observation_relic_features_present() {
        let cs = build_ironclad_state();
        let obs = observe_combat(&cs);
        // Ironclad starts with BurningBlood.
        assert_eq!(obs.player_relics[0].len(), 1);
        let bb_feats = obs.player_relics[0][0];
        // Starter rarity slot.
        assert_eq!(bb_feats.values[IDX_RELIC_RARITY_BASE + 1], 1.0);
    }

    #[test]
    fn observation_current_side_starts_player() {
        let cs = build_ironclad_state();
        let obs = observe_combat(&cs);
        assert_eq!(obs.current_side, 1);
        assert_eq!(obs.round_number, 1);
    }

    #[test]
    fn dead_creature_alive_flag_zero() {
        use crate::combat::{
            deck_from_ids, CombatState, PlayerSetup,
        };
        use crate::{character, encounter};
        let ironclad = character::by_id("Ironclad").unwrap();
        let enc = encounter::by_id("AxebotsNormal").unwrap();
        let deck = deck_from_ids(&ironclad.starting_deck);
        let setup = PlayerSetup {
            character: ironclad,
            current_hp: 80,
            max_hp: 80,
            deck,
            relics: ironclad.starting_relics.clone(),
        };
        let mut cs = CombatState::start(enc, vec![setup], Vec::new());
        cs.enemies[0].current_hp = 0;
        let f = creature_state_features(&cs.enemies[0]);
        assert_eq!(f.values[IDX_CREATURE_ALIVE], 0.0);
        assert_eq!(f.values[IDX_CREATURE_HP_FRAC], 0.0);
    }

    #[test]
    fn every_card_produces_finite_feature_vector() {
        // Sanity sweep: every card in the static table produces a
        // feature vector with no NaN/inf values. Catches future
        // table additions whose canonical_vars have unexpected
        // base_value shapes.
        for card in crate::card::ALL_CARDS.iter() {
            let f = card_features(card, None);
            for (i, x) in f.values.iter().enumerate() {
                assert!(
                    x.is_finite(),
                    "card {} index {} is not finite: {}",
                    card.id, i, x
                );
            }
        }
    }
}
