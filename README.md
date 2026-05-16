# Slay the Spire 2 Simulator

Headless, deterministic Rust port of Slay the Spire 2's run / map / combat
machinery, with Python bindings. Built to drive offline RL training and
post-hoc `.run`-file analysis without running the Godot client.

## Architecture: primitive vectors + composition

Every card interaction is **composition of primitive vectors at specific
stages of combat**. Cards / relics / potions / monster moves / events are
not Rust match-arms — they're `Vec<Effect>` data interpreted by
`execute_effects()`. The same shape that the C# game uses (each `OnPlay`
body is a sequence of `DamageCmd.Attack(...)`, `CreatureCmd.GainBlock(...)`,
`PowerCmd.Apply<T>(...)`).

```rust
"Bash" => Some(vec![
    Effect::DealDamage  { amount: AmountSpec::Canonical("Damage"),     target: ChosenEnemy, hits: 1 },
    Effect::ApplyPower  { power_id: "VulnerablePower", amount: AmountSpec::Canonical("Vulnerable"), target: ChosenEnemy },
]),
```

**Composition layers** (innermost → outermost; each tested in `composition_architecture.rs`):

1. **Base primitives** — `card_effects(id)` is a pure function returning the
   static `Vec<Effect>`.
2. **Upgrade delta** — `AmountSpec::Canonical(key)` resolves to
   `base_value + upgrade_level × delta`. The Effect vector is identical
   across upgrade levels; only resolved numbers change.
3. **Enchantment** (three sub-layers):
   - **3a** Damage/block modifier pipeline (Sharp, Corrupted, Nimble,
     Vigorous, Momentum).
   - **3b** OnPlay hooks — fire after the card's own body (Sown, Swift,
     Adroit, Inky).
   - **3c** Per-instance state mutation (`EnchantmentInstance.state`).
     Ramps like Momentum.ExtraDamage and Goopy.StackCount accumulate here.
     Critical invariant: **duplicates get a FRESH state map AND fresh
     `consumed_this_combat`** — Anger / DualWield / CloneSourceCardToPile
     all preserve the enchantment with reset per-instance flags, so
     once-per-combat triggers fire on each replica independently.
4. **Cost overrides** — priority chain
   `until_played > this_turn > this_combat > base`. Set by Discovery /
   SneckoOil / TouchOfInsanity / Slither.
5. **Per-card combat state** — `CardInstance.state: HashMap<String, i32>`
   for ramp counters (Maul, Claw, GeneticAlgorithm). Composed via
   `AmountSpec::SourceCardCounter`.

**Observer-layer constraint (load-bearing for RL):** the feature extractor
keys off the same effect-list data — never card-id lookups. A balance patch
that changes `Damage` from 6 to 7 changes the feature vector but does not
require retraining; novel cards composed of known primitives generalize.

## Status

**Card parity: 529/529 PASS (100%) · Relic parity: 286/286 PASS (100%).**
Diffed against the C# decompile via an oracle host that loads `sts2.dll`
reflectively.

| Test suite | Count | Status |
|---|---|---|
| `sts2-sim` unit tests | 774 | ✅ |
| Card parity sweep (Ironclad vs 2× BigDummy) | 529 | ✅ 100% |
| Relic parity sweep | 286 | ✅ 100% |
| MadScience 9-variant (TinkerTimeType × Rider) | 9 | ✅ 9/9 |
| Choice vs RNG semantics | 6 | ✅ |
| Audit: no-combat-effect relics (158 relics) + loose comparisons | 44 | ✅ |
| Enchantment audit incl. duplication invariant | 11 | ✅ |
| Potion audit | 8 | ✅ |
| Composition-architecture audit | 10 | ✅ |
| **Total** | **1177** | **100% PASS** |

**RNG, map, shuffle, acts** — bit-exact vs C# DLL via oracle (~36 tests).

**Combat behavior** — diffed against C# combat state after `combat_play_card`
RPC. Per-card RNG drift is tolerated via opt-in loose comparison; the
audit suite locks in expected behavior for every loose-compared item, so
the relaxation can't hide a real regression.

**Data-table coverage**: 529 cards, 286 relics (combat-side), 56 relics
(run-state-side), 63 potions, 189 monster intents, 23 enchantments
(13 effectively wired), 30+ powers wired in modifier pipelines + Power VM.

### Choice infrastructure (RL-relevant)

`CombatState.auto_resolve_choices: bool` distinguishes RNG primitives
(`Selector::Random`, auto-resolved) from player choices (`Effect::AwaitPlayerChoice`,
pauses combat and emits a `pending_choice` for the agent). Canonical
example: TrueGrit unupgraded → `ExhaustRandomInHand` (RNG); TrueGrit+ →
`AwaitPlayerChoice` (CHOICE). `resolve_pending_choice(picks)` validates +
applies. Auto-resolve defaults to `true` for parity sweeps and replay.

## Layout

```
sim/
├── crates/
│   ├── sts2-sim/                  Core: types + data tables + behavior.
│   │   └── src/effects.rs           Effect VM (~80 Effect variants,
│   │                                ~30 AmountSpec variants,
│   │                                ~25 Condition variants),
│   │                                card_effects / power_effects /
│   │                                relic_effects / potion_effects /
│   │                                run_state_effects / monster_move_effects.
│   ├── sts2-sim-py/               PyO3 bindings (PyCombatEnv, features).
│   └── sts2-sim-oracle-tests/     Bit-exact diff tests + audit suite.
├── oracle-host/                   C# console app: loads sts2.dll reflectively,
│                                  exposes game functions over stdio JSON-RPC.
├── tools/
│   ├── extract_*/                 Scrape decompile → JSON tables.
│   ├── merge_card_ports/          autogen + inject for effect-list ports.
│   ├── combat_smoke/              Random-policy Python harness.
│   ├── run_replay/                .run-corpus replay + coverage report.
│   └── run_analyzer/              .run → per-floor reconstruction.
└── docs/
    ├── effect-vocabulary.md       Closed primitive vocabulary.
    └── rng_streams.md             12-stream RngSet notes.
```

## Build

```powershell
# Rust
cargo check
cargo test                                          # 774 unit tests

# Oracle host + parity sweeps (needs sts2.dll)
dotnet build oracle-host -c Release
cargo test -p sts2-sim-oracle-tests -- --ignored    # parity sweeps + audit

# Python bindings
cd crates/sts2-sim-py && maturin build --release
pip install --force-reinstall target/wheels/sts2_sim_py-*.whl
```

Oracle expects `sts2.dll` at
`G:\SteamLibrary\steamapps\common\Slay the Spire 2\data_sts2_windows_x86_64\sts2.dll`;
override with `STS2_GAME_DIR`.

## Running combat from Python

```python
import json, random, sts2_sim_py

env = sts2_sim_py.PyCombatEnv(seed=42, character="Ironclad", encounter="AxebotsNormal")
rng = random.Random(42)
while not env.is_terminal():
    legal = json.loads(env.legal_actions())
    env.step(json.dumps(rng.choice(legal)))
print(json.loads(env.observation()))
```

`tools/run_replay/replay.py` runs this against every combat room in a
`.run` corpus and aggregates `victory` / `defeat` / `step-cap` /
`dispatch coverage`.

## What's next

Primitives the remaining gaps need (ordered by leverage):

1. **`EnchantPlayCount` hook chain** — unblocks Glam, Spiral
   (play-count modifier ×N).
2. **Per-card permanent field mutation** (`ModifyMasterDeckField`) —
   unblocks SoldiersStew (`BaseReplayCount++` on Strike-tagged cards).
3. **Choice continuation** (carry pick-count from `AwaitPlayerChoice`
   to a follow-up effect) — unblocks GamblersBrew (discard-then-
   draw-same-N).
4. **Enchantment lifecycle hooks** (`AfterCardDrawn`, `BeforeFlush`,
   `BeforePlayPhaseStart`, `ModifyShuffleOrder`, `AfterCardPlayed`) —
   unblocks Slither, SlumberingEssence, Imbued, PerfectFit, Goopy's
   AfterCardPlayed counter.
5. **Potion canonical resolver** — `AmountSpec::Canonical(key)` only
   looks up `CardData.canonical_vars`; potion bodies hardcode literals
   today. Threading `PotionData.canonical_vars` would let 60+ potion
   bodies use `Canonical` symmetrically with cards.
6. **Modifier-hook layer** (`ModifyHandDraw` chain, `ModifyMaxEnergy`,
   `ModifyDamage*`, `TryModifyRewards*`, `TryModifyRestSiteOptions`) —
   threads relic-driven value modifications into the existing pipelines
   beyond the round-1 hand-draw special case already landed.
7. **Hook dispatcher (#70)** — needs IL re-decompile of
   `IterateHookListeners.MoveNext` (compiler-generated state machine
   stripped from current decompile).
8. **Power VM expansion** — port hardcoded power behavior (Strength /
   Dex / Weak / Vulnerable / Frail / Poison / DemonForm / Ritual /
   Barricade) to `power_effects` data table.
9. **Forge runtime** — primitives exist (`Effect::Forge`); resolving
   `pending_forge` into a card-upgrade choice surface is pending.

See `tools/coverage_audit.txt` for per-id gap status.
