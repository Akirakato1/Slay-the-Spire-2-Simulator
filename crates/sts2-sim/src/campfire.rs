//! Campfire (rest site) actions.
//!
//! C# port of `Core/Entities/RestSite/RestSiteOption` family. Each
//! campfire visit offers a set of options driven by which relics the
//! player owns. The MVP supports the three universal options:
//!
//!   - **Rest**: heal 30% max HP (rounded down). Always available.
//!   - **Smith**: pick an upgradable card from the master deck and
//!     upgrade it. Always available; gated only by "any upgradable
//!     card exists in deck".
//!   - **Toke**: pick any card from the master deck and remove it.
//!     Gated by `PeacePipe` relic in real game; we expose it
//!     unconditionally for the MVP and let the relic gate land when
//!     `available_rest_site_options` becomes a real C# port.
//!
//! Relic-conditional options (Lift via Girya, Dig via Shovel,
//! HatchEgg via ByrdonisEgg in deck, etc.) are deferred — they
//! plug into a future `gather_rest_site_options(rs, player_idx)`
//! that consults each owned relic's `TryModifyRestSiteOptions`.

use crate::run_state::{DeckActionKind, PendingDeckAction, RunState};

/// Rest: heal 30% of max HP, rounded down. Mirrors C#
/// `RestRestSiteOption.OnChosen` (uses `Math.Floor`).
pub fn rest(rs: &mut RunState, player_idx: usize) {
    let Some(ps) = rs.player_state_mut(player_idx) else { return };
    let heal = ((ps.max_hp as f32) * 0.30).floor() as i32;
    ps.hp = (ps.hp + heal).min(ps.max_hp);
}

/// Stage a Smith choice: pick any upgradable card from the master
/// deck and upgrade it. Eligibility = `current_upgrade_level < max`
/// AND `max_upgrade_level > 0`. If no card is eligible, the option
/// is a no-op (mirrors C# `SmithRestSiteOption.CanBeChosen → false`
/// → option simply isn't offered in the UI).
///
/// Auto-resolve picks the first eligible card; deferred mode pauses
/// for the agent.
pub fn smith(rs: &mut RunState, player_idx: usize) {
    let eligible = upgradable_deck_indices(rs, player_idx);
    if eligible.is_empty() {
        return;
    }
    stage_deck_action(rs, player_idx, DeckActionKind::Upgrade, eligible, "Smith");
}

/// Stage a Toke choice: pick any card from the master deck and
/// remove it. Eligibility = any card NOT marked unremovable
/// (Necronomicurse / Curse-of-the-Bell etc. have removable=false
/// flags in C#; we approximate by excluding Curse-rarity cards,
/// which captures most unremovable curses). Refine when the
/// per-card `Removable` flag ports.
pub fn toke(rs: &mut RunState, player_idx: usize) {
    let eligible = removable_deck_indices(rs, player_idx);
    if eligible.is_empty() {
        return;
    }
    stage_deck_action(rs, player_idx, DeckActionKind::Remove, eligible, "Toke");
}

fn upgradable_deck_indices(rs: &RunState, player_idx: usize) -> Vec<usize> {
    let Some(ps) = rs.players().get(player_idx) else { return Vec::new() };
    ps.deck.iter().enumerate().filter_map(|(i, card)| {
        let data = crate::card::by_id(&card.id)?;
        if data.max_upgrade_level <= 0 {
            return None;
        }
        let cur = card.current_upgrade_level.unwrap_or(0);
        if cur < data.max_upgrade_level { Some(i) } else { None }
    }).collect()
}

fn removable_deck_indices(rs: &RunState, player_idx: usize) -> Vec<usize> {
    let Some(ps) = rs.players().get(player_idx) else { return Vec::new() };
    ps.deck.iter().enumerate().filter_map(|(i, card)| {
        let data = crate::card::by_id(&card.id)?;
        // Approximation: exclude Curse-rarity cards as a proxy for
        // C#'s Removable=false flag. Most unremovable cards are
        // curses (Ascender's Bane, Curse of the Bell, Necronomicurse).
        // Real game has a per-card Removable bool we don't store yet.
        if matches!(data.rarity, crate::card::CardRarity::Curse) {
            return None;
        }
        Some(i)
    }).collect()
}

fn stage_deck_action(
    rs: &mut RunState,
    player_idx: usize,
    action: DeckActionKind,
    eligible: Vec<usize>,
    source: &str,
) {
    if rs.auto_resolve_offers {
        // Auto-pick first eligible index.
        let pick = eligible[0];
        apply_deck_action(rs, player_idx, &action, pick);
    } else {
        rs.pending_deck_action = Some(PendingDeckAction {
            action,
            player_idx,
            eligible_indices: eligible,
            n_min: 1,
            n_max: 1,
            source: Some(source.to_string()),
        });
    }
}

fn apply_deck_action(
    rs: &mut RunState,
    player_idx: usize,
    action: &DeckActionKind,
    deck_idx: usize,
) {
    let Some(ps) = rs.player_state_mut(player_idx) else { return };
    if deck_idx >= ps.deck.len() {
        return;
    }
    match action {
        DeckActionKind::Upgrade => {
            let card = &mut ps.deck[deck_idx];
            let cur = card.current_upgrade_level.unwrap_or(0);
            card.current_upgrade_level = Some(cur + 1);
        }
        DeckActionKind::Remove => {
            ps.deck.remove(deck_idx);
        }
    }
}

/// Resolve a pending deck-action choice. Mirrors
/// `resolve_run_state_offer` but for the deck-action path. `picks`
/// must reference `eligible_indices` slot positions (NOT raw deck
/// indices — the agent picks from the eligibility list).
pub fn resolve_pending_deck_action(
    rs: &mut RunState,
    picks: &[usize],
) -> Result<(), String> {
    let Some(action) = rs.pending_deck_action.take() else {
        return Err("no pending deck action".to_string());
    };
    let count = picks.len() as i32;
    if count < action.n_min || count > action.n_max {
        rs.pending_deck_action = Some(action.clone());
        return Err(format!(
            "pick count {} outside [{}, {}]",
            count, action.n_min, action.n_max));
    }
    let mut seen = std::collections::HashSet::new();
    for &i in picks {
        if i >= action.eligible_indices.len() {
            rs.pending_deck_action = Some(action.clone());
            return Err(format!(
                "pick index {} out of range (eligible.len = {})",
                i, action.eligible_indices.len()));
        }
        if !seen.insert(i) {
            rs.pending_deck_action = Some(action.clone());
            return Err(format!("duplicate pick index {}", i));
        }
    }
    // Apply picks in descending deck-index order so removals don't
    // invalidate earlier indices.
    let mut deck_indices: Vec<usize> = picks.iter()
        .map(|&i| action.eligible_indices[i])
        .collect();
    deck_indices.sort_by(|a, b| b.cmp(a));
    for di in deck_indices {
        apply_deck_action(rs, action.player_idx, &action.action, di);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::act::ActId;
    use crate::run_log::CardRef;
    use crate::run_state::PlayerState;

    fn rs_with_deck(deck: Vec<(&str, i32)>) -> RunState {
        let player = PlayerState {
            character_id: "Ironclad".to_string(),
            id: 1,
            hp: 50, max_hp: 80, gold: 0,
            deck: deck.iter().map(|(id, up)| CardRef {
                id: id.to_string(),
                floor_added_to_deck: None,
                current_upgrade_level: if *up > 0 { Some(*up) } else { None },
                enchantment: None,
            }).collect(),
            relics: Vec::new(),
            potions: Vec::new(),
            max_potion_slot_count: 3,
        };
        RunState::new("seed", 0, vec![player], vec![ActId::Overgrowth], Vec::new())
    }

    #[test]
    fn rest_heals_30_percent_floor() {
        let mut rs = rs_with_deck(vec![]);
        // 80 max, currently 50 → +24 → 74.
        rest(&mut rs, 0);
        assert_eq!(rs.players()[0].hp, 74);
    }

    #[test]
    fn rest_caps_at_max_hp() {
        let mut rs = rs_with_deck(vec![]);
        rs.player_state_mut(0).unwrap().hp = 75;
        rest(&mut rs, 0); // +24 would go to 99 but caps at 80
        assert_eq!(rs.players()[0].hp, 80);
    }

    #[test]
    fn smith_auto_upgrades_first_eligible_card() {
        let mut rs = rs_with_deck(vec![
            ("StrikeIronclad", 0),
            ("DefendIronclad", 0),
        ]);
        smith(&mut rs, 0);
        assert_eq!(rs.players()[0].deck[0].current_upgrade_level, Some(1),
            "First eligible card should be upgraded under auto-resolve");
        assert_eq!(rs.players()[0].deck[1].current_upgrade_level, None);
    }

    #[test]
    fn smith_skips_fully_upgraded_cards() {
        let mut rs = rs_with_deck(vec![
            ("StrikeIronclad", 1), // already at max (1)
            ("DefendIronclad", 0),
        ]);
        smith(&mut rs, 0);
        // Strike already upgraded → first eligible is Defend.
        assert_eq!(rs.players()[0].deck[0].current_upgrade_level, Some(1),
            "Strike was already upgraded");
        assert_eq!(rs.players()[0].deck[1].current_upgrade_level, Some(1),
            "Defend should now be upgraded");
    }

    #[test]
    fn smith_noop_when_no_upgradable_cards() {
        let mut rs = rs_with_deck(vec![
            ("StrikeIronclad", 1),
            ("DefendIronclad", 1),
        ]);
        smith(&mut rs, 0);
        // Both already at max → no change, no pending action.
        assert_eq!(rs.players()[0].deck[0].current_upgrade_level, Some(1));
        assert_eq!(rs.players()[0].deck[1].current_upgrade_level, Some(1));
        assert!(rs.pending_deck_action.is_none());
    }

    #[test]
    fn smith_deferred_pauses_and_resolves() {
        let mut rs = rs_with_deck(vec![
            ("StrikeIronclad", 0),
            ("DefendIronclad", 0),
            ("Anger", 0),
        ]);
        rs.auto_resolve_offers = false;
        smith(&mut rs, 0);
        let pending = rs.pending_deck_action.as_ref().expect("staged");
        assert_eq!(pending.action, DeckActionKind::Upgrade);
        assert_eq!(pending.eligible_indices.len(), 3);
        // Agent picks index 1 in the eligible list → deck[1] (Defend).
        resolve_pending_deck_action(&mut rs, &[1]).expect("resolve");
        assert_eq!(rs.players()[0].deck[1].current_upgrade_level, Some(1));
        assert_eq!(rs.players()[0].deck[0].current_upgrade_level, None);
        assert_eq!(rs.players()[0].deck[2].current_upgrade_level, None);
    }

    #[test]
    fn toke_auto_removes_first_eligible_card() {
        let mut rs = rs_with_deck(vec![
            ("StrikeIronclad", 0),
            ("DefendIronclad", 0),
        ]);
        toke(&mut rs, 0);
        let deck = &rs.players()[0].deck;
        assert_eq!(deck.len(), 1, "One card should be removed");
        assert_eq!(deck[0].id, "DefendIronclad",
            "Strike (index 0) should have been removed");
    }

    #[test]
    fn toke_skips_curse_cards() {
        let mut rs = rs_with_deck(vec![
            ("Regret", 0),  // Curse (unremovable)
            ("StrikeIronclad", 0),
            ("AscendersBane", 0),  // Curse
            ("Anger", 0),
        ]);
        rs.auto_resolve_offers = false;
        toke(&mut rs, 0);
        let pending = rs.pending_deck_action.as_ref().expect("staged");
        // Eligible deck indices = [1, 3] (Strike + Anger).
        assert_eq!(pending.eligible_indices, vec![1, 3]);
    }

    #[test]
    fn toke_noop_with_only_curses() {
        let mut rs = rs_with_deck(vec![
            ("Regret", 0),
            ("AscendersBane", 0),
        ]);
        toke(&mut rs, 0);
        // No eligible cards → nothing happens.
        assert_eq!(rs.players()[0].deck.len(), 2);
        assert!(rs.pending_deck_action.is_none());
    }

    #[test]
    fn resolve_validates_pick_index_range() {
        let mut rs = rs_with_deck(vec![("StrikeIronclad", 0)]);
        rs.auto_resolve_offers = false;
        smith(&mut rs, 0);
        let err = resolve_pending_deck_action(&mut rs, &[5]).unwrap_err();
        assert!(err.contains("out of range"));
        assert!(rs.pending_deck_action.is_some(), "must restore on error");
    }
}
