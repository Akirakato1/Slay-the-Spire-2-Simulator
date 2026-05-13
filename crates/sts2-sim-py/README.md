# sts2-sim-py

Python bindings for [`sts2-sim`](../sts2-sim/), the headless Slay the Spire 2 simulator.

Initial surface is narrow on purpose — static data tables and `.run` file parsing only — so the Python-side agent-training plumbing can start iterating before the full simulator state surface is finalized.

## Build

From this directory, in a Python 3.9+ environment:

```bash
pip install maturin
maturin develop --release
```

That builds the Rust extension and installs it into the active virtualenv. After that:

```python
import sts2_sim_py
import json

# Static data lookups.
print(sts2_sim_py.card_count())        # 577
print(sts2_sim_py.relic_count())       # 294
print(sts2_sim_py.power_count())       # 243

# Parse a run file (returns a JSON string; load it into a dict).
summary = json.loads(
    sts2_sim_py.parse_run_file(r"C:\path\to\game.run")
)
print(summary["seed"], summary["ascension"], summary["win"])

# Per-card data lookup.
strike = json.loads(sts2_sim_py.card_data("StrikeIronclad"))
print(strike["energy_cost"], strike["canonical_vars"])
```

## Why JSON?

Returning JSON strings keeps the Rust↔Python boundary thin — we don't have to handcraft `#[pyclass]` wrappers for every type. The Rust types stay free to evolve; the Python side does `json.loads` and gets dicts. When a specific wrapper grows enough usage to warrant typed access (e.g., `RunState` clone/inspect from the analysis tool), replace its `String` return with a `#[pyclass]` — the JSON shape stays the same.

## Roadmap

Currently exposes:
- `parse_run_file(path)` → JSON of the full `RunLog`
- `card_count()` / `card_ids()` / `card_data(id)`
- `relic_count()` / `power_count()` / `monster_count()`
- `character_ids()`

Will grow:
- `RunState` construction + inspection (`new(seed, ascension)`, `enter_act`, …)
- `CombatState` clone/inspect for the analysis tool
- Combat replay through `.run` files once card / monster coverage is broader
