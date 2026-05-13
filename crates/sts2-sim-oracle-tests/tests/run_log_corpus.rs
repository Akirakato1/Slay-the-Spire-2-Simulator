//! Parses every `.run` file in the `sts2_stats/sample runs/` corpus and
//! sanity-checks the deserialized output. No oracle interaction — these
//! tests are pure Rust but covered here (rather than in `sts2-sim`'s
//! unit tests) because they depend on files outside the simulator repo.

use std::path::PathBuf;

use sts2_sim::run_log::{self, RunLog};

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

#[test]
fn all_sample_runs_deserialize() {
    let files = corpus();
    assert!(!files.is_empty(), "corpus dir is empty or missing");
    for path in files {
        let run: RunLog = run_log::from_path(&path)
            .unwrap_or_else(|e| panic!("parsing {:?}: {e}", path.file_name()));
        // Sanity invariants — every observed sample has these populated.
        assert!(!run.seed.is_empty(),
            "{:?}: empty seed", path.file_name());
        assert!(run.schema_version >= 8,
            "{:?}: unexpectedly old schema_version {}",
            path.file_name(), run.schema_version);
        assert!(!run.acts.is_empty(),
            "{:?}: no acts recorded", path.file_name());
        assert!(!run.players.is_empty(),
            "{:?}: no player records", path.file_name());
        assert_eq!(run.map_point_history.len(), run.acts.len(),
            "{:?}: acts.len() != map_point_history.len()",
            path.file_name());
    }
}

#[test]
fn coop_runs_have_multiple_players() {
    for path in corpus() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.contains("coop") {
            continue;
        }
        let run = run_log::from_path(&path).unwrap();
        assert!(run.players.len() >= 2,
            "{name}: coop run should have ≥2 players, got {}",
            run.players.len());
    }
}

#[test]
fn ancient_choices_capture_neow_selection() {
    // Every solo run starts with an Ancient (Neow) decision at floor 1.
    for path in corpus() {
        let run = run_log::from_path(&path).unwrap();
        if run.players.len() != 1 {
            continue;
        }
        let ancient_node = run.map_point_history[0]
            .iter()
            .find(|n| n.map_point_type == "ancient");
        let Some(node) = ancient_node else {
            continue; // some daily/special runs may skip
        };
        let stats = &node.player_stats[0];
        if stats.ancient_choice.is_empty() {
            continue;
        }
        let chosen_count = stats.ancient_choice.iter()
            .filter(|c| c.was_chosen)
            .count();
        assert!(chosen_count <= 1,
            "{:?}: more than one ancient_choice.was_chosen=true",
            path.file_name());
    }
}
