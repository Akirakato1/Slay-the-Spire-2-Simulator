# Slay the Spire 2 Simulator

Headless, deterministic Rust port of Slay the Spire 2's run / map / combat
machinery, with Python bindings. Built to drive offline RL training and
post-hoc `.run`-file analysis without running the Godot client.

## Status

Phase 0 (simulator port) is well underway. Concrete state today:

- **Effect VM** (`crates/sts2-sim/src/effects.rs`) — closed primitive
  vocabulary (~50 `Effect` variants). Cards / relics / potions / monster
  moves / events all encodable as `Vec<Effect>` over the same vocabulary.
  Includes control flow (`Conditional`, `Repeat`), `Condition` AST for
  predicates, `AmountSpec` for amount sources (Canonical / XEnergy /
  OwnerPowerAmount / BranchedOnUpgrade / Multiplied), `Target` enum
  (SelfPlayer / SelfActor / ChosenEnemy / AllEnemies / RandomEnemy),
  `Selector` for card-ref operations (All / Random / Top / Bottom /
  FirstMatching / PlayerInteractive).
- **Power VM** scaffold — same composition pattern applied to power
  lifecycle. `PowerHook::AfterTurnEnd { filter, body }` with
  `power_effects` registry mirroring `card_effects`. RegenPower is the
  first migration (heal + decrement at owner turn end as effect list).
- **326 / 577 cards** data-driven via effect lists (24 hand-migrated +
  302 auto-encoded from `cards.json` by `tools/merge_card_ports/autogen.py`).
  Remaining ~250 are mostly cards needing primitives that the
  vocabulary doesn't yet cover (orb subsystem, calc-var amount specs
  like SelfBlock / CardCountInPile / TargetPowerAmount, Osty companion,
  Forge mechanic) or have richer match-arm bodies the auto-shape would
  oversimplify. Each skip is annotated with a one-line reason.
- **63 monster types** with full intent state machines + dispatcher
- **30+ powers** with their hooks wired (Strength/Vulnerable/Weak/Frail/Dex,
  Poison, Intangible, Barricade, Burrowed, Plating, Slumber, Asleep, Curl Up,
  Skittish, Vital Spark, Hardened Shell, Vigor, Shriek, Flutter, Soar, Shrink,
  Rampart, Territorial, Ritual, Imbalanced, Paper Cuts, Thorns, Hard To Kill,
  Escape Artist, Doom, Demon Form, Setup Strike / temp-Strength family,
  Regen via Power VM, …)
- **Relic combat hooks**: BeforeCombatStart, AfterCombatVictory,
  AfterSideTurnStart (Anchor, Burning Blood, Brimstone)
- **744 spec-derived unit tests** passing
- **Replay harness**: 90%+ of corpus combat rooms run end-to-end with the
  enemy turn dispatcher exchanging real damage. Remaining gaps are mostly
  bosses needing summon system, multi-phase HP, or per-card affliction state.

### C#-fidelity audit (2026-05-14)

Three parallel audits cross-referenced the Rust pipeline against the
C# decompile (`Hook.cs`, `AttackCommand.cs`, `CardModel.OnPlayWrapper`,
sample `Models/Cards/`, `Models/Powers/`, `Models/Events/`). 12
discrepancies were found and tracked in
`project_pipeline_audit_2026_05_14.md` (memory). Seven have landed:

- ✅ `WasTargetKilled` → transition predicate (was post-state, would
  re-trigger Feed/HandOfGreed kill bonuses on already-dead corpses)
- ✅ `SkipNextDurationTick` on player-applied debuffs
- ✅ `BeforeAttack` / `AfterAttack` envelope + Vigor per-attack timing
- ✅ Before/AfterPowerAmountChanged hook stubs
- ✅ Enchantment threaded through `modify_block` (Nimble)
- ✅ Dead-dealer short-circuit on `deal_damage`
- ✅ `amount == 0` short-circuits `apply_power`

Remaining ones are LOW severity (Thorns timing, single-stage Intangible
cap, dead-creature hook filtering) or block on structural work (full
hook dispatcher, monster-move VM routing, IL re-decompile of
`IterateHookListeners.MoveNext`).

### Verification posture

| Subsystem | Validation | Status |
|---|---|---|
| `hash`, `rng`, `rng_set`, `shuffle`, `path_pruning`, all 5 acts, `StandardActMap` | Bit-exact diff vs `sts2.dll` via the oracle host | ✅ ~36 oracle-diff tests green |
| Combat behavior: cards, monsters, powers, relics, enchantments | Spec-derived unit tests + `.run`-corpus crash-free replay + C# audit cross-reference | 🟡 hand-rolled + 7 audit fixes landed; oracle diff pending |

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
│   │   └── src/effects.rs           Effect VM: enum Effect / AmountSpec /
│   │                                Condition / Target / Selector / Pile,
│   │                                card_effects() + power_effects()
│   │                                registries, execute_effects dispatcher,
│   │                                fire_power_hooks_after_turn_end.
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
│   ├── merge_card_ports/          autogen.py reads cards.json and emits
│   │                              card_effects() match arms by shape-
│   │                              matching on (card_type, target_type,
│   │                              canonical_vars); inject.py merges the
│   │                              output into effects.rs idempotently.
│   ├── combat_smoke/              Python random-policy harness against
│   │                              `PyCombatEnv` — sanity check the
│   │                              Rust↔Python boundary.
│   ├── run_replay/                `.run`-corpus harness (`replay.py`) +
│   │                              encounter coverage report (`coverage.py`).
│   └── run_analyzer/              CLI that walks a `.run` and reconstructs
│                                  per-floor state via the simulator.
└── docs/
    ├── effect-vocabulary.md       Closed primitive vocabulary from C# survey:
    │                              cards (577) / relics (294) / potions (64) /
    │                              monster moves / events / power lifecycle.
    │                              Pareto frontier, status table, audit findings.
    └── rng_streams.md             Notes on the 12 RNG streams.
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
cargo test                       # 744 unit + integration tests

# C# oracle host (only needed for the bit-exact diff tests)
dotnet build oracle-host -c Release
cargo test -p sts2-sim-oracle-tests -- --include-ignored

# Python bindings (PyO3 via maturin)
cd crates/sts2-sim-py
maturin build --release --interpreter python
pip install --force-reinstall target/wheels/sts2_sim_py-*.whl

# Regenerate the bulk-ported card_effects() registry from cards.json
python tools/merge_card_ports/autogen.py > tools/merge_card_ports/autogen_out.rs
python tools/merge_card_ports/inject.py
```

## Effect VM (the central architecture)

The simulator is structured as a **closed primitive vocabulary plus
data composition**. Cards / relics / potions / monster moves / events
are not Rust match-arms — they're `Vec<Effect>` data interpreted by
`execute_effects()`. This mirrors the C# game's structure (each
`OnPlay` body is a thin sequence of `DamageCmd.Attack(...)`,
`CreatureCmd.GainBlock(...)`, `PowerCmd.Apply<T>(...)`, etc.) and
gives the RL agent observer layer a stable feature schema that
generalizes across patches and new content.

```rust
// effects.rs — sample
pub enum Effect {
    DealDamage { amount: AmountSpec, target: Target, hits: i32 },
    GainBlock { amount: AmountSpec, target: Target },
    ApplyPower { power_id: String, amount: AmountSpec, target: Target },
    DrawCards { amount: AmountSpec },
    AddCardToPile { card_id: String, upgrade: i32, pile: Pile },
    Conditional { condition: Condition, then_branch: Vec<Effect>, else_branch: Vec<Effect> },
    Repeat { count: AmountSpec, body: Vec<Effect> },
    // ... ~50 variants total
}
```

A card like Bash becomes:

```rust
"Bash" => Some(vec![
    Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()),
        target: Target::ChosenEnemy, hits: 1 },
    Effect::ApplyPower { power_id: "VulnerablePower".to_string(),
        amount: AmountSpec::Canonical("Vulnerable".to_string()),
        target: Target::ChosenEnemy },
]),
```

Adding a new card whose primitives are already wired is a data-only
edit. See `docs/effect-vocabulary.md` for the full primitive catalog.

**Observer-layer constraint (load-bearing for RL)**: the feature
extractor in `features.rs` keys off the same effect-list data — never
card-id lookups. This means a balance patch that changes `Damage` from
6 to 7 changes the feature vector but does not require retraining;
novel cards composed of known primitives generalize from training
distribution.

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
- **C# cross-reference audit** (2026-05-14, see audit findings above):
  three parallel agents diffed the Rust pipeline against `Hook.cs`,
  `AttackCommand.cs`, sample `OnPlay`/power/event bodies. 12 discrepancies
  identified; 7 landed as commits. Audit memo lives in
  `project_pipeline_audit_2026_05_14.md`.

The path to closing the equivalence gap is a headless `CombatState`
endpoint in the oracle host — currently deferred because
`CombatState.CreateCreature` reaches into `RunState`/`MapCoord`/
`SaveManager` and `OnPlay` bodies call `Cmd.*` UI handlers. Reopening it
is a multi-day C# port.

Until that lands, every behavior commit message carries the line
`NOTE: spec-derived tests only; not yet oracle-diffed`.

## What's next

Roadmap of unfinished work (see `project_pipeline_audit_2026_05_14.md`
for the prioritized list):

1. **Calc-var AmountSpec extensions** — add `SelfBlock`,
   `CardCountInPile { pile, filter }`, `TargetPowerAmount { power_id }`,
   `HistoryCount { event, scope }`. Unblocks ~40 currently-skipped
   cards (BodySlam, PerfectedStrike, Conflagration, Bully, MindBlast,
   Mirage, etc.).
2. **Orb subsystem** — `PlayerState.orb_queue: Vec<OrbInstance>` plus
   per-orb passive/evoke effects (LightningOrb damage, FrostOrb block,
   DarkOrb damage-on-evoke, PlasmaOrb energy). Unblocks ~30 Defect
   cards.
3. **Power VM expansion** — port the next 10 powers (Strength, Dex,
   Weak, Vulnerable, Frail, Poison, DemonForm, Ritual, Barricade as
   `power_effects` entries; remove their hardcoded behavior from
   `combat.rs`).
4. **Hook dispatcher (#70)** — needs IL re-decompile of
   `IterateHookListeners.MoveNext` (compiler-generated state machine
   was stripped from current decompile).
5. **Monster move VM routing** — migrate `monster_dispatch.rs` match
   arms to data-driven effect lists with `Target::SelfActor`.
6. **Run-state dispatcher** — events / relic-pickup effects encode but
   don't fire (no `&mut RunState` handle in combat VM).
7. **Osty companion + Forge** — separate companion creature with HP/
   intent + smith-forge mechanic (smaller leverage; defer).
