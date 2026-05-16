//! Per-ascension modifiers. Mirrors C# `AscensionLevel.cs` (the
//! `AscensionLevel` enum) + `AscensionHelper.cs` + per-level
//! application logic.
//!
//! STS2 has 10 ascension levels. Each named level unlocks at its
//! integer value via C# `AscensionManager.HasLevel(level) =
//! (_level >= (int)level)`. A run at ascension N has every modifier
//! at level ≤ N active.
//!
//! | Level | Name             | Effect (high level)                        |
//! |-------|------------------|--------------------------------------------|
//! | 1     | SwarmingElites   | 5 → 8 elite map points                     |
//! | 2     | WearyTraveler    | -5 max HP at run start                     |
//! | 3     | Poverty          | combat gold reward × 0.75                  |
//! | 4     | TightBelt        | no potion drops from combat                |
//! | 5     | AscendersBane    | starts run with AscendersBane curse card   |
//! | 6     | Inflation        | shop prices / elite gold bumped            |
//! | 7     | Scarcity         | fewer card rewards offered                 |
//! | 8     | ToughEnemies     | monsters use `*_hp_ascended` HP            |
//! | 9     | DeadlyEnemies    | monster attacks deal ascended damage       |
//! | 10    | DoubleBoss       | second boss fight at act end               |

#![allow(non_upper_case_globals)]

/// Each constant is the ascension level at which the modifier kicks
/// in. Used by `AmountSpec::AscensionScaled.threshold` and the
/// run-flow / combat init paths that gate behavior on `ascension >=
/// LEVEL`. Keeping these named constants instead of bare numbers
/// makes the threshold meaning explicit at every call site.
pub mod level {
    pub const SwarmingElites: i32 = 1;
    pub const WearyTraveler: i32 = 2;
    pub const Poverty: i32 = 3;
    pub const TightBelt: i32 = 4;
    pub const AscendersBane: i32 = 5;
    pub const Inflation: i32 = 6;
    pub const Scarcity: i32 = 7;
    pub const ToughEnemies: i32 = 8;
    pub const DeadlyEnemies: i32 = 9;
    pub const DoubleBoss: i32 = 10;
}

/// `true` iff the run is at or above the named ascension threshold.
/// Mirrors C# `AscensionManager.HasLevel(level)`.
#[inline]
pub fn has_level(current_ascension: i32, threshold: i32) -> bool {
    current_ascension >= threshold
}

/// `C# AscensionHelper.PovertyAscensionGoldMultiplier`. Applied to
/// combat gold rewards at A3+.
pub const POVERTY_GOLD_MULTIPLIER: f64 = 0.75;

/// `C# WearyTraveler`-driven max-HP reduction applied at run start
/// when A2+.
pub const WEARY_TRAVELER_MAX_HP_DELTA: i32 = -5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_level_is_inclusive_at_threshold() {
        assert!(!has_level(0, level::ToughEnemies));
        assert!(!has_level(7, level::ToughEnemies));
        assert!(has_level(8, level::ToughEnemies));
        assert!(has_level(9, level::ToughEnemies));
        assert!(has_level(10, level::ToughEnemies));
    }

    #[test]
    fn a10_run_has_every_modifier_active() {
        for thr in [
            level::SwarmingElites, level::WearyTraveler, level::Poverty,
            level::TightBelt, level::AscendersBane, level::Inflation,
            level::Scarcity, level::ToughEnemies, level::DeadlyEnemies,
            level::DoubleBoss,
        ] {
            assert!(has_level(10, thr), "A10 should activate level {}", thr);
        }
    }

    #[test]
    fn a0_run_has_no_modifiers_active() {
        for thr in [
            level::SwarmingElites, level::WearyTraveler, level::Poverty,
            level::TightBelt, level::AscendersBane, level::Inflation,
            level::Scarcity, level::ToughEnemies, level::DeadlyEnemies,
            level::DoubleBoss,
        ] {
            assert!(!has_level(0, thr), "A0 should not activate level {}", thr);
        }
    }
}
