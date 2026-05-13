//! Containers for the named-stream Rngs that the game uses to keep
//! randomization deterministic and decoupled across systems.
//!
//! Two layers:
//! - `RunRngSet` (per run): 12 streams. Seeded from the player's string seed
//!   via `deterministic_hash_code`; each stream is sub-seeded by the
//!   snake-cased enum variant name.
//! - `PlayerRngSet` (per player, per run): 3 streams. Same sub-seeding scheme
//!   but the base seed is the run seed as `uint` (not re-hashed).
//!
//! Eager representation for now: all streams materialized inline. If
//! clone-heavy training becomes a bottleneck, swap to lazy materialization
//! while keeping this API stable.
//!
//! Stream names are hardcoded snake-case constants here. They correspond
//! 1:1 with C#'s `SnakeCase(enumValue.ToString())` from the
//! `RunRngType` / `PlayerRngType` enums. Diff tests verify each name
//! produces the same sub-seed as the real game.

use crate::hash::deterministic_hash_code;
use crate::rng::Rng;

// snake_case names matching SnakeCase(RunRngType.ToString()).
const NAME_UP_FRONT: &str = "up_front";
const NAME_SHUFFLE: &str = "shuffle";
const NAME_UNKNOWN_MAP_POINT: &str = "unknown_map_point";
const NAME_COMBAT_CARD_GENERATION: &str = "combat_card_generation";
const NAME_COMBAT_POTION_GENERATION: &str = "combat_potion_generation";
const NAME_COMBAT_CARD_SELECTION: &str = "combat_card_selection";
const NAME_COMBAT_ENERGY_COSTS: &str = "combat_energy_costs";
const NAME_COMBAT_TARGETS: &str = "combat_targets";
const NAME_MONSTER_AI: &str = "monster_ai";
const NAME_NICHE: &str = "niche";
const NAME_COMBAT_ORBS: &str = "combat_orbs";
const NAME_TREASURE_ROOM_RELICS: &str = "treasure_room_relics";

// snake_case names matching SnakeCase(PlayerRngType.ToString()).
const NAME_REWARDS: &str = "rewards";
const NAME_SHOPS: &str = "shops";
const NAME_TRANSFORMATIONS: &str = "transformations";

/// 12 run-level streams (`MegaCrit.Sts2.Core.Runs.RunRngSet`). Seed is the
/// `uint` form of the player's string seed (via `deterministic_hash_code`);
/// each field is `Rng::new_named(seed_uint, "<snake_case_name>")`.
#[derive(Debug, Clone)]
pub struct RunRngSet {
    string_seed: String,
    seed_uint: u32,
    pub up_front: Rng,
    pub shuffle: Rng,
    pub unknown_map_point: Rng,
    pub combat_card_generation: Rng,
    pub combat_potion_generation: Rng,
    pub combat_card_selection: Rng,
    pub combat_energy_costs: Rng,
    pub combat_targets: Rng,
    pub monster_ai: Rng,
    pub niche: Rng,
    pub combat_orbs: Rng,
    pub treasure_room_relics: Rng,
}

impl RunRngSet {
    pub fn new(string_seed: &str) -> Self {
        let seed_uint = deterministic_hash_code(string_seed) as u32;
        Self::from_seed_uint(string_seed, seed_uint)
    }

    fn from_seed_uint(string_seed: &str, seed_uint: u32) -> Self {
        Self {
            string_seed: string_seed.to_owned(),
            seed_uint,
            up_front: Rng::new_named(seed_uint, NAME_UP_FRONT),
            shuffle: Rng::new_named(seed_uint, NAME_SHUFFLE),
            unknown_map_point: Rng::new_named(seed_uint, NAME_UNKNOWN_MAP_POINT),
            combat_card_generation: Rng::new_named(seed_uint, NAME_COMBAT_CARD_GENERATION),
            combat_potion_generation: Rng::new_named(seed_uint, NAME_COMBAT_POTION_GENERATION),
            combat_card_selection: Rng::new_named(seed_uint, NAME_COMBAT_CARD_SELECTION),
            combat_energy_costs: Rng::new_named(seed_uint, NAME_COMBAT_ENERGY_COSTS),
            combat_targets: Rng::new_named(seed_uint, NAME_COMBAT_TARGETS),
            monster_ai: Rng::new_named(seed_uint, NAME_MONSTER_AI),
            niche: Rng::new_named(seed_uint, NAME_NICHE),
            combat_orbs: Rng::new_named(seed_uint, NAME_COMBAT_ORBS),
            treasure_room_relics: Rng::new_named(seed_uint, NAME_TREASURE_ROOM_RELICS),
        }
    }

    pub fn string_seed(&self) -> &str {
        &self.string_seed
    }

    pub fn seed_uint(&self) -> u32 {
        self.seed_uint
    }

    /// Save snapshot: just the seed plus each stream's counter.
    /// Restore is `RunRngSet::new(seed)` followed by per-stream
    /// `FastForwardCounter`. Matches the C# `SerializableRunRngSet` model.
    pub fn snapshot_counters(&self) -> [(&'static str, i32); 12] {
        [
            (NAME_UP_FRONT, self.up_front.counter()),
            (NAME_SHUFFLE, self.shuffle.counter()),
            (NAME_UNKNOWN_MAP_POINT, self.unknown_map_point.counter()),
            (NAME_COMBAT_CARD_GENERATION, self.combat_card_generation.counter()),
            (NAME_COMBAT_POTION_GENERATION, self.combat_potion_generation.counter()),
            (NAME_COMBAT_CARD_SELECTION, self.combat_card_selection.counter()),
            (NAME_COMBAT_ENERGY_COSTS, self.combat_energy_costs.counter()),
            (NAME_COMBAT_TARGETS, self.combat_targets.counter()),
            (NAME_MONSTER_AI, self.monster_ai.counter()),
            (NAME_NICHE, self.niche.counter()),
            (NAME_COMBAT_ORBS, self.combat_orbs.counter()),
            (NAME_TREASURE_ROOM_RELICS, self.treasure_room_relics.counter()),
        ]
    }
}

/// 3 per-player streams (`MegaCrit.Sts2.Core.Random.PlayerRngSet`). The
/// base seed is passed in directly as `uint` — the C# class takes it as
/// `uint`, not as a string to be hashed.
#[derive(Debug, Clone)]
pub struct PlayerRngSet {
    seed: u32,
    pub rewards: Rng,
    pub shops: Rng,
    pub transformations: Rng,
}

impl PlayerRngSet {
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            rewards: Rng::new_named(seed, NAME_REWARDS),
            shops: Rng::new_named(seed, NAME_SHOPS),
            transformations: Rng::new_named(seed, NAME_TRANSFORMATIONS),
        }
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn snapshot_counters(&self) -> [(&'static str, i32); 3] {
        [
            (NAME_REWARDS, self.rewards.counter()),
            (NAME_SHOPS, self.shops.counter()),
            (NAME_TRANSFORMATIONS, self.transformations.counter()),
        ]
    }
}
