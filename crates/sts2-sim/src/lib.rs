//! Slay the Spire 2 headless simulator.
//!
//! Modules are added as ports land. No module is considered done until it passes
//! the oracle diff tests in the `sts2-sim-oracle-tests` crate.

pub mod act;
pub mod affliction;
pub mod ascension;
pub mod campfire;
pub mod card;
pub mod card_reward;
pub mod character;
pub mod combat;
pub mod effects;
pub mod enchantment;
pub mod encounter;
pub mod env;
pub mod event;
pub mod event_room;
pub mod features;
pub mod hash;
pub mod map;
pub mod modifier;
pub mod monster;
pub mod monster_ai;
pub mod monster_dispatch;
pub mod path_align;
pub mod run_flow;
pub mod orb;
pub mod path_pruning;
pub mod potion;
pub mod power;
pub mod relic;
pub mod rng;
pub mod rng_set;
pub mod room_set;
pub mod run_log;
pub mod run_state;
pub mod unknown_room;
pub mod shop;
pub mod shuffle;
pub mod standard_act_map;
pub mod treasure;
