//! Phase 3: parity tests. Drive both rigs through identical inputs
//! and diff the JSON dumps field-by-field. The Rust dump is emitted
//! by `rust_rig::combat_dump`, which matches the C# host's schema.
//!
//! These tests don't fail on the FIRST divergence — they collect all
//! diffs and report them so we can fix divergences in one pass.

use serde_json::{json, Value};
use sts2_sim_oracle_tests::{rust_rig, Oracle};

fn unwrap_result(v: Value) -> Value {
    if let Some(err) = v.get("error") {
        panic!("oracle returned error: {err}");
    }
    v["result"].clone()
}

/// Canonical audit scenario: Ironclad vs 2× BigDummy. Two enemies
/// distinguishes ChosenEnemy (single-target) from AllEnemies (sweep)
/// from RandomEnemy (RNG pick) cards/effects.
fn setup_oracle(oracle: &mut Oracle, seed: i64, enemy_id: &str) -> i64 {
    let h = unwrap_result(
        oracle.call("combat_new", json!({})).expect("combat_new"),
    )
    .as_i64()
    .unwrap();
    oracle
        .call(
            "combat_add_player",
            json!({
                "handle": h,
                "character_id": "CHARACTER.IRONCLAD",
                "seed": seed,
            }),
        )
        .expect("combat_add_player");
    for _ in 0..2 {
        oracle
            .call(
                "combat_add_enemy",
                json!({ "handle": h, "monster_id": enemy_id }),
            )
            .expect("combat_add_enemy");
    }
    h
}

/// Set up the Rust rig with 2 enemies matching `setup_oracle`.
fn setup_rust(enemy_modelid: &str) -> rust_rig::RustRig {
    let mut r = rust_rig::RustRig::new();
    r.add_player("CHARACTER.IRONCLAD", 42);
    r.add_enemy(enemy_modelid);
    r.add_enemy(enemy_modelid);
    r
}

/// Collect divergences between two JSON values at the same path.
/// Returns a list of (path, oracle_value, rust_value) tuples.
fn collect_diffs(
    path: &str,
    oracle: &Value,
    rust: &Value,
    out: &mut Vec<(String, Value, Value)>,
) {
    match (oracle, rust) {
        (Value::Object(o), Value::Object(r)) => {
            let mut keys: std::collections::BTreeSet<&String> = o.keys().collect();
            keys.extend(r.keys());
            for k in keys {
                let sub = format!("{}.{}", path, k);
                let ov = o.get(k).unwrap_or(&Value::Null);
                let rv = r.get(k).unwrap_or(&Value::Null);
                collect_diffs(&sub, ov, rv, out);
            }
        }
        (Value::Array(o), Value::Array(r)) => {
            for i in 0..o.len().max(r.len()) {
                let sub = format!("{}[{}]", path, i);
                let ov = o.get(i).unwrap_or(&Value::Null);
                let rv = r.get(i).unwrap_or(&Value::Null);
                collect_diffs(&sub, ov, rv, out);
            }
        }
        (a, b) if a == b => {}
        (a, b) => {
            out.push((path.to_string(), a.clone(), b.clone()));
        }
    }
}

fn report_diffs(label: &str, diffs: &[(String, Value, Value)]) {
    if diffs.is_empty() {
        return;
    }
    eprintln!("\n=== {} divergences ({}) ===", label, diffs.len());
    for (path, ov, rv) in diffs {
        eprintln!("  {path}");
        eprintln!("    oracle: {ov}");
        eprintln!("    rust:   {rv}");
    }
}

#[test]
#[ignore = "requires `dotnet build oracle-host -c Release` + STS2 game install"]
fn parity_strike_unupgraded() {
    let mut oracle = Oracle::spawn().expect("spawn oracle host");
    let h = setup_oracle(&mut oracle, 42, "MONSTER.BIG_DUMMY");
    let mut rust_cs = setup_rust("MONSTER.BIG_DUMMY");

    // Force Strike to hand on both sides.
    oracle
        .call(
            "combat_force_card_to_hand",
            json!({ "handle": h, "card_id": "CARD.STRIKE_IRONCLAD" }),
        )
        .expect("force_to_hand");
    rust_cs.force_card_to_hand("CARD.STRIKE_IRONCLAD", 0);

    // Play card 0 (Strike) targeting enemy 0 (BigDummy).
    oracle
        .call(
            "combat_play_card",
            json!({ "handle": h, "hand_idx": 0, "target_idx": 0 }),
        )
        .expect("play_card");
    rust_cs.play_card(0, Some(0));

    let oracle_dump = unwrap_result(
        oracle.call("combat_dump", json!({ "handle": h })).expect("dump"),
    );
    let rust_dump = rust_cs.dump();

    let mut diffs = Vec::new();
    collect_diffs("$", &oracle_dump, &rust_dump, &mut diffs);
    report_diffs("strike_unupgraded", &diffs);

    // Core invariants — these MUST match.
    assert_eq!(
        oracle_dump["enemies"][0]["current_hp"],
        rust_dump["enemies"][0]["current_hp"],
        "Strike damage parity: oracle={} rust={}",
        oracle_dump["enemies"][0]["current_hp"],
        rust_dump["enemies"][0]["current_hp"],
    );
    assert_eq!(
        oracle_dump["allies"][0]["block"],
        rust_dump["allies"][0]["block"],
        "ally block parity",
    );
}

#[test]
#[ignore = "requires `dotnet build oracle-host -c Release` + STS2 game install"]
fn parity_defend() {
    let mut oracle = Oracle::spawn().expect("spawn oracle host");
    let h = setup_oracle(&mut oracle, 42, "MONSTER.BIG_DUMMY");
    let mut rust_cs = setup_rust("MONSTER.BIG_DUMMY");

    oracle
        .call(
            "combat_force_card_to_hand",
            json!({ "handle": h, "card_id": "CARD.DEFEND_IRONCLAD" }),
        )
        .expect("force_to_hand");
    rust_cs.force_card_to_hand("CARD.DEFEND_IRONCLAD", 0);

    oracle
        .call(
            "combat_play_card",
            json!({ "handle": h, "hand_idx": 0 }),
        )
        .expect("play_card");
    rust_cs.play_card(0, None);

    let oracle_dump = unwrap_result(
        oracle.call("combat_dump", json!({ "handle": h })).expect("dump"),
    );
    let rust_dump = rust_cs.dump();

    let mut diffs = Vec::new();
    collect_diffs("$", &oracle_dump, &rust_dump, &mut diffs);
    report_diffs("defend", &diffs);

    assert_eq!(
        oracle_dump["allies"][0]["block"],
        rust_dump["allies"][0]["block"],
        "Defend block parity",
    );
}

#[test]
#[ignore = "requires `dotnet build oracle-host -c Release` + STS2 game install"]
fn parity_strike_upgraded() {
    let mut oracle = Oracle::spawn().expect("spawn oracle host");
    let h = setup_oracle(&mut oracle, 42, "MONSTER.BIG_DUMMY");
    let mut rust_cs = setup_rust("MONSTER.BIG_DUMMY");

    oracle
        .call(
            "combat_force_card_to_hand",
            json!({
                "handle": h,
                "card_id": "CARD.STRIKE_IRONCLAD",
                "upgrade_level": 1,
            }),
        )
        .expect("force_to_hand");
    rust_cs.force_card_to_hand("CARD.STRIKE_IRONCLAD", 1);

    oracle
        .call(
            "combat_play_card",
            json!({ "handle": h, "hand_idx": 0, "target_idx": 0 }),
        )
        .expect("play_card");
    rust_cs.play_card(0, Some(0));

    let oracle_dump = unwrap_result(
        oracle.call("combat_dump", json!({ "handle": h })).expect("dump"),
    );
    let rust_dump = rust_cs.dump();

    let mut diffs = Vec::new();
    collect_diffs("$", &oracle_dump, &rust_dump, &mut diffs);
    report_diffs("strike_upgraded", &diffs);

    assert_eq!(
        oracle_dump["enemies"][0]["current_hp"],
        rust_dump["enemies"][0]["current_hp"],
        "Strike+1 damage parity",
    );
}

/// Multi-target attack: Thunderclap deals 4 dmg to ALL enemies and
/// applies 1 Vulnerable to each. With 2 BigDummies, both should take
/// identical damage + status on both sides.
#[test]
#[ignore = "requires `dotnet build oracle-host -c Release` + STS2 game install"]
fn parity_thunderclap_hits_both_enemies() {
    let mut oracle = Oracle::spawn().expect("spawn oracle host");
    let h = setup_oracle(&mut oracle, 42, "MONSTER.BIG_DUMMY");
    let mut rust_cs = setup_rust("MONSTER.BIG_DUMMY");

    oracle
        .call(
            "combat_force_card_to_hand",
            json!({ "handle": h, "card_id": "CARD.THUNDERCLAP" }),
        )
        .expect("force_to_hand");
    rust_cs.force_card_to_hand("CARD.THUNDERCLAP", 0);

    // Thunderclap is AllEnemies — target_idx ignored.
    oracle
        .call(
            "combat_play_card",
            json!({ "handle": h, "hand_idx": 0 }),
        )
        .expect("play_card");
    rust_cs.play_card(0, None);

    let oracle_dump = unwrap_result(
        oracle.call("combat_dump", json!({ "handle": h })).expect("dump"),
    );
    let rust_dump = rust_cs.dump();

    let mut diffs = Vec::new();
    collect_diffs("$", &oracle_dump, &rust_dump, &mut diffs);
    report_diffs("thunderclap", &diffs);

    for i in 0..2 {
        assert_eq!(
            oracle_dump["enemies"][i]["current_hp"],
            rust_dump["enemies"][i]["current_hp"],
            "Thunderclap dmg parity on enemy[{i}]",
        );
        // Each enemy should have a Vulnerable stack from Thunderclap.
        let oracle_powers = &oracle_dump["enemies"][i]["powers"];
        let rust_powers = &rust_dump["enemies"][i]["powers"];
        assert_eq!(
            oracle_powers, rust_powers,
            "Thunderclap Vulnerable application parity on enemy[{i}]",
        );
    }
}
