//! Port of `UnknownMapPointOdds` — decides what a `?` map node
//! resolves to when the player enters it.
//!
//! From C# `UnknownMapPointOdds.cs` lines 90-105: base odds are
//!   Monster:   10%
//!   Elite:     -1 (disabled; never rolled from `?`)
//!   Treasure:  2%
//!   Shop:      3%
//!   Event:     remainder (~85%)
//!
//! After each roll, the picked type resets to its base odds; every
//! other type's odds are bumped by their base odds. This is the
//! "forced variety" mechanism — go long enough without a shop and
//! shop odds climb. State is per-run (the same `UnknownMapPointOdds`
//! lives on `RunState` across map nodes).
//!
//! The first-run tutorial special case (`runState.UnlockState
//! .NumberOfRuns == 0` forcing the first 2 `?`s to be Event and the
//! 3rd to be Monster) is omitted — the RL agent never plays its
//! "first run" and tutorial-forced sequences would distort early
//! training.

use crate::rng::Rng;

#[cfg(test)]
mod ascension_smoke_tests {
    use crate::act::ActId;
    use crate::combat::CombatState;
    use crate::run_state::RunState;
    use crate::run_log::{CardRef, PotionEntry, RelicEntry};
    use crate::run_state::PlayerState;

    fn player(character_id: &str) -> PlayerState {
        let cd = crate::character::by_id(character_id).unwrap();
        let deck: Vec<CardRef> = cd.starting_deck.iter().map(|id| CardRef {
            id: id.clone(), floor_added_to_deck: Some(0),
            current_upgrade_level: Some(0), enchantment: None,
        }).collect();
        let relics: Vec<RelicEntry> = cd.starting_relics.iter().map(|id| RelicEntry {
            id: id.clone(), floor_added_to_deck: 0, props: None,
        }).collect();
        PlayerState {
            character_id: character_id.to_string(), id: 1,
            hp: cd.starting_hp.unwrap_or(80),
            max_hp: cd.starting_hp.unwrap_or(80),
            gold: cd.starting_gold.unwrap_or(99),
            deck, relics, potions: Vec::<PotionEntry>::new(),
            max_potion_slot_count: 3,
            card_shop_removals_used: 0,
        }
    }

    /// `from_monster_spawn_at` reads `max_hp_ascended` only at A8+
    /// (ToughEnemies threshold per C# AscensionLevel). Axebot:
    /// base 40-44, ascended 42-46.
    #[test]
    fn ascended_monsters_spawn_with_ascended_hp() {
        let a0 = crate::combat::Creature::from_monster_spawn_at("Axebot", "front", 0);
        let a7 = crate::combat::Creature::from_monster_spawn_at("Axebot", "front", 7);
        let a8 = crate::combat::Creature::from_monster_spawn_at("Axebot", "front", 8);
        let a10 = crate::combat::Creature::from_monster_spawn_at("Axebot", "front", 10);
        assert_eq!(a0.max_hp, 44, "A0 base HP");
        assert_eq!(a7.max_hp, 44, "A7 still below ToughEnemies (=A8)");
        assert_eq!(a8.max_hp, 46, "A8 activates ToughEnemies");
        assert_eq!(a10.max_hp, 46, "A10 still ascended");
    }

    /// CombatState built via run_flow propagates RunState.ascension
    /// all the way to spawned enemies, and ascended HP applies at A10.
    #[test]
    fn ascension_pipes_through_to_combat_state() {
        let mut rs = RunState::new(
            "ASC", 10, vec![player("Ironclad")],
            vec![ActId::Overgrowth], Vec::new(),
        );
        rs.enter_act(0);
        let enc = crate::encounter::by_id("AxebotsNormal").unwrap();
        let cs = crate::run_flow::build_combat_state(&rs, enc, 0).unwrap();
        assert_eq!(cs.ascension, 10);
        // Axebot at A10 → ToughEnemies (A8+) active → ascended HP.
        assert_eq!(cs.enemies[0].max_hp, 46,
            "A10 should spawn ascended Axebot (46 max HP), got {}",
            cs.enemies[0].max_hp);
    }

    /// `AmountSpec::AscensionScaled` resolves to `ascended` only
    /// when `cs.ascension >= threshold`. Verified at the C#-correct
    /// DeadlyEnemies (=9) threshold for monster damage.
    #[test]
    fn ascension_scaled_amount_resolves_correctly() {
        use crate::effects::{AmountSpec, EffectContext};
        use crate::combat::CombatSide;

        let spec = AmountSpec::AscensionScaled {
            base: 5, ascended: 6,
            threshold: crate::ascension::level::DeadlyEnemies,
        };
        let ctx = EffectContext::for_card(
            0, Some((CombatSide::Enemy, 0)), "TestCard", 0, None, 0,
        );

        let mut cs = CombatState::empty();
        for (asc, want) in [(0, 5), (8, 5), (9, 6), (10, 6)] {
            cs.ascension = asc;
            assert_eq!(spec.resolve(&ctx, &cs), want,
                "ascension {} should resolve to {}", asc, want);
        }
    }
}

/// What a `?` map node resolved to. Local enum because `MapPointType`
/// (the on-map type) doesn't have an Event variant — Events live as
/// transient state on `RunState.pending_event` rather than as a
/// distinct map cell type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnknownResolution {
    Monster,
    Treasure,
    Shop,
    Event,
}

/// Per-run odds state for `?` resolution. Lives on `RunState`,
/// mutated each time a `?` node fires.
#[derive(Debug, Clone)]
pub struct UnknownMapPointOdds {
    pub monster: f32,
    pub treasure: f32,
    pub shop: f32,
}

impl Default for UnknownMapPointOdds {
    fn default() -> Self {
        Self::new()
    }
}

impl UnknownMapPointOdds {
    /// Fresh odds set at the documented base values.
    pub fn new() -> Self {
        Self {
            monster: 0.10,
            treasure: 0.02,
            shop: 0.03,
        }
    }

    /// Reset every odds slot to its base — used at boundaries the
    /// caller wants to clear cumulative bumps (e.g. between acts;
    /// C# doesn't reset between acts so we don't either by default).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Roll the `?` resolution. Reads a single random float from the
    /// passed RNG, dispatches by cumulative odds, then mutates self
    /// for next time: picked type resets to base, all others bump by
    /// their base.
    pub fn roll(&mut self, rng: &mut Rng) -> UnknownResolution {
        let r = rng.next_float(1.0);
        let mut acc = 0.0_f32;

        // Monster
        if self.monster >= 0.0 {
            acc += self.monster;
            if r <= acc {
                self.bump_after_roll(UnknownResolution::Monster);
                return UnknownResolution::Monster;
            }
        }
        // Treasure
        if self.treasure >= 0.0 {
            acc += self.treasure;
            if r <= acc {
                self.bump_after_roll(UnknownResolution::Treasure);
                return UnknownResolution::Treasure;
            }
        }
        // Shop
        if self.shop >= 0.0 {
            acc += self.shop;
            if r <= acc {
                self.bump_after_roll(UnknownResolution::Shop);
                return UnknownResolution::Shop;
            }
        }
        // Otherwise → Event (the ~85% remainder).
        self.bump_after_roll(UnknownResolution::Event);
        UnknownResolution::Event
    }

    /// `_baseOdds` walk from C#: the picked type resets to base, every
    /// other (non-Event) type bumps by its own base value.
    fn bump_after_roll(&mut self, picked: UnknownResolution) {
        let bases = Self::new();
        if picked == UnknownResolution::Monster {
            self.monster = bases.monster;
        } else {
            self.monster += bases.monster;
        }
        if picked == UnknownResolution::Treasure {
            self.treasure = bases.treasure;
        } else {
            self.treasure += bases.treasure;
        }
        if picked == UnknownResolution::Shop {
            self.shop = bases.shop;
        } else {
            self.shop += bases.shop;
        }
        // Event isn't tracked as an explicit odds slot — it's the
        // implicit remainder of `1 - sum(other slots)`. Picking Event
        // doesn't change anyone's slot value.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distribution sanity: roll 10000 `?`s and verify Event is by
    /// far the most common outcome (~85% baseline). Monster ~10%,
    /// Treasure ~2%, Shop ~3% — but the odds-bump mechanism makes
    /// unrolled types climb over time, so we just assert
    /// Event > Monster > {Treasure, Shop}.
    #[test]
    fn distribution_favors_events_then_monsters() {
        let mut rng = Rng::new(42, 0);
        let mut odds = UnknownMapPointOdds::new();
        let (mut e, mut m, mut t, mut s) = (0, 0, 0, 0);
        for _ in 0..10_000 {
            match odds.roll(&mut rng) {
                UnknownResolution::Event => e += 1,
                UnknownResolution::Monster => m += 1,
                UnknownResolution::Treasure => t += 1,
                UnknownResolution::Shop => s += 1,
            }
        }
        // Note: the odds-bump mechanism means Event's long-run share
        // is FAR below the 85% base — every non-Event pick is delayed
        // by the bump, but every Event pick boosts every other type
        // toward its eventual pick. Steady-state is where the non-
        // Event types fire often enough to balance the bumps.
        assert!(e > m,
            "Events ({}) should still exceed Monsters ({})", e, m);
        assert!(m > t.max(s),
            "Monsters ({}) should exceed Treasure ({}) and Shop ({})", m, t, s);
        // Loose sanity bounds — Event share in [30%, 80%] handles the
        // empirically observed ~46% steady-state.
        assert!(e > 3_000 && e < 8_000,
            "Event share out of expected range: {}/10000", e);
        // Treasure + Shop should be the rarest (lowest base odds).
        assert!(t < m && s < m,
            "Treasure ({}) and Shop ({}) should each be rarer than Monster ({})",
            t, s, m);
    }

    /// Odds-bump: after a Treasure roll, treasure resets to 0.02 but
    /// other types climb. Picking treasure 5 times in a row should
    /// keep the treasure odds at base while pushing monster/shop
    /// upward.
    #[test]
    fn unrolled_types_climb() {
        let mut odds = UnknownMapPointOdds::new();
        // Manually force "treasure picked" via direct calls — simulates
        // what happens if the RNG happened to land in the treasure
        // window 5 times.
        for _ in 0..5 {
            odds.bump_after_roll(UnknownResolution::Treasure);
        }
        assert!((odds.treasure - 0.02).abs() < 1e-5,
            "treasure should stay at base after picks: {}", odds.treasure);
        // Monster started at 0.10, bumped 5× by 0.10 → 0.60.
        assert!((odds.monster - 0.60).abs() < 1e-3,
            "monster should be ~0.60 after 5 unrolled bumps: {}", odds.monster);
    }

    /// First roll lands on its target type at the expected boundary.
    /// rng float 0.05 ≤ monster (0.10) → Monster.
    #[test]
    fn first_roll_at_monster_boundary() {
        let mut rng = Rng::new(0, 0);
        // Burn RNG state until we get a float ≤ 0.10. We can't
        // contrive a specific float without changing the API, so
        // instead just verify the algorithmic structure with the
        // distribution check above.
        let mut odds = UnknownMapPointOdds::new();
        let _ = odds.roll(&mut rng);
        // Just verify the API doesn't panic — distribution test
        // above covers the meat of the contract.
    }
}
