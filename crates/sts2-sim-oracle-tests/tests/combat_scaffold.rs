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
        json!({
            "handle": handle,
            "character_id": "CHARACTER.IRONCLAD",
            "seed": 42,
        }),
    );
    let r = r.expect("combat_add_player");
    assert_eq!(r["result"], true, "add_player failed: {r}");
    // populate_warning is an acceptable soft-fail for now — Godot-dep
    // shuffle hooks NRE without scene tree. Future fix: more Harmony
    // patches.

    let dump = unwrap_result(
        oracle
            .call("combat_dump", json!({ "handle": handle }))
            .expect("combat_dump"),
    );
    assert_eq!(dump["allies"].as_array().unwrap().len(), 1);
    let creature = &dump["allies"][0];
    assert_eq!(creature["is_player"], true);
    assert_eq!(creature["current_hp"], 80, "Ironclad starting HP: {creature}");
    assert_eq!(creature["max_hp"], 80);
    assert_eq!(creature["block"], 0);
    assert_eq!(creature["powers"].as_array().unwrap().len(), 0);

    let player = &creature["player"];
    assert_eq!(player["max_energy_base"], 3, "Ironclad max energy: {player}");
    assert_eq!(player["energy"], 0, "pre-turn-start energy is 0: {player}");
    // 10-card starter deck: 5 Strikes, 4 Defends, 1 Bash.
    let master = player["master_deck"].as_array().unwrap();
    assert_eq!(master.len(), 10, "master deck size: {player}");
    let strikes = master
        .iter()
        .filter(|c| c["id"] == "CARD.STRIKE_IRONCLAD")
        .count();
    let defends = master
        .iter()
        .filter(|c| c["id"] == "CARD.DEFEND_IRONCLAD")
        .count();
    let bashes = master.iter().filter(|c| c["id"] == "CARD.BASH").count();
    assert_eq!(strikes, 5, "5 Strikes: {master:?}");
    assert_eq!(defends, 4, "4 Defends: {master:?}");
    assert_eq!(bashes, 1, "1 Bash: {master:?}");
    // Draw pile populated (PCS exists) with all 10 deck cards.
    assert_eq!(player["draw"].as_array().unwrap().len(), 10);
    assert_eq!(player["hand"].as_array().unwrap().len(), 0);
    assert_eq!(player["discard"].as_array().unwrap().len(), 0);
    assert_eq!(player["exhaust"].as_array().unwrap().len(), 0);
    // Ironclad starts with BurningBlood.
    let relics = player["relics"].as_array().unwrap();
    assert!(
        relics.iter().any(|r| r == "RELIC.BURNING_BLOOD"),
        "BurningBlood in starting relics: {relics:?}"
    );
    // 3 empty potion slots.
    assert_eq!(player["potions"].as_array().unwrap().len(), 3);
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
