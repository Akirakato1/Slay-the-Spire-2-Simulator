//! Enchantment data table. Enchantments are buffs that attach to cards
//! (modifying damage / block / triggers); 23 entries.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize)]
pub struct EnchantmentVar {
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

#[derive(Clone, Debug, Deserialize)]
pub struct EnchantmentData {
    pub id: String,
    #[serde(default)]
    pub has_extra_card_text: bool,
    #[serde(default)]
    pub show_amount: bool,
    /// `CardType` values referenced in the C# `CanEnchantCardType` override.
    /// Empty means the override is absent (= applies to any card per the
    /// base default). Interpretation of `==` vs `!=` is deferred to the
    /// behavior port.
    #[serde(default)]
    pub applicable_card_types: Vec<String>,
    #[serde(default)]
    pub canonical_vars: Vec<EnchantmentVar>,
}

const ENCHANTMENTS_JSON: &str = include_str!("../data/enchantments.json");

pub static ALL_ENCHANTMENTS: LazyLock<Vec<EnchantmentData>> = LazyLock::new(|| {
    let mut v: Vec<EnchantmentData> =
        serde_json::from_str(ENCHANTMENTS_JSON).expect("enchantments.json parse failed");
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
});

pub static ENCHANTMENT_INDEX: LazyLock<HashMap<&'static str, &'static EnchantmentData>> =
    LazyLock::new(|| {
        ALL_ENCHANTMENTS
            .iter()
            .map(|e| (e.id.as_str(), e))
            .collect()
    });

pub fn by_id(id: &str) -> Option<&'static EnchantmentData> {
    ENCHANTMENT_INDEX.get(id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_is_twenty_three() {
        assert_eq!(ALL_ENCHANTMENTS.len(), 23);
    }

    #[test]
    fn sharp_is_attack_only() {
        let s = by_id("Sharp").expect("Sharp present");
        assert_eq!(s.applicable_card_types, vec!["Attack"]);
        assert!(s.show_amount);
    }

    #[test]
    fn corrupted_is_attack_only() {
        let c = by_id("Corrupted").expect("Corrupted present");
        assert!(c.has_extra_card_text);
        assert_eq!(c.applicable_card_types, vec!["Attack"]);
    }

    #[test]
    fn adroit_has_block_var() {
        let a = by_id("Adroit").expect("Adroit present");
        assert!(a.has_extra_card_text);
        assert!(a.show_amount);
        assert_eq!(a.canonical_vars[0].kind, "Block");
    }
}
