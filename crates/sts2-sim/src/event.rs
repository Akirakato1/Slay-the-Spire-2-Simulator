//! Event data table. 59 EventModel subclasses (9 of the 68 files in
//! Models/Events/ are intermediate bases that don't derive directly).

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize)]
pub struct EventData {
    pub id: String,
    #[serde(default)]
    pub is_shared: bool,
    /// Localization keys of the initial-page options. Outcomes deferred to
    /// the behavior port.
    #[serde(default)]
    pub initial_option_labels: Vec<String>,
    #[serde(default)]
    pub canonical_vars: Vec<EventVar>,
    /// Acts whose runtime event pool includes this event. Acts'
    /// `AllEvents` overrides are concatenated with `ModelDb.AllSharedEvents`
    /// — shared events appear in all 4 canonical act pools, act-specific
    /// events appear only in their own. An empty list means the event is
    /// not in any canonical pool (deprecated / never-shipped / event-
    /// chain-only).
    #[serde(default)]
    pub acts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EventVar {
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

const EVENTS_JSON: &str = include_str!("../data/events.json");

pub static ALL_EVENTS: LazyLock<Vec<EventData>> = LazyLock::new(|| {
    let mut v: Vec<EventData> =
        serde_json::from_str(EVENTS_JSON).expect("events.json parse failed");
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
});

pub static EVENT_INDEX: LazyLock<HashMap<&'static str, &'static EventData>> =
    LazyLock::new(|| ALL_EVENTS.iter().map(|e| (e.id.as_str(), e)).collect());

pub fn by_id(id: &str) -> Option<&'static EventData> {
    EVENT_INDEX.get(id).copied()
}

/// All events in a specific act's runtime event pool (act-specific
/// `AllEvents` concatenated with `ModelDb.AllSharedEvents`). Mirrors
/// `ActModel.GenerateRooms` line 489.
pub fn events_for_act(act: &str) -> Vec<&'static EventData> {
    ALL_EVENTS
        .iter()
        .filter(|e| e.acts.iter().any(|a| a == act))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_is_fifty_nine() {
        assert_eq!(ALL_EVENTS.len(), 59);
    }

    #[test]
    fn abyssal_baths_has_two_initial_options() {
        let e = by_id("AbyssalBaths").expect("AbyssalBaths present");
        assert_eq!(e.initial_option_labels.len(), 2);
        assert!(e.canonical_vars.iter().any(|v| v.kind == "Heal"));
    }

    /// Many events have statically-listed options; some (e.g.
    /// ColorfulPhilosophers) build their option list dynamically in a loop
    /// driven by runtime state. The static extractor captures the literal
    /// string prefix(es) and leaves arity to the behavior port.
    #[test]
    fn at_least_half_of_events_have_extracted_options() {
        let with_options = ALL_EVENTS
            .iter()
            .filter(|e| !e.initial_option_labels.is_empty())
            .count();
        assert!(
            with_options >= ALL_EVENTS.len() / 2,
            "only {} of {} events have extracted option labels",
            with_options,
            ALL_EVENTS.len()
        );
    }

    /// Per-act event pools are populated. The pool sizes are
    /// (act-specific count) + (18 shared events) per C# source.
    #[test]
    fn act_event_pools_are_non_empty() {
        for act in ["Overgrowth", "Hive", "Glory", "Underdocks"] {
            let pool = events_for_act(act);
            assert!(!pool.is_empty(), "{} has no events", act);
            // 18 shared events appear in every act, so every act's
            // pool must be at least that big.
            assert!(pool.len() >= 18,
                "{} has only {} events (expected ≥ 18 shared)",
                act, pool.len(),
            );
        }
    }

    /// `FakeMerchant` is in `ModelDb.AllSharedEvents` (line 348) so it
    /// must show up in all 4 canonical act pools.
    #[test]
    fn shared_events_appear_in_every_act() {
        let fm = by_id("FakeMerchant").unwrap();
        for act in ["Overgrowth", "Hive", "Glory", "Underdocks"] {
            assert!(fm.acts.iter().any(|a| a == act),
                "FakeMerchant missing from {}", act);
        }
    }

    /// `ByrdonisNest` is in Overgrowth's `AllEvents` list (line 107)
    /// but is NOT shared — should appear in Overgrowth only.
    #[test]
    fn act_specific_events_stay_in_their_act() {
        let bn = by_id("ByrdonisNest").unwrap();
        assert!(bn.acts.iter().any(|a| a == "Overgrowth"));
        assert!(!bn.acts.iter().any(|a| a == "Hive"),
            "ByrdonisNest leaked into Hive");
    }
}
