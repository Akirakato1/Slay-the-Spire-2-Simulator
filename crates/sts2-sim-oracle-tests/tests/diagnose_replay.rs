//! One-off diagnostic for the path-replay divergence.
//!
//! Picks a known-failing (run, act) tuple and tries to find any parameter
//! combination (replace_treasure_with_elites × has_second_boss ×
//! is_multiplayer) that produces a map supporting the recorded type
//! sequence. If one combo works, that's the missing parameter our
//! `enter_act` plumbing isn't capturing.

use serde_json::{json, Value};
use sts2_sim::map::MapPointType;
use sts2_sim::run_log::{self, NodeEntry};
use sts2_sim_oracle_tests::Oracle;

fn types_for(history: &[NodeEntry]) -> Vec<MapPointType> {
    history.iter().skip(1)
        .filter_map(|n| MapPointType::from_run_log_str(&n.map_point_type))
        .collect()
}

/// True if the (cols, rows, grid_points) serialized from the oracle
/// supports any DFS path from starting through the type sequence to the
/// boss.
fn map_supports_path(result: &Value, types: &[MapPointType]) -> bool {
    let cols = result["cols"].as_i64().unwrap() as i32;
    let rows = result["rows"].as_i64().unwrap() as i32;
    // Build a coord → (point_type, children) map from the serialized grid
    // plus boss & starting.
    use std::collections::HashMap;
    let mut type_at: HashMap<(i32, i32), i32> = HashMap::new();
    let mut children_of: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
    let mut record = |o: &Value| {
        let col = o["col"].as_i64().unwrap() as i32;
        let row = o["row"].as_i64().unwrap() as i32;
        let pt = o["point_type"].as_i64().unwrap() as i32;
        type_at.insert((col, row), pt);
        let kids: Vec<(i32, i32)> = o["children"].as_array().unwrap().iter()
            .map(|c| (c["col"].as_i64().unwrap() as i32,
                      c["row"].as_i64().unwrap() as i32))
            .collect();
        children_of.insert((col, row), kids);
    };
    for p in result["grid_points"].as_array().unwrap() {
        record(p);
    }
    record(&result["boss"]);
    record(&result["starting"]);
    let starting = (cols / 2, 0);

    fn dfs(
        cursor: (i32, i32),
        idx: usize,
        types: &[MapPointType],
        type_at: &std::collections::HashMap<(i32, i32), i32>,
        children_of: &std::collections::HashMap<(i32, i32), Vec<(i32, i32)>>,
    ) -> bool {
        if idx == types.len() {
            return true;
        }
        let target = types[idx] as i32;
        let kids = match children_of.get(&cursor) {
            Some(k) => k,
            None => return false,
        };
        let mut cands: Vec<(i32, i32)> = kids.iter()
            .filter(|c| type_at.get(c).copied().unwrap_or(-1) == target)
            .copied()
            .collect();
        cands.sort();
        for c in cands {
            if dfs(c, idx + 1, types, type_at, children_of) {
                return true;
            }
        }
        false
    }

    dfs(starting, 0, types, &type_at, &children_of)
}

#[test]
#[ignore = "diagnostic, requires built oracle-host"]
fn diagnose_underdocks_7n4_act0_param_search() {
    let log = run_log::from_path(
        r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sample runs\1775855780.run"
    ).unwrap();
    let act_idx = 0;
    let history = &log.map_point_history[act_idx];
    let types = types_for(history);
    eprintln!("recorded types ({} steps):", types.len());
    for (i, t) in types.iter().enumerate() {
        eprintln!("  {}: {:?}", i + 1, t);
    }

    let seed_uint = sts2_sim::hash::deterministic_hash_code(&log.seed) as u32;
    let map_name = format!("act_{}_map", act_idx + 1);
    eprintln!("seed={}, seed_uint={}, map_name={}",
        log.seed, seed_uint, map_name);

    let mut oracle = Oracle::spawn().expect("spawn oracle");

    for &replace_treasure in &[false, true] {
        for &has_second_boss in &[false, true] {
            for &is_multiplayer in &[false, true] {
                let h = oracle.call("rng_new_named",
                    json!({ "seed": seed_uint as i64, "name": map_name }))
                    .unwrap()["result"].as_i64().unwrap();
                let resp = oracle.call("standard_act_map_construct", json!({
                    "act": "Underdocks",
                    "handle": h,
                    "is_multiplayer": is_multiplayer,
                    "replace_treasure_with_elites": replace_treasure,
                    "has_second_boss": has_second_boss,
                    "enable_pruning": true,
                })).unwrap();
                let _ = oracle.call("rng_dispose", json!({ "handle": h }));
                if let Some(err) = resp.get("error") {
                    eprintln!("  ({replace_treasure}, {has_second_boss}, {is_multiplayer}): ERROR {err}");
                    continue;
                }
                let ok = map_supports_path(&resp["result"], &types);
                eprintln!(
                    "  replace_treasure={replace_treasure} has_second_boss={has_second_boss} is_multiplayer={is_multiplayer}: path_supported = {ok}",
                );
            }
        }
    }
}
