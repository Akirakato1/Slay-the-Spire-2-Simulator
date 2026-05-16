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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct EventChoice {
    /// Short identifier (matches the C# enum-like option keys, e.g.
    /// "CLAIM", "SEARCH", "CONFRONT", "ACCEPT"). Used for replay /
    /// feature extraction; not displayed in-engine.
    pub label: String,
    /// Effects fired in order when this choice is resolved.
    /// Multi-page events end their body with an `Effect::SetEventChoices`
    /// to transition to a sub-menu instead of finishing the event.
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
/// commit. Multi-page events keep `pending_event` alive across
/// choices: the body of a transition choice ends with
/// `Effect::SetEventChoices` which mutates `choices` in place.
#[derive(Debug, Clone)]
pub struct PendingEvent {
    pub event_id: String,
    pub player_idx: usize,
    pub choices: Vec<EventChoice>,
}

/// Look up an event's choices. Returns None for unknown ids (caller
/// should treat as "event not implemented yet" — a one-arm-per-event
/// model that mirrors how cards/relics/potions are looked up).
/// Build a single dish's resolution body for EndlessConveyor.
/// Each dish runs a different effect, then transitions to the
/// re-grab page (or terminates if depth cap hit).
fn endless_conveyor_dish(label: &str, body: Vec<Effect>, next_grab_page: Vec<EventChoice>) -> Vec<EventChoice> {
    let mut full = body;
    full.push(Effect::SetEventChoices { choices: next_grab_page });
    vec![EventChoice {
        label: label.to_string(),
        body: full,
    }]
}

/// Build the GRAB-or-LEAVE page after a dish has been consumed.
/// Bounded recursion: depth=0 is the initial page, depth >=3 caps
/// to LEAVE-only so the event terminates in bounded chain length.
fn endless_conveyor_grab_page(depth: i32) -> Vec<EventChoice> {
    if depth >= 3 {
        return vec![EventChoice { label: "LEAVE".to_string(), body: vec![] }];
    }
    let next = endless_conveyor_grab_page(depth + 1);
    let dish_branches: Vec<Vec<EventChoice>> = vec![
        endless_conveyor_dish(
            "CAVIAR",
            vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(4) }],
            next.clone(),
        ),
        endless_conveyor_dish(
            "SPICY_SNAPPY",
            vec![Effect::UpgradeRandomDeckCards {
                n: AmountSpec::Fixed(1),
                filter: crate::effects::CardFilter::Upgradable,
            }],
            next.clone(),
        ),
        endless_conveyor_dish(
            "JELLY_LIVER",
            vec![Effect::TransformRandomDeckCards {
                n: AmountSpec::Fixed(1),
                filter: crate::effects::CardFilter::Any,
                pool: crate::effects::CardPoolRef::CharacterAny,
            }],
            next.clone(),
        ),
        endless_conveyor_dish(
            "FRIED_EEL",
            vec![Effect::OfferCardRewardFromPool {
                pool: crate::effects::CardPoolRef::Colorless,
                count: 1,
                n_min: 1,
                n_max: 1,
                source: Some("EndlessConveyor.FRIED_EEL".to_string()),
            }],
            next.clone(),
        ),
        endless_conveyor_dish(
            "SUSPICIOUS_CONDIMENT",
            // Approximation: skip the per-character potion pool roll;
            // pending potion-pool primitive. For now no-op.
            vec![],
            next.clone(),
        ),
        endless_conveyor_dish(
            "CLAM_ROLL",
            vec![Effect::HealRunState { amount: AmountSpec::Fixed(10) }],
            next.clone(),
        ),
        endless_conveyor_dish(
            "GOLDEN_FYSH",
            vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(75) }],
            next.clone(),
        ),
        endless_conveyor_dish(
            "SEAPUNK_SALAD",
            vec![Effect::AddCardToRunStateDeck {
                card_id: "FeedingFrenzy".to_string(),
                upgrade: 0,
            }],
            next,
        ),
    ];
    vec![
        EventChoice {
            label: "GRAB".to_string(),
            body: vec![
                Effect::LoseRunStateGold { amount: AmountSpec::Fixed(40) },
                Effect::RngBranchedSetEventChoices { branches: dish_branches },
            ],
        },
        EventChoice { label: "LEAVE".to_string(), body: vec![] },
    ]
}

/// Build the HOLD_ON page for SlipperyBridge at the given damage
/// level. C# loops HOLD_ON indefinitely but renames the page suffix
/// after the 7th. We bound the chain at damage = 9 (after 7 hold-ons:
/// 3, 4, 5, 6, 7, 8, 9 then exits). Damage is unblockable HP loss.
fn build_slippery_bridge_hold_on_page(damage: i32) -> Vec<EventChoice> {
    let body = if damage >= 10 {
        // Terminal hold-on: lose damage HP, then chain exits (no
        // further HOLD_ON option).
        vec![
            Effect::LoseRunStateHp { amount: AmountSpec::Fixed(damage) },
            // Remove 1 random non-curse card from the master deck
            // (the random card was rerolled in C#; we approximate by
            // removing one matching Any filter).
            Effect::RemoveRandomDeckCards {
                n: AmountSpec::Fixed(1),
                filter: crate::effects::CardFilter::Any,
            },
        ]
    } else {
        vec![
            Effect::LoseRunStateHp { amount: AmountSpec::Fixed(damage) },
            Effect::SetEventChoices {
                choices: build_slippery_bridge_hold_on_page(damage + 1),
            },
        ]
    };
    vec![
        EventChoice {
            label: "OVERCOME".to_string(),
            body: vec![Effect::RemoveRandomDeckCards {
                n: AmountSpec::Fixed(1),
                filter: crate::effects::CardFilter::Any,
            }],
        },
        EventChoice { label: "HOLD_ON".to_string(), body },
    ]
}

/// Build the LINGER page for AbyssalBaths at the given damage level.
/// C# loops indefinitely; we bound at damage=11 (after 8 lingers from
/// the initial IMMERSE which started at 3). Beyond that, LINGER stops
/// being offered — practical play hits this so rarely the divergence
/// is acceptable for the MVP.
fn build_abyssal_baths_linger_page(damage: i32) -> Vec<EventChoice> {
    // Each LINGER click: +2 maxHP, lose `damage` HP, damage +=1, page
    // continues. Cap at damage = 12 (after which LINGER terminates).
    let body = if damage >= 12 {
        // Terminal LINGER: +2 maxHP, lose damage HP. No further chain.
        vec![
            Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(2) },
            Effect::LoseRunStateHp { amount: AmountSpec::Fixed(damage) },
        ]
    } else {
        vec![
            Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(2) },
            Effect::LoseRunStateHp { amount: AmountSpec::Fixed(damage) },
            Effect::SetEventChoices {
                choices: build_abyssal_baths_linger_page(damage + 1),
            },
        ]
    };
    vec![
        EventChoice { label: "LINGER".to_string(), body },
        EventChoice { label: "EXIT_BATHS".to_string(), body: vec![] },
    ]
}

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
                    body: vec![Effect::GainRandomRelicFromPool {
                        pool: crate::effects::RelicPoolRef::CharacterPool,
                        count: 1,
                    }],
                },
            ],
        }),

        // TrashHeap: dive (-8 HP + random relic) vs grab (+100 gold).
        // C# picks from a curated event-specific relic list; we use
        // the player's character pool as a close approximation.
        "TrashHeap" => Some(EventModel {
            id: "TrashHeap".to_string(),
            choices: vec![
                EventChoice {
                    label: "DIVE_IN".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(8) },
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 1,
                        },
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

        // MorphicGrove: Loner → +5 MaxHp. Group → drops all gold.
        "MorphicGrove" => Some(EventModel {
            id: "MorphicGrove".to_string(),
            choices: vec![
                EventChoice {
                    label: "LONER".to_string(),
                    body: vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(5) }],
                },
                EventChoice {
                    label: "GROUP".to_string(),
                    body: vec![Effect::LoseAllGold],
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

        // LuminousChoir: REACH_INTO_FLESH picks 2 cards for removal.
        // OFFER_TRIBUTE: spend 149 gold + grant 1 random relic.
        "LuminousChoir" => Some(EventModel {
            id: "LuminousChoir".to_string(),
            choices: vec![
                EventChoice {
                    label: "REACH_INTO_FLESH".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Remove,
                        filter: crate::effects::CardFilter::Any,
                        n_min: 2,
                        n_max: 2,
                        source: "LuminousChoir.REACH_INTO_FLESH".to_string(),
                    }],
                },
                EventChoice {
                    label: "OFFER_TRIBUTE".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(149) },
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 1,
                        },
                    ],
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

        // ColorfulPhilosophers: a single COMMON_REWARD choice that
        // rolls 3 character-pool cards (Normal-style rarity weights).
        // C# has separate per-rarity options gated on player progress;
        // we collapse to one rolled-options offer.
        "ColorfulPhilosophers" => Some(EventModel {
            id: "ColorfulPhilosophers".to_string(),
            choices: vec![
                EventChoice {
                    label: "COMMON_REWARD".to_string(),
                    body: vec![Effect::OfferCardRewardFromPool {
                        pool: crate::effects::CardPoolRef::CharacterAny,
                        count: 3,
                        n_min: 0,
                        n_max: 1,
                        source: Some("ColorfulPhilosophers.COMMON_REWARD".to_string()),
                    }],
                },
            ],
        }),

        // AromaOfChaos: LetGo (transform 1 random card) vs MaintainControl
        // (pick 1 card to upgrade).
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
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Upgrade,
                        filter: crate::effects::CardFilter::Upgradable,
                        n_min: 1,
                        n_max: 1,
                        source: "AromaOfChaos.MAINTAIN_CONTROL".to_string(),
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
                    body: vec![Effect::UpgradeRandomDeckCards {
                        n: AmountSpec::Fixed(2),
                        filter: crate::effects::CardFilter::Upgradable,
                    }],
                },
                EventChoice {
                    label: "DARK".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Remove,
                        filter: crate::effects::CardFilter::Any,
                        n_min: 1,
                        n_max: 1,
                        source: "DoorsOfLightAndDark.DARK".to_string(),
                    }],
                },
            ],
        }),

        // BrainLeech: Rip (-5 HP + 3-Colorless card reward) vs
        // ShareKnowledge (pick 1 from 5 char-pool options). No Leave —
        // C# only exposes the 2 options.
        "BrainLeech" => Some(EventModel {
            id: "BrainLeech".to_string(),
            choices: vec![
                EventChoice {
                    label: "SHARE_KNOWLEDGE".to_string(),
                    body: vec![Effect::OfferCardRewardFromPool {
                        pool: crate::effects::CardPoolRef::CharacterAny,
                        count: 5,
                        n_min: 1,  // C#: cancellable=false
                        n_max: 1,
                        source: Some("BrainLeech.SHARE_KNOWLEDGE".to_string()),
                    }],
                },
                EventChoice {
                    label: "RIP".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(5) },
                        Effect::OfferCardRewardFromPool {
                            pool: crate::effects::CardPoolRef::Colorless,
                            count: 3,
                            n_min: 0,  // post-combat-style: skip allowed
                            n_max: 1,
                            source: Some("BrainLeech.RIP".to_string()),
                        },
                    ],
                },
            ],
        }),

        // SpiritGrafter: LetItIn → heal 25 + add Metamorphosis card.
        // Rejection → -10 HP + upgrade 1 picked card (pick is stub).
        "SpiritGrafter" => Some(EventModel {
            id: "SpiritGrafter".to_string(),
            choices: vec![
                EventChoice {
                    label: "LET_IT_IN".to_string(),
                    body: vec![
                        Effect::HealRunState { amount: AmountSpec::Fixed(25) },
                        Effect::AddCardToRunStateDeck {
                            card_id: "Metamorphosis".to_string(), upgrade: 0,
                        },
                    ],
                },
                EventChoice {
                    label: "REJECTION".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(10) },
                        Effect::StageDeckPick {
                            kind: crate::run_state::DeckActionKind::Upgrade,
                            filter: crate::effects::CardFilter::Upgradable,
                            n_min: 1,
                            n_max: 1,
                            source: "SpiritGrafter.REJECTION".to_string(),
                        },
                    ],
                },
            ],
        }),

        // TabletOfTruth: 5-step Decipher chain (loses escalating max HP
        // per step + upgrades a random card; final step upgrades all)
        // vs Smash (heal 20). C# order: [DECIPHER_1, SMASH] initial.
        // Decipher cost schedule (LoseMaxHpAndUpgrade arg per step):
        //   Decipher 1 → lose 3 max HP, +1 random upgrade. Next cost 6.
        //   Decipher 2 → lose 6 max HP, +1 random upgrade. Next cost 12.
        //   Decipher 3 → lose 12 max HP, +1 random upgrade. Next cost 24.
        //   Decipher 4 → lose 24 max HP, +1 random upgrade. Next cost ≈ all.
        //   Decipher 5 → lose (max_hp-1) max HP, upgrade ALL upgradable.
        // Mid-chain choices: [DECIPHER_N, GIVE_UP]. Encoded via
        // SetEventChoices side-channel (Phase 4 multi-page surface).
        "TabletOfTruth" => Some(EventModel {
            id: "TabletOfTruth".to_string(),
            choices: vec![
                EventChoice {
                    label: "DECIPHER_1".to_string(),
                    body: vec![
                        Effect::LoseRunStateMaxHp { amount: AmountSpec::Fixed(3) },
                        Effect::UpgradeRandomDeckCards {
                            n: AmountSpec::Fixed(1),
                            filter: crate::effects::CardFilter::Upgradable,
                        },
                        Effect::SetEventChoices {
                            choices: vec![
                                EventChoice {
                                    label: "DECIPHER_2".to_string(),
                                    body: vec![
                                        Effect::LoseRunStateMaxHp { amount: AmountSpec::Fixed(6) },
                                        Effect::UpgradeRandomDeckCards {
                                            n: AmountSpec::Fixed(1),
                                            filter: crate::effects::CardFilter::Upgradable,
                                        },
                                        Effect::SetEventChoices {
                                            choices: vec![
                                                EventChoice {
                                                    label: "DECIPHER_3".to_string(),
                                                    body: vec![
                                                        Effect::LoseRunStateMaxHp { amount: AmountSpec::Fixed(12) },
                                                        Effect::UpgradeRandomDeckCards {
                                                            n: AmountSpec::Fixed(1),
                                                            filter: crate::effects::CardFilter::Upgradable,
                                                        },
                                                        Effect::SetEventChoices {
                                                            choices: vec![
                                                                EventChoice {
                                                                    label: "DECIPHER_4".to_string(),
                                                                    body: vec![
                                                                        Effect::LoseRunStateMaxHp { amount: AmountSpec::Fixed(24) },
                                                                        Effect::UpgradeRandomDeckCards {
                                                                            n: AmountSpec::Fixed(1),
                                                                            filter: crate::effects::CardFilter::Upgradable,
                                                                        },
                                                                        Effect::SetEventChoices {
                                                                            choices: vec![
                                                                                EventChoice {
                                                                                    label: "DECIPHER_5".to_string(),
                                                                                    // Final tier: lose (max_hp-1), upgrade ALL.
                                                                                    // The exact "max_hp-1" amount isn't directly
                                                                                    // expressible without an AmountSpec for
                                                                                    // OwnerMaxHp-arithmetic on run state; use a
                                                                                    // large fixed value to mirror "near-death"
                                                                                    // and let LoseRunStateMaxHp clamp at 0.
                                                                                    body: vec![
                                                                                        Effect::LoseRunStateMaxHp { amount: AmountSpec::Fixed(9999) },
                                                                                        Effect::UpgradeDeckCards {
                                                                                            filter: crate::effects::CardFilter::Upgradable,
                                                                                        },
                                                                                    ],
                                                                                },
                                                                                EventChoice {
                                                                                    label: "GIVE_UP".to_string(),
                                                                                    body: vec![],
                                                                                },
                                                                            ],
                                                                        },
                                                                    ],
                                                                },
                                                                EventChoice {
                                                                    label: "GIVE_UP".to_string(),
                                                                    body: vec![],
                                                                },
                                                            ],
                                                        },
                                                    ],
                                                },
                                                EventChoice {
                                                    label: "GIVE_UP".to_string(),
                                                    body: vec![],
                                                },
                                            ],
                                        },
                                    ],
                                },
                                EventChoice {
                                    label: "GIVE_UP".to_string(),
                                    body: vec![],
                                },
                            ],
                        },
                    ],
                },
                EventChoice {
                    label: "SMASH".to_string(),
                    body: vec![Effect::HealRunState { amount: AmountSpec::Fixed(20) }],
                },
            ],
        }),

        // WhisperingHollow: Gold (-35 gold + 2 potion rewards) vs Hug
        // (-9 HP + transform 1 picked card).
        "WhisperingHollow" => Some(EventModel {
            id: "WhisperingHollow".to_string(),
            choices: vec![
                EventChoice {
                    label: "GOLD".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(35) },
                        Effect::OfferPotionRewardFromPool {
                            count: 2,
                            n_min: 0,
                            n_max: 2,
                            source: Some("WhisperingHollow.GOLD".to_string()),
                        },
                    ],
                },
                EventChoice {
                    label: "HUG".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(9) },
                        Effect::StageDeckPick {
                            kind: crate::run_state::DeckActionKind::Transform {
                                pool: "CharacterAny".to_string(),
                            },
                            filter: crate::effects::CardFilter::Any,
                            n_min: 1,
                            n_max: 1,
                            source: "WhisperingHollow.HUG".to_string(),
                        },
                    ],
                },
            ],
        }),

        // PotionCourier: Grab (3 FoulPotions to belt) vs Ransack
        // (1 random potion from the player's pool).
        "PotionCourier" => Some(EventModel {
            id: "PotionCourier".to_string(),
            choices: vec![
                EventChoice {
                    label: "GRAB_POTIONS".to_string(),
                    body: vec![
                        Effect::GainPotionToBelt { potion_id: "FoulPotion".to_string() },
                        Effect::GainPotionToBelt { potion_id: "FoulPotion".to_string() },
                        Effect::GainPotionToBelt { potion_id: "FoulPotion".to_string() },
                    ],
                },
                EventChoice {
                    label: "RANSACK".to_string(),
                    body: vec![Effect::OfferPotionRewardFromPool {
                        count: 1,
                        n_min: 1,
                        n_max: 1,
                        source: Some("PotionCourier.RANSACK".to_string()),
                    }],
                },
            ],
        }),

        // RoomFullOfCheese: Gorge (8-card character-pool reward) vs
        // Search (-14 HP unblockable + ChosenCheese relic).
        "RoomFullOfCheese" => Some(EventModel {
            id: "RoomFullOfCheese".to_string(),
            choices: vec![
                EventChoice {
                    label: "GORGE".to_string(),
                    body: vec![Effect::OfferCardRewardFromPool {
                        pool: crate::effects::CardPoolRef::CharacterAny,
                        count: 8,
                        n_min: 0,
                        n_max: 1,
                        source: Some("RoomFullOfCheese.GORGE".to_string()),
                    }],
                },
                EventChoice {
                    label: "SEARCH".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(14) },
                        Effect::GainRelic { relic_id: "ChosenCheese".to_string() },
                    ],
                },
            ],
        }),

        // Wellspring: BATHE adds Guilty curse + stages a remove-1-pick.
        // BOTTLE rolls 1 random potion from the player's pool.
        "Wellspring" => Some(EventModel {
            id: "Wellspring".to_string(),
            choices: vec![
                EventChoice {
                    label: "BATHE".to_string(),
                    body: vec![
                        Effect::AddCardToRunStateDeck {
                            card_id: "Guilty".to_string(), upgrade: 0,
                        },
                        Effect::StageDeckPick {
                            kind: crate::run_state::DeckActionKind::Remove,
                            filter: crate::effects::CardFilter::Any,
                            n_min: 1,
                            n_max: 1,
                            source: "Wellspring.BATHE".to_string(),
                        },
                    ],
                },
                EventChoice {
                    label: "BOTTLE".to_string(),
                    body: vec![Effect::OfferPotionRewardFromPool {
                        count: 1,
                        n_min: 1,
                        n_max: 1,
                        source: Some("Wellspring.BOTTLE".to_string()),
                    }],
                },
            ],
        }),

        // BattlewornDummy + DenseVegetation + PunchOff: combat-in-event.
        // FIGHT enters an event-combat encounter (EnterEventCombat is
        // currently a stub; combat itself isn't simulated yet).
        "BattlewornDummy" | "DenseVegetation" | "PunchOff" => Some(EventModel {
            id: id.to_string(),
            choices: vec![
                EventChoice {
                    label: "FIGHT".to_string(),
                    body: vec![Effect::EnterEventCombat {
                        encounter_id: format!("{}EventEncounter", id),
                    }],
                },
            ],
        }),

        // RelicTrader: pick one of your relics to swap for one of 3 new
        // ones. Encoded as 3 random-relic-from-pool choices where the
        // swap removes a random non-starter relic first.
        "RelicTrader" => Some(EventModel {
            id: "RelicTrader".to_string(),
            choices: vec![
                EventChoice {
                    label: "TOP".to_string(),
                    body: vec![
                        Effect::LoseRandomRelic,
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 1,
                        },
                    ],
                },
                EventChoice {
                    label: "MIDDLE".to_string(),
                    body: vec![
                        Effect::LoseRandomRelic,
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 1,
                        },
                    ],
                },
                EventChoice {
                    label: "BOTTOM".to_string(),
                    body: vec![
                        Effect::LoseRandomRelic,
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 1,
                        },
                    ],
                },
                EventChoice { label: "LEAVE".to_string(), body: vec![] },
            ],
        }),

        // FakeMerchant: deceptive 3-relic shop. C# fires a combat
        // after the relic interaction. Stubbed via EnterEventCombat.
        "FakeMerchant" => Some(EventModel {
            id: "FakeMerchant".to_string(),
            choices: vec![
                EventChoice {
                    label: "INTERACT".to_string(),
                    body: vec![Effect::EnterEventCombat {
                        encounter_id: "FakeMerchantEventEncounter".to_string(),
                    }],
                },
                EventChoice { label: "LEAVE".to_string(), body: vec![] },
            ],
        }),

        // Reflections: TouchAMirror downgrades 2 random upgraded
        // cards, then upgrades 4 random upgradable cards. Shatter
        // clones the entire deck and adds a BadLuck curse.
        "Reflections" => Some(EventModel {
            id: "Reflections".to_string(),
            choices: vec![
                EventChoice {
                    label: "TOUCH_A_MIRROR".to_string(),
                    body: vec![
                        Effect::DowngradeRandomDeckCards {
                            n: AmountSpec::Fixed(2),
                            filter: crate::effects::CardFilter::Any,
                        },
                        Effect::UpgradeRandomDeckCards {
                            n: AmountSpec::Fixed(4),
                            filter: crate::effects::CardFilter::Upgradable,
                        },
                    ],
                },
                EventChoice {
                    label: "SHATTER".to_string(),
                    body: vec![
                        Effect::CloneDeck,
                        Effect::AddCardToRunStateDeck {
                            card_id: "BadLuck".to_string(),
                            upgrade: 0,
                        },
                    ],
                },
            ],
        }),

        // SpiralingWhirlpool: ObserveSpiral (enchant 1 picked card
        // with Spiral — needs interactive enchant primitive, stub).
        // Drink heals 1/3 of max HP (the C# HealVar(0) is the base;
        // CalculateVars sets it to floor(MaxHp / 3)).
        "SpiralingWhirlpool" => Some(EventModel {
            id: "SpiralingWhirlpool".to_string(),
            choices: vec![
                EventChoice {
                    label: "OBSERVE_THE_SPIRAL".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Enchant {
                            enchantment_id: "Spiral".to_string(),
                            amount: 0,
                        },
                        filter: crate::effects::CardFilter::Any,
                        n_min: 1,
                        n_max: 1,
                        source: "SpiralingWhirlpool.OBSERVE_THE_SPIRAL".to_string(),
                    }],
                },
                EventChoice {
                    label: "DRINK".to_string(),
                    body: vec![Effect::HealRunStateMaxHpFraction {
                        numerator: 1,
                        denominator: 3,
                    }],
                },
            ],
        }),

        // Symbiote: Approach (enchant 1 with Corrupted) vs KillWithFire
        // (transform N cards). Both need primitives we haven't surfaced.
        "Symbiote" => Some(EventModel {
            id: "Symbiote".to_string(),
            choices: vec![
                EventChoice { label: "APPROACH".to_string(),    body: vec![] },
                EventChoice { label: "KILL_WITH_FIRE".to_string(), body: vec![] },
                EventChoice { label: "LEAVE".to_string(),         body: vec![] },
            ],
        }),

        // InfestedAutomaton: Study (Power card reward) vs TouchCore
        // (0-cost card reward) — both card-reward stubs.
        "InfestedAutomaton" => Some(EventModel {
            id: "InfestedAutomaton".to_string(),
            choices: vec![
                EventChoice { label: "STUDY".to_string(),     body: vec![] },
                EventChoice { label: "TOUCH_CORE".to_string(), body: vec![] },
                EventChoice { label: "LEAVE".to_string(),      body: vec![] },
            ],
        }),

        // FieldOfManSizedHoles: Resist (remove N cards) vs EnterYourHole
        // (enchant PerfectFit) vs Leave. Both options need primitives.
        // FieldOfManSizedHoles: RESIST removes 2 picked cards + adds
        // Normality curse. ENTER_YOUR_HOLE enchants 1 picked card with
        // PerfectFit(1). Optional LEAVE.
        "FieldOfManSizedHoles" => Some(EventModel {
            id: "FieldOfManSizedHoles".to_string(),
            choices: vec![
                EventChoice {
                    label: "RESIST".to_string(),
                    body: vec![
                        Effect::StageDeckPick {
                            kind: crate::run_state::DeckActionKind::Remove,
                            filter: crate::effects::CardFilter::Any,
                            n_min: 2,
                            n_max: 2,
                            source: "FieldOfManSizedHoles.RESIST".to_string(),
                        },
                        Effect::AddCardToRunStateDeck {
                            card_id: "Normality".to_string(),
                            upgrade: 0,
                        },
                    ],
                },
                EventChoice {
                    label: "ENTER_YOUR_HOLE".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Enchant {
                            enchantment_id: "PerfectFit".to_string(),
                            amount: 1,
                        },
                        filter: crate::effects::CardFilter::Any,
                        n_min: 1,
                        n_max: 1,
                        source: "FieldOfManSizedHoles.ENTER_YOUR_HOLE".to_string(),
                    }],
                },
                EventChoice { label: "LEAVE".to_string(), body: vec![] },
            ],
        }),

        // TeaMaster: 3 tea relics with different gold costs.
        "TeaMaster" => Some(EventModel {
            id: "TeaMaster".to_string(),
            choices: vec![
                EventChoice {
                    label: "BONE_TEA".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(50) },
                        Effect::GainRelic { relic_id: "BoneTea".to_string() },
                    ],
                },
                EventChoice {
                    label: "EMBER_TEA".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(150) },
                        Effect::GainRelic { relic_id: "EmberTea".to_string() },
                    ],
                },
                EventChoice {
                    label: "TEA_OF_DISCOURTESY".to_string(),
                    body: vec![Effect::GainRelic {
                        relic_id: "TeaOfDiscourtesy".to_string(),
                    }],
                },
            ],
        }),

        // TheLanternKey: ReturnTheKey (+100 gold) vs KeepTheKey (combat skeleton).
        "TheLanternKey" => Some(EventModel {
            id: "TheLanternKey".to_string(),
            choices: vec![
                EventChoice {
                    label: "RETURN_THE_KEY".to_string(),
                    body: vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(100) }],
                },
                EventChoice {
                    label: "KEEP_THE_KEY".to_string(),
                    body: vec![
                        Effect::AddCardToRunStateDeck {
                            card_id: "LanternKey".to_string(),
                            upgrade: 0,
                        },
                        Effect::EnterEventCombat {
                            encounter_id: "TheLanternKeyEventEncounter".to_string(),
                        },
                    ],
                },
            ],
        }),

        // CrystalSphere: UncoverFuture (-50 gold + minigame stub) vs
        // PaymentPlan (Debt curse + minigame stub).
        "CrystalSphere" => Some(EventModel {
            id: "CrystalSphere".to_string(),
            choices: vec![
                EventChoice {
                    label: "UNCOVER_FUTURE".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(50) },
                        Effect::OfferCardRewardFromPool {
                            pool: crate::effects::CardPoolRef::CharacterAny,
                            count: 3,
                            n_min: 0,
                            n_max: 1,
                            source: Some("CrystalSphere.UNCOVER_FUTURE".to_string()),
                        },
                    ],
                },
                EventChoice {
                    label: "PAYMENT_PLAN".to_string(),
                    body: vec![
                        Effect::AddCardToRunStateDeck {
                            card_id: "Debt".to_string(), upgrade: 0,
                        },
                        Effect::OfferCardRewardFromPool {
                            pool: crate::effects::CardPoolRef::CharacterAny,
                            count: 6,
                            n_min: 0,
                            n_max: 1,
                            source: Some("CrystalSphere.PAYMENT_PLAN".to_string()),
                        },
                    ],
                },
            ],
        }),

        // JungleMazeAdventure: DontNeedHelp (-18 HP, +150 gold) vs
        // SafetyInNumbers (+50 gold).
        "JungleMazeAdventure" => Some(EventModel {
            id: "JungleMazeAdventure".to_string(),
            choices: vec![
                EventChoice {
                    label: "DONT_NEED_HELP".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(18) },
                        Effect::GainRunStateGold { amount: AmountSpec::Fixed(150) },
                    ],
                },
                EventChoice {
                    label: "SAFETY_IN_NUMBERS".to_string(),
                    body: vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(50) }],
                },
            ],
        }),

        // DollRoom: 3 options, each grants 1 doll (random relic).
        //   CHOOSE_RANDOM   → 1 random doll (free)
        //   TAKE_SOME_TIME  → -5 HP, then pick from 2 dolls
        //   EXAMINE         → -15 HP, then pick from all 5 dolls
        // C# narrows the random pool to 5 specific dolls and lets the
        // player pick after the HP cost. Our approximation grants 1
        // random character-pool relic for each — same expected value,
        // loses the "curated picks" UI affordance.
        "DollRoom" => Some(EventModel {
            id: "DollRoom".to_string(),
            choices: vec![
                EventChoice {
                    label: "CHOOSE_RANDOM".to_string(),
                    body: vec![Effect::GainRandomRelicFromPool {
                        pool: crate::effects::RelicPoolRef::CharacterPool,
                        count: 1,
                    }],
                },
                EventChoice {
                    label: "TAKE_SOME_TIME".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(5) },
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 1,
                        },
                    ],
                },
                EventChoice {
                    label: "EXAMINE".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(15) },
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 1,
                        },
                    ],
                },
            ],
        }),

        // WelcomeToWongos: 3 shop-style purchases (relic for gold).
        // The "next-relic-from-front" picks are stubs (needs relic pool).
        "WelcomeToWongos" => Some(EventModel {
            id: "WelcomeToWongos".to_string(),
            choices: vec![
                EventChoice {
                    label: "BUY_BARGAIN_BIN".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(100) },
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 1,
                        },
                    ],
                },
                EventChoice {
                    label: "BUY_MYSTERY_BOX".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(300) },
                        Effect::GainRelic { relic_id: "WongosMysteryTicket".to_string() },
                    ],
                },
                EventChoice {
                    label: "BUY_FEATURED_ITEM".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(200) },
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 1,
                        },
                    ],
                },
                EventChoice { label: "LEAVE".to_string(), body: vec![] },
            ],
        }),

        // ZenWeaver: 3 graduated card-transform/remove options at gold cost.
        "ZenWeaver" => Some(EventModel {
            id: "ZenWeaver".to_string(),
            choices: vec![
                EventChoice {
                    label: "BREATHING_TECHNIQUES".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(50) },
                        Effect::StageDeckPick {
                            kind: crate::run_state::DeckActionKind::Transform {
                                pool: "CharacterAny".to_string(),
                            },
                            filter: crate::effects::CardFilter::Any,
                            n_min: 2,
                            n_max: 2,
                            source: "ZenWeaver.BREATHING_TECHNIQUES".to_string(),
                        },
                    ],
                },
                EventChoice {
                    label: "EMOTIONAL_AWARENESS".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(125) },
                        Effect::StageDeckPick {
                            kind: crate::run_state::DeckActionKind::Remove,
                            filter: crate::effects::CardFilter::Any,
                            n_min: 1,
                            n_max: 1,
                            source: "ZenWeaver.EMOTIONAL_AWARENESS".to_string(),
                        },
                    ],
                },
                EventChoice {
                    label: "ARACHNID_ACUPUNCTURE".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(250) },
                        Effect::StageDeckPick {
                            kind: crate::run_state::DeckActionKind::Remove,
                            filter: crate::effects::CardFilter::Any,
                            n_min: 2,
                            n_max: 2,
                            source: "ZenWeaver.ARACHNID_ACUPUNCTURE".to_string(),
                        },
                    ],
                },
            ],
        }),

        // Amalgamator: combine 2 Strikes (or 2 Defends) into 1 card.
        // Approximation: remove 2 strike/defend cards. The "combined
        // single card" is a unique upgraded variant — encoded as a
        // simple pair-removal since the upgrade-form mapping needs
        // per-card data we don't track yet.
        "Amalgamator" => Some(EventModel {
            id: "Amalgamator".to_string(),
            choices: vec![
                EventChoice {
                    label: "COMBINE_STRIKES".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Remove,
                        filter: crate::effects::CardFilter::TaggedAs("Strike".to_string()),
                        n_min: 2,
                        n_max: 2,
                        source: "Amalgamator.COMBINE_STRIKES".to_string(),
                    }],
                },
                EventChoice {
                    label: "COMBINE_DEFENDS".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Remove,
                        filter: crate::effects::CardFilter::TaggedAs("Defend".to_string()),
                        n_min: 2,
                        n_max: 2,
                        source: "Amalgamator.COMBINE_DEFENDS".to_string(),
                    }],
                },
                EventChoice { label: "LEAVE".to_string(), body: vec![] },
            ],
        }),

        // SapphireSeed: Eat (+9 heal + upgrade 1 picked) vs Plant
        // (enchant 1 picked card with Sown).
        "SapphireSeed" => Some(EventModel {
            id: "SapphireSeed".to_string(),
            choices: vec![
                EventChoice {
                    label: "EAT".to_string(),
                    body: vec![
                        Effect::HealRunState { amount: AmountSpec::Fixed(9) },
                        Effect::StageDeckPick {
                            kind: crate::run_state::DeckActionKind::Upgrade,
                            filter: crate::effects::CardFilter::Upgradable,
                            n_min: 1,
                            n_max: 1,
                            source: "SapphireSeed.EAT".to_string(),
                        },
                    ],
                },
                EventChoice {
                    label: "PLANT".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Enchant {
                            enchantment_id: "Sown".to_string(),
                            amount: 0,
                        },
                        filter: crate::effects::CardFilter::Any,
                        n_min: 1,
                        n_max: 1,
                        source: "SapphireSeed.PLANT".to_string(),
                    }],
                },
            ],
        }),

        // StoneOfAllTime: Drink (+10 MaxHp) / Push (-6 HP + Vigorous(8)
        // enchant on a picked card) / Lift (+10 MaxHp + discard a potion).
        "StoneOfAllTime" => Some(EventModel {
            id: "StoneOfAllTime".to_string(),
            choices: vec![
                EventChoice {
                    label: "DRINK".to_string(),
                    body: vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(10) }],
                },
                EventChoice {
                    label: "PUSH".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(6) },
                        Effect::StageDeckPick {
                            kind: crate::run_state::DeckActionKind::Enchant {
                                enchantment_id: "Vigorous".to_string(),
                                amount: 8,
                            },
                            filter: crate::effects::CardFilter::Any,
                            n_min: 1,
                            n_max: 1,
                            source: "StoneOfAllTime.PUSH".to_string(),
                        },
                    ],
                },
                EventChoice {
                    label: "LIFT".to_string(),
                    body: vec![
                        Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(10) },
                        Effect::DiscardPotion {
                            strategy: crate::effects::PotionDiscardStrategy::Random,
                        },
                    ],
                },
            ],
        }),

        // WoodCarvings: 3 carvings — Bird transforms a Basic card to
        // Peck. Snake enchants any card with Slither(1). Torus
        // transforms a Basic card to ToricToughness.
        "WoodCarvings" => Some(EventModel {
            id: "WoodCarvings".to_string(),
            choices: vec![
                EventChoice {
                    label: "BIRD".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::TransformTo {
                            card_id: "Peck".to_string(),
                        },
                        filter: crate::effects::CardFilter::OfRarity("Basic".to_string()),
                        n_min: 1,
                        n_max: 1,
                        source: "WoodCarvings.BIRD".to_string(),
                    }],
                },
                EventChoice {
                    label: "SNAKE".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Enchant {
                            enchantment_id: "Slither".to_string(),
                            amount: 1,
                        },
                        filter: crate::effects::CardFilter::Any,
                        n_min: 1,
                        n_max: 1,
                        source: "WoodCarvings.SNAKE".to_string(),
                    }],
                },
                EventChoice {
                    label: "TORUS".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::TransformTo {
                            card_id: "ToricToughness".to_string(),
                        },
                        filter: crate::effects::CardFilter::OfRarity("Basic".to_string()),
                        n_min: 1,
                        n_max: 1,
                        source: "WoodCarvings.TORUS".to_string(),
                    }],
                },
            ],
        }),

        // WaterloggedScriptorium: 3 ink-themed enchantment options.
        //   BLOODY_INK     → +6 max HP (free)
        //   TENTACLE_QUILL → spend 55 gold, pick 1 card, enchant Steady(1)
        //   PRICKLY_SPONGE → spend 99 gold, pick 2 cards, enchant Steady(1)
        // C# locks Quill/Sponge when gold insufficient; we offer all
        // three and let LoseRunStateGold + StageDeckPick handle the
        // empty-eligibility path gracefully.
        "WaterloggedScriptorium" => Some(EventModel {
            id: "WaterloggedScriptorium".to_string(),
            choices: vec![
                EventChoice {
                    label: "BLOODY_INK".to_string(),
                    body: vec![Effect::GainRunStateMaxHp {
                        amount: AmountSpec::Fixed(6),
                    }],
                },
                EventChoice {
                    label: "TENTACLE_QUILL".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(55) },
                        Effect::StageDeckPick {
                            kind: crate::run_state::DeckActionKind::Enchant {
                                enchantment_id: "Steady".to_string(),
                                amount: 1,
                            },
                            filter: crate::effects::CardFilter::Any,
                            n_min: 1,
                            n_max: 1,
                            source: "WaterloggedScriptorium.TENTACLE_QUILL".to_string(),
                        },
                    ],
                },
                EventChoice {
                    label: "PRICKLY_SPONGE".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(99) },
                        Effect::StageDeckPick {
                            kind: crate::run_state::DeckActionKind::Enchant {
                                enchantment_id: "Steady".to_string(),
                                amount: 1,
                            },
                            filter: crate::effects::CardFilter::Any,
                            n_min: 2,
                            n_max: 2,
                            source: "WaterloggedScriptorium.PRICKLY_SPONGE".to_string(),
                        },
                    ],
                },
            ],
        }),

        // SelfHelpBook: 3 enchantment options.
        //   READ_THE_BACK   → pick 1 Attack card, enchant Sharp(2)
        //   READ_PASSAGE    → pick 1 Skill  card, enchant Nimble(2)
        //   READ_ENTIRE_BOOK→ pick 1 Power  card, enchant Swift(2)
        // C# locks each option if no eligible card of that type
        // exists (PlayerHasCardsAvailable). We offer all three; the
        // StageDeckPick handler silently no-ops on empty eligibility.
        "SelfHelpBook" => Some(EventModel {
            id: "SelfHelpBook".to_string(),
            choices: vec![
                EventChoice {
                    label: "READ_THE_BACK".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Enchant {
                            enchantment_id: "Sharp".to_string(),
                            amount: 2,
                        },
                        filter: crate::effects::CardFilter::OfType("Attack".to_string()),
                        n_min: 1,
                        n_max: 1,
                        source: "SelfHelpBook.READ_THE_BACK".to_string(),
                    }],
                },
                EventChoice {
                    label: "READ_PASSAGE".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Enchant {
                            enchantment_id: "Nimble".to_string(),
                            amount: 2,
                        },
                        filter: crate::effects::CardFilter::OfType("Skill".to_string()),
                        n_min: 1,
                        n_max: 1,
                        source: "SelfHelpBook.READ_PASSAGE".to_string(),
                    }],
                },
                EventChoice {
                    label: "READ_ENTIRE_BOOK".to_string(),
                    body: vec![Effect::StageDeckPick {
                        kind: crate::run_state::DeckActionKind::Enchant {
                            enchantment_id: "Swift".to_string(),
                            amount: 2,
                        },
                        filter: crate::effects::CardFilter::OfType("Power".to_string()),
                        n_min: 1,
                        n_max: 1,
                        source: "SelfHelpBook.READ_ENTIRE_BOOK".to_string(),
                    }],
                },
            ],
        }),

        // WarHistorianRepy: UNLOCK_CAGE (remove all LanternKey cards,
        // gain HistoryCourse relic) vs UNLOCK_CHEST (remove all
        // LanternKey, offer 2 potion + 2 relic rewards — reward bundle
        // is event-time infra not yet built; encode the LanternKey
        // removal so the deck-state effect lands correctly).
        "WarHistorianRepy" => Some(EventModel {
            id: "WarHistorianRepy".to_string(),
            choices: vec![
                EventChoice {
                    label: "UNLOCK_CAGE".to_string(),
                    body: vec![
                        Effect::RemoveAllCardsOfType {
                            card_id: "LanternKey".to_string(),
                        },
                        Effect::GainRelic {
                            relic_id: "HistoryCourse".to_string(),
                        },
                    ],
                },
                EventChoice {
                    label: "UNLOCK_CHEST".to_string(),
                    body: vec![
                        Effect::RemoveAllCardsOfType {
                            card_id: "LanternKey".to_string(),
                        },
                        // C# offers a custom bundle of 2 potions +
                        // 2 relics in one mixed-reward screen. We
                        // approximate by granting one of each kind
                        // directly: 2 random potions go to the belt
                        // (via offer-with-must-pick), 2 random relics
                        // straight to the relic list.
                        Effect::OfferPotionRewardFromPool {
                            count: 2,
                            n_min: 0,
                            n_max: 2,
                            source: Some("WarHistorianRepy.UNLOCK_CHEST".to_string()),
                        },
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 2,
                        },
                    ],
                },
            ],
        }),

        // Trial: Accept (clean run end / advance) vs Reject (open
        // double-down sub-menu — multi-page). Skeleton until event
        // state machine supports multi-page.
        // Trial: ACCEPT enters a random sub-trial (Merchant/Noble/
        // Nondescript via RngBranchedSetEventChoices). REJECT opens a
        // 2-option sub-menu where DOUBLE_DOWN kills the player.
        // Sub-trial outcomes:
        //   Merchant Guilty   → +Regret + 2 random relics
        //   Merchant Innocent → +Shame  + StageDeckPick(Upgrade, 2)
        //   Noble Guilty      → heal 10
        //   Noble Innocent    → +Regret + 300 gold
        //   Nondescript Guilty → +Doubt + 2 character-pool card rewards
        //   Nondescript Innocent → +Doubt + StageDeckPick(Transform, 2)
        "Trial" => Some(EventModel {
            id: "Trial".to_string(),
            choices: vec![
                EventChoice {
                    label: "ACCEPT".to_string(),
                    body: vec![Effect::RngBranchedSetEventChoices {
                        branches: vec![
                            // Merchant
                            vec![
                                EventChoice {
                                    label: "MERCHANT_GUILTY".to_string(),
                                    body: vec![
                                        Effect::AddCardToRunStateDeck {
                                            card_id: "Regret".to_string(),
                                            upgrade: 0,
                                        },
                                        Effect::GainRandomRelicFromPool {
                                            pool: crate::effects::RelicPoolRef::CharacterPool,
                                            count: 2,
                                        },
                                    ],
                                },
                                EventChoice {
                                    label: "MERCHANT_INNOCENT".to_string(),
                                    body: vec![
                                        Effect::AddCardToRunStateDeck {
                                            card_id: "Shame".to_string(),
                                            upgrade: 0,
                                        },
                                        Effect::StageDeckPick {
                                            kind: crate::run_state::DeckActionKind::Upgrade,
                                            filter: crate::effects::CardFilter::Upgradable,
                                            n_min: 2,
                                            n_max: 2,
                                            source: "Trial.MERCHANT_INNOCENT".to_string(),
                                        },
                                    ],
                                },
                            ],
                            // Noble
                            vec![
                                EventChoice {
                                    label: "NOBLE_GUILTY".to_string(),
                                    body: vec![Effect::HealRunState { amount: AmountSpec::Fixed(10) }],
                                },
                                EventChoice {
                                    label: "NOBLE_INNOCENT".to_string(),
                                    body: vec![
                                        Effect::AddCardToRunStateDeck {
                                            card_id: "Regret".to_string(),
                                            upgrade: 0,
                                        },
                                        Effect::GainRunStateGold { amount: AmountSpec::Fixed(300) },
                                    ],
                                },
                            ],
                            // Nondescript
                            vec![
                                EventChoice {
                                    label: "NONDESCRIPT_GUILTY".to_string(),
                                    body: vec![
                                        Effect::AddCardToRunStateDeck {
                                            card_id: "Doubt".to_string(),
                                            upgrade: 0,
                                        },
                                        // C# offers 2 card-reward bundles in
                                        // sequence; we use a single 3-option
                                        // pool reward as the closest
                                        // primitive — captures the spirit
                                        // (extra card from char pool) if
                                        // not the exact mixed-reward shape.
                                        Effect::OfferCardRewardFromPool {
                                            pool: crate::effects::CardPoolRef::CharacterAny,
                                            count: 3,
                                            n_min: 0,
                                            n_max: 1,
                                            source: Some("Trial.NONDESCRIPT_GUILTY".to_string()),
                                        },
                                    ],
                                },
                                EventChoice {
                                    label: "NONDESCRIPT_INNOCENT".to_string(),
                                    body: vec![
                                        Effect::AddCardToRunStateDeck {
                                            card_id: "Doubt".to_string(),
                                            upgrade: 0,
                                        },
                                        Effect::StageDeckPick {
                                            kind: crate::run_state::DeckActionKind::Transform {
                                                pool: "CharacterAny".to_string(),
                                            },
                                            filter: crate::effects::CardFilter::Any,
                                            n_min: 2,
                                            n_max: 2,
                                            source: "Trial.NONDESCRIPT_INNOCENT".to_string(),
                                        },
                                    ],
                                },
                            ],
                        ],
                    }],
                },
                EventChoice {
                    label: "REJECT".to_string(),
                    body: vec![Effect::SetEventChoices {
                        choices: vec![
                            EventChoice {
                                label: "ACCEPT".to_string(),
                                body: vec![Effect::RngBranchedSetEventChoices {
                                    // Same 3 trials. Inline rather than
                                    // factor out — Rust literal cycle.
                                    branches: vec![
                                        vec![EventChoice {
                                            label: "MERCHANT_GUILTY".to_string(),
                                            body: vec![Effect::AddCardToRunStateDeck { card_id: "Regret".to_string(), upgrade: 0 }],
                                        }],
                                        vec![EventChoice {
                                            label: "NOBLE_GUILTY".to_string(),
                                            body: vec![Effect::HealRunState { amount: AmountSpec::Fixed(10) }],
                                        }],
                                        vec![EventChoice {
                                            label: "NONDESCRIPT_GUILTY".to_string(),
                                            body: vec![Effect::AddCardToRunStateDeck { card_id: "Doubt".to_string(), upgrade: 0 }],
                                        }],
                                    ],
                                }],
                            },
                            EventChoice {
                                label: "DOUBLE_DOWN".to_string(),
                                // Mirrors C# "kills the player" by zeroing
                                // current HP. Run-state HP loss > current HP
                                // clamps at 0.
                                body: vec![Effect::LoseRunStateHp { amount: AmountSpec::Fixed(9999) }],
                            },
                        ],
                    }],
                },
            ],
        }),

        // RoundTeaParty: ENJOY_TEA grants RoyalPoison relic + full heal.
        // PICK_FIGHT → -11 HP, transitions to CONTINUE_FIGHT → -11 HP
        // again + random relic from the character pool.
        "RoundTeaParty" => Some(EventModel {
            id: "RoundTeaParty".to_string(),
            choices: vec![
                EventChoice {
                    label: "ENJOY_TEA".to_string(),
                    body: vec![
                        Effect::GainRelic { relic_id: "RoyalPoison".to_string() },
                        // Approximate "heal to full" with a huge amount;
                        // HealRunState clamps at max_hp.
                        Effect::HealRunState { amount: AmountSpec::Fixed(9999) },
                    ],
                },
                EventChoice {
                    label: "PICK_FIGHT".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(11) },
                        Effect::SetEventChoices {
                            choices: vec![EventChoice {
                                label: "CONTINUE_FIGHT".to_string(),
                                body: vec![
                                    Effect::LoseRunStateHp { amount: AmountSpec::Fixed(11) },
                                    Effect::GainRandomRelicFromPool {
                                        pool: crate::effects::RelicPoolRef::CharacterPool,
                                        count: 1,
                                    },
                                ],
                            }],
                        },
                    ],
                },
            ],
        }),

        // ColossalFlower: 3-level dig. Each REACH_DEEPER deals 5/6/7
        // unblockable damage, then offers either EXTRACT (gold) or
        // continue digging. Final tier (dig 3) offers POLLINOUS_CORE
        // relic in place of "REACH_DEEPER_3".
        // Prize gold:  35 / 75 / 135   (dig 0 / 1 / 2 extract)
        // Dig damage:   5 /  6 /  7    (going to dig 1 / 2 / 3)
        "ColossalFlower" => Some(EventModel {
            id: "ColossalFlower".to_string(),
            choices: vec![
                EventChoice {
                    label: "EXTRACT_CURRENT_PRIZE_1".to_string(),
                    body: vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(35) }],
                },
                EventChoice {
                    label: "REACH_DEEPER_1".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(5) },
                        Effect::SetEventChoices {
                            choices: vec![
                                EventChoice {
                                    label: "EXTRACT_CURRENT_PRIZE_2".to_string(),
                                    body: vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(75) }],
                                },
                                EventChoice {
                                    label: "REACH_DEEPER_2".to_string(),
                                    body: vec![
                                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(6) },
                                        Effect::SetEventChoices {
                                            choices: vec![
                                                EventChoice {
                                                    label: "EXTRACT_INSTEAD".to_string(),
                                                    body: vec![Effect::GainRunStateGold { amount: AmountSpec::Fixed(135) }],
                                                },
                                                EventChoice {
                                                    label: "POLLINOUS_CORE".to_string(),
                                                    body: vec![
                                                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(7) },
                                                        Effect::GainRelic { relic_id: "PollinousCore".to_string() },
                                                    ],
                                                },
                                            ],
                                        },
                                    ],
                                },
                            ],
                        },
                    ],
                },
            ],
        }),

        // AbyssalBaths: IMMERSE (+2 maxHP, take 3 dmg) opens a LINGER
        // loop where each click does +2 maxHP and increasing damage
        // (4, 5, 6, ...). C# loops indefinitely; we encode 8 LINGER
        // levels (covers practical play depth), beyond which the chain
        // exits. ABSTAIN heals 10.
        "AbyssalBaths" => Some(EventModel {
            id: "AbyssalBaths".to_string(),
            choices: vec![
                EventChoice {
                    label: "IMMERSE".to_string(),
                    body: vec![
                        Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(2) },
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(3) },
                        Effect::SetEventChoices {
                            choices: build_abyssal_baths_linger_page(4),
                        },
                    ],
                },
                EventChoice {
                    label: "ABSTAIN".to_string(),
                    body: vec![Effect::HealRunState { amount: AmountSpec::Fixed(10) }],
                },
            ],
        }),

        // SlipperyBridge: OVERCOME removes 1 random card (initial pre-roll
        // re-rolled on each HOLD_ON). HOLD_ON deals 3+N HP unblockable
        // and re-rolls the card. C# loops indefinitely past 7 hold-ons;
        // we cap at damage=10 (7 hold-ons in).
        "SlipperyBridge" => Some(EventModel {
            id: "SlipperyBridge".to_string(),
            choices: build_slippery_bridge_hold_on_page(3),
        }),

        // RanwidTheElder: 3 choices (each trades something for relic(s))
        //   POTION → discard random potion, gain 1 random relic
        //   GOLD   → spend 100 gold, gain 1 random relic
        //   RELIC  → lose random tradeable relic, gain 2 random relics
        "RanwidTheElder" => Some(EventModel {
            id: "RanwidTheElder".to_string(),
            choices: vec![
                EventChoice {
                    label: "POTION".to_string(),
                    body: vec![
                        Effect::DiscardPotion {
                            strategy: crate::effects::PotionDiscardStrategy::Random,
                        },
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 1,
                        },
                    ],
                },
                EventChoice {
                    label: "GOLD".to_string(),
                    body: vec![
                        Effect::LoseRunStateGold { amount: AmountSpec::Fixed(100) },
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 1,
                        },
                    ],
                },
                EventChoice {
                    label: "RELIC".to_string(),
                    body: vec![
                        Effect::LoseRandomRelic,
                        Effect::GainRandomRelicFromPool {
                            pool: crate::effects::RelicPoolRef::CharacterPool,
                            count: 2,
                        },
                    ],
                },
            ],
        }),

        // TinkerTime: 3-stage choice chain producing a MadScience card.
        //   Stage 1: CHOOSE_CARD_TYPE   (single button)
        //   Stage 2: pick ATTACK/SKILL/POWER (C# randomly subsets to 2;
        //            we offer all 3)
        //   Stage 3: pick 1 of 3 riders for the chosen type (again C#
        //            subsets to 2; we offer all 3)
        // Limitation: AddCardToRunStateDeck has no slot for per-card
        // counters (tinker_time_type / tinker_time_rider). The MadScience
        // primitive dispatches via those counters; without them the card
        // plays as a no-op. Adding the counters needs a `props` field on
        // CardRef, tracked separately.
        "TinkerTime" => {
            // Helper to build a rider-choice page.
            fn rider_page(_card_type: i32) -> Vec<EventChoice> {
                // Each rider just adds MadScience; counter wiring TBD.
                let mad = |label: &str| EventChoice {
                    label: label.to_string(),
                    body: vec![Effect::AddCardToRunStateDeck {
                        card_id: "MadScience".to_string(),
                        upgrade: 0,
                    }],
                };
                vec![mad("RIDER_A"), mad("RIDER_B"), mad("RIDER_C")]
            }
            Some(EventModel {
                id: "TinkerTime".to_string(),
                choices: vec![EventChoice {
                    label: "CHOOSE_CARD_TYPE".to_string(),
                    body: vec![Effect::SetEventChoices {
                        choices: vec![
                            EventChoice {
                                label: "ATTACK".to_string(),
                                body: vec![Effect::SetEventChoices {
                                    choices: rider_page(1),
                                }],
                            },
                            EventChoice {
                                label: "SKILL".to_string(),
                                body: vec![Effect::SetEventChoices {
                                    choices: rider_page(2),
                                }],
                            },
                            EventChoice {
                                label: "POWER".to_string(),
                                body: vec![Effect::SetEventChoices {
                                    choices: rider_page(3),
                                }],
                            },
                        ],
                    }],
                }],
            })
        }

        // TheArchitect: a dialogue-driven event-combat encounter.
        // C# routes through Models/Encounters/TheArchitectEventEncounter
        // which spins up a combat with character-specific dialogue.
        // EnterEventCombat is currently a stub; the choice resolves
        // but combat itself isn't simulated. Two C#-faithful options:
        // ACCEPT (enter combat) and REJECT (leave).
        "TheArchitect" => Some(EventModel {
            id: "TheArchitect".to_string(),
            choices: vec![
                EventChoice {
                    label: "ACCEPT".to_string(),
                    body: vec![Effect::EnterEventCombat {
                        encounter_id: "TheArchitectEventEncounter".to_string(),
                    }],
                },
                EventChoice { label: "REJECT".to_string(), body: vec![] },
            ],
        }),

        // EndlessConveyor: GRAB rolls a random dish (weighted in C#;
        // we use uniform RngBranchedSetEventChoices). Each dish has
        // a unique effect, then transitions to a GRAB-or-LEAVE page.
        // Capped at 3 grabs deep so the chain terminates.
        "EndlessConveyor" => Some(EventModel {
            id: "EndlessConveyor".to_string(),
            choices: {
                let mut initial = endless_conveyor_grab_page(0);
                // Replace LEAVE button at index 1 with OBSERVE_CHEF.
                initial[1] = EventChoice {
                    label: "OBSERVE_CHEF".to_string(),
                    body: vec![Effect::UpgradeRandomDeckCards {
                        n: AmountSpec::Fixed(1),
                        filter: crate::effects::CardFilter::Upgradable,
                    }],
                };
                initial
            },
        }),

        // TheFutureOfPotions: trade a random potion for a 3-card
        // reward of matching rarity. C# offers per-potion options
        // (rarity follows potion rarity, type rolled per potion).
        // We approximate with a single "TRADE" button that discards
        // a random potion and offers a 3-card character-pool reward.
        "TheFutureOfPotions" => Some(EventModel {
            id: "TheFutureOfPotions".to_string(),
            choices: vec![EventChoice {
                label: "TRADE".to_string(),
                body: vec![
                    Effect::DiscardPotion {
                        strategy: crate::effects::PotionDiscardStrategy::Random,
                    },
                    Effect::OfferCardRewardFromPool {
                        pool: crate::effects::CardPoolRef::CharacterAny,
                        count: 3,
                        n_min: 0,
                        n_max: 1,
                        source: Some("TheFutureOfPotions.TRADE".to_string()),
                    },
                ],
            }],
        }),

        // Deprecated stub — kept registered so unknown-event errors
        // don't surface, but the body is a no-op.
        "DeprecatedEvent" => Some(EventModel {
            id: id.to_string(),
            choices: vec![EventChoice {
                label: "LEAVE".to_string(),
                body: vec![],
            }],
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
    let event_id = event.event_id.clone();
    // Ensure the side-channel is empty before running the body so
    // we can detect whether `SetEventChoices` fired.
    rs.next_event_choices = None;
    crate::effects::execute_run_state_effects(rs, player_idx, &body);
    // If the body called SetEventChoices, transition to the new
    // page by re-parking pending_event. Otherwise the event ends.
    if let Some(next_choices) = rs.next_event_choices.take() {
        rs.pending_event = Some(PendingEvent {
            event_id,
            player_idx,
            choices: next_choices,
        });
    }
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
    fn every_event_in_data_table_is_registered() {
        // Coverage gate: every event id in events.json (minus
        // DeprecatedEvent if we ever drop it) must have a non-None
        // entry in `event_choices`. New events extracted from the
        // C# decompile would fail this test until they're wired.
        let mut missing: Vec<String> = Vec::new();
        let raw = include_str!("../data/events.json");
        let entries: Vec<serde_json::Value> = serde_json::from_str(raw).unwrap();
        for entry in entries {
            let id = entry["id"].as_str().unwrap_or("").to_string();
            if id.is_empty() { continue; }
            if event_choices(&id).is_none() {
                missing.push(id);
            }
        }
        assert!(missing.is_empty(),
            "Events without registry entries: {:?}", missing);
    }

    #[test]
    fn tea_master_buys_bone_tea_with_gold() {
        let mut rs = fresh_rs();
        rs.player_state_mut(0).unwrap().gold = 100;
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "TeaMaster");
        resolve_event_choice(&mut rs, 0).expect("bone tea");
        assert_eq!(rs.players()[0].gold, 50);
        assert!(rs.players()[0].relics.iter().any(|r| r.id == "BoneTea"));
    }

    #[test]
    fn the_lantern_key_return_grants_gold() {
        let mut rs = fresh_rs();
        let gold = rs.players()[0].gold;
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "TheLanternKey");
        resolve_event_choice(&mut rs, 0).expect("return");
        assert_eq!(rs.players()[0].gold, gold + 100);
    }

    #[test]
    fn welcome_to_wongos_mystery_box_charges_300_grants_ticket() {
        let mut rs = fresh_rs();
        rs.player_state_mut(0).unwrap().gold = 500;
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "WelcomeToWongos");
        resolve_event_choice(&mut rs, 1).expect("mystery box");
        assert_eq!(rs.players()[0].gold, 200);
        assert!(rs.players()[0].relics.iter()
            .any(|r| r.id == "WongosMysteryTicket"));
    }

    #[test]
    fn stone_of_all_time_drink_grants_10_max_hp() {
        let mut rs = fresh_rs();
        let max = rs.players()[0].max_hp;
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "StoneOfAllTime");
        resolve_event_choice(&mut rs, 0).expect("drink");
        assert_eq!(rs.players()[0].max_hp, max + 10);
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
    fn morphic_grove_group_drops_all_gold() {
        let mut rs = fresh_rs();
        rs.player_state_mut(0).unwrap().gold = 250;
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "MorphicGrove");
        resolve_event_choice(&mut rs, 1).expect("group");
        assert_eq!(rs.players()[0].gold, 0);
    }

    #[test]
    fn whispering_hollow_gold_deducts_35() {
        let mut rs = fresh_rs();
        rs.player_state_mut(0).unwrap().gold = 100;
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "WhisperingHollow");
        resolve_event_choice(&mut rs, 0).expect("gold");
        assert_eq!(rs.players()[0].gold, 65);
    }

    #[test]
    fn whispering_hollow_gold_clamps_at_zero() {
        let mut rs = fresh_rs();
        rs.player_state_mut(0).unwrap().gold = 10;
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "WhisperingHollow");
        resolve_event_choice(&mut rs, 0).expect("gold");
        assert_eq!(rs.players()[0].gold, 0,
            "gold loss must clamp at 0 even if amount > current");
    }

    #[test]
    fn tablet_of_truth_smash_heals_20() {
        let mut rs = fresh_rs();
        rs.player_state_mut(0).unwrap().hp = 50; // out of 80
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "TabletOfTruth");
        // SMASH is index 1 (DECIPHER_1 is 0).
        resolve_event_choice(&mut rs, 1).expect("smash");
        assert_eq!(rs.players()[0].hp, 70);
        assert_eq!(rs.players()[0].max_hp, 80,
            "Smash should heal current HP, not raise max");
    }

    #[test]
    fn tablet_of_truth_smash_caps_at_max_hp() {
        let mut rs = fresh_rs();
        rs.player_state_mut(0).unwrap().hp = 75; // out of 80
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "TabletOfTruth");
        resolve_event_choice(&mut rs, 1).expect("smash");
        assert_eq!(rs.players()[0].hp, 80,
            "heal must cap at max_hp");
    }

    #[test]
    fn tablet_of_truth_decipher_chain_transitions_pages() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.add_card(0, "StrikeIronclad", 0);
        rs.add_card(0, "DefendIronclad", 0);
        enter_event(&mut rs, 0, "TabletOfTruth");
        // INITIAL: [DECIPHER_1, SMASH].
        let pending = rs.pending_event.as_ref().expect("event entered");
        assert_eq!(pending.choices.len(), 2);
        assert_eq!(pending.choices[0].label, "DECIPHER_1");
        assert_eq!(pending.choices[1].label, "SMASH");
        // Pick DECIPHER_1: -3 max HP, upgrade 1 random card, page →
        // [DECIPHER_2, GIVE_UP].
        resolve_event_choice(&mut rs, 0).expect("decipher_1");
        assert_eq!(rs.players()[0].max_hp, 77, "lose 3 max HP");
        let pending = rs.pending_event.as_ref()
            .expect("event must still be pending after page transition");
        assert_eq!(pending.choices.len(), 2);
        assert_eq!(pending.choices[0].label, "DECIPHER_2");
        assert_eq!(pending.choices[1].label, "GIVE_UP");
        // Decipher_2: -6 max HP, transitions to Decipher_3.
        resolve_event_choice(&mut rs, 0).expect("decipher_2");
        assert_eq!(rs.players()[0].max_hp, 71);
        let pending = rs.pending_event.as_ref().expect("decipher_3 pending");
        assert_eq!(pending.choices[0].label, "DECIPHER_3");
    }

    #[test]
    fn tablet_of_truth_decipher_then_give_up_ends_event() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.add_card(0, "StrikeIronclad", 0);
        enter_event(&mut rs, 0, "TabletOfTruth");
        // DECIPHER_1 (index 0) opens [DECIPHER_2, GIVE_UP].
        resolve_event_choice(&mut rs, 0).expect("decipher_1");
        // GIVE_UP (index 1) clears pending_event.
        resolve_event_choice(&mut rs, 1).expect("give_up");
        assert!(rs.pending_event.is_none(),
            "GIVE_UP must end the event with no further page transition");
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

    #[test]
    fn reflections_shatter_doubles_deck_and_adds_bad_luck() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        // Stock deck with a few cards.
        rs.add_card(0, "StrikeIronclad", 0);
        rs.add_card(0, "DefendIronclad", 0);
        rs.add_card(0, "Bash", 0);
        let pre_len = rs.players()[0].deck.len();
        assert_eq!(pre_len, 3);
        enter_event(&mut rs, 0, "Reflections");
        // SHATTER is index 1 (TOUCH_A_MIRROR is 0).
        resolve_event_choice(&mut rs, 1).expect("shatter");
        let post_len = rs.players()[0].deck.len();
        // CloneDeck duplicates every card (2x), then BadLuck is added.
        assert_eq!(post_len, pre_len * 2 + 1,
            "Shatter should clone every card and append BadLuck");
        let has_bad_luck = rs.players()[0]
            .deck
            .iter()
            .any(|c| c.id == "BadLuck");
        assert!(has_bad_luck, "Shatter must add BadLuck card to deck");
    }

    #[test]
    fn reflections_touch_a_mirror_upgrades_more_than_downgrades() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        // Stock deck with upgradable cards.
        for _ in 0..6 {
            rs.add_card(0, "StrikeIronclad", 0);
        }
        let pre_len = rs.players()[0].deck.len();
        enter_event(&mut rs, 0, "Reflections");
        // TOUCH_A_MIRROR is index 0.
        resolve_event_choice(&mut rs, 0).expect("touch");
        // Deck size preserved.
        assert_eq!(rs.players()[0].deck.len(), pre_len);
        // Net upgrade count: +4 upgrades, -2 downgrades = +2 from baseline
        // (baseline 0 since StrikeIronclad starts at 0).
        let total_upgrade: i32 = rs.players()[0]
            .deck
            .iter()
            .map(|c| c.current_upgrade_level.unwrap_or(0))
            .sum();
        // Each random pick is independent and may collide, so the exact
        // net is bounded but not deterministic; we assert the loose
        // bound: net is in [+2, +4] (some downgrades may have clamped
        // at 0 since cards start unupgraded).
        assert!((2..=4).contains(&total_upgrade),
            "Net upgrade count was {} (expected 2-4)", total_upgrade);
    }

    #[test]
    fn spiraling_whirlpool_drink_heals_one_third_max_hp() {
        let mut rs = fresh_rs();
        rs.player_state_mut(0).unwrap().max_hp = 90;
        rs.player_state_mut(0).unwrap().hp = 30; // out of 90
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "SpiralingWhirlpool");
        // DRINK is index 1 (OBSERVE_THE_SPIRAL is 0).
        resolve_event_choice(&mut rs, 1).expect("drink");
        // 90 / 3 = 30 heal → 30 + 30 = 60.
        assert_eq!(rs.players()[0].hp, 60);
        assert_eq!(rs.players()[0].max_hp, 90);
    }

    #[test]
    fn aroma_of_chaos_maintain_control_auto_upgrades_first_upgradable() {
        let mut rs = fresh_rs();
        // auto_resolve_offers = true (default).
        rs.add_card(0, "StrikeIronclad", 0); // upgradable, level 0
        rs.add_card(0, "AscendersBane", 0);  // not upgradable (curse)
        enter_event(&mut rs, 0, "AromaOfChaos");
        // Default first choice would be LET_GO (transform). Disable
        // auto-resolve to address MAINTAIN_CONTROL explicitly.
        // First clear the auto-applied LET_GO by re-creating rs.
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.add_card(0, "StrikeIronclad", 0);
        rs.add_card(0, "AscendersBane", 0);
        enter_event(&mut rs, 0, "AromaOfChaos");
        // MAINTAIN_CONTROL is index 1.
        resolve_event_choice(&mut rs, 1).expect("maintain control");
        // The StageDeckPick should park a pending deck action.
        assert!(rs.pending_deck_action.is_some(),
            "MAINTAIN_CONTROL must stage a deck-pick for RL agent");
        let pending = rs.pending_deck_action.as_ref().unwrap();
        // Only StrikeIronclad is eligible (upgradable).
        assert_eq!(pending.eligible_indices.len(), 1);
    }

    #[test]
    fn stage_deck_pick_auto_resolves_first_eligible() {
        let mut rs = fresh_rs();
        // auto_resolve_offers = true (default).
        rs.add_card(0, "StrikeIronclad", 0);
        rs.add_card(0, "DefendIronclad", 0);
        enter_event(&mut rs, 0, "AromaOfChaos");
        // First choice (LET_GO) was auto-applied: 1 card transformed.
        // No pending action since auto-resolve.
        assert!(rs.pending_deck_action.is_none());
    }

    #[test]
    fn brain_leech_rip_loses_hp_and_offers_3_colorless_cards() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.player_state_mut(0).unwrap().hp = 50;
        enter_event(&mut rs, 0, "BrainLeech");
        // RIP is index 1 (SHARE_KNOWLEDGE is 0).
        resolve_event_choice(&mut rs, 1).expect("rip");
        // HP dropped by 5.
        assert_eq!(rs.players()[0].hp, 45);
        // Card-reward offer staged.
        let pending = rs.pending_offer.as_ref()
            .expect("RIP must stage a colorless card reward");
        assert_eq!(pending.options.len(), 3,
            "C# default: 3 colorless options");
        // Every option must be a Colorless-pool card.
        for opt in &pending.options {
            let data = crate::card::by_id(opt).expect("valid card id");
            assert_eq!(data.pool, "Colorless",
                "Card {} should be Colorless, got pool={}", opt, data.pool);
        }
    }

    #[test]
    fn brain_leech_share_knowledge_offers_5_character_cards() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "BrainLeech");
        // SHARE_KNOWLEDGE is index 0.
        resolve_event_choice(&mut rs, 0).expect("share knowledge");
        let pending = rs.pending_offer.as_ref()
            .expect("SHARE_KNOWLEDGE must stage a 5-card character pool offer");
        assert_eq!(pending.options.len(), 5);
        for opt in &pending.options {
            let data = crate::card::by_id(opt).expect("valid card id");
            assert_eq!(data.pool, "Ironclad",
                "Card {} should be Ironclad, got {}", opt, data.pool);
        }
    }

    #[test]
    fn doors_of_light_and_dark_dark_stages_remove_pick() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.add_card(0, "StrikeIronclad", 0);
        rs.add_card(0, "DefendIronclad", 0);
        rs.add_card(0, "AscendersBane", 0); // curse — should be filtered out for remove? No, CardFilter::Any allows it
        enter_event(&mut rs, 0, "DoorsOfLightAndDark");
        // DARK is index 1.
        resolve_event_choice(&mut rs, 1).expect("dark");
        let pending = rs.pending_deck_action.as_ref()
            .expect("DARK must stage a deck-pick");
        assert_eq!(pending.action, crate::run_state::DeckActionKind::Remove);
        // CardFilter::Any allows all 3.
        assert_eq!(pending.eligible_indices.len(), 3);
    }

    #[test]
    fn war_historian_repy_unlock_cage_removes_lantern_keys_and_grants_relic() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        // Plant 2 LanternKey cards + 1 other card.
        rs.add_card(0, "LanternKey", 0);
        rs.add_card(0, "LanternKey", 0);
        rs.add_card(0, "StrikeIronclad", 0);
        enter_event(&mut rs, 0, "WarHistorianRepy");
        // UNLOCK_CAGE is index 0.
        resolve_event_choice(&mut rs, 0).expect("unlock cage");
        // All LanternKey cards removed.
        let lantern_count = rs.players()[0]
            .deck
            .iter()
            .filter(|c| c.id == "LanternKey")
            .count();
        assert_eq!(lantern_count, 0, "All LanternKey cards must be removed");
        // Strike preserved.
        let strike_count = rs.players()[0]
            .deck
            .iter()
            .filter(|c| c.id == "StrikeIronclad")
            .count();
        assert_eq!(strike_count, 1);
        // HistoryCourse relic granted.
        let has_relic = rs.players()[0]
            .relics
            .iter()
            .any(|r| r.id == "HistoryCourse");
        assert!(has_relic, "HistoryCourse relic must be granted on UnlockCage");
    }

    #[test]
    fn self_help_book_read_back_stages_sharp_enchant_on_attacks() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.add_card(0, "StrikeIronclad", 0); // Attack
        rs.add_card(0, "DefendIronclad", 0); // Skill
        enter_event(&mut rs, 0, "SelfHelpBook");
        resolve_event_choice(&mut rs, 0).expect("read the back");
        let pending = rs.pending_deck_action.as_ref()
            .expect("Sharp enchant must stage a deck-pick");
        assert!(matches!(
            pending.action,
            crate::run_state::DeckActionKind::Enchant { ref enchantment_id, amount: 2 } if enchantment_id == "Sharp"
        ));
        // Only Attack cards eligible (Strike).
        assert_eq!(pending.eligible_indices.len(), 1);
    }

    #[test]
    fn waterlogged_scriptorium_bloody_ink_gains_6_max_hp() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        let pre = rs.players()[0].max_hp;
        enter_event(&mut rs, 0, "WaterloggedScriptorium");
        resolve_event_choice(&mut rs, 0).expect("bloody ink");
        assert_eq!(rs.players()[0].max_hp, pre + 6);
    }

    #[test]
    fn waterlogged_scriptorium_prickly_sponge_spends_99_and_stages_2_picks() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.player_state_mut(0).unwrap().gold = 200;
        rs.add_card(0, "StrikeIronclad", 0);
        rs.add_card(0, "DefendIronclad", 0);
        rs.add_card(0, "Bash", 0);
        enter_event(&mut rs, 0, "WaterloggedScriptorium");
        // PRICKLY_SPONGE is index 2.
        resolve_event_choice(&mut rs, 2).expect("prickly sponge");
        assert_eq!(rs.players()[0].gold, 101, "lose 99 gold");
        let pending = rs.pending_deck_action.as_ref()
            .expect("must stage 2-card pick");
        assert_eq!(pending.n_min, 2);
        assert_eq!(pending.n_max, 2);
    }

    #[test]
    fn round_tea_party_enjoy_tea_grants_relic_and_heals() {
        let mut rs = fresh_rs();
        rs.player_state_mut(0).unwrap().hp = 40; // out of 80
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "RoundTeaParty");
        resolve_event_choice(&mut rs, 0).expect("enjoy tea");
        assert_eq!(rs.players()[0].hp, 80, "full heal");
        let has_relic = rs.players()[0]
            .relics
            .iter()
            .any(|r| r.id == "RoyalPoison");
        assert!(has_relic);
    }

    #[test]
    fn round_tea_party_pick_fight_then_continue_grants_random_relic() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        let pre_hp = rs.players()[0].hp;
        enter_event(&mut rs, 0, "RoundTeaParty");
        resolve_event_choice(&mut rs, 1).expect("pick fight"); // PICK_FIGHT
        assert_eq!(rs.players()[0].hp, pre_hp - 11);
        // Sub-page should show CONTINUE_FIGHT.
        let pending = rs.pending_event.as_ref().expect("continue fight pending");
        assert_eq!(pending.choices.len(), 1);
        assert_eq!(pending.choices[0].label, "CONTINUE_FIGHT");
        resolve_event_choice(&mut rs, 0).expect("continue fight");
        assert_eq!(rs.players()[0].hp, pre_hp - 22, "11 + 11 damage");
        assert!(
            !rs.players()[0].relics.is_empty(),
            "CONTINUE_FIGHT must grant a random relic"
        );
    }

    #[test]
    fn colossal_flower_extract_1_grants_35_gold() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        let pre = rs.players()[0].gold;
        enter_event(&mut rs, 0, "ColossalFlower");
        resolve_event_choice(&mut rs, 0).expect("extract 1");
        assert_eq!(rs.players()[0].gold, pre + 35);
    }

    #[test]
    fn colossal_flower_reach_deeper_chain_to_pollinous_core() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.player_state_mut(0).unwrap().hp = 50; // safe HP
        enter_event(&mut rs, 0, "ColossalFlower");
        // REACH_DEEPER_1 (index 1) → -5 HP, page [EXTRACT_2, REACH_DEEPER_2]
        resolve_event_choice(&mut rs, 1).expect("dig 1");
        assert_eq!(rs.players()[0].hp, 45);
        resolve_event_choice(&mut rs, 1).expect("dig 2"); // REACH_DEEPER_2
        assert_eq!(rs.players()[0].hp, 39);
        // Now page [EXTRACT_INSTEAD, POLLINOUS_CORE]
        resolve_event_choice(&mut rs, 1).expect("pollinous core");
        assert_eq!(rs.players()[0].hp, 32);
        let has = rs.players()[0]
            .relics
            .iter()
            .any(|r| r.id == "PollinousCore");
        assert!(has, "POLLINOUS_CORE must grant the relic");
    }

    #[test]
    fn abyssal_baths_abstain_heals_10() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.player_state_mut(0).unwrap().hp = 50;
        enter_event(&mut rs, 0, "AbyssalBaths");
        resolve_event_choice(&mut rs, 1).expect("abstain"); // ABSTAIN
        assert_eq!(rs.players()[0].hp, 60);
    }

    #[test]
    fn abyssal_baths_immerse_then_linger_escalates_damage() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        let pre_hp = rs.players()[0].hp;
        let pre_max = rs.players()[0].max_hp;
        enter_event(&mut rs, 0, "AbyssalBaths");
        resolve_event_choice(&mut rs, 0).expect("immerse"); // IMMERSE
        // IMMERSE: +2 max HP (also raises cur HP by 2 — engine
        // semantics), then -3 HP. Net hp = pre + 2 - 3 = pre - 1.
        assert_eq!(rs.players()[0].max_hp, pre_max + 2);
        assert_eq!(rs.players()[0].hp, pre_hp - 1);
        // LINGER at damage=4: +2 max HP (+2 cur HP), -4 HP.
        // Net hp = (pre - 1) + 2 - 4 = pre - 3.
        resolve_event_choice(&mut rs, 0).expect("linger 1"); // LINGER
        assert_eq!(rs.players()[0].max_hp, pre_max + 4);
        assert_eq!(rs.players()[0].hp, pre_hp - 3);
    }

    #[test]
    fn ranwid_the_elder_potion_swaps_potion_for_relic() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        // Stock a potion.
        rs.player_state_mut(0).unwrap().potions.push(crate::run_log::PotionEntry {
            id: "BlockPotion".to_string(),
            slot_index: 0,
        });
        let pre_potions = rs.players()[0].potions.len();
        enter_event(&mut rs, 0, "RanwidTheElder");
        resolve_event_choice(&mut rs, 0).expect("potion swap");
        assert_eq!(rs.players()[0].potions.len(), pre_potions - 1);
        assert!(!rs.players()[0].relics.is_empty(), "must gain a relic");
    }

    #[test]
    fn ranwid_the_elder_gold_spends_100_and_grants_relic() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        let pre_gold = rs.players()[0].gold;
        enter_event(&mut rs, 0, "RanwidTheElder");
        resolve_event_choice(&mut rs, 1).expect("gold swap"); // GOLD
        assert_eq!(rs.players()[0].gold, pre_gold - 100);
        assert!(!rs.players()[0].relics.is_empty());
    }

    #[test]
    fn slippery_bridge_overcome_removes_random_card() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.add_card(0, "StrikeIronclad", 0);
        rs.add_card(0, "DefendIronclad", 0);
        let pre = rs.players()[0].deck.len();
        enter_event(&mut rs, 0, "SlipperyBridge");
        resolve_event_choice(&mut rs, 0).expect("overcome"); // OVERCOME
        assert_eq!(rs.players()[0].deck.len(), pre - 1);
    }

    #[test]
    fn slippery_bridge_hold_on_escalates_damage() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        let pre = rs.players()[0].hp;
        enter_event(&mut rs, 0, "SlipperyBridge");
        // HOLD_ON at damage=3.
        resolve_event_choice(&mut rs, 1).expect("hold on 1");
        assert_eq!(rs.players()[0].hp, pre - 3);
        // HOLD_ON at damage=4.
        resolve_event_choice(&mut rs, 1).expect("hold on 2");
        assert_eq!(rs.players()[0].hp, pre - 7);
    }

    #[test]
    fn tinker_time_chain_adds_mad_science_card() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "TinkerTime");
        // CHOOSE_CARD_TYPE → page [ATTACK, SKILL, POWER]
        resolve_event_choice(&mut rs, 0).expect("choose type");
        let pending = rs.pending_event.as_ref().expect("type-pick page");
        assert_eq!(pending.choices[0].label, "ATTACK");
        // Pick ATTACK → page [RIDER_A, RIDER_B, RIDER_C]
        resolve_event_choice(&mut rs, 0).expect("attack");
        let pending = rs.pending_event.as_ref().expect("rider page");
        assert_eq!(pending.choices.len(), 3);
        resolve_event_choice(&mut rs, 0).expect("rider a");
        // MadScience added to deck.
        let has_mad = rs.players()[0]
            .deck
            .iter()
            .any(|c| c.id == "MadScience");
        assert!(has_mad, "MadScience must be added to deck");
    }

    #[test]
    fn the_architect_accept_calls_event_combat_stub() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "TheArchitect");
        // ACCEPT is index 0. The body fires EnterEventCombat (stub).
        resolve_event_choice(&mut rs, 0).expect("accept");
        // No assertable side effects from the stub — just confirm
        // the event terminated cleanly.
        assert!(rs.pending_event.is_none());
    }

    #[test]
    fn trial_accept_picks_one_of_three_random_sub_trials() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "Trial");
        resolve_event_choice(&mut rs, 0).expect("accept");
        let pending = rs.pending_event.as_ref()
            .expect("sub-trial page must be open");
        // Each sub-trial has 2 choices.
        assert_eq!(pending.choices.len(), 2);
        let labels: Vec<&str> = pending.choices.iter()
            .map(|c| c.label.as_str())
            .collect();
        // One of the three sub-trial pairs must match.
        let is_merchant = labels == ["MERCHANT_GUILTY", "MERCHANT_INNOCENT"];
        let is_noble = labels == ["NOBLE_GUILTY", "NOBLE_INNOCENT"];
        let is_nondescript = labels == ["NONDESCRIPT_GUILTY", "NONDESCRIPT_INNOCENT"];
        assert!(is_merchant || is_noble || is_nondescript,
            "Sub-trial labels {:?} don't match any expected pair", labels);
    }

    #[test]
    fn trial_reject_then_double_down_zeros_hp() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        enter_event(&mut rs, 0, "Trial");
        resolve_event_choice(&mut rs, 1).expect("reject"); // REJECT
        let pending = rs.pending_event.as_ref().expect("double-down page");
        assert_eq!(pending.choices[0].label, "ACCEPT");
        assert_eq!(pending.choices[1].label, "DOUBLE_DOWN");
        resolve_event_choice(&mut rs, 1).expect("double down");
        assert_eq!(rs.players()[0].hp, 0, "DOUBLE_DOWN should zero HP");
    }

    #[test]
    fn endless_conveyor_observe_chef_upgrades_card() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.add_card(0, "StrikeIronclad", 0);
        enter_event(&mut rs, 0, "EndlessConveyor");
        // OBSERVE_CHEF is index 1.
        resolve_event_choice(&mut rs, 1).expect("observe");
        let upgraded = rs.players()[0]
            .deck
            .iter()
            .any(|c| c.current_upgrade_level == Some(1));
        assert!(upgraded, "OBSERVE_CHEF must upgrade 1 card");
    }

    #[test]
    fn endless_conveyor_grab_rolls_dish_and_loses_gold() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.player_state_mut(0).unwrap().gold = 200;
        enter_event(&mut rs, 0, "EndlessConveyor");
        resolve_event_choice(&mut rs, 0).expect("grab"); // GRAB
        assert!(rs.players()[0].gold <= 160,
            "GRAB must lose 40 gold (may lose more if dish costs)");
        // A dish branch should be staged.
        let pending = rs.pending_event.as_ref().expect("dish page");
        assert_eq!(pending.choices.len(), 1, "single dish button");
    }

    #[test]
    fn the_future_of_potions_trade_discards_potion_and_offers_card() {
        let mut rs = fresh_rs();
        rs.auto_resolve_offers = false;
        rs.player_state_mut(0).unwrap().potions.push(crate::run_log::PotionEntry {
            id: "BlockPotion".to_string(),
            slot_index: 0,
        });
        let pre_potions = rs.players()[0].potions.len();
        enter_event(&mut rs, 0, "TheFutureOfPotions");
        resolve_event_choice(&mut rs, 0).expect("trade");
        assert_eq!(rs.players()[0].potions.len(), pre_potions - 1);
        let offer = rs.pending_offer.as_ref()
            .expect("card-reward offer must be staged");
        assert_eq!(offer.options.len(), 3);
    }
}
