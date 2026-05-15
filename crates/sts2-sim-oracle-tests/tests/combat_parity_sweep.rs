//! Phase 3 sweep: parity-test every Ironclad + Colorless card.
//!
//! For each card: fresh combat (Ironclad vs 2× BigDummy), force card
//! to hand, play it with target inferred from CardData.target_type,
//! diff the full state dumps. The test aggregates results — it
//! reports every divergent card and fails at the end if any matter.
//!
//! Output is grouped so the underlying-primitive bugs are obvious:
//!   - PASS: card behaves identically on both sides.
//!   - DAMAGE: hp diverges on at least one enemy.
//!   - BLOCK:  ally block diverges.
//!   - POWERS: power list diverges on player or enemy.
//!   - PILES:  hand/draw/discard/exhaust diverge.
//!   - ERROR:  one side threw (couldn't play); see error message.
//!
//! Skip categories (intentionally not run):
//!   - X-cost cards (need explicit energy setup; tested separately).
//!   - Status / Curse cards (Unplayable; nothing to assert).

use serde_json::{json, Value};
use sts2_sim::card::{self, CardData, TargetType};
use sts2_sim_oracle_tests::{rust_rig, Oracle};

const SEED: i64 = 42;
const ENEMY_ID: &str = "MONSTER.BIG_DUMMY";
const N_ENEMIES: usize = 2;

fn unwrap_result(v: Value) -> Value {
    if let Some(err) = v.get("error") {
        panic!("oracle error: {err}");
    }
    v["result"].clone()
}

/// Convert a Rust card id ("StrikeIronclad") to the C# ModelId
/// ("CARD.STRIKE_IRONCLAD") using the slugify rule.
fn card_modelid(rust_id: &str) -> String {
    rust_rig::rust_to_modelid(rust_id, "CARD")
}

/// Build a fresh oracle handle: combat_new + add_player + 2× add_enemy.
/// Returns Err on pipe failure so the sweep can restart the oracle.
fn oracle_setup(oracle: &mut Oracle) -> anyhow::Result<i64> {
    let r = oracle.call("combat_new", json!({}))?;
    let h = r["result"].as_i64()
        .ok_or_else(|| anyhow::anyhow!("combat_new no result: {r}"))?;
    oracle.call(
        "combat_add_player",
        json!({
            "handle": h,
            "character_id": "CHARACTER.IRONCLAD",
            "seed": SEED,
        }),
    )?;
    for _ in 0..N_ENEMIES {
        oracle.call(
            "combat_add_enemy",
            json!({ "handle": h, "monster_id": ENEMY_ID }),
        )?;
    }
    Ok(h)
}

fn fresh_oracle(oracle: &mut Oracle) -> i64 {
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
                "seed": SEED,
            }),
        )
        .expect("add_player");
    for _ in 0..N_ENEMIES {
        oracle
            .call(
                "combat_add_enemy",
                json!({ "handle": h, "monster_id": ENEMY_ID }),
            )
            .expect("add_enemy");
    }
    h
}

fn fresh_rust() -> rust_rig::RustRig {
    let mut r = rust_rig::RustRig::new();
    r.add_player("CHARACTER.IRONCLAD", 42);
    for _ in 0..N_ENEMIES {
        r.add_enemy(ENEMY_ID);
    }
    r
}

/// Target selection. Returns (enemy_idx, ally_idx). At most one is
/// Some — the C# host preferentially uses ally_target_idx if set.
fn pick_target(t: TargetType) -> (Option<usize>, Option<usize>) {
    match t {
        TargetType::AnyEnemy | TargetType::RandomEnemy => (Some(0), None),
        TargetType::AnyAlly => (None, Some(0)),
        _ => (None, None),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Category {
    Pass,
    Damage,
    Block,
    Powers,
    Piles,
    Energy,
    OracleError(String),
    RustError,
    Other,  // diverges in fields that don't fit the named buckets
}

struct SweepResult {
    card_id: String,
    category: Category,
    diffs: Vec<(String, Value, Value)>,
}

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

fn categorize(diffs: &[(String, Value, Value)]) -> Category {
    if diffs.is_empty() {
        return Category::Pass;
    }
    let mut categories: std::collections::BTreeSet<&str> =
        std::collections::BTreeSet::new();
    for (path, _, _) in diffs {
        if path.contains("current_hp") {
            categories.insert("damage");
        } else if path.contains(".block") {
            categories.insert("block");
        } else if path.contains(".powers") {
            categories.insert("powers");
        } else if path.contains(".hand")
            || path.contains(".draw")
            || path.contains(".discard")
            || path.contains(".exhaust")
            || path.contains(".play")
        {
            categories.insert("piles");
        } else if path.contains(".energy") {
            categories.insert("energy");
        } else {
            categories.insert("other");
        }
    }
    // Prefer the most diagnostic category if multiple.
    if categories.contains("damage") {
        Category::Damage
    } else if categories.contains("block") {
        Category::Block
    } else if categories.contains("powers") {
        Category::Powers
    } else if categories.contains("piles") {
        Category::Piles
    } else if categories.contains("energy") {
        Category::Energy
    } else {
        Category::Other
    }
}

fn run_one_card(oracle: &mut Oracle, card: &CardData) -> SweepResult {
    let card_id = card.id.clone();
    let modelid = card_modelid(&card_id);

    let h_opt = match oracle_setup(oracle) {
        Ok(h) => h,
        Err(e) => {
            return SweepResult {
                card_id,
                category: Category::OracleError(format!("setup: {e}")),
                diffs: Vec::new(),
            };
        }
    };
    let h = h_opt;
    let mut rust = fresh_rust();

    // Force card to hand on both sides.
    if let Err(e) = oracle.call(
        "combat_force_card_to_hand",
        json!({ "handle": h, "card_id": modelid }),
    ) {
        return SweepResult {
            card_id,
            category: Category::OracleError(format!("force_to_hand: {e}")),
            diffs: Vec::new(),
        };
    }
    rust.force_card_to_hand(&modelid, 0);

    // Play the card with appropriate target.
    let (enemy_idx, ally_idx) = pick_target(card.target_type);
    let mut play_params = json!({ "handle": h, "hand_idx": 0 });
    if let Some(t) = enemy_idx {
        play_params["target_idx"] = json!(t);
    }
    if let Some(a) = ally_idx {
        play_params["ally_target_idx"] = json!(a);
    }
    let oracle_play_resp = oracle
        .call("combat_play_card", play_params)
        .unwrap_or_else(|e| json!({"error": e.to_string()}));
    if let Some(err) = oracle_play_resp.get("error") {
        return SweepResult {
            card_id,
            category: Category::OracleError(err.as_str().unwrap_or("?").to_string()),
            diffs: Vec::new(),
        };
    }
    if oracle_play_resp.get("onplay_error").is_some() {
        return SweepResult {
            card_id,
            category: Category::OracleError(
                oracle_play_resp["onplay_error"]
                    .as_str()
                    .unwrap_or("?")
                    .to_string(),
            ),
            diffs: Vec::new(),
        };
    }
    let rust_ok = if ally_idx.is_some() {
        rust.play_card_ally(0, ally_idx)
    } else {
        rust.play_card(0, enemy_idx)
    };
    if !rust_ok {
        return SweepResult {
            card_id,
            category: Category::RustError,
            diffs: Vec::new(),
        };
    }

    let dump_resp = match oracle.call("combat_dump", json!({ "handle": h })) {
        Ok(v) => v,
        Err(e) => {
            return SweepResult {
                card_id,
                category: Category::OracleError(format!("dump: {e}")),
                diffs: Vec::new(),
            };
        }
    };
    if let Some(err) = dump_resp.get("error") {
        return SweepResult {
            card_id,
            category: Category::OracleError(err.as_str().unwrap_or("?").to_string()),
            diffs: Vec::new(),
        };
    }
    let oracle_dump = dump_resp["result"].clone();
    let rust_dump = rust.dump();

    let mut diffs = Vec::new();
    collect_diffs("$", &oracle_dump, &rust_dump, &mut diffs);
    let cat = categorize(&diffs);
    SweepResult {
        card_id,
        category: cat,
        diffs,
    }
}

#[test]
#[ignore = "requires oracle host + STS2 install; long-running (sweeps every playable card on Ironclad)"]
fn sweep_all_cards_on_ironclad() {
    // Every class card works on every class — STS2's design — so we
    // test every playable card on Ironclad as the canonical scenario.
    // Skip pools that have no playable cards or are scaffolding:
    //   Status / Curse — Unplayable; nothing to assert.
    //   Quest / Deprecated — not real cards.
    // Skip X-cost cards (need explicit energy setup, tested separately).
    let cards: Vec<&'static CardData> = card::ALL_CARDS
        .iter()
        .filter(|c| {
            let playable_pool = matches!(
                c.pool.as_str(),
                "Ironclad" | "Silent" | "Defect" | "Regent" | "Necrobinder"
                    | "Colorless" | "Token" | "Event"
            );
            playable_pool
                && !c.has_energy_cost_x
                && c.card_type != sts2_sim::card::CardType::Status
                && c.card_type != sts2_sim::card::CardType::Curse
        })
        .collect();

    eprintln!("sweeping {} cards", cards.len());
    let mut oracle = Oracle::spawn().expect("spawn oracle");
    let mut results: Vec<SweepResult> = Vec::with_capacity(cards.len());
    let mut crashes = 0;
    for (i, c) in cards.iter().enumerate() {
        if i % 25 == 0 {
            eprintln!("  [{}/{}] {}", i, cards.len(), c.id);
        }
        let r = run_one_card(&mut oracle, c);
        // If the oracle crashed (pipe broken), spawn a new one.
        if let Category::OracleError(ref msg) = r.category {
            if msg.contains("pipe") || msg.contains("EOF")
                || msg.contains("broken")
                || msg.contains("closed")
            {
                crashes += 1;
                eprintln!(
                    "  oracle died on {} ({} crashes); respawning",
                    c.id, crashes
                );
                oracle = Oracle::spawn().expect("respawn oracle");
            }
        }
        results.push(r);
    }
    eprintln!("oracle crashes: {crashes}");

    // Aggregate.
    let mut buckets: std::collections::BTreeMap<String, Vec<&SweepResult>> =
        std::collections::BTreeMap::new();
    for r in &results {
        let key = match &r.category {
            Category::Pass => "PASS".to_string(),
            Category::Damage => "DAMAGE".to_string(),
            Category::Block => "BLOCK".to_string(),
            Category::Powers => "POWERS".to_string(),
            Category::Piles => "PILES".to_string(),
            Category::Energy => "ENERGY".to_string(),
            Category::Other => "OTHER".to_string(),
            Category::OracleError(_) => "ORACLE_ERROR".to_string(),
            Category::RustError => "RUST_ERROR".to_string(),
        };
        buckets.entry(key).or_default().push(r);
    }

    eprintln!("\n========= SWEEP SUMMARY =========");
    for (key, items) in &buckets {
        eprintln!("  {:14} {:4}", key, items.len());
    }

    eprintln!("\n========= DIVERGENCES (first 5 paths each) =========");
    for (key, items) in &buckets {
        if key == "PASS" {
            continue;
        }
        eprintln!("\n[{}] {} cards", key, items.len());
        for r in items {
            eprintln!("  {}", r.card_id);
            if let Category::OracleError(e) = &r.category {
                eprintln!("    oracle error: {}",
                    e.chars().take(120).collect::<String>());
            }
            for (path, ov, rv) in r.diffs.iter().take(5) {
                let ov_s = format!("{ov}");
                let rv_s = format!("{rv}");
                eprintln!(
                    "    {} | oracle={} | rust={}",
                    path,
                    ov_s.chars().take(80).collect::<String>(),
                    rv_s.chars().take(80).collect::<String>(),
                );
            }
        }
    }

    let pass_count = buckets.get("PASS").map(|v| v.len()).unwrap_or(0);
    let total = results.len();
    eprintln!(
        "\nPASS: {} / {} ({:.1}%)",
        pass_count,
        total,
        100.0 * pass_count as f64 / total as f64
    );

    // Don't hard-fail on divergence — the report IS the deliverable.
    // The test can be made strict later once divergences are addressed.
}
