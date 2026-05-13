//! Minimum `ActModel` surface needed for map generation.
//!
//! The real `MegaCrit.Sts2.Core.Models.ActModel` is a 700+ LOC abstract class
//! with encounters, events, ancients, multiplayer hooks, etc. Map generation
//! only consumes four pieces of it:
//!   - `BaseNumberOfRooms` (int constant per act)
//!   - `GetNumberOfRooms(bool)` (= base minus multiplayer flag)
//!   - `GetMapPointTypes(Rng)` (per-act PRNG-derived MapPointTypeCounts)
//!   - `HasSecondBoss` (runs-state-driven; defaults false for map gen)
//!
//! That's what we port here. Encounters, events, etc. land with the
//! relevant later modules.

use crate::map::MapPointTypeCounts;
use crate::rng::Rng;

/// Stable identifier for each concrete act. Used in oracle tests to route
/// to the right C# act class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActId {
    Overgrowth,
    Hive,
    Glory,
    Underdocks,
    DeprecatedAct,
}

impl ActId {
    /// Fully-qualified C# type name, used by the oracle to reflect into the
    /// shipping `sts2.dll`. Mirrors the namespace + class name exactly.
    pub fn csharp_type_name(self) -> &'static str {
        match self {
            ActId::Overgrowth => "MegaCrit.Sts2.Core.Models.Acts.Overgrowth",
            ActId::Hive => "MegaCrit.Sts2.Core.Models.Acts.Hive",
            ActId::Glory => "MegaCrit.Sts2.Core.Models.Acts.Glory",
            ActId::Underdocks => "MegaCrit.Sts2.Core.Models.Acts.Underdocks",
            ActId::DeprecatedAct => "MegaCrit.Sts2.Core.Models.Acts.DeprecatedAct",
        }
    }
}

/// The map-gen-only ActModel surface.
pub trait ActModel {
    fn id(&self) -> ActId;
    /// `BaseNumberOfRooms` from the C# `ActModel` subclasses. Hard-coded per act.
    fn base_number_of_rooms(&self) -> i32;
    /// `GetMapPointTypes(Rng)`. Consumes PRNG state in a specific order per act.
    /// The C# initializer for `MapPointTypeCounts.NumOfElites` reads
    /// `AscensionHelper.HasAscension(SwarmingElites)` (= ascension >= 1)
    /// to scale 5 → 8 elites; pass the run's ascension level through.
    fn get_map_point_types(&self, rng: &mut Rng, ascension: i32) -> MapPointTypeCounts;
    /// `HasSecondBoss`: in the C# class this delegates to `_rooms.HasSecondBoss`,
    /// which is set by run-state and is false for normal solo runs. Map gen
    /// itself only branches on this flag, so we expose a default false here
    /// and let the call site override (e.g. when porting RunState).
    fn has_second_boss(&self) -> bool {
        false
    }

    /// `GetNumberOfRooms(bool)` — defined once on the base class as
    /// `BaseNumberOfRooms - (isMultiplayer ? 1 : 0)`.
    fn get_number_of_rooms(&self, is_multiplayer: bool) -> i32 {
        let base = self.base_number_of_rooms();
        if is_multiplayer { base - 1 } else { base }
    }
}

// ---- concrete acts ----

#[derive(Debug, Clone, Copy)]
pub struct Overgrowth;

impl ActModel for Overgrowth {
    fn id(&self) -> ActId { ActId::Overgrowth }
    fn base_number_of_rooms(&self) -> i32 { 15 }
    fn get_map_point_types(&self, rng: &mut Rng, ascension: i32) -> MapPointTypeCounts {
        // Order of PRNG consumption is load-bearing — mirror C# exactly.
        let rests = rng.next_gaussian_int(7, 1, 6, 7);
        let unknowns = MapPointTypeCounts::standard_random_unknown_count(rng);
        MapPointTypeCounts::for_ascension(unknowns, rests, ascension)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Hive;

impl ActModel for Hive {
    fn id(&self) -> ActId { ActId::Hive }
    fn base_number_of_rooms(&self) -> i32 { 14 }
    fn get_map_point_types(&self, rng: &mut Rng, ascension: i32) -> MapPointTypeCounts {
        let rests = rng.next_gaussian_int(6, 1, 6, 7);
        let unknowns = MapPointTypeCounts::standard_random_unknown_count(rng) - 1;
        MapPointTypeCounts::for_ascension(unknowns, rests, ascension)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Glory;

impl ActModel for Glory {
    fn id(&self) -> ActId { ActId::Glory }
    fn base_number_of_rooms(&self) -> i32 { 13 }
    fn get_map_point_types(&self, rng: &mut Rng, ascension: i32) -> MapPointTypeCounts {
        // Note: Glory uses NextInt(5, 7), not NextGaussianInt.
        let rests = rng.next_int_range(5, 7);
        let unknowns = MapPointTypeCounts::standard_random_unknown_count(rng) - 1;
        MapPointTypeCounts::for_ascension(unknowns, rests, ascension)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Underdocks;

impl ActModel for Underdocks {
    fn id(&self) -> ActId { ActId::Underdocks }
    fn base_number_of_rooms(&self) -> i32 { 15 }
    fn get_map_point_types(&self, rng: &mut Rng, ascension: i32) -> MapPointTypeCounts {
        let rests = rng.next_gaussian_int(7, 1, 6, 7);
        let unknowns = MapPointTypeCounts::standard_random_unknown_count(rng);
        MapPointTypeCounts::for_ascension(unknowns, rests, ascension)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DeprecatedAct;

impl ActModel for DeprecatedAct {
    fn id(&self) -> ActId { ActId::DeprecatedAct }
    fn base_number_of_rooms(&self) -> i32 { 0 }
    fn get_map_point_types(&self, _rng: &mut Rng, ascension: i32) -> MapPointTypeCounts {
        // Deprecated act: returns (0, 0) without touching the Rng. Still
        // applies SwarmingElites scaling so the elite count is consistent
        // if anything queries it.
        MapPointTypeCounts::for_ascension(0, 0, ascension)
    }
}

/// Convenience accessor — get the canonical act instance for an `ActId`.
/// Returns a `Box<dyn ActModel>` so callers can hold it without knowing
/// the concrete type.
pub fn act_for(id: ActId) -> Box<dyn ActModel> {
    match id {
        ActId::Overgrowth => Box::new(Overgrowth),
        ActId::Hive => Box::new(Hive),
        ActId::Glory => Box::new(Glory),
        ActId::Underdocks => Box::new(Underdocks),
        ActId::DeprecatedAct => Box::new(DeprecatedAct),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_room_counts_match_decompile() {
        assert_eq!(Overgrowth.base_number_of_rooms(), 15);
        assert_eq!(Hive.base_number_of_rooms(), 14);
        assert_eq!(Glory.base_number_of_rooms(), 13);
        assert_eq!(Underdocks.base_number_of_rooms(), 15);
        assert_eq!(DeprecatedAct.base_number_of_rooms(), 0);
    }

    #[test]
    fn multiplayer_subtracts_one_room() {
        assert_eq!(Overgrowth.get_number_of_rooms(false), 15);
        assert_eq!(Overgrowth.get_number_of_rooms(true), 14);
        assert_eq!(Hive.get_number_of_rooms(true), 13);
    }

    #[test]
    fn deprecated_act_does_not_advance_rng() {
        // Sanity: DeprecatedAct.GetMapPointTypes(rng) returns (0,0) without
        // touching the Rng. Verify counter stays at 0 after the call.
        let mut rng = Rng::new(42, 0);
        let initial_counter = rng.counter();
        let counts = DeprecatedAct.get_map_point_types(&mut rng, 0);
        assert_eq!(counts.num_of_unknowns, 0);
        assert_eq!(counts.num_of_rests, 0);
        assert_eq!(rng.counter(), initial_counter,
            "DeprecatedAct must not advance the Rng counter");
    }

    #[test]
    fn overgrowth_rests_in_range() {
        let mut rng = Rng::new(123, 0);
        for _ in 0..50 {
            let counts = Overgrowth.get_map_point_types(&mut rng, 0);
            assert!((6..=7).contains(&counts.num_of_rests),
                "rests out of range: {}", counts.num_of_rests);
            assert!((10..=14).contains(&counts.num_of_unknowns),
                "unknowns out of range: {}", counts.num_of_unknowns);
            assert_eq!(counts.num_of_shops, 3);
            assert_eq!(counts.num_of_elites, 5);
        }
    }

    #[test]
    fn glory_uses_next_int_range_for_rests() {
        // Glory's GetMapPointTypes uses NextInt(5, 7) for rests, which
        // unlike NextGaussianInt DOES advance the counter (it's a regular
        // NextInt). Verify counter advances by exactly 1 per call.
        let mut rng = Rng::new(7, 0);
        let initial = rng.counter();
        Glory.get_map_point_types(&mut rng, 0);
        // Glory: 1 NextInt call (counter +1), then StandardRandomUnknownCount
        // which is NextGaussianInt (counter unchanged). Total +1.
        assert_eq!(rng.counter(), initial + 1,
            "Glory should advance counter by exactly 1 (the NextInt for rests)");
    }
}
