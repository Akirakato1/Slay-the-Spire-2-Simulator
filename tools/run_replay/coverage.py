#!/usr/bin/env python3
"""Coverage / readiness report for .run-corpus combat-replay (#72).

For each .run file given (or every .run in given directories), walk
the recorded combat rooms and check whether the simulator has:

  - the encounter id in its extracted encounter table
  - every monster id in its extracted monster table
  - a Rust intent state machine for every monster

Prints a sorted-by-frequency table that tells us which monster /
encounter to port next based on real-corpus appearance.

No actual combat simulation yet — that lands once monster coverage is
broad enough that a random-policy replay can reach the end of an act
without hitting Unhandled walls.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from collections import Counter
from pathlib import Path

# Monsters with a fully ported intent state machine in combat.rs.
# Keep in sync with the MonsterIntent enums and execute_*_move fns.
PORTED_INTENT_MACHINES = {
    "Axebot",
    "Myte",
    "Nibbit",
    "FlailKnight",
    "BowlbugEgg",
    "BowlbugNectar",
    "BowlbugSilk",
    "ScrollOfBiting",
    "CorpseSlug",
    "Seapunk",
    "TwigSlimeS",
    "LeafSlimeS",
    "TwigSlimeM",
    "LeafSlimeM",
    "TurretOperator",
    "Chomper",
}


def load_known_ids() -> tuple[set[str], set[str]]:
    """Load the simulator's encounter and monster id tables from the
    JSON data files. Returns (encounter_ids, monster_ids)."""
    sim_data = Path(__file__).resolve().parents[2] / "crates" / "sts2-sim" / "data"
    with open(sim_data / "encounters.json", encoding="utf-8") as f:
        enc = {c["id"] for c in json.load(f)}
    with open(sim_data / "monsters.json", encoding="utf-8") as f:
        mon = {c["id"] for c in json.load(f)}
    return enc, mon


def normalize_id(prefixed: str) -> str:
    """Convert "ENCOUNTER.NIBBITS_WEAK" → "NibbitsWeak".

    .run files use prefixed SCREAMING_SNAKE_CASE. The simulator's
    extracted tables use C# class-name PascalCase. We strip the prefix,
    split on `_`, and uppercase the first letter of each chunk.
    """
    _, _, raw = prefixed.partition(".")
    if not raw:
        raw = prefixed
    return "".join(part[:1].upper() + part[1:].lower() for part in raw.split("_"))


def walk_run(
    path: Path,
    enc_counts: Counter,
    mon_counts: Counter,
    enc_to_monsters: dict[str, set[str]],
) -> int:
    """Process one .run file, updating shared counters. Returns the
    number of combat rooms found."""
    try:
        with open(path, encoding="utf-8") as f:
            log = json.load(f)
    except Exception as e:
        print(f"  skip {path.name}: {e}", file=sys.stderr)
        return 0
    n_combats = 0
    for act in log.get("map_point_history", []) or []:
        for node in act:
            for room in node.get("rooms", []) or []:
                if room.get("room_type") not in ("monster", "elite", "boss"):
                    continue
                enc_id = normalize_id(room.get("model_id", ""))
                if not enc_id:
                    continue
                n_combats += 1
                enc_counts[enc_id] += 1
                mons = [normalize_id(m) for m in room.get("monster_ids") or []]
                enc_to_monsters.setdefault(enc_id, set()).update(mons)
                for m in mons:
                    mon_counts[m] += 1
    return n_combats


def collect_run_paths(paths: list[str]) -> list[Path]:
    out: list[Path] = []
    for p in paths:
        pp = Path(p)
        if pp.is_dir():
            out.extend(sorted(pp.glob("*.run")))
        elif pp.is_file():
            out.append(pp)
        else:
            print(f"  skip: not a file or dir: {p}", file=sys.stderr)
    return out


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "paths",
        nargs="+",
        help=".run files or directories containing them",
    )
    args = parser.parse_args()

    known_encounters, known_monsters = load_known_ids()
    paths = collect_run_paths(args.paths)
    if not paths:
        sys.exit("no .run files found")

    enc_counts: Counter = Counter()
    mon_counts: Counter = Counter()
    enc_to_monsters: dict[str, set[str]] = {}
    total_combats = 0
    for p in paths:
        total_combats += walk_run(p, enc_counts, mon_counts, enc_to_monsters)

    # Per-encounter readiness
    print(f"=== Encounter coverage across {len(paths)} runs "
          f"({total_combats} combat rooms total) ===")
    print()

    ready_encs = 0
    rows: list[tuple[int, str, str]] = []
    for enc, count in enc_counts.most_common():
        reasons: list[str] = []
        if enc not in known_encounters:
            reasons.append("missing-encounter")
        mons = enc_to_monsters.get(enc, set())
        for m in sorted(mons):
            if m not in known_monsters:
                reasons.append(f"missing-monster:{m}")
            elif m not in PORTED_INTENT_MACHINES:
                reasons.append(f"missing-intent:{m}")
        if not reasons:
            ready_encs += 1
            status = "READY"
        else:
            status = " | ".join(reasons)
        rows.append((count, enc, status))

    print(f"  fully ready: {ready_encs} / {len(enc_counts)} "
          f"({100*ready_encs//max(len(enc_counts),1)}%)")
    print()
    print("  by encounter, sorted by occurrences:")
    for count, enc, status in rows:
        print(f"    {count:3}x {enc:40} [{status}]")

    # Per-monster summary
    print()
    print("=== Monster occurrences (across all combat rooms) ===")
    print()
    ready_mons = 0
    for mon, count in mon_counts.most_common():
        if mon not in known_monsters:
            tag = "missing-from-extracted-table"
        elif mon not in PORTED_INTENT_MACHINES:
            tag = "missing-intent-machine"
        else:
            tag = "READY"
            ready_mons += 1
        print(f"   {count:3}x {mon:40} [{tag}]")
    print()
    print(f"  monsters fully ready: {ready_mons} / {len(mon_counts)} "
          f"({100*ready_mons//max(len(mon_counts),1)}%)")


if __name__ == "__main__":
    main()
