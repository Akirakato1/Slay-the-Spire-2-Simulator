//! MadScience parity: 9 (TinkerTimeType × TinkerTimeRider) variants.
//!
//! MadScience is an event-acquired card whose behavior is locked in
//! at the event's selection step. In real play, the event creates
//! the card with one of 3 types × 3 riders. Our oracle harness can't
//! run the event, so we set the per-instance state directly via
//! `combat_set_card_property` (oracle) / `set_card_state` (rust).
//!
//! Test matrix:
//!   Attack × Sapping  — damage(12) + Weak(2)/Vuln(2)
//!   Attack × Violence — damage(12) × 3 hits
//!   Attack × Choking  — damage(12) + StranglePower(6)
//!   Skill × Energized — block(8) + GainEnergy(2)
//!   Skill × Wisdom    — block(8) + DrawCards(3)
//!   Skill × Chaos     — block(8) + AddRandomCard (RNG-driven)
//!   Power × Expertise — Strength(2) + Dexterity(2)
//!   Power × Curious   — CuriousPower(1)
//!   Power × Improvement — ImprovementPower(1)

use serde_json::{json, Value};
use sts2_sim_oracle_tests::{rust_rig, Oracle};

const SEED: i64 = 42;

#[derive(Copy, Clone, Debug)]
struct Variant {
    name: &'static str,
    tinker_type: i32,
    tinker_rider: i32,
    // True if this variant uses RNG to inject a random card (Chaos),
    // making per-position pile diffs non-deterministic.
    rng_driven_pile: bool,
}

const VARIANTS: &[Variant] = &[
    // tinker_type: 1=Attack, 2=Skill, 3=Power.
    // tinker_rider: 1=Sapping, 2=Violence, 3=Choking, 4=Energized,
    //               5=Wisdom, 6=Chaos, 7=Expertise, 8=Curious, 9=Improvement.
    Variant { name: "Attack/Sapping",    tinker_type: 1, tinker_rider: 1, rng_driven_pile: false },
    Variant { name: "Attack/Violence",   tinker_type: 1, tinker_rider: 2, rng_driven_pile: false },
    Variant { name: "Attack/Choking",    tinker_type: 1, tinker_rider: 3, rng_driven_pile: false },
    Variant { name: "Skill/Energized",   tinker_type: 2, tinker_rider: 4, rng_driven_pile: false },
    Variant { name: "Skill/Wisdom",      tinker_type: 2, tinker_rider: 5, rng_driven_pile: false },
    Variant { name: "Skill/Chaos",       tinker_type: 2, tinker_rider: 6, rng_driven_pile: true  },
    Variant { name: "Power/Expertise",   tinker_type: 3, tinker_rider: 7, rng_driven_pile: false },
    Variant { name: "Power/Curious",     tinker_type: 3, tinker_rider: 8, rng_driven_pile: false },
    Variant { name: "Power/Improvement", tinker_type: 3, tinker_rider: 9, rng_driven_pile: false },
];

fn oracle_setup(oracle: &mut Oracle) -> anyhow::Result<i64> {
    let r = oracle.call("combat_new", json!({}))?;
    let h = r["result"].as_i64()
        .ok_or_else(|| anyhow::anyhow!("combat_new no result: {r}"))?;
    oracle.call("combat_add_player", json!({
        "handle": h, "character_id": "CHARACTER.IRONCLAD", "seed": SEED,
    }))?;
    oracle.call("combat_add_enemy", json!({ "handle": h, "monster_id": "MONSTER.BIG_DUMMY" }))?;
    oracle.call("combat_add_enemy", json!({ "handle": h, "monster_id": "MONSTER.BIG_DUMMY" }))?;
    oracle.call("combat_init_run_state", json!({ "handle": h, "seed": SEED.to_string() }))?;
    Ok(h)
}

fn fresh_rust() -> rust_rig::RustRig {
    let mut r = rust_rig::RustRig::new();
    r.add_player("CHARACTER.IRONCLAD", SEED as u32);
    r.add_enemy("MONSTER.BIG_DUMMY");
    r.add_enemy("MONSTER.BIG_DUMMY");
    r
}

/// Sum enemy current_hp + named power amounts across both enemies.
/// For the Chaos variant we also collapse hand to count-only.
fn signature(dump: &Value, ignore_hand: bool) -> Value {
    let total_hp: i64 = dump
        .pointer("/enemies")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get("current_hp").and_then(|h| h.as_i64()))
                .sum()
        })
        .unwrap_or(0);
    let powers_on_enemies: std::collections::BTreeMap<String, i64> = {
        let mut m: std::collections::BTreeMap<String, i64> = Default::default();
        if let Some(arr) = dump.pointer("/enemies").and_then(|v| v.as_array()) {
            for e in arr {
                if let Some(ps) = e.get("powers").and_then(|v| v.as_array()) {
                    for p in ps {
                        if let (Some(id), Some(amt)) = (
                            p.get("id").and_then(|x| x.as_str()),
                            p.get("amount").and_then(|x| x.as_i64()),
                        ) {
                            *m.entry(id.to_string()).or_default() += amt;
                        }
                    }
                }
            }
        }
        m
    };
    let powers_on_player: std::collections::BTreeMap<String, i64> = {
        let mut m: std::collections::BTreeMap<String, i64> = Default::default();
        if let Some(arr) = dump.pointer("/allies/0/powers").and_then(|v| v.as_array()) {
            for p in arr {
                if let (Some(id), Some(amt)) = (
                    p.get("id").and_then(|x| x.as_str()),
                    p.get("amount").and_then(|x| x.as_i64()),
                ) {
                    *m.entry(id.to_string()).or_default() += amt;
                }
            }
        }
        m
    };
    let player_block = dump.pointer("/allies/0/block").and_then(|v| v.as_i64()).unwrap_or(0);
    let player_energy = dump.pointer("/allies/0/player/energy").and_then(|v| v.as_i64()).unwrap_or(0);
    let hand_size = dump
        .pointer("/allies/0/player/hand")
        .and_then(|v| v.as_array())
        .map(|a| a.len() as i64)
        .unwrap_or(0);
    let mut sig = json!({
        "total_enemy_hp": total_hp,
        "player_block": player_block,
        "player_energy": player_energy,
        "powers_on_enemies": powers_on_enemies,
        "powers_on_player": powers_on_player,
        "hand_size": hand_size,
    });
    if !ignore_hand {
        // Capture hand ids when non-RNG so we catch any wrong-pile move.
        let hand_ids: Vec<String> = dump
            .pointer("/allies/0/player/hand")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.get("id").and_then(|x| x.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        sig["hand_ids"] = json!(hand_ids);
    }
    sig
}

#[test]
#[ignore = "requires oracle host + STS2 install; 9 MadScience variants"]
fn sweep_all_mad_science_variants() {
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut failures: Vec<(String, Value, Value)> = Vec::new();
    let mut passes: Vec<&'static str> = Vec::new();

    for v in VARIANTS {
        eprintln!("--- {} (type={}, rider={}) ---", v.name, v.tinker_type, v.tinker_rider);

        // Oracle: fresh combat, force MadScience to hand, set both
        // TinkerTime properties, play with target=enemy 0.
        let h = match oracle_setup(&mut oracle) {
            Ok(h) => h,
            Err(e) => {
                failures.push((v.name.to_string(),
                    Value::String(format!("oracle setup: {e}")), Value::Null));
                continue;
            }
        };
        let _ = oracle.call("combat_force_card_to_hand", json!({
            "handle": h, "card_id": "CARD.MAD_SCIENCE",
        }));
        let _ = oracle.call("combat_set_card_property", json!({
            "handle": h, "hand_idx": 0,
            "prop_name": "TinkerTimeType", "value": v.tinker_type,
        }));
        let _ = oracle.call("combat_set_card_property", json!({
            "handle": h, "hand_idx": 0,
            "prop_name": "TinkerTimeRider", "value": v.tinker_rider,
        }));
        // Always pass target_idx=0. MadScience's static CardData
        // target_type is AnyEnemy (set at construction), so rust's
        // validate_target requires a target. C# Skill/Power variants
        // route through self regardless of cardPlay.Target, so the
        // extra target arg is benign on the oracle side.
        let play_params = json!({ "handle": h, "hand_idx": 0, "target_idx": 0 });
        let play_resp = oracle.call("combat_play_card", play_params)
            .unwrap_or_else(|e| json!({"error": e.to_string()}));
        if let Some(err) = play_resp.get("onplay_error") {
            failures.push((v.name.to_string(),
                Value::String(format!("oracle onplay: {err}")),
                Value::Null));
            continue;
        }
        if let Some(err) = play_resp.get("error") {
            failures.push((v.name.to_string(),
                Value::String(format!("oracle: {err}")), Value::Null));
            continue;
        }
        let oracle_dump = oracle.call("combat_dump", json!({ "handle": h }))
            .map(|v| v["result"].clone()).unwrap_or(Value::Null);

        // Rust mirror.
        let mut rust = fresh_rust();
        rust.force_card_to_hand("CARD.MAD_SCIENCE", 0);
        rust.set_card_state(0, "tinker_time_type", v.tinker_type);
        rust.set_card_state(0, "tinker_time_rider", v.tinker_rider);
        if !rust.play_card(0, Some(0)) {
            failures.push((v.name.to_string(),
                Value::Null, Value::String("rust play failed".into())));
            continue;
        }
        let rust_dump = rust.dump();

        let o_sig = signature(&oracle_dump, v.rng_driven_pile);
        let r_sig = signature(&rust_dump, v.rng_driven_pile);
        if o_sig != r_sig {
            failures.push((v.name.to_string(), o_sig, r_sig));
        } else {
            passes.push(v.name);
        }
    }

    eprintln!("\n========= MADSCIENCE VARIANT SUMMARY =========");
    eprintln!("PASS ({}/{}):", passes.len(), VARIANTS.len());
    for n in &passes {
        eprintln!("  {}", n);
    }
    if !failures.is_empty() {
        eprintln!("FAIL ({}):", failures.len());
        for (name, o, r) in &failures {
            eprintln!("  {}", name);
            eprintln!("    oracle: {}", o);
            eprintln!("    rust:   {}", r);
        }
        panic!("{} MadScience variants diverge", failures.len());
    }
}
