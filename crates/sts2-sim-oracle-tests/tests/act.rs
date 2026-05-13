//! Diff tests for the ActModel surface (M-B): `GetMapPointTypes(Rng)` must
//! produce the same (NumOfUnknowns, NumOfRests, NumOfShops, NumOfElites)
//! tuple as the live game's concrete act classes.

use serde_json::json;
use sts2_sim::act::{
    ActModel, DeprecatedAct, Glory, Hive, Overgrowth, Underdocks,
};
use sts2_sim::rng::Rng;
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

fn dispose(oracle: &mut Oracle, handle: i64) {
    let _ = oracle.call("rng_dispose", json!({ "handle": handle }));
}

fn check_act(
    oracle: &mut Oracle,
    act_name: &str,
    rust_act: &dyn ActModel,
    seed: u32,
) {
    let mut rust_rng = Rng::new(seed, 0);
    let rust_counts = rust_act.get_map_point_types(&mut rust_rng);

    let rng_handle = new_rng_handle(oracle, seed, 0);
    let resp = oracle
        .call(
            "act_get_map_point_types",
            json!({ "act": act_name, "handle": rng_handle }),
        )
        .unwrap();

    let oracle_unknowns = resp["result"]["num_of_unknowns"].as_i64().unwrap() as i32;
    let oracle_rests = resp["result"]["num_of_rests"].as_i64().unwrap() as i32;
    let oracle_shops = resp["result"]["num_of_shops"].as_i64().unwrap() as i32;
    let oracle_elites = resp["result"]["num_of_elites"].as_i64().unwrap() as i32;

    assert_eq!(rust_counts.num_of_unknowns, oracle_unknowns,
        "{act_name} num_of_unknowns: seed={seed} rust={} oracle={oracle_unknowns}",
        rust_counts.num_of_unknowns);
    assert_eq!(rust_counts.num_of_rests, oracle_rests,
        "{act_name} num_of_rests: seed={seed} rust={} oracle={oracle_rests}",
        rust_counts.num_of_rests);
    assert_eq!(rust_counts.num_of_shops, oracle_shops,
        "{act_name} num_of_shops: seed={seed} rust={} oracle={oracle_shops}",
        rust_counts.num_of_shops);
    assert_eq!(rust_counts.num_of_elites, oracle_elites,
        "{act_name} num_of_elites: seed={seed} rust={} oracle={oracle_elites}",
        rust_counts.num_of_elites);

    // Also assert post-call counter alignment. Each act's PRNG consumption
    // is deterministic — divergence here means the order or method choices
    // diverge from C#.
    let resp = oracle.call("rng_counter", json!({ "handle": rng_handle })).unwrap();
    let oracle_counter = resp["result"].as_i64().unwrap() as i32;
    assert_eq!(rust_rng.counter(), oracle_counter,
        "{act_name} counter drift: seed={seed} rust={} oracle={oracle_counter}",
        rust_rng.counter());

    dispose(oracle, rng_handle);
}

#[test]
#[ignore = "requires built oracle-host"]
fn overgrowth_get_map_point_types_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x4F_5C_03_72_91_8B_AD_E2);
    for _ in 0..100 {
        check_act(&mut oracle, "Overgrowth", &Overgrowth, d.next_u32());
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn hive_get_map_point_types_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x97_4A_F1_B0_DD_24_67_38);
    for _ in 0..100 {
        check_act(&mut oracle, "Hive", &Hive, d.next_u32());
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn glory_get_map_point_types_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xE3_A0_57_64_18_CA_BB_71);
    for _ in 0..100 {
        check_act(&mut oracle, "Glory", &Glory, d.next_u32());
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn underdocks_get_map_point_types_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x6B_29_FA_8E_05_77_E1_CC);
    for _ in 0..100 {
        check_act(&mut oracle, "Underdocks", &Underdocks, d.next_u32());
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn deprecated_act_get_map_point_types_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x0A_BB_CC_DD_EE_FF_00_11);
    for _ in 0..20 {
        check_act(&mut oracle, "DeprecatedAct", &DeprecatedAct, d.next_u32());
    }
}
