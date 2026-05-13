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
use crate::map::MapCoord;
use crate::rng::Rng;
use crate::rng_set::RunRngSet;
use crate::run_log::RunLog;
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
            None, true,
        );
        self.current_act_index = act_index;
        self.act_floor = 0;
        self.current_map = Some(map);
        self.current_map.as_ref().unwrap()
    }

    /// Advance `act_floor` by 1. Sets `current_map_coord` to None for the
    /// MVP — proper map traversal lands when we port the player's
    /// `CurrentMapCoord` selection logic.
    pub fn advance_floor(&mut self) {
        self.act_floor += 1;
    }

    /// Stub: real implementation will track the player's chosen map node
    /// each floor. Returns None until that lands.
    pub fn current_map_coord(&self) -> Option<MapCoord> {
        None
    }
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
