//! Diff tests for the deterministic hash port against the live game DLL's
//! `StringHelper.GetDeterministicHashCode`. Also validates the snake_case
//! conversion the game uses to derive stream names from enum variants —
//! we hardcode those snake_case names in `sts2-sim`'s `rng_set` module,
//! and this test confirms each hardcoded name matches what
//! `SnakeCase(enumVariant.ToString())` produces.

use serde_json::json;
use sts2_sim::hash::deterministic_hash_code;
use sts2_sim_oracle_tests::Oracle;

#[test]
#[ignore = "requires built oracle-host"]
fn hash_matches_oracle_on_ascii() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    // A mix of: stream names, content ids, free-form strings, edge cases.
    let cases: &[&str] = &[
        "", "a", "A", "0", "Hello", "hello", "Hello, world!",
        "up_front", "shuffle", "unknown_map_point",
        "combat_card_generation", "combat_potion_generation",
        "combat_card_selection", "combat_energy_costs",
        "combat_targets", "monster_ai", "niche", "combat_orbs",
        "treasure_room_relics",
        "rewards", "shops", "transformations",
        "FurCoat", "Byrdpip", "PaelsLegion",
        "spoils_map", "map_for_act_1", "map_for_act_2",
        "the quick brown fox jumps over the lazy dog",
        // odd-length strings, single chars, repeated chars
        "abcde", "z", "zz", "zzz", "zzzz",
    ];
    for s in cases {
        let rust = deterministic_hash_code(s);
        let resp = oracle.call("hash_string", json!({ "str": s })).unwrap();
        let oracle_v = resp["result"].as_i64().unwrap() as i32;
        assert_eq!(rust, oracle_v,
            "hash mismatch on {s:?}: rust={rust} oracle={oracle_v}");
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn hash_matches_oracle_on_non_ascii() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let cases: &[&str] = &[
        "café",          // 1-byte UTF-8 then 2-byte
        "naïve",
        "日本語",        // 3-byte UTF-8 (CJK)
        "한국어",
        "русский",
        // supplementary plane (4-byte UTF-8 → 2 UTF-16 code units / surrogate pair)
        "𝓗ello",
        "🎴",            // single supplementary-plane char
        "x🎴y",          // ASCII surrounding a surrogate pair
    ];
    for s in cases {
        let rust = deterministic_hash_code(s);
        let resp = oracle.call("hash_string", json!({ "str": s })).unwrap();
        let oracle_v = resp["result"].as_i64().unwrap() as i32;
        assert_eq!(rust, oracle_v,
            "hash mismatch on {s:?}: rust={rust} oracle={oracle_v}");
    }
}

#[test]
#[ignore = "requires built oracle-host"]
fn snake_case_matches_hardcoded_stream_names() {
    // Verifies that each (CamelCaseEnumVariant -> snake_case) translation
    // we hardcoded in sts2-sim/src/rng_set.rs matches what the live game
    // produces. If MegaCrit ever changes a variant name, this fails first.
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let cases: &[(&str, &str)] = &[
        ("UpFront", "up_front"),
        ("Shuffle", "shuffle"),
        ("UnknownMapPoint", "unknown_map_point"),
        ("CombatCardGeneration", "combat_card_generation"),
        ("CombatPotionGeneration", "combat_potion_generation"),
        ("CombatCardSelection", "combat_card_selection"),
        ("CombatEnergyCosts", "combat_energy_costs"),
        ("CombatTargets", "combat_targets"),
        ("MonsterAi", "monster_ai"),
        ("Niche", "niche"),
        ("CombatOrbs", "combat_orbs"),
        ("TreasureRoomRelics", "treasure_room_relics"),
        ("Rewards", "rewards"),
        ("Shops", "shops"),
        ("Transformations", "transformations"),
    ];
    for (camel, expected_snake) in cases {
        let resp = oracle.call("snake_case", json!({ "str": camel })).unwrap();
        let actual = resp["result"].as_str().unwrap();
        assert_eq!(actual, *expected_snake,
            "SnakeCase({camel:?}) -> {actual:?}, hardcoded {expected_snake:?}");
    }
}
