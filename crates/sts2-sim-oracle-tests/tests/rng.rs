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
fn next_double_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x1234_5678_ABCD_EF01);

    for _ in 0..50 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..200 {
            let rust_v = rust.next_double();
            let resp = oracle.call("rng_next_double", json!({ "handle": handle })).unwrap();
            let oracle_bits = resp["result"].as_i64().unwrap();
            let oracle_v = f64::from_bits(oracle_bits as u64);
            assert_eq!(
                rust_v.to_bits(),
                oracle_v.to_bits(),
                "next_double mismatch (seed={seed}): rust={rust_v} oracle={oracle_v}"
            );
        }
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn next_double_range_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xFACE_FEED_BAAD_F00D);

    for _ in 0..50 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..100 {
            let lo = (d.next_i32() as f64) * 0.001;
            let hi = lo + (d.next_range(1_000_000) as f64 + 1.0) * 0.001;
            let rust_v = rust.next_double_range(lo, hi);
            let resp = oracle.call(
                "rng_next_double_range",
                json!({
                    "handle": handle,
                    "min_bits": lo.to_bits() as i64,
                    "max_bits": hi.to_bits() as i64,
                }),
            ).unwrap();
            let oracle_v = f64::from_bits(resp["result"].as_i64().unwrap() as u64);
            assert_eq!(
                rust_v.to_bits(),
                oracle_v.to_bits(),
                "next_double_range mismatch (seed={seed}, lo={lo}, hi={hi}): rust={rust_v} oracle={oracle_v}"
            );
        }
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn next_float_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xBADC_0FFE_E0DD_F00D);

    for _ in 0..50 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..100 {
            let max = ((d.next_range(1000) + 1) as f32) * 0.5;
            let rust_v = rust.next_float(max);
            let resp = oracle.call(
                "rng_next_float",
                json!({ "handle": handle, "max_bits": max.to_bits() as i32 }),
            ).unwrap();
            let oracle_v = f32::from_bits(resp["result"].as_i64().unwrap() as u32);
            assert_eq!(
                rust_v.to_bits(),
                oracle_v.to_bits(),
                "next_float mismatch (seed={seed}, max={max}): rust={rust_v} oracle={oracle_v}"
            );
        }
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn next_float_range_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xCAFE_BABE_DEAD_BEEF);

    for _ in 0..50 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..100 {
            let lo = (d.next_i32() as f32) * 0.0001;
            let span = ((d.next_range(10_000) + 1) as f32) * 0.0001;
            let hi = lo + span;
            let rust_v = rust.next_float_range(lo, hi);
            let resp = oracle.call(
                "rng_next_float_range",
                json!({
                    "handle": handle,
                    "min_bits": lo.to_bits() as i32,
                    "max_bits": hi.to_bits() as i32,
                }),
            ).unwrap();
            let oracle_v = f32::from_bits(resp["result"].as_i64().unwrap() as u32);
            assert_eq!(
                rust_v.to_bits(),
                oracle_v.to_bits(),
                "next_float_range mismatch (seed={seed}, lo={lo}, hi={hi}): rust={rust_v} oracle={oracle_v}"
            );
        }
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn next_uint_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x9E37_79B9_7F4A_7C15);

    for _ in 0..50 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..100 {
            let max = d.next_u32().max(1);
            let rust_v = rust.next_unsigned_int(max);
            let resp = oracle.call(
                "rng_next_uint",
                json!({ "handle": handle, "max_exclusive": max as i64 }),
            ).unwrap();
            let oracle_v = resp["result"].as_i64().unwrap() as u32;
            assert_eq!(rust_v, oracle_v,
                "next_uint mismatch (seed={seed}, max={max}): rust={rust_v} oracle={oracle_v}");
        }
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn next_uint_range_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x517C_C1B7_2722_0A95);

    for _ in 0..50 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..100 {
            let min = d.next_u32() / 2;
            let span = d.next_u32().max(1) / 2 + 1;
            let max = min.saturating_add(span);
            let rust_v = rust.next_unsigned_int_range(min, max);
            let resp = oracle.call(
                "rng_next_uint_range",
                json!({
                    "handle": handle,
                    "min_inclusive": min as i64,
                    "max_exclusive": max as i64,
                }),
            ).unwrap();
            let oracle_v = resp["result"].as_i64().unwrap() as u32;
            assert_eq!(rust_v, oracle_v,
                "next_uint_range mismatch (seed={seed}, min={min}, max={max}): rust={rust_v} oracle={oracle_v}");
        }
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn next_gaussian_double_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x1F_6B_70_5A_7C_D9_E1_03);

    // Use parameters where the rejection loop accepts quickly (mean in [0,1],
    // stdDev small) so tests don't hang chasing rare acceptance. Counter
    // alignment is checked after each call.
    for _ in 0..40 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..80 {
            let mean = 0.3 + (d.next_range(1000) as f64) * 0.0004; // [0.3, 0.7]
            let std = 0.05 + (d.next_range(1000) as f64) * 0.00015; // [0.05, 0.2]
            let lo = (d.next_i32() as f64) * 0.001;
            let span = (d.next_range(10_000) as f64 + 1.0) * 0.001;
            let hi = lo + span;

            let rust_v = rust.next_gaussian_double(mean, std, lo, hi);
            let resp = oracle.call(
                "rng_next_gaussian_double",
                json!({
                    "handle": handle,
                    "mean_bits": mean.to_bits() as i64,
                    "std_dev_bits": std.to_bits() as i64,
                    "min_bits": lo.to_bits() as i64,
                    "max_bits": hi.to_bits() as i64,
                }),
            ).unwrap();
            let oracle_v = f64::from_bits(resp["result"].as_i64().unwrap() as u64);
            assert_eq!(
                rust_v.to_bits(),
                oracle_v.to_bits(),
                "next_gaussian_double mismatch (seed={seed}, mean={mean}, std={std}, lo={lo}, hi={hi}): rust={rust_v} oracle={oracle_v}"
            );

            let counter_resp = oracle
                .call("rng_counter", json!({ "handle": handle })).unwrap();
            assert_eq!(
                rust.counter(),
                counter_resp["result"].as_i64().unwrap() as i32,
                "counter drift after next_gaussian_double (seed={seed})"
            );
        }
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn next_gaussian_float_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x3A_B7_C4_91_E5_02_77_F1);

    for _ in 0..40 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..50 {
            let mean = 0.3f32 + (d.next_range(1000) as f32) * 0.0004;
            let std = 0.05f32 + (d.next_range(1000) as f32) * 0.00015;
            let lo = (d.next_i32() as f32) * 0.001;
            let hi = lo + ((d.next_range(10_000) as f32) + 1.0) * 0.001;

            let rust_v = rust.next_gaussian_float(mean, std, lo, hi);
            let resp = oracle.call(
                "rng_next_gaussian_float",
                json!({
                    "handle": handle,
                    "mean_bits": mean.to_bits() as i32,
                    "std_dev_bits": std.to_bits() as i32,
                    "min_bits": lo.to_bits() as i32,
                    "max_bits": hi.to_bits() as i32,
                }),
            ).unwrap();
            let oracle_v = f32::from_bits(resp["result"].as_i64().unwrap() as u32);
            assert_eq!(
                rust_v.to_bits(),
                oracle_v.to_bits(),
                "next_gaussian_float mismatch (seed={seed}, mean={mean}, std={std}, lo={lo}, hi={hi}): rust={rust_v} oracle={oracle_v}"
            );
        }
        dispose(&mut oracle, handle);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn next_gaussian_int_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0xE3_07_2B_55_18_AD_BC_4F);

    for _ in 0..40 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..50 {
            // Wide accepting range so the rejection loop terminates quickly.
            let mean = (d.next_i32() % 100).clamp(-100, 100);
            let std_dev = ((d.next_range(50) + 1) as i32).abs();
            let min = mean - 500;
            let max = mean + 500;

            let rust_v = rust.next_gaussian_int(mean, std_dev, min, max);
            let resp = oracle.call(
                "rng_next_gaussian_int",
                json!({
                    "handle": handle,
                    "mean": mean,
                    "std_dev": std_dev,
                    "min": min,
                    "max": max,
                }),
            ).unwrap();
            let oracle_v = resp["result"].as_i64().unwrap() as i32;
            assert_eq!(
                rust_v, oracle_v,
                "next_gaussian_int mismatch (seed={seed}, mean={mean}, std={std_dev}, min={min}, max={max}): rust={rust_v} oracle={oracle_v}"
            );

            // NextGaussianInt does NOT advance the MegaCrit counter; assert
            // that bug-for-bug behavior matches.
            let counter_resp = oracle
                .call("rng_counter", json!({ "handle": handle })).unwrap();
            let oracle_counter = counter_resp["result"].as_i64().unwrap() as i32;
            assert_eq!(rust.counter(), oracle_counter,
                "counter drift after next_gaussian_int (seed={seed})");
            assert_eq!(rust.counter(), 0,
                "next_gaussian_int unexpectedly advanced counter to {} (seed={seed})",
                rust.counter());
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
