//! Helpers for diff-testing `sts2-sim` against the C# oracle host.
//!
//! The oracle is a separate process that reflectively loads the shipping
//! `sts2.dll` and exposes game functions over stdio JSON-RPC. Tests live under
//! `tests/`; they spawn the oracle, drive both implementations with the same
//! inputs, and assert bit-exact outputs.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{Context, Result};
use serde_json::Value;

pub struct Oracle {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Oracle {
    pub fn spawn() -> Result<Self> {
        let exe = oracle_exe_path()?;
        let mut child = Command::new(&exe)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("failed to spawn oracle host: {}", exe.display()))?;
        let stdin = child.stdin.take().context("missing stdin on oracle host")?;
        let stdout = BufReader::new(
            child.stdout.take().context("missing stdout on oracle host")?,
        );
        Ok(Self { child, stdin, stdout })
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let req = serde_json::json!({ "method": method, "params": params });
        let line = serde_json::to_string(&req)?;
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        let mut response = String::new();
        self.stdout
            .read_line(&mut response)
            .context("reading oracle response")?;
        let value: Value = serde_json::from_str(response.trim())
            .with_context(|| format!("parsing oracle response: {response:?}"))?;
        Ok(value)
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn oracle_exe_path() -> Result<PathBuf> {
    // CARGO_MANIFEST_DIR = sim/crates/sts2-sim-oracle-tests.
    // Workspace root = two parents up. Oracle host built output:
    //   sim/oracle-host/bin/Release/net9.0/oracle-host.exe
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest
        .parent()
        .and_then(|p| p.parent())
        .context("could not resolve workspace root from CARGO_MANIFEST_DIR")?
        .to_path_buf();
    Ok(workspace
        .join("oracle-host")
        .join("bin")
        .join("Release")
        .join("net9.0")
        .join("oracle-host.exe"))
}
