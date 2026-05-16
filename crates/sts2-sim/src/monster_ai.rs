//! Data-driven monster AI: intent / action / move-pattern primitives.
//!
//! Most monsters' behavior in STS2 is one of three patterns:
//!  1. **Cycle** — deterministic rotation (Myte: Toxic → Bite → Suck → …).
//!  2. **WeightedRandom** — pick with weights, often with a no-repeat rule
//!     (Axebot: OneTwo:2 / Sharpen:1 / HammerUppercut:2, Sharpen blocked
//!     if just played).
//!  3. **HpThresholdSwitch** — boss transitions to a new pattern when its
//!     HP falls below a threshold (TheArchitect phase 2, etc.).
//!
//! These three compose via `Box<MovePattern>` into the four wrappers:
//!  - `FirstTurnOverride { first_move, then_pattern }`
//!  - `BySlot { branches, default }`
//!  - `HpThresholdSwitch { threshold_pct, below, above }`
//!  - `Conditional { predicate, then_branch, else_branch }`
//!
//! Each `MonsterMove` carries an `IntentKind` (the visible hint —
//! Attack/Defend/Buff/Debuff) and an `Effect`-list body. The body
//! resolves through the same Effect VM as cards: `Target::SelfActor`
//! → the monster, `Target::ChosenEnemy` → the targeted player.
//!
//! Adding a new monster is now ~20 lines of data instead of 2 hand-rolled
//! `pick_*_intent` / `execute_*_move` functions. The dispatcher tries
//! the data-driven `MONSTER_AI_REGISTRY` first; falls through to the
//! legacy hand-rolled match arms for monsters not yet migrated.

use crate::combat::{CombatSide, CombatState};
use crate::effects::{Effect, EffectContext, Target};
use crate::rng::Rng;
use std::collections::HashMap;
use std::sync::LazyLock;

/// One move a monster can execute. Mirrors the C# per-move classes
/// (`AxebotBootUpMove`, `MyteBiteMove`, …) with the intent hint and
/// the effect-list body factored out.
#[derive(Clone, Debug)]
pub struct MonsterMove {
    /// Stable string id used for replay logs / `monster.intent_move`.
    /// Must match the C# `MoveModel.Id.Entry` exactly so .run-file
    /// replay can route through this dispatcher.
    pub id: &'static str,
    /// What the intent UI shows. Drives the feature-vector view of
    /// "what's the enemy doing this turn".
    pub kind: IntentKind,
    /// Effects fired when the monster takes its turn. Same vocabulary
    /// as card OnPlay bodies.
    pub body: Vec<Effect>,
}

/// Intent hint shown to the player. Mirrors the C# `IntentModel` family.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IntentKind {
    /// Pure damage. `hits` is how many separate hits the body deals
    /// (used by the UI to render "12 x 2" etc).
    Attack { hits: i32 },
    /// Gain block, no damage.
    Defend,
    /// Self buff (power apply on self / strength gain).
    Buff,
    /// Apply debuff to player(s).
    Debuff,
    /// Damage plus a self/buff component.
    AttackBuff { hits: i32 },
    /// Damage plus block (defensive attack).
    AttackDefend { hits: i32 },
    /// Damage plus a debuff applied to the target.
    AttackDebuff { hits: i32 },
    /// Hidden intent (`???` in C#). Used by bosses / mystery moves.
    Unknown,
    /// Stunned / does-nothing turn.
    Sleep,
    /// Summon an enemy.
    Summon,
}

/// State-machine rule for picking the next intent.
///
/// `Cycle` and `WeightedRandom` are the two terminal cases. The
/// wrappers (`FirstTurnOverride`, `BySlot`, `HpThresholdSwitch`,
/// `Conditional`) compose around them.
#[derive(Clone, Debug)]
pub enum MovePattern {
    /// Pure rotation. `moves[(i + 1) mod n]` each turn. `i` is the
    /// index of the last move; the first turn picks `moves[0]` unless
    /// wrapped by `FirstTurnOverride` or `BySlot`.
    Cycle { moves: Vec<&'static str> },
    /// Weighted random with optional no-repeat constraint.
    WeightedRandom {
        /// (move_id, weight). Weights are summed; `rng.next_float(total)`
        /// picks. Iteration order matches the listed order (matches the
        /// C# `RandomBranchState.States` list order).
        weights: Vec<(&'static str, i32)>,
        /// Move ids that can't repeat if they were the last move
        /// played. Their weight is zeroed for that one roll.
        no_repeat: Vec<&'static str>,
    },
    /// First turn fires `first_move`; every subsequent turn delegates
    /// to `then`.
    FirstTurnOverride {
        first_move: &'static str,
        then: Box<MovePattern>,
    },
    /// Branch on slot label ("first", "second", "front", "back", etc).
    /// First match wins; `default` is the fallback.
    BySlot {
        branches: Vec<(&'static str, MovePattern)>,
        default: Box<MovePattern>,
    },
    /// One-shot HP-threshold switch. The first turn that
    /// `current_hp * 100 / max_hp < threshold_pct`, fire `below` and
    /// stick with it. Otherwise use `above`.
    HpThresholdSwitch {
        threshold_pct: i32,
        below: Box<MovePattern>,
        above: Box<MovePattern>,
    },
    /// Generic conditional: evaluate `predicate`; pick from `then` or
    /// `else_branch`.
    Conditional {
        predicate: AiCondition,
        then_branch: Box<MovePattern>,
        else_branch: Box<MovePattern>,
    },
}

/// Predicates over the combat state for `MovePattern::Conditional`.
/// Closed set — extend only when a new C# branch surfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiCondition {
    /// First turn (no prior move).
    FirstTurn,
    /// Last move played has this id.
    LastMoveWas(&'static str),
    /// Living-enemy count matches.
    LivingEnemyCountEquals(i32),
    /// Living-enemy count < N (used by "alone" checks).
    LivingEnemyCountLessThan(i32),
    /// Monster carries the named flag on its MonsterState.
    HasFlag(&'static str),
    /// `current_hp * 100 / max_hp < pct`.
    HpBelowPct(i32),
    /// Negation.
    Not(Box<AiCondition>),
    /// Logical AND.
    And(Box<AiCondition>, Box<AiCondition>),
}

/// Full per-monster AI table. Owns the move list + the state-machine
/// pattern. Stored in `MONSTER_AI_REGISTRY` keyed by model_id.
#[derive(Clone, Debug)]
pub struct MonsterAi {
    pub model_id: &'static str,
    pub moves: Vec<MonsterMove>,
    pub pattern: MovePattern,
}

impl MonsterAi {
    /// Look up a move by id. None if the id isn't in `moves`.
    pub fn get_move(&self, id: &str) -> Option<&MonsterMove> {
        self.moves.iter().find(|m| m.id == id)
    }
}

// ---------------------------------------------------------------- Evaluation

/// Evaluate a `MovePattern` to pick the next intent.
///
/// `last`: id of the move played last turn (None on first turn).
/// `slot`: monster's slot label (e.g. "first", "second").
/// `rng`: shared combat RNG (the dispatcher's `take/put_rng` pattern).
/// Returns the chosen move id, or None if no move could be picked.
pub fn pick_next_move(
    pattern: &MovePattern,
    cs: &CombatState,
    enemy_idx: usize,
    last: Option<&str>,
    slot: &str,
    rng: &mut Rng,
) -> Option<&'static str> {
    match pattern {
        MovePattern::Cycle { moves } => {
            if moves.is_empty() {
                return None;
            }
            let next_idx = match last {
                None => 0,
                Some(prev) => {
                    let prev_pos = moves.iter().position(|m| *m == prev).unwrap_or(usize::MAX);
                    if prev_pos == usize::MAX {
                        0
                    } else {
                        (prev_pos + 1) % moves.len()
                    }
                }
            };
            Some(moves[next_idx])
        }
        MovePattern::WeightedRandom { weights, no_repeat } => {
            let last_is_blocked = |id: &str| -> bool {
                last.map(|p| p == id).unwrap_or(false) && no_repeat.iter().any(|nr| *nr == id)
            };
            let total: f32 = weights
                .iter()
                .map(|(id, w)| if last_is_blocked(id) { 0.0 } else { *w as f32 })
                .sum();
            if total <= 0.0 {
                return weights.first().map(|(id, _)| *id);
            }
            let mut roll = rng.next_float(total);
            for (id, w) in weights {
                let weight = if last_is_blocked(id) { 0.0 } else { *w as f32 };
                roll -= weight;
                if roll <= 0.0 {
                    return Some(*id);
                }
            }
            // Numerical fallback — pick the last one.
            weights.last().map(|(id, _)| *id)
        }
        MovePattern::FirstTurnOverride { first_move, then } => {
            if last.is_none() {
                Some(*first_move)
            } else {
                pick_next_move(then, cs, enemy_idx, last, slot, rng)
            }
        }
        MovePattern::BySlot { branches, default } => {
            for (slot_label, branch) in branches {
                if *slot_label == slot {
                    return pick_next_move(branch, cs, enemy_idx, last, slot, rng);
                }
            }
            pick_next_move(default, cs, enemy_idx, last, slot, rng)
        }
        MovePattern::HpThresholdSwitch {
            threshold_pct,
            below,
            above,
        } => {
            let hp = cs.enemies.get(enemy_idx).map(|c| c.current_hp).unwrap_or(0);
            let max_hp = cs.enemies.get(enemy_idx).map(|c| c.max_hp).unwrap_or(1).max(1);
            let pct = hp * 100 / max_hp;
            if pct < *threshold_pct {
                pick_next_move(below, cs, enemy_idx, last, slot, rng)
            } else {
                pick_next_move(above, cs, enemy_idx, last, slot, rng)
            }
        }
        MovePattern::Conditional {
            predicate,
            then_branch,
            else_branch,
        } => {
            if evaluate_condition(predicate, cs, enemy_idx, last) {
                pick_next_move(then_branch, cs, enemy_idx, last, slot, rng)
            } else {
                pick_next_move(else_branch, cs, enemy_idx, last, slot, rng)
            }
        }
    }
}

fn evaluate_condition(
    cond: &AiCondition,
    cs: &CombatState,
    enemy_idx: usize,
    last: Option<&str>,
) -> bool {
    match cond {
        AiCondition::FirstTurn => last.is_none(),
        AiCondition::LastMoveWas(id) => last.map(|p| p == *id).unwrap_or(false),
        AiCondition::LivingEnemyCountEquals(n) => {
            let count = cs.enemies.iter().filter(|e| e.current_hp > 0).count() as i32;
            count == *n
        }
        AiCondition::LivingEnemyCountLessThan(n) => {
            let count = cs.enemies.iter().filter(|e| e.current_hp > 0).count() as i32;
            count < *n
        }
        AiCondition::HasFlag(name) => cs
            .enemies
            .get(enemy_idx)
            .and_then(|c| c.monster.as_ref())
            .map(|m| m.flag(name))
            .unwrap_or(false),
        AiCondition::HpBelowPct(pct) => {
            let hp = cs.enemies.get(enemy_idx).map(|c| c.current_hp).unwrap_or(0);
            let max_hp = cs.enemies.get(enemy_idx).map(|c| c.max_hp).unwrap_or(1).max(1);
            (hp * 100 / max_hp) < *pct
        }
        AiCondition::Not(inner) => !evaluate_condition(inner, cs, enemy_idx, last),
        AiCondition::And(a, b) => {
            evaluate_condition(a, cs, enemy_idx, last) && evaluate_condition(b, cs, enemy_idx, last)
        }
    }
}

/// Execute the selected move by running its body through the standard
/// Effect VM with the monster as actor and the player as target.
///
/// Caller is responsible for resolving the actual `target_player_idx`
/// (single-player runs always use 0; co-op may differ).
pub fn execute_move(
    cs: &mut CombatState,
    ai: &MonsterAi,
    move_id: &str,
    enemy_idx: usize,
    target_player_idx: usize,
) -> bool {
    let Some(m) = ai.get_move(move_id) else {
        return false;
    };
    let body = m.body.clone();
    let mut ctx = EffectContext::for_monster_move(
        enemy_idx,
        Some((CombatSide::Player, target_player_idx)),
    );
    ctx.player_idx = target_player_idx;
    crate::effects::execute_effects(cs, &body, &ctx);
    true
}

// ---------------------------------------------------------------- Registry

/// Initial registry of data-driven monster AIs. Starts small —
/// monsters get migrated from the hand-rolled dispatcher one at a
/// time as their patterns get encoded here.
pub static MONSTER_AI_REGISTRY: LazyLock<HashMap<&'static str, MonsterAi>> = LazyLock::new(|| {
    let mut m: HashMap<&'static str, MonsterAi> = HashMap::new();
    register_ruby_raiders(&mut m);
    register_simple_test_monsters(&mut m);
    register_basic_gremlins(&mut m);
    m
});

/// Look up a monster's data-driven AI. None means "use the legacy
/// hand-rolled dispatcher".
pub fn ai_for(model_id: &str) -> Option<&'static MonsterAi> {
    MONSTER_AI_REGISTRY.get(model_id)
}

// ---- Per-monster registrations ----------------------------------

/// The four "RubyRaider" basic enemies follow a simple 2-move cycle.
/// Damage / block numbers mirror the C# `*RubyRaider.cs` files.
fn register_ruby_raiders(m: &mut HashMap<&'static str, MonsterAi>) {
    // AxeRubyRaider: Slash(7) → Sharpen(self +2 Strength) → ...
    m.insert(
        "AxeRubyRaider",
        MonsterAi {
            model_id: "AxeRubyRaider",
            moves: vec![
                MonsterMove {
                    id: "SLASH_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(7),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "SHARPEN_MOVE",
                    kind: IntentKind::Buff,
                    body: vec![Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: crate::effects::AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    }],
                },
            ],
            pattern: MovePattern::Cycle {
                moves: vec!["SLASH_MOVE", "SHARPEN_MOVE"],
            },
        },
    );
    // CrossbowRubyRaider: Shoot(5) → Reload(self block 6) → ...
    m.insert(
        "CrossbowRubyRaider",
        MonsterAi {
            model_id: "CrossbowRubyRaider",
            moves: vec![
                MonsterMove {
                    id: "SHOOT_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(5),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "RELOAD_MOVE",
                    kind: IntentKind::Defend,
                    body: vec![Effect::GainBlock {
                        amount: crate::effects::AmountSpec::Fixed(6),
                        target: Target::SelfActor,
                    }],
                },
            ],
            pattern: MovePattern::Cycle {
                moves: vec!["SHOOT_MOVE", "RELOAD_MOVE"],
            },
        },
    );
    // BruteRubyRaider: 50/50 Slam(9) vs Stomp(6+Weak). No repeat.
    m.insert(
        "BruteRubyRaider",
        MonsterAi {
            model_id: "BruteRubyRaider",
            moves: vec![
                MonsterMove {
                    id: "SLAM_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(9),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "STOMP_MOVE",
                    kind: IntentKind::AttackDebuff { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: crate::effects::AmountSpec::Fixed(6),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        Effect::ApplyPower {
                            power_id: "WeakPower".to_string(),
                            amount: crate::effects::AmountSpec::Fixed(1),
                            target: Target::ChosenEnemy,
                        },
                    ],
                },
            ],
            pattern: MovePattern::WeightedRandom {
                weights: vec![("SLAM_MOVE", 1), ("STOMP_MOVE", 1)],
                no_repeat: vec!["SLAM_MOVE", "STOMP_MOVE"],
            },
        },
    );
    // AssassinRubyRaider: Backstab(4×2) → Hide(self block 5+Dodge) → ...
    m.insert(
        "AssassinRubyRaider",
        MonsterAi {
            model_id: "AssassinRubyRaider",
            moves: vec![
                MonsterMove {
                    id: "BACKSTAB_MOVE",
                    kind: IntentKind::Attack { hits: 2 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(4),
                        target: Target::ChosenEnemy,
                        hits: 2,
                    }],
                },
                MonsterMove {
                    id: "HIDE_MOVE",
                    kind: IntentKind::Defend,
                    body: vec![Effect::GainBlock {
                        amount: crate::effects::AmountSpec::Fixed(5),
                        target: Target::SelfActor,
                    }],
                },
            ],
            pattern: MovePattern::Cycle {
                moves: vec!["BACKSTAB_MOVE", "HIDE_MOVE"],
            },
        },
    );
    // TrackerRubyRaider: Track(self +1 Strength) → Fire(8) → repeat.
    m.insert(
        "TrackerRubyRaider",
        MonsterAi {
            model_id: "TrackerRubyRaider",
            moves: vec![
                MonsterMove {
                    id: "TRACK_MOVE",
                    kind: IntentKind::Buff,
                    body: vec![Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: crate::effects::AmountSpec::Fixed(1),
                        target: Target::SelfActor,
                    }],
                },
                MonsterMove {
                    id: "FIRE_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(8),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
            ],
            pattern: MovePattern::Cycle {
                moves: vec!["TRACK_MOVE", "FIRE_MOVE"],
            },
        },
    );
}

/// The test-stub monsters live in C# `Models/Monsters/Test*` as
/// scaffolding for the framework's own unit tests. Encoding them
/// here matches the same scaffold function but goes through the
/// data-driven path — a useful smoke test of the dispatcher.
fn register_simple_test_monsters(m: &mut HashMap<&'static str, MonsterAi>) {
    // SingleAttackMoveMonster: one move, damage 10 every turn.
    m.insert(
        "SingleAttackMoveMonster",
        MonsterAi {
            model_id: "SingleAttackMoveMonster",
            moves: vec![MonsterMove {
                id: "ATTACK_MOVE",
                kind: IntentKind::Attack { hits: 1 },
                body: vec![Effect::DealDamage {
                    amount: crate::effects::AmountSpec::Fixed(10),
                    target: Target::ChosenEnemy,
                    hits: 1,
                }],
            }],
            pattern: MovePattern::Cycle {
                moves: vec!["ATTACK_MOVE"],
            },
        },
    );
    // MultiAttackMoveMonster: alternates two damage moves.
    m.insert(
        "MultiAttackMoveMonster",
        MonsterAi {
            model_id: "MultiAttackMoveMonster",
            moves: vec![
                MonsterMove {
                    id: "ATTACK_MOVE_A",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(5),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "ATTACK_MOVE_B",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(8),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
            ],
            pattern: MovePattern::Cycle {
                moves: vec!["ATTACK_MOVE_A", "ATTACK_MOVE_B"],
            },
        },
    );
}

/// Three of the gremlin family use clean cycles or weighted rolls.
fn register_basic_gremlins(m: &mut HashMap<&'static str, MonsterAi>) {
    // FatGremlin: Smash(5) → Pummel(2×2) → Smash → ...
    m.insert(
        "FatGremlin",
        MonsterAi {
            model_id: "FatGremlin",
            moves: vec![
                MonsterMove {
                    id: "SMASH_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(5),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "PUMMEL_MOVE",
                    kind: IntentKind::Attack { hits: 2 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(2),
                        target: Target::ChosenEnemy,
                        hits: 2,
                    }],
                },
            ],
            pattern: MovePattern::Cycle {
                moves: vec!["SMASH_MOVE", "PUMMEL_MOVE"],
            },
        },
    );
    // SneakyGremlin: Strike(6) → Stab(4+Vulnerable) cycle.
    m.insert(
        "SneakyGremlin",
        MonsterAi {
            model_id: "SneakyGremlin",
            moves: vec![
                MonsterMove {
                    id: "STRIKE_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(6),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "STAB_MOVE",
                    kind: IntentKind::AttackDebuff { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: crate::effects::AmountSpec::Fixed(4),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        Effect::ApplyPower {
                            power_id: "VulnerablePower".to_string(),
                            amount: crate::effects::AmountSpec::Fixed(1),
                            target: Target::ChosenEnemy,
                        },
                    ],
                },
            ],
            pattern: MovePattern::Cycle {
                moves: vec!["STRIKE_MOVE", "STAB_MOVE"],
            },
        },
    );
    // GremlinMerc: weighted random between Slice(7) and Encourage
    // (self +2 Strength). No-repeat on Encourage to avoid stacking
    // infinitely.
    m.insert(
        "GremlinMerc",
        MonsterAi {
            model_id: "GremlinMerc",
            moves: vec![
                MonsterMove {
                    id: "SLICE_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(7),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "ENCOURAGE_MOVE",
                    kind: IntentKind::Buff,
                    body: vec![Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: crate::effects::AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    }],
                },
            ],
            pattern: MovePattern::WeightedRandom {
                weights: vec![("SLICE_MOVE", 2), ("ENCOURAGE_MOVE", 1)],
                no_repeat: vec!["ENCOURAGE_MOVE"],
            },
        },
    );
}

// ---------------------------------------------------------------- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_advances_through_moves() {
        let pat = MovePattern::Cycle {
            moves: vec!["A", "B", "C"],
        };
        let cs = CombatState::empty();
        let mut rng = Rng::new(0, 0);
        assert_eq!(pick_next_move(&pat, &cs, 0, None, "first", &mut rng), Some("A"));
        assert_eq!(pick_next_move(&pat, &cs, 0, Some("A"), "first", &mut rng), Some("B"));
        assert_eq!(pick_next_move(&pat, &cs, 0, Some("B"), "first", &mut rng), Some("C"));
        assert_eq!(pick_next_move(&pat, &cs, 0, Some("C"), "first", &mut rng), Some("A"));
    }

    #[test]
    fn weighted_random_blocks_no_repeat() {
        let pat = MovePattern::WeightedRandom {
            weights: vec![("A", 1), ("B", 1)],
            no_repeat: vec!["A"],
        };
        let cs = CombatState::empty();
        let mut rng = Rng::new(0, 0);
        // After "A", "A" is blocked → must pick "B" regardless of roll.
        for _ in 0..50 {
            let pick = pick_next_move(&pat, &cs, 0, Some("A"), "first", &mut rng);
            assert_eq!(pick, Some("B"));
        }
    }

    #[test]
    fn first_turn_override_fires_only_once() {
        let pat = MovePattern::FirstTurnOverride {
            first_move: "OPENER",
            then: Box::new(MovePattern::Cycle {
                moves: vec!["A", "B"],
            }),
        };
        let cs = CombatState::empty();
        let mut rng = Rng::new(0, 0);
        assert_eq!(pick_next_move(&pat, &cs, 0, None, "first", &mut rng), Some("OPENER"));
        assert_eq!(pick_next_move(&pat, &cs, 0, Some("OPENER"), "first", &mut rng), Some("A"));
        assert_eq!(pick_next_move(&pat, &cs, 0, Some("A"), "first", &mut rng), Some("B"));
    }

    #[test]
    fn ai_registry_has_ruby_raiders() {
        assert!(ai_for("AxeRubyRaider").is_some());
        assert!(ai_for("AssassinRubyRaider").is_some());
        assert!(ai_for("BruteRubyRaider").is_some());
        assert!(ai_for("CrossbowRubyRaider").is_some());
        assert!(ai_for("TrackerRubyRaider").is_some());
    }

    #[test]
    fn ai_registry_misses_unknown_monster() {
        assert!(ai_for("NotARealMonster").is_none());
    }

    #[test]
    fn axe_ruby_raider_alternates_slash_and_sharpen() {
        // Pure-cycle integration: Slash → Sharpen → Slash → ...
        let ai = ai_for("AxeRubyRaider").unwrap();
        let cs = CombatState::empty();
        let mut rng = Rng::new(0, 0);
        assert_eq!(
            pick_next_move(&ai.pattern, &cs, 0, None, "front", &mut rng),
            Some("SLASH_MOVE")
        );
        assert_eq!(
            pick_next_move(&ai.pattern, &cs, 0, Some("SLASH_MOVE"), "front", &mut rng),
            Some("SHARPEN_MOVE")
        );
        assert_eq!(
            pick_next_move(&ai.pattern, &cs, 0, Some("SHARPEN_MOVE"), "front", &mut rng),
            Some("SLASH_MOVE")
        );
    }

    #[test]
    fn brute_ruby_raider_alternates_under_no_repeat() {
        // After SLAM the no_repeat rule forces STOMP; after STOMP it
        // forces SLAM. Result: forced alternation regardless of RNG.
        let ai = ai_for("BruteRubyRaider").unwrap();
        let cs = CombatState::empty();
        let mut rng = Rng::new(0, 0);
        for _ in 0..30 {
            assert_eq!(
                pick_next_move(&ai.pattern, &cs, 0, Some("SLAM_MOVE"), "front", &mut rng),
                Some("STOMP_MOVE")
            );
            assert_eq!(
                pick_next_move(&ai.pattern, &cs, 0, Some("STOMP_MOVE"), "front", &mut rng),
                Some("SLAM_MOVE")
            );
        }
    }

    #[test]
    fn intent_kinds_classify_moves() {
        // Smoke-test: every registered move has a sensible intent kind.
        for ai in MONSTER_AI_REGISTRY.values() {
            for m in &ai.moves {
                // Attack-family intents must have non-zero hits.
                match m.kind {
                    IntentKind::Attack { hits }
                    | IntentKind::AttackBuff { hits }
                    | IntentKind::AttackDefend { hits }
                    | IntentKind::AttackDebuff { hits } => {
                        assert!(hits > 0,
                            "{}.{}: attack intent must have hits > 0",
                            ai.model_id, m.id);
                    }
                    _ => {}
                }
            }
        }
    }
}
