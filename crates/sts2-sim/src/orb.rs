//! Orb (Defect channel) data table.
//!
//! 5 concrete `OrbModel` subclasses, loaded from `data/orbs.json`. Captures
//! starting passive/evoke values. Some orbs (GlassOrb's evoke) compute their
//! value from another property at runtime — those record `None` and the
//! computation lives in the (deferred) behavior port.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize)]
pub struct OrbData {
    pub id: String,
    /// Starting passive value. `None` if the value is computed at runtime
    /// from another property.
    pub passive_val: Option<f64>,
    /// Starting evoke value. `None` if computed.
    pub evoke_val: Option<f64>,
}

const ORBS_JSON: &str = include_str!("../data/orbs.json");

pub static ALL_ORBS: LazyLock<Vec<OrbData>> = LazyLock::new(|| {
    let mut orbs: Vec<OrbData> =
        serde_json::from_str(ORBS_JSON).expect("orbs.json parse failed");
    orbs.sort_by(|a, b| a.id.cmp(&b.id));
    orbs
});

pub static ORB_INDEX: LazyLock<HashMap<&'static str, &'static OrbData>> =
    LazyLock::new(|| ALL_ORBS.iter().map(|o| (o.id.as_str(), o)).collect());

pub fn by_id(id: &str) -> Option<&'static OrbData> {
    ORB_INDEX.get(id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orb_count_is_five() {
        assert_eq!(ALL_ORBS.len(), 5);
    }

    #[test]
    fn basics_have_known_signatures() {
        let lightning = by_id("LightningOrb").expect("LightningOrb present");
        assert_eq!(lightning.passive_val, Some(3.0));
        assert_eq!(lightning.evoke_val, Some(8.0));

        let frost = by_id("FrostOrb").expect("FrostOrb present");
        assert_eq!(frost.passive_val, Some(2.0));
        assert_eq!(frost.evoke_val, Some(5.0));

        // DarkOrb's evoke starts at 6 (initial field value) and grows on
        // passive triggers — the static table captures the starting value.
        let dark = by_id("DarkOrb").expect("DarkOrb present");
        assert_eq!(dark.passive_val, Some(6.0));
        assert_eq!(dark.evoke_val, Some(6.0));

        // GlassOrb's evoke is `PassiveVal * 2` — computed, so None.
        let glass = by_id("GlassOrb").expect("GlassOrb present");
        assert_eq!(glass.passive_val, Some(4.0));
        assert!(glass.evoke_val.is_none());
    }
}
