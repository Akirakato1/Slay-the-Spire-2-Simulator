//! Diff tests for `stable_shuffle` against ListExtensions.StableShuffle
//! reflected from sts2.dll. `unstable_shuffle` is identical to `Rng::shuffle`
//! (already covered by the existing `shuffle_matches` test in `rng.rs`).

use serde_json::{json, Value};
use sts2_sim::rng::Rng;
use sts2_sim::shuffle::stable_shuffle;
use sts2_sim_oracle_tests::Oracle;

/// LCG used only to vary inputs. Independent of the Rng under test.
struct Driver(u64);
impl Driver {
    fn new(seed: u64) -> Self { Self(seed) }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
    fn next_u32(&mut self) -> u32 { (self.next() >> 32) as u32 }
    fn next_i32(&mut self) -> i32 { self.next_u32() as i32 }
    fn next_range(&mut self, max: u32) -> u32 { (self.next() % max as u64) as u32 }
}

fn new_handle(oracle: &mut Oracle, seed: u32, counter: i32) -> i64 {
    oracle
        .call("rng_new", json!({ "seed": seed, "counter": counter }))
        .expect("rng_new")["result"]
        .as_i64()
        .expect("non-integer handle")
}

fn dispose(oracle: &mut Oracle, handle: i64) {
    let _ = oracle.call("rng_dispose", json!({ "handle": handle }));
}

#[test]
#[ignore = "requires built oracle-host"]
fn stable_shuffle_matches() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut d = Driver::new(0x9E37_79B9_AA22_4B0D);

    for _ in 0..50 {
        let seed = d.next_u32();
        let mut rust = Rng::new(seed, 0);
        let handle = new_handle(&mut oracle, seed, 0);

        for _ in 0..20 {
            let n = (d.next_range(60) + 1) as usize;
            // Mix of duplicates, negatives, and random ordering — exercises
            // the sort step in stable_shuffle.
            let mut list: Vec<i32> = (0..n)
                .map(|_| d.next_i32() % 100)
                .collect();
            let mut rust_list = list.clone();
            let oracle_input: Value = json!(list.clone());

            stable_shuffle(&mut rust_list, &mut rust);
            let resp = oracle.call(
                "stable_shuffle",
                json!({ "handle": handle, "list": oracle_input }),
            ).unwrap();
            // Response format mirrors rng_shuffle: a top-level array, not a
            // {result: ...} object — match how shuffle dispatch was wired.
            let oracle_list: Vec<i32> = resp
                .as_array()
                .or_else(|| resp["result"].as_array())
                .expect("oracle stable_shuffle returned non-array")
                .iter()
                .map(|v| v.as_i64().unwrap() as i32)
                .collect();

            assert_eq!(rust_list, oracle_list,
                "stable_shuffle diverged: seed={seed}, n={n}, input={list:?}");
            // Sanity: the shuffled output must be a permutation of the sorted
            // input (which is what stable_shuffle internally produces before
            // shuffling).
            list.sort();
            let mut sorted_out = rust_list.clone();
            sorted_out.sort();
            assert_eq!(sorted_out, list, "stable_shuffle output is not a permutation of input");
        }
        dispose(&mut oracle, handle);
    }
}
