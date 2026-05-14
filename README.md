# Slay the Spire 2 Simulator

Headless, deterministic Rust port of Slay the Spire 2's run / map / combat
machinery, with Python bindings. Built to drive offline RL training and
post-hoc `.run`-file analysis without running the Godot client.

## Status

Phase 0 (simulator port) is most of the way through. Concrete state today:

- **61 monster types** with full intent state machines + dispatcher
- **70+ card OnPlay** handlers across all 5 character pools
- **30+ powers** with their hooks wired (Strength/Vulnerable/Weak/Frail/Dex,
  Poison, Intangible, Barricade, Burrowed, Plating, Slumber, Asleep, Curl Up,
  Skittish, Vital Spark, Hardened Shell, Vigor, Shriek, Flutter, Soar, Shrink,
  Rampart, Territorial, Ritual, Imbalanced, Paper Cuts, Thorns, Hard To Kill,
  Escape Artist, Doom, Demon Form, Setup Strike / temp-Strength family, …)
- **Relic combat hooks**: BeforeCombatStart, AfterCombatVictory,
  AfterSideTurnStart (Anchor, Burning Blood, Brimstone)
- **702 spec-derived unit tests** passing
- **Replay harness**: 90.5% of corpus combat rooms run end-to-end with the
  enemy turn dispatcher exchanging real damage. Remaining gaps are mostly
  bosses needing summon system, multi-phase HP, or per-card affliction state.

### Verification posture

| Subsystem | Validation | Status |
|---|---|---|
| `hash`, `rng`, `rng_set`, `shuffle`, `path_pruning`, all 5 acts, `StandardActMap` | Bit-exact diff vs `sts2.dll` via the oracle host | ✅ ~36 oracle-diff tests green |
| Combat behavior: cards, monsters, powers, relics, enchantments | Spec-derived unit tests + `.run`-corpus crash-free replay | 🟡 hand-rolled; oracle diff pending |

Combat behavior tests are spec-derived from the C# decompile — they're the
floor, not equivalence proof. Every commit message for behavior ports
includes the line `NOTE: spec-derived tests only; not yet oracle-diffed`
to keep the bar honest. Reopening the combat oracle is the multi-day work
on the C# side ([discussed under "Verification posture for combat"][resume]).

[resume]: https://github.com/Akirakato1/Slay-the-Spire-2-Simulator/blob/main/README.md#verification-posture

## Layout

```
sim/
├── crates/
│   ├── sts2-sim/                  Rust core: types + data tables + behavior.
│   ├── sts2-sim-py/               PyO3 bindings (PyCombatEnv, observation,
│   │                              card/relic feature extractors).
│   └── sts2-sim-oracle-tests/     Bit-exact diff tests vs the C# oracle host
│                                  for the deterministic subsystems.
├── oracle-host/                   C# console app: loads `sts2.dll`
│                                  reflectively, exposes game functions over
│                                  stdio JSON-RPC.
├── tools/
│   ├── extract_*/                 One-shot tools that scrape the decompile
│   │                              for cards, relics, powers, encounters,
│   │                              monsters, … and emit JSON into
│   │                              `crates/sts2-sim/data/`.
│   ├── combat_smoke/              Python random-policy harness against
│   │                              `PyCombatEnv` — sanity check the
│   │                              Rust↔Python boundary.
│   ├── run_replay/                `.run`-corpus harness (`replay.py`) +
│   │                              encounter coverage report (`coverage.py`).
│   └── run_analyzer/              CLI that walks a `.run` and reconstructs
│                                  per-floor state via the simulator.
└── docs/                          Notes on RNG streams, etc.
```

### Data tables

`crates/sts2-sim/data/` holds JSON tables extracted by `tools/extract_*`:

- `cards.json` — 577 cards (id / pool / cost / canonical vars / upgrade deltas)
- `relics.json` — 294 relics
- `powers.json` — 256 powers (type / stack_type / allow_negative)
- `monsters.json` — 121 monsters (HP ranges, walks inheritance chains)
- `encounters.json` — 88 encounters (canonical spawns + possible monsters)
- `characters.json`, `events.json`, `potions.json`, `orbs.json`,
  `afflictions.json`, `enchantments.json`, `modifiers.json` — supporting tables

Re-extract any table with `cargo run -p extract-<thing>`.

## Build

```powershell
# Rust workspace
cargo check
cargo test                       # ~700 unit + integration tests

# C# oracle host (only needed for the bit-exact diff tests)
dotnet build oracle-host -c Release
cargo test -p sts2-sim-oracle-tests -- --include-ignored

# Python bindings (PyO3 via maturin)
cd crates/sts2-sim-py
maturin build --release --interpreter python
pip install --force-reinstall target/wheels/sts2_sim_py-*.whl
```

## Running combat from Python

```python
import json, random, sts2_sim_py

env = sts2_sim_py.PyCombatEnv(
    seed=42,
    character="Ironclad",
    encounter="AxebotsNormal",
)
rng = random.Random(42)
while not env.is_terminal():
    legal = json.loads(env.legal_actions())
    action = rng.choice(legal)
    env.step(json.dumps(action))
print(json.loads(env.observation()))
```

The harness in `tools/run_replay/replay.py` does the same against every
combat room in a `.run` corpus and aggregates pass / fail / dispatch
coverage.

## `.run` replay harness

```powershell
python tools/run_replay/replay.py "C:\path\to\sample runs"
```

Walks each combat room, looks up the encounter in `encounters.json`,
spins up `PyCombatEnv` with the recorded `monster_ids`, and runs a random
policy until terminal. Aggregates:

- `victory` / `defeat` / `step-cap` / `env-build-failed` / `step-crashed`
- `dispatch coverage` — fraction of rooms where every enemy has a Rust
  dispatcher (vs. silent no-op for unported monsters)

Random-policy win rate is **not** a quality signal — it's a regression
canary. The honest signals are `crash count` (currently ~0% across 168
corpus rooms) and `dispatch coverage` (currently 90%+).

## Oracle prerequisites

The bit-exact diff tests need the real game DLL. By default the oracle
host looks for:

```
G:\SteamLibrary\steamapps\common\Slay the Spire 2\data_sts2_windows_x86_64\sts2.dll
```

Override with the `STS2_GAME_DIR` environment variable.

## Verification posture

- **RNG / map / shuffle / acts** are bit-exact against the C# DLL,
  validated over a randomized input distribution by the oracle test suite.
- **Combat behavior** is currently spec-derived: I read the C#
  decompile and write Rust + a unit test for each piece. These tests catch
  obvious bugs but don't prove equivalence. A drift (off-by-one rounding,
  wrong order of multiplicative modifiers, a missed edge case) wouldn't be
  caught.

The path to closing that gap is a headless `CombatState` endpoint in the
oracle host — currently deferred because `CombatState.CreateCreature`
reaches into `RunState`/`MapCoord`/`SaveManager` and `OnPlay` bodies call
`Cmd.*` UI handlers. Reopening it is a multi-day C# port.

Until that lands, every behavior commit message carries the line
`NOTE: spec-derived tests only; not yet oracle-diffed`.
