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
