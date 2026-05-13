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
use sts2_sim::run_state::RunState;

const CORPUS_DIR: &str =
    r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sample runs";

fn corpus() -> Vec<PathBuf> {
    let dir = PathBuf::from(CORPUS_DIR);
    if !dir.is_dir() {
        return Vec::new();
    }
    std::fs::read_dir(&dir)
        .expect("read corpus dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("run"))
        .collect()
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
        }
    }
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
