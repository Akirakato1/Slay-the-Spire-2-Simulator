//! Combat performance benchmarks (Phase 0.5).
//!
//! Targets from `[[project-plan-sts2-rl]]`:
//!   - 1000+ combat-only runs/sec/core
//!   - 100+ full runs/sec/core (flagged optimistic; 30–100 realistic)
//!
//! Run with `cargo bench --bench combat_perf`. Criterion reports
//! `<group>/<name>` time + throughput in `target/criterion/`.
//!
//! What each bench measures:
//!   - `combat_setup`: building a fresh CombatState (encounter spawn +
//!     deck wiring). Establishes the per-fight constant overhead.
//!   - `clone_combat_state`: deep clone. Matters for MCTS-style rollouts
//!     where each branch starts from a snapshot.
//!   - `modify_damage_clean`: pure pipeline cost with no powers — the
//!     fast path for vanilla card play.
//!   - `modify_damage_full`: same with Strength + Vulnerable + Weak +
//!     Intangible cap active — worst-case modifier composition for the
//!     current power set.
//!   - `draw_five_cards_with_reshuffle`: hand-draw cost when the
//!     `Rng.Shuffle` path fires.
//!   - `ironclad_kills_axebot_full_fight`: end-to-end fight cost. The
//!     headline metric for the 1000-runs/sec target. We run the
//!     deterministic "16 Strikes kills both Axebots" scenario.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use sts2_sim::card::{by_id as card_by_id, CardData};
use sts2_sim::character;
use sts2_sim::combat::{
    deck_from_ids, CardInstance, CombatResult, CombatSide, CombatState, PlayerSetup,
    PlayResult, ValueProp,
};
use sts2_sim::encounter;
use sts2_sim::rng::Rng;

fn build_ironclad_combat() -> CombatState {
    let ironclad = character::by_id("Ironclad").expect("Ironclad present");
    let enc = encounter::by_id("AxebotsNormal").expect("AxebotsNormal present");
    let deck = deck_from_ids(&ironclad.starting_deck);
    let setup = PlayerSetup {
        character: ironclad,
        current_hp: ironclad.starting_hp.unwrap(),
        max_hp: ironclad.starting_hp.unwrap(),
        deck,
        relics: ironclad.starting_relics.clone(),
    };
    CombatState::start(enc, vec![setup], Vec::new())
}

fn bench_combat_setup(c: &mut Criterion) {
    let mut group = c.benchmark_group("combat_setup");
    group.throughput(Throughput::Elements(1));
    group.bench_function("ironclad_axebots", |b| {
        b.iter(|| {
            let cs = build_ironclad_combat();
            black_box(cs);
        });
    });
    group.finish();
}

fn bench_clone_combat_state(c: &mut Criterion) {
    let mut group = c.benchmark_group("clone_combat_state");
    group.throughput(Throughput::Elements(1));
    let cs = build_ironclad_combat();
    group.bench_function("ironclad_axebots_fresh", |b| {
        b.iter(|| {
            let clone = cs.clone();
            black_box(clone);
        });
    });
    group.finish();
}

fn bench_modify_damage(c: &mut Criterion) {
    let mut group = c.benchmark_group("modify_damage");
    group.throughput(Throughput::Elements(1));

    // Clean: no powers active. Best case.
    let cs_clean = build_ironclad_combat();
    group.bench_function("clean", |b| {
        b.iter(|| {
            let d = cs_clean.modify_damage(
                black_box((CombatSide::Player, 0)),
                black_box((CombatSide::Enemy, 0)),
                black_box(6),
                ValueProp::MOVE,
            );
            black_box(d);
        });
    });

    // Full: Strength + Vulnerable + Weak + Intangible (Intangible caps,
    // so result == 1; still exercises every pass).
    let mut cs_full = build_ironclad_combat();
    cs_full.apply_power(CombatSide::Player, 0, "StrengthPower", 3);
    cs_full.apply_power(CombatSide::Player, 0, "WeakPower", 1);
    cs_full.apply_power(CombatSide::Enemy, 0, "VulnerablePower", 1);
    cs_full.apply_power(CombatSide::Enemy, 0, "IntangiblePower", 1);
    group.bench_function("strength_vuln_weak_intangible", |b| {
        b.iter(|| {
            let d = cs_full.modify_damage(
                black_box((CombatSide::Player, 0)),
                black_box((CombatSide::Enemy, 0)),
                black_box(6),
                ValueProp::MOVE,
            );
            black_box(d);
        });
    });
    group.finish();
}

fn bench_draw_with_reshuffle(c: &mut Criterion) {
    let mut group = c.benchmark_group("draw_cards");
    group.throughput(Throughput::Elements(5));
    group.bench_function("draw_5_with_reshuffle", |b| {
        b.iter_with_setup(
            || {
                let mut cs = build_ironclad_combat();
                // Drain draw into discard so the next draw triggers
                // a full reshuffle.
                {
                    let ps = cs.allies[0].player.as_mut().unwrap();
                    let all = std::mem::take(&mut ps.draw.cards);
                    ps.discard.cards = all;
                }
                (cs, Rng::new(42, 0))
            },
            |(mut cs, mut rng)| {
                let n = cs.draw_cards(0, 5, &mut rng);
                black_box(n);
            },
        );
    });
    group.finish();
}

/// The "headline" benchmark: how long does it take to resolve a full
/// Ironclad-vs-AxebotsNormal combat from CombatState::start to Victory?
/// Determinism: we inject 16 StrikeIroncladS directly into hand and play
/// them — no draw RNG involvement. Total damage = 16 × 6 = 96, enough
/// to drop both Axebots (44 + 44 = 88 HP).
fn bench_full_fight(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_fight");
    group.throughput(Throughput::Elements(1));
    let strike: &'static CardData = card_by_id("StrikeIronclad").unwrap();
    group.bench_function("ironclad_strikes_axebots", |b| {
        b.iter(|| {
            let mut cs = build_ironclad_combat();
            // Skip draw shuffle for a deterministic timing profile.
            {
                let ps = cs.allies[0].player.as_mut().unwrap();
                ps.hand.cards.clear();
                for _ in 0..16 {
                    ps.hand.cards.push(CardInstance::from_card(strike, 0));
                }
                ps.energy = 99;
            }
            // Play 8 at enemy 0 then 8 at enemy 1.
            for _ in 0..8 {
                let r = cs.play_card(0, 0, Some((CombatSide::Enemy, 0)));
                debug_assert_eq!(r, PlayResult::Ok);
            }
            for _ in 0..8 {
                let r = cs.play_card(0, 0, Some((CombatSide::Enemy, 1)));
                debug_assert_eq!(r, PlayResult::Ok);
            }
            debug_assert_eq!(cs.is_combat_over(), Some(CombatResult::Victory));
            black_box(cs);
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_combat_setup,
    bench_clone_combat_state,
    bench_modify_damage,
    bench_draw_with_reshuffle,
    bench_full_fight
);
criterion_main!(benches);
