//! Gym-style combat environment.
//!
//! Phase 0.3 API surface. Wraps `CombatState` in the canonical RL
//! interface — `reset` / `step` / `legal_actions` / `observation` /
//! `clone_state` / `set_state` — so training and analysis code can treat
//! combat as a stationary MDP.
//!
//! Scope for this commit: **combat-only**. Strategic-layer decisions
//! (card pick, map node choice, event option, shop, campfire) are a
//! separate `StrategicEnv` that lands when the strategic loop ports.
//!
//! Determinism: every randomized op routes through an explicit `Rng`
//! the caller seeded. Reset takes a seed; `clone_state` snapshots the
//! whole state including the RNG counter so MCTS-style branching
//! reproduces.
//!
//! ## Action space
//!
//! Variable per turn. `legal_actions` returns the set of valid plays
//! given current state. Most cards target an enemy or self; some
//! cards / potions target arbitrary creatures. The agent sees masking
//! as `legal_actions` — illegal `step` calls return an error result.
//!
//! ## Observation
//!
//! For now we expose the full `CombatState` as the observation —
//! featurization (cards → embeddings, transformer input) lives in the
//! agent crate. This keeps the simulator's surface stable while
//! featurization details iterate.

use crate::combat::{
    CombatResult, CombatRewards, CombatSide, CombatState, PlayResult, PlayerSetup,
    INITIAL_HAND_SIZE,
};
use crate::rng::Rng;
use serde::{Deserialize, Serialize};

/// The set of actions the agent can take during a combat turn.
///
/// Mirrors the C# `PlayerChoice` family (card-play, end-turn, potion-use)
/// at a coarse granularity. Card-play with multi-card-pick choices
/// (Discovery, etc.) will need a richer sub-action; not modeled yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Play the card at `hand_idx` from the given player's hand.
    /// `target` is None when the card's `TargetType` doesn't need one
    /// (Self, AllEnemies, RandomEnemy, TargetedNoCreature).
    PlayCard {
        player_idx: usize,
        hand_idx: usize,
        target: Option<(CombatSide, usize)>,
    },
    /// End the current player's turn. Triggers the enemy turn loop in
    /// `step()` (intent execution + intent selection for next turn).
    EndTurn { player_idx: usize },
    /// Use the potion at the given slot. Targeting same as cards.
    /// Potion behavior is deferred — `step` returns `Unhandled` for
    /// potion actions until potion OnUse lands.
    UsePotion {
        player_idx: usize,
        slot_index: i32,
        target: Option<(CombatSide, usize)>,
    },
}

/// Result of one `step()`. Reward shaping is intentionally minimal at
/// this layer — the agent crate adds dense shaping (HP-preserved, etc.)
/// during training as needed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepOutcome {
    /// True iff the combat has terminated (Victory / Defeat).
    pub terminal: bool,
    /// Sparse terminal reward: +1 for Victory, -1 for Defeat, 0
    /// otherwise. Agent training overlays its own shaping.
    pub reward: f32,
    /// `PlayResult` from the underlying card-play, when the action
    /// was a `PlayCard`. `None` for EndTurn / UsePotion.
    pub play_result: Option<PlayResult>,
    /// `CombatResult` once `terminal == true`. None until then.
    pub result: Option<CombatResult>,
    /// End-of-combat rewards when terminal == true && Victory.
    pub rewards: Option<CombatRewards>,
}

impl Default for StepOutcome {
    fn default() -> Self {
        Self {
            terminal: false,
            reward: 0.0,
            play_result: None,
            result: None,
            rewards: None,
        }
    }
}

/// Combat-only environment. Owns the `CombatState`, an Rng for draws /
/// monster intents / reward rolls, and a snapshot of the last action's
/// outcome. Compose with strategic-layer logic upstream.
#[derive(Clone, Debug)]
pub struct CombatEnv {
    pub state: CombatState,
    pub rng: Rng,
}

impl CombatEnv {
    /// Build a fresh combat from `CombatState::start`-style inputs.
    /// Caller supplies the encounter, players, modifiers, and the seed
    /// for this combat's deterministic Rng stream.
    pub fn reset(
        encounter: &crate::encounter::EncounterData,
        players: Vec<PlayerSetup>,
        modifier_ids: Vec<String>,
        rng_seed: u32,
    ) -> Self {
        let mut state = CombatState::start(encounter, players, modifier_ids);
        // Seed the combat-scoped RNG inside the state. Cards that need
        // randomness (PommelStrike draw, Cinder hand exhaust, ...) read
        // it via `state.rng`. We use a derived seed so it doesn't share
        // a stream with the env-level rng used for rewards.
        state.rng = Rng::new(rng_seed.wrapping_add(0x1f_b7_d6_c5), 0);
        // Fire combat-start relic hooks. C# CombatManager does this as
        // part of opening combat; we run it eagerly so the agent's
        // first `observation()` reflects post-hook state.
        state.fire_before_combat_start_hooks();
        // Monster `AfterAddedToRoom` payloads — applies per-monster
        // start-of-combat powers (HardToKill, EscapeArtist, Plating,
        // Artifact, ...). Mirrors C# CombatRoom's per-monster
        // AfterAddedToRoom dispatch.
        crate::monster_dispatch::fire_monster_spawn_hooks(&mut state);
        // Initial draw: every player starts the first turn with 5 cards
        // in hand. C# CombatManager.BeginCombat triggers this. Without
        // this, agents see an empty hand on the first observation and
        // can only EndTurn.
        let n_players = state.allies.len();
        for player_idx in 0..n_players {
            // Innate keyword: pull Innate cards from the draw pile to
            // the hand BEFORE the standard initial draw. Mirrors C#
            // PlayerCombatState start-of-combat innate priority — Innates
            // are guaranteed in the opening hand. The remaining draw
            // fills up to INITIAL_HAND_SIZE total.
            let innate_count = state.move_innate_cards_to_hand(player_idx);
            // Round-1 hand-draw delta from ModifyRound1HandDraw (set by
            // BagOfPreparation/RingOfTheSnake/BoomingConch in their
            // BeforeCombatStart hooks). Consumed once.
            let round1_delta = state
                .allies
                .get(player_idx)
                .and_then(|c| c.player.as_ref())
                .map(|ps| ps.hand_draw_round1_delta)
                .unwrap_or(0);
            if let Some(ps) = state.allies
                .get_mut(player_idx)
                .and_then(|c| c.player.as_mut())
            {
                ps.hand_draw_round1_delta = 0;
            }
            let target = INITIAL_HAND_SIZE + round1_delta;
            let remaining = (target - innate_count).max(0);
            if remaining > 0 {
                let mut rng_taken =
                    std::mem::replace(&mut state.rng, Rng::new(0, 0));
                state.draw_cards(player_idx, remaining, &mut rng_taken);
                state.rng = rng_taken;
            }
        }
        Self {
            state,
            rng: Rng::new(rng_seed, 0),
        }
    }

    /// Apply one action and return its outcome. Walks the enemy turn
    /// automatically after a successful `EndTurn` (intent execution
    /// for each living enemy), then re-detects combat-end.
    ///
    /// Returns `play_result = InvalidHand / InvalidTarget / ...` for
    /// rejected actions; state stays untouched and `terminal` is false.
    pub fn step(&mut self, action: Action) -> StepOutcome {
        match action {
            Action::PlayCard {
                player_idx,
                hand_idx,
                target,
            } => {
                let play_result = self.state.play_card(player_idx, hand_idx, target);
                let success = matches!(
                    play_result,
                    PlayResult::Ok | PlayResult::Unhandled
                );
                let mut outcome = StepOutcome {
                    play_result: Some(play_result),
                    ..StepOutcome::default()
                };
                if success {
                    self.detect_terminal(&mut outcome);
                }
                outcome
            }
            Action::EndTurn { player_idx: _ } => {
                self.state.end_turn();
                self.state.begin_turn(CombatSide::Enemy);
                // Enemy turn dispatch: for every living enemy, pick
                // and execute its next intent. Unported monsters
                // (model_id not in the registry) skip silently —
                // their `dispatch_enemy_turn` returns false. The
                // for-loop reads len() each iteration so spawned
                // monsters mid-turn would be picked up; today no
                // monster summons enemies inside a turn.
                for enemy_idx in 0..self.state.enemies.len() {
                    // Always target the first player; multiplayer
                    // target selection isn't modeled.
                    crate::monster_dispatch::dispatch_enemy_turn(
                        &mut self.state,
                        enemy_idx,
                        0,
                    );
                    // If the player died mid-turn, bail — no more
                    // enemy moves should resolve.
                    if self
                        .state
                        .allies
                        .first()
                        .map(|a| a.current_hp == 0)
                        .unwrap_or(true)
                    {
                        break;
                    }
                }
                self.state.end_turn();
                self.state.begin_turn(CombatSide::Player);
                // Per-turn 5-card draw at the start of the player's
                // turn. C# CombatManager triggers this via the
                // Hook.ModifyDraw pipeline; without it the player
                // can't refill their hand between turns. Run after
                // begin_turn so block clears + start-of-turn power
                // ticks (DemonForm/Poison) sequence first.
                let n_players = self.state.allies.len();
                for player_idx in 0..n_players {
                    let mut rng_taken = std::mem::replace(
                        &mut self.state.rng,
                        Rng::new(0, 0),
                    );
                    self.state
                        .draw_cards(player_idx, INITIAL_HAND_SIZE, &mut rng_taken);
                    self.state.rng = rng_taken;
                }
                let mut outcome = StepOutcome::default();
                self.detect_terminal(&mut outcome);
                outcome
            }
            Action::UsePotion { player_idx, slot_index: _, target } => {
                // Combat state doesn't carry the potion belt today; the
                // caller (Python-side strategic layer) supplies the
                // potion id via a side channel. Until the belt lands
                // here we route via `step_with_potion_id` for tests
                // and leave this arm as Unhandled.
                let _ = (player_idx, target);
                StepOutcome {
                    play_result: Some(PlayResult::Unhandled),
                    ..StepOutcome::default()
                }
            }
        }
    }

    fn detect_terminal(&mut self, outcome: &mut StepOutcome) {
        if let Some(result) = self.state.is_combat_over() {
            outcome.terminal = true;
            outcome.reward = match result {
                CombatResult::Victory => 1.0,
                CombatResult::Defeat => -1.0,
            };
            outcome.result = Some(result);
            if result == CombatResult::Victory {
                self.state.fire_after_combat_victory_hooks();
                outcome.rewards = Some(self.state.generate_rewards(&mut self.rng));
            }
        }
    }

    /// All actions the agent can legally take *right now*. Cards in
    /// hand that the player has energy for + EndTurn + any usable
    /// potions. Targeting is enumerated by `Action::PlayCard.target`:
    /// for `AnyEnemy` cards, one action per live enemy.
    pub fn legal_actions(&self) -> Vec<Action> {
        let mut actions: Vec<Action> = Vec::new();
        for (player_idx, creature) in self.state.allies.iter().enumerate() {
            let Some(ps) = creature.player.as_ref() else {
                continue;
            };
            for (hand_idx, card) in ps.hand.cards.iter().enumerate() {
                if card.effective_energy_cost() > ps.energy {
                    continue;
                }
                let Some(data) = crate::card::by_id(&card.id) else {
                    continue;
                };
                // Targeting: enumerate per the card's TargetType.
                use crate::card::TargetType as T;
                match data.target_type {
                    T::AnyEnemy => {
                        for (i, enemy) in self.state.enemies.iter().enumerate() {
                            if enemy.current_hp > 0 {
                                actions.push(Action::PlayCard {
                                    player_idx,
                                    hand_idx,
                                    target: Some((CombatSide::Enemy, i)),
                                });
                            }
                        }
                    }
                    T::None
                    | T::SelfTarget
                    | T::AllEnemies
                    | T::AllAllies
                    | T::RandomEnemy
                    | T::AnyPlayer
                    | T::AnyAlly
                    | T::TargetedNoCreature
                    | T::Osty => {
                        actions.push(Action::PlayCard {
                            player_idx,
                            hand_idx,
                            target: None,
                        });
                    }
                }
            }
            actions.push(Action::EndTurn { player_idx });
        }
        actions
    }

    /// Snapshot the env state. Use as the starting point for MCTS-style
    /// rollouts: clone, step variant actions, compare outcomes.
    pub fn clone_state(&self) -> Self {
        self.clone()
    }

    /// Restore env state from a prior snapshot. The state replaces
    /// in-place; no validation that the snapshot belongs to "this" env
    /// — caller's responsibility.
    pub fn set_state(&mut self, snapshot: Self) {
        *self = snapshot;
    }

    /// Current observation. Returns the full state for now; agent-side
    /// featurization (card → feature vector, transformer input)
    /// consumes it. Borrow the live state to avoid copies during
    /// training-time queries.
    pub fn observation(&self) -> &CombatState {
        &self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::by_id as card_by_id;
    use crate::character;
    use crate::combat::{deck_from_ids, CardInstance};
    use crate::encounter;

    fn fresh_env() -> CombatEnv {
        let ironclad = character::by_id("Ironclad").expect("Ironclad");
        let enc = encounter::by_id("AxebotsNormal").expect("AxebotsNormal");
        let deck = deck_from_ids(&ironclad.starting_deck);
        let setup = PlayerSetup {
            character: ironclad,
            current_hp: ironclad.starting_hp.unwrap(),
            max_hp: ironclad.starting_hp.unwrap(),
            deck,
            relics: ironclad.starting_relics.clone(),
        };
        CombatEnv::reset(enc, vec![setup], Vec::new(), 42)
    }

    #[test]
    fn reset_produces_fresh_state() {
        let env = fresh_env();
        assert_eq!(env.state.allies.len(), 1);
        assert_eq!(env.state.enemies.len(), 2);
        assert_eq!(env.state.round_number, 1);
        assert_eq!(env.state.current_side, CombatSide::Player);
    }

    #[test]
    fn step_play_strike_succeeds() {
        let mut env = fresh_env();
        // Inject a Strike directly into hand so legal_actions has
        // something playable.
        let strike = card_by_id("StrikeIronclad").unwrap();
        env.state.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(strike, 0));
        let hand_idx = env.state.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let out = env.step(Action::PlayCard {
            player_idx: 0,
            hand_idx,
            target: Some((CombatSide::Enemy, 0)),
        });
        assert_eq!(out.play_result, Some(PlayResult::Ok));
        assert!(!out.terminal);
        assert_eq!(out.reward, 0.0);
        assert!(env.state.enemies[0].current_hp < env.state.enemies[0].max_hp);
    }

    #[test]
    fn step_invalid_target_does_not_mutate() {
        let mut env = fresh_env();
        let strike = card_by_id("StrikeIronclad").unwrap();
        env.state.allies[0].player.as_mut().unwrap().hand.cards.push(
            CardInstance::from_card(strike, 0),
        );
        let hand_idx = env.state.allies[0].player.as_ref().unwrap().hand.len() - 1;
        let hp_before = env.state.enemies[0].current_hp;
        let energy_before = env.state.allies[0].player.as_ref().unwrap().energy;
        let out = env.step(Action::PlayCard {
            player_idx: 0,
            hand_idx,
            target: None, // Strike requires AnyEnemy
        });
        assert_eq!(out.play_result, Some(PlayResult::InvalidTarget));
        assert_eq!(env.state.enemies[0].current_hp, hp_before);
        assert_eq!(
            env.state.allies[0].player.as_ref().unwrap().energy,
            energy_before
        );
    }

    #[test]
    fn legal_actions_includes_strikes_and_end_turn() {
        let mut env = fresh_env();
        // env.reset triggers an initial 5-card draw. Empty the hand
        // first so we can assert about a controlled single-Strike setup.
        env.state.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .clear();
        // Move one Strike into hand.
        let strike = card_by_id("StrikeIronclad").unwrap();
        env.state.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .extend([CardInstance::from_card(strike, 0)]);
        let actions = env.legal_actions();
        // 1 Strike in hand × 2 enemies = 2 PlayCard variants + 1 EndTurn.
        assert_eq!(actions.len(), 3);
        assert!(actions
            .iter()
            .any(|a| matches!(a, Action::EndTurn { .. })));
        let play_count = actions
            .iter()
            .filter(|a| matches!(a, Action::PlayCard { .. }))
            .count();
        assert_eq!(play_count, 2);
    }

    #[test]
    fn legal_actions_excludes_unaffordable_cards() {
        // Strain energy; ensure cards exceeding cost are filtered out.
        let mut env = fresh_env();
        env.state.allies[0].player.as_mut().unwrap().energy = 0;
        let strike = card_by_id("StrikeIronclad").unwrap();
        env.state.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(strike, 0));
        let actions = env.legal_actions();
        let play_count = actions
            .iter()
            .filter(|a| matches!(a, Action::PlayCard { .. }))
            .count();
        assert_eq!(play_count, 0);
        // EndTurn still legal.
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn clone_and_set_state_round_trip() {
        let mut env = fresh_env();
        let snapshot = env.clone_state();
        // Mutate.
        env.state.allies[0].current_hp = 5;
        // Restore.
        env.set_state(snapshot);
        assert_eq!(env.state.allies[0].current_hp, 80);
    }

    #[test]
    fn step_terminal_with_victory_reward_one() {
        let mut env = fresh_env();
        // Cheat: drop both enemies to 1 HP; play Strike to kill the
        // first then another to kill the second.
        for e in env.state.enemies.iter_mut() {
            e.current_hp = 1;
        }
        let strike = card_by_id("StrikeIronclad").unwrap();
        env.state.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .clear();
        env.state.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(strike, 0));
        env.state.allies[0]
            .player
            .as_mut()
            .unwrap()
            .hand
            .cards
            .push(CardInstance::from_card(strike, 0));
        env.state.allies[0].player.as_mut().unwrap().energy = 10;

        // Strike 1 -> kill enemy 0 (first Strike in hand moves to discard).
        let out1 = env.step(Action::PlayCard {
            player_idx: 0,
            hand_idx: 0,
            target: Some((CombatSide::Enemy, 0)),
        });
        assert_eq!(out1.play_result, Some(PlayResult::Ok));
        assert!(!out1.terminal, "first kill shouldn't end combat — enemy 1 alive");
        assert_eq!(env.state.enemies[0].current_hp, 0);

        // Strike 2 -> kill enemy 1 (idx 1; enemies aren't compacted).
        let out2 = env.step(Action::PlayCard {
            player_idx: 0,
            hand_idx: 0,
            target: Some((CombatSide::Enemy, 1)),
        });
        assert_eq!(out2.play_result, Some(PlayResult::Ok));
        assert!(out2.terminal);
        assert_eq!(out2.reward, 1.0);
        assert_eq!(out2.result, Some(CombatResult::Victory));
        let rewards = out2.rewards.expect("Victory carries rewards");
        assert!(
            rewards.gold >= 10 && rewards.gold <= 20,
            "gold {} out of Monster range",
            rewards.gold
        );
    }
}
