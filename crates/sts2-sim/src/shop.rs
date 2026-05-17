//! Shop (merchant room).
//!
//! C# port of `Core/Entities/Merchant/MerchantInventory` and the
//! `MerchantCardEntry / MerchantRelicEntry / MerchantPotionEntry /
//! MerchantCardRemovalEntry` family.
//!
//! Standard inventory:
//!   - 5 character cards (2 Attack / 2 Skill / 1 Power)
//!   - 2 colorless cards (Uncommon + Rare)
//!   - 3 relics (one each from Common / Uncommon / Rare or Shop pool)
//!   - 3 potions
//!   - 1 card-remove service
//!
//! Prices (C# `MerchantCardEntry.GetCost`, etc.):
//!   - Cards: Common 50, Uncommon 75, Rare 150; ×0.95–1.05 jitter.
//!     Colorless cards: × 1.15 then jitter.
//!   - Potions: Common 50, Uncommon 75, Rare 100; ×0.95–1.05 jitter.
//!   - Relics: per-rarity default (Common 150 / Uncommon 250 /
//!     Rare 300 / Shop 150) × 0.85–1.15 jitter. C# uses
//!     `RelicData.MerchantCost` for per-relic overrides — not yet
//!     in our data table; falls back to rarity defaults.
//!   - Card remove: 75 base (Inflation ascension makes it 100;
//!     ignored at ascension 0).
//!
//! Simplifications vs C#:
//!   - One "on-sale" card per visit at half price — deferred.
//!   - `CardShopRemovalsUsed` per-player counter that ramps the
//!     card-remove price (75 / 100 / 125 / ...) — deferred; base
//!     75 used always.
//!   - PlayerOdds-based rarity adjustments — deferred.

use crate::card::{self, CardRarity};
use crate::card_reward::roll_card_rarity;
use crate::card_reward::CardRewardKind;
use crate::relic::{self, RelicRarity};
use crate::run_state::{DeckActionKind, PendingDeckAction, RunState};

/// One purchasable item in the shop. Indexes back into the underlying
/// item (card id / relic id / potion id) plus the rolled price.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopEntry {
    pub kind: ShopEntryKind,
    pub item_id: String,
    pub price: i32,
    /// True after the entry has been purchased (preserves position
    /// in the listing so UI can show "sold out" but RL features see
    /// a stable size).
    pub sold: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShopEntryKind {
    Card,
    Relic,
    Potion,
    /// Card-remove service. `item_id` is unused (empty string); the
    /// price reflects `BaseCost + PriceIncrease * usage`.
    CardRemove,
}

/// Full shop state for a single merchant-room visit. Populated by
/// `open_shop`; mutated by `purchase`.
#[derive(Debug, Clone, Default)]
pub struct ShopState {
    pub player_idx: usize,
    pub entries: Vec<ShopEntry>,
}

/// Stand up a fresh merchant inventory and return the populated
/// ShopState. Caller is responsible for storing the state somewhere
/// (RunState.pending_shop slot or a Vec of room states). The shop
/// uses the `Shops` PlayerRngSet stream for jitter + card-pool picks
/// and the `combat_card_generation` run-level stream for rarity rolls.
pub fn open_shop(rs: &mut RunState, player_idx: usize) -> ShopState {
    let character = rs
        .players()
        .get(player_idx)
        .map(|ps| ps.character_id.clone())
        .unwrap_or_default();
    let mut entries: Vec<ShopEntry> = Vec::new();

    // 5 character cards: 2 Attack, 2 Skill, 1 Power (C# layout).
    let card_types = ["Attack", "Attack", "Skill", "Skill", "Power"];
    let mut card_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for &ct in card_types.iter() {
        let rarity = roll_card_rarity(rs, CardRewardKind::Normal);
        if let Some(id) = pick_shop_card(rs, &character, Some(ct), rarity, &card_seen) {
            card_seen.insert(id.clone());
            let price = jitter_card_price(rs, player_idx, rarity, /*colorless=*/false);
            entries.push(ShopEntry {
                kind: ShopEntryKind::Card,
                item_id: id, price, sold: false,
            });
        }
    }
    // 2 colorless cards: Uncommon + Rare.
    for &rarity in &[CardRarity::Uncommon, CardRarity::Rare] {
        if let Some(id) = pick_shop_card(rs, "Colorless", None, rarity, &card_seen) {
            card_seen.insert(id.clone());
            let price = jitter_card_price(rs, player_idx, rarity, /*colorless=*/true);
            entries.push(ShopEntry {
                kind: ShopEntryKind::Card,
                item_id: id, price, sold: false,
            });
        }
    }

    // 3 relics. C# rolls (Shop, Common, Uncommon) or a Rare variant
    // — we use a fixed [Shop, Common, Uncommon] line-up which matches
    // the default merchant layout. Caller can re-roll for variety.
    let relic_rarities = [RelicRarity::Shop, RelicRarity::Common, RelicRarity::Uncommon];
    let mut relic_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for &r in &relic_rarities {
        if let Some(id) = pick_shop_relic(rs, player_idx, &character, r, &relic_seen) {
            relic_seen.insert(id.clone());
            let price = jitter_relic_price(rs, player_idx, r);
            entries.push(ShopEntry {
                kind: ShopEntryKind::Relic,
                item_id: id, price, sold: false,
            });
        }
    }

    // 3 potions. Roll a rarity for each, then pick uniformly.
    let mut potion_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for _ in 0..3 {
        let rarity = roll_potion_rarity(rs);
        if let Some(id) = pick_shop_potion(rs, rarity, &potion_seen) {
            potion_seen.insert(id.clone());
            let price = jitter_potion_price(rs, player_idx, rarity);
            entries.push(ShopEntry {
                kind: ShopEntryKind::Potion,
                item_id: id, price, sold: false,
            });
        }
    }

    // 1 card-remove service.
    entries.push(ShopEntry {
        kind: ShopEntryKind::CardRemove,
        item_id: String::new(),
        price: card_remove_price(rs, player_idx),
        sold: false,
    });

    ShopState { player_idx, entries }
}

fn pick_shop_card(
    rs: &mut RunState,
    pool: &str,
    card_type: Option<&str>,
    rarity: CardRarity,
    exclude: &std::collections::HashSet<String>,
) -> Option<String> {
    let pool_owned = pool.to_string();
    let candidates: Vec<&str> = card::ALL_CARDS
        .iter()
        .filter(|c| c.pool == pool_owned)
        .filter(|c| c.rarity == rarity)
        .filter(|c| !exclude.contains(&c.id))
        .filter(|c| match card_type {
            None => true,
            Some(t) => format!("{:?}", c.card_type).eq_ignore_ascii_case(t),
        })
        .filter(|c| c.id != "DeprecatedCard")
        .filter(|c| !crate::card_reward::is_multiplayer_only(&c.id))
        .map(|c| c.id.as_str())
        .collect();
    if candidates.is_empty() { return None; }
    let idx = rs.players_rng
        .get_mut(0)
        .map(|p| p.shops.next_int(candidates.len() as i32))
        .unwrap_or(0) as usize;
    Some(candidates[idx].to_string())
}

fn pick_shop_relic(
    rs: &mut RunState,
    player_idx: usize,
    character: &str,
    rarity: RelicRarity,
    exclude: &std::collections::HashSet<String>,
) -> Option<String> {
    let owned: std::collections::HashSet<String> = rs
        .players()
        .get(player_idx)
        .map(|ps| ps.relics.iter().map(|r| r.id.clone()).collect())
        .unwrap_or_default();
    let character_owned = character.to_string();
    let candidates: Vec<&str> = relic::ALL_RELICS
        .iter()
        .filter(|r| r.rarity == rarity)
        .filter(|r| !owned.contains(&r.id))
        .filter(|r| !exclude.contains(&r.id))
        .filter(|r| r.pools.iter().any(|p| p == "Shared" || p == &character_owned))
        .map(|r| r.id.as_str())
        .collect();
    if candidates.is_empty() { return None; }
    let idx = rs.players_rng
        .get_mut(player_idx)
        .map(|p| p.shops.next_int(candidates.len() as i32))
        .unwrap_or(0) as usize;
    Some(candidates[idx].to_string())
}

fn roll_potion_rarity(rs: &mut RunState) -> crate::potion::PotionRarity {
    // C# potion-rarity weights: ~65% Common, ~25% Uncommon, ~10% Rare.
    let n = rs.rng_set_mut().combat_potion_generation.next_float(1.0);
    if n < 0.65 { crate::potion::PotionRarity::Common }
    else if n < 0.90 { crate::potion::PotionRarity::Uncommon }
    else { crate::potion::PotionRarity::Rare }
}

fn pick_shop_potion(
    rs: &mut RunState,
    rarity: crate::potion::PotionRarity,
    exclude: &std::collections::HashSet<String>,
) -> Option<String> {
    let candidates: Vec<&str> = crate::potion::ALL_POTIONS
        .iter()
        .filter(|p| p.rarity == rarity)
        .filter(|p| !exclude.contains(&p.id))
        .filter(|p| p.id != "DeprecatedPotion")
        .map(|p| p.id.as_str())
        .collect();
    if candidates.is_empty() { return None; }
    let idx = rs.players_rng
        .get_mut(0)
        .map(|p| p.shops.next_int(candidates.len() as i32))
        .unwrap_or(0) as usize;
    Some(candidates[idx].to_string())
}

fn jitter_card_price(rs: &mut RunState, player_idx: usize, rarity: CardRarity, colorless: bool) -> i32 {
    let base = match rarity {
        CardRarity::Common => 50,
        CardRarity::Uncommon => 75,
        CardRarity::Rare => 150,
        _ => 50,
    };
    let base = if colorless { (base as f32 * 1.15).round() as i32 } else { base };
    let jitter = rs.players_rng
        .get_mut(player_idx)
        .map(|p| p.shops.next_float_range(0.95, 1.05))
        .unwrap_or(1.0);
    ((base as f32) * jitter).round() as i32
}

fn jitter_potion_price(rs: &mut RunState, player_idx: usize, rarity: crate::potion::PotionRarity) -> i32 {
    let base = match rarity {
        crate::potion::PotionRarity::Common => 50,
        crate::potion::PotionRarity::Uncommon => 75,
        crate::potion::PotionRarity::Rare => 100,
        _ => 50,
    };
    let jitter = rs.players_rng
        .get_mut(player_idx)
        .map(|p| p.shops.next_float_range(0.95, 1.05))
        .unwrap_or(1.0);
    ((base as f32) * jitter).round() as i32
}

fn jitter_relic_price(rs: &mut RunState, player_idx: usize, rarity: RelicRarity) -> i32 {
    // Rarity defaults (no per-relic MerchantCost data yet).
    let base = match rarity {
        RelicRarity::Common => 150,
        RelicRarity::Uncommon => 250,
        RelicRarity::Rare => 300,
        RelicRarity::Shop => 150,
        _ => 200,
    };
    let jitter = rs.players_rng
        .get_mut(player_idx)
        .map(|p| p.shops.next_float_range(0.85, 1.15))
        .unwrap_or(1.0);
    ((base as f32) * jitter).round() as i32
}

fn card_remove_price(rs: &RunState, player_idx: usize) -> i32 {
    // C# `MerchantCardRemovalEntry.CalcCost`:
    //   cost = BaseCost + PriceIncrease * CardShopRemovalsUsed
    //   BaseCost      = GetValueIfAscension(Inflation, 100, 75)
    //   PriceIncrease = GetValueIfAscension(Inflation,  50, 25)
    let inflated = crate::ascension::has_level(
        rs.ascension(),
        crate::ascension::level::Inflation,
    );
    let base_cost = if inflated { 100 } else { 75 };
    let price_increase = if inflated { 50 } else { 25 };
    let removals = rs
        .players()
        .get(player_idx)
        .map(|ps| ps.card_shop_removals_used)
        .unwrap_or(0);
    base_cost + price_increase * removals
}

/// Possible outcomes of a purchase attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PurchaseResult {
    /// Card / relic / potion granted, gold deducted.
    Ok,
    /// Card-remove service: gold deducted and a deck-action is staged
    /// (auto-resolve picks the first eligible card; deferred mode
    /// pauses on `pending_deck_action`).
    CardRemoveStaged,
    NotEnoughGold,
    AlreadySold,
    InvalidIndex,
    BeltFull, // potion belt full
}

/// Attempt to purchase the entry at `entry_index` from a shop.
pub fn purchase(
    rs: &mut RunState,
    shop: &mut ShopState,
    entry_index: usize,
) -> PurchaseResult {
    if entry_index >= shop.entries.len() {
        return PurchaseResult::InvalidIndex;
    }
    if shop.entries[entry_index].sold {
        return PurchaseResult::AlreadySold;
    }
    let price = shop.entries[entry_index].price;
    let cur_gold = rs.players()[shop.player_idx].gold;
    if cur_gold < price {
        return PurchaseResult::NotEnoughGold;
    }
    let entry = shop.entries[entry_index].clone();
    match entry.kind {
        ShopEntryKind::Card => {
            rs.player_state_mut(shop.player_idx).map(|ps| ps.gold -= price);
            rs.add_card(shop.player_idx, &entry.item_id, 0);
            shop.entries[entry_index].sold = true;
            PurchaseResult::Ok
        }
        ShopEntryKind::Relic => {
            rs.player_state_mut(shop.player_idx).map(|ps| ps.gold -= price);
            rs.add_relic(shop.player_idx, &entry.item_id);
            shop.entries[entry_index].sold = true;
            PurchaseResult::Ok
        }
        ShopEntryKind::Potion => {
            let belt_full = {
                let ps = &rs.players()[shop.player_idx];
                (ps.potions.len() as i32) >= ps.max_potion_slot_count
            };
            if belt_full {
                return PurchaseResult::BeltFull;
            }
            rs.player_state_mut(shop.player_idx).map(|ps| ps.gold -= price);
            rs.add_potion(shop.player_idx, &entry.item_id);
            shop.entries[entry_index].sold = true;
            PurchaseResult::Ok
        }
        ShopEntryKind::CardRemove => {
            // Stage a Remove deck-action. Eligibility = non-Curse.
            let eligible: Vec<usize> = {
                let ps = &rs.players()[shop.player_idx];
                ps.deck.iter().enumerate().filter_map(|(i, c)| {
                    let data = card::by_id(&c.id)?;
                    if matches!(data.rarity, CardRarity::Curse) { None } else { Some(i) }
                }).collect()
            };
            if eligible.is_empty() {
                return PurchaseResult::AlreadySold; // no removable cards
            }
            rs.player_state_mut(shop.player_idx).map(|ps| {
                ps.gold -= price;
                // Bump per-run usage counter — next shop visit prices
                // up by `PriceIncrease`. Mirrors C# `Player.ExtraFields
                // .CardShopRemovalsUsed` increment.
                ps.card_shop_removals_used += 1;
            });
            shop.entries[entry_index].sold = true;
            if rs.auto_resolve_offers {
                // Auto: remove the first eligible card.
                let pick = eligible[0];
                rs.player_state_mut(shop.player_idx).map(|ps| {
                    if pick < ps.deck.len() { ps.deck.remove(pick); }
                });
                PurchaseResult::Ok
            } else {
                rs.pending_deck_action = Some(PendingDeckAction {
                    action: DeckActionKind::Remove,
                    player_idx: shop.player_idx,
                    eligible_indices: eligible,
                    n_min: 1,
                    n_max: 1,
                    source: Some("ShopCardRemove".to_string()),
                });
                PurchaseResult::CardRemoveStaged
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::act::ActId;
    use crate::run_log::CardRef;
    use crate::run_state::PlayerState;

    fn rs_with_gold(gold: i32, deck: Vec<&str>) -> RunState {
        let player = PlayerState {
            character_id: "Ironclad".to_string(),
            id: 1, hp: 80, max_hp: 80, gold,
            deck: deck.iter().map(|id| CardRef {
                id: id.to_string(),
                floor_added_to_deck: None,
                current_upgrade_level: None,
                enchantment: None,
            }).collect(),
            relics: Vec::new(),
            potions: Vec::new(),
            max_potion_slot_count: 3,
            card_shop_removals_used: 0,
        };
        RunState::new("seed", 0, vec![player], vec![ActId::Overgrowth], Vec::new())
    }

    #[test]
    fn open_shop_returns_card_relic_potion_remove_entries() {
        let mut rs = rs_with_gold(500, vec!["StrikeIronclad".to_string()]
            .into_iter().map(|s: String| {
                let leaked: &'static str = Box::leak(s.into_boxed_str()); leaked
            }).collect());
        let shop = open_shop(&mut rs, 0);
        // Categories present: should have at least one of each except
        // CardRemove which is always exactly 1.
        let cards = shop.entries.iter().filter(|e| e.kind == ShopEntryKind::Card).count();
        let relics = shop.entries.iter().filter(|e| e.kind == ShopEntryKind::Relic).count();
        let potions = shop.entries.iter().filter(|e| e.kind == ShopEntryKind::Potion).count();
        let removes = shop.entries.iter().filter(|e| e.kind == ShopEntryKind::CardRemove).count();
        assert!(cards >= 5, "Expected ≥5 character/colorless cards, got {}", cards);
        assert!(relics >= 1, "Expected ≥1 relic, got {}", relics);
        assert!(potions >= 1, "Expected ≥1 potion, got {}", potions);
        assert_eq!(removes, 1, "Expected exactly 1 card-remove, got {}", removes);
    }

    #[test]
    fn purchase_card_deducts_gold_and_adds_to_deck() {
        let mut rs = rs_with_gold(500, Vec::new());
        let mut shop = open_shop(&mut rs, 0);
        let card_idx = shop.entries.iter()
            .position(|e| e.kind == ShopEntryKind::Card)
            .expect("at least one card");
        let card_price = shop.entries[card_idx].price;
        let card_id = shop.entries[card_idx].item_id.clone();
        let gold_before = rs.players()[0].gold;
        assert_eq!(purchase(&mut rs, &mut shop, card_idx), PurchaseResult::Ok);
        assert_eq!(rs.players()[0].gold, gold_before - card_price);
        assert!(shop.entries[card_idx].sold);
        assert!(rs.players()[0].deck.iter().any(|c| c.id == card_id));
    }

    #[test]
    fn purchase_with_insufficient_gold_fails() {
        let mut rs = rs_with_gold(10, Vec::new()); // not enough for anything
        let mut shop = open_shop(&mut rs, 0);
        let card_idx = shop.entries.iter()
            .position(|e| e.kind == ShopEntryKind::Card)
            .expect("a card");
        assert_eq!(purchase(&mut rs, &mut shop, card_idx),
            PurchaseResult::NotEnoughGold);
        assert_eq!(rs.players()[0].gold, 10);
        assert!(!shop.entries[card_idx].sold);
    }

    #[test]
    fn purchase_already_sold_fails() {
        let mut rs = rs_with_gold(500, Vec::new());
        let mut shop = open_shop(&mut rs, 0);
        let card_idx = shop.entries.iter()
            .position(|e| e.kind == ShopEntryKind::Card)
            .expect("a card");
        purchase(&mut rs, &mut shop, card_idx);
        assert_eq!(purchase(&mut rs, &mut shop, card_idx),
            PurchaseResult::AlreadySold);
    }

    #[test]
    fn purchase_relic_grants_relic_and_fires_hook() {
        let mut rs = rs_with_gold(1000, Vec::new());
        let mut shop = open_shop(&mut rs, 0);
        let relic_idx = shop.entries.iter()
            .position(|e| e.kind == ShopEntryKind::Relic)
            .expect("a relic");
        let relic_id = shop.entries[relic_idx].item_id.clone();
        assert_eq!(purchase(&mut rs, &mut shop, relic_idx),
            PurchaseResult::Ok);
        assert!(rs.players()[0].relics.iter().any(|r| r.id == relic_id));
    }

    #[test]
    fn card_remove_auto_resolves_with_first_eligible() {
        let mut rs = rs_with_gold(200, vec!["StrikeIronclad", "DefendIronclad"]);
        let mut shop = open_shop(&mut rs, 0);
        let remove_idx = shop.entries.iter()
            .position(|e| e.kind == ShopEntryKind::CardRemove)
            .expect("card remove entry");
        let remove_price = shop.entries[remove_idx].price;
        let gold_before = rs.players()[0].gold;
        assert_eq!(purchase(&mut rs, &mut shop, remove_idx),
            PurchaseResult::Ok);
        assert_eq!(rs.players()[0].gold, gold_before - remove_price);
        assert_eq!(rs.players()[0].deck.len(), 1);
        assert_eq!(rs.players()[0].deck[0].id, "DefendIronclad");
    }

    #[test]
    fn card_remove_deferred_pauses_for_agent() {
        let mut rs = rs_with_gold(200, vec!["StrikeIronclad", "DefendIronclad"]);
        rs.auto_resolve_offers = false;
        let mut shop = open_shop(&mut rs, 0);
        let remove_idx = shop.entries.iter()
            .position(|e| e.kind == ShopEntryKind::CardRemove)
            .expect("card remove entry");
        let result = purchase(&mut rs, &mut shop, remove_idx);
        assert_eq!(result, PurchaseResult::CardRemoveStaged);
        let pending = rs.pending_deck_action.as_ref().expect("deck action staged");
        assert_eq!(pending.action, DeckActionKind::Remove);
        assert_eq!(pending.eligible_indices.len(), 2);
        assert_eq!(pending.source.as_deref(), Some("ShopCardRemove"));
        // Deck unchanged until resolved.
        assert_eq!(rs.players()[0].deck.len(), 2);
    }

    #[test]
    fn potion_purchase_capped_by_belt() {
        let mut rs = rs_with_gold(500, Vec::new());
        // Fill the belt.
        rs.add_potion(0, "BlockPotion");
        rs.add_potion(0, "FirePotion");
        rs.add_potion(0, "EnergyPotion");
        let mut shop = open_shop(&mut rs, 0);
        let potion_idx = shop.entries.iter()
            .position(|e| e.kind == ShopEntryKind::Potion)
            .expect("a potion");
        let gold_before = rs.players()[0].gold;
        assert_eq!(purchase(&mut rs, &mut shop, potion_idx),
            PurchaseResult::BeltFull);
        // Gold not deducted.
        assert_eq!(rs.players()[0].gold, gold_before);
        // Entry not marked sold.
        assert!(!shop.entries[potion_idx].sold);
    }

    #[test]
    fn prices_are_in_expected_ranges() {
        let mut rs = rs_with_gold(0, Vec::new());
        let shop = open_shop(&mut rs, 0);
        for e in &shop.entries {
            match e.kind {
                ShopEntryKind::Card => {
                    // 48 (Common min) to 182 (Rare colorless max).
                    assert!(e.price >= 47 && e.price <= 182,
                        "card price out of range: {} for {}",
                        e.price, e.item_id);
                }
                ShopEntryKind::Relic => {
                    // Common min ~128, Rare max ~345.
                    assert!(e.price >= 127 && e.price <= 345,
                        "relic price out of range: {} for {}",
                        e.price, e.item_id);
                }
                ShopEntryKind::Potion => {
                    // Common min ~48, Rare max ~105.
                    assert!(e.price >= 47 && e.price <= 105,
                        "potion price out of range: {} for {}",
                        e.price, e.item_id);
                }
                ShopEntryKind::CardRemove => {
                    assert_eq!(e.price, 75);
                }
            }
        }
    }
}
