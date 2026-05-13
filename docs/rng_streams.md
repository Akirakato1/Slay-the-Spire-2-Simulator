# RNG architecture

Reverse-engineered from `MegaCrit/sts2/Core/Random` and consumer sites. This
document is the source of truth for what RNG state the simulator must carry
and how it must be derived from the run seed.

## The big picture

The game uses **deterministic sub-seeding**: a single string seed becomes a
`uint`, that `uint` is mixed with stream identifiers (other strings) to
produce per-stream seeds, and each stream is its own `Rng` instance with its
own state. Saving a run records the *counter* of each stream; restoring
re-derives each stream's seed and fast-forwards its counter.

This means a saved-game checkpoint is just `(run seed, per-stream counters)`.
The Rust simulator's `RunState` will follow the same model.

## Layers of RNG state

| Layer | Class | Streams | Lifetime |
|---|---|---|---|
| Run-global | `RunRngSet` | 12 named streams (`RunRngType`) | Whole run |
| Per-player | `PlayerRngSet` | 3 named streams (`PlayerRngType`) | Whole run, per player |
| Per-act-map | ad-hoc `new Rng(seed, "spoils_map")` / `StandardActMap` | 1 each | One act |
| Per-encounter | `EncounterModel._rng` | 1 | One combat |
| Per-monster | `Creature.Rng` (set in `CombatState`) | 1 each | One combat |
| Per-event | `EventModel.Rng` | 1 | One event resolution |
| Per-relic | ad-hoc inside specific relics (e.g. `FurCoat`, `Byrdpip`, `PaelsLegion`) | 1 each | Run, cosmetic |
| Multiplayer | `EventSynchronizer`, `MapSelectionSynchronizer` | 1 each | Run, multiplayer-only |

## `RunRngSet` — 12 named streams

Constructed from a *string* seed. The string is hashed to a `uint` via
`StringHelper.GetDeterministicHashCode(string)`, then each `RunRngType` enum
member becomes a stream seeded by `(uint_seed + hash(snake_case_name))`.

| Stream | Probable purpose (inferred from name + grep) |
|---|---|
| `UpFront` | Pre-run / initial setup randomization |
| `Shuffle` | Deck shuffling inside combats |
| `UnknownMapPoint` | "?" map node resolution |
| `CombatCardGeneration` | Generating ad-hoc cards mid-combat (Discovery-style effects) |
| `CombatPotionGeneration` | Generating potions mid-combat |
| `CombatCardSelection` | Selecting cards mid-combat (random-card effects) |
| `CombatEnergyCosts` | Randomized energy costs (X-cost, cost-shuffles) |
| `CombatTargets` | Random monster targeting |
| `MonsterAi` | Monster move selection |
| `Niche` | Catch-all for low-frequency randomization |
| `CombatOrbs` | Orb generation (Defect-style) |
| `TreasureRoomRelics` | Treasure room relic rolls |

## `PlayerRngSet` — 3 named streams per player

| Stream | Purpose |
|---|---|
| `Rewards` | Combat/event reward selection (cards, gold, etc.) |
| `Shops` | Shop content generation |
| `Transformations` | Outcomes of "transform a card" effects |

Each player in a multiplayer run has its own `PlayerRngSet` seeded from the
run seed.

## Ad-hoc RNG instances

Various subsystems spin up their own `Rng` from a deterministic function of
the run seed plus identity. Worth knowing because the simulator must derive
each the same way:

- **Map generation** (`StandardActMap`, `SpoilsActMap`): `new Rng(runState.Rng.Seed, "map_for_act_<n>")` and similar.
- **Encounters** (`EncounterModel`): `new Rng(num, 0)` where `num` is a hash of `(run seed, act, encounter id)`. Also has a "now" seed at line 492 for non-deterministic test paths.
- **Combat monster Rngs** (`CombatState.cs:232`): `monster.Rng = new Rng(num5, 0)` — per-monster, seeded inside `CombatState` from the encounter Rng.
- **Combat rooms** (`NCombatRoom.cs:238`): `new Rng(state.Rng.Seed + num, 0)` — per-room.
- **Events** (`EventModel.cs:164`): `new Rng(runStateSeed + (multiplayer ? 0 : NetId) + hash(eventId), 0)` — per-event.
- **Cosmetic relics** (`FurCoat`, `Byrdpip`, `PaelsLegion`): each picks a skin once via `new Rng(seed + NetId + hash(relic_name), 0).NextItem(...)`.
- **Daily run setup** (`NDailyRunScreen.cs`): generates four nested Rngs from a date-string seed for daily modifiers.
- **Big Game Hunter modifier** (`BigGameHunter.cs`): per-encounter Rng for elite selection.

The multiplayer-only ones (`EventSynchronizer`, `MapSelectionSynchronizer`)
can be ignored for solo-mode.

## The hash function

`StringHelper.GetDeterministicHashCode(string)` is the classic .NET Framework
pre-randomization `string.GetHashCode`:

```csharp
int num = 352654597;
int num2 = num;
for (int i = 0; i < str.Length; i += 2)
{
    num = ((num << 5) + num) ^ (int)str[i];
    if (i == str.Length - 1) break;
    num2 = ((num2 << 5) + num2) ^ (int)str[i + 1];
}
return num + num2 * 1566083941;
```

Notes for the Rust port:
- `(num << 5) + num` is `num.wrapping_mul(33)`.
- `(int)str[i]` is the UTF-16 code unit (`u16` → sign-extended to `i32`).
- For ASCII inputs (all enum names and most internal seed strings) this
  reduces to iterating bytes. Daily/seed strings from users may contain
  non-ASCII — at that point we must convert UTF-8 → UTF-16 first.
- The final return is `i32` wrapping arithmetic.
- This must be a separate oracle-validated module before any further port
  that uses named-seed `Rng` construction.

## The named `Rng` constructor

`new Rng(uint seed, string name)` is sugar for:

```csharp
new Rng(seed + (uint)GetDeterministicHashCode(name), 0)
```

The Rust port needs both `Rng::new(seed, counter)` (already done) and
`Rng::new_named(seed, name)` once the hash function is ported.

## Save / restore semantics

`RunRngSet.ToSerializable` writes:

```
{ Seed: string, Counters: dict<RunRngType, int> }
```

`FromSave` re-creates each `Rng` and calls `FastForwardCounter` to the
saved value. Same pattern for `PlayerRngSet`.

The ad-hoc Rngs (per-encounter, per-event, etc.) are NOT serialized — they
get regenerated on load from the same identity-derived seed plus current
state context. This implies that any ad-hoc Rng must be **idempotent** with
respect to load: same identity → same seed → same draws.

## Simulator design implications

1. **State representation**: the Rust `RunState` should carry `RunRngSet`
   and per-player `PlayerRngSet` as first-class fields. The "live" Rng
   instances can be lazy — for `clone_state` performance during MCTS-style
   rollouts, we may want to represent the streams as `(seed_string,
   counters: [i32; N])` and instantiate `Rng` on demand.
2. **Cloning cost**: a deep clone of all Rng instances (every stream + every
   per-monster/per-event Rng) is expensive (each Rng is ~224 bytes of
   `[i32; 56]` state). Lazy reconstruction from `(seed, counter)` makes
   clones cheap (just copy counters) at the cost of re-initialization on
   first use. Worth benchmarking before committing.
3. **Determinism enforcement**: never use any non-Rng source of randomness in
   the simulator (e.g. iteration order over `HashMap`). Use `BTreeMap` or
   sorted vecs anywhere iteration matters.
4. **Ad-hoc Rng seeds**: every ad-hoc construction site must be replicated
   exactly — the same hash mixing, the same arithmetic on the run seed. The
   oracle host will let us diff each site individually.

## Next steps that depend on this

- Port `GetDeterministicHashCode` (next task, small, oracle-validate).
- Port the named `Rng` constructor (`Rng::new_named`).
- Port `RunRngSet` and `PlayerRngSet` (data classes + lazy materialization).
- Each future module that derives a sub-Rng (Map, Encounter, Event, Combat)
  must follow the exact mixing pattern documented above.
