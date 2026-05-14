# combat_smoke

Smoke-test Python harness for the `sts2-sim-py` PyO3 bindings. Run a
random-policy agent in combat against the `AxebotsNormal` encounter and
print a summary (win rate, average rounds, average HP at end, action
distribution). Useful to:

1. Verify the Python boundary works end-to-end after a fresh
   `maturin develop` build.
2. Sanity-check the observation / legal-actions / step pipeline before
   wiring an actual RL agent.
3. Gather a baseline against which any agent must beat.

## Build the extension

From the repo root in a Python 3.9+ environment:

```
cd crates/sts2-sim-py
pip install maturin
maturin develop --release
```

The compiled `sts2_sim_py.pyd` (Windows) / `.so` (Linux/Mac) lands in
the active venv's site-packages.

## Run

```
python tools/combat_smoke/random_policy.py --fights 200 --seed 0
```

Sample output:

```
sts2_sim_py random-policy smoke test
  character          = Ironclad
  encounter          = AxebotsNormal
  fights             = 200
  seed (initial)     = 0
  schema version     = 1

results:
  win rate           = 0.385 (77/200)
  defeat rate        = 0.615 (123/200)
  avg rounds         = 6.8
  avg final-hp (win) = 38.4
  action mix:
    PlayCard         = 1521
    EndTurn          = 1359
```

Random policy isn't expected to win much against Axebots — the value
is in the loop running cleanly. If win rate is near 0 even after
seeded retries, look for stuck legal-action sets (an empty list with
non-terminal state is the most common bug).

## Flags

- `--fights N` — number of independent combats to run (default 100).
- `--seed S` — initial seed; per-fight seeds are `S, S+1, S+2, ...`.
- `--character ID` — which character to play (default `Ironclad`).
- `--encounter ID` — which encounter to fight (default `AxebotsNormal`).
- `--verbose` — print per-step action and outcome details.
