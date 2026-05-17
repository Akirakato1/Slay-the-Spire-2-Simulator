//! Post-combat card rewards.
//!
//! C# `CardFactory.CreateForCardReward` + `Rewards/CardReward.cs`
//! port. Three rarity-weight profiles depending on what was killed:
//!
//!   - `Normal`  (post-Monster combat):  Common 60% / Uncommon 37% / Rare  3%
//!   - `Elite`   (post-Elite combat):    Common 50% / Uncommon 40% / Rare 10%
//!   - `Boss`    (post-Boss combat):     Rare 100%
//!
//! Pool: character pool ∪ Colorless (Status / Curse / Token / Quest
//! / Basic / Ancient / Event are excluded — the C# filter is
//! `Rarity ∉ {Basic, Ancient, Event} && CanBeGeneratedInCombat`).
//!
//! The MVP uses simplified probabilities; C# layers a `PlayerOdds`
//! system on top that tracks streaks (no-rare-recently → bumps rare
//! chance). PlayerOdds isn't ported yet — when it lands, this module
//! will switch to consulting it. Until then, the per-roll probability
//! is fixed.

use crate::card::{self, CardRarity};
use crate::effects::Effect;
use crate::run_state::RunState;

/// Which combat type generated the card reward. Determines rarity
/// weights. Mirrors C# `CardRarityOddsType` for the post-combat path.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CardRewardKind {
    Normal,
    Elite,
    Boss,
}

/// Roll the rarity for a single card-reward card. Uses the
/// `combat_card_generation` RNG stream (mirrors C# which uses the
/// player's rewards stream for reward rolls; the run-level stream is
/// the closest available analog until PlayerOdds lands).
pub fn roll_card_rarity(rs: &mut RunState, kind: CardRewardKind) -> CardRarity {
    // Scarcity (A7+) shifts the rarity table per C# `CardRarityOdds`:
    //   Normal: RegularRareOdds 0.03 → 0.0149,
    //           regularCommonOdds 0.60 → 0.615.
    //   Elite : EliteRareOdds   0.10 → 0.05,
    //           EliteCommonOdds 0.50 → 0.549.
    // Common-and-Uncommon split is computed from the cumulative
    // threshold (common%, common%+uncommon%) with Rare being the
    // tail. The Uncommon share absorbs the Rare reduction net of the
    // Common bump (matches C# behavior — total still sums to 1.0).
    let scarcity = crate::ascension::has_level(
        rs.ascension(),
        crate::ascension::level::Scarcity,
    );
    let n = rs.rng_set_mut().combat_card_generation.next_float(1.0);
    match kind {
        CardRewardKind::Boss => CardRarity::Rare,
        CardRewardKind::Normal => {
            let (common, common_plus_uncommon) = if scarcity {
                (0.615, 1.0 - 0.0149)
            } else {
                (0.60, 0.97)
            };
            if n < common { CardRarity::Common }
            else if n < common_plus_uncommon { CardRarity::Uncommon }
            else { CardRarity::Rare }
        }
        CardRewardKind::Elite => {
            let (common, common_plus_uncommon) = if scarcity {
                (0.549, 1.0 - 0.05)
            } else {
                (0.50, 0.90)
            };
            if n < common { CardRarity::Common }
            else if n < common_plus_uncommon { CardRarity::Uncommon }
            else { CardRarity::Rare }
        }
    }
}

/// Cards whose C# `MultiplayerConstraint` overrides to
/// `MultiplayerOnly`. The simulator targets single-player runs, so
/// these MUST be excluded from every card-generation pool (post-
/// combat rewards, shop, events, transforms). Hardcoded list to
/// avoid the cost of a per-card extractor pass for ~10 cards across
/// all character pools; entries below are confirmed by grepping
/// `CardMultiplayerConstraint.MultiplayerOnly` in the C# source.
pub fn is_multiplayer_only(card_id: &str) -> bool {
    matches!(card_id,
        // Ironclad
        | "Tank" | "DemonicShield"
        // Silent
        | "Flanking" | "Sneaky"
        // Defect / Regent / Necrobinder: confirm via extractor pass;
        // add ids here as they surface.
    )
}

#[cfg(test)]
mod multiplayer_filter_tests {
    use super::*;

    /// Multiplayer-only cards must NOT appear in 1000 Normal-tier
    /// card-reward rolls across either character pool.
    #[test]
    fn multiplayer_only_cards_never_in_reward_pool() {
        let mut rs = crate::run_state::RunState::start_run(
            "MP", 0, "Ironclad",
            vec![crate::act::ActId::Overgrowth], Vec::new(),
        ).unwrap();
        for _ in 0..1000 {
            let opts = build_card_reward_options(
                &mut rs, 0, CardRewardKind::Normal, 3,
            );
            for id in &opts {
                assert!(!is_multiplayer_only(id),
                    "{id} (multiplayer-only) leaked into reward pool");
            }
        }
    }
}

/// Pick a single card of the given rarity from the player's pool.
/// Pool = (player's character pool ∪ "Colorless") with playable
/// rarities only. Excludes any id already in `exclude` (so the 3 card
/// rewards don't duplicate each other).
fn pick_card_of_rarity(
    rs: &mut RunState,
    player_idx: usize,
    rarity: CardRarity,
    exclude: &std::collections::HashSet<String>,
) -> Option<String> {
    let character = rs
        .players()
        .get(player_idx)
        .map(|ps| ps.character_id.clone())
        .unwrap_or_default();
    let candidates: Vec<&str> = card::ALL_CARDS
        .iter()
        .filter(|c| c.rarity == rarity)
        .filter(|c| c.pool == character || c.pool == "Colorless")
        .filter(|c| !exclude.contains(&c.id))
        // C# CardFactory.CardSourceFilter:
        //   c.CanBeGeneratedInCombat && Rarity ∉ {Basic, Ancient, Event}
        // Basic/Ancient/Event are already filtered out by the rarity
        // gate. CanBeGeneratedInCombat is a per-card boolean; we
        // approximate by excluding Quest cards (which all override
        // CanBeGenerated*=false) and the deprecated stub card.
        .filter(|c| c.id != "DeprecatedCard")
        // Multiplayer-only cards are excluded entirely — this sim
        // targets single-player runs.
        .filter(|c| !is_multiplayer_only(&c.id))
        .map(|c| c.id.as_str())
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let idx = rs.rng_set_mut().combat_card_generation
        .next_int(candidates.len() as i32) as usize;
    Some(candidates[idx].to_string())
}

/// Build the N-card option list for a post-combat reward. Default
/// count is 3; some relics modify this (Question Card → 4, etc.) —
/// the caller passes `count` explicitly to keep this function pure.
pub fn build_card_reward_options(
    rs: &mut RunState,
    player_idx: usize,
    kind: CardRewardKind,
    count: i32,
) -> Vec<String> {
    let count = count.max(0) as usize;
    let mut picks: Vec<String> = Vec::with_capacity(count);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for _ in 0..count {
        let rarity = roll_card_rarity(rs, kind);
        let Some(id) = pick_card_of_rarity(rs, player_idx, rarity, &seen) else {
            break;
        };
        seen.insert(id.clone());
        picks.push(id);
    }
    picks
}

/// Emit a post-combat card-reward offer. Three cards by default;
/// `n_min=0` (skip allowed), `n_max=1` (pick at most one).
///
/// Mirrors C# `RewardsCmd.OfferForCombatEnd` (card-reward portion).
/// Auto-resolve takes 0 picks (skip); deferred mode pauses for the
/// agent.
pub fn offer_post_combat_card_reward(
    rs: &mut RunState,
    player_idx: usize,
    kind: CardRewardKind,
) {
    // Standard card-reward count is 3. Some relics bump this; that
    // overlay lands when the ModifyHandDraw-style modifier-hook layer
    // generalization comes in. For now, fixed 3.
    let options = build_card_reward_options(rs, player_idx, kind, 3);
    if options.is_empty() {
        return;
    }
    let source = match kind {
        CardRewardKind::Normal => "PostMonsterReward",
        CardRewardKind::Elite => "PostEliteReward",
        CardRewardKind::Boss => "PostBossReward",
    };
    let body = vec![Effect::OfferCardReward {
        options,
        n_min: 0,
        n_max: 1,
        source: Some(source.to_string()),
    }];
    crate::effects::execute_run_state_effects(rs, player_idx, &body);
}

/// Pool-aware sibling of `pick_card_of_rarity`. Restricts the
/// candidate set to a specific pool reference (CharacterAny / Colorless /
/// CharacterAttack / CharacterSkill / CharacterPower). Used by events
/// that roll a card-reward from a *specific* pool — e.g. BrainLeech
/// Rip (Colorless only) — distinct from the normal post-combat reward
/// that draws from character ∪ Colorless.
fn pick_card_of_rarity_from_pool(
    rs: &mut RunState,
    player_idx: usize,
    rarity: CardRarity,
    pool_ref: &crate::effects::CardPoolRef,
    exclude: &std::collections::HashSet<String>,
) -> Option<String> {
    use crate::effects::CardPoolRef;
    let character = rs
        .players()
        .get(player_idx)
        .map(|ps| ps.character_id.clone())
        .unwrap_or_default();
    let type_filter: Option<crate::card::CardType> = match pool_ref {
        CardPoolRef::CharacterAttack => Some(crate::card::CardType::Attack),
        CardPoolRef::CharacterSkill => Some(crate::card::CardType::Skill),
        CardPoolRef::CharacterPower => Some(crate::card::CardType::Power),
        _ => None,
    };
    let pool_match = |c: &card::CardData| -> bool {
        match pool_ref {
            CardPoolRef::Colorless => c.pool == "Colorless",
            CardPoolRef::CharacterAny
            | CardPoolRef::CharacterAttack
            | CardPoolRef::CharacterSkill
            | CardPoolRef::CharacterPower => c.pool == character,
        }
    };
    let candidates: Vec<&str> = card::ALL_CARDS
        .iter()
        .filter(|c| c.rarity == rarity)
        .filter(|c| pool_match(c))
        .filter(|c| type_filter.map_or(true, |t| c.card_type == t))
        .filter(|c| !exclude.contains(&c.id))
        .filter(|c| c.id != "DeprecatedCard")
        .filter(|c| !is_multiplayer_only(&c.id))
        .map(|c| c.id.as_str())
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let idx = rs.rng_set_mut().combat_card_generation
        .next_int(candidates.len() as i32) as usize;
    Some(candidates[idx].to_string())
}

/// Pool-targeted version of `build_card_reward_options`. Used by
/// event-time `OfferCardRewardFromPool` to materialize the N-card
/// option list at offer-emit time. Rolls a per-card rarity via
/// CardRewardKind::Normal odds (default for non-combat events;
/// `ForNonCombatWithDefaultOdds` in C#).
pub fn build_card_options_from_pool(
    rs: &mut RunState,
    player_idx: usize,
    pool_ref: &crate::effects::CardPoolRef,
    count: i32,
) -> Vec<String> {
    let count = count.max(0) as usize;
    let mut picks: Vec<String> = Vec::with_capacity(count);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Fallback order if the rolled rarity is empty in this pool —
    // e.g. Colorless has no Commons. Tries the rolled rarity first,
    // then the alternates. Matches the C# behavior of
    // `ForNonCombatWithDefaultOdds` which redistributes weight when
    // a tier is empty for the configured pool.
    let fallback_order = |primary: CardRarity| -> [CardRarity; 3] {
        match primary {
            CardRarity::Common => [CardRarity::Common, CardRarity::Uncommon, CardRarity::Rare],
            CardRarity::Uncommon => [CardRarity::Uncommon, CardRarity::Common, CardRarity::Rare],
            CardRarity::Rare => [CardRarity::Rare, CardRarity::Uncommon, CardRarity::Common],
            // Should never occur for combat/event rolls, but cover the case.
            _ => [CardRarity::Common, CardRarity::Uncommon, CardRarity::Rare],
        }
    };
    for _ in 0..count {
        let primary = roll_card_rarity(rs, CardRewardKind::Normal);
        let mut got: Option<String> = None;
        for r in fallback_order(primary) {
            if let Some(id) = pick_card_of_rarity_from_pool(rs, player_idx, r, pool_ref, &seen) {
                got = Some(id);
                break;
            }
        }
        let Some(id) = got else { break };
        seen.insert(id.clone());
        picks.push(id);
    }
    picks
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
            gold: 0,
            deck: Vec::new(),
            relics: Vec::new(),
            potions: Vec::new(),
            max_potion_slot_count: 3,
            card_shop_removals_used: 0,
        };
        RunState::new("seed", 0, vec![player], vec![ActId::Overgrowth], Vec::new())
    }

    #[test]
    fn boss_reward_is_always_rare() {
        let mut rs = fresh_run_state();
        for _ in 0..50 {
            assert_eq!(roll_card_rarity(&mut rs, CardRewardKind::Boss),
                CardRarity::Rare);
        }
    }

    #[test]
    fn normal_reward_distribution_matches_60_37_3() {
        // Statistical check: 2000 rolls, ~60% Common / 37% Uncommon / 3% Rare.
        let mut rs = fresh_run_state();
        let mut c = 0u32; let mut u = 0u32; let mut r = 0u32;
        for _ in 0..2000 {
            match roll_card_rarity(&mut rs, CardRewardKind::Normal) {
                CardRarity::Common => c += 1,
                CardRarity::Uncommon => u += 1,
                CardRarity::Rare => r += 1,
                _ => panic!("unexpected rarity"),
            }
        }
        let total = (c + u + r) as f64;
        let p_c = c as f64 / total;
        let p_u = u as f64 / total;
        let p_r = r as f64 / total;
        assert!((p_c - 0.60).abs() < 0.05, "Common p={:.3} (want ~0.60)", p_c);
        assert!((p_u - 0.37).abs() < 0.05, "Uncommon p={:.3} (want ~0.37)", p_u);
        assert!((p_r - 0.03).abs() < 0.03, "Rare p={:.3} (want ~0.03)", p_r);
    }

    #[test]
    fn elite_reward_distribution_matches_50_40_10() {
        let mut rs = fresh_run_state();
        let mut c = 0u32; let mut u = 0u32; let mut r = 0u32;
        for _ in 0..2000 {
            match roll_card_rarity(&mut rs, CardRewardKind::Elite) {
                CardRarity::Common => c += 1,
                CardRarity::Uncommon => u += 1,
                CardRarity::Rare => r += 1,
                _ => panic!("unexpected rarity"),
            }
        }
        let total = (c + u + r) as f64;
        let p_c = c as f64 / total;
        let p_u = u as f64 / total;
        let p_r = r as f64 / total;
        assert!((p_c - 0.50).abs() < 0.05, "Common p={:.3} (want ~0.50)", p_c);
        assert!((p_u - 0.40).abs() < 0.05, "Uncommon p={:.3} (want ~0.40)", p_u);
        assert!((p_r - 0.10).abs() < 0.04, "Rare p={:.3} (want ~0.10)", p_r);
    }

    #[test]
    fn three_card_reward_has_three_distinct_options() {
        let mut rs = fresh_run_state();
        let options = build_card_reward_options(&mut rs, 0, CardRewardKind::Normal, 3);
        assert_eq!(options.len(), 3, "should generate exactly 3");
        // All distinct.
        let mut seen = std::collections::HashSet::new();
        for id in &options {
            assert!(seen.insert(id.clone()),
                "duplicate option {} in {:?}", id, options);
        }
    }

    #[test]
    fn options_drawn_only_from_character_and_colorless_pools() {
        let mut rs = fresh_run_state();
        for _ in 0..30 {
            let options = build_card_reward_options(&mut rs, 0, CardRewardKind::Normal, 3);
            for id in &options {
                let data = card::by_id(id).expect("real card");
                assert!(data.pool == "Ironclad" || data.pool == "Colorless",
                    "{} is in pool {:?}, expected Ironclad/Colorless",
                    id, data.pool);
            }
        }
    }

    #[test]
    fn auto_resolve_skip_default_does_not_add_to_deck() {
        let mut rs = fresh_run_state();
        offer_post_combat_card_reward(&mut rs, 0, CardRewardKind::Normal);
        // Auto-resolve picks 0 (n_min=0 → skip).
        assert_eq!(rs.players()[0].deck.len(), 0);
    }

    #[test]
    fn deferred_offer_lets_agent_pick() {
        let mut rs = fresh_run_state();
        rs.auto_resolve_offers = false;
        offer_post_combat_card_reward(&mut rs, 0, CardRewardKind::Elite);
        // Snapshot the offer before the resolve call mutates rs.
        let (chosen_id, options_len, n_min, n_max, source) = {
            let offer = rs.pending_offer.as_ref().expect("offer staged");
            (offer.options[1].clone(),
             offer.options.len(),
             offer.n_min, offer.n_max,
             offer.source.clone())
        };
        assert_eq!(options_len, 3);
        assert_eq!(n_min, 0);
        assert_eq!(n_max, 1);
        assert_eq!(source.as_deref(), Some("PostEliteReward"));
        crate::effects::resolve_run_state_offer(&mut rs, &[1])
            .expect("pick succeeds");
        assert_eq!(rs.players()[0].deck.len(), 1);
        assert_eq!(rs.players()[0].deck[0].id, chosen_id);
    }

    #[test]
    fn boss_reward_options_are_all_rare() {
        let mut rs = fresh_run_state();
        let options = build_card_reward_options(&mut rs, 0, CardRewardKind::Boss, 3);
        for id in &options {
            let data = card::by_id(id).expect("real card");
            assert_eq!(data.rarity, CardRarity::Rare,
                "{} should be Rare for Boss reward", id);
        }
    }
}
