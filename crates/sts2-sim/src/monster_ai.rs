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
    /// Last move was NOT this id (covers Fogmog's CannotRepeat
    /// semantics where a move can't immediately repeat itself).
    LastMoveWasNot(&'static str),
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
    /// Logical OR — FrogKnight's `HasCharged || HP >= 50%`.
    Or(Box<AiCondition>, Box<AiCondition>),
}

/// Full per-monster AI table. Owns the move list + the state-machine
/// pattern. Optional `spawn` is the effect body fired by
/// `AfterAddedToRoom` (preset powers like HighVoltage, Plating, etc).
/// Stored in `MONSTER_AI_REGISTRY` keyed by model_id.
#[derive(Clone, Debug)]
pub struct MonsterAi {
    pub model_id: &'static str,
    pub moves: Vec<MonsterMove>,
    pub pattern: MovePattern,
    /// Effects fired once when the monster enters combat. Empty = no
    /// preset powers / pre-combat setup.
    #[allow(dead_code)]
    pub spawn: Vec<Effect>,
}

impl MonsterAi {
    /// Look up a move by id. None if the id isn't in `moves`.
    pub fn get_move(&self, id: &str) -> Option<&MonsterMove> {
        self.moves.iter().find(|m| m.id == id)
    }
}

/// Execute a monster's spawn payload through the Effect VM. Called
/// from `fire_one_monster_spawn` for monsters in the registry.
/// Idempotent if `spawn` is empty.
pub fn execute_spawn(cs: &mut CombatState, ai: &MonsterAi, enemy_idx: usize) {
    if ai.spawn.is_empty() {
        return;
    }
    let body = ai.spawn.clone();
    let ctx = EffectContext::for_monster_move(enemy_idx, None);
    crate::effects::execute_effects(cs, &body, &ctx);
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
        AiCondition::LastMoveWasNot(id) => last.map(|p| p != *id).unwrap_or(true),
        AiCondition::Not(inner) => !evaluate_condition(inner, cs, enemy_idx, last),
        AiCondition::And(a, b) => {
            evaluate_condition(a, cs, enemy_idx, last) && evaluate_condition(b, cs, enemy_idx, last)
        }
        AiCondition::Or(a, b) => {
            evaluate_condition(a, cs, enemy_idx, last) || evaluate_condition(b, cs, enemy_idx, last)
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
    register_single_move_monsters(&mut m);
    register_two_move_cycles(&mut m);
    register_three_move_cycles(&mut m);
    register_weighted_random_monsters(&mut m);
    register_flag_state_monsters(&mut m);
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
            spawn: vec![],
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
            spawn: vec![],
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
            spawn: vec![],
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
            spawn: vec![],
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
            spawn: vec![],
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
            spawn: vec![],
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
            spawn: vec![],
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
            spawn: vec![],
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
            spawn: vec![],
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
            spawn: vec![],
            pattern: MovePattern::WeightedRandom {
                weights: vec![("SLICE_MOVE", 2), ("ENCOURAGE_MOVE", 1)],
                no_repeat: vec!["ENCOURAGE_MOVE"],
            },
        },
    );
}

/// Monsters with a single move that repeats every turn. Damage/block
/// numbers from C# `*.cs` per-move getters. A0 (no-ascension) values
/// used; the higher branch of `AscensionHelper.GetValueIfAscension`
/// switches to the ascended value once that's plumbed.
fn register_single_move_monsters(m: &mut HashMap<&'static str, MonsterAi>) {
    // Byrdpip: NOTHING_MOVE — no-op (Byrdpips do nothing themselves;
    // their threat is via the Byrdonis they spawn from).
    m.insert(
        "Byrdpip",
        MonsterAi {
            model_id: "Byrdpip",
            moves: vec![MonsterMove {
                id: "NOTHING_MOVE",
                kind: IntentKind::Sleep,
                body: vec![],
            }],
            spawn: vec![],
            pattern: MovePattern::Cycle { moves: vec!["NOTHING_MOVE"] },
        },
    );
    // Stabbot: STAB damage 11 + Frail(1) every turn.
    m.insert(
        "Stabbot",
        MonsterAi {
            model_id: "Stabbot",
            moves: vec![MonsterMove {
                id: "STAB_MOVE",
                kind: IntentKind::AttackDebuff { hits: 1 },
                body: vec![
                    Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(11),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    },
                    Effect::ApplyPower {
                        power_id: "FrailPower".to_string(),
                        amount: crate::effects::AmountSpec::Fixed(1),
                        target: Target::ChosenEnemy,
                    },
                ],
            }],
            spawn: vec![],
            pattern: MovePattern::Cycle { moves: vec!["STAB_MOVE"] },
        },
    );
    // Zapbot: ZAP 14 damage every turn. Spawn applies HighVoltage(2).
    m.insert(
        "Zapbot",
        MonsterAi {
            model_id: "Zapbot",
            moves: vec![MonsterMove {
                id: "ZAP_MOVE",
                kind: IntentKind::Attack { hits: 1 },
                body: vec![Effect::DealDamage {
                    amount: crate::effects::AmountSpec::Fixed(14),
                    target: Target::ChosenEnemy,
                    hits: 1,
                }],
            }],
            spawn: vec![Effect::ApplyPower {
                power_id: "HighVoltagePower".to_string(),
                amount: crate::effects::AmountSpec::Fixed(2),
                target: Target::SelfActor,
            }],
            pattern: MovePattern::Cycle { moves: vec!["ZAP_MOVE"] },
        },
    );
    // Noisebot: NOISE adds 2 Dazed cards to the player's discard.
    m.insert(
        "Noisebot",
        MonsterAi {
            model_id: "Noisebot",
            moves: vec![MonsterMove {
                id: "NOISE_MOVE",
                kind: IntentKind::Debuff,
                body: vec![
                    Effect::AddCardToPile {
                        card_id: "Dazed".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                    Effect::AddCardToPile {
                        card_id: "Dazed".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                ],
            }],
            spawn: vec![],
            pattern: MovePattern::Cycle { moves: vec!["NOISE_MOVE"] },
        },
    );
    // EyeWithTeeth: DISTRACT adds 3 Dazed to player discard.
    // Spawn: Illusion(1) — any damage kills the illusion.
    m.insert(
        "EyeWithTeeth",
        MonsterAi {
            model_id: "EyeWithTeeth",
            moves: vec![MonsterMove {
                id: "DISTRACT_MOVE",
                kind: IntentKind::Debuff,
                body: vec![
                    Effect::AddCardToPile {
                        card_id: "Dazed".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                    Effect::AddCardToPile {
                        card_id: "Dazed".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                    Effect::AddCardToPile {
                        card_id: "Dazed".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                ],
            }],
            spawn: vec![Effect::ApplyPower {
                power_id: "IllusionPower".to_string(),
                amount: crate::effects::AmountSpec::Fixed(1),
                target: Target::SelfActor,
            }],
            pattern: MovePattern::Cycle { moves: vec!["DISTRACT_MOVE"] },
        },
    );
    // Parafright: SLAM 16 damage every turn. Spawn: Illusion(1).
    m.insert(
        "Parafright",
        MonsterAi {
            model_id: "Parafright",
            moves: vec![MonsterMove {
                id: "SLAM_MOVE",
                kind: IntentKind::Attack { hits: 1 },
                body: vec![Effect::DealDamage {
                    amount: crate::effects::AmountSpec::Fixed(16),
                    target: Target::ChosenEnemy,
                    hits: 1,
                }],
            }],
            spawn: vec![Effect::ApplyPower {
                power_id: "IllusionPower".to_string(),
                amount: crate::effects::AmountSpec::Fixed(1),
                target: Target::SelfActor,
            }],
            pattern: MovePattern::Cycle { moves: vec!["SLAM_MOVE"] },
        },
    );
    // SnappingJaxfruit: ENERGY_ORB damage 3 + self +2 Strength.
    m.insert(
        "SnappingJaxfruit",
        MonsterAi {
            model_id: "SnappingJaxfruit",
            moves: vec![MonsterMove {
                id: "ENERGY_ORB_MOVE",
                kind: IntentKind::AttackBuff { hits: 1 },
                body: vec![
                    Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(3),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    },
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: crate::effects::AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    },
                ],
            }],
            spawn: vec![],
            pattern: MovePattern::Cycle { moves: vec!["ENERGY_ORB_MOVE"] },
        },
    );
}

/// Two-move cycle monsters. First move often differs from C#
/// expectation, so each gets a FirstTurnOverride wrapper.
fn register_two_move_cycles(m: &mut HashMap<&'static str, MonsterAi>) {
    // DampCultist: INCANTATION (+5 Ritual) → DARK_STRIKE (1 damage).
    // First turn: INCANTATION.
    m.insert(
        "DampCultist",
        MonsterAi {
            model_id: "DampCultist",
            moves: vec![
                MonsterMove {
                    id: "INCANTATION_MOVE",
                    kind: IntentKind::Buff,
                    body: vec![Effect::ApplyPower {
                        power_id: "RitualPower".to_string(),
                        amount: crate::effects::AmountSpec::Fixed(5),
                        target: Target::SelfActor,
                    }],
                },
                MonsterMove {
                    id: "DARK_STRIKE_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(1),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::Cycle {
                moves: vec!["INCANTATION_MOVE", "DARK_STRIKE_MOVE"],
            },
        },
    );
    // SewerClam: PRESSURIZE (self +4 Strength) ↔ JET (10 damage).
    // First turn: JET. Spawn: Plating(8) — block-on-hit preset.
    m.insert(
        "SewerClam",
        MonsterAi {
            model_id: "SewerClam",
            moves: vec![
                MonsterMove {
                    id: "PRESSURIZE_MOVE",
                    kind: IntentKind::Buff,
                    body: vec![Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: crate::effects::AmountSpec::Fixed(4),
                        target: Target::SelfActor,
                    }],
                },
                MonsterMove {
                    id: "JET_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(10),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
            ],
            spawn: vec![Effect::ApplyPower {
                power_id: "PlatingPower".to_string(),
                amount: crate::effects::AmountSpec::Fixed(8),
                target: Target::SelfActor,
            }],
            pattern: MovePattern::FirstTurnOverride {
                first_move: "JET_MOVE",
                then: Box::new(MovePattern::Cycle {
                    moves: vec!["JET_MOVE", "PRESSURIZE_MOVE"],
                }),
            },
        },
    );
    // ToughEgg: HATCH → NIBBLE (4 damage) → HATCH again.
    // Spawn: HatchPower(1) — countdown that resolves on enemy turn end.
    // Summon mechanics for HATCH not yet wired through this path; the
    // body is a no-op so the cycle still ticks the intent correctly.
    m.insert(
        "ToughEgg",
        MonsterAi {
            model_id: "ToughEgg",
            moves: vec![
                MonsterMove {
                    id: "HATCH_MOVE",
                    kind: IntentKind::Summon,
                    body: vec![],
                },
                MonsterMove {
                    id: "NIBBLE_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(4),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
            ],
            spawn: vec![Effect::ApplyPower {
                power_id: "HatchPower".to_string(),
                amount: crate::effects::AmountSpec::Fixed(1),
                target: Target::SelfActor,
            }],
            pattern: MovePattern::Cycle {
                moves: vec!["HATCH_MOVE", "NIBBLE_MOVE"],
            },
        },
    );
}

/// Three-move cycle monsters and similar.
fn register_three_move_cycles(m: &mut HashMap<&'static str, MonsterAi>) {
    // VineShambler: SWIPE (6×2) → GRASPING_VINES (8 + Tangled?) → CHOMP (16).
    // First turn: SWIPE. We approximate Tangled with WeakPower since
    // TangledPower may not be wired.
    m.insert(
        "VineShambler",
        MonsterAi {
            model_id: "VineShambler",
            moves: vec![
                MonsterMove {
                    id: "SWIPE_MOVE",
                    kind: IntentKind::Attack { hits: 2 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(6),
                        target: Target::ChosenEnemy,
                        hits: 2,
                    }],
                },
                MonsterMove {
                    id: "GRASPING_VINES_MOVE",
                    kind: IntentKind::AttackDebuff { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: crate::effects::AmountSpec::Fixed(8),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        // TangledPower (debuff Counter) — applies to
                        // target; per C#, afflicts every Attack card
                        // with Entangled at apply time. Behavior body
                        // not yet wired through the power VM, but
                        // applying the stack at least surfaces the
                        // correct power on the feature vector.
                        Effect::ApplyPower {
                            power_id: "TangledPower".to_string(),
                            amount: crate::effects::AmountSpec::Fixed(1),
                            target: Target::ChosenEnemy,
                        },
                    ],
                },
                MonsterMove {
                    id: "CHOMP_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(16),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::Cycle {
                moves: vec!["SWIPE_MOVE", "GRASPING_VINES_MOVE", "CHOMP_MOVE"],
            },
        },
    );
    // KinFollower: QUICK_SLASH (5) → BOOMERANG (2×2) → POWER_DANCE (+2 Str).
    m.insert(
        "KinFollower",
        MonsterAi {
            model_id: "KinFollower",
            moves: vec![
                MonsterMove {
                    id: "QUICK_SLASH_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(5),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "BOOMERANG_MOVE",
                    kind: IntentKind::Attack { hits: 2 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(2),
                        target: Target::ChosenEnemy,
                        hits: 2,
                    }],
                },
                MonsterMove {
                    id: "POWER_DANCE_MOVE",
                    kind: IntentKind::Buff,
                    body: vec![Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: crate::effects::AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    }],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::Cycle {
                moves: vec!["QUICK_SLASH_MOVE", "BOOMERANG_MOVE", "POWER_DANCE_MOVE"],
            },
        },
    );
    // KinPriest: ORB_FRAILTY (8+Frail) → ORB_WEAKNESS (8+Weak) → BEAM (3×3) → RITUAL (+2 Str).
    m.insert(
        "KinPriest",
        MonsterAi {
            model_id: "KinPriest",
            moves: vec![
                MonsterMove {
                    id: "ORB_OF_FRAILTY_MOVE",
                    kind: IntentKind::AttackDebuff { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: crate::effects::AmountSpec::Fixed(8),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        Effect::ApplyPower {
                            power_id: "FrailPower".to_string(),
                            amount: crate::effects::AmountSpec::Fixed(1),
                            target: Target::ChosenEnemy,
                        },
                    ],
                },
                MonsterMove {
                    id: "ORB_OF_WEAKNESS_MOVE",
                    kind: IntentKind::AttackDebuff { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: crate::effects::AmountSpec::Fixed(8),
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
                MonsterMove {
                    id: "BEAM_MOVE",
                    kind: IntentKind::Attack { hits: 3 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(3),
                        target: Target::ChosenEnemy,
                        hits: 3,
                    }],
                },
                MonsterMove {
                    id: "RITUAL_MOVE",
                    kind: IntentKind::Buff,
                    body: vec![Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: crate::effects::AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    }],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::Cycle {
                moves: vec![
                    "ORB_OF_FRAILTY_MOVE",
                    "ORB_OF_WEAKNESS_MOVE",
                    "BEAM_MOVE",
                    "RITUAL_MOVE",
                ],
            },
        },
    );
    // PunchConstruct: READY (10 block) → STRONG_PUNCH (14) → FAST_PUNCH (5×2 + Weak).
    // Spawn: Artifact(1) — blocks the first debuff applied.
    m.insert(
        "PunchConstruct",
        MonsterAi {
            model_id: "PunchConstruct",
            moves: vec![
                MonsterMove {
                    id: "READY_MOVE",
                    kind: IntentKind::Defend,
                    body: vec![Effect::GainBlock {
                        amount: crate::effects::AmountSpec::Fixed(10),
                        target: Target::SelfActor,
                    }],
                },
                MonsterMove {
                    id: "STRONG_PUNCH_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(14),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "FAST_PUNCH_MOVE",
                    kind: IntentKind::AttackDebuff { hits: 2 },
                    body: vec![
                        Effect::DealDamage {
                            amount: crate::effects::AmountSpec::Fixed(5),
                            target: Target::ChosenEnemy,
                            hits: 2,
                        },
                        Effect::ApplyPower {
                            power_id: "WeakPower".to_string(),
                            amount: crate::effects::AmountSpec::Fixed(1),
                            target: Target::ChosenEnemy,
                        },
                    ],
                },
            ],
            spawn: vec![Effect::ApplyPower {
                power_id: "ArtifactPower".to_string(),
                amount: crate::effects::AmountSpec::Fixed(1),
                target: Target::SelfActor,
            }],
            pattern: MovePattern::Cycle {
                moves: vec!["READY_MOVE", "STRONG_PUNCH_MOVE", "FAST_PUNCH_MOVE"],
            },
        },
    );
}

/// Monsters with weighted-random patterns.
fn register_weighted_random_monsters(m: &mut HashMap<&'static str, MonsterAi>) {
    // FossilStalker: weighted random {LATCH:2, TACKLE:2, LASH:2}.
    // First turn: LATCH. Spawn: Suck(3).
    m.insert(
        "FossilStalker",
        MonsterAi {
            model_id: "FossilStalker",
            moves: vec![
                MonsterMove {
                    id: "TACKLE_MOVE",
                    kind: IntentKind::AttackDebuff { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: crate::effects::AmountSpec::Fixed(9),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        Effect::ApplyPower {
                            power_id: "FrailPower".to_string(),
                            amount: crate::effects::AmountSpec::Fixed(1),
                            target: Target::ChosenEnemy,
                        },
                    ],
                },
                MonsterMove {
                    id: "LATCH_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(12),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "LASH_MOVE",
                    kind: IntentKind::Attack { hits: 2 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(3),
                        target: Target::ChosenEnemy,
                        hits: 2,
                    }],
                },
            ],
            spawn: vec![Effect::ApplyPower {
                power_id: "SuckPower".to_string(),
                amount: crate::effects::AmountSpec::Fixed(3),
                target: Target::SelfActor,
            }],
            pattern: MovePattern::FirstTurnOverride {
                first_move: "LATCH_MOVE",
                then: Box::new(MovePattern::WeightedRandom {
                    weights: vec![
                        ("LATCH_MOVE", 2),
                        ("TACKLE_MOVE", 2),
                        ("LASH_MOVE", 2),
                    ],
                    no_repeat: vec![],
                }),
            },
        },
    );
    // HunterKiller: TENDERIZING_GOOP (Tender 1) first; then weighted
    // {BITE:1 no_repeat, PUNCTURE:2}.
    m.insert(
        "HunterKiller",
        MonsterAi {
            model_id: "HunterKiller",
            moves: vec![
                MonsterMove {
                    id: "TENDERIZING_GOOP_MOVE",
                    kind: IntentKind::Debuff,
                    // TenderPower (debuff Counter) — per C#, tracks
                    // CardsPlayedThisTurn and scales damage. Body not
                    // yet wired; apply by real name so the feature
                    // vector sees the right power.
                    body: vec![Effect::ApplyPower {
                        power_id: "TenderPower".to_string(),
                        amount: crate::effects::AmountSpec::Fixed(1),
                        target: Target::ChosenEnemy,
                    }],
                },
                MonsterMove {
                    id: "BITE_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(17),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "PUNCTURE_MOVE",
                    kind: IntentKind::Attack { hits: 3 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(7),
                        target: Target::ChosenEnemy,
                        hits: 3,
                    }],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::FirstTurnOverride {
                first_move: "TENDERIZING_GOOP_MOVE",
                then: Box::new(MovePattern::WeightedRandom {
                    weights: vec![("BITE_MOVE", 1), ("PUNCTURE_MOVE", 2)],
                    no_repeat: vec!["BITE_MOVE"],
                }),
            },
        },
    );
    // Flyconid: first turn weighted {FRAIL_SPORES:2, SMASH:1}, then
    // {VULNERABLE_SPORES:3, FRAIL_SPORES:2, SMASH:1}.
    m.insert(
        "Flyconid",
        MonsterAi {
            model_id: "Flyconid",
            moves: vec![
                MonsterMove {
                    id: "VULNERABLE_SPORES_MOVE",
                    kind: IntentKind::Debuff,
                    body: vec![Effect::ApplyPower {
                        power_id: "VulnerablePower".to_string(),
                        amount: crate::effects::AmountSpec::Fixed(2),
                        target: Target::ChosenEnemy,
                    }],
                },
                MonsterMove {
                    id: "FRAIL_SPORES_MOVE",
                    kind: IntentKind::AttackDebuff { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: crate::effects::AmountSpec::Fixed(8),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        Effect::ApplyPower {
                            power_id: "FrailPower".to_string(),
                            amount: crate::effects::AmountSpec::Fixed(2),
                            target: Target::ChosenEnemy,
                        },
                    ],
                },
                MonsterMove {
                    id: "SMASH_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(11),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::Conditional {
                predicate: AiCondition::FirstTurn,
                then_branch: Box::new(MovePattern::WeightedRandom {
                    weights: vec![("FRAIL_SPORES_MOVE", 2), ("SMASH_MOVE", 1)],
                    no_repeat: vec![],
                }),
                else_branch: Box::new(MovePattern::WeightedRandom {
                    weights: vec![
                        ("VULNERABLE_SPORES_MOVE", 3),
                        ("FRAIL_SPORES_MOVE", 2),
                        ("SMASH_MOVE", 1),
                    ],
                    no_repeat: vec![],
                }),
            },
        },
    );
    // Inklet: first turn slot conditional — middle slot starts on
    // WHIRLWIND; else weighted {JAB:2, WHIRLWIND:1 no_repeat,
    // PIERCING_GAZE:1 no_repeat}.
    m.insert(
        "Inklet",
        MonsterAi {
            model_id: "Inklet",
            moves: vec![
                MonsterMove {
                    id: "JAB_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(3),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "WHIRLWIND_MOVE",
                    kind: IntentKind::Attack { hits: 3 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(2),
                        target: Target::ChosenEnemy,
                        hits: 3,
                    }],
                },
                MonsterMove {
                    id: "PIERCING_GAZE_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: crate::effects::AmountSpec::Fixed(10),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::BySlot {
                branches: vec![(
                    "second",
                    MovePattern::FirstTurnOverride {
                        first_move: "WHIRLWIND_MOVE",
                        then: Box::new(MovePattern::WeightedRandom {
                            weights: vec![
                                ("JAB_MOVE", 2),
                                ("WHIRLWIND_MOVE", 1),
                                ("PIERCING_GAZE_MOVE", 1),
                            ],
                            no_repeat: vec!["WHIRLWIND_MOVE", "PIERCING_GAZE_MOVE"],
                        }),
                    },
                )],
                default: Box::new(MovePattern::WeightedRandom {
                    weights: vec![
                        ("JAB_MOVE", 2),
                        ("WHIRLWIND_MOVE", 1),
                        ("PIERCING_GAZE_MOVE", 1),
                    ],
                    no_repeat: vec!["WHIRLWIND_MOVE", "PIERCING_GAZE_MOVE"],
                }),
            },
        },
    );
}

/// Monsters whose state machine tracks a persistent flag (UseOnce
/// gates, one-shot HP thresholds, self-kill on play). The flag is
/// stored on the MonsterState and flipped via `Effect::SetMonsterFlag`
/// inside the move body. Predicates that read it use
/// `AiCondition::HasFlag`.
fn register_flag_state_monsters(m: &mut HashMap<&'static str, MonsterAi>) {
    use crate::effects::AmountSpec;
    // GasBomb: EXPLODE deals 8 damage, kills self. Single move,
    // single use. Spawn: MinionPower(1).
    m.insert(
        "GasBomb",
        MonsterAi {
            model_id: "GasBomb",
            moves: vec![MonsterMove {
                id: "EXPLODE_MOVE",
                kind: IntentKind::Attack { hits: 1 },
                body: vec![
                    Effect::DealDamage {
                        amount: AmountSpec::Fixed(8),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    },
                    Effect::KillSelf,
                ],
            }],
            spawn: vec![Effect::ApplyPower {
                power_id: "MinionPower".to_string(),
                amount: AmountSpec::Fixed(1),
                target: Target::SelfActor,
            }],
            pattern: MovePattern::Cycle { moves: vec!["EXPLODE_MOVE"] },
        },
    );

    // Mawler: weighted random {CLAW:1 no_repeat, RIP_AND_TEAR:1
    // no_repeat, ROAR:1 use-once}. The `roar_used` flag gates ROAR
    // out after its first play. First turn: CLAW.
    //
    // Encoded as a Conditional: if `roar_used` flag is set, pick from
    // CLAW + RIP_AND_TEAR only; else include ROAR. ROAR's body sets
    // the flag so subsequent rolls skip it.
    m.insert(
        "Mawler",
        MonsterAi {
            model_id: "Mawler",
            moves: vec![
                MonsterMove {
                    id: "CLAW_MOVE",
                    kind: IntentKind::Attack { hits: 2 },
                    body: vec![Effect::DealDamage {
                        amount: AmountSpec::Fixed(4),
                        target: Target::ChosenEnemy,
                        hits: 2,
                    }],
                },
                MonsterMove {
                    id: "RIP_AND_TEAR_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: AmountSpec::Fixed(14),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "ROAR_MOVE",
                    kind: IntentKind::Debuff,
                    body: vec![
                        Effect::ApplyPower {
                            power_id: "VulnerablePower".to_string(),
                            amount: AmountSpec::Fixed(3),
                            target: Target::ChosenEnemy,
                        },
                        Effect::SetMonsterFlag {
                            flag: "roar_used".to_string(),
                            value: true,
                        },
                    ],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::FirstTurnOverride {
                first_move: "CLAW_MOVE",
                then: Box::new(MovePattern::Conditional {
                    predicate: AiCondition::HasFlag("roar_used"),
                    // ROAR already used → only CLAW + RIP_AND_TEAR.
                    then_branch: Box::new(MovePattern::WeightedRandom {
                        weights: vec![("CLAW_MOVE", 1), ("RIP_AND_TEAR_MOVE", 1)],
                        no_repeat: vec!["CLAW_MOVE", "RIP_AND_TEAR_MOVE"],
                    }),
                    // ROAR still available → full 3-option weighted.
                    else_branch: Box::new(MovePattern::WeightedRandom {
                        weights: vec![
                            ("CLAW_MOVE", 1),
                            ("RIP_AND_TEAR_MOVE", 1),
                            ("ROAR_MOVE", 1),
                        ],
                        no_repeat: vec!["CLAW_MOVE", "RIP_AND_TEAR_MOVE"],
                    }),
                }),
            },
        },
    );

    // FrogKnight: cycle TONGUE_LASH → STRIKE_DOWN_EVIL → FOR_THE_QUEEN
    // → HALF_HEALTH-conditional → ... Spawn: Plating(15) + clears
    // beetle_charged flag implicitly (default false).
    //
    // HALF_HEALTH branch:
    //   if HasCharged || HP >= 50%   → play TONGUE_LASH
    //   else                          → play BEETLE_CHARGE (sets HasCharged)
    //
    // Encoded as a 4-position cycle where the 4th slot is a Conditional.
    // We approximate the "4th position" semantics by using LastMoveWas
    // FOR_THE_QUEEN as the gate for the conditional branch.
    m.insert(
        "FrogKnight",
        MonsterAi {
            model_id: "FrogKnight",
            moves: vec![
                MonsterMove {
                    id: "TONGUE_LASH_MOVE",
                    kind: IntentKind::AttackDebuff { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: AmountSpec::Fixed(13),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        Effect::ApplyPower {
                            power_id: "FrailPower".to_string(),
                            amount: AmountSpec::Fixed(2),
                            target: Target::ChosenEnemy,
                        },
                    ],
                },
                MonsterMove {
                    id: "STRIKE_DOWN_EVIL_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: AmountSpec::Fixed(21),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "FOR_THE_QUEEN_MOVE",
                    kind: IntentKind::Buff,
                    body: vec![Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(5),
                        target: Target::SelfActor,
                    }],
                },
                MonsterMove {
                    id: "BEETLE_CHARGE_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: AmountSpec::Fixed(35),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        Effect::SetMonsterFlag {
                            flag: "beetle_charged".to_string(),
                            value: true,
                        },
                    ],
                },
            ],
            spawn: vec![Effect::ApplyPower {
                power_id: "PlatingPower".to_string(),
                amount: AmountSpec::Fixed(15),
                target: Target::SelfActor,
            }],
            // First-turn override: TONGUE_LASH. Then 4-position cycle
            // with the 4th slot being conditional.
            pattern: MovePattern::FirstTurnOverride {
                first_move: "TONGUE_LASH_MOVE",
                then: Box::new(MovePattern::Conditional {
                    // After TONGUE_LASH → STRIKE_DOWN_EVIL.
                    predicate: AiCondition::LastMoveWas("TONGUE_LASH_MOVE"),
                    then_branch: Box::new(MovePattern::Cycle {
                        moves: vec!["STRIKE_DOWN_EVIL_MOVE"],
                    }),
                    else_branch: Box::new(MovePattern::Conditional {
                        // After STRIKE_DOWN_EVIL → FOR_THE_QUEEN.
                        predicate: AiCondition::LastMoveWas("STRIKE_DOWN_EVIL_MOVE"),
                        then_branch: Box::new(MovePattern::Cycle {
                            moves: vec!["FOR_THE_QUEEN_MOVE"],
                        }),
                        else_branch: Box::new(MovePattern::Conditional {
                            // After FOR_THE_QUEEN → HALF_HEALTH branch.
                            //   If HasCharged || HP >= 50% → TONGUE_LASH (loop)
                            //   Else                       → BEETLE_CHARGE
                            predicate: AiCondition::LastMoveWas("FOR_THE_QUEEN_MOVE"),
                            then_branch: Box::new(MovePattern::Conditional {
                                predicate: AiCondition::Or(
                                    Box::new(AiCondition::HasFlag("beetle_charged")),
                                    Box::new(AiCondition::Not(Box::new(AiCondition::HpBelowPct(50)))),
                                ),
                                then_branch: Box::new(MovePattern::Cycle {
                                    moves: vec!["TONGUE_LASH_MOVE"],
                                }),
                                else_branch: Box::new(MovePattern::Cycle {
                                    moves: vec!["BEETLE_CHARGE_MOVE"],
                                }),
                            }),
                            // Anything else (BEETLE_CHARGE just fired) → TONGUE_LASH (resume cycle).
                            else_branch: Box::new(MovePattern::Cycle {
                                moves: vec!["TONGUE_LASH_MOVE"],
                            }),
                        }),
                    }),
                }),
            },
        },
    );

    // Fogmog: ILLUSION → SWIPE → weighted{SWIPE:0.4, HEADBUTT:0.6
    // cannot_repeat} → ... ILLUSION spawns an EyeWithTeeth.
    m.insert(
        "Fogmog",
        MonsterAi {
            model_id: "Fogmog",
            moves: vec![
                MonsterMove {
                    id: "ILLUSION_MOVE",
                    kind: IntentKind::Summon,
                    body: vec![Effect::SummonMonster {
                        monster_id: "EyeWithTeeth".to_string(),
                        slot: "illusion".to_string(),
                    }],
                },
                MonsterMove {
                    id: "SWIPE_MOVE",
                    kind: IntentKind::AttackBuff { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: AmountSpec::Fixed(8),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        Effect::ApplyPower {
                            power_id: "StrengthPower".to_string(),
                            amount: AmountSpec::Fixed(1),
                            target: Target::SelfActor,
                        },
                    ],
                },
                MonsterMove {
                    id: "HEADBUTT_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: AmountSpec::Fixed(14),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::FirstTurnOverride {
                first_move: "ILLUSION_MOVE",
                then: Box::new(MovePattern::Conditional {
                    // After ILLUSION → SWIPE.
                    predicate: AiCondition::LastMoveWas("ILLUSION_MOVE"),
                    then_branch: Box::new(MovePattern::Cycle {
                        moves: vec!["SWIPE_MOVE"],
                    }),
                    // After SWIPE or HEADBUTT → weighted random
                    // {SWIPE:0.4, HEADBUTT:0.6}, no_repeat on both.
                    else_branch: Box::new(MovePattern::WeightedRandom {
                        weights: vec![("SWIPE_MOVE", 4), ("HEADBUTT_MOVE", 6)],
                        no_repeat: vec!["SWIPE_MOVE", "HEADBUTT_MOVE"],
                    }),
                }),
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
    fn ai_registry_covers_new_monsters() {
        // 5 RubyRaiders + 2 test + 3 gremlins + 7 single + 3 two-move
        // + 4 three-move + 4 weighted + 4 flag-state = 32 monsters.
        let expected = [
            "AxeRubyRaider", "CrossbowRubyRaider", "BruteRubyRaider",
            "AssassinRubyRaider", "TrackerRubyRaider",
            "SingleAttackMoveMonster", "MultiAttackMoveMonster",
            "FatGremlin", "SneakyGremlin", "GremlinMerc",
            "Byrdpip", "Stabbot", "Zapbot", "Noisebot",
            "EyeWithTeeth", "Parafright", "SnappingJaxfruit",
            "DampCultist", "SewerClam", "ToughEgg",
            "VineShambler", "KinFollower", "KinPriest", "PunchConstruct",
            "FossilStalker", "HunterKiller", "Flyconid", "Inklet",
            "GasBomb", "Mawler", "FrogKnight", "Fogmog",
        ];
        for id in expected {
            assert!(ai_for(id).is_some(), "Missing AI for {}", id);
        }
        assert_eq!(MONSTER_AI_REGISTRY.len(), expected.len());
    }

    #[test]
    fn damp_cultist_opens_with_incantation() {
        let ai = ai_for("DampCultist").unwrap();
        let cs = CombatState::empty();
        let mut rng = Rng::new(0, 0);
        assert_eq!(
            pick_next_move(&ai.pattern, &cs, 0, None, "front", &mut rng),
            Some("INCANTATION_MOVE")
        );
    }

    #[test]
    fn sewer_clam_first_turn_jet_then_alternates() {
        let ai = ai_for("SewerClam").unwrap();
        let cs = CombatState::empty();
        let mut rng = Rng::new(0, 0);
        assert_eq!(
            pick_next_move(&ai.pattern, &cs, 0, None, "front", &mut rng),
            Some("JET_MOVE")
        );
        assert_eq!(
            pick_next_move(&ai.pattern, &cs, 0, Some("JET_MOVE"), "front", &mut rng),
            Some("PRESSURIZE_MOVE")
        );
        assert_eq!(
            pick_next_move(&ai.pattern, &cs, 0, Some("PRESSURIZE_MOVE"), "front", &mut rng),
            Some("JET_MOVE")
        );
    }

    #[test]
    fn inklet_middle_slot_starts_on_whirlwind() {
        let ai = ai_for("Inklet").unwrap();
        let cs = CombatState::empty();
        let mut rng = Rng::new(0, 0);
        // Middle slot ("second" in C# encoder).
        assert_eq!(
            pick_next_move(&ai.pattern, &cs, 0, None, "second", &mut rng),
            Some("WHIRLWIND_MOVE")
        );
    }

    #[test]
    fn hunter_killer_opens_with_tenderizing_goop() {
        let ai = ai_for("HunterKiller").unwrap();
        let cs = CombatState::empty();
        let mut rng = Rng::new(0, 0);
        assert_eq!(
            pick_next_move(&ai.pattern, &cs, 0, None, "front", &mut rng),
            Some("TENDERIZING_GOOP_MOVE")
        );
        // After GOOP we're in weighted mode → either BITE or PUNCTURE.
        let after = pick_next_move(
            &ai.pattern,
            &cs,
            0,
            Some("TENDERIZING_GOOP_MOVE"),
            "front",
            &mut rng,
        );
        assert!(matches!(after, Some("BITE_MOVE") | Some("PUNCTURE_MOVE")));
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
