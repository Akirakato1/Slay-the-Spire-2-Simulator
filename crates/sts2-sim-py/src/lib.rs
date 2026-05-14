//! Python bindings for `sts2-sim`.
//!
//! Surface:
//!   - Static data lookups (cards / relics / powers / monsters /
//!     characters) returning JSON strings.
//!   - `.run` file parser.
//!   - Feature vectors: `card_features_vec`, `relic_features_vec`.
//!   - `PyCombatEnv` — stateful gym-style env (reset / step /
//!     legal_actions / observation / clone_state / set_state).
//!   - Schema-version constants so the Python side can assert
//!     compatibility before loading a checkpoint.
//!
//! ## Build
//!
//! From this directory in a Python env (Python 3.9+):
//!
//!     pip install maturin
//!     maturin develop --release
//!
//! Then in Python:
//!
//!     import sts2_sim_py, json
//!     env = sts2_sim_py.PyCombatEnv(
//!         seed=42,
//!         character="Ironclad",
//!         encounter="AxebotsNormal",
//!     )
//!     obs = json.loads(env.observation())
//!     legal = json.loads(env.legal_actions())
//!     outcome = json.loads(env.step(json.dumps(legal[0])))
//!     while not outcome["terminal"]:
//!         legal = json.loads(env.legal_actions())
//!         outcome = json.loads(env.step(json.dumps(legal[0])))
//!
//! ## Why JSON for everything?
//!
//! Returning JSON strings keeps the Rust↔Python boundary thin — no
//! handcrafted `#[pyclass]` wrapper for every type. Rust types stay
//! free to evolve; the Python side does `json.loads`. When training
//! profiling shows JSON serialization is hot, add per-type
//! `#[pyclass]` wrappers that expose direct numpy arrays. The wire
//! format (the JSON shape) stays stable.

use pyo3::exceptions::{PyIOError, PyValueError};
use pyo3::prelude::*;
use sts2_sim::{
    card, character, combat, encounter, env as sim_env, features, monster,
    monster_dispatch, power, relic, run_log,
};

// ---------- Static data lookups -------------------------------------

#[pyfunction]
fn parse_run_file(path: &str) -> PyResult<String> {
    let log = run_log::from_path(path)
        .map_err(|e| PyIOError::new_err(format!("reading {path}: {e}")))?;
    serde_json::to_string(&log)
        .map_err(|e| PyValueError::new_err(format!("serializing RunLog: {e}")))
}

#[pyfunction]
fn card_count() -> usize {
    card::ALL_CARDS.len()
}

#[pyfunction]
fn card_ids() -> Vec<String> {
    card::ALL_CARDS.iter().map(|c| c.id.clone()).collect()
}

#[pyfunction]
fn card_data(id: &str) -> PyResult<Option<String>> {
    let Some(c) = card::by_id(id) else {
        return Ok(None);
    };
    serde_json::to_string(c)
        .map(Some)
        .map_err(|e| PyValueError::new_err(format!("serializing card {id}: {e}")))
}

#[pyfunction]
fn relic_count() -> usize {
    relic::ALL_RELICS.len()
}

#[pyfunction]
fn power_count() -> usize {
    power::ALL_POWERS.len()
}

#[pyfunction]
fn monster_count() -> usize {
    monster::ALL_MONSTERS.len()
}

/// True if the sim has a per-turn intent dispatcher for this monster
/// id. Used by tools/run_replay/coverage.py to derive ported-monster
/// coverage without manually mirroring a list.
#[pyfunction]
fn monster_has_dispatch(model_id: &str) -> bool {
    monster_dispatch::monster_has_dispatch(model_id)
}

#[pyfunction]
fn character_ids() -> Vec<String> {
    character::ALL_CHARACTERS
        .iter()
        .map(|c| c.id.clone())
        .collect()
}

// ---------- Feature vector accessors --------------------------------

/// Featurize one card and return its [f32; CARD_FEATURE_DIM] vector
/// as a Python list. `upgrade_level` defaults to 0; pass 1 to get the
/// upgraded form's features. Returns None if the id isn't in the table.
#[pyfunction]
#[pyo3(signature = (id, upgrade_level=0))]
fn card_features_vec(id: &str, upgrade_level: i32) -> Option<Vec<f32>> {
    let card = card::by_id(id)?;
    let instance = combat::CardInstance::from_card(card, upgrade_level);
    let f = features::card_features(card, Some(&instance));
    Some(f.as_slice().to_vec())
}

/// Featurize one relic and return its [f32; RELIC_FEATURE_DIM] vector.
/// None if id isn't in the table.
#[pyfunction]
fn relic_features_vec(id: &str) -> Option<Vec<f32>> {
    let r = relic::by_id(id)?;
    Some(features::relic_features(r).as_slice().to_vec())
}

/// Observation schema version. The Python agent should pin this to its
/// training-time value and refuse to load checkpoints whose version
/// doesn't match.
#[pyfunction]
fn observation_schema_version() -> u32 {
    features::OBSERVATION_SCHEMA_VERSION
}

/// Card feature vector size — convenience for sizing tensor inputs.
#[pyfunction]
fn card_feature_dim() -> usize {
    features::CARD_FEATURE_DIM
}

#[pyfunction]
fn relic_feature_dim() -> usize {
    features::RELIC_FEATURE_DIM
}

#[pyfunction]
fn creature_state_feature_dim() -> usize {
    features::CREATURE_STATE_FEATURE_DIM
}

// ---------- PyCombatEnv ---------------------------------------------

/// Stateful combat environment. Construction runs `reset` implicitly;
/// call methods on the returned instance for the step loop. JSON I/O
/// per the module docstring.
#[pyclass]
pub struct PyCombatEnv {
    inner: sim_env::CombatEnv,
    /// Character id captured for `reset()` re-use.
    character_id: String,
    /// Encounter id captured for `reset()` re-use.
    encounter_id: String,
    /// Seed captured for `reset()` re-use.
    seed: u32,
}

#[pymethods]
impl PyCombatEnv {
    /// Build a fresh CombatEnv. Looks up `character` and `encounter`
    /// by id in the static tables; raises `ValueError` if either is
    /// unknown. `seed` is the deterministic Rng seed for this
    /// combat's stream (used for reward rolls, monster intents, draw
    /// shuffle, …).
    #[new]
    fn new(seed: u32, character: &str, encounter: &str) -> PyResult<Self> {
        let inner = build_env(seed, character, encounter)?;
        Ok(Self {
            inner,
            character_id: character.to_string(),
            encounter_id: encounter.to_string(),
            seed,
        })
    }

    /// Build a combat from an explicit monster list, bypassing the
    /// static encounter table. Useful when replaying a `.run` log
    /// whose multi-monster encounter (BowlbugsNormal, SlimesWeak,
    /// ToadpolesWeak, …) uses dynamic spawn logic the extractor can't
    /// follow — the .run records the actual monsters that spawned, so
    /// we pass those directly. `monsters` is a list of "Foo" model
    /// ids; slots are synthesized in order
    /// ("front", "back", "third", "fourth").
    #[staticmethod]
    fn from_monsters(
        seed: u32,
        character: &str,
        encounter_id: &str,
        monsters: Vec<String>,
    ) -> PyResult<Self> {
        let inner = build_env_from_monsters(seed, character, encounter_id, &monsters)?;
        Ok(Self {
            inner,
            character_id: character.to_string(),
            encounter_id: encounter_id.to_string(),
            seed,
        })
    }

    /// Reset to a fresh combat with the same character / encounter.
    /// Optionally takes a new seed.
    #[pyo3(signature = (seed=None))]
    fn reset(&mut self, seed: Option<u32>) -> PyResult<()> {
        if let Some(s) = seed {
            self.seed = s;
        }
        self.inner = build_env(self.seed, &self.character_id, &self.encounter_id)?;
        Ok(())
    }

    /// Apply one action. `action_json` deserializes into the Rust
    /// `Action` enum — e.g. `{"PlayCard":{"player_idx":0,
    /// "hand_idx":0,"target":["Enemy",0]}}` or `{"EndTurn":
    /// {"player_idx":0}}`. Returns StepOutcome serialized to JSON.
    fn step(&mut self, action_json: &str) -> PyResult<String> {
        let action: sim_env::Action = serde_json::from_str(action_json)
            .map_err(|e| PyValueError::new_err(format!("parsing action: {e}")))?;
        let out = self.inner.step(action);
        serde_json::to_string(&out)
            .map_err(|e| PyValueError::new_err(format!("serializing StepOutcome: {e}")))
    }

    /// JSON array of currently-legal Actions.
    fn legal_actions(&self) -> PyResult<String> {
        let actions = self.inner.legal_actions();
        serde_json::to_string(&actions)
            .map_err(|e| PyValueError::new_err(format!("serializing actions: {e}")))
    }

    /// Current CombatObservation as JSON. Caller does `json.loads`.
    fn observation(&self) -> PyResult<String> {
        let obs = features::observe_combat(&self.inner.state);
        serde_json::to_string(&obs)
            .map_err(|e| PyValueError::new_err(format!("serializing observation: {e}")))
    }

    /// Has the combat resolved? Convenience over inspecting
    /// `observation()`.
    fn is_terminal(&self) -> bool {
        self.inner.state.is_combat_over().is_some()
    }

    /// Round number (1-based).
    fn round_number(&self) -> i32 {
        self.inner.state.round_number
    }

    /// Snapshot for MCTS-style rollouts. Returns a new PyCombatEnv
    /// that can be advanced independently.
    fn clone_state(&self) -> Self {
        Self {
            inner: self.inner.clone_state(),
            character_id: self.character_id.clone(),
            encounter_id: self.encounter_id.clone(),
            seed: self.seed,
        }
    }

    /// Restore from a prior `clone_state`. Caller is responsible for
    /// passing a snapshot taken from the *same* env type.
    fn set_state(&mut self, snapshot: &PyCombatEnv) {
        self.inner = snapshot.inner.clone();
        self.character_id = snapshot.character_id.clone();
        self.encounter_id = snapshot.encounter_id.clone();
        self.seed = snapshot.seed;
    }
}

/// Slot names assigned to the 1st..Nth monster when no slot info is
/// otherwise available (the `.run`-driven harness path).
const DEFAULT_SLOTS: &[&str] = &["front", "back", "third", "fourth", "fifth"];

/// Map `.run`-file display-title aliases to canonical class names.
/// Today this covers the C# title-swap pattern (e.g. Doormaker shows
/// "Door" while its IsPortalOpen flag is false). Add more entries as
/// the corpus surfaces them.
fn normalize_monster_alias(id: &str) -> &str {
    match id {
        "Door" => "Doormaker",
        other => other,
    }
}

fn build_env_from_monsters(
    seed: u32,
    character_id: &str,
    encounter_id: &str,
    monster_ids: &[String],
) -> PyResult<sim_env::CombatEnv> {
    let ch = character::by_id(character_id)
        .ok_or_else(|| PyValueError::new_err(format!("unknown character: {character_id}")))?;
    let canonical: Vec<encounter::MonsterSpawn> = monster_ids
        .iter()
        .enumerate()
        .map(|(i, m)| encounter::MonsterSpawn {
            // Normalize .run-recorded model_id aliases to the
            // canonical class name. C# Doormaker switches its
            // displayed title to "Door" while IsPortalOpen=false;
            // some .run files capture that title instead of the
            // class name. Both refer to the same monster.
            monster: normalize_monster_alias(m).to_string(),
            slot: DEFAULT_SLOTS
                .get(i)
                .copied()
                .unwrap_or("")
                .to_string(),
        })
        .collect();
    let synthetic = encounter::EncounterData {
        id: encounter_id.to_string(),
        room_type: None,
        slots: Vec::new(),
        canonical_monsters: canonical,
        possible_monsters: monster_ids.to_vec(),
    };
    let deck = combat::deck_from_ids(&ch.starting_deck);
    let setup = combat::PlayerSetup {
        character: ch,
        current_hp: ch.starting_hp.unwrap_or(0),
        max_hp: ch.starting_hp.unwrap_or(0),
        deck,
        relics: ch.starting_relics.clone(),
    };
    Ok(sim_env::CombatEnv::reset(
        &synthetic,
        vec![setup],
        Vec::new(),
        seed,
    ))
}

fn build_env(
    seed: u32,
    character_id: &str,
    encounter_id: &str,
) -> PyResult<sim_env::CombatEnv> {
    let ch = character::by_id(character_id)
        .ok_or_else(|| PyValueError::new_err(format!("unknown character: {character_id}")))?;
    let enc = encounter::by_id(encounter_id)
        .ok_or_else(|| PyValueError::new_err(format!("unknown encounter: {encounter_id}")))?;
    let deck = combat::deck_from_ids(&ch.starting_deck);
    let setup = combat::PlayerSetup {
        character: ch,
        current_hp: ch.starting_hp.unwrap_or(0),
        max_hp: ch.starting_hp.unwrap_or(0),
        deck,
        relics: ch.starting_relics.clone(),
    };
    Ok(sim_env::CombatEnv::reset(enc, vec![setup], Vec::new(), seed))
}

// ---------- Module entrypoint ---------------------------------------

#[pymodule]
fn sts2_sim_py(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse_run_file, m)?)?;
    m.add_function(wrap_pyfunction!(card_count, m)?)?;
    m.add_function(wrap_pyfunction!(card_ids, m)?)?;
    m.add_function(wrap_pyfunction!(card_data, m)?)?;
    m.add_function(wrap_pyfunction!(relic_count, m)?)?;
    m.add_function(wrap_pyfunction!(power_count, m)?)?;
    m.add_function(wrap_pyfunction!(monster_count, m)?)?;
    m.add_function(wrap_pyfunction!(character_ids, m)?)?;
    m.add_function(wrap_pyfunction!(card_features_vec, m)?)?;
    m.add_function(wrap_pyfunction!(relic_features_vec, m)?)?;
    m.add_function(wrap_pyfunction!(observation_schema_version, m)?)?;
    m.add_function(wrap_pyfunction!(card_feature_dim, m)?)?;
    m.add_function(wrap_pyfunction!(relic_feature_dim, m)?)?;
    m.add_function(wrap_pyfunction!(creature_state_feature_dim, m)?)?;
    m.add_function(wrap_pyfunction!(monster_has_dispatch, m)?)?;
    m.add_class::<PyCombatEnv>()?;
    Ok(())
}
