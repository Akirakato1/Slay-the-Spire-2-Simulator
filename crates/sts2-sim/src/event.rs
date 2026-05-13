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
}
