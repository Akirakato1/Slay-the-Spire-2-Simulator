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

impl MonsterMove {
    /// Single- or multi-hit attack on the targeted player. The
    /// `hits` field on `DealDamage` re-rolls modifiers per hit
    /// (StS rules for block recompute between hits).
    pub fn attack(id: &'static str, damage: i32, hits: i32) -> Self {
        Self {
            id,
            kind: IntentKind::Attack { hits },
            body: vec![Effect::DealDamage {
                amount: crate::effects::AmountSpec::Fixed(damage),
                target: Target::ChosenEnemy,
                hits,
            }],
        }
    }

    /// Pure block gain on self. No damage.
    pub fn defend(id: &'static str, block: i32) -> Self {
        Self {
            id,
            kind: IntentKind::Defend,
            body: vec![Effect::GainBlock {
                amount: crate::effects::AmountSpec::Fixed(block),
                target: Target::SelfActor,
            }],
        }
    }

    /// Self-buff via a power apply. Common: Strength, Plating, Dexterity.
    pub fn buff(id: &'static str, power_id: &str, amount: i32) -> Self {
        Self {
            id,
            kind: IntentKind::Buff,
            body: vec![Effect::ApplyPower {
                power_id: power_id.to_string(),
                amount: crate::effects::AmountSpec::Fixed(amount),
                target: Target::SelfActor,
            }],
        }
    }

    /// Player-targeting debuff via a power apply. Common: Weak,
    /// Frail, Vulnerable.
    pub fn debuff(id: &'static str, power_id: &str, amount: i32) -> Self {
        Self {
            id,
            kind: IntentKind::Debuff,
            body: vec![Effect::ApplyPower {
                power_id: power_id.to_string(),
                amount: crate::effects::AmountSpec::Fixed(amount),
                target: Target::ChosenEnemy,
            }],
        }
    }

    /// Damage + apply a debuff to the same target. The most common
    /// "attack with rider" pattern: BruteRubyRaider STOMP (6 dmg +
    /// Weak), AssassinRubyRaider, etc.
    pub fn attack_debuff(
        id: &'static str,
        damage: i32,
        hits: i32,
        debuff_id: &str,
        debuff_amount: i32,
    ) -> Self {
        Self {
            id,
            kind: IntentKind::AttackDebuff { hits },
            body: vec![
                Effect::DealDamage {
                    amount: crate::effects::AmountSpec::Fixed(damage),
                    target: Target::ChosenEnemy,
                    hits,
                },
                Effect::ApplyPower {
                    power_id: debuff_id.to_string(),
                    amount: crate::effects::AmountSpec::Fixed(debuff_amount),
                    target: Target::ChosenEnemy,
                },
            ],
        }
    }

    /// Damage + self-buff. KinFollower QUICK_SLASH-with-strength,
    /// SnappingJaxfruit ENERGY_ORB, Fogmog SWIPE.
    pub fn attack_buff(
        id: &'static str,
        damage: i32,
        hits: i32,
        buff_id: &str,
        buff_amount: i32,
    ) -> Self {
        Self {
            id,
            kind: IntentKind::AttackBuff { hits },
            body: vec![
                Effect::DealDamage {
                    amount: crate::effects::AmountSpec::Fixed(damage),
                    target: Target::ChosenEnemy,
                    hits,
                },
                Effect::ApplyPower {
                    power_id: buff_id.to_string(),
                    amount: crate::effects::AmountSpec::Fixed(buff_amount),
                    target: Target::SelfActor,
                },
            ],
        }
    }

    /// Damage + self-block. Defensive-attack pattern: Nibbit SLICE.
    pub fn attack_defend(
        id: &'static str,
        damage: i32,
        hits: i32,
        block: i32,
    ) -> Self {
        Self {
            id,
            kind: IntentKind::AttackDefend { hits },
            body: vec![
                Effect::DealDamage {
                    amount: crate::effects::AmountSpec::Fixed(damage),
                    target: Target::ChosenEnemy,
                    hits,
                },
                Effect::GainBlock {
                    amount: crate::effects::AmountSpec::Fixed(block),
                    target: Target::SelfActor,
                },
            ],
        }
    }

    /// No-effect move. Byrdpip NOTHING_MOVE.
    pub fn sleep(id: &'static str) -> Self {
        Self {
            id,
            kind: IntentKind::Sleep,
            body: vec![],
        }
    }

    /// Compose with an extra body suffix. Useful when adding a flag
    /// set or self-kill to a builder-constructed move.
    pub fn with_extra(mut self, extra: Vec<Effect>) -> Self {
        self.body.extend(extra);
        self
    }
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
    register_migrated_legacy(&mut m);
    register_migrated_legacy_b2(&mut m);
    register_migrated_legacy_b3(&mut m);
    register_bosses_b4(&mut m);
    register_misc_b5(&mut m);
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

/// Monsters previously implemented in the legacy hand-rolled
/// dispatcher (monster_dispatch.rs match arms + combat.rs
/// pick_*/execute_* fns), now expressed as data. The dispatcher
/// auto-prefers the data-driven path when a monster appears in the
/// registry, so adding here transparently replaces the hand-rolled
/// version. Provides oracle-grade validation: the existing
/// combat-side tests for these monsters must still pass.
fn register_migrated_legacy(m: &mut HashMap<&'static str, MonsterAi>) {
    use crate::effects::AmountSpec;

    // Axebot: first-turn BOOT_UP_MOVE, then weighted
    // {ONE_TWO:2, SHARPEN:1 (blocked-if-just-played), HAMMER_UPPERCUT:2}.
    // No spawn payload. Damage / block / power numbers from C#
    // Axebot.cs (A0 — DeadlyEnemies values higher).
    // Myte: cycle Toxic → Bite → Suck → Toxic → ... with first-turn
    // determined by slot (slot "second" starts Suck; otherwise Toxic).
    // No spawn payload. C# Myte.cs (A0).
    m.insert(
        "Myte",
        MonsterAi {
            model_id: "Myte",
            moves: vec![
                MonsterMove {
                    id: "TOXIC_MOVE",
                    kind: IntentKind::Debuff,
                    body: vec![
                        Effect::AddCardToPile {
                            card_id: "Toxic".to_string(),
                            upgrade: 0,
                            pile: crate::effects::Pile::Hand,
                        },
                        Effect::AddCardToPile {
                            card_id: "Toxic".to_string(),
                            upgrade: 0,
                            pile: crate::effects::Pile::Hand,
                        },
                    ],
                },
                MonsterMove {
                    id: "BITE_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: AmountSpec::Fixed(13),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "SUCK_MOVE",
                    kind: IntentKind::AttackBuff { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: AmountSpec::Fixed(4),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        Effect::ApplyPower {
                            power_id: "StrengthPower".to_string(),
                            amount: AmountSpec::Fixed(2),
                            target: Target::SelfActor,
                        },
                    ],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::BySlot {
                branches: vec![(
                    "second",
                    MovePattern::FirstTurnOverride {
                        first_move: "SUCK_MOVE",
                        then: Box::new(MovePattern::Cycle {
                            moves: vec!["TOXIC_MOVE", "BITE_MOVE", "SUCK_MOVE"],
                        }),
                    },
                )],
                default: Box::new(MovePattern::Cycle {
                    moves: vec!["TOXIC_MOVE", "BITE_MOVE", "SUCK_MOVE"],
                }),
            },
        },
    );

    // Nibbit: cycle Butt → Slice → Hiss with conditional first turn:
    //   if alone (1 living enemy): BUTT
    //   else if front: SLICE
    //   else: HISS
    // Encoded as Conditional(LivingEnemyCountEquals(1)) wrapping BySlot.
    m.insert(
        "Nibbit",
        MonsterAi {
            model_id: "Nibbit",
            moves: vec![
                MonsterMove {
                    id: "BUTT_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: AmountSpec::Fixed(12),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
                MonsterMove {
                    id: "SLICE_MOVE",
                    kind: IntentKind::AttackDefend { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: AmountSpec::Fixed(6),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        Effect::GainBlock {
                            amount: AmountSpec::Fixed(5),
                            target: Target::SelfActor,
                        },
                    ],
                },
                MonsterMove {
                    id: "HISS_MOVE",
                    kind: IntentKind::Buff,
                    body: vec![Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    }],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::Conditional {
                // First-turn-only branching; subsequent turns use the
                // base cycle regardless.
                predicate: AiCondition::FirstTurn,
                then_branch: Box::new(MovePattern::Conditional {
                    predicate: AiCondition::LivingEnemyCountEquals(1),
                    // Alone → BUTT.
                    then_branch: Box::new(MovePattern::Cycle {
                        moves: vec!["BUTT_MOVE"],
                    }),
                    else_branch: Box::new(MovePattern::BySlot {
                        // Front slots → SLICE; back slots → HISS.
                        branches: vec![
                            ("front", MovePattern::Cycle { moves: vec!["SLICE_MOVE"] }),
                            ("first", MovePattern::Cycle { moves: vec!["SLICE_MOVE"] }),
                        ],
                        default: Box::new(MovePattern::Cycle {
                            moves: vec!["HISS_MOVE"],
                        }),
                    }),
                }),
                else_branch: Box::new(MovePattern::Cycle {
                    moves: vec!["BUTT_MOVE", "SLICE_MOVE", "HISS_MOVE"],
                }),
            },
        },
    );

    // FlailKnight: first-turn RAM, then weighted random
    // {WAR_CHANT:1 (no_repeat), FLAIL:2, RAM:2}. No spawn.
    m.insert(
        "FlailKnight",
        MonsterAi {
            model_id: "FlailKnight",
            moves: vec![
                MonsterMove {
                    id: "WAR_CHANT",
                    kind: IntentKind::Buff,
                    body: vec![Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(3),
                        target: Target::SelfActor,
                    }],
                },
                MonsterMove {
                    id: "FLAIL_MOVE",
                    kind: IntentKind::Attack { hits: 2 },
                    body: vec![Effect::DealDamage {
                        amount: AmountSpec::Fixed(9),
                        target: Target::ChosenEnemy,
                        hits: 2,
                    }],
                },
                MonsterMove {
                    id: "RAM_MOVE",
                    kind: IntentKind::Attack { hits: 1 },
                    body: vec![Effect::DealDamage {
                        amount: AmountSpec::Fixed(15),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    }],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::FirstTurnOverride {
                first_move: "RAM_MOVE",
                then: Box::new(MovePattern::WeightedRandom {
                    weights: vec![
                        ("WAR_CHANT", 1),
                        ("FLAIL_MOVE", 2),
                        ("RAM_MOVE", 2),
                    ],
                    no_repeat: vec!["WAR_CHANT"],
                }),
            },
        },
    );

    // Toadpole: triangle cycle Spiken → SpikeSpit → Whirl → Spiken.
    // First turn: front slot starts on Spiken; back slot starts on
    // Whirl. SpikeSpit subtracts 2 ThornsPower from self, then deals
    // 1×3 damage (consumes the Thorns buff Spiken built up).
    m.insert(
        "Toadpole",
        MonsterAi {
            model_id: "Toadpole",
            moves: vec![
                MonsterMove::buff("SPIKEN_MOVE", "ThornsPower", 2),
                MonsterMove {
                    id: "SPIKE_SPIT_MOVE",
                    kind: IntentKind::Attack { hits: 3 },
                    body: vec![
                        // Spend 2 ThornsPower stacks (C# uses Apply<Thorns>(-amount)).
                        Effect::ApplyPower {
                            power_id: "ThornsPower".to_string(),
                            amount: AmountSpec::Fixed(-2),
                            target: Target::SelfActor,
                        },
                        Effect::DealDamage {
                            amount: AmountSpec::Fixed(1),
                            target: Target::ChosenEnemy,
                            hits: 3,
                        },
                    ],
                },
                MonsterMove::attack("WHIRL_MOVE", 7, 1),
            ],
            spawn: vec![],
            pattern: MovePattern::BySlot {
                // Front slot: enter at Spiken (idx 0).
                // Back slot: enter at Whirl (idx 2), wraps to Spiken next.
                branches: vec![
                    ("front", MovePattern::FirstTurnOverride {
                        first_move: "SPIKEN_MOVE",
                        then: Box::new(MovePattern::Cycle {
                            moves: vec!["SPIKEN_MOVE", "SPIKE_SPIT_MOVE", "WHIRL_MOVE"],
                        }),
                    }),
                    ("first", MovePattern::FirstTurnOverride {
                        first_move: "SPIKEN_MOVE",
                        then: Box::new(MovePattern::Cycle {
                            moves: vec!["SPIKEN_MOVE", "SPIKE_SPIT_MOVE", "WHIRL_MOVE"],
                        }),
                    }),
                ],
                default: Box::new(MovePattern::FirstTurnOverride {
                    first_move: "WHIRL_MOVE",
                    then: Box::new(MovePattern::Cycle {
                        moves: vec!["SPIKEN_MOVE", "SPIKE_SPIT_MOVE", "WHIRL_MOVE"],
                    }),
                }),
            },
        },
    );

    // ---- Simple cycle / weighted-random batch via builders ----
    // Numbers from each monster's combat.rs pick/execute pair (A0).

    // CalcifiedCultist: cycle INCANTATION(+ritual 3) → DARK_STRIKE(9).
    m.insert("CalcifiedCultist", MonsterAi {
        model_id: "CalcifiedCultist",
        moves: vec![
            MonsterMove::buff("INCANTATION_MOVE", "RitualPower", 3),
            MonsterMove::attack("DARK_STRIKE_MOVE", 9, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle { moves: vec!["INCANTATION_MOVE", "DARK_STRIKE_MOVE"] },
    });

    // DevotedSculptor: cycle FORBIDDEN_INCANTATION(+ritual 9) → SAVAGE(12).
    m.insert("DevotedSculptor", MonsterAi {
        model_id: "DevotedSculptor",
        moves: vec![
            MonsterMove::buff("FORBIDDEN_INCANTATION_MOVE", "RitualPower", 9),
            MonsterMove::attack("SAVAGE_MOVE", 12, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec!["FORBIDDEN_INCANTATION_MOVE", "SAVAGE_MOVE"],
        },
    });

    // Seapunk: cycle SEA_KICK(11) → SPINNING_KICK(2×4) → BUBBLE_BURP(block 7 + str 1).
    m.insert("Seapunk", MonsterAi {
        model_id: "Seapunk",
        moves: vec![
            MonsterMove::attack("SEA_KICK_MOVE", 11, 1),
            MonsterMove::attack("SPINNING_KICK_MOVE", 2, 4),
            MonsterMove {
                id: "BUBBLE_BURP_MOVE",
                kind: IntentKind::Defend,
                body: vec![
                    Effect::GainBlock {
                        amount: AmountSpec::Fixed(7),
                        target: Target::SelfActor,
                    },
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(1),
                        target: Target::SelfActor,
                    },
                ],
            },
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec!["SEA_KICK_MOVE", "SPINNING_KICK_MOVE", "BUBBLE_BURP_MOVE"],
        },
    });

    // GlobeHead: cycle SHOCKING_SLAP(13 + Frail 2) → THUNDER_STRIKE(6×3) → GALVANIC_BURST(16 + Str 2).
    m.insert("GlobeHead", MonsterAi {
        model_id: "GlobeHead",
        moves: vec![
            MonsterMove::attack_debuff("SHOCKING_SLAP_MOVE", 13, 1, "FrailPower", 2),
            MonsterMove::attack("THUNDER_STRIKE_MOVE", 6, 3),
            MonsterMove::attack_buff("GALVANIC_BURST_MOVE", 16, 1, "StrengthPower", 2),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "SHOCKING_SLAP_MOVE",
                "THUNDER_STRIKE_MOVE",
                "GALVANIC_BURST_MOVE",
            ],
        },
    });

    // TwigSlimeS: single move loop BUTT(4).
    m.insert("TwigSlimeS", MonsterAi {
        model_id: "TwigSlimeS",
        moves: vec![MonsterMove::attack("BUTT_MOVE", 4, 1)],
        spawn: vec![],
        pattern: MovePattern::Cycle { moves: vec!["BUTT_MOVE"] },
    });

    // LeafSlimeM: strict alternation STICKY_SHOT(+slimed 2) ↔ CLUMP_SHOT(8).
    m.insert("LeafSlimeM", MonsterAi {
        model_id: "LeafSlimeM",
        moves: vec![
            MonsterMove {
                id: "STICKY_SHOT_MOVE",
                kind: IntentKind::Debuff,
                body: vec![
                    Effect::AddCardToPile {
                        card_id: "Slimed".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                    Effect::AddCardToPile {
                        card_id: "Slimed".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                ],
            },
            MonsterMove::attack("CLUMP_SHOT_MOVE", 8, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec!["STICKY_SHOT_MOVE", "CLUMP_SHOT_MOVE"],
        },
    });

    // BowlbugEgg: single move loop BITE (deal 7 + gain 7 block — attack_defend builder).
    m.insert("BowlbugEgg", MonsterAi {
        model_id: "BowlbugEgg",
        moves: vec![MonsterMove::attack_defend("BITE_MOVE", 7, 1, 7)],
        spawn: vec![],
        pattern: MovePattern::Cycle { moves: vec!["BITE_MOVE"] },
    });

    // Vantom: 4-position cycle INK_BLOT(7) → INKY_LANCE(6×2) → DISMEMBER(27+Wound 3) → PREPARE(+str 2).
    m.insert("Vantom", MonsterAi {
        model_id: "Vantom",
        moves: vec![
            MonsterMove::attack("INK_BLOT_MOVE", 7, 1),
            MonsterMove::attack("INKY_LANCE_MOVE", 6, 2),
            MonsterMove {
                id: "DISMEMBER_MOVE",
                kind: IntentKind::AttackDebuff { hits: 1 },
                body: vec![
                    Effect::DealDamage {
                        amount: AmountSpec::Fixed(27),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    },
                    // Wound is a status card, not a power. C# uses
                    // CardPileCmd.AddToCombat<Wound>(3).
                    Effect::AddCardToPile {
                        card_id: "Wound".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                    Effect::AddCardToPile {
                        card_id: "Wound".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                    Effect::AddCardToPile {
                        card_id: "Wound".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                ],
            },
            MonsterMove::buff("PREPARE_MOVE", "StrengthPower", 2),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "INK_BLOT_MOVE",
                "INKY_LANCE_MOVE",
                "DISMEMBER_MOVE",
                "PREPARE_MOVE",
            ],
        },
    });

    // SpinyToad: cycle SPIKES(+thorns 5) → EXPLOSION(23, -thorns 5) → LASH(17).
    m.insert("SpinyToad", MonsterAi {
        model_id: "SpinyToad",
        moves: vec![
            MonsterMove::buff("SPIKES_MOVE", "ThornsPower", 5),
            MonsterMove {
                id: "EXPLOSION_MOVE",
                kind: IntentKind::Attack { hits: 1 },
                body: vec![
                    Effect::ApplyPower {
                        power_id: "ThornsPower".to_string(),
                        amount: AmountSpec::Fixed(-5),
                        target: Target::SelfActor,
                    },
                    Effect::DealDamage {
                        amount: AmountSpec::Fixed(23),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    },
                ],
            },
            MonsterMove::attack("LASH_MOVE", 17, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec!["SPIKES_MOVE", "EXPLOSION_MOVE", "LASH_MOVE"],
        },
    });

    // SlimedBerserker: cycle VOMIT_ICHOR (10 Slimed) → FURIOUS_PUMMELING (4×4)
    //   → LEECHING_HUG (Weak 3 + Str 3) → SMOTHER (30).
    m.insert("SlimedBerserker", MonsterAi {
        model_id: "SlimedBerserker",
        moves: vec![
            MonsterMove {
                id: "VOMIT_ICHOR_MOVE",
                kind: IntentKind::Debuff,
                body: (0..10).map(|_| Effect::AddCardToPile {
                    card_id: "Slimed".to_string(),
                    upgrade: 0,
                    pile: crate::effects::Pile::Discard,
                }).collect(),
            },
            MonsterMove::attack("FURIOUS_PUMMELING_MOVE", 4, 4),
            MonsterMove {
                id: "LEECHING_HUG_MOVE",
                kind: IntentKind::Debuff,
                body: vec![
                    Effect::ApplyPower {
                        power_id: "WeakPower".to_string(),
                        amount: AmountSpec::Fixed(3),
                        target: Target::ChosenEnemy,
                    },
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(3),
                        target: Target::SelfActor,
                    },
                ],
            },
            MonsterMove::attack("SMOTHER_MOVE", 30, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "VOMIT_ICHOR_MOVE",
                "FURIOUS_PUMMELING_MOVE",
                "LEECHING_HUG_MOVE",
                "SMOTHER_MOVE",
            ],
        },
    });

    // PhrogParasite: strict alternation INFECT (Infection 3) ↔ LASH (4×4).
    m.insert("PhrogParasite", MonsterAi {
        model_id: "PhrogParasite",
        moves: vec![
            MonsterMove::debuff("INFECT_MOVE", "InfectionPower", 3),
            MonsterMove::attack("LASH_MOVE", 4, 4),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec!["INFECT_MOVE", "LASH_MOVE"],
        },
    });

    // SoulFysh: cycle BECKON(+Beckon 2) → DE_GAS(16) → GAZE(7 + Beckon 1)
    //   → FADE(+Intangible 2) → SCREAM(11 + Vulnerable 3).
    m.insert("SoulFysh", MonsterAi {
        model_id: "SoulFysh",
        moves: vec![
            MonsterMove::buff("BECKON_MOVE", "BeckonPower", 2),
            MonsterMove::attack("DE_GAS_MOVE", 16, 1),
            MonsterMove::attack_buff("GAZE_MOVE", 7, 1, "BeckonPower", 1),
            MonsterMove::buff("FADE_MOVE", "IntangiblePower", 2),
            MonsterMove::attack_debuff("SCREAM_MOVE", 11, 1, "VulnerablePower", 3),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "BECKON_MOVE",
                "DE_GAS_MOVE",
                "GAZE_MOVE",
                "FADE_MOVE",
                "SCREAM_MOVE",
            ],
        },
    });

    // TurretOperator: cycle UNLOAD1(3×5) → UNLOAD2(3×5) → RELOAD(+Str 1).
    m.insert("TurretOperator", MonsterAi {
        model_id: "TurretOperator",
        moves: vec![
            MonsterMove::attack("UNLOAD1_MOVE", 3, 5),
            MonsterMove::attack("UNLOAD2_MOVE", 3, 5),
            MonsterMove::buff("RELOAD_MOVE", "StrengthPower", 1),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec!["UNLOAD1_MOVE", "UNLOAD2_MOVE", "RELOAD_MOVE"],
        },
    });

    // OwlMagistrate: cycle SCRUTINY(16) → PECK_ASSAULT(4×6)
    //   → JUDICIAL_FLIGHT(+SoarPower 1) → VERDICT(33 + Vulnerable 4 - SoarPower).
    m.insert("OwlMagistrate", MonsterAi {
        model_id: "OwlMagistrate",
        moves: vec![
            MonsterMove::attack("MAGISTRATE_SCRUTINY", 16, 1),
            MonsterMove::attack("PECK_ASSAULT", 4, 6),
            MonsterMove::buff("JUDICIAL_FLIGHT", "SoarPower", 1),
            MonsterMove {
                id: "VERDICT",
                kind: IntentKind::AttackDebuff { hits: 1 },
                body: vec![
                    Effect::DealDamage {
                        amount: AmountSpec::Fixed(33),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    },
                    Effect::ApplyPower {
                        power_id: "VulnerablePower".to_string(),
                        amount: AmountSpec::Fixed(4),
                        target: Target::ChosenEnemy,
                    },
                    Effect::ApplyPower {
                        power_id: "SoarPower".to_string(),
                        amount: AmountSpec::Fixed(-1),
                        target: Target::SelfActor,
                    },
                ],
            },
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "MAGISTRATE_SCRUTINY",
                "PECK_ASSAULT",
                "JUDICIAL_FLIGHT",
                "VERDICT",
            ],
        },
    });

    // SoulNexus: SOUL_BURN(29) first, then weighted random
    //   {MAELSTROM(6×4) no_repeat, DRAIN_LIFE(18 + Vuln 2 + Weak 2) no_repeat}.
    m.insert("SoulNexus", MonsterAi {
        model_id: "SoulNexus",
        moves: vec![
            MonsterMove::attack("SOUL_BURN_MOVE", 29, 1),
            MonsterMove::attack("MAELSTROM_MOVE", 6, 4),
            MonsterMove {
                id: "DRAIN_LIFE_MOVE",
                kind: IntentKind::AttackDebuff { hits: 1 },
                body: vec![
                    Effect::DealDamage {
                        amount: AmountSpec::Fixed(18),
                        target: Target::ChosenEnemy,
                        hits: 1,
                    },
                    Effect::ApplyPower {
                        power_id: "VulnerablePower".to_string(),
                        amount: AmountSpec::Fixed(2),
                        target: Target::ChosenEnemy,
                    },
                    Effect::ApplyPower {
                        power_id: "WeakPower".to_string(),
                        amount: AmountSpec::Fixed(2),
                        target: Target::ChosenEnemy,
                    },
                ],
            },
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "SOUL_BURN_MOVE",
            then: Box::new(MovePattern::WeightedRandom {
                weights: vec![("MAELSTROM_MOVE", 1), ("DRAIN_LIFE_MOVE", 1)],
                no_repeat: vec!["MAELSTROM_MOVE", "DRAIN_LIFE_MOVE"],
            }),
        },
    });

    // ThievingHopper: chain Thievery → Flutter → HatTrick → Nab →
    // Escape (loop). Spawn: EscapeArtistPower(5). Steal-cards and
    // Flutter mechanics are deferred — encoded as plain damage moves
    // here. ESCAPE_MOVE uses the new Effect::EscapeFromCombat.
    m.insert(
        "ThievingHopper",
        MonsterAi {
            model_id: "ThievingHopper",
            moves: vec![
                MonsterMove::attack("THIEVERY_MOVE", 17, 1),
                MonsterMove::buff("FLUTTER_MOVE", "FlutterPower", 5),
                MonsterMove::attack("HAT_TRICK_MOVE", 21, 1),
                MonsterMove::attack("NAB_MOVE", 14, 1),
                MonsterMove {
                    id: "ESCAPE_MOVE",
                    kind: IntentKind::Sleep,
                    body: vec![Effect::EscapeFromCombat],
                },
            ],
            spawn: vec![Effect::ApplyPower {
                power_id: "EscapeArtistPower".to_string(),
                amount: AmountSpec::Fixed(5),
                target: Target::SelfActor,
            }],
            pattern: MovePattern::Cycle {
                moves: vec![
                    "THIEVERY_MOVE",
                    "FLUTTER_MOVE",
                    "HAT_TRICK_MOVE",
                    "NAB_MOVE",
                    "ESCAPE_MOVE",
                ],
            },
        },
    );

    m.insert(
        "Axebot",
        MonsterAi {
            model_id: "Axebot",
            moves: vec![
                MonsterMove {
                    id: "BOOT_UP_MOVE",
                    kind: IntentKind::Defend,
                    body: vec![
                        Effect::GainBlock {
                            amount: AmountSpec::Fixed(10),
                            target: Target::SelfActor,
                        },
                        Effect::ApplyPower {
                            power_id: "StrengthPower".to_string(),
                            amount: AmountSpec::Fixed(1),
                            target: Target::SelfActor,
                        },
                    ],
                },
                MonsterMove {
                    id: "ONE_TWO_MOVE",
                    kind: IntentKind::Attack { hits: 2 },
                    body: vec![Effect::DealDamage {
                        amount: AmountSpec::Fixed(5),
                        target: Target::ChosenEnemy,
                        hits: 2,
                    }],
                },
                MonsterMove {
                    id: "SHARPEN_MOVE",
                    kind: IntentKind::Buff,
                    body: vec![Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(4),
                        target: Target::SelfActor,
                    }],
                },
                MonsterMove {
                    id: "HAMMER_UPPERCUT_MOVE",
                    kind: IntentKind::AttackDebuff { hits: 1 },
                    body: vec![
                        Effect::DealDamage {
                            amount: AmountSpec::Fixed(8),
                            target: Target::ChosenEnemy,
                            hits: 1,
                        },
                        Effect::ApplyPower {
                            power_id: "WeakPower".to_string(),
                            amount: AmountSpec::Fixed(1),
                            target: Target::ChosenEnemy,
                        },
                        Effect::ApplyPower {
                            power_id: "FrailPower".to_string(),
                            amount: AmountSpec::Fixed(1),
                            target: Target::ChosenEnemy,
                        },
                    ],
                },
            ],
            spawn: vec![],
            pattern: MovePattern::FirstTurnOverride {
                first_move: "BOOT_UP_MOVE",
                then: Box::new(MovePattern::WeightedRandom {
                    weights: vec![
                        ("ONE_TWO_MOVE", 2),
                        ("SHARPEN_MOVE", 1),
                        ("HAMMER_UPPERCUT_MOVE", 2),
                    ],
                    no_repeat: vec!["SHARPEN_MOVE"],
                }),
            },
        },
    );
}

/// Second batch of legacy migrations. C# damage / block / power
/// values pulled directly from each monster's combat.rs port.
#[allow(clippy::too_many_lines)]
fn register_migrated_legacy_b2(m: &mut HashMap<&'static str, MonsterAi>) {
    use crate::effects::AmountSpec;

    // BowlbugSilk: TOXIC_SPIT(+Weak 1) ↔ TRASH(4×2). Strict alternation.
    m.insert("BowlbugSilk", MonsterAi {
        model_id: "BowlbugSilk",
        moves: vec![
            MonsterMove::debuff("TOXIC_SPIT_MOVE", "WeakPower", 1),
            MonsterMove::attack("TRASH_MOVE", 4, 2),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle { moves: vec!["TOXIC_SPIT_MOVE", "TRASH_MOVE"] },
    });

    // BowlbugNectar: Thrash(3) → Buff(+15 Str) → Thrash2(3) → Thrash2(loop).
    // After 3 turns, locks on Thrash2 forever. Encoded as nested
    // Conditional on LastMoveWas.
    m.insert("BowlbugNectar", MonsterAi {
        model_id: "BowlbugNectar",
        moves: vec![
            MonsterMove::attack("THRASH_MOVE", 3, 1),
            MonsterMove::buff("BUFF_MOVE", "StrengthPower", 15),
            MonsterMove::attack("THRASH2_MOVE", 3, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "THRASH_MOVE",
            then: Box::new(MovePattern::Conditional {
                predicate: AiCondition::LastMoveWas("THRASH_MOVE"),
                then_branch: Box::new(MovePattern::Cycle { moves: vec!["BUFF_MOVE"] }),
                else_branch: Box::new(MovePattern::Cycle { moves: vec!["THRASH2_MOVE"] }),
            }),
        },
    });

    // BygoneEffigy: InitialSleep → Wake(+10 Str) → Slashes(13) → Slashes(loop).
    m.insert("BygoneEffigy", MonsterAi {
        model_id: "BygoneEffigy",
        moves: vec![
            MonsterMove::sleep("INITIAL_SLEEP_MOVE"),
            MonsterMove::buff("WAKE_MOVE", "StrengthPower", 10),
            MonsterMove::attack("SLASHES_MOVE", 13, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "INITIAL_SLEEP_MOVE",
            then: Box::new(MovePattern::Conditional {
                predicate: AiCondition::LastMoveWas("INITIAL_SLEEP_MOVE"),
                then_branch: Box::new(MovePattern::Cycle { moves: vec!["WAKE_MOVE"] }),
                else_branch: Box::new(MovePattern::Cycle { moves: vec!["SLASHES_MOVE"] }),
            }),
        },
    });

    // Byrdonis: SWOOP(8) ↔ PECK(8×2). Strict alternation.
    m.insert("Byrdonis", MonsterAi {
        model_id: "Byrdonis",
        moves: vec![
            MonsterMove::attack("SWOOP_MOVE", 8, 1),
            MonsterMove::attack("PECK_MOVE", 8, 2),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle { moves: vec!["SWOOP_MOVE", "PECK_MOVE"] },
    });

    // Chomper: CLAMP(8×2) ↔ SCREECH (3 Dazed). Spawn: Artifact(2).
    m.insert("Chomper", MonsterAi {
        model_id: "Chomper",
        moves: vec![
            MonsterMove::attack("CLAMP_MOVE", 8, 2),
            MonsterMove {
                id: "SCREECH_MOVE",
                kind: IntentKind::Debuff,
                body: (0..3).map(|_| Effect::AddCardToPile {
                    card_id: "Dazed".to_string(),
                    upgrade: 0,
                    pile: crate::effects::Pile::Discard,
                }).collect(),
            },
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "ArtifactPower".to_string(),
            amount: AmountSpec::Fixed(2),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Cycle { moves: vec!["CLAMP_MOVE", "SCREECH_MOVE"] },
    });

    // CorpseSlug: WHIP_SLAP(6×2) → GLOMP(13) → GOOP(+Frail 2).
    m.insert("CorpseSlug", MonsterAi {
        model_id: "CorpseSlug",
        moves: vec![
            MonsterMove::attack("WHIP_SLAP_MOVE", 6, 2),
            MonsterMove::attack("GLOMP_MOVE", 13, 1),
            MonsterMove::debuff("GOOP_MOVE", "FrailPower", 2),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec!["WHIP_SLAP_MOVE", "GLOMP_MOVE", "GOOP_MOVE"],
        },
    });

    // Crusher: 5-cycle. Spawn: BackAttackLeft + CrabRage.
    m.insert("Crusher", MonsterAi {
        model_id: "Crusher",
        moves: vec![
            MonsterMove::attack("THRASH_MOVE", 12, 1),
            MonsterMove::attack("ENLARGING_STRIKE_MOVE", 4, 1),
            MonsterMove {
                id: "BUG_STING_MOVE",
                kind: IntentKind::AttackDebuff { hits: 2 },
                body: vec![
                    Effect::DealDamage {
                        amount: AmountSpec::Fixed(6),
                        target: Target::ChosenEnemy,
                        hits: 2,
                    },
                    Effect::ApplyPower {
                        power_id: "WeakPower".to_string(),
                        amount: AmountSpec::Fixed(2),
                        target: Target::ChosenEnemy,
                    },
                    Effect::ApplyPower {
                        power_id: "FrailPower".to_string(),
                        amount: AmountSpec::Fixed(2),
                        target: Target::ChosenEnemy,
                    },
                ],
            },
            MonsterMove::buff("ADAPT_MOVE", "StrengthPower", 2),
            MonsterMove::attack_defend("GUARDED_STRIKE_MOVE", 12, 1, 18),
        ],
        spawn: vec![
            Effect::ApplyPower {
                power_id: "BackAttackLeftPower".to_string(),
                amount: AmountSpec::Fixed(1),
                target: Target::SelfActor,
            },
            Effect::ApplyPower {
                power_id: "CrabRagePower".to_string(),
                amount: AmountSpec::Fixed(1),
                target: Target::SelfActor,
            },
        ],
        pattern: MovePattern::Cycle {
            moves: vec![
                "THRASH_MOVE",
                "ENLARGING_STRIKE_MOVE",
                "BUG_STING_MOVE",
                "ADAPT_MOVE",
                "GUARDED_STRIKE_MOVE",
            ],
        },
    });

    // Entomancer: BEES(3×7) → SPEAR(18) → SPIT(+1 PersonalHive).
    // Spawn: PersonalHive(1).
    m.insert("Entomancer", MonsterAi {
        model_id: "Entomancer",
        moves: vec![
            MonsterMove::attack("BEES_MOVE", 3, 7),
            MonsterMove::attack("SPEAR_MOVE", 18, 1),
            MonsterMove::buff("PHEROMONE_SPIT_MOVE", "PersonalHivePower", 1),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "PersonalHivePower".to_string(),
            amount: AmountSpec::Fixed(1),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Cycle {
            moves: vec!["BEES_MOVE", "SPEAR_MOVE", "PHEROMONE_SPIT_MOVE"],
        },
    });

    // FuzzyWurmCrawler: FirstAcidGoop(4) → Inhale(+Str 7) → AcidGoop(4) loop.
    m.insert("FuzzyWurmCrawler", MonsterAi {
        model_id: "FuzzyWurmCrawler",
        moves: vec![
            MonsterMove::attack("FIRST_ACID_GOOP", 4, 1),
            MonsterMove::buff("INHALE", "StrengthPower", 7),
            MonsterMove::attack("ACID_GOOP", 4, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "FIRST_ACID_GOOP",
            then: Box::new(MovePattern::Conditional {
                predicate: AiCondition::LastMoveWas("FIRST_ACID_GOOP"),
                then_branch: Box::new(MovePattern::Cycle { moves: vec!["INHALE"] }),
                else_branch: Box::new(MovePattern::Cycle { moves: vec!["ACID_GOOP"] }),
            }),
        },
    });

    // HauntedShip: HAUNT(4 Dazed) first, then weighted Ramming/Swipe/Stomp.
    m.insert("HauntedShip", MonsterAi {
        model_id: "HauntedShip",
        moves: vec![
            MonsterMove {
                id: "HAUNT_MOVE",
                kind: IntentKind::Debuff,
                body: (0..4).map(|_| Effect::AddCardToPile {
                    card_id: "Dazed".to_string(),
                    upgrade: 0,
                    pile: crate::effects::Pile::Discard,
                }).collect(),
            },
            MonsterMove::attack_debuff("RAMMING_SPEED_MOVE", 10, 1, "WeakPower", 1),
            MonsterMove::attack("SWIPE_MOVE", 11, 1),
            MonsterMove::attack("STOMP_MOVE", 4, 2),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "HAUNT_MOVE",
            then: Box::new(MovePattern::WeightedRandom {
                weights: vec![
                    ("RAMMING_SPEED_MOVE", 1),
                    ("SWIPE_MOVE", 1),
                    ("STOMP_MOVE", 1),
                ],
                no_repeat: vec!["RAMMING_SPEED_MOVE", "SWIPE_MOVE", "STOMP_MOVE"],
            }),
        },
    });

    // LeafSlimeS: BUTT(3) vs GOOP(+1 Slimed). Weighted no_repeat.
    m.insert("LeafSlimeS", MonsterAi {
        model_id: "LeafSlimeS",
        moves: vec![
            MonsterMove::attack("BUTT_MOVE", 3, 1),
            MonsterMove {
                id: "GOOP_MOVE",
                kind: IntentKind::Debuff,
                body: vec![Effect::AddCardToPile {
                    card_id: "Slimed".to_string(),
                    upgrade: 0,
                    pile: crate::effects::Pile::Discard,
                }],
            },
        ],
        spawn: vec![],
        pattern: MovePattern::WeightedRandom {
            weights: vec![("BUTT_MOVE", 1), ("GOOP_MOVE", 1)],
            no_repeat: vec!["BUTT_MOVE", "GOOP_MOVE"],
        },
    });

    // LouseProgenitor: CurlAndGrow(14 block + 5 Str) → Pounce(14) → Web(9 + 2 Frail).
    // Spawn: CurlUp(14).
    m.insert("LouseProgenitor", MonsterAi {
        model_id: "LouseProgenitor",
        moves: vec![
            MonsterMove {
                id: "CURL_AND_GROW_MOVE",
                kind: IntentKind::Defend,
                body: vec![
                    Effect::GainBlock {
                        amount: AmountSpec::Fixed(14),
                        target: Target::SelfActor,
                    },
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(5),
                        target: Target::SelfActor,
                    },
                ],
            },
            MonsterMove::attack("POUNCE_MOVE", 14, 1),
            MonsterMove::attack_debuff("WEB_CANNON_MOVE", 9, 1, "FrailPower", 2),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "CurlUpPower".to_string(),
            amount: AmountSpec::Fixed(14),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Cycle {
            moves: vec!["CURL_AND_GROW_MOVE", "POUNCE_MOVE", "WEB_CANNON_MOVE"],
        },
    });

    // MagiKnight: 6-cycle ending in Spear loop.
    // PowerShield(4 + 2 block) → Dampen(1 Dampen) → Spear(12) → Prep(11 block)
    //   → MagicBomb(20) → Spear(loop)
    m.insert("MagiKnight", MonsterAi {
        model_id: "MagiKnight",
        moves: vec![
            MonsterMove::attack_defend("POWER_SHIELD_MOVE", 4, 1, 2),
            MonsterMove::buff("DAMPEN_MOVE", "DampenPower", 1),
            MonsterMove::attack("SPEAR_MOVE", 12, 1),
            MonsterMove::defend("PREP_MOVE", 11),
            MonsterMove::attack("MAGIC_BOMB_MOVE", 20, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "POWER_SHIELD_MOVE",
            then: Box::new(MovePattern::Conditional {
                predicate: AiCondition::LastMoveWas("POWER_SHIELD_MOVE"),
                then_branch: Box::new(MovePattern::Cycle { moves: vec!["DAMPEN_MOVE"] }),
                else_branch: Box::new(MovePattern::Conditional {
                    predicate: AiCondition::LastMoveWas("DAMPEN_MOVE"),
                    then_branch: Box::new(MovePattern::Cycle { moves: vec!["SPEAR_MOVE"] }),
                    else_branch: Box::new(MovePattern::Conditional {
                        predicate: AiCondition::LastMoveWas("SPEAR_MOVE"),
                        then_branch: Box::new(MovePattern::Cycle { moves: vec!["PREP_MOVE"] }),
                        else_branch: Box::new(MovePattern::Conditional {
                            predicate: AiCondition::LastMoveWas("PREP_MOVE"),
                            then_branch: Box::new(MovePattern::Cycle {
                                moves: vec!["MAGIC_BOMB_MOVE"],
                            }),
                            // After MAGIC_BOMB → Spear loop.
                            else_branch: Box::new(MovePattern::Cycle {
                                moves: vec!["SPEAR_MOVE"],
                            }),
                        }),
                    }),
                }),
            }),
        },
    });

    // MechaKnight: Charge(25) → Flamethrower(4 Burn to hand) → Windup(15 block + 5 Str)
    //   → HeavyCleave(35) → Flamethrower(loop). Spawn: Artifact(3).
    m.insert("MechaKnight", MonsterAi {
        model_id: "MechaKnight",
        moves: vec![
            MonsterMove::attack("CHARGE_MOVE", 25, 1),
            MonsterMove {
                id: "FLAMETHROWER_MOVE",
                kind: IntentKind::Debuff,
                body: (0..4).map(|_| Effect::AddCardToPile {
                    card_id: "Burn".to_string(),
                    upgrade: 0,
                    pile: crate::effects::Pile::Hand,
                }).collect(),
            },
            MonsterMove {
                id: "WINDUP_MOVE",
                kind: IntentKind::Defend,
                body: vec![
                    Effect::GainBlock {
                        amount: AmountSpec::Fixed(15),
                        target: Target::SelfActor,
                    },
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(5),
                        target: Target::SelfActor,
                    },
                ],
            },
            MonsterMove::attack("HEAVY_CLEAVE_MOVE", 35, 1),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "ArtifactPower".to_string(),
            amount: AmountSpec::Fixed(3),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "CHARGE_MOVE",
            then: Box::new(MovePattern::Conditional {
                predicate: AiCondition::LastMoveWas("CHARGE_MOVE"),
                then_branch: Box::new(MovePattern::Cycle {
                    moves: vec!["FLAMETHROWER_MOVE"],
                }),
                else_branch: Box::new(MovePattern::Conditional {
                    predicate: AiCondition::LastMoveWas("FLAMETHROWER_MOVE"),
                    then_branch: Box::new(MovePattern::Cycle { moves: vec!["WINDUP_MOVE"] }),
                    else_branch: Box::new(MovePattern::Conditional {
                        predicate: AiCondition::LastMoveWas("WINDUP_MOVE"),
                        then_branch: Box::new(MovePattern::Cycle {
                            moves: vec!["HEAVY_CLEAVE_MOVE"],
                        }),
                        // After HEAVY_CLEAVE → Flamethrower loop.
                        else_branch: Box::new(MovePattern::Cycle {
                            moves: vec!["FLAMETHROWER_MOVE"],
                        }),
                    }),
                }),
            }),
        },
    });

    // ShrinkerBeetle: Shrinker (apply -1 ShrinkPower) → Chomp(7) ↔ Stomp(13).
    m.insert("ShrinkerBeetle", MonsterAi {
        model_id: "ShrinkerBeetle",
        moves: vec![
            MonsterMove::debuff("SHRINKER_MOVE", "ShrinkPower", 1),
            MonsterMove::attack("CHOMP_MOVE", 7, 1),
            MonsterMove::attack("STOMP_MOVE", 13, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "SHRINKER_MOVE",
            then: Box::new(MovePattern::Cycle {
                moves: vec!["CHOMP_MOVE", "STOMP_MOVE"],
            }),
        },
    });

    // SkulkingColony: Smash(8) → Zoom(6+6 block) → Inertia(10+2 Str) → Stabs(4×3).
    // Spawn: HardenedShellPower(15).
    m.insert("SkulkingColony", MonsterAi {
        model_id: "SkulkingColony",
        moves: vec![
            MonsterMove::attack("SMASH_MOVE", 8, 1),
            MonsterMove::attack_defend("ZOOM_MOVE", 6, 1, 6),
            MonsterMove::attack_buff("INERTIA_MOVE", 10, 1, "StrengthPower", 2),
            MonsterMove::attack("PIERCING_STABS_MOVE", 4, 3),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "HardenedShellPower".to_string(),
            amount: AmountSpec::Fixed(15),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Cycle {
            moves: vec![
                "SMASH_MOVE",
                "ZOOM_MOVE",
                "INERTIA_MOVE",
                "PIERCING_STABS_MOVE",
            ],
        },
    });

    // SludgeSpinner: weighted {OilSpray(7+Weak 1), Slam(12), Rage(10+Str 2)}, no_repeat.
    m.insert("SludgeSpinner", MonsterAi {
        model_id: "SludgeSpinner",
        moves: vec![
            MonsterMove::attack_debuff("OIL_SPRAY_MOVE", 7, 1, "WeakPower", 1),
            MonsterMove::attack("SLAM_MOVE", 12, 1),
            MonsterMove::attack_buff("RAGE_MOVE", 10, 1, "StrengthPower", 2),
        ],
        spawn: vec![],
        pattern: MovePattern::WeightedRandom {
            weights: vec![("OIL_SPRAY_MOVE", 1), ("SLAM_MOVE", 1), ("RAGE_MOVE", 1)],
            no_repeat: vec!["OIL_SPRAY_MOVE", "SLAM_MOVE", "RAGE_MOVE"],
        },
    });

    // SpectralKnight: Hex(2 HexPower) → SoulSlash(15), then weighted
    //   {SoulSlash:2, SoulFlame(3×3):1 no_repeat}.
    m.insert("SpectralKnight", MonsterAi {
        model_id: "SpectralKnight",
        moves: vec![
            MonsterMove::debuff("HEX_MOVE", "HexPower", 2),
            MonsterMove::attack("SOUL_SLASH_MOVE", 15, 1),
            MonsterMove::attack("SOUL_FLAME_MOVE", 3, 3),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "HEX_MOVE",
            then: Box::new(MovePattern::Conditional {
                predicate: AiCondition::LastMoveWas("HEX_MOVE"),
                then_branch: Box::new(MovePattern::Cycle {
                    moves: vec!["SOUL_SLASH_MOVE"],
                }),
                else_branch: Box::new(MovePattern::WeightedRandom {
                    weights: vec![("SOUL_SLASH_MOVE", 2), ("SOUL_FLAME_MOVE", 1)],
                    no_repeat: vec!["SOUL_FLAME_MOVE"],
                }),
            }),
        },
    });

    // Rocket: 5-cycle. Spawn: BackAttackRight + CrabRage + Surrounded.
    // Surrounded targets player, can't express here cleanly (target
    // is ChosenEnemy from monster context). For now apply to SelfActor
    // as marker; the legacy spawn code already handles the player
    // application separately.
    m.insert("Rocket", MonsterAi {
        model_id: "Rocket",
        moves: vec![
            MonsterMove::attack("TARGETING_RETICLE_MOVE", 3, 1),
            MonsterMove::attack("PRECISION_BEAM_MOVE", 18, 1),
            MonsterMove::buff("CHARGE_UP_MOVE", "StrengthPower", 2),
            MonsterMove::attack("LASER_MOVE", 31, 1),
            MonsterMove::sleep("RECHARGE_MOVE"),
        ],
        spawn: vec![
            Effect::ApplyPower {
                power_id: "BackAttackRightPower".to_string(),
                amount: AmountSpec::Fixed(1),
                target: Target::SelfActor,
            },
            Effect::ApplyPower {
                power_id: "CrabRagePower".to_string(),
                amount: AmountSpec::Fixed(1),
                target: Target::SelfActor,
            },
            Effect::ApplyPower {
                power_id: "SurroundedPower".to_string(),
                amount: AmountSpec::Fixed(1),
                target: Target::ChosenEnemy,
            },
        ],
        pattern: MovePattern::Cycle {
            moves: vec![
                "TARGETING_RETICLE_MOVE",
                "PRECISION_BEAM_MOVE",
                "CHARGE_UP_MOVE",
                "LASER_MOVE",
                "RECHARGE_MOVE",
            ],
        },
    });

    // TwoTailedRat: Scratch(8), DiseaseBite(6), Screech(+Weak 1).
    // C# pattern: pick uniformly from 2 non-last. Approximate as
    // WeightedRandom with no_repeat on all.
    m.insert("TwoTailedRat", MonsterAi {
        model_id: "TwoTailedRat",
        moves: vec![
            MonsterMove::attack("SCRATCH_MOVE", 8, 1),
            MonsterMove::attack("DISEASE_BITE_MOVE", 6, 1),
            MonsterMove::debuff("SCREECH_MOVE", "WeakPower", 1),
        ],
        spawn: vec![],
        pattern: MovePattern::WeightedRandom {
            weights: vec![
                ("SCRATCH_MOVE", 1),
                ("DISEASE_BITE_MOVE", 1),
                ("SCREECH_MOVE", 1),
            ],
            no_repeat: vec!["SCRATCH_MOVE", "DISEASE_BITE_MOVE", "SCREECH_MOVE"],
        },
    });
}

/// Third batch — flag-state and slot-conditional monsters that need
/// the full primitive vocabulary. A0 values from each monster's
/// combat.rs port.
#[allow(clippy::too_many_lines)]
fn register_migrated_legacy_b3(m: &mut HashMap<&'static str, MonsterAi>) {
    use crate::effects::AmountSpec;

    // TwigSlimeM: cycle StickyShot (+Slimed) → ClumpShot(11) → loop.
    // C# weights this 2:1 the first time but Cycle approximation is
    // close enough for non-RNG semantics.
    m.insert("TwigSlimeM", MonsterAi {
        model_id: "TwigSlimeM",
        moves: vec![
            MonsterMove {
                id: "STICKY_SHOT_MOVE",
                kind: IntentKind::Debuff,
                body: vec![Effect::AddCardToPile {
                    card_id: "Slimed".to_string(),
                    upgrade: 0,
                    pile: crate::effects::Pile::Discard,
                }],
            },
            MonsterMove::attack("CLUMP_SHOT_MOVE", 11, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec!["STICKY_SHOT_MOVE", "CLUMP_SHOT_MOVE"],
        },
    });

    // MysteriousKnight: FlailKnight state machine + +6 Strength + +6 Plating spawn.
    // Reuses FlailKnight's move ids.
    m.insert("MysteriousKnight", MonsterAi {
        model_id: "MysteriousKnight",
        moves: vec![
            MonsterMove::buff("WAR_CHANT", "StrengthPower", 3),
            MonsterMove::attack("FLAIL_MOVE", 9, 2),
            MonsterMove::attack("RAM_MOVE", 15, 1),
        ],
        spawn: vec![
            Effect::ApplyPower {
                power_id: "StrengthPower".to_string(),
                amount: AmountSpec::Fixed(6),
                target: Target::SelfActor,
            },
            Effect::ApplyPower {
                power_id: "PlatingPower".to_string(),
                amount: AmountSpec::Fixed(6),
                target: Target::SelfActor,
            },
        ],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "RAM_MOVE",
            then: Box::new(MovePattern::WeightedRandom {
                weights: vec![("WAR_CHANT", 1), ("FLAIL_MOVE", 2), ("RAM_MOVE", 2)],
                no_repeat: vec!["WAR_CHANT"],
            }),
        },
    });

    // Tunneler: Bite(13) → Burrow(12 block + Burrowed) → Below(23) → Below(loop).
    m.insert("Tunneler", MonsterAi {
        model_id: "Tunneler",
        moves: vec![
            MonsterMove::attack("BITE_MOVE", 13, 1),
            MonsterMove {
                id: "BURROW_MOVE",
                kind: IntentKind::Defend,
                body: vec![
                    Effect::GainBlock {
                        amount: AmountSpec::Fixed(12),
                        target: Target::SelfActor,
                    },
                    Effect::ApplyPower {
                        power_id: "BurrowedPower".to_string(),
                        amount: AmountSpec::Fixed(1),
                        target: Target::SelfActor,
                    },
                ],
            },
            MonsterMove::attack("BELOW_MOVE", 23, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "BITE_MOVE",
            then: Box::new(MovePattern::Conditional {
                predicate: AiCondition::LastMoveWas("BITE_MOVE"),
                then_branch: Box::new(MovePattern::Cycle { moves: vec!["BURROW_MOVE"] }),
                else_branch: Box::new(MovePattern::Cycle { moves: vec!["BELOW_MOVE"] }),
            }),
        },
    });

    // TorchHeadAmalgam: 6-cycle Tackle1 → Tackle2 → Beam → Tackle3 → Tackle4 → Beam.
    m.insert("TorchHeadAmalgam", MonsterAi {
        model_id: "TorchHeadAmalgam",
        moves: vec![
            MonsterMove::attack("TACKLE_1_MOVE", 18, 1),
            MonsterMove::attack("TACKLE_2_MOVE", 18, 1),
            MonsterMove::attack("BEAM_MOVE", 8, 3),
            MonsterMove::attack("TACKLE_3_MOVE", 14, 1),
            MonsterMove::attack("TACKLE_4_MOVE", 14, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "TACKLE_1_MOVE",
                "TACKLE_2_MOVE",
                "BEAM_MOVE",
                "TACKLE_3_MOVE",
                "TACKLE_4_MOVE",
                "BEAM_MOVE",
            ],
        },
    });

    // LivingFog: AdvancedGas(8 + 1 Smoggy) → Bloat(5 + summon stub)
    //   → SuperGas(8) → Bloat → loop. Bloat's summon body is no-op
    //   for now (needs encounter-specific summon target).
    m.insert("LivingFog", MonsterAi {
        model_id: "LivingFog",
        moves: vec![
            MonsterMove::attack_debuff("ADVANCED_GAS_MOVE", 8, 1, "SmoggyPower", 1),
            MonsterMove::attack("BLOAT_MOVE", 5, 1),
            MonsterMove::attack("SUPER_GAS_MOVE", 8, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "ADVANCED_GAS_MOVE",
                "BLOAT_MOVE",
                "SUPER_GAS_MOVE",
                "BLOAT_MOVE",
            ],
        },
    });

    // Doormaker: DramaticOpen → Hunger(30) → Scrutiny(24) → Grasp(10×2 + 3 Str)
    //   → Hunger(loop). Spawn: HungerPower.
    m.insert("Doormaker", MonsterAi {
        model_id: "Doormaker",
        moves: vec![
            MonsterMove::sleep("DRAMATIC_OPEN_MOVE"),
            MonsterMove::attack("HUNGER_MOVE", 30, 1),
            MonsterMove::attack("SCRUTINY_MOVE", 24, 1),
            MonsterMove::attack_buff("GRASP_MOVE", 10, 2, "StrengthPower", 3),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "HungerPower".to_string(),
            amount: AmountSpec::Fixed(1),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "DRAMATIC_OPEN_MOVE",
            then: Box::new(MovePattern::Conditional {
                predicate: AiCondition::LastMoveWas("DRAMATIC_OPEN_MOVE"),
                then_branch: Box::new(MovePattern::Cycle {
                    moves: vec!["HUNGER_MOVE"],
                }),
                else_branch: Box::new(MovePattern::Conditional {
                    predicate: AiCondition::LastMoveWas("HUNGER_MOVE"),
                    then_branch: Box::new(MovePattern::Cycle {
                        moves: vec!["SCRUTINY_MOVE"],
                    }),
                    else_branch: Box::new(MovePattern::Conditional {
                        predicate: AiCondition::LastMoveWas("SCRUTINY_MOVE"),
                        then_branch: Box::new(MovePattern::Cycle {
                            moves: vec!["GRASP_MOVE"],
                        }),
                        // After Grasp → Hunger loop.
                        else_branch: Box::new(MovePattern::Cycle {
                            moves: vec!["HUNGER_MOVE"],
                        }),
                    }),
                }),
            }),
        },
    });

    // InfestedPrism: Jab(22) → Radiate(16+16 block) → Whirlwind(9×3)
    //   → Pulsate(20 block + 4 Str) → Jab(loop). Spawn: VitalSpark(1).
    m.insert("InfestedPrism", MonsterAi {
        model_id: "InfestedPrism",
        moves: vec![
            MonsterMove::attack("JAB_MOVE", 22, 1),
            MonsterMove::attack_defend("RADIATE_MOVE", 16, 1, 16),
            MonsterMove::attack("WHIRLWIND_MOVE", 9, 3),
            MonsterMove {
                id: "PULSATE_MOVE",
                kind: IntentKind::Defend,
                body: vec![
                    Effect::GainBlock {
                        amount: AmountSpec::Fixed(20),
                        target: Target::SelfActor,
                    },
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(4),
                        target: Target::SelfActor,
                    },
                ],
            },
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "VitalSparkPower".to_string(),
            amount: AmountSpec::Fixed(1),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Cycle {
            moves: vec![
                "JAB_MOVE",
                "RADIATE_MOVE",
                "WHIRLWIND_MOVE",
                "PULSATE_MOVE",
            ],
        },
    });

    // SlumberingBeetle: HasFlag(slumber) → Snore; else Rollout. Spawn:
    // Plating(15) + Slumber(3).
    m.insert("SlumberingBeetle", MonsterAi {
        model_id: "SlumberingBeetle",
        moves: vec![
            MonsterMove::sleep("SNORE_MOVE"),
            MonsterMove::attack_buff("ROLL_OUT_MOVE", 16, 1, "StrengthPower", 2),
        ],
        spawn: vec![
            Effect::ApplyPower {
                power_id: "PlatingPower".to_string(),
                amount: AmountSpec::Fixed(15),
                target: Target::SelfActor,
            },
            Effect::ApplyPower {
                power_id: "SlumberPower".to_string(),
                amount: AmountSpec::Fixed(3),
                target: Target::SelfActor,
            },
        ],
        pattern: MovePattern::Conditional {
            // SlumberPower is the slumber flag — encoded via HasFlag
            // semantics from the power stack. When slumber is broken
            // (HP threshold via external hook), the flag clears and
            // we fall to Rollout.
            predicate: AiCondition::HasFlag("slumber_active"),
            then_branch: Box::new(MovePattern::Cycle {
                moves: vec!["SNORE_MOVE"],
            }),
            else_branch: Box::new(MovePattern::Cycle {
                moves: vec!["ROLL_OUT_MOVE"],
            }),
        },
    });

    // TerrorEel: Conditional(HasFlag(shriek_triggered) → Terror, else Crash ↔ Thrash).
    // Spawn: ShriekPower(70 — HP threshold).
    m.insert("TerrorEel", MonsterAi {
        model_id: "TerrorEel",
        moves: vec![
            MonsterMove::attack("CRASH_MOVE", 16, 1),
            MonsterMove::attack_buff("THRASH_MOVE", 3, 3, "VigorPower", 6),
            MonsterMove::debuff("TERROR_MOVE", "VulnerablePower", 99),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "ShriekPower".to_string(),
            amount: AmountSpec::Fixed(70),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Conditional {
            predicate: AiCondition::HasFlag("shriek_triggered"),
            then_branch: Box::new(MovePattern::Cycle {
                moves: vec!["TERROR_MOVE"],
            }),
            else_branch: Box::new(MovePattern::Cycle {
                moves: vec!["CRASH_MOVE", "THRASH_MOVE"],
            }),
        },
    });

    // BowlbugRock: Conditional(HasFlag(is_off_balance) → Dizzy, else Headbutt).
    // Spawn: ImbalancedPower(1) — separate hook system maintains the
    // is_off_balance flag.
    m.insert("BowlbugRock", MonsterAi {
        model_id: "BowlbugRock",
        moves: vec![
            MonsterMove::attack("HEADBUTT_MOVE", 15, 1),
            MonsterMove::sleep("DIZZY_MOVE"),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "ImbalancedPower".to_string(),
            amount: AmountSpec::Fixed(1),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Conditional {
            predicate: AiCondition::HasFlag("is_off_balance"),
            then_branch: Box::new(MovePattern::Cycle {
                moves: vec!["DIZZY_MOVE"],
            }),
            else_branch: Box::new(MovePattern::Cycle {
                moves: vec!["HEADBUTT_MOVE"],
            }),
        },
    });

    // LivingShield: if alone → Smash(16 + 3 Str); else ShieldSlam(6).
    // Spawn: Rampart(25).
    m.insert("LivingShield", MonsterAi {
        model_id: "LivingShield",
        moves: vec![
            MonsterMove::attack("SHIELD_SLAM_MOVE", 6, 1),
            MonsterMove::attack_buff("SMASH_MOVE", 16, 1, "StrengthPower", 3),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "RampartPower".to_string(),
            amount: AmountSpec::Fixed(25),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Conditional {
            predicate: AiCondition::LivingEnemyCountEquals(1),
            then_branch: Box::new(MovePattern::Cycle {
                moves: vec!["SMASH_MOVE"],
            }),
            else_branch: Box::new(MovePattern::Cycle {
                moves: vec!["SHIELD_SLAM_MOVE"],
            }),
        },
    });

    // Exoskeleton: BySlot first-turn else weighted Skitter ↔ Mandibles.
    // Slot 1: Skitter. Slot 2: Mandibles. Slot 3+: Enrage. Then random.
    // Spawn: HardToKill(9).
    m.insert("Exoskeleton", MonsterAi {
        model_id: "Exoskeleton",
        moves: vec![
            MonsterMove::attack("SKITTER_MOVE", 1, 3),
            MonsterMove::attack("MANDIBLE_MOVE", 8, 1),
            MonsterMove::buff("ENRAGE_MOVE", "StrengthPower", 2),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "HardToKillPower".to_string(),
            amount: AmountSpec::Fixed(9),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::BySlot {
            branches: vec![
                ("first", MovePattern::FirstTurnOverride {
                    first_move: "SKITTER_MOVE",
                    then: Box::new(MovePattern::WeightedRandom {
                        weights: vec![("SKITTER_MOVE", 1), ("MANDIBLE_MOVE", 1)],
                        no_repeat: vec!["SKITTER_MOVE", "MANDIBLE_MOVE"],
                    }),
                }),
                ("second", MovePattern::FirstTurnOverride {
                    first_move: "MANDIBLE_MOVE",
                    then: Box::new(MovePattern::WeightedRandom {
                        weights: vec![("SKITTER_MOVE", 1), ("MANDIBLE_MOVE", 1)],
                        no_repeat: vec!["SKITTER_MOVE", "MANDIBLE_MOVE"],
                    }),
                }),
            ],
            default: Box::new(MovePattern::FirstTurnOverride {
                first_move: "ENRAGE_MOVE",
                then: Box::new(MovePattern::WeightedRandom {
                    weights: vec![("SKITTER_MOVE", 1), ("MANDIBLE_MOVE", 1)],
                    no_repeat: vec!["SKITTER_MOVE", "MANDIBLE_MOVE"],
                }),
            }),
        },
    });

    // PhantasmalGardener: BySlot first-turn else cycle Bite → Lash → Flail → Enlarge.
    // Slot 1: Flail. Slot 2: Bite. Slot 3: Lash. Slot 4: Enlarge.
    // Spawn: Skittish(6).
    m.insert("PhantasmalGardener", MonsterAi {
        model_id: "PhantasmalGardener",
        moves: vec![
            MonsterMove::attack("BITE_MOVE", 5, 1),
            MonsterMove::attack("LASH_MOVE", 7, 1),
            MonsterMove::attack("FLAIL_MOVE", 1, 3),
            MonsterMove::buff("ENLARGE_MOVE", "StrengthPower", 2),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "SkittishPower".to_string(),
            amount: AmountSpec::Fixed(6),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::BySlot {
            branches: vec![
                ("first", MovePattern::FirstTurnOverride {
                    first_move: "FLAIL_MOVE",
                    then: Box::new(MovePattern::Cycle {
                        moves: vec!["BITE_MOVE", "LASH_MOVE", "FLAIL_MOVE", "ENLARGE_MOVE"],
                    }),
                }),
                ("second", MovePattern::FirstTurnOverride {
                    first_move: "BITE_MOVE",
                    then: Box::new(MovePattern::Cycle {
                        moves: vec!["BITE_MOVE", "LASH_MOVE", "FLAIL_MOVE", "ENLARGE_MOVE"],
                    }),
                }),
                ("third", MovePattern::FirstTurnOverride {
                    first_move: "LASH_MOVE",
                    then: Box::new(MovePattern::Cycle {
                        moves: vec!["BITE_MOVE", "LASH_MOVE", "FLAIL_MOVE", "ENLARGE_MOVE"],
                    }),
                }),
                ("fourth", MovePattern::FirstTurnOverride {
                    first_move: "ENLARGE_MOVE",
                    then: Box::new(MovePattern::Cycle {
                        moves: vec!["BITE_MOVE", "LASH_MOVE", "FLAIL_MOVE", "ENLARGE_MOVE"],
                    }),
                }),
            ],
            default: Box::new(MovePattern::Cycle {
                moves: vec!["BITE_MOVE", "LASH_MOVE", "FLAIL_MOVE", "ENLARGE_MOVE"],
            }),
        },
    });

    // ScrollOfBiting: Chomp(14) → MoreTeeth(+2 Str) → Chew(5×2) →
    //   WeightedRandom{Chomp no_repeat, Chew:2}.
    m.insert("ScrollOfBiting", MonsterAi {
        model_id: "ScrollOfBiting",
        moves: vec![
            MonsterMove::attack("CHOMP_MOVE", 14, 1),
            MonsterMove::attack("CHEW_MOVE", 5, 2),
            MonsterMove::buff("MORE_TEETH_MOVE", "StrengthPower", 2),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "CHOMP_MOVE",
            then: Box::new(MovePattern::Conditional {
                predicate: AiCondition::LastMoveWas("CHOMP_MOVE"),
                then_branch: Box::new(MovePattern::Cycle {
                    moves: vec!["MORE_TEETH_MOVE"],
                }),
                else_branch: Box::new(MovePattern::Conditional {
                    predicate: AiCondition::LastMoveWas("MORE_TEETH_MOVE"),
                    then_branch: Box::new(MovePattern::Cycle {
                        moves: vec!["CHEW_MOVE"],
                    }),
                    else_branch: Box::new(MovePattern::WeightedRandom {
                        weights: vec![("CHOMP_MOVE", 1), ("CHEW_MOVE", 2)],
                        no_repeat: vec!["CHOMP_MOVE"],
                    }),
                }),
            }),
        },
    });

    // SlitheringStrangler: Constrict → weighted {Thwack(7+5 block), Lash(12)}.
    m.insert("SlitheringStrangler", MonsterAi {
        model_id: "SlitheringStrangler",
        moves: vec![
            MonsterMove::debuff("CONSTRICT_MOVE", "ConstrictPower", 3),
            MonsterMove::attack_defend("THWACK_MOVE", 7, 1, 5),
            MonsterMove::attack("LASH_MOVE", 12, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "CONSTRICT_MOVE",
            then: Box::new(MovePattern::WeightedRandom {
                weights: vec![("THWACK_MOVE", 1), ("LASH_MOVE", 1)],
                no_repeat: vec!["THWACK_MOVE", "LASH_MOVE"],
            }),
        },
    });

    // DecimillipedeSegmentFront/Middle/Back share the same state
    // machine: Constrict → Bulk → Writhe → loop.
    let decimillipede_moves = || vec![
        MonsterMove::debuff("CONSTRICT_MOVE", "ConstrictPower", 2),
        MonsterMove::buff("BULK_MOVE", "StrengthPower", 1),
        MonsterMove::attack("WRITHE_MOVE", 8, 1),
    ];
    let decimillipede_pattern = || MovePattern::Cycle {
        moves: vec!["CONSTRICT_MOVE", "BULK_MOVE", "WRITHE_MOVE"],
    };
    for id in ["DecimillipedeSegmentFront", "DecimillipedeSegmentMiddle", "DecimillipedeSegmentBack"] {
        m.insert(id, MonsterAi {
            model_id: id,
            moves: decimillipede_moves(),
            spawn: vec![],
            pattern: decimillipede_pattern(),
        });
    }
}

/// Fourth batch — bosses + remaining specials. Most boss state
/// machines are clean cycles; complex multi-phase bosses (Queen,
/// KnowledgeDemon, TheInsatiable, WaterfallGiant) get partial ports
/// here and full ports when their bespoke primitives land.
#[allow(clippy::too_many_lines)]
fn register_bosses_b4(m: &mut HashMap<&'static str, MonsterAi>) {
    use crate::effects::AmountSpec;

    // Architect: passive tutorial encounter, no actions.
    m.insert("Architect", MonsterAi {
        model_id: "Architect",
        moves: vec![MonsterMove::sleep("NOTHING_MOVE")],
        spawn: vec![],
        pattern: MovePattern::Cycle { moves: vec!["NOTHING_MOVE"] },
    });

    // PaelsLegion: passive encounter (9999 HP, no actions).
    m.insert("PaelsLegion", MonsterAi {
        model_id: "PaelsLegion",
        moves: vec![MonsterMove::sleep("NOTHING_MOVE")],
        spawn: vec![],
        pattern: MovePattern::Cycle { moves: vec!["NOTHING_MOVE"] },
    });

    // TheLost: cycle DebilitatingSmog (Str -2 / +2 self) ↔ EyeLasers (4×2).
    // Spawn: PossessStrength(1).
    m.insert("TheLost", MonsterAi {
        model_id: "TheLost",
        moves: vec![
            MonsterMove {
                id: "DEBILITATING_SMOG_MOVE",
                kind: IntentKind::Debuff,
                body: vec![
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(-2),
                        target: Target::ChosenEnemy,
                    },
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    },
                ],
            },
            MonsterMove::attack("EYE_LASERS_MOVE", 4, 2),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "PossessStrengthPower".to_string(),
            amount: AmountSpec::Fixed(1),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Cycle {
            moves: vec!["DEBILITATING_SMOG_MOVE", "EYE_LASERS_MOVE"],
        },
    });

    // TheForgotten: cycle Miasma (Dex -2 / +2 self + 8 block) ↔ Dread (13 dmg).
    // Spawn: PossessSpeed(1). C# scales dread damage with Dex; we use base 13.
    m.insert("TheForgotten", MonsterAi {
        model_id: "TheForgotten",
        moves: vec![
            MonsterMove {
                id: "MIASMA_MOVE",
                kind: IntentKind::Defend,
                body: vec![
                    Effect::ApplyPower {
                        power_id: "DexterityPower".to_string(),
                        amount: AmountSpec::Fixed(-2),
                        target: Target::ChosenEnemy,
                    },
                    Effect::ApplyPower {
                        power_id: "DexterityPower".to_string(),
                        amount: AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    },
                    Effect::GainBlock {
                        amount: AmountSpec::Fixed(8),
                        target: Target::SelfActor,
                    },
                ],
            },
            MonsterMove::attack("DREAD_MOVE", 13, 1),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "PossessSpeedPower".to_string(),
            amount: AmountSpec::Fixed(1),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Cycle {
            moves: vec!["MIASMA_MOVE", "DREAD_MOVE"],
        },
    });

    // TheAdversaryMkOne: cycle Smash(12) → Beam(15) → Barrage(8×2+Str2)
    //   → Smash(loop). Spawn: Artifact(0) — placeholder per C#.
    m.insert("TheAdversaryMkOne", MonsterAi {
        model_id: "TheAdversaryMkOne",
        moves: vec![
            MonsterMove::attack("SMASH_MOVE", 12, 1),
            MonsterMove::attack("BEAM_MOVE", 15, 1),
            MonsterMove::attack_buff("BARRAGE_MOVE", 8, 2, "StrengthPower", 2),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec!["SMASH_MOVE", "BEAM_MOVE", "BARRAGE_MOVE"],
        },
    });

    // TheAdversaryMkTwo: cycle Bash(13) → FlameBeam(16) → Barrage(9×2+Str3)
    //   → Bash(loop). Spawn: Artifact(1).
    m.insert("TheAdversaryMkTwo", MonsterAi {
        model_id: "TheAdversaryMkTwo",
        moves: vec![
            MonsterMove::attack("BASH_MOVE", 13, 1),
            MonsterMove::attack("FLAME_BEAM_MOVE", 16, 1),
            MonsterMove::attack_buff("BARRAGE_MOVE", 9, 2, "StrengthPower", 3),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "ArtifactPower".to_string(),
            amount: AmountSpec::Fixed(1),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Cycle {
            moves: vec!["BASH_MOVE", "FLAME_BEAM_MOVE", "BARRAGE_MOVE"],
        },
    });

    // TheAdversaryMkThree: cycle Crash(15) → FlameBeam(18) → Barrage(10×2+Str4)
    //   → Crash(loop). Spawn: Artifact(2).
    m.insert("TheAdversaryMkThree", MonsterAi {
        model_id: "TheAdversaryMkThree",
        moves: vec![
            MonsterMove::attack("CRASH_MOVE", 15, 1),
            MonsterMove::attack("FLAME_BEAM_MOVE", 18, 1),
            MonsterMove::attack_buff("BARRAGE_MOVE", 10, 2, "StrengthPower", 4),
        ],
        spawn: vec![Effect::ApplyPower {
            power_id: "ArtifactPower".to_string(),
            amount: AmountSpec::Fixed(2),
            target: Target::SelfActor,
        }],
        pattern: MovePattern::Cycle {
            moves: vec!["CRASH_MOVE", "FLAME_BEAM_MOVE", "BARRAGE_MOVE"],
        },
    });

    // CubexConstruct: ChargeUp(+Str 2) → Repeater(7+Str 2) → Repeater(7+Str 2)
    //   → ExpelBlast(6×2) → Repeater(7+Str 2) → Submerge(15 block) → loop.
    // Spawn: Block 13 + Artifact applied via combat init.
    m.insert("CubexConstruct", MonsterAi {
        model_id: "CubexConstruct",
        moves: vec![
            MonsterMove::buff("CHARGE_UP_MOVE", "StrengthPower", 2),
            MonsterMove::attack_buff("REPEATER_MOVE", 7, 1, "StrengthPower", 2),
            MonsterMove::attack("EXPEL_BLAST_MOVE", 6, 2),
            MonsterMove::defend("SUBMERGE_MOVE", 15),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "CHARGE_UP_MOVE",
                "REPEATER_MOVE",
                "REPEATER_MOVE",
                "EXPEL_BLAST_MOVE",
                "REPEATER_MOVE",
                "SUBMERGE_MOVE",
            ],
        },
    });

    // LagavulinMatriarch: HasFlag(asleep) → Sleep; else
    //   Slash(19) → Disembowel(9×2) → Slash2(12+14 block) → SoulSiphon
    //   → Slash(loop). Spawn: AsleepPower + PlatingPower.
    m.insert("LagavulinMatriarch", MonsterAi {
        model_id: "LagavulinMatriarch",
        moves: vec![
            MonsterMove::sleep("SLEEP_MOVE"),
            MonsterMove::attack("SLASH_MOVE", 19, 1),
            MonsterMove::attack("DISEMBOWEL_MOVE", 9, 2),
            MonsterMove::attack_defend("SLASH2_MOVE", 12, 1, 14),
            MonsterMove {
                id: "SOUL_SIPHON_MOVE",
                kind: IntentKind::Buff,
                body: vec![
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(-2),
                        target: Target::ChosenEnemy,
                    },
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    },
                    Effect::ApplyPower {
                        power_id: "DexterityPower".to_string(),
                        amount: AmountSpec::Fixed(-2),
                        target: Target::ChosenEnemy,
                    },
                    Effect::ApplyPower {
                        power_id: "DexterityPower".to_string(),
                        amount: AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    },
                ],
            },
        ],
        spawn: vec![
            Effect::ApplyPower {
                power_id: "AsleepPower".to_string(),
                amount: AmountSpec::Fixed(1),
                target: Target::SelfActor,
            },
            Effect::ApplyPower {
                power_id: "PlatingPower".to_string(),
                amount: AmountSpec::Fixed(8),
                target: Target::SelfActor,
            },
        ],
        pattern: MovePattern::Conditional {
            predicate: AiCondition::HasFlag("asleep"),
            then_branch: Box::new(MovePattern::Cycle { moves: vec!["SLEEP_MOVE"] }),
            else_branch: Box::new(MovePattern::FirstTurnOverride {
                first_move: "SLASH_MOVE",
                then: Box::new(MovePattern::Conditional {
                    predicate: AiCondition::LastMoveWas("SLASH_MOVE"),
                    then_branch: Box::new(MovePattern::Cycle {
                        moves: vec!["DISEMBOWEL_MOVE"],
                    }),
                    else_branch: Box::new(MovePattern::Conditional {
                        predicate: AiCondition::LastMoveWas("DISEMBOWEL_MOVE"),
                        then_branch: Box::new(MovePattern::Cycle {
                            moves: vec!["SLASH2_MOVE"],
                        }),
                        else_branch: Box::new(MovePattern::Conditional {
                            predicate: AiCondition::LastMoveWas("SLASH2_MOVE"),
                            then_branch: Box::new(MovePattern::Cycle {
                                moves: vec!["SOUL_SIPHON_MOVE"],
                            }),
                            // After SoulSiphon → Slash loop.
                            else_branch: Box::new(MovePattern::Cycle {
                                moves: vec!["SLASH_MOVE"],
                            }),
                        }),
                    }),
                }),
            }),
        },
    });

    // TheObscura: Illusion (summon Parafright) → weighted {Gaze(10),
    //   Sail(+3 Str), HardeningStrike(6+7 block)}, no_repeat all.
    m.insert("TheObscura", MonsterAi {
        model_id: "TheObscura",
        moves: vec![
            MonsterMove {
                id: "ILLUSION_MOVE",
                kind: IntentKind::Summon,
                body: vec![Effect::SummonMonster {
                    monster_id: "Parafright".to_string(),
                    slot: "illusion".to_string(),
                }],
            },
            MonsterMove::attack("PIERCING_GAZE_MOVE", 10, 1),
            MonsterMove::buff("SAIL_MOVE", "StrengthPower", 3),
            MonsterMove::attack_defend("HARDENING_STRIKE_MOVE", 6, 1, 7),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "ILLUSION_MOVE",
            then: Box::new(MovePattern::WeightedRandom {
                weights: vec![
                    ("PIERCING_GAZE_MOVE", 1),
                    ("SAIL_MOVE", 1),
                    ("HARDENING_STRIKE_MOVE", 1),
                ],
                no_repeat: vec![
                    "PIERCING_GAZE_MOVE",
                    "SAIL_MOVE",
                    "HARDENING_STRIKE_MOVE",
                ],
            }),
        },
    });

    // Queen: PuppetStrings → YourMine → BurnBright → OffWithYourHead →
    //   Execution → Enrage → OffWithYourHead loop. The "HasAmalgamDied"
    //   conditional split is approximated as straight cycle for now —
    //   tracking amalgam death requires a custom hook.
    m.insert("Queen", MonsterAi {
        model_id: "Queen",
        moves: vec![
            MonsterMove::debuff("PUPPET_STRINGS_MOVE", "ChainsOfBindingPower", 3),
            MonsterMove {
                id: "YOUR_MINE_MOVE",
                kind: IntentKind::Debuff,
                body: vec![
                    Effect::ApplyPower {
                        power_id: "FrailPower".to_string(),
                        amount: AmountSpec::Fixed(99),
                        target: Target::ChosenEnemy,
                    },
                    Effect::ApplyPower {
                        power_id: "WeakPower".to_string(),
                        amount: AmountSpec::Fixed(99),
                        target: Target::ChosenEnemy,
                    },
                    Effect::ApplyPower {
                        power_id: "VulnerablePower".to_string(),
                        amount: AmountSpec::Fixed(99),
                        target: Target::ChosenEnemy,
                    },
                ],
            },
            MonsterMove {
                id: "BURN_BRIGHT_FOR_ME_MOVE",
                kind: IntentKind::Buff,
                body: vec![
                    Effect::GainBlock {
                        amount: AmountSpec::Fixed(20),
                        target: Target::SelfActor,
                    },
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(1),
                        target: Target::SelfActor,
                    },
                ],
            },
            MonsterMove::attack("OFF_WITH_YOUR_HEAD_MOVE", 3, 5),
            MonsterMove::attack("EXECUTION_MOVE", 15, 1),
            MonsterMove::buff("ENRAGE_MOVE", "StrengthPower", 2),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "PUPPET_STRINGS_MOVE",
                "YOUR_MINE_MOVE",
                "BURN_BRIGHT_FOR_ME_MOVE",
                "OFF_WITH_YOUR_HEAD_MOVE",
                "EXECUTION_MOVE",
                "ENRAGE_MOVE",
            ],
        },
    });

    // CeremonialBeast: Stamp(+Plow 150) → Plow(18+Str 2) → Plow(loop).
    // C# also has a Beast Cry → Stomp → Crush sub-cycle (post-plow-removal
    // phase) which we approximate as the same cycle.
    m.insert("CeremonialBeast", MonsterAi {
        model_id: "CeremonialBeast",
        moves: vec![
            MonsterMove::buff("STAMP_MOVE", "PlowPower", 150),
            MonsterMove::attack_buff("PLOW_MOVE", 18, 1, "StrengthPower", 2),
            MonsterMove::debuff("BEAST_CRY_MOVE", "RingingPower", 1),
            MonsterMove::attack("STOMP_MOVE", 15, 1),
            MonsterMove::attack_buff("CRUSH_MOVE", 17, 1, "StrengthPower", 3),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "STAMP_MOVE",
            then: Box::new(MovePattern::Cycle {
                moves: vec!["PLOW_MOVE", "PLOW_MOVE"],
            }),
        },
    });

    // KnowledgeDemon: simplified port. The C# spec has a curse-card
    // injection minigame; we approximate as a 4-cycle attack pattern
    // until card-injection-from-curse-pool primitive lands.
    m.insert("KnowledgeDemon", MonsterAi {
        model_id: "KnowledgeDemon",
        moves: vec![
            MonsterMove {
                id: "CURSE_OF_KNOWLEDGE_MOVE",
                kind: IntentKind::Debuff,
                // Approximation: add a Doubt curse to player deck.
                // Actual C# offers a player-pick from 3 curse sets.
                body: vec![Effect::AddCardToPile {
                    card_id: "Doubt".to_string(),
                    upgrade: 0,
                    pile: crate::effects::Pile::Discard,
                }],
            },
            MonsterMove::attack("SLAP_MOVE", 17, 1),
            MonsterMove::attack("KNOWLEDGE_OVERWHELMING_MOVE", 8, 3),
            MonsterMove::attack_buff("PONDER_MOVE", 11, 1, "StrengthPower", 2),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "CURSE_OF_KNOWLEDGE_MOVE",
                "SLAP_MOVE",
                "KNOWLEDGE_OVERWHELMING_MOVE",
                "PONDER_MOVE",
            ],
        },
    });

    // TheInsatiable: Liquify → Thrash(8×2) → LungingBite(28) → Salivate(+Str 2)
    //   → Thrash → Thrash loop. C# injects 6 FranticEscape status cards on
    //   Liquify — partially modeled.
    m.insert("TheInsatiable", MonsterAi {
        model_id: "TheInsatiable",
        moves: vec![
            MonsterMove {
                id: "LIQUIFY_GROUND_MOVE",
                kind: IntentKind::Buff,
                body: vec![
                    Effect::ApplyPower {
                        power_id: "SandpitPower".to_string(),
                        amount: AmountSpec::Fixed(4),
                        target: Target::SelfActor,
                    },
                    Effect::AddCardToPile {
                        card_id: "FranticEscape".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Draw,
                    },
                ],
            },
            MonsterMove::attack("THRASH_MOVE", 8, 2),
            MonsterMove::attack("LUNGING_BITE_MOVE", 28, 1),
            MonsterMove::buff("SALIVATE_MOVE", "StrengthPower", 2),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "LIQUIFY_GROUND_MOVE",
                "THRASH_MOVE",
                "LUNGING_BITE_MOVE",
                "SALIVATE_MOVE",
                "THRASH_MOVE",
                "THRASH_MOVE",
            ],
        },
    });

    // WaterfallGiant: Pressurize (+SteamEruption 20) → Stomp(15+Weak 1) →
    //   Ram(10) → Siphon(no-op — heal not yet wired through monster context)
    //   → PressureGun(23) → PressureUp(13) → Stomp(loop). Phase transition
    //   (AboutToBlow → Explode) requires HpThresholdSwitch wired with custom
    //   state; deferred.
    m.insert("WaterfallGiant", MonsterAi {
        model_id: "WaterfallGiant",
        moves: vec![
            MonsterMove::buff("PRESSURIZE_MOVE", "SteamEruptionPower", 20),
            MonsterMove::attack_debuff("STOMP_MOVE", 15, 1, "WeakPower", 1),
            MonsterMove::attack("RAM_MOVE", 10, 1),
            MonsterMove::sleep("SIPHON_MOVE"),
            MonsterMove::attack("PRESSURE_GUN_MOVE", 23, 1),
            MonsterMove::attack("PRESSURE_UP_MOVE", 13, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec![
                "PRESSURIZE_MOVE",
                "STOMP_MOVE",
                "RAM_MOVE",
                "SIPHON_MOVE",
                "PRESSURE_GUN_MOVE",
                "PRESSURE_UP_MOVE",
            ],
        },
    });

    // Fabricator: Conditional on ally count.
    //   if can fabricate (allies < cap) → 50/50 {Fabricate (summon), FabricatingStrike (18 + summon)}
    //   else → Disintegrate(11). Summon body deferred — only the strike does damage.
    m.insert("Fabricator", MonsterAi {
        model_id: "Fabricator",
        moves: vec![
            MonsterMove::sleep("FABRICATE_MOVE"),
            MonsterMove::attack("FABRICATING_STRIKE_MOVE", 18, 1),
            MonsterMove::attack("DISINTEGRATE_MOVE", 11, 1),
        ],
        spawn: vec![],
        pattern: MovePattern::Conditional {
            // Approximation: treat ally count >= 3 as "cap reached".
            predicate: AiCondition::LivingEnemyCountLessThan(3),
            then_branch: Box::new(MovePattern::WeightedRandom {
                weights: vec![("FABRICATE_MOVE", 1), ("FABRICATING_STRIKE_MOVE", 1)],
                no_repeat: vec![],
            }),
            else_branch: Box::new(MovePattern::Cycle {
                moves: vec!["DISINTEGRATE_MOVE"],
            }),
        },
    });

    // Ovicopter: Conditional on ally count.
    //   if can lay (alive allies ≤ 3) → LayEggs (summon stub)
    //   else → NutritionalPaste(+3 Str).
    //   Plus Tenderizer(7+Vuln 2) and Smash(16) as alternates.
    m.insert("Ovicopter", MonsterAi {
        model_id: "Ovicopter",
        moves: vec![
            MonsterMove::sleep("LAY_EGGS_MOVE"),
            MonsterMove::attack("SMASH_MOVE", 16, 1),
            MonsterMove::attack_debuff("TENDERIZER_MOVE", 7, 1, "VulnerablePower", 2),
            MonsterMove::buff("NUTRITIONAL_PASTE_MOVE", "StrengthPower", 3),
        ],
        spawn: vec![],
        pattern: MovePattern::FirstTurnOverride {
            first_move: "LAY_EGGS_MOVE",
            then: Box::new(MovePattern::WeightedRandom {
                weights: vec![
                    ("SMASH_MOVE", 2),
                    ("TENDERIZER_MOVE", 1),
                    ("NUTRITIONAL_PASTE_MOVE", 1),
                ],
                no_repeat: vec![],
            }),
        },
    });

    // Guardbot: minimal port — guards Fabricators with block. C# applies
    // 15 block to Fabricator allies; in lieu of multi-target block we
    // give the Guardbot its own block as a placeholder.
    m.insert("Guardbot", MonsterAi {
        model_id: "Guardbot",
        moves: vec![MonsterMove::defend("GUARD_MOVE", 15)],
        spawn: vec![],
        pattern: MovePattern::Cycle { moves: vec!["GUARD_MOVE"] },
    });

    // FakeMerchantMonster: a "talks then attacks once revealed" boss.
    // C# uses dialogue choices to start combat; once combat starts the
    // monster pattern is a simple cycle. Encode as 3-move cycle.
    m.insert("FakeMerchantMonster", MonsterAi {
        model_id: "FakeMerchantMonster",
        moves: vec![
            MonsterMove::attack("SLASH_MOVE", 12, 1),
            MonsterMove::attack("BACKSTAB_MOVE", 18, 1),
            MonsterMove::buff("DEALS_MOVE", "StrengthPower", 2),
        ],
        spawn: vec![],
        pattern: MovePattern::Cycle {
            moves: vec!["SLASH_MOVE", "BACKSTAB_MOVE", "DEALS_MOVE"],
        },
    });
}

/// Fifth batch — test fixtures, Osty (player pet), and the remaining
/// odd-shaped real monsters (TestSubject + Wriggler).
///
/// Every entry is still a generic `MovePattern` over `MonsterMove`
/// primitives — no monster-name branching in the runtime. The agent
/// observes the move list (id + body) and the next-move selection
/// signal, never the monster id.
fn register_misc_b5(m: &mut HashMap<&'static str, MonsterAi>) {
    use crate::effects::AmountSpec;

    // Test fixtures (BigDummy / DeprecatedMonster / OneHpMonster /
    // TenHpMonster) and Osty (player pet) all share the same trivial
    // "do nothing forever" pattern. Registered so the AI lookup never
    // returns None for valid MonsterModel ids — keeps the runtime
    // free of model-id allowlists.
    for id in [
        "BigDummy",
        "DeprecatedMonster",
        "OneHpMonster",
        "TenHpMonster",
        "Osty",
    ] {
        m.insert(
            id,
            MonsterAi {
                model_id: id,
                moves: vec![MonsterMove::sleep("NOTHING_MOVE")],
                spawn: vec![],
                pattern: MovePattern::Cycle { moves: vec!["NOTHING_MOVE"] },
            },
        );
    }

    // BattleFriendV1/V2/V3: passive NPC ally tiers. All NOTHING-loop;
    // spawn applies BattlewornDummyTimeLimitPower(3) which counts down
    // and expires the ally. Same pattern, different HP (handled in
    // MonsterData, not here).
    for id in ["BattleFriendV1", "BattleFriendV2", "BattleFriendV3"] {
        m.insert(
            id,
            MonsterAi {
                model_id: id,
                moves: vec![MonsterMove::sleep("NOTHING_MOVE")],
                spawn: vec![Effect::ApplyPower {
                    power_id: "BattlewornDummyTimeLimitPower".to_string(),
                    amount: AmountSpec::Fixed(3),
                    target: Target::SelfActor,
                }],
                pattern: MovePattern::Cycle { moves: vec!["NOTHING_MOVE"] },
            },
        );
    }

    // TestSubject: a 3-phase HP-threshold boss. C# tracks a Respawns
    // counter that bumps each time HP crosses a threshold; we
    // approximate with HpThresholdSwitch since the agent-visible
    // distinction is "which cycle is active right now."
    //
    // Phase 1 (>66% HP): BITE ↔ SKULL_BASH alternation.
    // Phase 2 (33-66% HP): MULTI_CLAW cycle.
    // Phase 3 (<33% HP): LACERATE → BIG_POUNCE → BURNING_GROWL cycle.
    //
    // Spawn payload: AdaptablePower(1) + EnragePower(2). These power
    // ids may not be wired in the combat VM yet — the Effect list is
    // still the primitive, the no-op fallback is graceful, and the
    // observation surface is unchanged when those powers land.
    m.insert("TestSubject", MonsterAi {
        model_id: "TestSubject",
        moves: vec![
            MonsterMove::attack("BITE_MOVE", 20, 1),
            MonsterMove::attack_debuff("SKULL_BASH_MOVE", 12, 1, "VulnerablePower", 2),
            MonsterMove::attack("MULTI_CLAW_MOVE", 10, 3),
            MonsterMove::attack("PHASE3_LACERATE_MOVE", 10, 3),
            MonsterMove::attack("BIG_POUNCE_MOVE", 45, 1),
            MonsterMove {
                id: "BURNING_GROWL_MOVE",
                kind: IntentKind::Debuff,
                body: vec![
                    Effect::AddCardToPile {
                        card_id: "Burn".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    },
                ],
            },
        ],
        spawn: vec![
            Effect::ApplyPower {
                power_id: "AdaptablePower".to_string(),
                amount: AmountSpec::Fixed(1),
                target: Target::SelfActor,
            },
            Effect::ApplyPower {
                power_id: "EnragePower".to_string(),
                amount: AmountSpec::Fixed(2),
                target: Target::SelfActor,
            },
        ],
        pattern: MovePattern::HpThresholdSwitch {
            threshold_pct: 33,
            below: Box::new(MovePattern::Cycle {
                moves: vec![
                    "PHASE3_LACERATE_MOVE",
                    "BIG_POUNCE_MOVE",
                    "BURNING_GROWL_MOVE",
                ],
            }),
            above: Box::new(MovePattern::HpThresholdSwitch {
                threshold_pct: 66,
                below: Box::new(MovePattern::Cycle { moves: vec!["MULTI_CLAW_MOVE"] }),
                above: Box::new(MovePattern::Cycle {
                    moves: vec!["BITE_MOVE", "SKULL_BASH_MOVE"],
                }),
            }),
        },
    });

    // Wriggler: encounter-config-driven slot dispatch. Slots
    // "wriggler1" / "wriggler3" open on NastyBite; "wriggler2" /
    // "wriggler4" open on Wriggle; thereafter alternate.
    //
    // WRIGGLE_MOVE: apply Infection card to player + Strength+2 self.
    // The C# StartStunned flag is a per-instance encounter property,
    // not part of the AI table — handled by combat init if/when the
    // encounter sets it.
    m.insert("Wriggler", MonsterAi {
        model_id: "Wriggler",
        moves: vec![
            MonsterMove::attack("NASTY_BITE_MOVE", 6, 1),
            MonsterMove {
                id: "WRIGGLE_MOVE",
                kind: IntentKind::Buff,
                body: vec![
                    Effect::AddCardToPile {
                        card_id: "Infection".to_string(),
                        upgrade: 0,
                        pile: crate::effects::Pile::Discard,
                    },
                    Effect::ApplyPower {
                        power_id: "StrengthPower".to_string(),
                        amount: AmountSpec::Fixed(2),
                        target: Target::SelfActor,
                    },
                ],
            },
            MonsterMove::sleep("SPAWNED_MOVE"),
        ],
        spawn: vec![],
        pattern: MovePattern::BySlot {
            branches: vec![
                ("wriggler1", MovePattern::Cycle {
                    moves: vec!["NASTY_BITE_MOVE", "WRIGGLE_MOVE"],
                }),
                ("wriggler3", MovePattern::Cycle {
                    moves: vec!["NASTY_BITE_MOVE", "WRIGGLE_MOVE"],
                }),
                ("wriggler2", MovePattern::Cycle {
                    moves: vec!["WRIGGLE_MOVE", "NASTY_BITE_MOVE"],
                }),
                ("wriggler4", MovePattern::Cycle {
                    moves: vec!["WRIGGLE_MOVE", "NASTY_BITE_MOVE"],
                }),
            ],
            default: Box::new(MovePattern::Cycle {
                moves: vec!["NASTY_BITE_MOVE", "WRIGGLE_MOVE"],
            }),
        },
    });
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
        // + 4 three-move + 4 weighted + 4 flag-state
        // + 22 batch 1 + 19 batch 2 + 18 batch 3 + 19 batch 4
        // + 10 batch 5 (fixtures + Osty + BattleFriendV1/2/3 +
        //   TestSubject + Wriggler)
        // = 120 monsters. Abstract DecimillipedeSegment is unused;
        // its 3 concrete subclasses live in the Decimillipede
        // for-loop already counted in batch 3.
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
            "Axebot", "Myte", "Nibbit", "FlailKnight",
            "Toadpole", "ThievingHopper",
            "CalcifiedCultist", "DevotedSculptor", "Seapunk", "GlobeHead",
            "TwigSlimeS", "LeafSlimeM", "BowlbugEgg", "Vantom",
            "SpinyToad", "SlimedBerserker", "PhrogParasite", "SoulFysh",
            "TurretOperator", "OwlMagistrate", "SoulNexus",
            // Batch 2:
            "BowlbugSilk", "BowlbugNectar", "BygoneEffigy", "Byrdonis",
            "Chomper", "CorpseSlug", "Crusher", "Entomancer",
            "FuzzyWurmCrawler", "HauntedShip", "LeafSlimeS",
            "LouseProgenitor", "MagiKnight", "MechaKnight",
            "ShrinkerBeetle", "SkulkingColony", "SludgeSpinner",
            "SpectralKnight", "Rocket", "TwoTailedRat",
            // Batch 3:
            "TwigSlimeM", "MysteriousKnight", "Tunneler",
            "TorchHeadAmalgam", "LivingFog", "Doormaker", "InfestedPrism",
            "SlumberingBeetle", "TerrorEel", "BowlbugRock", "LivingShield",
            "Exoskeleton", "PhantasmalGardener", "ScrollOfBiting",
            "SlitheringStrangler",
            "DecimillipedeSegmentFront", "DecimillipedeSegmentMiddle",
            "DecimillipedeSegmentBack",
            // Batch 4 — bosses + specials:
            "Architect", "PaelsLegion", "TheLost", "TheForgotten",
            "TheAdversaryMkOne", "TheAdversaryMkTwo", "TheAdversaryMkThree",
            "CubexConstruct", "LagavulinMatriarch", "TheObscura", "Queen",
            "CeremonialBeast", "KnowledgeDemon", "TheInsatiable",
            "WaterfallGiant", "Fabricator", "Ovicopter", "Guardbot",
            "FakeMerchantMonster",
            // Batch 5 — test fixtures + Osty + BattleFriends +
            // odd-shaped bosses:
            "BigDummy", "DeprecatedMonster", "OneHpMonster",
            "TenHpMonster", "Osty", "TestSubject", "Wriggler",
            "BattleFriendV1", "BattleFriendV2", "BattleFriendV3",
        ];
        for id in expected {
            assert!(ai_for(id).is_some(), "Missing AI for {}", id);
        }
        assert_eq!(MONSTER_AI_REGISTRY.len(), expected.len());
    }

    /// Property: every concrete MonsterModel in `ALL_MONSTERS` has an
    /// AI registration. The abstract `DecimillipedeSegment` base has
    /// no instances — its three concrete subclasses are registered
    /// instead. This test catches new MonsterModel subclasses that
    /// the extractor picks up but no AI port has landed for yet.
    #[test]
    fn every_concrete_monster_resolves_to_ai() {
        use crate::monster::ALL_MONSTERS;

        // Abstract bases that exist in monsters.json (via the
        // inheritance walker) but never spawn standalone.
        const ABSTRACT_BASES: &[&str] = &["DecimillipedeSegment"];

        let mut missing = Vec::new();
        for md in ALL_MONSTERS.iter() {
            if ABSTRACT_BASES.contains(&md.id.as_str()) {
                continue;
            }
            if ai_for(&md.id).is_none() {
                missing.push(md.id.clone());
            }
        }
        assert!(
            missing.is_empty(),
            "MonsterModel ids without AI registration: {:?}",
            missing,
        );
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
