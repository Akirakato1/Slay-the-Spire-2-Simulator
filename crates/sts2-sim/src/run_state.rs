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
use crate::rng_set::RunRngSet;
use crate::run_log::{NodeEntry, RunLog};
use crate::standard_act_map::StandardActMap;

/// Names a player by their character id (C# `CHARACTER.IRONCLAD` etc.) and
/// their numeric id (1 for solo, Steam id in coop). Player runtime state
/// (HP/gold/deck/relics/...) lands when we port the Player module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerSlot {
    pub character_id: String,
    pub id: i64,
}

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
    players: Vec<PlayerSlot>,
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
}

impl RunState {
    pub fn new(
        seed_string: &str,
        ascension: i32,
        players: Vec<PlayerSlot>,
        acts: Vec<ActId>,
        modifiers: Vec<String>,
    ) -> Self {
        let n = acts.len();
        Self {
            seed_string: seed_string.to_owned(),
            rng_set: RunRngSet::new(seed_string),
            ascension,
            acts,
            acts_have_second_boss: vec![false; n],
            players,
            current_act_index: -1,
            act_floor: 0,
            current_map: None,
            current_coord: None,
            modifiers,
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
        let players = log.players.iter()
            .map(|p| PlayerSlot {
                character_id: p.character.clone(),
                id: p.id,
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
    pub fn players(&self) -> &[PlayerSlot] { &self.players }
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
        let players = vec![PlayerSlot {
            character_id: "CHARACTER.IRONCLAD".to_owned(),
            id: 1,
        }];
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
        let players = vec![PlayerSlot {
            character_id: "CHARACTER.IRONCLAD".to_owned(),
            id: 1,
        }];
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
