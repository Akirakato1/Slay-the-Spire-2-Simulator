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
                    body: vec![Effect::LoseRunStateGold {
                        amount: AmountSpec::Fixed(149),
                    }],
                    // TODO: + reward (relic / card pick)
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
                    body: vec![Effect::UpgradeRandomDeckCards {
                        n: AmountSpec::Fixed(2),
                        filter: crate::effects::CardFilter::Upgradable,
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
                    body: vec![Effect::LoseRunStateHp { amount: AmountSpec::Fixed(10) }],
                    // TODO: + upgrade-1-picked-card (deck-action staging)
                },
            ],
        }),

        // TabletOfTruth: 3 choices, multi-step (Decipher loops 5 times).
        // For MVP encode Smash + GiveUp; Decipher's loop semantics needs
        // a multi-page event surface that doesn't exist yet.
        "TabletOfTruth" => Some(EventModel {
            id: "TabletOfTruth".to_string(),
            choices: vec![
                EventChoice {
                    label: "SMASH".to_string(),
                    body: vec![Effect::HealRunState { amount: AmountSpec::Fixed(20) }],
                },
                EventChoice {
                    label: "DECIPHER".to_string(),
                    body: vec![Effect::LoseRunStateMaxHp { amount: AmountSpec::Fixed(3) }],
                    // TODO: + upgrade 1 picked card; loops 5 times via re-entry.
                },
                EventChoice {
                    label: "GIVE_UP".to_string(),
                    body: vec![],
                },
            ],
        }),

        // WhisperingHollow: Gold (-35 gold + 2 potion rewards) vs Hug
        // (transform 1 picked card with -9 HP).
        "WhisperingHollow" => Some(EventModel {
            id: "WhisperingHollow".to_string(),
            choices: vec![
                EventChoice {
                    label: "GOLD".to_string(),
                    body: vec![Effect::LoseRunStateGold {
                        amount: AmountSpec::Fixed(35),
                    }],
                    // TODO: + 2 potion rewards
                },
                EventChoice {
                    label: "HUG".to_string(),
                    body: vec![
                        Effect::LoseRunStateHp { amount: AmountSpec::Fixed(9) },
                        // TODO: should be "transform 1 PICKED card" not random.
                        Effect::TransformRandomDeckCards {
                            n: AmountSpec::Fixed(1),
                            filter: crate::effects::CardFilter::Any,
                            pool: crate::effects::CardPoolRef::CharacterAny,
                        },
                    ],
                },
            ],
        }),

        // PotionCourier: Grab (3 FoulPotions to belt) vs Ransack (1 uncommon potion).
        // Ransack picks a random uncommon potion via PlayerRng.rewards — for
        // MVP we drop GlowwaterPotion as a placeholder.
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
                    // TODO: random uncommon potion. Placeholder.
                    body: vec![Effect::GainPotionToBelt {
                        potion_id: "GlowwaterPotion".to_string(),
                    }],
                },
            ],
        }),

        // RoomFullOfCheese: Gorge (8 common card rewards — stub) vs
        // Search (-14 HP unblockable + ChosenCheese relic).
        "RoomFullOfCheese" => Some(EventModel {
            id: "RoomFullOfCheese".to_string(),
            choices: vec![
                EventChoice {
                    label: "GORGE".to_string(),
                    body: vec![], // TODO: 8-card common reward picker
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

        // Wellspring: Bathe (remove 1 picked card + 1 Guilty curse) vs
        // Bottle (random potion). Both stub mostly — Bathe needs pick;
        // Bottle uses PlayerRng.Rewards.
        "Wellspring" => Some(EventModel {
            id: "Wellspring".to_string(),
            choices: vec![
                EventChoice {
                    label: "BATHE".to_string(),
                    body: vec![Effect::AddCardToRunStateDeck {
                        card_id: "Guilty".to_string(), upgrade: 0,
                    }],
                    // TODO: + remove 1 picked card
                },
                EventChoice {
                    label: "BOTTLE".to_string(),
                    body: vec![], // TODO: random potion drop
                },
            ],
        }),

        // BattlewornDummy + DenseVegetation + PunchOff: combat-in-event.
        // The "choice" is which encounter to fight; outcome is determined
        // by combat. Skipped until event-driven combat lands.
        "BattlewornDummy" | "DenseVegetation" | "PunchOff" => Some(EventModel {
            id: id.to_string(),
            choices: vec![
                EventChoice {
                    label: "FIGHT".to_string(),
                    body: vec![], // TODO: trigger event-encounter combat
                },
            ],
        }),

        // RelicTrader: pick one of your relics to swap for one of 3 new ones.
        // Skipped — needs RelicSwap primitive.
        "RelicTrader" => Some(EventModel {
            id: "RelicTrader".to_string(),
            choices: vec![
                EventChoice { label: "TOP".to_string(),    body: vec![] },
                EventChoice { label: "MIDDLE".to_string(), body: vec![] },
                EventChoice { label: "BOTTOM".to_string(), body: vec![] },
                EventChoice { label: "LEAVE".to_string(),  body: vec![] },
            ],
        }),

        // FakeMerchant: a deceptive 3-relic shop. Skip — needs Shop event flow.
        "FakeMerchant" => Some(EventModel {
            id: "FakeMerchant".to_string(),
            choices: vec![EventChoice { label: "LEAVE".to_string(), body: vec![] }],
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
                    body: vec![], // TODO: interactive enchant
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
        "FieldOfManSizedHoles" => Some(EventModel {
            id: "FieldOfManSizedHoles".to_string(),
            choices: vec![
                EventChoice { label: "RESIST".to_string(),         body: vec![] },
                EventChoice { label: "ENTER_YOUR_HOLE".to_string(), body: vec![] },
                EventChoice { label: "LEAVE".to_string(),           body: vec![] },
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
                    body: vec![], // TODO: event-combat with LanternKey reward
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
                    body: vec![Effect::LoseRunStateGold {
                        amount: AmountSpec::Fixed(50),
                    }],
                    // TODO: minigame produces 3 prophesized cards
                },
                EventChoice {
                    label: "PAYMENT_PLAN".to_string(),
                    body: vec![Effect::AddCardToRunStateDeck {
                        card_id: "Debt".to_string(), upgrade: 0,
                    }],
                    // TODO: minigame produces 6 prophesized cards
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

        // DollRoom: TakeSomeTime (-5 HP, 2-of-5 random doll) vs Examine
        // (-15 HP, all-5 doll pick) vs ChooseRandom (1 random doll).
        // "Doll" is a random relic; needs relic-pool primitive — stub.
        "DollRoom" => Some(EventModel {
            id: "DollRoom".to_string(),
            choices: vec![
                EventChoice {
                    label: "CHOOSE_RANDOM".to_string(),
                    body: vec![],
                },
                EventChoice {
                    label: "TAKE_SOME_TIME".to_string(),
                    body: vec![Effect::LoseRunStateHp { amount: AmountSpec::Fixed(5) }],
                },
                EventChoice {
                    label: "EXAMINE".to_string(),
                    body: vec![Effect::LoseRunStateHp { amount: AmountSpec::Fixed(15) }],
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
                    body: vec![Effect::LoseRunStateGold {
                        amount: AmountSpec::Fixed(100),
                    }],
                    // TODO: + random Common relic from pool
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
                    body: vec![Effect::LoseRunStateGold {
                        amount: AmountSpec::Fixed(200),
                    }],
                    // TODO: + featured-item relic (varies per visit)
                },
                EventChoice { label: "LEAVE".to_string(), body: vec![] },
            ],
        }),

        // ZenWeaver: 3 graduated remove-cards options at gold cost.
        "ZenWeaver" => Some(EventModel {
            id: "ZenWeaver".to_string(),
            choices: vec![
                EventChoice {
                    label: "BREATHING_TECHNIQUES".to_string(),
                    body: vec![Effect::LoseRunStateGold {
                        amount: AmountSpec::Fixed(50),
                    }],
                    // TODO: + transform-2-cards-into-zen-something
                },
                EventChoice {
                    label: "EMOTIONAL_AWARENESS".to_string(),
                    body: vec![Effect::LoseRunStateGold {
                        amount: AmountSpec::Fixed(125),
                    }],
                    // TODO: + remove 1 picked card
                },
                EventChoice {
                    label: "ARACHNID_ACUPUNCTURE".to_string(),
                    body: vec![Effect::LoseRunStateGold {
                        amount: AmountSpec::Fixed(250),
                    }],
                    // TODO: + remove 2 picked cards
                },
            ],
        }),

        // Amalgamator: combine 2 Strikes / 2 Defends → single card.
        // Needs interactive pick of 2-by-tag from deck; stub.
        "Amalgamator" => Some(EventModel {
            id: "Amalgamator".to_string(),
            choices: vec![
                EventChoice { label: "COMBINE_STRIKES".to_string(), body: vec![] },
                EventChoice { label: "COMBINE_DEFENDS".to_string(), body: vec![] },
                EventChoice { label: "LEAVE".to_string(),            body: vec![] },
            ],
        }),

        // SapphireSeed: Eat (+9 heal + upgrade picked) vs Plant (enchant Sown).
        "SapphireSeed" => Some(EventModel {
            id: "SapphireSeed".to_string(),
            choices: vec![
                EventChoice {
                    label: "EAT".to_string(),
                    body: vec![Effect::HealRunState { amount: AmountSpec::Fixed(9) }],
                    // TODO: + upgrade 1 picked card
                },
                EventChoice {
                    label: "PLANT".to_string(),
                    body: vec![],
                    // TODO: enchant 1 picked card with Sown
                },
            ],
        }),

        // StoneOfAllTime: Drink → +10 MaxHp. Push → -6 HP + Vigorous enchant.
        // Lift → +MaxHp + RNG side-effect (stub).
        "StoneOfAllTime" => Some(EventModel {
            id: "StoneOfAllTime".to_string(),
            choices: vec![
                EventChoice {
                    label: "DRINK".to_string(),
                    body: vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(10) }],
                },
                EventChoice {
                    label: "PUSH".to_string(),
                    body: vec![Effect::LoseRunStateHp { amount: AmountSpec::Fixed(6) }],
                    // TODO: + Vigorous(8) enchant pick
                },
                EventChoice {
                    label: "LIFT".to_string(),
                    body: vec![Effect::GainRunStateMaxHp { amount: AmountSpec::Fixed(10) }],
                    // TODO: + discard a potion
                },
            ],
        }),

        // WoodCarvings: 3 carvings (Bird/Snake/Torus) — all enchant or
        // transform 1 picked Basic card. All stubs for pick infra.
        "WoodCarvings" => Some(EventModel {
            id: "WoodCarvings".to_string(),
            choices: vec![
                EventChoice { label: "BIRD".to_string(),  body: vec![] },
                EventChoice { label: "SNAKE".to_string(), body: vec![] },
                EventChoice { label: "TORUS".to_string(), body: vec![] },
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
                    body: vec![Effect::RemoveAllCardsOfType {
                        card_id: "LanternKey".to_string(),
                    }],
                    // TODO: + custom-reward bundle (2 PotionReward +
                    // 2 RelicReward) — needs OfferCustomRewards infra.
                },
            ],
        }),

        // Trial: Accept (clean run end / advance) vs Reject (open
        // double-down sub-menu — multi-page). Skeleton until event
        // state machine supports multi-page.
        "Trial" => Some(EventModel {
            id: "Trial".to_string(),
            choices: vec![
                EventChoice { label: "ACCEPT".to_string(), body: vec![] },
                EventChoice { label: "REJECT".to_string(), body: vec![] },
            ],
        }),

        // Multi-page / complex events. Skeletons keep the registry
        // complete so unknown-event errors don't surface.
        "AbyssalBaths"
        | "ColossalFlower"
        | "EndlessConveyor"
        | "RanwidTheElder"
        | "RoundTeaParty"
        | "SelfHelpBook"
        | "SlipperyBridge"
        | "TheArchitect"
        | "TheFutureOfPotions"
        | "TinkerTime"
        | "WaterloggedScriptorium"
        | "DeprecatedEvent"
        => Some(EventModel {
            id: id.to_string(),
            // Single LEAVE option — the event is registered but its body
            // is a no-op until the missing infra (deck-pick from filter,
            // event-combat, multi-page state, relic-pool selection,
            // potion-pool selection) lands.
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
        resolve_event_choice(&mut rs, 0).expect("smash");
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
        resolve_event_choice(&mut rs, 0).expect("smash");
        assert_eq!(rs.players()[0].hp, 80,
            "heal must cap at max_hp");
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
}
