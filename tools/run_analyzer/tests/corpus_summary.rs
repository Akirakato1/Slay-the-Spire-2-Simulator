//! Smoke test: parse one corpus run and assert the analyzer extracts
//! sane summary fields. Catches obvious regressions (path resolution,
//! field shape changes, etc.). Doesn't lock specific values — those
//! would change when the corpus is refreshed.

use std::path::PathBuf;
use std::process::Command;

const CORPUS_REL: &str = r"..\..\..\sample_run_103.2";

fn corpus_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join(CORPUS_REL)
}

fn first_run_file() -> Option<PathBuf> {
    let dir = corpus_dir();
    if !dir.exists() {
        return None;
    }
    std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|s| s.to_str()) == Some("run"))
}

#[test]
fn analyzer_emits_summary_for_corpus_run() {
    let Some(path) = first_run_file() else {
        eprintln!("skip: no .run files in corpus dir; nothing to analyze");
        return;
    };

    // Run the binary; assert exit 0 + JSON shape.
    let bin = env!("CARGO_BIN_EXE_run-analyzer");
    let out = Command::new(bin)
        .arg(&path)
        .output()
        .expect("run-analyzer binary");
    assert!(
        out.status.success(),
        "analyzer exited non-zero on {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("analyzer output is valid JSON");

    // Sanity-check the top-level fields exist and have plausible shapes.
    let obj = v.as_object().expect("top level is object");
    assert!(obj.contains_key("seed"));
    assert!(obj.contains_key("ascension"));
    assert!(obj.contains_key("acts"));
    assert!(obj.contains_key("act_summaries"));
    assert!(obj.contains_key("final_states"));
    let total_floors = obj["total_floors"].as_u64().expect("total_floors u64");
    assert!(total_floors > 0, "total_floors should be > 0");
    // Per-act summaries should sum to total_floors.
    let act_sum: u64 = obj["act_summaries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["floor_count"].as_u64().unwrap())
        .sum();
    assert_eq!(
        act_sum, total_floors,
        "per-act floor_count should sum to total_floors"
    );
}
