//! Minimum-viable `RunState` — the run-global state container.
//!
//! The C# `IRunState` is a much larger surface (cards / relics / odds /
//! unlock state / combat / map history / multiplayer scaling / extra
//! fields / ...). This MVP captures only what's needed to drive the
//! map-gen replay against parsed `.run` files:
//!   - seed (string + uint) and the named RNG set
//!   - ascension level
//!   - character per player (just the C# id string for now)
//!   - the sequence of acts the run visits
//!   - current act index and act floor
//!   - the currently-generated map for that act
//!
//! Card / relic / potion / combat / encounter / event state will land
//! incrementally as those modules port. The public API is shaped so those
//! additions don't break the existing surface.

use crate::act::{act_for, ActId, ActModel};
use crate::map::{MapCoord, MapPointType};
use crate::rng::Rng;
use crate::rng_set::{PlayerRngSet, RunRngSet};
use crate::run_log::{CardRef, NodeEntry, PotionEntry, RelicEntry, RunLog};
use crate::standard_act_map::StandardActMap;

/// Per-player runtime state. Identity fields (`character_id`, `id`) plus
/// HP / gold / deck / relics / potions. Mirrors the `.run` file's
/// per-player record. Card / relic / potion entries reuse the `run_log`
/// types — they're identifier-shaped, no behavior attached. Behavior
/// lands when the Card / Relic / Potion modules port.
#[derive(Debug, Clone)]
pub struct PlayerState {
    pub character_id: String,
    /// Solo runs use 1; coop uses Steam IDs (17-digit). i64 covers both.
    pub id: i64,
    pub hp: i32,
    pub max_hp: i32,
    pub gold: i32,
    pub deck: Vec<CardRef>,
    pub relics: Vec<RelicEntry>,
    pub potions: Vec<PotionEntry>,
    pub max_potion_slot_count: i32,
}

impl PlayerState {
    /// Construct a fresh slot at run-start with empty deck/relics/potions
    /// and zero gold/hp. Use this when forward-simulating a new run;
    /// callers populate the starting deck/relics from the character's
    /// loadout afterwards (those models aren't ported yet).
    pub fn empty(character_id: &str, id: i64) -> Self {
        Self {
            character_id: character_id.to_owned(),
            id,
            hp: 0,
            max_hp: 0,
            gold: 0,
            deck: Vec::new(),
            relics: Vec::new(),
            potions: Vec::new(),
            max_potion_slot_count: 0,
        }
    }
}

/// Backwards-compat alias for code that wants just the identity tuple.
pub type PlayerSlot = PlayerState;

/// Decode a C# `ActModel.Id.Entry` string (e.g. "OVERGROWTH", "ACT.HIVE") into
/// our `ActId` enum. Returns `None` for unknown ids; callers should treat
/// that as a bug worth surfacing rather than silently substituting.
pub fn act_id_from_run_log_string(s: &str) -> Option<ActId> {
    // Run logs prefix act ids with "ACT." (e.g. "ACT.OVERGROWTH"); strip
    // the prefix when present so callers can pass either form.
    let bare = s.strip_prefix("ACT.").unwrap_or(s);
    match bare {
        "OVERGROWTH" => Some(ActId::Overgrowth),
        "HIVE" => Some(ActId::Hive),
        "GLORY" => Some(ActId::Glory),
        "UNDERDOCKS" => Some(ActId::Underdocks),
        "DEPRECATEDACT" | "DEPRECATED_ACT" | "DEPRECATED" => Some(ActId::DeprecatedAct),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct RunState {
    seed_string: String,
    rng_set: RunRngSet,
    ascension: i32,
    /// `acts[i]` = the act played at run-act-index `i` (typically 3 acts for
    /// a standard run).
    acts: Vec<ActId>,
    /// `acts_have_second_boss[i]` mirrors C#'s per-act `HasSecondBoss`
    /// flag, which the game sets from run-setup logic (ascension /
    /// modifiers). Until those are ported we accept it as input — either
    /// caller-set on `new()`, or detected from a `.run` log's history
    /// (trailing double-`boss` entries).
    acts_have_second_boss: Vec<bool>,
    players: Vec<PlayerState>,
    /// 0-based index into `acts`. `-1` = before the first act.
    current_act_index: i32,
    /// 0-based floor within the current act. 0 = starting (ancient) node.
    act_floor: i32,
    /// The generated map for the current act, if entered.
    current_map: Option<StandardActMap>,
    /// The map node the player is currently at within `current_map`. Set
    /// to the starting (ancient) coord on `enter_act`; updated by
    /// `advance_to` as the player navigates.
    current_coord: Option<MapCoord>,
    /// `modifiers` is a list of run modifier ids (e.g. ascension toggles,
    /// daily mutators). Kept as plain strings until the modifier module
    /// lands.
    modifiers: Vec<String>,
    /// Per-player RNG sets (rewards / shops / transformations).
    /// Mirrors C# `Player.PlayerRng`. Seeded from the run seed
    /// XOR'd with the player index — deterministic but not yet
    /// bit-exact to C# (which derives the seed from a separate
    /// player-setup path). Single-player runs use index 0.
    pub players_rng: Vec<PlayerRngSet>,
    /// When true, OfferX effects auto-pick the first option and apply
    /// immediately. RL training flips this to false so the agent can
    /// see and resolve each offer through `resolve_run_state_offer`.
    /// Default true for replay / scripted-fight purposes.
    pub auto_resolve_offers: bool,
    /// One in-flight player offer (card / relic / potion). When `Some`,
    /// the run is paused waiting for the agent to commit. Resolution
    /// applies the picked indices to the player's deck / relics / potions.
    pub pending_offer: Option<PendingRunStateOffer>,
    /// One in-flight deck-action choice (upgrade / remove a card the
    /// player already owns). Distinct from `pending_offer` because
    /// the picks index into the master deck, not an external pool.
    pub pending_deck_action: Option<PendingDeckAction>,
}

/// What kind of reward is being offered. The resolver dispatches on
/// this when applying the agent's picks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferKind {
    /// Card to add to the player's master deck.
    Card,
    /// Relic to add to the player's relic list (fires AfterObtained).
    Relic,
    /// Potion to add to the player's potion belt (capped at
    /// max_potion_slot_count).
    Potion,
}

/// Per-deck action that targets an existing card in the player's
/// master deck. Distinct from `OfferKind` because the picks reference
/// `eligible_indices` slots (positions in the player's `deck` vec),
/// not external pool options. Smith / Toke / Dig / event-driven
/// transforms all share this shape.
#[derive(Debug, Clone)]
pub struct PendingDeckAction {
    pub action: DeckActionKind,
    pub player_idx: usize,
    /// Indices into `players[player_idx].deck` that the agent may
    /// pick from. Built at offer-emit time so the agent never picks
    /// an ineligible card (e.g., a non-upgradable card for Smith).
    pub eligible_indices: Vec<usize>,
    pub n_min: i32,
    pub n_max: i32,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeckActionKind {
    /// Increment `current_upgrade_level` by 1. Smith at campfire,
    /// AfterRoomEntered upgrades from events. C# `CardCmd.Upgrade`.
    Upgrade,
    /// Remove the card from the master deck. Toke at campfire,
    /// "remove a card" event branches, shop card-remove. C#
    /// `CardCmd.RemoveFromDeck`.
    Remove,
}

/// One in-flight reward offer. Mirrors `CombatState.PendingChoice`
/// but operates on the run-state. RL training reads this to know
/// what's on offer; the simulator resolves it via
/// `resolve_run_state_offer(picks)`.
#[derive(Debug, Clone)]
pub struct PendingRunStateOffer {
    pub kind: OfferKind,
    pub player_idx: usize,
    /// Candidate ids the agent picks from.
    pub options: Vec<String>,
    /// Minimum picks (0 = skip allowed; standard card reward is 0).
    pub n_min: i32,
    /// Maximum picks (typically 1 for card / relic / potion offers).
    pub n_max: i32,
    /// Optional source-card / source-room / source-effect tag for
    /// replay diagnostics and feature extraction. e.g. "TreasureRoom",
    /// "EliteReward", "JackOfAllTrades".
    pub source: Option<String>,
}

impl RunState {
    pub fn new(
        seed_string: &str,
        ascension: i32,
        players: Vec<PlayerState>,
        acts: Vec<ActId>,
        modifiers: Vec<String>,
    ) -> Self {
        let n = acts.len();
        let rng_set = RunRngSet::new(seed_string);
        let seed_uint = rng_set.seed_uint();
        let players_rng: Vec<PlayerRngSet> = (0..players.len())
            .map(|i| PlayerRngSet::new(seed_uint ^ (i as u32)))
            .collect();
        Self {
            seed_string: seed_string.to_owned(),
            rng_set,
            ascension,
            acts,
            acts_have_second_boss: vec![false; n],
            players,
            current_act_index: -1,
            act_floor: 0,
            current_map: None,
            current_coord: None,
            modifiers,
            players_rng,
            auto_resolve_offers: true,
            pending_offer: None,
            pending_deck_action: None,
        }
    }

    /// Set the `HasSecondBoss` flag for a given act index. Useful when
    /// driving a forward-simulation run where the second-boss decision
    /// needs to be made up front; replay paths can use
    /// `from_run_log` which auto-detects from history.
    pub fn set_act_has_second_boss(&mut self, act_index: i32, has: bool) {
        self.acts_have_second_boss[act_index as usize] = has;
    }

    pub fn act_has_second_boss(&self, act_index: i32) -> bool {
        self.acts_have_second_boss[act_index as usize]
    }

    /// Build a RunState from a parsed `.run` log. The log records the acts
    /// the run visited and the ascension/seed/players that drove it.
    /// Returns `None` if any act id is unrecognized (caller decides whether
    /// that's a hard failure or a skip).
    pub fn from_run_log(log: &RunLog) -> Option<Self> {
        let acts: Option<Vec<ActId>> = log.acts.iter()
            .map(|s| act_id_from_run_log_string(s))
            .collect();
        let acts = acts?;
        let players: Vec<PlayerState> = log.players.iter()
            .map(|p| {
                // Populate runtime state from the FINAL recorded state.
                // (For mid-run reconstruction we'd later replay per-floor
                //  stat deltas; not in scope for the MVP.)
                let final_stats = last_player_stats(log, p.id);
                PlayerState {
                    character_id: p.character.clone(),
                    id: p.id,
                    hp: final_stats.map(|s| s.current_hp).unwrap_or(0),
                    max_hp: final_stats.map(|s| s.max_hp).unwrap_or(0),
                    gold: final_stats.map(|s| s.current_gold).unwrap_or(0),
                    deck: p.deck.clone(),
                    relics: p.relics.clone(),
                    potions: p.potions.clone(),
                    max_potion_slot_count: p.max_potion_slot_count,
                }
            })
            .collect();
        let modifiers = log.modifiers.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_owned()))
            .collect();
        let mut rs = Self::new(&log.seed, log.ascension, players, acts, modifiers);
        // Detect HasSecondBoss per act by counting trailing "boss" entries
        // in the recorded history. Two-or-more trailing bosses means the
        // act ran with a second-boss layout.
        for (i, hist) in log.map_point_history.iter().enumerate() {
            if i >= rs.acts.len() {
                break;
            }
            let trailing_bosses = hist.iter().rev()
                .take_while(|n| n.map_point_type == "boss")
                .count();
            if trailing_bosses >= 2 {
                rs.acts_have_second_boss[i] = true;
            }
        }
        Some(rs)
    }

    pub fn seed_string(&self) -> &str { &self.seed_string }
    pub fn rng_set(&self) -> &RunRngSet { &self.rng_set }
    pub fn rng_set_mut(&mut self) -> &mut RunRngSet { &mut self.rng_set }
    pub fn ascension(&self) -> i32 { self.ascension }
    pub fn acts(&self) -> &[ActId] { &self.acts }
    pub fn players(&self) -> &[PlayerState] { &self.players }
    pub fn players_mut(&mut self) -> &mut [PlayerState] { &mut self.players }
    /// Indexed mutable access to a single player's state.
    pub fn player_state_mut(&mut self, player_idx: usize) -> Option<&mut PlayerState> {
        self.players.get_mut(player_idx)
    }

    /// Grant a relic to the player and fire its `AfterObtained` hook
    /// through the run-state effect VM. Mirrors C# `Relics.Add(relic) +
    /// await relic.AfterObtained()` from `RelicCmd.AddRelic`.
    pub fn add_relic(&mut self, player_idx: usize, relic_id: &str) {
        if let Some(ps) = self.players.get_mut(player_idx) {
            ps.relics.push(crate::run_log::RelicEntry {
                id: relic_id.to_string(),
                floor_added_to_deck: self.act_floor,
                props: None,
            });
        }
        let Some(arms) = crate::effects::run_state_effects(relic_id) else {
            return;
        };
        for (hook, body) in arms {
            if matches!(hook, crate::effects::RunStateHook::AfterObtained) {
                crate::effects::execute_run_state_effects_with_relic(
                    self, player_idx, &body, Some(relic_id),
                );
            }
        }
    }

    /// Add a card to the player's master deck. Mirrors C# `CardCmd.AddToDeck`.
    /// Fires AfterCardAddedToDeck hooks on every owned relic (BingBong /
    /// BookOfFiveRings / DarkstonePeriapt / LuckyFysh / etc.).
    pub fn add_card(&mut self, player_idx: usize, card_id: &str, upgrade: i32) {
        if let Some(ps) = self.players.get_mut(player_idx) {
            ps.deck.push(crate::run_log::CardRef {
                id: card_id.to_string(),
                floor_added_to_deck: Some(self.act_floor),
                current_upgrade_level: Some(upgrade),
                enchantment: None,
            });
        }
        // Fire AfterCardAddedToDeck hooks. Filter is on the added card
        // (the relic's filter narrows which adds trigger it).
        let n = self.players[player_idx].relics.len();
        let relic_ids: Vec<String> = self.players[player_idx]
            .relics
            .iter()
            .map(|r| r.id.clone())
            .collect();
        for relic_id in relic_ids {
            let Some(arms) = crate::effects::run_state_effects(&relic_id) else {
                continue;
            };
            for (hook, body) in arms {
                if let crate::effects::RunStateHook::AfterCardAddedToDeck { filter } = &hook {
                    let matches = match filter {
                        None => true,
                        Some(f) => crate::effects::card_id_matches_filter(card_id, f),
                    };
                    if matches {
                        crate::effects::execute_run_state_effects_with_relic(
                            self, player_idx, &body, Some(&relic_id),
                        );
                    }
                }
            }
        }
        let _ = n;
    }

    /// Add a potion to the player's belt. Capped at `max_potion_slot_count`;
    /// excess potions are silently dropped (C# behavior — UI offers a
    /// "swap or skip" choice in real play, but RL replay uses fill-or-drop).
    pub fn add_potion(&mut self, player_idx: usize, potion_id: &str) -> bool {
        let Some(ps) = self.players.get_mut(player_idx) else { return false };
        if (ps.potions.len() as i32) >= ps.max_potion_slot_count {
            return false;
        }
        ps.potions.push(crate::run_log::PotionEntry {
            id: potion_id.to_string(),
            slot_index: ps.potions.len() as i32,
        });
        true
    }

    /// Notify run-state of room entry. Fires AfterRoomEntered hooks on
    /// every owned relic. `room_type` is one of "Monster" / "Elite" /
    /// "Boss" / "MerchantRoom" / "RestRoom" / "EventRoom" / "TreasureRoom".
    pub fn enter_room(&mut self, room_type: &str) {
        let n = self.players.len();
        for player_idx in 0..n {
            let relic_ids: Vec<String> = self.players[player_idx]
                .relics
                .iter()
                .map(|r| r.id.clone())
                .collect();
            for relic_id in relic_ids {
                let Some(arms) = crate::effects::run_state_effects(&relic_id) else {
                    continue;
                };
                for (hook, body) in arms {
                    if let crate::effects::RunStateHook::AfterRoomEntered { room_type_filter } = &hook {
                        if let Some(want) = room_type_filter {
                            if want != room_type {
                                continue;
                            }
                        }
                        crate::effects::execute_run_state_effects_with_relic(
                            self, player_idx, &body, Some(&relic_id),
                        );
                    }
                }
            }
        }
    }
    pub fn current_act_index(&self) -> i32 { self.current_act_index }
    pub fn act_floor(&self) -> i32 { self.act_floor }
    pub fn current_map(&self) -> Option<&StandardActMap> { self.current_map.as_ref() }
    pub fn modifiers(&self) -> &[String] { &self.modifiers }
    pub fn is_multiplayer(&self) -> bool { self.players.len() > 1 }

    /// `ActModel` for the current act, if one has been entered.
    pub fn current_act(&self) -> Option<Box<dyn ActModel>> {
        if self.current_act_index < 0 {
            return None;
        }
        let id = *self.acts.get(self.current_act_index as usize)?;
        Some(act_for(id))
    }

    /// Enter act number `act_index` (0-based) and generate its map. The
    /// map RNG seed mirrors C# `StandardActMap.CreateFor`:
    /// `new Rng(runState.Rng.Seed, "act_{n+1}_map")`.
    ///
    /// Resets `act_floor` to 0. Returns the generated map for inspection.
    pub fn enter_act(&mut self, act_index: i32) -> &StandardActMap {
        let act_id = self.acts[act_index as usize];
        let act = act_for(act_id);
        let name = format!("act_{}_map", act_index + 1);
        let map_rng = Rng::new_named(self.rng_set.seed_uint(), &name);
        let is_multiplayer = self.players.len() > 1;
        let has_second_boss = self.acts_have_second_boss[act_index as usize];
        // shouldReplaceTreasureWithElites is a run-state-dependent flag
        // (ascension / modifier driven) we haven't ported yet. Stays
        // false until the modifier module lands.
        let map = StandardActMap::new(
            map_rng, act.as_ref(), is_multiplayer, false, has_second_boss,
            None, true, self.ascension,
        );
        self.current_act_index = act_index;
        self.act_floor = 0;
        self.current_map = Some(map);
        // Standing at the starting (ancient) node when an act begins.
        self.current_coord = self.current_map
            .as_ref()
            .map(|m| m.starting().coord);
        self.current_map.as_ref().unwrap()
    }

    /// Advance `act_floor` by 1 without moving the cursor. Mostly useful
    /// in tests / harnesses that don't track precise map navigation.
    pub fn advance_floor(&mut self) {
        self.act_floor += 1;
    }

    /// Returns the map coord the player is currently standing on within
    /// the current act, or `None` if `enter_act` hasn't been called.
    pub fn current_map_coord(&self) -> Option<MapCoord> {
        self.current_coord
    }

    /// Move the cursor to `coord`. Returns `Err` if no map is loaded, no
    /// current coord is set, or `coord` is not a child of the current
    /// position in the generated map.
    pub fn advance_to(&mut self, coord: MapCoord) -> Result<(), String> {
        let map = self.current_map.as_ref()
            .ok_or_else(|| "no current map".to_string())?;
        let current = self.current_coord
            .ok_or_else(|| "no current coord".to_string())?;
        let current_pt = map.get_point(current.col, current.row)
            .ok_or_else(|| format!("current coord {current:?} not in map"))?;
        if !current_pt.children.iter().any(|c| *c == coord) {
            return Err(format!(
                "{coord:?} is not a child of {current:?} (children: {:?})",
                current_pt.children.iter().copied().collect::<Vec<_>>()
            ));
        }
        self.current_coord = Some(coord);
        self.act_floor += 1;
        Ok(())
    }
}

/// Outcome of replaying a recorded `.run` act through our generated map.
#[derive(Debug, Clone)]
pub struct ReplayOutcome {
    /// Number of floors successfully advanced through.
    pub advanced_floors: i32,
    /// Floors where the recorded type matched multiple children of the
    /// previous node; replay picked the smallest-col candidate to keep
    /// going. Useful as a diagnostic: a clean run has 0 ambiguities.
    pub ambiguous_floors: Vec<i32>,
    /// Whether replay successfully reached a Boss node at the end.
    pub reached_boss: bool,
}

/// Replay the recorded `map_point_history` for a single act through the
/// already-entered map. Caller must have called `enter_act(act_idx)` on
/// `state` first; this advances the cursor through a valid path.
///
/// Uses DFS over the recorded type sequence — the run log doesn't
/// disambiguate between same-type junctions, so we try every viable
/// successor and backtrack on dead ends. Returns `Err` if no path through
/// the generated map satisfies the entire sequence (which would indicate
/// our map genuinely diverges from what the run experienced).
pub fn replay_act_log(
    state: &mut RunState,
    history: &[NodeEntry],
) -> Result<ReplayOutcome, String> {
    let map = state.current_map().ok_or("no current map")?.clone();
    let start = state.current_map_coord().ok_or("no cursor")?;

    // Recorded type sequence, skipping the starting (ancient) node we're
    // already standing on.
    let types: Result<Vec<MapPointType>, String> = history
        .iter()
        .skip(1)
        .enumerate()
        .map(|(i, n)| {
            MapPointType::from_run_log_str(&n.map_point_type)
                .ok_or_else(|| {
                    format!("floor {}: unknown map_point_type {:?}",
                        i + 1, n.map_point_type)
                })
        })
        .collect();
    let types = types?;

    let mut path = vec![start];
    if !find_path(&map, &types, 0, &mut path) {
        return Err(format!(
            "no path through generated map matches recorded type sequence \
             of length {}",
            types.len()
        ));
    }

    // path[0] is start (cursor already there); advance through the rest.
    let mut reached_boss = false;
    for coord in path.iter().skip(1) {
        state.advance_to(*coord)
            .map_err(|e| format!("advance to {coord:?}: {e}"))?;
        if let Some(p) = map.get_point(coord.col, coord.row) {
            if p.point_type == MapPointType::Boss {
                reached_boss = true;
            }
        }
        if let Some(sb) = map.second_boss() {
            if sb.coord == *coord {
                reached_boss = true;
            }
        }
    }

    // Count ambiguous junctions along the chosen path — wherever multiple
    // children of the previous coord matched the same recorded type.
    let mut ambiguous_floors = Vec::new();
    for (i, win) in path.windows(2).enumerate() {
        let prev = win[0];
        let target_type = types[i];
        if target_type == MapPointType::Boss {
            continue;
        }
        let prev_pt = match map.get_point(prev.col, prev.row) {
            Some(p) => p,
            None => continue,
        };
        let matches = prev_pt.children.iter().filter(|c| {
            map.get_point(c.col, c.row)
                .map(|p| p.point_type == target_type)
                .unwrap_or(false)
        }).count();
        if matches > 1 {
            ambiguous_floors.push((i + 1) as i32);
        }
    }

    Ok(ReplayOutcome {
        advanced_floors: (path.len() - 1) as i32,
        ambiguous_floors,
        reached_boss,
    })
}

/// Walk every node entry's `player_stats` in act+floor order and return the
/// most recent entry whose `player_id` matches. Used to seed a
/// `PlayerState` with the run's final HP/gold values.
fn last_player_stats(log: &RunLog, player_id: i64) -> Option<&crate::run_log::PlayerStats> {
    log.map_point_history.iter().rev().flat_map(|act| {
        act.iter().rev().flat_map(|node| {
            node.player_stats.iter().rev()
                .find(|s| s.player_id == player_id)
        })
    }).next()
}

fn find_path(
    map: &StandardActMap,
    types: &[MapPointType],
    idx: usize,
    path: &mut Vec<MapCoord>,
) -> bool {
    if idx == types.len() {
        return true;
    }
    let current = *path.last().expect("path non-empty");
    let target = types[idx];

    // Collect candidate children, with the boss specials handled explicitly.
    let mut candidates: Vec<MapCoord> = Vec::new();
    if target == MapPointType::Boss {
        // From a row-(rows-1) point, children include map.boss(); from
        // map.boss() itself, children may include map.second_boss().
        if let Some(p) = map.get_point(current.col, current.row) {
            for c in p.children.iter().copied() {
                let is_boss = map.get_point(c.col, c.row)
                    .map(|cp| cp.point_type == MapPointType::Boss)
                    .unwrap_or(false);
                if is_boss {
                    candidates.push(c);
                }
            }
        }
    } else if let Some(p) = map.get_point(current.col, current.row) {
        for c in p.children.iter().copied() {
            let matches = map.get_point(c.col, c.row)
                .map(|cp| cp.point_type == target)
                .unwrap_or(false);
            if matches {
                candidates.push(c);
            }
        }
    }
    // Deterministic order (smallest coord first) so ambiguity-bookkeeping
    // is reproducible.
    candidates.sort();

    for c in candidates {
        path.push(c);
        if find_path(map, types, idx + 1, path) {
            return true;
        }
        path.pop();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_relic_fires_after_obtained_max_hp() {
        let players = vec![PlayerState {
            character_id: "CHARACTER.IRONCLAD".into(),
            id: 1,
            hp: 70,
            max_hp: 70,
            gold: 0,
            deck: Vec::new(),
            relics: Vec::new(),
            potions: Vec::new(),
            max_potion_slot_count: 3,
        }];
        let mut rs = RunState::new(
            "ABC123",
            0,
            players,
            vec![ActId::Overgrowth],
            Vec::new(),
        );
        rs.add_relic(0, "Mango");
        let ps = &rs.players()[0];
        assert_eq!(ps.max_hp, 84); // 70 + 14
        assert_eq!(ps.hp, 84);
        assert_eq!(ps.relics.len(), 1);
        assert_eq!(ps.relics[0].id, "Mango");
    }

    #[test]
    fn add_relic_unknown_just_appends_to_list() {
        let players = vec![PlayerState::empty("CHARACTER.IRONCLAD", 1)];
        let mut rs = RunState::new(
            "ABC123",
            0,
            players,
            vec![ActId::Overgrowth],
            Vec::new(),
        );
        rs.add_relic(0, "TotallyMadeUpRelic");
        let ps = &rs.players()[0];
        assert_eq!(ps.relics.len(), 1);
        // No mutation since there's no run-state hook for the id.
        assert_eq!(ps.max_hp, 0);
        assert_eq!(ps.gold, 0);
    }

    #[test]
    fn add_relic_gold_relic_grants_gold() {
        let players = vec![PlayerState {
            character_id: "CHARACTER.IRONCLAD".into(),
            id: 1,
            hp: 70,
            max_hp: 70,
            gold: 99,
            deck: Vec::new(),
            relics: Vec::new(),
            potions: Vec::new(),
            max_potion_slot_count: 3,
        }];
        let mut rs = RunState::new(
            "ABC123",
            0,
            players,
            vec![ActId::Overgrowth],
            Vec::new(),
        );
        rs.add_relic(0, "OldCoin");
        assert_eq!(rs.players()[0].gold, 99 + 300);
    }

    #[test]
    fn act_id_strings_round_trip() {
        assert_eq!(act_id_from_run_log_string("ACT.OVERGROWTH"), Some(ActId::Overgrowth));
        assert_eq!(act_id_from_run_log_string("OVERGROWTH"), Some(ActId::Overgrowth));
        assert_eq!(act_id_from_run_log_string("ACT.HIVE"), Some(ActId::Hive));
        assert_eq!(act_id_from_run_log_string("ACT.GLORY"), Some(ActId::Glory));
        assert_eq!(act_id_from_run_log_string("ACT.UNDERDOCKS"), Some(ActId::Underdocks));
        assert_eq!(act_id_from_run_log_string("ACT.NONSENSE"), None);
    }

    #[test]
    fn run_state_construction_seeds_rng_set() {
        let players = vec![PlayerState::empty("CHARACTER.IRONCLAD", 1)];
        let rs = RunState::new(
            "ABC123", 9, players,
            vec![ActId::Overgrowth, ActId::Hive, ActId::Glory],
            Vec::new(),
        );
        assert_eq!(rs.seed_string(), "ABC123");
        assert_eq!(rs.ascension(), 9);
        assert_eq!(rs.acts().len(), 3);
        assert_eq!(rs.current_act_index(), -1);
        assert!(rs.current_map().is_none());
        assert!(rs.current_act().is_none());
        assert!(!rs.is_multiplayer());
        // The rng set's seed_uint must equal hash(seed_string).
        let expected = crate::hash::deterministic_hash_code("ABC123") as u32;
        assert_eq!(rs.rng_set().seed_uint(), expected);
    }

    #[test]
    fn enter_act_generates_map_and_resets_floor() {
        let players = vec![PlayerState::empty("CHARACTER.IRONCLAD", 1)];
        let mut rs = RunState::new(
            "ABC123", 0, players,
            vec![ActId::Overgrowth, ActId::Hive, ActId::Glory],
            Vec::new(),
        );
        rs.advance_floor(); // pretend to have advanced before entering act
        rs.advance_floor();
        let map = rs.enter_act(0);
        assert_eq!(map.cols(), 7);
        // Overgrowth: BaseNumberOfRooms = 15 → map_length = rows = 16.
        assert_eq!(map.rows(), 16);
        // Entering an act resets the floor counter.
        assert_eq!(rs.act_floor(), 0);
        assert_eq!(rs.current_act_index(), 0);
        assert!(rs.current_map().is_some());
        assert!(rs.current_act().is_some());
    }

    #[test]
    fn from_run_log_pulls_seed_and_acts() {
        // A minimal RunLog stub via JSON.
        let json = r#"{
            "win": true,
            "ascension": 9,
            "seed": "F0NPZN1C6U",
            "schema_version": 8,
            "acts": ["ACT.OVERGROWTH", "ACT.HIVE", "ACT.GLORY"],
            "players": [{
                "character": "CHARACTER.IRONCLAD", "id": 1, "deck": [],
                "relics": [], "potions": [], "max_potion_slot_count": 3
            }]
        }"#;
        let log: RunLog = crate::run_log::from_str(json).unwrap();
        let rs = RunState::from_run_log(&log).expect("act ids must resolve");
        assert_eq!(rs.seed_string(), "F0NPZN1C6U");
        assert_eq!(rs.ascension(), 9);
        assert_eq!(rs.acts(),
            &[ActId::Overgrowth, ActId::Hive, ActId::Glory]);
        assert_eq!(rs.players().len(), 1);
        assert_eq!(rs.players()[0].character_id, "CHARACTER.IRONCLAD");
        assert_eq!(rs.players()[0].id, 1);
    }
}
