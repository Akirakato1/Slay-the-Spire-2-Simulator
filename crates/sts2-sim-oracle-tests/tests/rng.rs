//! Diff tests for sts2-sim's Rng port against the live game DLL.
//!
//! Each test spawns one oracle-host instance and runs a randomized batch of
//! operations through both Rust and the oracle, asserting bit-exact equality.
//! All tests are #[ignore]'d by default because they require the host to be
//! pre-built (`dotnet build oracle-host -c Release`).

use serde_json::{json, Value};
use sts2_sim::rng::Rng;
use sts2_sim_oracle_tests::Oracle;

/// A small deterministic LCG used only to vary the inputs we feed to both
/// implementations. Independent from the Rng under test.
struct Driver(u64);

impl Driver {
    fn new(seed: u64) -> Self { Self(seed) }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
    fn next_u32(&mut self) -> u32 { (self.next() >> 32) as u32 }
    fn next_i32(&mut self) -> i32 { self.next_u32() as i32 }
    fn next_pos_i32(&mut self) -> i32 { (self.next_u32() & 0x7FFF_FFFF) as i32 }
    fn next_range(&mut self, max: u32) -> u32 { (self.next() % max as u64) as u32 }
}

fn new_handle(oracle: &mut Oracle, seed: u32, counter: i32) -> i64 {
    let resp = oracle
        .call("rng_new", json!({ "seed": seed, "counter": counter }))
        .expect("rng_new");
    resp["result"].as_i64().expect("rng_new returned non-integer")
}

fn dispose(oracle: &mut Oracle, handle: i64) {
    let _ = oracle.call("rng_dispose", json!({ "handle": handle }));
}

#[test]
#[ignore = "requires built oracle-host"]
fn next_int_single_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xC0FFEE_C0FFEE);

    for _ in 0..200 {
        let seed = d.next_u32();
        let counter = (d.next_range(1024)) as i32;
        let mut rust = Rng::new(seed, counter);
        let handle = new_handle(&mut oracle, seed, counter);

        for _ in 0..100 {
            // pick max in [0, i32::MAX]
            let max = match d.next_range(8) {
                0 => 0,
                1 => 1,
                2 => 2,
                3 => i32::MAX,
                _ => d.next_pos_i32().max(1),
            };
            let rust_v = rust.next_int(max);
            let resp = oracle
                .call("rng_next_int", json!({ "handle": handle, "max_exclusive": max }))
                .expect("rng_next_int");
            let oracle_v = resp["result"].as_i64().unwrap() as i32;
            assert_eq!(
                rust_v, oracle_v,
                "next_int mismatch: seed={seed} counter={counter} max={max} rust={rust_v} oracle={oracle_v}"
            );
        }

        // Counter must also match.
        let resp = oracle.call("rng_counter", json!({ "handle": handle })).unwrap();
        let oracle_counter = resp["result"].as_i64().unwrap() as i32;
        assert_eq!(rust.counter(), oracle_counter, "counter drift");
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn next_int_range_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xDEADBEEF_DEADBEEF);

    for _ in 0..100 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..100 {
            // pick a small-range [min, max) with min < max.
            let lo = d.next_i32() / 2; // avoid overflow
            let span = (d.next_range(10_000) + 1) as i32;
            let hi = lo.saturating_add(span);
            let rust_v = rust.next_int_range(lo, hi);
            let resp = oracle.call(
                "rng_next_int_range",
                json!({ "handle": handle, "min_inclusive": lo, "max_exclusive": hi }),
            ).expect("rng_next_int_range");
            let oracle_v = resp["result"].as_i64().unwrap() as i32;
            assert_eq!(rust_v, oracle_v,
                "next_int_range mismatch: seed={seed} lo={lo} hi={hi} rust={rust_v} oracle={oracle_v}");
        }
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn next_bool_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x12345678_87654321);

    for _ in 0..50 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);
        for _ in 0..200 {
            let rust_v = rust.next_bool();
            let resp = oracle.call("rng_next_bool", json!({ "handle": handle })).unwrap();
            let oracle_v = resp["result"].as_bool().unwrap();
            assert_eq!(rust_v, oracle_v, "next_bool mismatch at seed={seed}");
        }
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn fast_forward_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xABCDEF01_23456789);

    for _ in 0..50 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        // Fast-forward both by the same target, then ensure subsequent draws agree.
        let target = d.next_range(10_000) as i32 + 1;
        rust.fast_forward_counter(target);
        oracle.call("rng_fast_forward",
            json!({ "handle": handle, "target_count": target })).unwrap();

        let resp = oracle.call("rng_counter", json!({ "handle": handle })).unwrap();
        let oracle_counter = resp["result"].as_i64().unwrap() as i32;
        assert_eq!(rust.counter(), oracle_counter, "counter mismatch post-fast-forward");

        // Drain a few NextInts to confirm state alignment.
        for _ in 0..20 {
            let rust_v = rust.next_int(1_000_000);
            let oracle_v = oracle
                .call("rng_next_int",
                    json!({ "handle": handle, "max_exclusive": 1_000_000 }))
                .unwrap()["result"].as_i64().unwrap() as i32;
            assert_eq!(rust_v, oracle_v,
                "post-fast-forward draws diverge: seed={seed} target={target}");
        }
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn shuffle_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xFEEDFACE_CAFEBABE);

    for _ in 0..50 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..20 {
            let n = (d.next_range(40) + 1) as usize;
            let mut rust_list: Vec<i32> = (0..n as i32).collect();
            let json_list: Value = json!(rust_list.iter().collect::<Vec<_>>());

            rust.shuffle(&mut rust_list);
            let resp = oracle.call(
                "rng_shuffle",
                json!({ "handle": handle, "list": json_list }),
            ).expect("rng_shuffle");
            let oracle_list: Vec<i32> = resp["result"].as_array().unwrap()
                .iter().map(|v| v.as_i64().unwrap() as i32).collect();

            assert_eq!(rust_list, oracle_list,
                "shuffle mismatch: seed={seed} n={n} rust={rust_list:?} oracle={oracle_list:?}");
        }
        dispose(&mut oracle, handle);
    }
}
