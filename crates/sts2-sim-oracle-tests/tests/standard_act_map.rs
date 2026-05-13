//! Diff tests for `StandardActMap` (M-C + M-E): full constructor pipeline
//! (generate_map → assign_point_types → CenterGrid → SpreadAdjacent →
//! StraightenPaths) with pruning disabled.

use std::collections::BTreeSet;

use serde_json::{json, Value};
use sts2_sim::act::{ActModel, Glory, Hive, Overgrowth, Underdocks};
use sts2_sim::map::MapCoord;
use sts2_sim::rng::Rng;
use sts2_sim::standard_act_map::StandardActMap;
use sts2_sim_oracle_tests::Oracle;

struct Driver(u64);
impl Driver {
    fn new(seed: u64) -> Self { Self(seed) }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
    fn next_u32(&mut self) -> u32 { (self.next() >> 32) as u32 }
}

fn new_rng_handle(oracle: &mut Oracle, seed: u32, counter: i32) -> i64 {
    oracle
        .call("rng_new", json!({ "seed": seed, "counter": counter }))
        .expect("rng_new")["result"]
        .as_i64()
        .expect("non-integer handle")
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Clone)]
struct CanonicalPoint {
    col: i32,
    row: i32,
    point_type: i32,
    children: BTreeSet<(i32, i32)>,
    parents: BTreeSet<(i32, i32)>,
}

fn rust_canonical(sam: &StandardActMap) -> Vec<CanonicalPoint> {
    let mut out: Vec<CanonicalPoint> = sam.iter_grid_points()
        .map(|p| CanonicalPoint {
            col: p.coord.col,
            row: p.coord.row,
            point_type: p.point_type as i32,
            children: p.children.iter().map(|c| (c.col, c.row)).collect(),
            parents: p.parents.iter().map(|c| (c.col, c.row)).collect(),
        })
        .collect();
    out.sort();
    out
}

fn oracle_canonical(grid_points: &Value) -> Vec<CanonicalPoint> {
    let mut out: Vec<CanonicalPoint> = grid_points
        .as_array()
        .unwrap()
        .iter()
        .map(|p| CanonicalPoint {
            col: p["col"].as_i64().unwrap() as i32,
            row: p["row"].as_i64().unwrap() as i32,
            point_type: p["point_type"].as_i64().unwrap() as i32,
            children: p["children"].as_array().unwrap().iter()
                .map(|c| (c["col"].as_i64().unwrap() as i32, c["row"].as_i64().unwrap() as i32))
                .collect(),
            parents: p["parents"].as_array().unwrap().iter()
                .map(|c| (c["col"].as_i64().unwrap() as i32, c["row"].as_i64().unwrap() as i32))
                .collect(),
        })
        .collect();
    out.sort();
    out
}

fn compare_one(
    oracle: &mut Oracle,
    act_name: &str,
    rust_act: &dyn ActModel,
    seed: u32,
) {
    compare_one_with_pruning(oracle, act_name, rust_act, seed, false);
}

fn compare_one_with_pruning(
    oracle: &mut Oracle,
    act_name: &str,
    rust_act: &dyn ActModel,
    seed: u32,
    enable_pruning: bool,
) {
    let rust_rng = Rng::new(seed, 0);
    let rust_map = StandardActMap::new(rust_rng, rust_act, false, false, false, None, enable_pruning);

    let rng_handle = new_rng_handle(oracle, seed, 0);
    let resp = oracle.call(
        "standard_act_map_construct",
        json!({
            "act": act_name,
            "handle": rng_handle,
            "is_multiplayer": false,
            "replace_treasure_with_elites": false,
            "has_second_boss": false,
            "enable_pruning": enable_pruning,
        }),
    ).unwrap();

    if resp.get("error").is_some() {
        panic!(
            "oracle error on {act_name} seed={seed}: {}",
            resp["error"].as_str().unwrap_or("(non-string error)")
        );
    }
    let result = &resp["result"];
    assert_eq!(rust_map.cols(), result["cols"].as_i64().unwrap() as i32,
        "cols mismatch on {act_name} seed={seed}");
    assert_eq!(rust_map.rows(), result["rows"].as_i64().unwrap() as i32,
        "rows mismatch on {act_name} seed={seed}");

    let rust_points = rust_canonical(&rust_map);
    let oracle_points = oracle_canonical(&result["grid_points"]);

    if rust_points != oracle_points {
        // Format a useful diff for the first divergence.
        let mut msg = String::new();
        let max = rust_points.len().max(oracle_points.len());
        for i in 0..max {
            let r = rust_points.get(i);
            let o = oracle_points.get(i);
            if r != o {
                msg.push_str(&format!(
                    "first diff at index {i}: rust={r:?} oracle={o:?}\n"
                ));
                break;
            }
        }
        panic!(
            "{act_name} seed={seed} grid mismatch ({} rust pts vs {} oracle pts)\n{msg}",
            rust_points.len(),
            oracle_points.len()
        );
    }

    // Boss and starting are returned at top level; spot-check that coords
    // match.
    let oracle_boss_col = result["boss"]["col"].as_i64().unwrap() as i32;
    let oracle_boss_row = result["boss"]["row"].as_i64().unwrap() as i32;
    assert_eq!(rust_map.boss().coord, MapCoord::new(oracle_boss_col, oracle_boss_row),
        "boss coord mismatch on {act_name} seed={seed}");
}

#[test]
#[ignore = "requires built oracle-host"]
fn overgrowth_map_matches_for_random_seeds() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x1A2B_3C4D_5E6F_7080);
    for _ in 0..20 {
        compare_one(&mut oracle, "Overgrowth", &Overgrowth, d.next_u32());
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn hive_map_matches_for_random_seeds() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xAB_CD_01_23_45_67_89_EF);
    for _ in 0..20 {
        compare_one(&mut oracle, "Hive", &Hive, d.next_u32());
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn glory_map_matches_for_random_seeds() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xDE_AD_BE_EF_FA_CE_FE_ED);
    for _ in 0..20 {
        compare_one(&mut oracle, "Glory", &Glory, d.next_u32());
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn underdocks_map_matches_for_random_seeds() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x91_82_73_64_55_46_37_28);
    for _ in 0..20 {
        compare_one(&mut oracle, "Underdocks", &Underdocks, d.next_u32());
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn overgrowth_map_matches_with_pruning() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x77_88_99_AA_BB_CC_DD_EE);
    for _ in 0..20 {
        compare_one_with_pruning(&mut oracle, "Overgrowth", &Overgrowth, d.next_u32(), true);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn hive_map_matches_with_pruning() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x12_34_56_78_9A_BC_DE_F0);
    for _ in 0..20 {
        compare_one_with_pruning(&mut oracle, "Hive", &Hive, d.next_u32(), true);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn glory_map_matches_with_pruning() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xFE_DC_BA_98_76_54_32_10);
    for _ in 0..20 {
        compare_one_with_pruning(&mut oracle, "Glory", &Glory, d.next_u32(), true);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn underdocks_map_matches_with_pruning() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x55_55_AA_AA_33_33_CC_CC);
    for _ in 0..20 {
        compare_one_with_pruning(&mut oracle, "Underdocks", &Underdocks, d.next_u32(), true);
    }
}
