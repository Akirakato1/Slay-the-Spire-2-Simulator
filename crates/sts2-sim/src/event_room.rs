//! Event room infrastructure.
//!
//! Each event in the C# game is a subclass of `EventModel` with 2-4
//! `EventOption` choices, each triggering an arbitrary effect chain.
//! The Rust port models this as data: `EventModel { id, choices }`
//! with `EventChoice { label, body: Vec<Effect> }`. New events are
//! one match arm in `event_choices(id)` rather than a new struct.
//!
//! The full event roster (59 in `events.json`) lands incrementally —
//! this MVP wires the infrastructure plus two canonical examples:
//!
//!   - **LostWisp**: 2 choices. Claim → add Decay curse + grant
//!     LostWisp relic. Search → gain 45-75 gold.
//!   - **GraveOfTheForgotten**: 2 choices. Confront → add Decay curse
//!     (simplified; full C# also enchants a card with SoulsPower
//!     which is left as a TODO once the run-state enchantment-apply
//!     primitive lands). Accept → grant ForgottenSoul relic.

use crate::effects::{AmountSpec, Effect};
use crate::run_state::RunState;

/// One choice within an event. C# `EventOption`.
#[derive(Debug, Clone)]
pub struct EventChoice {
    /// Short identifier (matches the C# enum-like option keys, e.g.
    /// "CLAIM", "SEARCH", "CONFRONT", "ACCEPT"). Used for replay /
    /// feature extraction; not displayed in-engine.
    pub label: String,
    /// Effects fired in order when this choice is resolved.
    pub body: Vec<Effect>,
}

/// One event: id + the available choices. Loaded via
/// `event_choices(id)`.
#[derive(Debug, Clone)]
pub struct EventModel {
    pub id: String,
    pub choices: Vec<EventChoice>,
}

/// One in-flight event awaiting resolution. RL agent reads this to
/// know what options are on offer; calls `resolve_event_choice` to
/// commit.
#[derive(Debug, Clone)]
pub struct PendingEvent {
    pub event_id: String,
    pub player_idx: usize,
    pub choices: Vec<EventChoice>,
}

/// Look up an event's choices. Returns None for unknown ids (caller
/// should treat as "event not implemented yet" — a one-arm-per-event
/// model that mirrors how cards/relics/potions are looked up).
pub fn event_choices(id: &str) -> Option<EventModel> {
    match id {
        // LostWisp: claim → +Decay curse + LostWisp relic. Search →
        // +45-75 gold (C# rolls Gold ∈ [60-15, 60+15] = 45-75 at
        // CalculateVars time; we encode the midpoint as the
        // GainRunStateGold amount and ignore the per-event jitter for
        // the MVP). Functionally captures the average outcome.
        "LostWisp" => Some(EventModel {
            id: "LostWisp".to_string(),
            choices: vec![
                EventChoice {
                    label: "CLAIM".to_string(),
                    body: vec![
                        Effect::AddCardToRunStateDeck {
                            card_id: "Decay".to_string(),
                            upgrade: 0,
                        },
                        Effect::GainRelic {
                            relic_id: "LostWisp".to_string(),
                        },
                    ],
                },
                EventChoice {
                    label: "SEARCH".to_string(),
                    body: vec![Effect::GainRunStateGold {
                        amount: AmountSpec::Fixed(60),
                    }],
                },
            ],
        }),
        // GraveOfTheForgotten: confront → add Decay curse (the
        // companion SoulsPower enchant on a deck card is deferred —
        // run-state enchantment-apply primitive doesn't exist yet).
        // Accept → grant ForgottenSoul relic.
        "GraveOfTheForgotten" => Some(EventModel {
            id: "GraveOfTheForgotten".to_string(),
            choices: vec![
                EventChoice {
                    label: "CONFRONT".to_string(),
                    body: vec![Effect::AddCardToRunStateDeck {
                        card_id: "Decay".to_string(),
                        upgrade: 0,
                    }],
                },
                EventChoice {
                    label: "ACCEPT".to_string(),
                    body: vec![Effect::GainRelic {
                        relic_id: "ForgottenSoul".to_string(),
                    }],
                },
            ],
        }),
        _ => None,
    }
}

/// Enter an event. Looks up its choices and either auto-resolves the
/// first one (default) or sets `pending_event` for an RL agent.
/// Returns true if the event was found, false otherwise.
pub fn enter_event(rs: &mut RunState, player_idx: usize, event_id: &str) -> bool {
    let Some(model) = event_choices(event_id) else {
        return false;
    };
    if rs.auto_resolve_offers {
        // Auto-resolve: take the first choice. (Not always the
        // optimal pick — RL replay should set auto_resolve_offers=false
        // and inject the recorded `.run` choice.)
        if let Some(first) = model.choices.first() {
            let body = first.body.clone();
            crate::effects::execute_run_state_effects(rs, player_idx, &body);
        }
    } else {
        rs.pending_event = Some(PendingEvent {
            event_id: event_id.to_string(),
            player_idx,
            choices: model.choices,
        });
    }
    true
}

/// Resolve a deferred event choice. `choice_index` references the
/// `choices` vec on the `pending_event`. Returns Err on invalid
/// index; the pending event is preserved on error so the caller can
/// retry with a valid pick.
pub fn resolve_event_choice(
    rs: &mut RunState,
    choice_index: usize,
) -> Result<(), String> {
    let Some(event) = rs.pending_event.take() else {
        return Err("no pending event".to_string());
    };
    let Some(choice) = event.choices.get(choice_index) else {
        let n = event.choices.len();
        rs.pending_event = Some(event);
        return Err(format!(
            "choice index {} out of range (event has {} choices)",
            choice_index, n));
    };
    let body = choice.body.clone();
    let player_idx = event.player_idx;
    crate::effects::execute_run_state_effects(rs, player_idx, &body);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::act::ActId;
    use crate::run_state::PlayerState;

    fn fresh_rs() -> RunState {
        let player = PlayerState {
            character_id: "Ironclad".to_string(),
            id: 1, hp: 80, max_hp: 80, gold: 100,
            deck: Vec::new(),
            relics: Vec::new(),
            potions: Vec::new(),
            max_potion_slot_count: 3,
        };
        RunState::new("seed", 0, vec![player], vec![ActId::Overgrowth], Vec::new())
    }

    #[test]
    fn unknown_event_returns_false() {
        let mut rs = fresh_rs();
        assert!(!enter_event(&mut rs, 0, "NonexistentEvent"));
    }

    #[test]
    fn lost_wisp_claim_grants_decay_and_relic_auto() {
        let mut rs = fresh_rs();
        assert!(enter_event(&mut rs, 0, "LostWisp"));
        // Auto-resolves to CLAIM (first choice).
        assert_eq!(rs.players()[0].deck.len(), 1);
        assert_eq!(rs.players()[0].deck[0].id, "Decay");
        assert_eq!(rs.players()[0].relics.len(), 1);
        assert_eq!(rs.players()[0].relics[0].id, "LostWisp");
    }

    #[test]
    fn deferred_lost_wisp_lets_agent_pick_search() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        assert!(enter_event(&mut rs, 0, "LostWisp"));
        let pending = rs.pending_event.as_ref().expect("staged");
        assert_eq!(pending.event_id, "LostWisp");
        assert_eq!(pending.choices.len(), 2);
        assert_eq!(pending.choices[0].label, "CLAIM");
        assert_eq!(pending.choices[1].label, "SEARCH");
        // Agent picks SEARCH (choice 1) — should gain 60 gold.
        let gold_before = rs.players()[0].gold;
        resolve_event_choice(&mut rs, 1).expect("resolve");
        assert_eq!(rs.players()[0].gold, gold_before + 60);
        // Deck untouched (no Decay).
        assert_eq!(rs.players()[0].deck.len(), 0);
        // Relics untouched.
        assert_eq!(rs.players()[0].relics.len(), 0);
    }

    #[test]
    fn grave_of_the_forgotten_accept_grants_relic() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "GraveOfTheForgotten");
        resolve_event_choice(&mut rs, 1).expect("accept choice");
        assert!(rs.players()[0].relics.iter().any(|r| r.id == "ForgottenSoul"));
        // No Decay added under Accept.
        assert!(rs.players()[0].deck.iter().all(|c| c.id != "Decay"));
    }

    #[test]
    fn grave_of_the_forgotten_confront_adds_decay() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "GraveOfTheForgotten");
        resolve_event_choice(&mut rs, 0).expect("confront choice");
        assert!(rs.players()[0].deck.iter().any(|c| c.id == "Decay"));
        // No ForgottenSoul granted.
        assert!(rs.players()[0].relics.iter().all(|r| r.id != "ForgottenSoul"));
    }

    #[test]
    fn resolve_invalid_choice_preserves_pending_event() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "LostWisp");
        let err = resolve_event_choice(&mut rs, 99).unwrap_err();
        assert!(err.contains("out of range"));
        assert!(rs.pending_event.is_some(),
            "Invalid pick must preserve pending event for retry");
    }

    #[test]
    fn resolve_without_pending_event_errors() {
        let mut rs = fresh_rs();
        let err = resolve_event_choice(&mut rs, 0).unwrap_err();
        assert!(err.contains("no pending event"));
    }
}
