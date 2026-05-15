//! Phase 1 smoke tests for the combat oracle host scaffold:
//!   - construct a mock CombatState
//!   - add a player (Ironclad) + an enemy
//!   - dump state and verify basic fields are populated
//!
//! These tests don't assert against the Rust sim yet — they only verify
//! the C# host can stand up a combat shell. Behavioral parity comes in
//! Phase 2 when we wire card execution on both sides.

use serde_json::json;
use sts2_sim_oracle_tests::Oracle;

fn unwrap_result(v: serde_json::Value) -> serde_json::Value {
    if let Some(err) = v.get("error") {
        panic!("oracle returned error: {err}");
    }
    v["result"].clone()
}

#[test]
#[ignore = "requires `dotnet build oracle-host -c Release` + STS2 game install"]
fn combat_new_creates_empty_state() {
    let mut oracle = Oracle::spawn().expect("spawn oracle host");
    let r = oracle.call("combat_new", json!({})).expect("combat_new");
    let handle = unwrap_result(r).as_i64().expect("handle int");
    assert!(handle > 0, "expected positive handle, got {handle}");

    let dump = unwrap_result(
        oracle
            .call("combat_dump", json!({ "handle": handle }))
            .expect("combat_dump"),
    );
    assert_eq!(dump["round_number"], 1);
    // CombatSide.Player == 0 in C# (typical enum order: None/Player/Enemy).
    // We assert the dump has the field; the exact int value can be
    // refined once we verify the enum order.
    assert!(dump.get("current_side").is_some(), "missing current_side: {dump}");
    assert_eq!(dump["allies"].as_array().unwrap().len(), 0);
    assert_eq!(dump["enemies"].as_array().unwrap().len(), 0);
}

#[test]
#[ignore = "requires `dotnet build oracle-host -c Release` + STS2 game install"]
fn combat_add_player_ironclad() {
    let mut oracle = Oracle::spawn().expect("spawn oracle host");
    let handle = unwrap_result(
        oracle.call("combat_new", json!({})).expect("combat_new"),
    )
    .as_i64()
    .unwrap();

    // Character ids are namespaced: "CHARACTER.IRONCLAD".
    let r = oracle.call(
        "combat_add_player",
        json!({ "handle": handle, "character_id": "CHARACTER.IRONCLAD" }),
    );
    let r = unwrap_result(r.expect("combat_add_player"));
    assert_eq!(r, true);

    let dump = unwrap_result(
        oracle
            .call("combat_dump", json!({ "handle": handle }))
            .expect("combat_dump"),
    );
    assert_eq!(
        dump["allies"].as_array().unwrap().len(),
        1,
        "expected 1 ally after add_player: {dump}"
    );
    let player = &dump["allies"][0];
    assert_eq!(player["is_player"], true);
    assert!(player["max_hp"].as_i64().unwrap() > 0, "max_hp not positive: {player}");
}

#[test]
#[ignore = "requires `dotnet build oracle-host -c Release` + STS2 game install"]
fn combat_add_enemy() {
    let mut oracle = Oracle::spawn().expect("spawn oracle host");
    let handle = unwrap_result(
        oracle.call("combat_new", json!({})).expect("combat_new"),
    )
    .as_i64()
    .unwrap();

    // Add a Player first since CreateCreature for enemy reads
    // RunState.Rng — having a player ensures the combat has a
    // baseline. (NullRunState may or may not satisfy this; we'll
    // adjust if it errors.)
    let _ = unwrap_result(
        oracle
            .call(
                "combat_add_player",
                json!({ "handle": handle, "character_id": "CHARACTER.IRONCLAD" }),
            )
            .expect("combat_add_player"),
    );

    // BigDummy is a stable HP test target in STS2.
    let r = oracle.call(
        "combat_add_enemy",
        json!({ "handle": handle, "monster_id": "MONSTER.BIG_DUMMY" }),
    );
    let r = unwrap_result(r.expect("combat_add_enemy"));
    assert_eq!(r, true);

    let dump = unwrap_result(
        oracle
            .call("combat_dump", json!({ "handle": handle }))
            .expect("combat_dump"),
    );
    assert_eq!(
        dump["enemies"].as_array().unwrap().len(),
        1,
        "expected 1 enemy after add_enemy: {dump}"
    );
    let enemy = &dump["enemies"][0];
    assert_eq!(enemy["is_player"], false);
    assert!(enemy["max_hp"].as_i64().unwrap() > 0);
}
