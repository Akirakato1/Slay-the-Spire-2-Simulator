//! Treasure room rewards.
//!
//! C# `OneOffSynchronizer.DoTreasureRoomRewards` +
//! `TreasureRoomRelicSynchronizer.BeginRelicPicking` ported to Rust.
//! Standard treasure room grants:
//!   1. Gold 42-52 (uniform via the `rewards` RNG stream).
//!   2. One relic, rolled by rarity then picked uniformly from the
//!      matching pool (using the `treasure_room_relics` RNG stream).
//!      Owned relics are excluded.
//!
//! Rarity weights (mirrors `RelicFactory.RollRarity` exactly):
//!   roll < 0.5    → Common   (50%)
//!   roll < 0.83   → Uncommon (33%)
//!   roll < 1.0    → Rare     (17%)
//!
//! The "Boss chest" (3-relic pick from Boss-rarity pool) is not
//! modeled here yet — it lives in a different code path in C# (post-
//! boss rewards). Add a `BossChest` variant when boss-encounter
//! support lands.

use crate::effects::Effect;
use crate::relic::{self, RelicRarity};
use crate::run_state::RunState;

/// Roll the rarity tier for the next treasure-room relic. Uses the
/// `treasure_room_relics` RNG stream and the C# 0.5 / 0.83 thresholds.
pub fn roll_treasure_rarity(rs: &mut RunState) -> RelicRarity {
    let n = rs.rng_set_mut().treasure_room_relics.next_float(1.0);
    if n < 0.5 {
        RelicRarity::Common
    } else if n < 0.83 {
        RelicRarity::Uncommon
    } else {
        RelicRarity::Rare
    }
}

/// Pick a single relic of the given rarity, excluding ones the player
/// already owns. Returns None if the pool is exhausted (would fall back
/// to RelicFactory.FallbackRelic in C#, which is a degenerate
/// edge case we ignore here — no real run carries every relic of any
/// single rarity).
pub fn pick_treasure_relic(
    rs: &mut RunState,
    player_idx: usize,
    rarity: RelicRarity,
) -> Option<String> {
    let owned: std::collections::HashSet<String> = rs
        .players()
        .get(player_idx)
        .map(|ps| ps.relics.iter().map(|r| r.id.clone()).collect())
        .unwrap_or_default();
    // Treasure-room pool = Shared rarity tier minus owned + character-
    // pool of the player's character. The C# `_sharedGrabBag` is
    // populated at run start from the Shared pool; per-character relics
    // come from a separate grab bag. We approximate by including
    // relics with `pools` containing "Shared" OR the player's character id.
    let character: String = rs
        .players()
        .get(player_idx)
        .map(|ps| ps.character_id.clone())
        .unwrap_or_default();
    let candidates: Vec<&str> = relic::ALL_RELICS
        .iter()
        .filter(|r| r.rarity == rarity)
        .filter(|r| !owned.contains(&r.id))
        .filter(|r| {
            r.pools.iter().any(|p| p == "Shared" || p == &character)
        })
        .map(|r| r.id.as_str())
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let idx = rs.rng_set_mut().treasure_room_relics
        .next_int(candidates.len() as i32) as usize;
    Some(candidates[idx].to_string())
}

/// Open a standard treasure-room chest:
///   1. Grants 42-52 gold via the `rewards` RNG.
///   2. Rolls + offers a single relic via the `treasure_room_relics` RNG.
///
/// Auto-resolve flag on RunState controls whether the relic offer
/// auto-accepts (default) or pauses for an RL agent. Either way, gold
/// lands immediately.
///
/// Mirrors C# `OneOffSynchronizer.DoTreasureRoomRewards` +
/// `TreasureRoomRelicSynchronizer.BeginRelicPicking` flow.
pub fn open_standard_chest(rs: &mut RunState, player_idx: usize) {
    // Step 1: gold. C# uses `player.PlayerRng.Rewards.NextInt(42, 53)`
    // — half-open [42, 53) i.e. inclusive 42-52.
    let gold = rs.players_rng
        .get_mut(player_idx)
        .map(|prng| prng.rewards.next_int_range(42, 53))
        .unwrap_or(42);
    if let Some(ps) = rs.player_state_mut(player_idx) {
        ps.gold += gold;
    }
    // Step 2: roll rarity, pick relic, emit offer.
    let rarity = roll_treasure_rarity(rs);
    let Some(relic_id) = pick_treasure_relic(rs, player_idx, rarity) else {
        return;
    };
    let body = vec![Effect::OfferRelicReward {
        options: vec![relic_id],
        n_min: 1,
        n_max: 1,
        source: Some("TreasureRoom".to_string()),
    }];
    crate::effects::execute_run_state_effects(rs, player_idx, &body);
    // Fire room-entry hooks last so any relics granted by this chest
    // observe the post-entry combat state. The C# flow fires
    // AfterRoomEntered before the chest-open click; we collapse the
    // two events because the simulator doesn't model click timing.
    rs.enter_room("TreasureRoom");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::act::ActId;
    use crate::run_state::PlayerState;

    fn fresh_run_state() -> RunState {
        let player = PlayerState {
            character_id: "Ironclad".to_string(),
            id: 1,
            hp: 80,
            max_hp: 80,
            gold: 99,
            deck: Vec::new(),
            relics: Vec::new(),
            potions: Vec::new(),
            max_potion_slot_count: 3,
        };
        RunState::new("seed", 0, vec![player], vec![ActId::Overgrowth], Vec::new())
    }

    #[test]
    fn roll_rarity_uses_treasure_room_rng_and_buckets_correctly() {
        let mut rs = fresh_run_state();
        // Just exercise the API; can't pin the exact rarity without
        // pinning the RNG sequence (treasure_room_relics is seed-driven
        // off "seed"). Confirm the call doesn't panic and returns one
        // of the 3 valid buckets.
        let r = roll_treasure_rarity(&mut rs);
        assert!(matches!(
            r,
            RelicRarity::Common | RelicRarity::Uncommon | RelicRarity::Rare));
    }

    #[test]
    fn open_chest_grants_gold_in_range_and_a_relic() {
        let mut rs = fresh_run_state();
        let gold_before = rs.players()[0].gold;
        open_standard_chest(&mut rs, 0);
        let gold_after = rs.players()[0].gold;
        let gained = gold_after - gold_before;
        assert!(gained >= 42 && gained <= 52,
            "Treasure gold should be 42-52 (got {})", gained);
        // Auto-resolve is on by default → relic should already be in the
        // player's relic list.
        let relics = &rs.players()[0].relics;
        assert_eq!(relics.len(), 1,
            "Auto-resolve should add one relic to the player");
    }

    #[test]
    fn deferred_chest_pauses_for_agent() {
        let mut rs = fresh_run_state();
        rs.auto_resolve_offers = false;
        open_standard_chest(&mut rs, 0);
        // Gold lands immediately even when deferred.
        assert!(rs.players()[0].gold >= 99 + 42);
        // Relic is staged but not granted.
        assert_eq!(rs.players()[0].relics.len(), 0);
        let offer = rs.pending_offer.as_ref().expect("offer staged");
        assert_eq!(offer.options.len(), 1);
        assert_eq!(offer.source.as_deref(), Some("TreasureRoom"));
    }

    #[test]
    fn open_chest_does_not_grant_duplicate_relic() {
        // Pre-load the player with Akabeko (Common rarity, Shared pool).
        // Run many chest openings — none should re-grant Akabeko.
        let mut rs = fresh_run_state();
        rs.add_relic(0, "Akabeko");
        let initial_relic_count = rs.players()[0].relics.len();
        assert_eq!(initial_relic_count, 1);
        // Open 5 chests; none should produce a second Akabeko.
        for _ in 0..5 {
            open_standard_chest(&mut rs, 0);
        }
        let akabekos: usize = rs.players()[0].relics.iter()
            .filter(|r| r.id == "Akabeko").count();
        assert_eq!(akabekos, 1,
            "Duplicate-relic exclusion failed: {} Akabekos", akabekos);
    }

    #[test]
    fn rarity_weights_are_approximately_correct() {
        // Statistical check: over many rolls, ~50% Common, ~33%
        // Uncommon, ~17% Rare. Use a wide tolerance to avoid flakiness.
        let mut rs = fresh_run_state();
        let mut common = 0u32;
        let mut uncommon = 0u32;
        let mut rare = 0u32;
        for _ in 0..2000 {
            match roll_treasure_rarity(&mut rs) {
                RelicRarity::Common => common += 1,
                RelicRarity::Uncommon => uncommon += 1,
                RelicRarity::Rare => rare += 1,
                _ => panic!("Unexpected rarity from roll_treasure_rarity"),
            }
        }
        let total = (common + uncommon + rare) as f64;
        let p_common = common as f64 / total;
        let p_uncommon = uncommon as f64 / total;
        let p_rare = rare as f64 / total;
        // Expected: 0.50 / 0.33 / 0.17. ±0.05 tolerance.
        assert!((p_common - 0.50).abs() < 0.05,
            "p_common = {:.3} (want ~0.50)", p_common);
        assert!((p_uncommon - 0.33).abs() < 0.05,
            "p_uncommon = {:.3} (want ~0.33)", p_uncommon);
        assert!((p_rare - 0.17).abs() < 0.05,
            "p_rare = {:.3} (want ~0.17)", p_rare);
    }
}
