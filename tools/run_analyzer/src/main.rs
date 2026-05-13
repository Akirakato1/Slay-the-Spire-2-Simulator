//! `.run` file analyzer.
//!
//! Reads one or more `.run` files and emits a structured per-run summary
//! (JSON). This is the scaffolding for the user-facing analysis tool
//! described in [[project-user-facing-product]]. The current output is
//! deterministic; the eventual product layers per-decision agent
//! recommendations on top.
//!
//! Usage:
//!
//!     cargo run -p run-analyzer -- <path-to-run-file> [<more>...]
//!
//! Output is one JSON object per file on stdout. With no args, prints
//! usage to stderr.
//!
//! Architecture intentionally narrow: parses with the simulator's
//! `run_log` module, walks `map_point_history`, derives per-floor and
//! summary metadata. No agent queries, no simulator state reconstruction
//! yet — those land once combat replay is bit-exact against the corpus.

use anyhow::{Context, Result};
use serde::Serialize;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use sts2_sim::run_log::{self, NodeEntry, PlayerFinalState, PlayerStats, RunLog};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!(
            "usage: run-analyzer <path-to-run-file> [<more>...]\n\
             Emits one JSON RunSummary per file to stdout."
        );
        return ExitCode::from(2);
    }
    let mut any_error = false;
    for arg in &args {
        match analyze(PathBuf::from(arg)) {
            Ok(summary) => match serde_json::to_string_pretty(&summary) {
                Ok(json) => println!("{json}"),
                Err(e) => {
                    eprintln!("serialize {arg}: {e}");
                    any_error = true;
                }
            },
            Err(e) => {
                eprintln!("analyze {arg}: {e:#}");
                any_error = true;
            }
        }
    }
    if any_error {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[derive(Serialize)]
struct RunSummary {
    path: String,
    seed: String,
    ascension: i32,
    build_id: String,
    schema_version: i32,
    win: bool,
    was_abandoned: bool,
    killed_by_encounter: String,
    killed_by_event: String,
    run_time_seconds: i32,
    acts: Vec<String>,
    /// Total floors visited across all acts.
    total_floors: usize,
    /// Per-act floor count and node-type histogram.
    act_summaries: Vec<ActSummary>,
    player_count: usize,
    /// Per-player end-of-run summary (HP / gold / deck size / relics).
    final_states: Vec<FinalStateSummary>,
}

#[derive(Serialize)]
struct ActSummary {
    /// Act index (0-based) into `acts`.
    act_index: usize,
    /// Act name (e.g., "Overgrowth", "Hive").
    act_name: String,
    floor_count: usize,
    /// Node-type histogram: { "Monster": 4, "Event": 2, ... }.
    node_type_counts: std::collections::BTreeMap<String, usize>,
}

#[derive(Serialize)]
struct FinalStateSummary {
    /// Player identifier. Solo runs use 1; coop uses Steam IDs (17-digit).
    player_id: i64,
    /// Character class (matches `CharacterData.id`).
    character: String,
    /// HP / max HP / gold from the LAST recorded PlayerStats entry across
    /// the run. `None` if no PlayerStats were ever recorded for this
    /// player (e.g., player joined mid-run, coop edge cases).
    final_hp: Option<i32>,
    final_max_hp: Option<i32>,
    final_gold: Option<i32>,
    deck_size: usize,
    relic_count: usize,
    potion_count: usize,
}

fn analyze(path: PathBuf) -> Result<RunSummary> {
    let path_display = path.display().to_string();
    let log: RunLog = run_log::from_path(&path)
        .with_context(|| format!("reading {}", path_display))?;
    Ok(summarize(path_display, &log))
}

fn summarize(path: String, log: &RunLog) -> RunSummary {
    let total_floors: usize = log.map_point_history.iter().map(|act| act.len()).sum();
    let act_summaries = log
        .map_point_history
        .iter()
        .enumerate()
        .map(|(idx, nodes)| {
            let act_name = log.acts.get(idx).cloned().unwrap_or_default();
            ActSummary {
                act_index: idx,
                act_name,
                floor_count: nodes.len(),
                node_type_counts: node_histogram(nodes),
            }
        })
        .collect();
    let final_states = log
        .players
        .iter()
        .map(|p| final_state_summary(p, &log.map_point_history))
        .collect();
    RunSummary {
        path,
        seed: log.seed.clone(),
        ascension: log.ascension,
        build_id: log.build_id.clone(),
        schema_version: log.schema_version,
        win: log.win,
        was_abandoned: log.was_abandoned,
        killed_by_encounter: log.killed_by_encounter.clone(),
        killed_by_event: log.killed_by_event.clone(),
        run_time_seconds: log.run_time,
        acts: log.acts.clone(),
        total_floors,
        act_summaries,
        player_count: log.players.len(),
        final_states,
    }
}

/// Walk `map_point_history` in reverse and return the most recent
/// PlayerStats entry matching this player. `.run` files record HP / gold
/// per floor inside each NodeEntry's `player_stats` array; the last entry
/// is the run's terminal state.
fn last_player_stats<'a>(
    player_id: i64,
    history: &'a [Vec<NodeEntry>],
) -> Option<&'a PlayerStats> {
    for act in history.iter().rev() {
        for node in act.iter().rev() {
            for stats in node.player_stats.iter().rev() {
                if stats.player_id == player_id {
                    return Some(stats);
                }
            }
        }
    }
    None
}

fn final_state_summary(
    p: &PlayerFinalState,
    history: &[Vec<NodeEntry>],
) -> FinalStateSummary {
    let last = last_player_stats(p.id, history);
    FinalStateSummary {
        player_id: p.id,
        character: p.character.clone(),
        final_hp: last.map(|s| s.current_hp),
        final_max_hp: last.map(|s| s.max_hp),
        final_gold: last.map(|s| s.current_gold),
        deck_size: p.deck.len(),
        relic_count: p.relics.len(),
        potion_count: p.potions.len(),
    }
}

fn node_histogram(nodes: &[NodeEntry]) -> std::collections::BTreeMap<String, usize> {
    let mut counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for node in nodes {
        let key = if node.map_point_type.is_empty() {
            "Unknown".to_string()
        } else {
            node.map_point_type.clone()
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}
