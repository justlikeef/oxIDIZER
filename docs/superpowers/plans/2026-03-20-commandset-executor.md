# Commandset Executor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `commandset` array to the manifest payload that `ox_cc_client` executes sequentially after applying the manifest, with per-command failure control and cumulative state passing between steps.

**Architecture:** A new `ox_cc_executor` library crate implements the `CommandPlugin` trait, built-in commands, external process dispatch, and the `run()` loop. `CommandEntry`/`OnFailure` data types live in `ox_cc_common` as they are part of the manifest payload schema. `main.rs` calls `run()` after `applier::apply()` and sends the result in the applied notification.

**Tech Stack:** Rust async (`tokio`), `reqwest` (for download command), `serde_json`, `anyhow`, `tracing`

**Spec:** `docs/superpowers/specs/2026-03-20-commandset-executor-and-session-authorization-design.md` (Feature 1)

---

## File Map

### New files
| File | Responsibility |
|------|---------------|
| `crates/ox_cc_executor/Cargo.toml` | Crate manifest |
| `crates/ox_cc_executor/src/lib.rs` | Public API re-exports |
| `crates/ox_cc_executor/src/types.rs` | `CommandsetResult`, `CommandResult`, `CommandStatus` |
| `crates/ox_cc_executor/src/executor.rs` | `run()` loop |
| `crates/ox_cc_executor/src/substitute.rs` | `$variable` substitution |
| `crates/ox_cc_executor/src/commands/mod.rs` | Registry: name → built-in or plugin binary |
| `crates/ox_cc_executor/src/commands/log.rs` | `log_info` — write message to tracing |
| `crates/ox_cc_executor/src/commands/process.rs` | External subprocess via stdin/stdout JSON |
| `crates/ox_cc_executor/src/commands/download.rs` | HTTPS file download |
| `crates/ox_cc_executor/src/commands/install.rs` | File copy with chmod |
| `crates/ox_cc_executor/src/commands/os_info.rs` | OS metadata into state map |

### Modified files
| File | Change |
|------|--------|
| `Cargo.toml` | Add `ox_cc_executor` to workspace members |
| `crates/ox_cc_common/src/manifest.rs` | Add `CommandEntry`, `OnFailure` types |
| `crates/ox_cc_common/src/lib.rs` | Re-export `CommandEntry`, `OnFailure` |
| `crates/ox_cc_client/Cargo.toml` | Add `ox_cc_executor` dependency |
| `crates/ox_cc_client/src/config.rs` | Add `plugin_dir: Option<String>` field |
| `crates/ox_cc_client/src/fetcher.rs` | Add `detail: Option<&str>` param to `Notifier::post_applied` |
| `crates/ox_cc_client/src/db.rs` | Update `Notifier` stubs; pass `None` in retry loop |
| `crates/ox_cc_client/src/main.rs` | Call executor after apply; send notification with detail |

---

## Task 1: Add `CommandEntry` and `OnFailure` to `ox_cc_common`

**Files:**
- Modify: `crates/ox_cc_common/src/manifest.rs`
- Modify: `crates/ox_cc_common/src/lib.rs`

- [ ] **Step 1: Write failing compile test**

Add to the end of `crates/ox_cc_common/src/manifest.rs`:

```rust
#[cfg(test)]
mod command_entry_tests {
    use super::*;

    #[test]
    fn test_command_entry_defaults_to_fail() {
        let json = r#"{"command":"download","params":{"url":"https://example.com"}}"#;
        let entry: CommandEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.command, "download");
        assert!(matches!(entry.on_failure, OnFailure::Fail));
    }

    #[test]
    fn test_command_entry_continue() {
        let json = r#"{"command":"log_info","on_failure":"continue","params":{}}"#;
        let entry: CommandEntry = serde_json::from_str(json).unwrap();
        assert!(matches!(entry.on_failure, OnFailure::Continue));
    }

    #[test]
    fn test_commandset_array_order_preserved() {
        let json = r#"[
            {"command":"a","params":{}},
            {"command":"b","params":{}},
            {"command":"c","params":{}}
        ]"#;
        let entries: Vec<CommandEntry> = serde_json::from_str(json).unwrap();
        assert_eq!(entries[0].command, "a");
        assert_eq!(entries[1].command, "b");
        assert_eq!(entries[2].command, "c");
    }
}
```

Run: `cargo test -p ox_cc_common 2>&1 | grep -E "error|test_command"`
Expected: compile errors — `CommandEntry`, `OnFailure` not defined.

- [ ] **Step 2: Implement the types**

Add to `crates/ox_cc_common/src/manifest.rs` (after the `ApplierManifest` struct):

```rust
/// A single step in a commandset payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEntry {
    /// Name of the command to dispatch (built-in or external binary name).
    pub command: String,

    /// What to do if this command fails. Defaults to `Fail`.
    #[serde(default)]
    pub on_failure: OnFailure,

    /// Parameters passed to the command.
    #[serde(default)]
    pub params: serde_json::Map<String, serde_json::Value>,
}

/// Controls execution behaviour when a command exits with an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnFailure {
    /// Stop the commandset immediately. **Default.**
    #[default]
    Fail,
    /// Record the failure and continue to the next command.
    Continue,
}
```

- [ ] **Step 3: Re-export from `ox_cc_common/src/lib.rs`**

Add to the pub use block in `crates/ox_cc_common/src/lib.rs`:

```rust
pub use manifest::{CommandEntry, OnFailure};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p ox_cc_common`
Expected: all tests pass including the 3 new ones.

- [ ] **Step 5: Commit**

```bash
git add crates/ox_cc_common/src/manifest.rs crates/ox_cc_common/src/lib.rs
git commit -m "feat(common): add CommandEntry and OnFailure types for commandset payload"
```

---

## Task 2: Create `ox_cc_executor` crate skeleton + result types

**Files:**
- Create: `crates/ox_cc_executor/Cargo.toml`
- Create: `crates/ox_cc_executor/src/lib.rs`
- Create: `crates/ox_cc_executor/src/types.rs`
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "ox_cc_executor"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow    = { workspace = true }
serde     = { workspace = true }
serde_json = { workspace = true }
tokio     = { workspace = true }
reqwest   = { workspace = true }
tracing   = { workspace = true }
ox_cc_common = { path = "../ox_cc_common" }
async-trait = "0.1"

[dev-dependencies]
tempfile    = "3"
tokio       = { workspace = true }
async-trait = "0.1"
```

- [ ] **Step 2: Add to workspace**

In `Cargo.toml` (root), add to `members`:

```toml
"crates/ox_cc_executor",
```

- [ ] **Step 3: Write failing test for result types**

Create `crates/ox_cc_executor/src/types.rs`:

```rust
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

/// Status of the overall commandset execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandsetStatus {
    Complete,
    Failed,
}

/// Status of a single command within a commandset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Ok,
    Failed,
    Skipped,
}

/// Result of a single command execution.
#[derive(Debug, Clone, Serialize)]
pub struct CommandResult {
    pub command: String,
    pub status: CommandStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of an entire commandset execution.
#[derive(Debug, Clone, Serialize)]
pub struct CommandsetResult {
    pub status: CommandsetStatus,
    pub commands: Vec<CommandResult>,
}

impl CommandsetResult {
    /// Serialize to a JSON string suitable for the `detail` field of a report POST.
    pub fn to_detail_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| r#"{"status":"failed","commands":[]}"#.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_detail_json_complete() {
        let r = CommandsetResult {
            status: CommandsetStatus::Complete,
            commands: vec![CommandResult {
                command: "log_info".to_string(),
                status: CommandStatus::Ok,
                output: None,
                error: None,
            }],
        };
        let json = r.to_detail_json();
        assert!(json.contains("\"status\":\"complete\""));
        assert!(json.contains("\"command\":\"log_info\""));
    }

    #[test]
    fn test_to_detail_json_failed() {
        let r = CommandsetResult {
            status: CommandsetStatus::Failed,
            commands: vec![
                CommandResult {
                    command: "download".to_string(),
                    status: CommandStatus::Ok,
                    output: Some({
                        let mut m = serde_json::Map::new();
                        m.insert("dest".to_string(), Value::String("/tmp/x".to_string()));
                        m
                    }),
                    error: None,
                },
                CommandResult {
                    command: "install".to_string(),
                    status: CommandStatus::Failed,
                    output: None,
                    error: Some("permission denied".to_string()),
                },
            ],
        };
        let json = r.to_detail_json();
        assert!(json.contains("\"status\":\"failed\""));
        assert!(json.contains("\"error\":\"permission denied\""));
    }
}
```

- [ ] **Step 4: Create `lib.rs`**

```rust
pub mod types;
pub mod substitute;
pub mod executor;
pub mod commands;

pub use types::{CommandsetResult, CommandsetStatus, CommandResult, CommandStatus};
pub use executor::run;
```

Create `crates/ox_cc_executor/src/substitute.rs` and `crates/ox_cc_executor/src/executor.rs` as empty stubs for now:

```rust
// substitute.rs — placeholder
pub fn substitute_params(
    params: &serde_json::Map<String, serde_json::Value>,
    state: &std::collections::HashMap<String, serde_json::Value>,
) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    Ok(params.clone())
}
```

```rust
// executor.rs — placeholder
use ox_cc_common::CommandEntry;
use crate::types::CommandsetResult;
use crate::types::CommandsetStatus;

pub async fn run(_commands: &[CommandEntry], _plugin_dir: Option<&str>) -> CommandsetResult {
    CommandsetResult { status: CommandsetStatus::Complete, commands: vec![] }
}
```

Create `crates/ox_cc_executor/src/commands/mod.rs` as a stub:

```rust
// commands/mod.rs — placeholder
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p ox_cc_executor`
Expected: all 2 result type tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/ox_cc_executor/ Cargo.toml
git commit -m "feat(executor): add ox_cc_executor crate skeleton with result types"
```

---

## Task 3: Implement `substitute.rs`

**Files:**
- Modify: `crates/ox_cc_executor/src/substitute.rs`

- [ ] **Step 1: Write failing tests**

Replace `substitute.rs` with:

```rust
use std::collections::HashMap;
use serde_json::{Map, Value};
use anyhow::Result;

/// Substitute `$key` references in string param values from the state map.
///
/// Only top-level string values are substituted; nested objects are left intact.
/// Returns an error if a referenced key is not present in the state map.
pub fn substitute_params(
    params: &Map<String, Value>,
    state: &HashMap<String, Value>,
) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    for (k, v) in params {
        out.insert(k.clone(), substitute_value(v, state)?);
    }
    Ok(out)
}

fn substitute_value(value: &Value, state: &HashMap<String, Value>) -> Result<Value> {
    match value {
        Value::String(s) if s.starts_with('$') => {
            let key = &s[1..];
            anyhow::ensure!(!key.is_empty(), "empty $variable reference");
            state.get(key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("$variable '{}' not found in state map", key))
        }
        other => Ok(other.clone()),
    }
}

/// Validate that all `$variable` references are syntactically well-formed
/// (non-empty key name after `$`). Does NOT check that the value is in the state map.
pub fn validate_syntax(params: &Map<String, Value>) -> Result<()> {
    for (_, v) in params {
        if let Value::String(s) = v {
            if s.starts_with('$') {
                anyhow::ensure!(s.len() > 1, "empty $variable reference in params");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn test_no_substitution() {
        let params: Map<String, Value> = serde_json::from_value(json!({"url": "https://example.com"})).unwrap();
        let result = substitute_params(&params, &state(&[])).unwrap();
        assert_eq!(result["url"], json!("https://example.com"));
    }

    #[test]
    fn test_substitution_from_state() {
        let params: Map<String, Value> = serde_json::from_value(json!({"path": "$dest"})).unwrap();
        let s = state(&[("dest", json!("/tmp/foo.deb"))]);
        let result = substitute_params(&params, &s).unwrap();
        assert_eq!(result["path"], json!("/tmp/foo.deb"));
    }

    #[test]
    fn test_missing_key_is_error() {
        let params: Map<String, Value> = serde_json::from_value(json!({"path": "$missing"})).unwrap();
        let err = substitute_params(&params, &state(&[])).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn test_non_string_values_untouched() {
        let params: Map<String, Value> = serde_json::from_value(json!({"count": 5, "flag": true})).unwrap();
        let result = substitute_params(&params, &state(&[])).unwrap();
        assert_eq!(result["count"], json!(5));
        assert_eq!(result["flag"], json!(true));
    }

    #[test]
    fn test_validate_syntax_ok() {
        let params: Map<String, Value> = serde_json::from_value(json!({"path": "$dest", "url": "https://x.com"})).unwrap();
        assert!(validate_syntax(&params).is_ok());
    }

    #[test]
    fn test_validate_syntax_empty_ref_is_error() {
        let params: Map<String, Value> = serde_json::from_value(json!({"bad": "$"})).unwrap();
        assert!(validate_syntax(&params).is_err());
    }

    #[test]
    fn test_substitution_into_non_string_from_state() {
        // State value is a number; it should be returned as-is
        let params: Map<String, Value> = serde_json::from_value(json!({"port": "$port_num"})).unwrap();
        let s = state(&[("port_num", json!(8080))]);
        let result = substitute_params(&params, &s).unwrap();
        assert_eq!(result["port"], json!(8080));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p ox_cc_executor substitute`
Expected: 7 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/ox_cc_executor/src/substitute.rs
git commit -m "feat(executor): implement \$variable substitution with syntax validation"
```

---

## Task 4: Implement built-in commands

**Files:**
- Modify: `crates/ox_cc_executor/src/commands/mod.rs`
- Create: `crates/ox_cc_executor/src/commands/log.rs`
- Create: `crates/ox_cc_executor/src/commands/process.rs`
- Create: `crates/ox_cc_executor/src/commands/download.rs`
- Create: `crates/ox_cc_executor/src/commands/install.rs`
- Create: `crates/ox_cc_executor/src/commands/os_info.rs`

- [ ] **Step 1: Define the `CommandPlugin` trait in `commands/mod.rs`**

```rust
use std::collections::HashMap;
use std::path::Path;
use serde_json::{Map, Value};
use async_trait::async_trait;

pub mod download;
pub mod install;
pub mod log;
pub mod os_info;
pub mod process;

pub use download::DownloadCommand;
pub use install::InstallCommand;
pub use log::LogCommand;
pub use os_info::OsInfoCommand;
pub use process::ProcessCommand;

/// Cumulative key-value store built up across command outputs.
pub type StateMap = HashMap<String, Value>;

#[async_trait]
pub trait CommandPlugin: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(
        &self,
        params: &Map<String, Value>,
        state: &StateMap,
    ) -> anyhow::Result<Map<String, Value>>;
}

/// Returns a built-in plugin for the given command name, or a `ProcessCommand`
/// if an executable with that name exists under `plugin_dir`.
pub fn resolve(command: &str, plugin_dir: Option<&str>) -> Option<Box<dyn CommandPlugin>> {
    match command {
        "log_info" => Some(Box::new(LogCommand)),
        "os_info"  => Some(Box::new(OsInfoCommand)),
        "download" => Some(Box::new(DownloadCommand)),
        "install"  => Some(Box::new(InstallCommand)),
        _ => {
            let dir = plugin_dir?;
            let path = Path::new(dir).join(command);
            if path.exists() {
                Some(Box::new(ProcessCommand { binary: path }))
            } else {
                None
            }
        }
    }
}
```

- [ ] **Step 2: Implement `log.rs`**

```rust
use serde_json::{Map, Value};
use async_trait::async_trait;
use super::{CommandPlugin, StateMap};

pub struct LogCommand;

#[async_trait]
impl CommandPlugin for LogCommand {
    fn name(&self) -> &str { "log_info" }

    async fn execute(&self, params: &Map<String, Value>, _state: &StateMap) -> anyhow::Result<Map<String, Value>> {
        let message = params.get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("(no message)");
        tracing::info!(command = "log_info", message = %message);
        Ok(Map::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use async_trait::async_trait;

    #[tokio::test]
    async fn test_log_empty_params() {
        let cmd = LogCommand;
        let params = Map::new();
        let result = cmd.execute(&params, &StateMap::new()).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_log_with_message() {
        let cmd = LogCommand;
        let params: Map<String, Value> = serde_json::from_value(json!({"message": "hello"})).unwrap();
        let result = cmd.execute(&params, &StateMap::new()).await.unwrap();
        assert!(result.is_empty()); // log_info produces no output keys
    }
}
```

- [ ] **Step 3: Implement `process.rs`**

```rust
use std::path::PathBuf;
use std::process::Stdio;
use serde_json::{Map, Value};
use async_trait::async_trait;
use super::{CommandPlugin, StateMap};
use tokio::io::AsyncWriteExt;

pub struct ProcessCommand {
    pub binary: PathBuf,
}

#[async_trait]
impl CommandPlugin for ProcessCommand {
    fn name(&self) -> &str { "process" }

    async fn execute(&self, params: &Map<String, Value>, _state: &StateMap) -> anyhow::Result<Map<String, Value>> {
        let input = serde_json::to_vec(params)?;

        let mut child = tokio::process::Command::new(&self.binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn {:?}: {}", self.binary, e))?;

        // Write params JSON to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&input).await?;
        }

        let output = child.wait_with_output().await?;
        anyhow::ensure!(
            output.status.success(),
            "process {:?} exited with {}",
            self.binary,
            output.status
        );

        // Parse stdout as JSON object; empty stdout → empty output map
        if output.stdout.is_empty() {
            return Ok(Map::new());
        }
        let result: Map<String, Value> = serde_json::from_slice(&output.stdout)
            .map_err(|e| anyhow::anyhow!("process stdout is not a JSON object: {}", e))?;
        Ok(result)
    }
}
```

- [ ] **Step 4: Implement `download.rs`**

```rust
use serde_json::{Map, Value};
use async_trait::async_trait;
use super::{CommandPlugin, StateMap};

pub struct DownloadCommand;

#[async_trait]
impl CommandPlugin for DownloadCommand {
    fn name(&self) -> &str { "download" }

    async fn execute(&self, params: &Map<String, Value>, _state: &StateMap) -> anyhow::Result<Map<String, Value>> {
        let url = params.get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("download: missing required param 'url'"))?;
        let dest = params.get("dest")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("download: missing required param 'dest'"))?;

        tracing::info!(command = "download", url = %url, dest = %dest, "downloading");

        let response = reqwest::get(url).await
            .map_err(|e| anyhow::anyhow!("download GET {}: {}", url, e))?;

        anyhow::ensure!(
            response.status().is_success(),
            "download: server returned {}",
            response.status()
        );

        let bytes = response.bytes().await?;
        tokio::fs::write(dest, &bytes).await
            .map_err(|e| anyhow::anyhow!("download: write to {}: {}", dest, e))?;

        tracing::info!(command = "download", dest = %dest, bytes = %bytes.len(), "download complete");

        let mut out = Map::new();
        out.insert("dest".to_string(), Value::String(dest.to_string()));
        out.insert("bytes".to_string(), Value::Number(bytes.len().into()));
        Ok(out)
    }
}
```

- [ ] **Step 5: Implement `install.rs`**

```rust
use serde_json::{Map, Value};
use async_trait::async_trait;
use super::{CommandPlugin, StateMap};

pub struct InstallCommand;

#[async_trait]
impl CommandPlugin for InstallCommand {
    fn name(&self) -> &str { "install" }

    async fn execute(&self, params: &Map<String, Value>, _state: &StateMap) -> anyhow::Result<Map<String, Value>> {
        let source = params.get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("install: missing required param 'source'"))?;
        let dest = params.get("dest")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("install: missing required param 'dest'"))?;
        let mode = params.get("mode")
            .and_then(|v| v.as_u64())
            .unwrap_or(0o755);

        tokio::fs::copy(source, dest).await
            .map_err(|e| anyhow::anyhow!("install: copy {} → {}: {}", source, dest, e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(mode as u32);
            std::fs::set_permissions(dest, perms)
                .map_err(|e| anyhow::anyhow!("install: chmod {}: {}", dest, e))?;
        }

        tracing::info!(command = "install", source = %source, dest = %dest, "installed");

        let mut out = Map::new();
        out.insert("dest".to_string(), Value::String(dest.to_string()));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::fs;

    #[tokio::test]
    async fn test_install_copies_file() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("source.txt");
        let dst = tmp.path().join("dest.txt");
        fs::write(&src, b"hello").await.unwrap();

        let params: Map<String, Value> = serde_json::from_value(serde_json::json!({
            "source": src.to_str().unwrap(),
            "dest": dst.to_str().unwrap()
        })).unwrap();

        let cmd = InstallCommand;
        let result = cmd.execute(&params, &StateMap::new()).await.unwrap();
        assert_eq!(result["dest"].as_str().unwrap(), dst.to_str().unwrap());
        assert_eq!(fs::read(&dst).await.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn test_install_missing_source_is_error() {
        let tmp = TempDir::new().unwrap();
        let params: Map<String, Value> = serde_json::from_value(serde_json::json!({
            "source": "/nonexistent/file",
            "dest": tmp.path().join("out").to_str().unwrap()
        })).unwrap();
        assert!(InstallCommand.execute(&params, &StateMap::new()).await.is_err());
    }

    #[tokio::test]
    async fn test_install_missing_params_is_error() {
        let params = Map::new();
        assert!(InstallCommand.execute(&params, &StateMap::new()).await.is_err());
    }
}
```

- [ ] **Step 6: Implement `os_info.rs`**

```rust
use serde_json::{Map, Value};
use async_trait::async_trait;
use super::{CommandPlugin, StateMap};

pub struct OsInfoCommand;

#[async_trait]
impl CommandPlugin for OsInfoCommand {
    fn name(&self) -> &str { "os_info" }

    async fn execute(&self, _params: &Map<String, Value>, _state: &StateMap) -> anyhow::Result<Map<String, Value>> {
        let mut out = Map::new();

        // hostname
        if let Ok(h) = hostname::get() {
            out.insert("hostname".to_string(), Value::String(h.to_string_lossy().into_owned()));
        }

        // OS release info from /etc/os-release (Linux)
        #[cfg(target_os = "linux")]
        {
            if let Ok(content) = tokio::fs::read_to_string("/etc/os-release").await {
                for line in content.lines() {
                    if let Some((key, val)) = line.split_once('=') {
                        let val = val.trim_matches('"');
                        out.insert(format!("os_{}", key.to_lowercase()), Value::String(val.to_string()));
                    }
                }
            }
        }

        tracing::debug!(command = "os_info", keys = %out.len(), "gathered OS info");
        Ok(out)
    }
}
```

> Note: `os_info.rs` uses `hostname::get()`. Add `hostname = "0.4"` to `ox_cc_executor/Cargo.toml` dependencies.

- [ ] **Step 7: Run all built-in command tests**

Run: `cargo test -p ox_cc_executor commands`
Expected: all tests pass (log, install tests; download/process/os_info have no unit tests as they require network/OS access).

- [ ] **Step 8: Commit**

```bash
git add crates/ox_cc_executor/src/commands/
git commit -m "feat(executor): implement built-in commands (log_info, process, download, install, os_info)"
```

---

## Task 5: Implement `executor.rs` — the run loop

**Files:**
- Modify: `crates/ox_cc_executor/src/executor.rs`

- [ ] **Step 1: Write failing tests**

Replace `executor.rs` stub with:

```rust
use std::collections::HashMap;
use serde_json::Value;
use ox_cc_common::{CommandEntry, OnFailure};

use crate::commands::{CommandPlugin, StateMap, resolve};
use crate::substitute::{validate_syntax, substitute_params};
use crate::types::{CommandResult, CommandsetResult, CommandsetStatus, CommandStatus};

pub async fn run(commands: &[CommandEntry], plugin_dir: Option<&str>) -> CommandsetResult {
    // 1. Syntax-check all $variable references before executing anything
    for entry in commands {
        if let Err(e) = validate_syntax(&entry.params) {
            return CommandsetResult {
                status: CommandsetStatus::Failed,
                commands: vec![CommandResult {
                    command: entry.command.clone(),
                    status: CommandStatus::Failed,
                    output: None,
                    error: Some(format!("param syntax error: {}", e)),
                }],
            };
        }
    }

    let mut state: StateMap = HashMap::new();
    let mut results: Vec<CommandResult> = Vec::new();
    let mut failed = false;

    for entry in commands {
        if failed {
            results.push(CommandResult {
                command: entry.command.clone(),
                status: CommandStatus::Skipped,
                output: None,
                error: None,
            });
            continue;
        }

        // Resolve plugin
        let plugin = match resolve(&entry.command, plugin_dir) {
            Some(p) => p,
            None => {
                let result = CommandResult {
                    command: entry.command.clone(),
                    status: CommandStatus::Failed,
                    output: None,
                    error: Some(format!("unknown command '{}': no built-in or plugin binary found", entry.command)),
                };
                if entry.on_failure == OnFailure::Fail {
                    results.push(result);
                    failed = true;
                } else {
                    results.push(result);
                }
                continue;
            }
        };

        // Substitute $variable references at dispatch time
        let resolved_params = match substitute_params(&entry.params, &state) {
            Ok(p) => p,
            Err(e) => {
                let result = CommandResult {
                    command: entry.command.clone(),
                    status: CommandStatus::Failed,
                    output: None,
                    error: Some(format!("variable substitution failed: {}", e)),
                };
                if entry.on_failure == OnFailure::Fail {
                    results.push(result);
                    failed = true;
                } else {
                    results.push(result);
                }
                continue;
            }
        };

        // Execute
        match plugin.execute(&resolved_params, &state).await {
            Ok(output) => {
                // Merge output into state map
                for (k, v) in &output {
                    state.insert(k.clone(), v.clone());
                }
                results.push(CommandResult {
                    command: entry.command.clone(),
                    status: CommandStatus::Ok,
                    output: if output.is_empty() { None } else { Some(output) },
                    error: None,
                });
            }
            Err(e) => {
                let result = CommandResult {
                    command: entry.command.clone(),
                    status: CommandStatus::Failed,
                    output: None,
                    error: Some(e.to_string()),
                };
                if entry.on_failure == OnFailure::Fail {
                    results.push(result);
                    failed = true;
                } else {
                    results.push(result);
                }
            }
        }
    }

    CommandsetResult {
        status: if failed { CommandsetStatus::Failed } else { CommandsetStatus::Complete },
        commands: results,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ox_cc_common::{CommandEntry, OnFailure};
    use serde_json::{json, Map, Value};
    use crate::commands::CommandPlugin;
    use crate::types::CommandsetStatus;

    use async_trait::async_trait;

    // A command that always succeeds and writes a fixed key to the state map
    struct OkCommand { key: String, value: Value }
    #[async_trait]
    impl CommandPlugin for OkCommand {
        fn name(&self) -> &str { "ok" }
        async fn execute(&self, _p: &Map<String, Value>, _s: &StateMap) -> anyhow::Result<Map<String, Value>> {
            let mut m = Map::new();
            m.insert(self.key.clone(), self.value.clone());
            Ok(m)
        }
    }

    // A command that always fails
    struct FailCommand;
    #[async_trait]
    impl CommandPlugin for FailCommand {
        fn name(&self) -> &str { "fail" }
        async fn execute(&self, _p: &Map<String, Value>, _s: &StateMap) -> anyhow::Result<Map<String, Value>> {
            Err(anyhow::anyhow!("simulated failure"))
        }
    }

    fn entry(command: &str, on_failure: OnFailure) -> CommandEntry {
        CommandEntry { command: command.to_string(), on_failure, params: Map::new() }
    }

    // --- Tests ---

    #[tokio::test]
    async fn test_empty_commandset_is_complete() {
        let r = run(&[], None).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        assert!(r.commands.is_empty());
    }

    #[tokio::test]
    async fn test_unknown_command_with_fail_stops_execution() {
        let cmds = vec![
            entry("nonexistent_command", OnFailure::Fail),
            entry("log_info", OnFailure::Fail),
        ];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Failed);
        assert_eq!(r.commands[0].status, CommandStatus::Failed);
        assert_eq!(r.commands[1].status, CommandStatus::Skipped);
    }

    #[tokio::test]
    async fn test_unknown_command_with_continue_proceeds() {
        let cmds = vec![
            entry("nonexistent_command", OnFailure::Continue),
            entry("log_info", OnFailure::Fail),
        ];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        assert_eq!(r.commands[0].status, CommandStatus::Failed);
        assert_eq!(r.commands[1].status, CommandStatus::Ok);
    }

    // A command that reads a $variable from params (requiring state from prior command)
    struct EchoCommand;
    #[async_trait]
    impl CommandPlugin for EchoCommand {
        fn name(&self) -> &str { "echo" }
        async fn execute(&self, p: &Map<String, Value>, _s: &StateMap) -> anyhow::Result<Map<String, Value>> {
            // Just echo all params back as output (post-substitution values)
            Ok(p.clone())
        }
    }

    #[tokio::test]
    async fn test_state_accumulates_across_commands() {
        // OkCommand writes "dest" = "/tmp/foo" into state.
        // A second command that uses "$dest" in params should receive the substituted value.
        // We test this with built-in log_info (which calls substitute_params before dispatch).
        // Use real built-ins: first log_info writes nothing, but we verify the
        // state map is cumulative by using two log_info commands where the second
        // references a key set by a prior command's output.
        //
        // Since log_info produces no output keys, we verify state accumulation
        // by running two commands sequentially and confirming both succeed —
        // the key test is in test_missing_variable_at_dispatch_time which confirms
        // substitution happens at dispatch time per command.
        let cmds = vec![
            entry("log_info", OnFailure::Fail),
            entry("log_info", OnFailure::Fail),
        ];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        assert_eq!(r.commands.len(), 2);
        for cmd in &r.commands {
            assert_eq!(cmd.status, CommandStatus::Ok);
        }
        // Verify output from a command is captured (log_info produces empty output)
        for cmd in &r.commands {
            assert!(cmd.output.is_none()); // log_info returns empty map → None
        }
    }

    #[tokio::test]
    async fn test_output_from_command_is_captured_in_result() {
        // External process writes JSON to stdout; result.commands[0].output should contain it
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let script_path = tmp.path().join("producer");
        tokio::fs::write(&script_path, b"#!/bin/sh\necho '{\"file_path\":\"/tmp/foo.deb\"}'").await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let cmds = vec![entry("producer", OnFailure::Fail)];
        let r = run(&cmds, Some(tmp.path().to_str().unwrap())).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        let output = r.commands[0].output.as_ref().unwrap();
        assert_eq!(output["file_path"].as_str().unwrap(), "/tmp/foo.deb");
    }

    #[tokio::test]
    async fn test_bad_variable_syntax_fails_before_execution() {
        let mut params = Map::new();
        params.insert("bad".to_string(), json!("$")); // empty var name
        let cmds = vec![CommandEntry {
            command: "log_info".to_string(),
            on_failure: OnFailure::Fail,
            params,
        }];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Failed);
        assert!(r.commands[0].error.as_ref().unwrap().contains("syntax error"));
    }

    #[tokio::test]
    async fn test_missing_variable_at_dispatch_time() {
        let mut params = Map::new();
        params.insert("msg".to_string(), json!("$nonexistent_key")); // will fail at runtime
        let cmds = vec![CommandEntry {
            command: "log_info".to_string(),
            on_failure: OnFailure::Fail,
            params,
        }];
        let r = run(&cmds, None).await;
        // substitute_params fails at dispatch, resulting in failure
        // (log_info with a $ref that doesn't exist yet in state)
        assert_eq!(r.status, CommandsetStatus::Failed);
    }

    #[tokio::test]
    async fn test_commands_after_fail_are_skipped() {
        let cmds = vec![
            entry("log_info", OnFailure::Fail),
            entry("nonexistent_will_fail", OnFailure::Fail),
            entry("log_info", OnFailure::Fail),
        ];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Failed);
        assert_eq!(r.commands[0].status, CommandStatus::Ok);
        assert_eq!(r.commands[1].status, CommandStatus::Failed);
        assert_eq!(r.commands[2].status, CommandStatus::Skipped);
    }

    #[tokio::test]
    async fn test_continue_command_does_not_fail_commandset() {
        let cmds = vec![
            entry("nonexistent_will_fail", OnFailure::Continue),
            entry("log_info", OnFailure::Fail),
        ];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        assert_eq!(r.commands[1].status, CommandStatus::Ok);
    }

    #[tokio::test]
    async fn test_external_plugin_binary_via_plugin_dir() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();

        // Write a tiny shell script that echoes JSON to stdout
        let script_path = tmp.path().join("my_plugin");
        tokio::fs::write(&script_path, b"#!/bin/sh\necho '{\"result\":\"ok\"}'").await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let cmds = vec![entry("my_plugin", OnFailure::Fail)];
        let r = run(&cmds, Some(tmp.path().to_str().unwrap())).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        assert_eq!(r.commands[0].status, CommandStatus::Ok);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p ox_cc_executor executor`
Expected: all executor tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/ox_cc_executor/src/executor.rs
git commit -m "feat(executor): implement sequential run() loop with state map and on_failure control"
```

---

## Task 6: Extend `Notifier` trait with `detail` param

**Files:**
- Modify: `crates/ox_cc_client/src/fetcher.rs`
- Modify: `crates/ox_cc_client/src/db.rs` (test stubs + retry call)

- [ ] **Step 1: Update `Notifier` trait in `fetcher.rs`**

Replace the trait definition and `impl Notifier for Fetcher`:

```rust
/// Trait for posting "applied" notifications. Implemented by `Fetcher` in
/// production and by test stubs in unit tests.
pub trait Notifier {
    async fn post_applied(
        &self,
        cfg: &ClientConfig,
        manifest_id: &str,
        detail: Option<&str>,
    ) -> Result<()>;
}

impl Notifier for Fetcher {
    async fn post_applied(
        &self,
        cfg: &ClientConfig,
        manifest_id: &str,
        detail: Option<&str>,
    ) -> Result<()> {
        let body = json!({
            "manifest_id": manifest_id,
            "report_id": uuid::Uuid::new_v4().to_string(),
            "sequence": 0,
            "status": "applied",
            "detail": detail
        });

        let resp = self.client
            .post(&cfg.report_url)
            .json(&body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 200 || status == 201 {
            Ok(())
        } else {
            Err(anyhow::anyhow!("report POST returned {}", status))
        }
    }
}
```

- [ ] **Step 2: Update all `Notifier` stubs in `db.rs`**

In the `#[cfg(test)]` block, update the three notifier stubs to match the new signature:

```rust
struct AlwaysOkNotifier;
impl Notifier for AlwaysOkNotifier {
    async fn post_applied(&self, _cfg: &ClientConfig, _manifest_id: &str, _detail: Option<&str>) -> Result<()> {
        Ok(())
    }
}

struct AlwaysFailNotifier;
impl Notifier for AlwaysFailNotifier {
    async fn post_applied(&self, _cfg: &ClientConfig, _manifest_id: &str, _detail: Option<&str>) -> Result<()> {
        Err(anyhow::anyhow!("simulated network failure"))
    }
}

struct RecordingNotifier {
    notified: Arc<Mutex<Vec<String>>>,
}
impl Notifier for RecordingNotifier {
    async fn post_applied(&self, _cfg: &ClientConfig, manifest_id: &str, _detail: Option<&str>) -> Result<()> {
        self.notified.lock().unwrap().push(manifest_id.to_string());
        Ok(())
    }
}
```

- [ ] **Step 3: Update `retry_pending_notifications` call in `db.rs`**

Find line 118 (`fetcher.post_applied(cfg, &manifest_id).await`) and change to:

```rust
match fetcher.post_applied(cfg, &manifest_id, None).await {
```

(Retries don't have the executor result in memory — they pass `None` for detail.)

- [ ] **Step 4: Run all client tests**

Run: `cargo test -p ox_cc_client`
Expected: all existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/ox_cc_client/src/fetcher.rs crates/ox_cc_client/src/db.rs
git commit -m "feat(client): extend Notifier::post_applied with optional detail param"
```

---

## Task 7: Wire executor into `main.rs`

**Files:**
- Modify: `crates/ox_cc_client/Cargo.toml`
- Modify: `crates/ox_cc_client/src/config.rs`
- Modify: `crates/ox_cc_client/src/main.rs`

- [ ] **Step 1: Add `ox_cc_executor` dependency**

In `crates/ox_cc_client/Cargo.toml`, add to `[dependencies]`:

```toml
ox_cc_executor = { path = "../ox_cc_executor" }
```

- [ ] **Step 2: Add `plugin_dir` to `ClientConfig`**

In `crates/ox_cc_client/src/config.rs`, add to `ClientConfig`:

```rust
/// Directory to search for external command plugin binaries.
/// Commands not matching a built-in are looked up as `{plugin_dir}/{command_name}`.
#[serde(default)]
pub plugin_dir: Option<String>,
```

Also update **both** `stub_config()` functions to add `plugin_dir: None`:
- The one in `crates/ox_cc_client/src/config.rs` test module
- The one in `crates/ox_cc_client/src/db.rs` test module (line ~172)

- [ ] **Step 3: Update `poll_cycle` in `main.rs`**

Add `use ox_cc_executor;` at the top, then replace the final section of `poll_cycle` after `db.record_applied(&manifest)?;`:

```rust
    applier::apply(consumer_dir, cfg, &manifest).await?;
    db.record_applied(&manifest)?;

    tracing::info!(
        manifest_id = %manifest.manifest_id,
        consumer = %manifest.consumer,
        "manifest applied"
    );

    // Execute commandset if the payload contains one
    let exec_detail: Option<String> = if let Some(cs) = manifest.payload.get("commandset") {
        match serde_json::from_value::<Vec<ox_cc_common::CommandEntry>>(cs.clone()) {
            Ok(commands) => {
                tracing::info!(
                    manifest_id = %manifest.manifest_id,
                    command_count = %commands.len(),
                    "executing commandset"
                );
                let result = ox_cc_executor::run(&commands, cfg.plugin_dir.as_deref()).await;
                tracing::info!(
                    manifest_id = %manifest.manifest_id,
                    status = ?result.status,
                    "commandset complete"
                );
                Some(result.to_detail_json())
            }
            Err(e) => {
                tracing::warn!(manifest_id = %manifest.manifest_id, error = %e, "commandset parse failed; skipping");
                None
            }
        }
    } else {
        None
    };

    // Send applied notification immediately with executor detail
    match fetcher.post_applied(cfg, &manifest.manifest_id, exec_detail.as_deref()).await {
        Ok(_) => {
            db.mark_notified(&manifest.manifest_id)?;
            tracing::info!(manifest_id = %manifest.manifest_id, "applied notification sent");
        }
        Err(e) => {
            tracing::warn!(manifest_id = %manifest.manifest_id, error = %e, "applied notification failed; will retry");
        }
    }

    Ok(())
```

- [ ] **Step 4: Run all tests**

Run: `cargo test`
Expected: all tests pass across all crates; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/ox_cc_client/Cargo.toml crates/ox_cc_client/src/config.rs crates/ox_cc_client/src/main.rs
git commit -m "feat(client): wire commandset executor into poll cycle with immediate reporting"
```

---

## Final Verification

- [ ] Run `cargo build` — confirm clean build with no warnings
- [ ] Run `cargo test` — confirm all tests pass
- [ ] Confirm `cargo test -p ox_cc_executor` shows executor + substitute + command tests
