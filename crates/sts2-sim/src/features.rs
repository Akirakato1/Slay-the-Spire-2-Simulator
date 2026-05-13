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
use crate::combat::CardInstance;

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
