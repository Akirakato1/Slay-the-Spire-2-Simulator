# Slay the Spire 2 Simulator

Headless, deterministic simulator for Slay the Spire 2. Rust translation of the
MegaCrit decompile, validated against the shipping game DLL via a C# "oracle
host" that loads `sts2.dll` reflectively and exposes its functions over
stdio JSON-RPC.

## Status

Phase 0 — scaffolding. No game mechanics ported yet. The next port is the RNG.

## Layout

```
sim/
├── crates/
│   ├── sts2-sim/                  Rust core simulator library.
│   └── sts2-sim-oracle-tests/     Diff tests vs the C# oracle host.
└── oracle-host/                   C# console app. Loads sts2.dll reflectively
                                   and exposes game functions over stdio JSON-RPC.
```

A `sts2-sim-py` (PyO3) bindings crate will be added once the simulator is
mature enough to be useful from Python.

## Build

```powershell
# Rust workspace
cargo check

# C# oracle host
dotnet build oracle-host -c Release

# Diff tests (oracle host must be built first)
cargo test -p sts2-sim-oracle-tests -- --include-ignored
```

## Oracle prerequisites

The oracle host needs the real game DLL. By default it looks for:

```
G:\SteamLibrary\steamapps\common\Slay the Spire 2\data_sts2_windows_x86_64\sts2.dll
```

Override with the `STS2_GAME_DIR` environment variable.

## Authenticity discipline

Translations are bit-exact only when validated. No module is considered done
until its diff tests are green against the oracle host over a randomized input
distribution.
