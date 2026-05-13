//! Potion data table. 64 potions; captures rarity, usage timing,
//! target_type, and any canonical magic numbers. Behavior (`OnUse`) deferred.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize)]
pub struct PotionData {
    pub id: String,
    pub pools: Vec<String>,
    pub rarity: PotionRarity,
    pub usage: PotionUsage,
    pub target_type: Option<String>,
    #[serde(default)]
    pub canonical_vars: Vec<PotionVar>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PotionVar {
    pub kind: String,
    #[serde(default)]
    pub generic: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub base_value: Option<f64>,
    #[serde(default)]
    pub value_prop: Option<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize)]
pub enum PotionRarity {
    None,
    Common,
    Uncommon,
    Rare,
    Event,
    Token,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize)]
pub enum PotionUsage {
    None,
    CombatOnly,
    AnyTime,
    Automatic,
}

const POTIONS_JSON: &str = include_str!("../data/potions.json");

pub static ALL_POTIONS: LazyLock<Vec<PotionData>> = LazyLock::new(|| {
    let mut v: Vec<PotionData> =
        serde_json::from_str(POTIONS_JSON).expect("potions.json parse failed");
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
});

pub static POTION_INDEX: LazyLock<HashMap<&'static str, &'static PotionData>> =
    LazyLock::new(|| ALL_POTIONS.iter().map(|p| (p.id.as_str(), p)).collect());

pub fn by_id(id: &str) -> Option<&'static PotionData> {
    POTION_INDEX.get(id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_is_sixty_four() {
        assert_eq!(ALL_POTIONS.len(), 64);
    }

    #[test]
    fn basics_have_known_signatures() {
        let attack = by_id("AttackPotion").unwrap();
        assert_eq!(attack.rarity, PotionRarity::Common);
        assert_eq!(attack.usage, PotionUsage::CombatOnly);
        assert!(attack.pools.contains(&"Shared".to_string()));

        let fairy = by_id("FairyInABottle").unwrap();
        assert_eq!(fairy.rarity, PotionRarity::Rare);
        assert_eq!(fairy.usage, PotionUsage::Automatic);

        let entropic = by_id("EntropicBrew").unwrap();
        assert_eq!(entropic.usage, PotionUsage::AnyTime);
    }
}
