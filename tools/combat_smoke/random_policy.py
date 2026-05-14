#!/usr/bin/env python3
"""Random-policy smoke test for the sts2-sim-py PyO3 bindings.

Runs N combats against an encounter using a uniformly random policy and
prints aggregate stats. See README.md for usage.

Designed to crash fast on common boundary issues: empty legal-action
sets while non-terminal, JSON parse errors, observation-schema drift.
"""

from __future__ import annotations

import argparse
import json
import random
import sys
from collections import Counter
from dataclasses import dataclass

try:
    import sts2_sim_py
except ImportError:
    sys.exit(
        "sts2_sim_py not importable. Build it with:\n"
        "  cd crates/sts2-sim-py && maturin develop --release"
    )


@dataclass
class FightResult:
    won: bool
    rounds: int
    final_hp: int
    action_counts: Counter

    @classmethod
    def empty(cls) -> "FightResult":
        return cls(won=False, rounds=0, final_hp=0, action_counts=Counter())


def play_one(seed: int, character: str, encounter: str, *, verbose: bool) -> FightResult:
    """Play a single combat with a uniformly-random policy.

    Returns a FightResult. Crashes (intentionally) if the legal-action
    list is empty while the env is non-terminal — that's a simulator
    bug worth surfacing.
    """
    env = sts2_sim_py.PyCombatEnv(seed=seed, character=character, encounter=encounter)
    rng = random.Random(seed)
    actions_taken = Counter()
    step_cap = 500  # cheap safety net for infinite-loop bugs

    for step in range(step_cap):
        if env.is_terminal():
            break
        legal = json.loads(env.legal_actions())
        if not legal:
            sys.exit(
                f"non-terminal env produced no legal actions (seed={seed}, "
                f"round={env.round_number()})"
            )
        action = rng.choice(legal)
        # Action variants are like {"PlayCard": {...}} or {"EndTurn":
        # {...}} — single-key dicts. Use the key as the bucket name.
        actions_taken[next(iter(action))] += 1
        outcome = json.loads(env.step(json.dumps(action)))
        if verbose:
            tag = next(iter(action))
            res = outcome.get("result") or outcome.get("play_result") or ""
            print(f"  [seed={seed} r{env.round_number()}] {tag} -> {res}")
    else:
        sys.exit(f"step cap ({step_cap}) hit on seed={seed} — possible loop bug")

    obs = json.loads(env.observation())
    # CreatureStateFeatures is a 14-float vector. Indices we care about
    # (matches features.rs IDX_CREATURE_*):
    #   1: alive (1.0 if hp > 0)
    #   2: hp_frac = current_hp / max(1, max_hp)
    #   3: max_hp (raw)
    HP_FRAC = 2
    MAX_HP = 3
    ALIVE = 1

    def creature_hp(feat: dict) -> int:
        v = feat["values"]
        return round(v[HP_FRAC] * v[MAX_HP])

    players = obs.get("players", [])
    enemies = obs.get("enemies", [])
    final_hp = creature_hp(players[0]) if players else 0
    # Victory: every enemy is no longer alive.
    won = bool(enemies) and all(e["values"][ALIVE] == 0.0 for e in enemies)
    return FightResult(
        won=won,
        rounds=env.round_number(),
        final_hp=final_hp,
        action_counts=actions_taken,
    )


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--fights", type=int, default=100)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--character", default="Ironclad")
    p.add_argument("--encounter", default="AxebotsNormal")
    p.add_argument("--verbose", action="store_true")
    args = p.parse_args()

    print("sts2_sim_py random-policy smoke test")
    print(f"  character          = {args.character}")
    print(f"  encounter          = {args.encounter}")
    print(f"  fights             = {args.fights}")
    print(f"  seed (initial)     = {args.seed}")
    print(f"  schema version     = {sts2_sim_py.observation_schema_version()}")

    wins = 0
    total_rounds = 0
    win_hp_sum = 0
    actions = Counter()
    for i in range(args.fights):
        r = play_one(
            seed=args.seed + i,
            character=args.character,
            encounter=args.encounter,
            verbose=args.verbose,
        )
        if r.won:
            wins += 1
            win_hp_sum += r.final_hp
        total_rounds += r.rounds
        actions.update(r.action_counts)

    losses = args.fights - wins
    avg_rounds = total_rounds / max(args.fights, 1)
    avg_win_hp = win_hp_sum / wins if wins else 0.0

    print()
    print("results:")
    print(f"  win rate           = {wins/args.fights:.3f} ({wins}/{args.fights})")
    print(f"  defeat rate        = {losses/args.fights:.3f} ({losses}/{args.fights})")
    print(f"  avg rounds         = {avg_rounds:.1f}")
    print(f"  avg final-hp (win) = {avg_win_hp:.1f}")
    print(f"  action mix:")
    for tag, n in actions.most_common():
        print(f"    {tag:16} = {n}")


if __name__ == "__main__":
    main()
