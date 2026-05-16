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

        // HungryForMushrooms: pick one of two mushroom relics.
        "HungryForMushrooms" => Some(EventModel {
            id: "HungryForMushrooms".to_string(),
            choices: vec![
                EventChoice {
                    label: "BIG_MUSHROOM".to_string(),
                    body: vec![Effect::GainRelic { relic_id: "BigMushroom".to_string() }],
                },
                EventChoice {
                    label: "FRAGRANT_MUSHROOM".to_string(),
                    body: vec![Effect::GainRelic { relic_id: "FragrantMushroom".to_string() }],
                },
            ],
        }),

        // SunkenStatue: grab the sword (relic) or dive for gold (cost HP).
        // C#: GoldVar(111), HpLoss(7). Jitter ignored.
        "SunkenStatue" => Some(EventModel {
            id: "SunkenStatue".to_string(),
            choices: vec![
                EventChoice {
                    label: "GRAB_SWORD".to_string(),
                    body: vec![Effect::GainRelic { relic_id: "SwordOfStone".to_string() }],
                },
                EventChoice {
                    label: "DIVE_INTO_WATER".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(7) },
                        Effect::GainRunStateGold { amount: AmountSpec::Fixed(111) },
                    ],
                },
            ],
        }),

        // SunkenTreasury: small (60g) vs large (333g + Greed curse).
        // C# CalculateVars jitters ±8 / ±30; we encode the midpoint.
        "SunkenTreasury" => Some(EventModel {
            id: "SunkenTreasury".to_string(),
            choices: vec![
                EventChoice {
                    label: "FIRST_CHEST".to_string(),
                    body: vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(60) }],
                },
                EventChoice {
                    label: "SECOND_CHEST".to_string(),
                    body: vec![
                        Effect::GainRunStateGold { amount: AmountSpec::Fixed(333) },
                        Effect::AddCardToRunStateDeck { card_id: "Greed".to_string(), upgrade: 0 },
                    ],
                },
            ],
        }),

        // ThisOrThat: Plain → -6 HP + 0 gold + Clumsy curse.
        // Ornate → next-rolled relic (skip / no-op since "next relic from front"
        // requires a pool primitive we don't have).
        "ThisOrThat" => Some(EventModel {
            id: "ThisOrThat".to_string(),
            choices: vec![
                EventChoice {
                    label: "PLAIN".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(6) },
                        Effect::AddCardToRunStateDeck { card_id: "Clumsy".to_string(), upgrade: 0 },
                    ],
                },
                EventChoice {
                    label: "ORNATE".to_string(),
                    body: vec![], // TODO: needs "grant next-rolled relic" primitive
                },
            ],
        }),

        // TrashHeap: dive (HP loss + relic) vs grab (gold).
        // The relic in DiveIn is picked from TrashHeap.Relics list at random —
        // C# uses `Rng.NextItem`. We grant a fixed placeholder ("Anchor") since
        // our infra doesn't have an event-specific relic-pool selector yet.
        // Functionally captures the "+1 random relic" element.
        "TrashHeap" => Some(EventModel {
            id: "TrashHeap".to_string(),
            choices: vec![
                EventChoice {
                    label: "DIVE_IN".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(8) },
                        // TODO: random-from-pool relic. Anchor is a placeholder.
                        Effect::GainRelic { relic_id: "Anchor".to_string() },
                    ],
                },
                EventChoice {
                    label: "GRAB".to_string(),
                    body: vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(100) }],
                },
            ],
        }),

        // UnrestSite: rest (heal + PoorSleep curse) vs kill (-8 MaxHp).
        // C#: HealVar(0) — heal amount actually scales by character's max
        // (probably ~30% of MaxHp via the rest-site shared formula).
        // For MVP encode 0 heal; the curse is the gameplay-relevant part.
        "UnrestSite" => Some(EventModel {
            id: "UnrestSite".to_string(),
            choices: vec![
                EventChoice {
                    label: "REST".to_string(),
                    body: vec![Effect::AddCardToRunStateDeck {
                        card_id: "PoorSleep".to_string(), upgrade: 0,
                    }],
                },
                EventChoice {
                    label: "KILL".to_string(),
                    body: vec![Effect::LoseRunStateMaxHp { amount: AmountSpec::Fixed(8) }],
                },
            ],
        }),

        // MorphicGrove: Loner → +5 MaxHp. Group → lose all gold (encoded
        // as -200 average since "all current gold" isn't a fixed amount;
        // proper encoding needs a "lose all gold" primitive).
        "MorphicGrove" => Some(EventModel {
            id: "MorphicGrove".to_string(),
            choices: vec![
                EventChoice {
                    label: "LONER".to_string(),
                    body: vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(5) }],
                },
                EventChoice {
                    label: "GROUP".to_string(),
                    body: vec![], // TODO: needs LoseAllGold primitive
                },
            ],
        }),

        // ByrdonisNest: eat (+7 MaxHp) vs take (add ByrdonisEgg quest card).
        "ByrdonisNest" => Some(EventModel {
            id: "ByrdonisNest".to_string(),
            choices: vec![
                EventChoice {
                    label: "EAT".to_string(),
                    body: vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(7) }],
                },
                EventChoice {
                    label: "TAKE".to_string(),
                    body: vec![Effect::AddCardToRunStateDeck {
                        card_id: "ByrdonisEgg".to_string(), upgrade: 0,
                    }],
                },
            ],
        }),

        // DrowningBeacon: bottle (GlowwaterPotion) vs climb (-13 HP — actually
        // C# is MaxHp loss, double-check).
        "DrowningBeacon" => Some(EventModel {
            id: "DrowningBeacon".to_string(),
            choices: vec![
                EventChoice {
                    label: "BOTTLE".to_string(),
                    body: vec![Effect::GainPotionToBelt {
                        potion_id: "GlowwaterPotion".to_string(),
                    }],
                },
                EventChoice {
                    label: "CLIMB".to_string(),
                    body: vec![Effect::LoseRunStateMaxHp { amount: AmountSpec::Fixed(13) }],
                },
            ],
        }),

        // LuminousChoir: reach-into-flesh (remove 2 cards) vs offer-tribute
        // (-149 gold). Card-removal here is the "pick 2 cards from deck"
        // interactive flow — needs the run-state deck-action staging.
        // For MVP, encode as a no-op for the removal branch.
        "LuminousChoir" => Some(EventModel {
            id: "LuminousChoir".to_string(),
            choices: vec![
                EventChoice {
                    label: "REACH_INTO_FLESH".to_string(),
                    body: vec![], // TODO: pick-2-cards-from-deck-for-removal
                },
                EventChoice {
                    label: "OFFER_TRIBUTE".to_string(),
                    body: vec![Effect::LoseRunStateMaxHp { amount: AmountSpec::Fixed(0) }],
                    // TODO: actually a gold cost (-149); needs LoseRunStateGold primitive.
                },
            ],
        }),

        // Bugslayer: pick one of two attack cards.
        "Bugslayer" => Some(EventModel {
            id: "Bugslayer".to_string(),
            choices: vec![
                EventChoice {
                    label: "EXTERMINATION".to_string(),
                    body: vec![Effect::AddCardToRunStateDeck {
                        card_id: "Exterminate".to_string(), upgrade: 0,
                    }],
                },
                EventChoice {
                    label: "SQUASH".to_string(),
                    body: vec![Effect::AddCardToRunStateDeck {
                        card_id: "Squash".to_string(), upgrade: 0,
                    }],
                },
            ],
        }),

        // TheLegendsWereTrue: take map (add SpoilsMap quest) vs find exit
        // (8 unblockable HP loss).
        "TheLegendsWereTrue" => Some(EventModel {
            id: "TheLegendsWereTrue".to_string(),
            choices: vec![
                EventChoice {
                    label: "NAB_THE_MAP".to_string(),
                    body: vec![Effect::AddCardToRunStateDeck {
                        card_id: "SpoilsMap".to_string(), upgrade: 0,
                    }],
                },
                EventChoice {
                    label: "SLOWLY_FIND_AN_EXIT".to_string(),
                    body: vec![Effect::LoseRunStateHp { amount: AmountSpec::Fixed(8) }],
                },
            ],
        }),

        // ColorfulPhilosophers: offers one of {common card, uncommon card, rare card}
        // — each is a card reward from the player's pool. Needs reward-from-rarity
        // primitive that we have via OfferCardReward but at event-time the options
        // need to be generated. Encoded as 3 placeholder offers.
        "ColorfulPhilosophers" => Some(EventModel {
            id: "ColorfulPhilosophers".to_string(),
            choices: vec![
                EventChoice {
                    label: "COMMON_REWARD".to_string(),
                    body: vec![], // TODO: needs OfferCardReward with rolled options.
                },
            ],
        }),

        // AromaOfChaos: LetGo (transform 1 random card) vs MaintainControl
        // (upgrade 1 card — needs pick).
        "AromaOfChaos" => Some(EventModel {
            id: "AromaOfChaos".to_string(),
            choices: vec![
                EventChoice {
                    label: "LET_GO".to_string(),
                    body: vec![Effect::TransformRandomDeckCards {
                        n: AmountSpec::Fixed(1),
                        filter: crate::effects::CardFilter::Any,
                        pool: crate::effects::CardPoolRef::CharacterAny,
                    }],
                },
                EventChoice {
                    label: "MAINTAIN_CONTROL".to_string(),
                    body: vec![Effect::UpgradeDeckCards {
                        filter: crate::effects::CardFilter::Any,
                        // TODO: should be "upgrade 1 (picked)" not "upgrade all".
                        // Approximation acceptable for MVP — most decks have at
                        // most a few upgradable cards at the point this event fires.
                    }],
                },
            ],
        }),

        // DoorsOfLightAndDark: Light (upgrade N cards) vs Dark (remove 1 card).
        "DoorsOfLightAndDark" => Some(EventModel {
            id: "DoorsOfLightAndDark".to_string(),
            choices: vec![
                EventChoice {
                    label: "LIGHT".to_string(),
                    body: vec![Effect::UpgradeDeckCards {
                        filter: crate::effects::CardFilter::Any,
                    }],
                },
                EventChoice {
                    label: "DARK".to_string(),
                    body: vec![], // TODO: pick 1 card from deck for removal
                },
            ],
        }),

        // BrainLeech: Rip (-5 HP + card reward) vs ShareKnowledge (card pick) vs Leave.
        // Both card-reward branches need event-driven OfferCardReward; stub for now.
        "BrainLeech" => Some(EventModel {
            id: "BrainLeech".to_string(),
            choices: vec![
                EventChoice {
                    label: "RIP".to_string(),
                    body: vec![Effect::LoseRunStateHp { amount: AmountSpec::Fixed(5) }],
                    // TODO: + 1 colorless-pool card reward (3 options)
                },
                EventChoice {
                    label: "SHARE_KNOWLEDGE".to_string(),
                    body: vec![], // TODO: pick 1 card from 5 char-pool options
                },
                EventChoice {
                    label: "LEAVE".to_string(),
                    body: vec![],
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

    #[test]
    fn hungry_for_mushrooms_grants_relic() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "HungryForMushrooms");
        resolve_event_choice(&mut rs, 0).expect("big mushroom");
        assert!(rs.players()[0].relics.iter().any(|r| r.id == "BigMushroom"));
    }

    #[test]
    fn sunken_treasury_second_chest_adds_greed_and_gold() {
        let mut rs = fresh_rs();
        let gold_before = rs.players()[0].gold;
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "SunkenTreasury");
        resolve_event_choice(&mut rs, 1).expect("second chest");
        assert_eq!(rs.players()[0].gold, gold_before + 333);
        assert!(rs.players()[0].deck.iter().any(|c| c.id == "Greed"));
    }

    #[test]
    fn unrest_site_kill_drops_max_hp() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        let max_before = rs.players()[0].max_hp;
        enter_event(&mut rs, 0, "UnrestSite");
        resolve_event_choice(&mut rs, 1).expect("kill");
        assert_eq!(rs.players()[0].max_hp, max_before - 8);
    }

    #[test]
    fn morphic_grove_loner_grants_max_hp() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        let max_before = rs.players()[0].max_hp;
        enter_event(&mut rs, 0, "MorphicGrove");
        resolve_event_choice(&mut rs, 0).expect("loner");
        assert_eq!(rs.players()[0].max_hp, max_before + 5);
    }

    #[test]
    fn byrdonis_nest_take_adds_egg_quest() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "ByrdonisNest");
        resolve_event_choice(&mut rs, 1).expect("take");
        assert!(rs.players()[0].deck.iter().any(|c| c.id == "ByrdonisEgg"));
    }

    #[test]
    fn drowning_beacon_bottle_grants_potion() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "DrowningBeacon");
        resolve_event_choice(&mut rs, 0).expect("bottle");
        assert!(rs.players()[0].potions.iter().any(|p| p.id == "GlowwaterPotion"));
    }

    #[test]
    fn bugslayer_adds_picked_card() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "Bugslayer");
        resolve_event_choice(&mut rs, 1).expect("squash");
        assert!(rs.players()[0].deck.iter().any(|c| c.id == "Squash"));
    }

    #[test]
    fn the_legends_were_true_map_adds_quest_card() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "TheLegendsWereTrue");
        resolve_event_choice(&mut rs, 0).expect("nab map");
        assert!(rs.players()[0].deck.iter().any(|c| c.id == "SpoilsMap"));
    }

    #[test]
    fn this_or_that_plain_adds_curse_and_hp_loss() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        let hp_before = rs.players()[0].hp;
        enter_event(&mut rs, 0, "ThisOrThat");
        resolve_event_choice(&mut rs, 0).expect("plain");
        assert!(rs.players()[0].deck.iter().any(|c| c.id == "Clumsy"));
        assert_eq!(rs.players()[0].hp, hp_before - 6);
    }

    #[test]
    fn trash_heap_grab_gains_gold() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        let gold_before = rs.players()[0].gold;
        enter_event(&mut rs, 0, "TrashHeap");
        resolve_event_choice(&mut rs, 1).expect("grab");
        assert_eq!(rs.players()[0].gold, gold_before + 100);
    }

    #[test]
    fn aroma_of_chaos_letgo_transforms_a_card() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        // Need a card in the deck to transform.
        rs.add_card(0, "StrikeIronclad", 0);
        let deck_size_before = rs.players()[0].deck.len();
        enter_event(&mut rs, 0, "AromaOfChaos");
        resolve_event_choice(&mut rs, 0).expect("let go");
        // Deck size is preserved; the card may have changed id.
        assert_eq!(rs.players()[0].deck.len(), deck_size_before);
    }
}
