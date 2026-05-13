//! Drives every `.run` file in the sample corpus through
//! `RunState::from_run_log` and through `enter_act` for each recorded act.
//! Validates that the pipeline doesn't panic and that each generated map
//! looks structurally sane (boss/starting in place, every grid point has
//! a type assigned, every grid point is connected to at least one
//! neighbor — no orphans).
//!
//! Defers full path-replay (asserting the player's recorded sequence of
//! visited types is realizable in our generated map) until we have a
//! `CurrentMapCoord` tracker on `RunState`.

use std::path::PathBuf;

use sts2_sim::map::MapPointType;
use sts2_sim::run_log::{self, RunLog};
use sts2_sim::run_state::{replay_act_log, RunState};

/// Returns the gold balance after every node in act+floor order for one
/// player, plus the (current, gained, lost, spent, stolen) tuple. Skips
/// nodes with no stats for that player.
fn gold_trace_for(log: &RunLog, player_id: i64) -> Vec<(i32, i32, i32, i32, i32)> {
    let mut out = Vec::new();
    for act in &log.map_point_history {
        for node in act {
            if let Some(s) = node.player_stats.iter().find(|s| s.player_id == player_id) {
                out.push((s.current_gold, s.gold_gained, s.gold_lost, s.gold_spent, s.gold_stolen));
            }
        }
    }
    out
}

const CORPUS_DIRS: &[&str] = &[
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sample runs",
    r"C:\Users\zhuyl\OneDrive\Desktop\STS2 RL\sample_run_103.2",
];

fn corpus() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen_names = std::collections::HashSet::new();
    for d in CORPUS_DIRS {
        let dir = PathBuf::from(d);
        if !dir.is_dir() {
            continue;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("run") {
                continue;
            }
            // Skip duplicates across corpus dirs (one file appears in both).
            let name = p.file_name().unwrap().to_owned();
            if seen_names.insert(name) {
                out.push(p);
            }
        }
    }
    out
}

fn parse(p: &PathBuf) -> RunLog {
    run_log::from_path(p).expect("parse .run file")
}

#[test]
fn every_corpus_run_constructs_runstate() {
    let files = corpus();
    assert!(!files.is_empty(), "corpus dir missing or empty");
    for path in files {
        let log = parse(&path);
        let rs = RunState::from_run_log(&log).unwrap_or_else(|| {
            panic!("unrecognized act id in {:?}: acts={:?}",
                path.file_name(), log.acts);
        });
        assert_eq!(rs.acts().len(), log.acts.len());
        assert_eq!(rs.players().len(), log.players.len());
        assert_eq!(rs.seed_string(), log.seed);
        assert_eq!(rs.ascension(), log.ascension);
    }
}

#[test]
fn every_corpus_act_generates_clean_map() {
    for path in corpus() {
        let log = parse(&path);
        let mut rs = match RunState::from_run_log(&log) {
            Some(r) => r,
            None => continue,
        };
        let act_count = rs.acts().len() as i32;
        for act_idx in 0..act_count {
            let has_sb = rs.act_has_second_boss(act_idx);
            let map = rs.enter_act(act_idx);
            // Bounds: boss at (cols/2, rows), starting at (cols/2, 0).
            let cols = map.cols();
            let rows = map.rows();
            assert_eq!(map.boss().coord.col, cols / 2,
                "{:?} act {}: boss col", path.file_name(), act_idx);
            assert_eq!(map.boss().coord.row, rows,
                "{:?} act {}: boss row", path.file_name(), act_idx);
            assert_eq!(map.starting().coord.col, cols / 2,
                "{:?} act {}: starting col", path.file_name(), act_idx);
            assert_eq!(map.starting().coord.row, 0,
                "{:?} act {}: starting row", path.file_name(), act_idx);

            // Every grid point must have a type assigned, and must be
            // connected to at least one neighbor (no orphans).
            for p in map.iter_grid_points() {
                assert_ne!(p.point_type, MapPointType::Unassigned,
                    "{:?} act {}: point {:?} left Unassigned",
                    path.file_name(), act_idx, p.coord);
                assert!(!p.parents.is_empty() || !p.children.is_empty(),
                    "{:?} act {}: orphan at {:?}",
                    path.file_name(), act_idx, p.coord);
            }

            // If the run had a second boss for this act, our regenerated
            // map should also have one (at row=rows+1).
            if has_sb {
                let sb = map.second_boss().expect(
                    "act flagged as second-boss but map.second_boss() is None"
                );
                assert_eq!(sb.coord.col, map.cols() / 2);
                assert_eq!(sb.coord.row, map.rows() + 1);
            } else {
                assert!(map.second_boss().is_none(),
                    "{:?} act {}: unexpected second_boss when run didn't flag one",
                    path.file_name(), act_idx);
            }
        }
    }
}

/// Best-effort path replay across the corpus. Counts how many (run, act)
/// pairs replay cleanly through our generated map vs. how many produce
/// no valid path. Failures here mean our generated map structurally
/// differs from the map the recorded run experienced — that is a real
/// gap (we're missing some map-gen-affecting parameter the game uses,
/// likely character / ascension / modifier-driven), but the strict
/// assertion is deferred until we've ported enough of those subsystems
/// to diagnose properly.
///
/// We still assert that the replay machinery itself works (any success
/// at all) and that recorded sequences are at least *structurally*
/// well-formed (start with "ancient", end with "boss").
#[test]
fn corpus_player_runtime_state_populates_from_log() {
    for path in corpus() {
        let log = parse(&path);
        let rs = match RunState::from_run_log(&log) {
            Some(r) => r,
            None => continue,
        };
        assert_eq!(rs.players().len(), log.players.len());
        for (i, p) in rs.players().iter().enumerate() {
            let logged = &log.players[i];
            assert_eq!(p.character_id, logged.character);
            assert_eq!(p.id, logged.id);
            assert_eq!(p.deck.len(), logged.deck.len(),
                "{:?} player {}: deck size mismatch", path.file_name(), i);
            assert_eq!(p.relics.len(), logged.relics.len(),
                "{:?} player {}: relic count mismatch", path.file_name(), i);
            assert_eq!(p.potions.len(), logged.potions.len(),
                "{:?} player {}: potion count mismatch", path.file_name(), i);
            assert_eq!(p.max_potion_slot_count, logged.max_potion_slot_count);

            // HP and gold must be present (final stats recorded for every
            // run we sampled). Sanity bounds: gold non-negative,
            // current_hp <= max_hp (or =0 if the player died at floor end).
            assert!(p.gold >= 0,
                "{:?} player {}: final gold {} negative",
                path.file_name(), i, p.gold);
            assert!(p.max_hp > 0 || log.was_abandoned,
                "{:?} player {}: max_hp = {} on non-abandoned run",
                path.file_name(), i, p.max_hp);
        }
    }
}

/// Per-floor HP stats: (current_hp, max_hp, damage_taken, hp_healed,
/// max_hp_gained, max_hp_lost) in act+floor order for one player.
fn hp_trace_for(log: &RunLog, player_id: i64) -> Vec<(i32, i32, i32, i32, i32, i32)> {
    let mut out = Vec::new();
    for act in &log.map_point_history {
        for node in act {
            if let Some(s) = node.player_stats.iter().find(|s| s.player_id == player_id) {
                out.push((s.current_hp, s.max_hp, s.damage_taken, s.hp_healed,
                    s.max_hp_gained, s.max_hp_lost));
            }
        }
    }
    out
}

#[test]
fn corpus_max_hp_accounting_consistent_per_floor() {
    // max_hp moves linearly with gained/lost — no capping logic to worry
    // about. Strict equality.
    let mut total_checks = 0;
    let mut divergences: Vec<String> = Vec::new();
    for path in corpus() {
        let log = parse(&path);
        if log.players.len() > 1 { continue }
        let pid = log.players[0].id;
        let trace = hp_trace_for(&log, pid);
        for w in trace.windows(2) {
            let (_, prev_max, _, _, _, _) = w[0];
            let (_, curr_max, _, _, gained, lost) = w[1];
            let expected = prev_max + gained - lost;
            total_checks += 1;
            if expected != curr_max {
                divergences.push(format!(
                    "{:?}: prev_max={prev_max} +{gained} -{lost} = {expected}, recorded {curr_max}",
                    path.file_name(),
                ));
            }
        }
    }
    eprintln!("max_hp accounting: {total_checks} checks, {} divergences",
        divergences.len());
    for d in divergences.iter().take(5) {
        eprintln!("  {d}");
    }
    assert!(total_checks > 0);
    assert!(divergences.is_empty(),
        "{} max_hp divergences", divergences.len());
}

#[test]
fn corpus_current_hp_within_max_hp_each_floor() {
    // current_hp never exceeds max_hp at end of any floor. (Edge case:
    // hp_healed > damage_taken can push past the previous max_hp but
    // current_hp is capped to max_hp afterwards.)
    let mut total = 0;
    let mut violations = Vec::new();
    for path in corpus() {
        let log = parse(&path);
        if log.players.len() > 1 { continue }
        let pid = log.players[0].id;
        for (curr_hp, max_hp, _, _, _, _) in hp_trace_for(&log, pid) {
            total += 1;
            if curr_hp > max_hp {
                violations.push(format!(
                    "{:?}: curr_hp={curr_hp} > max_hp={max_hp}",
                    path.file_name(),
                ));
            }
            if curr_hp < 0 {
                violations.push(format!(
                    "{:?}: curr_hp={curr_hp} negative",
                    path.file_name(),
                ));
            }
        }
    }
    eprintln!("current_hp invariant: {total} checks, {} violations",
        violations.len());
    for v in violations.iter().take(5) {
        eprintln!("  {v}");
    }
    assert!(total > 0);
    assert!(violations.is_empty(),
        "{} current_hp invariant violations", violations.len());
}

#[test]
fn corpus_gold_accounting_consistent_per_floor() {
    // For each solo run, walk the recorded per-floor gold stats and check
    // that prev_gold + gained - lost - spent - stolen == current_gold.
    // Records the first divergence so failures are diagnosable.
    let mut total_checks = 0;
    let mut divergences: Vec<String> = Vec::new();
    for path in corpus() {
        let log = parse(&path);
        if log.players.len() > 1 {
            // Multi-player gold accounting may share pools / interleave;
            // defer until we port the coop money model.
            continue;
        }
        let pid = log.players[0].id;
        let trace = gold_trace_for(&log, pid);
        for w in trace.windows(2) {
            let (prev_gold, _, _, _, _) = w[0];
            let (curr_gold, gained, lost, spent, stolen) = w[1];
            let expected = prev_gold + gained - lost - spent - stolen;
            total_checks += 1;
            if expected != curr_gold {
                divergences.push(format!(
                    "{:?}: prev={prev_gold} +{gained} -{lost} -{spent} -{stolen} = {expected}, recorded {curr_gold}",
                    path.file_name(),
                ));
            }
        }
    }
    eprintln!("gold accounting: {total_checks} checks, {} divergences",
        divergences.len());
    for d in divergences.iter().take(5) {
        eprintln!("  {d}");
    }
    assert!(total_checks > 0, "no gold checks ran");
    assert!(divergences.is_empty(),
        "{} gold-accounting divergences", divergences.len());
}

#[test]
fn corpus_path_replay_best_effort() {
    let mut total = 0;
    let mut ok = 0;
    let mut failures: Vec<String> = Vec::new();
    for path in corpus() {
        let log = parse(&path);
        if log.players.len() > 1 {
            continue;
        }
        let mut rs = match RunState::from_run_log(&log) {
            Some(r) => r,
            None => continue,
        };
        for (act_idx, history) in log.map_point_history.iter().enumerate() {
            // Structural: first node is starting ancient.
            assert_eq!(history.first().map(|n| n.map_point_type.as_str()),
                Some("ancient"),
                "{:?} act {}: first node should be ancient",
                path.file_name(), act_idx);
            // Skip acts where the player died mid-floor — replay only
            // makes sense for complete acts that end at the boss.
            let last_is_boss = history.last()
                .map(|n| n.map_point_type == "boss")
                .unwrap_or(false);
            if !last_is_boss {
                continue;
            }

            rs.enter_act(act_idx as i32);
            total += 1;
            match replay_act_log(&mut rs, history) {
                Ok(outcome) => {
                    if outcome.reached_boss
                        && outcome.advanced_floors == history.len() as i32 - 1
                    {
                        ok += 1;
                    } else {
                        failures.push(format!(
                            "{:?} act {} ({:?}): partial replay (advanced {}, reached_boss {})",
                            path.file_name(), act_idx, rs.acts()[act_idx],
                            outcome.advanced_floors, outcome.reached_boss,
                        ));
                    }
                }
                Err(e) => {
                    failures.push(format!(
                        "{:?} act {} ({:?}): {e}",
                        path.file_name(), act_idx, rs.acts()[act_idx],
                    ));
                }
            }
        }
    }
    eprintln!(
        "path replay summary: {ok}/{total} acts replayed cleanly. \
         Failures ({}):", failures.len(),
    );
    for f in &failures {
        eprintln!("  - {f}");
    }
    // The replay machinery itself must work — at least one success.
    assert!(ok > 0,
        "no acts replayed cleanly; replay machinery may be broken");
}

#[test]
fn corpus_node_counts_match_act_lengths() {
    // The .run `map_point_history` records every node visited. For a solo
    // run with no second-boss act it's:
    //   1 starting (Ancient) + GetNumberOfRooms in-grid floors + 1 Boss
    // = GetNumberOfRooms + 2.
    // Acts with a second boss add one more node (the second boss); we
    // detect that by counting trailing "boss" entries in the history.
    for path in corpus() {
        let log = parse(&path);
        let Some(rs) = RunState::from_run_log(&log) else { continue };
        for (act_idx, history_per_act) in log.map_point_history.iter().enumerate() {
            if log.players.len() > 1 {
                continue;
            }
            // Skip incomplete acts (player died mid-floor).
            let last_is_boss = history_per_act
                .last()
                .map(|n| n.map_point_type == "boss")
                .unwrap_or(false);
            if !last_is_boss {
                continue;
            }
            let act = sts2_sim::act::act_for(rs.acts()[act_idx]);
            let trailing_bosses = history_per_act
                .iter()
                .rev()
                .take_while(|n| n.map_point_type == "boss")
                .count() as i32;
            let expected_floors =
                act.get_number_of_rooms(false) + 1 + trailing_bosses;
            assert_eq!(
                history_per_act.len() as i32,
                expected_floors,
                "{:?} act {} ({:?}): history records {} nodes; act says {} (incl. {} boss)",
                path.file_name(),
                act_idx,
                rs.acts()[act_idx],
                history_per_act.len(),
                expected_floors,
                trailing_bosses,
            );
        }
    }
}
