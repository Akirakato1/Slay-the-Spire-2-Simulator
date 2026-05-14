# run_replay

Coverage / readiness report for `.run`-corpus combat-replay (task #72).

Walks one or more `.run` files, lists every combat-room encounter the
player faced, and tags each with simulator readiness:

- **encounter-known** — `ENCOUNTER.X` is in our extracted encounter
  table.
- **monsters-known** — every monster in the room is in our extracted
  monster table.
- **monster-intents-ported** — every monster has a Rust intent state
  machine in `combat.rs`.

Output is a sorted by-encounter coverage table and a by-monster
frequency table. Tells us which monsters/encounters to prioritize next
to unlock end-to-end run replay.

The tool does NOT yet drive the simulator through the combats — that's
the next step once monster coverage is broad enough that random-policy
replay reaches the end of a run without hitting an Unhandled wall.

## Usage

```
python tools/run_replay/coverage.py path/to/run-or-dir [more paths...]
```

Pass a directory and every `.run` file under it is processed.

Sample output:

```
=== Encounter coverage across N runs ===

  fully ready: 8 / 47 (17%)

  by encounter, sorted by occurrences:
    12x NibbitsWeak                    [READY]
     7x ScrollsOfBitingWeak            [missing-monster: ScrollOfBiting]
     5x AxebotsNormal                  [READY]
     ...

=== Monster occurrences (across all encounters) ===

   18x Axebot                          [READY]
   15x Nibbit                          [READY]
    7x ScrollOfBiting                  [missing-intent-machine]
    ...
```
