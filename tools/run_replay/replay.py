#!/usr/bin/env python3
"""Replay each combat room in a `.run` corpus through the simulator.

Walks every combat node in each `.run` file, looks up the encounter in
the simulator's encounter table, builds a `PyCombatEnv` for it, and
runs a random-policy player-turn loop until terminal or a step cap
hits. Aggregates per-status counts so we can see what fraction of
real-game combats the simulator can currently load and step.

Limitations of the v1 harness:
- Enemy intents do NOT execute yet (`PyCombatEnv.step(EndTurn)` is a
  no-op on the enemy side until the monster-turn dispatcher lands).
  That means win rates here aren't meaningful — but encounter loads,
  spawn payloads, and player-action paths all are.
- Player state is the character's starting deck / starting HP. The
  `.run` file's per-floor `current_hp` / deck snapshot isn't applied,
  so all combats start at full HP with the starter deck. Fine for
  smoke-testing the simulator's combat plumbing, not for verifying
  damage-taken matches.

Output: prints aggregate counts then a per-encounter breakdown of
status frequencies. Exit code is 0 if every combat that loaded
reached a terminal state or step cap (no crashes), 1 if any combat
crashed.
"""

from __future__ import annotations

import argparse
import json
import random
import sys
import traceback
from collections import Counter, defaultdict
from pathlib import Path

try:
    import sts2_sim_py
except ImportError:
    sys.exit(
        "sts2_sim_py not importable. Build it with:\n"
        "  cd crates/sts2-sim-py && maturin develop --release"
    )

CHARACTER_PREFIX = "CHARACTER."
ENCOUNTER_PREFIX = "ENCOUNTER."

# Status codes for per-combat outcomes.
STATUS_VICTORY = "victory"
STATUS_DEFEAT = "defeat"
STATUS_STEP_CAP = "step-cap"
STATUS_ENV_BUILD_FAILED = "env-build-failed"
STATUS_STEP_CRASHED = "step-crashed"
STATUS_NO_LEGAL_ACTIONS = "no-legal-actions"
STATUS_UNKNOWN_ENCOUNTER = "unknown-encounter"


def normalize_id(prefixed: str) -> str:
    """Convert "ENCOUNTER.SEAPUNK_WEAK" → "SeapunkWeak"."""
    _, _, raw = prefixed.partition(".")
    if not raw:
        raw = prefixed
    return "".join(part[:1].upper() + part[1:].lower() for part in raw.split("_"))


def all_monsters_dispatched(monster_ids: list[str]) -> bool:
    """True iff every monster id in the encounter has a Rust-side
    dispatcher arm. Encounters where at least one monster no-ops on
    its turn give misleading victory counts — flag them separately."""
    if not monster_ids:
        return False
    return all(
        sts2_sim_py.monster_has_dispatch(normalize_id(m)) for m in monster_ids
    )


def normalize_character(prefixed: str) -> str:
    """Convert "CHARACTER.IRONCLAD" → "Ironclad"."""
    _, _, raw = prefixed.partition(".")
    if not raw:
        raw = prefixed
    return raw[:1].upper() + raw[1:].lower()


def load_known_encounters() -> set[str]:
    sim_data = Path(__file__).resolve().parents[2] / "crates" / "sts2-sim" / "data"
    with open(sim_data / "encounters.json", encoding="utf-8") as f:
        return {c["id"] for c in json.load(f)}


def replay_one(
    encounter: str,
    character: str,
    monster_ids: list[str],
    *,
    seed: int,
    step_cap: int,
) -> tuple[str, dict]:
    """Replay one combat. Returns (status_code, info_dict).

    Uses the `.run`-recorded monster_ids list directly via
    `PyCombatEnv.from_monsters` — bypasses the encounter table's
    canonical_monsters (which is dynamically filled at runtime in
    C# for some multi-monster encounters and so doesn't extract
    cleanly).
    """
    rng = random.Random(seed)
    # Normalize monster ids from "MONSTER.SEAPUNK" → "Seapunk".
    sim_monsters = [normalize_id(m) for m in monster_ids]
    try:
        env = sts2_sim_py.PyCombatEnv.from_monsters(
            seed=seed,
            character=character,
            encounter_id=encounter,
            monsters=sim_monsters,
        )
    except Exception as exc:
        return STATUS_ENV_BUILD_FAILED, {"error": repr(exc)}

    rounds = 0
    actions_taken = 0
    for _ in range(step_cap):
        if env.is_terminal():
            break
        try:
            legal_json = env.legal_actions()
        except Exception as exc:
            return STATUS_STEP_CRASHED, {
                "error": repr(exc),
                "rounds": rounds,
                "where": "legal_actions",
            }
        legal = json.loads(legal_json)
        if not legal:
            return STATUS_NO_LEGAL_ACTIONS, {
                "rounds": rounds,
                "where": "non-terminal but no legal actions",
            }
        action = rng.choice(legal)
        try:
            env.step(json.dumps(action))
        except Exception as exc:
            return STATUS_STEP_CRASHED, {
                "error": repr(exc),
                "action": action,
                "rounds": rounds,
                "where": "step",
            }
        rounds = env.round_number()
        actions_taken += 1
    else:
        return STATUS_STEP_CAP, {"rounds": rounds, "actions": actions_taken}

    # Terminal — determine victory vs defeat from observation.
    obs = json.loads(env.observation())
    enemies = obs.get("enemies", [])
    # CreatureStateFeatures serializes as a bare float array (see
    # features.rs:494 — custom Serialize on the struct). Index 1 is
    # "alive" (1.0 if hp > 0). Tolerate either bare-array or
    # {"values": [...]} in case the schema changes.
    ALIVE_IDX = 1

    def is_alive(e):
        vals = e["values"] if isinstance(e, dict) else e
        return vals[ALIVE_IDX] != 0.0

    won = bool(enemies) and not any(is_alive(e) for e in enemies)
    return (STATUS_VICTORY if won else STATUS_DEFEAT), {
        "rounds": rounds,
        "actions": actions_taken,
    }


def walk_run(
    path: Path,
    known_encounters: set[str],
    aggregate: Counter,
    per_encounter: defaultdict,
    examples: defaultdict,
    *,
    seed_salt: int,
    step_cap: int,
) -> None:
    """Process one .run file: replay every combat room found in it."""
    try:
        with open(path, encoding="utf-8") as f:
            log = json.load(f)
    except (json.JSONDecodeError, OSError) as exc:
        print(f"  ! could not read {path.name}: {exc}", file=sys.stderr)
        return

    # Character: prefer the first player's id (multiplayer files have N).
    players = log.get("players") or []
    if not players:
        return
    char_raw = players[0].get("character", "")
    character = normalize_character(char_raw)

    history = log.get("map_point_history") or []
    for act_i, act in enumerate(history):
        for node_i, node in enumerate(act):
            for r in node.get("rooms") or []:
                if r.get("room_type") not in ("monster", "elite", "boss"):
                    continue
                enc_raw = r.get("model_id", "")
                enc = normalize_id(enc_raw)
                if enc not in known_encounters:
                    aggregate[STATUS_UNKNOWN_ENCOUNTER] += 1
                    per_encounter[enc][STATUS_UNKNOWN_ENCOUNTER] += 1
                    continue
                monster_ids = r.get("monster_ids") or []
                fully_dispatched = all_monsters_dispatched(monster_ids)
                # Per-combat seed: stable across runs so reruns reproduce.
                combat_seed = (
                    (seed_salt * 1_000_003)
                    ^ (act_i * 10007)
                    ^ (node_i * 257)
                    ^ hash(enc) & 0xFFFFFFFF
                ) & 0xFFFFFFFF
                status, info = replay_one(
                    enc,
                    character,
                    monster_ids,
                    seed=combat_seed,
                    step_cap=step_cap,
                )
                aggregate[status] += 1
                per_encounter[enc][status] += 1
                # Side stat: separate fully-dispatched vs partially-
                # dispatched combats so victory percentages can be
                # interpreted properly.
                key = (
                    "fully-dispatched"
                    if fully_dispatched
                    else "partial-dispatched"
                )
                aggregate[key] += 1
                per_encounter[enc][key] += 1
                if status in (
                    STATUS_ENV_BUILD_FAILED,
                    STATUS_STEP_CRASHED,
                    STATUS_NO_LEGAL_ACTIONS,
                ):
                    # Keep up to 2 example failure infos per (encounter, status).
                    bucket = examples[(enc, status)]
                    if len(bucket) < 2:
                        bucket.append({
                            "run": path.name,
                            "act": act_i,
                            "node": node_i,
                            "character": character,
                            "info": info,
                        })


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument(
        "paths",
        nargs="+",
        type=Path,
        help="One or more directories or .run files to process",
    )
    p.add_argument(
        "--step-cap",
        type=int,
        default=2000,
        help="Max step()s per combat before declaring step-cap (default: 2000)",
    )
    p.add_argument(
        "--seed",
        type=int,
        default=0,
        help="Base seed; each combat derives from this + (act, node, encounter)",
    )
    p.add_argument(
        "--show-examples",
        action="store_true",
        help="Print example failure infos for each (encounter, error) bucket",
    )
    args = p.parse_args()

    known = load_known_encounters()

    files: list[Path] = []
    for path in args.paths:
        if path.is_dir():
            files.extend(sorted(path.glob("*.run")))
        elif path.suffix == ".run":
            files.append(path)
    if not files:
        sys.exit("no .run files found in the provided paths")

    print(f"replaying {len(files)} run file(s) against {len(known)} known encounters")

    aggregate: Counter = Counter()
    per_encounter: defaultdict = defaultdict(Counter)
    examples: defaultdict = defaultdict(list)

    for f in files:
        walk_run(
            f,
            known,
            aggregate,
            per_encounter,
            examples,
            seed_salt=args.seed,
            step_cap=args.step_cap,
        )

    # `total` counts combat rooms; the dispatch keys are a parallel
    # bookkeeping pair, so they shouldn't be summed into the total.
    dispatch_keys = {"fully-dispatched", "partial-dispatched"}
    total = sum(v for k, v in aggregate.items() if k not in dispatch_keys)
    print()
    print(f"=== aggregate ({total} combat rooms) ===")
    for status in (
        STATUS_VICTORY,
        STATUS_DEFEAT,
        STATUS_STEP_CAP,
        STATUS_NO_LEGAL_ACTIONS,
        STATUS_ENV_BUILD_FAILED,
        STATUS_STEP_CRASHED,
        STATUS_UNKNOWN_ENCOUNTER,
    ):
        n = aggregate.get(status, 0)
        if n:
            pct = 100.0 * n / max(total, 1)
            print(f"  {status:24} {n:4}  ({pct:5.1f}%)")
    fd = aggregate.get("fully-dispatched", 0)
    pd = aggregate.get("partial-dispatched", 0)
    print(
        f"  dispatch coverage:       fully-dispatched={fd} "
        f"({100.0*fd/max(total,1):.1f}%)  "
        f"partial-dispatched={pd}"
    )

    print()
    print("=== per encounter (sorted by occurrences) ===")
    rows = []
    for enc, statuses in per_encounter.items():
        n = sum(
            v for k, v in statuses.items() if k not in dispatch_keys
        )
        # Status priority: surface the worst outcome first.
        bad = (
            statuses.get(STATUS_STEP_CRASHED, 0)
            + statuses.get(STATUS_ENV_BUILD_FAILED, 0)
            + statuses.get(STATUS_NO_LEGAL_ACTIONS, 0)
        )
        rows.append((n, bad, enc, statuses))
    rows.sort(key=lambda t: (-t[0], t[2]))
    for n, _bad, enc, statuses in rows:
        # Compact dispatch hint as a leading flag instead of mixing
        # it into the comma list.
        fd = statuses.get("fully-dispatched", 0)
        pd = statuses.get("partial-dispatched", 0)
        dispatch_flag = "[D]" if pd == 0 and fd > 0 else "[~]"
        bits = ", ".join(
            f"{k}={v}"
            for k, v in sorted(statuses.items())
            if v > 0 and k not in {"fully-dispatched", "partial-dispatched"}
        )
        print(f"  {dispatch_flag} {n:3}x {enc:40} {bits}")

    if args.show_examples and examples:
        print()
        print("=== failure examples ===")
        for (enc, status), infos in sorted(examples.items()):
            print(f"\n[{enc}] {status}")
            for info in infos:
                print(f"  - {info}")

    # Exit non-zero if any combats crashed (we want CI-style breakage signal).
    if aggregate.get(STATUS_STEP_CRASHED, 0) or aggregate.get(
        STATUS_ENV_BUILD_FAILED, 0
    ):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
