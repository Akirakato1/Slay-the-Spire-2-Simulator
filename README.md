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
| `sts2-sim` unit tests | 958 | ✅ |
| Game-flow integration (Neow → combat → reward, multi-character) | 10 | ✅ |
| Mapgen parity (vs dashboard JS reference) | 1 | ✅ |
| Card parity sweep (Ironclad vs 2× BigDummy) | 529 | ✅ 100% |
| Relic parity sweep | 286 | ✅ 100% |
| MadScience 9-variant (TinkerTimeType × Rider) | 9 | ✅ 9/9 |
| Choice vs RNG semantics | 6 | ✅ |
| Audit: no-combat-effect relics (158 relics) + loose comparisons | 44 | ✅ |
| Enchantment audit (all 22 non-deprecated wired) | 24 | ✅ |
| Potion audit (incl. SoldiersStew replay-count) | 9 | ✅ |
| Composition-architecture audit | 10 | ✅ |
| **Total** | **1386** | **100% PASS** |

**RNG, map, shuffle, acts** — bit-exact vs C# DLL via oracle (~36 tests).

**Combat behavior** — diffed against C# combat state after `combat_play_card`
RPC. Per-card RNG drift is tolerated via opt-in loose comparison; the
audit suite locks in expected behavior for every loose-compared item, so
the relaxation can't hide a real regression.

**Data-table coverage**: 529 cards, 286 relics (combat-side), 56 relics
(run-state-side), 63 potions, **120/120 monsters with data-driven AI**
(`MovePattern` over `MonsterMove`+`Effect` primitives — `Cycle`,
`WeightedRandom`, `FirstTurnOverride`, `BySlot`, `HpThresholdSwitch`,
`Conditional`), **88 encounters with per-act pool assignment + IsWeak
+ tags**, **59 events with per-act pools**, **22/23 enchantments wired**
(all non-deprecated — modifier pipeline + OnPlay + EnchantPlayCount
loop + per-instance state + AfterCardDrawn + BeforeFlush +
BeforePlayPhaseStart + ModifyShuffleOrder hooks), 30+ powers wired in
modifier pipelines + Power VM.

### End-to-end run flow

`RunState::start_run` → `enter_act` builds map + per-act `RoomSet`
(pre-shuffled weak / regular / elite / event pools with tag-avoidance,
pre-selected boss) → cursor navigation through map nodes →
`pick_encounter_for_current_node` modulo-cycles the appropriate pool →
`build_combat_state` pipes the run's ascension + player loadout into
combat → `auto_play_combat` drives the env → `extract_outcome` +
`apply_combat_outcome` fold rewards back to RunState.

**Room-generation rules baked in:** weak vs regular hallway split (first
3 hallway fights = weak pool), per-act encounter pools, tag-based no-
repeat (`AddWithoutRepeatingTags`), modulo cycle on pool exhaustion,
boss pre-selected at act gen, 15 pre-generated elites, per-act event
pools (act-specific + 18 shared) with visited-event tracking, and `?`-
room resolution (10% Monster / 2% Treasure / 3% Shop / ~85% Event with
odds-bump for unrolled types).

### Ascension

`CombatState.ascension` is piped from `RunState.ascension`. `Creature::
from_monster_spawn_at` reads `min_hp_ascended` / `max_hp_ascended` at
A1+ (ToughEnemies threshold). `AmountSpec::AscensionScaled { base,
ascended, threshold }` mirrors C# `GetValueIfAscension` for damage and
similar scaled values; `MonsterMove::attack_a` is the convenience
builder. Bulk-port of per-move damage values from C# `GetValueIfAscension`
getters is in progress (~100 monster classes). Reward modifiers
(Poverty), run-state init scaling, and event-pool filtering are
deferred.

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
cargo test                                          # 958 unit + 11 integration

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

## Sandbox UI

`crates/sts2-sim-ui` (binary `sts2-ui`) is an egui-based interactive
sandbox for testing combat behavior. Build a deck with any cards / relics
/ potions / enchantments, pick any subset of the 120 monsters as enemies
(defaults to 2× BigDummy), and play through real combat — every enemy
runs its AI intent through `monster_dispatch::dispatch_enemy_turn`. Useful
for hand-validating card behavior, monster move patterns, and unusual
interactions before committing to a parity test.

```powershell
cargo run -p sts2-sim-ui --release
```

## What's next

**Combat side feature-complete** for cards / relics / enchantments /
potions / monster AI. **Run side feature-complete** for map generation,
room pools, ?-resolution, encounter selection, event pools. Open work:

1. **Ascension bulk-port** — extract per-move ascended damage from C#
   `GetValueIfAscension` getters into `MonsterMove::attack_a` calls
   across all 120 monsters (infrastructure landed; ~100 classes to
   bulk-process).
2. **Ascension reward / run-state / event modifiers** — Poverty 0.75×
   gold, WearyTraveler HP reduction, NoBeneficialEvents pool filter.
3. **Reward-offer primitives** (`Effect::OfferCardReward` /
   `OfferRelicReward` / `OfferPotionReward`) — foundation for treasure
   rooms, post-combat rewards, and event branches.
4. **Power-VM expansion** (refactor, not correctness): migrate
   hardcoded power behavior to the `power_effects` data table.
5. **Modifier-hook layer** (refactor): generalize `ModifyHandDraw`,
   `ModifyMaxEnergy`, `ModifyDamage*` chains.
6. **Hook dispatcher iteration order** (correctness-adjacent):
   canonical order from C# `IterateHookListeners.MoveNext` — compiler-
   stripped from decompile; current order works in practice but isn't
   formally validated.

See `tools/coverage_audit.txt` for per-id gap status.
