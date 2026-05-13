//! Diff tests for RunRngSet and PlayerRngSet against the live game DLL.
//!
//! Strategy: for each stream of each RngSet, build a Rust Rng via the same
//! named-Rng path the set uses internally, build an oracle Rng via
//! `rng_new_named(seed, name)`, and confirm a batch of draws agrees. If
//! the hardcoded snake_case stream names in `rng_set.rs` are wrong, or
//! the named-Rng constructor diverges, this fails fast.

use serde_json::json;
use sts2_sim::rng::Rng;
use sts2_sim::rng_set::{PlayerRngSet, RunRngSet};
use sts2_sim_oracle_tests::Oracle;

fn check_streams(
    oracle: &mut Oracle,
    seed_uint: u32,
    streams: Vec<(&str, &mut Rng)>,
) {
    for (name, rust_rng) in streams {
        let handle = oracle
            .call(
                "rng_new_named",
                json!({ "seed": seed_uint as i64, "name": name }),
            )
            .unwrap()["result"]
            .as_i64()
            .unwrap();
        // Draw a batch and compare. 50 draws is enough to catch any
        // mismatch in seed mixing or state advance.
        for _ in 0..50 {
            let rust_v = rust_rng.next_int(1_000_000);
            let oracle_v = oracle
                .call(
                    "rng_next_int",
                    json!({ "handle": handle, "max_exclusive": 1_000_000 }),
                )
                .unwrap()["result"]
                .as_i64()
                .unwrap() as i32;
            assert_eq!(
                rust_v, oracle_v,
                "stream {name} diverged at seed_uint={seed_uint}: rust={rust_v} oracle={oracle_v}"
            );
        }
        oracle
            .call("rng_dispose", json!({ "handle": handle }))
            .unwrap();
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn run_rng_set_streams_match() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");

    for seed_string in &[
        "",
        "DEFAULT",
        "abcdef",
        "alpha-1",
        "Hello, world!",
        "🎴_run",
        "OK_so_this_is_a_longer_seed_for_more_coverage",
    ] {
        // Confirm the string -> uint conversion matches first (uses
        // hash_string under the hood, already separately tested).
        let rust_set = RunRngSet::new(seed_string);
        let oracle_uint = oracle
            .call("hash_string", json!({ "str": seed_string }))
            .unwrap()["result"]
            .as_i64()
            .unwrap() as i32 as u32;
        assert_eq!(
            rust_set.seed_uint(),
            oracle_uint,
            "RunRngSet seed_uint mismatch for {seed_string:?}: rust={} oracle={oracle_uint}",
            rust_set.seed_uint()
        );

        let mut rust_set = rust_set;
        let pairs: Vec<(&str, &mut Rng)> = vec![
            ("up_front", &mut rust_set.up_front),
            ("shuffle", &mut rust_set.shuffle),
            ("unknown_map_point", &mut rust_set.unknown_map_point),
            ("combat_card_generation", &mut rust_set.combat_card_generation),
            ("combat_potion_generation", &mut rust_set.combat_potion_generation),
            ("combat_card_selection", &mut rust_set.combat_card_selection),
            ("combat_energy_costs", &mut rust_set.combat_energy_costs),
            ("combat_targets", &mut rust_set.combat_targets),
            ("monster_ai", &mut rust_set.monster_ai),
            ("niche", &mut rust_set.niche),
            ("combat_orbs", &mut rust_set.combat_orbs),
            ("treasure_room_relics", &mut rust_set.treasure_room_relics),
        ];
        check_streams(&mut oracle, oracle_uint, pairs);
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn player_rng_set_streams_match() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    for seed_uint in &[0u32, 1, 42, 0xDEAD_BEEF, 0xFFFF_FFFF] {
        let mut set = PlayerRngSet::new(*seed_uint);
        let pairs: Vec<(&str, &mut Rng)> = vec![
            ("rewards", &mut set.rewards),
            ("shops", &mut set.shops),
            ("transformations", &mut set.transformations),
        ];
        check_streams(&mut oracle, *seed_uint, pairs);
    }
}
