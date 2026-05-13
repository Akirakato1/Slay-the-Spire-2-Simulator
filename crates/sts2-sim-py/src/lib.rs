//! Python bindings for `sts2-sim`.
//!
//! Initial surface intentionally narrow — exposes the static data tables
//! (card / relic / power / monster counts and id listings) plus the
//! `.run` file parser. The agent-training / observation-pipeline work in
//! Python can iterate against this surface while the Rust side keeps
//! growing.
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
//!     import sts2_sim_py
//!     summary = sts2_sim_py.parse_run_file(r"path\to\file.run")
//!     # `summary` is a JSON string; load with json.loads to get a dict.
//!
//! ## API stability
//!
//! Functions return JSON strings rather than `#[pyclass]` wrappers, which
//! keeps the binding layer thin and lets us evolve the Rust types
//! freely. The Python side does the deserialization with `json.loads`.
//! When a wrapper grows enough to warrant typed access, replace the
//! `String` return with a `#[pyclass]` — the JSON shape is stable.

use pyo3::exceptions::{PyIOError, PyValueError};
use pyo3::prelude::*;
use sts2_sim::{card, monster, power, relic, run_log};

/// Parse a `.run` file and return its contents serialized as a JSON
/// string. Caller does `json.loads(s)` to get a Python dict.
///
/// Raises `IOError` for missing/unreadable files, `ValueError` for JSON
/// that doesn't match the expected schema.
#[pyfunction]
fn parse_run_file(path: &str) -> PyResult<String> {
    let log = run_log::from_path(path)
        .map_err(|e| PyIOError::new_err(format!("reading {path}: {e}")))?;
    serde_json::to_string(&log).map_err(|e| {
        PyValueError::new_err(format!("serializing RunLog: {e}"))
    })
}

/// Number of cards in the static table (577 as of v0.103.2).
#[pyfunction]
fn card_count() -> usize {
    card::ALL_CARDS.len()
}

/// All card ids, sorted (stable order).
#[pyfunction]
fn card_ids() -> Vec<String> {
    card::ALL_CARDS.iter().map(|c| c.id.clone()).collect()
}

/// Look up one card's data as JSON. Returns `None` if id isn't in the
/// table.
#[pyfunction]
fn card_data(id: &str) -> PyResult<Option<String>> {
    let Some(c) = card::by_id(id) else {
        return Ok(None);
    };
    serde_json::to_string(c)
        .map(Some)
        .map_err(|e| PyValueError::new_err(format!("serializing card {id}: {e}")))
}

/// Number of relics in the static table (294).
#[pyfunction]
fn relic_count() -> usize {
    relic::ALL_RELICS.len()
}

/// Number of powers in the static table (243).
#[pyfunction]
fn power_count() -> usize {
    power::ALL_POWERS.len()
}

/// Number of monsters in the static table (117).
#[pyfunction]
fn monster_count() -> usize {
    monster::ALL_MONSTERS.len()
}

/// All character ids — useful for agent training scaffolding.
#[pyfunction]
fn character_ids() -> Vec<String> {
    use sts2_sim::character;
    character::ALL_CHARACTERS
        .iter()
        .map(|c| c.id.clone())
        .collect()
}

/// Module entrypoint. PyO3 registers this name as the imported module
/// (`import sts2_sim_py`).
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
    Ok(())
}
