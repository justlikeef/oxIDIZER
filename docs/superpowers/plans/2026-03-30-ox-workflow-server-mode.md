# ox_workflow Server Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement ox_workflow as three ox_webservice cdylib plugins (scheduler, API, management) with MQTT as the shared control/task plane, enabling horizontal scaling and cluster-wide management.

**Architecture:** Three independent cdylib plugins share SQLite + MQTT as their coordination layer — no in-process shared state between plugins. The scheduler plugin subscribes to MQTT task queues and a control topic; the API plugin enqueues via MQTT; the management plugin publishes control messages. Any node with the management plugin can control the entire cluster.

**Tech Stack:** Rust, tokio, axum (inside plugins via direct HTTP field handling), sqlx/SQLite WAL, rumqttc (MQTT), ox_workflow_abi cdylib ABI, ox_event_bus_mqtt, serde_json, clap (ox_webservice CLI extension).

---

## File Map

### New files
```
crates/workflow/ox_workflow_scheduler_plugin/
  Cargo.toml
  src/lib.rs          — ox_plugin_init/process/error/destroy + background task spawning
  src/config.rs       — SchedulerPluginConfig (serde, load via ox_fileproc)
  src/control.rs      — MQTT control topic listener, command dispatch
  conf/scheduler.yaml — dev config

crates/workflow/ox_workflow_api_plugin/
  Cargo.toml
  src/lib.rs          — ox_plugin_init/process/error/destroy + HTTP dispatch
  src/config.rs       — ApiPluginConfig
  src/handlers.rs     — enqueue, list, get, cancel, resume, delete handlers
  conf/api.yaml       — dev config

crates/workflow/ox_workflow_management_plugin/
  Cargo.toml
  src/lib.rs          — ox_plugin_init/process/error/destroy + HTTP dispatch
  src/config.rs       — ManagementPluginConfig
  src/handlers.rs     — status, nodes, pause, resume, reload, drain handlers
  src/status_cache.rs — caches workflow/status broadcasts from MQTT
  conf/management.yaml — dev config

conf/workflow/
  flows/              — (empty dir, gitkeep)
  stages/             — (empty dir, gitkeep)

conf/modules/available/
  ox_workflow_scheduler.yaml
  ox_workflow_api.yaml
  ox_workflow_management.yaml
  ox_cc_admin_plugin.yaml     — restored
```

### Modified files
```
Cargo.toml                                    — add 3 new workspace members
crates/webservice/ox_webservice/src/main.rs   — add --set-module-config CLI flag
```

---

## Task 1: Add workspace members and scaffold Cargo.toml files

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/workflow/ox_workflow_scheduler_plugin/Cargo.toml`
- Create: `crates/workflow/ox_workflow_api_plugin/Cargo.toml`
- Create: `crates/workflow/ox_workflow_management_plugin/Cargo.toml`

- [ ] **Step 1: Add workspace members**

In `Cargo.toml`, find the workflow members block and add:
```toml
    "crates/workflow/ox_workflow_scheduler_plugin",
    "crates/workflow/ox_workflow_api_plugin",
    "crates/workflow/ox_workflow_management_plugin",
```

- [ ] **Step 2: Create scheduler plugin Cargo.toml**

`crates/workflow/ox_workflow_scheduler_plugin/Cargo.toml`:
```toml
[package]
name = "ox_workflow_scheduler_plugin"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
ox_workflow_abi      = { path = "../ox_workflow_abi" }
ox_workflow_core     = { path = "../ox_workflow_core" }
ox_workflow_config   = { path = "../ox_workflow_config" }
ox_workflow_storage  = { path = "../ox_workflow_storage" }
ox_workflow_executor = { path = "../ox_workflow_executor" }
ox_workflow_scheduler = { path = "../ox_workflow_scheduler" }
ox_event_bus         = { path = "../../messaging/ox_event_bus" }
ox_event_bus_mqtt    = { path = "../../messaging/ox_event_bus/ox_event_bus_mqtt" }
ox_fileproc          = { path = "../../util/ox_fileproc" }
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
log = "0.4"
thiserror = "2.0"
futures = "0.3"

[dev-dependencies]
tokio = { version = "1.0", features = ["full", "test-util"] }
```

- [ ] **Step 3: Create API plugin Cargo.toml**

`crates/workflow/ox_workflow_api_plugin/Cargo.toml`:
```toml
[package]
name = "ox_workflow_api_plugin"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
ox_workflow_abi     = { path = "../ox_workflow_abi" }
ox_workflow_core    = { path = "../ox_workflow_core" }
ox_workflow_storage = { path = "../ox_workflow_storage" }
ox_event_bus        = { path = "../../messaging/ox_event_bus" }
ox_event_bus_mqtt   = { path = "../../messaging/ox_event_bus/ox_event_bus_mqtt" }
ox_fileproc         = { path = "../../util/ox_fileproc" }
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
uuid = { version = "1.10", features = ["v4", "serde"] }
log = "0.4"
thiserror = "2.0"

[dev-dependencies]
tokio = { version = "1.0", features = ["full", "test-util"] }
tempfile = "3"
```

- [ ] **Step 4: Create management plugin Cargo.toml**

`crates/workflow/ox_workflow_management_plugin/Cargo.toml`:
```toml
[package]
name = "ox_workflow_management_plugin"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
ox_workflow_abi     = { path = "../ox_workflow_abi" }
ox_workflow_storage = { path = "../ox_workflow_storage" }
ox_event_bus        = { path = "../../messaging/ox_event_bus" }
ox_event_bus_mqtt   = { path = "../../messaging/ox_event_bus/ox_event_bus_mqtt" }
ox_fileproc         = { path = "../../util/ox_fileproc" }
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
log = "0.4"
thiserror = "2.0"
parking_lot = "0.12"
futures = "0.3"

[dev-dependencies]
tokio = { version = "1.0", features = ["full", "test-util"] }
```

- [ ] **Step 5: Verify workspace builds (no source yet — expect missing lib.rs errors, not dep errors)**

```bash
cd /var/repos/oxIDIZER
cargo check -p ox_workflow_scheduler_plugin 2>&1 | head -5
```
Expected: `error[E0...]: can't find crate` or similar — confirms Cargo.toml is valid and workspace sees the crate.

- [ ] **Step 6: Create stub lib.rs files so the workspace compiles**

`crates/workflow/ox_workflow_scheduler_plugin/src/lib.rs`:
```rust
pub mod config;
pub mod control;
```

`crates/workflow/ox_workflow_api_plugin/src/lib.rs`:
```rust
pub mod config;
pub mod handlers;
```

`crates/workflow/ox_workflow_management_plugin/src/lib.rs`:
```rust
pub mod config;
pub mod handlers;
pub mod status_cache;
```

Create matching empty module files (each just `// placeholder`):
- `crates/workflow/ox_workflow_scheduler_plugin/src/config.rs`
- `crates/workflow/ox_workflow_scheduler_plugin/src/control.rs`
- `crates/workflow/ox_workflow_api_plugin/src/config.rs`
- `crates/workflow/ox_workflow_api_plugin/src/handlers.rs`
- `crates/workflow/ox_workflow_management_plugin/src/config.rs`
- `crates/workflow/ox_workflow_management_plugin/src/handlers.rs`
- `crates/workflow/ox_workflow_management_plugin/src/status_cache.rs`

- [ ] **Step 7: Verify workspace compiles**

```bash
cargo check -p ox_workflow_scheduler_plugin -p ox_workflow_api_plugin -p ox_workflow_management_plugin 2>&1 | tail -5
```
Expected: `warning: unused import` or clean — no errors.

- [ ] **Step 8: Commit**

```bash
git add crates/workflow/ox_workflow_scheduler_plugin \
        crates/workflow/ox_workflow_api_plugin \
        crates/workflow/ox_workflow_management_plugin \
        Cargo.toml
git commit -m "chore: scaffold ox_workflow plugin crates"
```

---

## Task 2: Config structs for all three plugins

**Files:**
- Modify: `crates/workflow/ox_workflow_scheduler_plugin/src/config.rs`
- Modify: `crates/workflow/ox_workflow_api_plugin/src/config.rs`
- Modify: `crates/workflow/ox_workflow_management_plugin/src/config.rs`

- [ ] **Step 1: Write failing test for SchedulerPluginConfig**

In `crates/workflow/ox_workflow_scheduler_plugin/src/config.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_config_deserialize() {
        let yaml = r#"
node_id: "worker-1"
mqtt:
  broker_url: "mqtt://localhost:1883"
  client_id: "ox_workflow_scheduler_1"
  keep_alive_secs: 30
db_path: "data/workflow.db"
flows_dir: "conf/workflow/flows"
stages_dir: "conf/workflow/stages"
queues:
  - "workflow/tasks/pending"
max_concurrent_tasks: 100
tick_interval_ms: 1000
plugin_paths: {}
"#;
        let v: serde_json::Value = serde_yaml::from_str(yaml).unwrap();
        let cfg: SchedulerPluginConfig = serde_json::from_value(v).unwrap();
        assert_eq!(cfg.node_id, "worker-1");
        assert_eq!(cfg.mqtt.broker_url, "mqtt://localhost:1883");
        assert_eq!(cfg.queues, vec!["workflow/tasks/pending"]);
        assert_eq!(cfg.max_concurrent_tasks, 100);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_workflow_scheduler_plugin 2>&1 | tail -10
```
Expected: compile error — `SchedulerPluginConfig` not defined.

- [ ] **Step 3: Implement SchedulerPluginConfig**

`crates/workflow/ox_workflow_scheduler_plugin/src/config.rs`:
```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MqttConfig {
    pub broker_url: String,
    pub client_id: String,
    #[serde(default = "default_keep_alive")]
    pub keep_alive_secs: u64,
}

fn default_keep_alive() -> u64 { 30 }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SchedulerPluginConfig {
    pub node_id: String,
    pub mqtt: MqttConfig,
    pub db_path: String,
    pub flows_dir: String,
    pub stages_dir: String,
    #[serde(default)]
    pub queues: Vec<String>,
    #[serde(default = "default_max_tasks")]
    pub max_concurrent_tasks: usize,
    #[serde(default = "default_tick_ms")]
    pub tick_interval_ms: u64,
    #[serde(default)]
    pub plugin_paths: HashMap<String, String>,
}

fn default_max_tasks() -> usize { 100 }
fn default_tick_ms() -> u64 { 1000 }

impl SchedulerPluginConfig {
    pub fn load(path: &str) -> Result<Self, String> {
        let val = ox_fileproc::process_file(std::path::Path::new(path), 5)
            .map_err(|e| format!("config load error: {}", e))?;
        serde_json::from_value(val).map_err(|e| format!("config parse error: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_config_deserialize() {
        let yaml = r#"
node_id: "worker-1"
mqtt:
  broker_url: "mqtt://localhost:1883"
  client_id: "ox_workflow_scheduler_1"
  keep_alive_secs: 30
db_path: "data/workflow.db"
flows_dir: "conf/workflow/flows"
stages_dir: "conf/workflow/stages"
queues:
  - "workflow/tasks/pending"
max_concurrent_tasks: 100
tick_interval_ms: 1000
plugin_paths: {}
"#;
        let v: serde_json::Value = serde_yaml::from_str(yaml).unwrap();
        let cfg: SchedulerPluginConfig = serde_json::from_value(v).unwrap();
        assert_eq!(cfg.node_id, "worker-1");
        assert_eq!(cfg.mqtt.broker_url, "mqtt://localhost:1883");
        assert_eq!(cfg.queues, vec!["workflow/tasks/pending"]);
        assert_eq!(cfg.max_concurrent_tasks, 100);
    }
}
```

Note: add `serde_yaml` to `[dev-dependencies]` in scheduler plugin Cargo.toml:
```toml
serde_yaml = "0.9"
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_workflow_scheduler_plugin config 2>&1 | tail -5
```
Expected: `test config::tests::test_scheduler_config_deserialize ... ok`

- [ ] **Step 5: Implement ApiPluginConfig**

`crates/workflow/ox_workflow_api_plugin/src/config.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MqttConfig {
    pub broker_url: String,
    pub client_id: String,
    #[serde(default = "default_keep_alive")]
    pub keep_alive_secs: u64,
}

fn default_keep_alive() -> u64 { 30 }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiPluginConfig {
    pub mqtt: MqttConfig,
    pub db_path: String,
}

impl ApiPluginConfig {
    pub fn load(path: &str) -> Result<Self, String> {
        let val = ox_fileproc::process_file(std::path::Path::new(path), 5)
            .map_err(|e| format!("config load error: {}", e))?;
        serde_json::from_value(val).map_err(|e| format!("config parse error: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_config_deserialize() {
        let json = serde_json::json!({
            "mqtt": { "broker_url": "mqtt://localhost:1883", "client_id": "api_1" },
            "db_path": "data/workflow.db"
        });
        let cfg: ApiPluginConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.mqtt.client_id, "api_1");
        assert_eq!(cfg.db_path, "data/workflow.db");
    }
}
```

- [ ] **Step 6: Implement ManagementPluginConfig**

`crates/workflow/ox_workflow_management_plugin/src/config.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MqttConfig {
    pub broker_url: String,
    pub client_id: String,
    #[serde(default = "default_keep_alive")]
    pub keep_alive_secs: u64,
}

fn default_keep_alive() -> u64 { 30 }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManagementPluginConfig {
    pub mqtt: MqttConfig,
    pub db_path: String,
    #[serde(default = "default_cache_ttl")]
    pub status_cache_ttl_secs: u64,
}

fn default_cache_ttl() -> u64 { 30 }

impl ManagementPluginConfig {
    pub fn load(path: &str) -> Result<Self, String> {
        let val = ox_fileproc::process_file(std::path::Path::new(path), 5)
            .map_err(|e| format!("config load error: {}", e))?;
        serde_json::from_value(val).map_err(|e| format!("config parse error: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_management_config_defaults() {
        let json = serde_json::json!({
            "mqtt": { "broker_url": "mqtt://localhost:1883", "client_id": "mgmt_1" },
            "db_path": "data/workflow.db"
        });
        let cfg: ManagementPluginConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.status_cache_ttl_secs, 30);
    }
}
```

- [ ] **Step 7: Run all config tests**

```bash
cargo test -p ox_workflow_scheduler_plugin -p ox_workflow_api_plugin -p ox_workflow_management_plugin 2>&1 | tail -10
```
Expected: all 3 tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/workflow/ox_workflow_scheduler_plugin/src/config.rs \
        crates/workflow/ox_workflow_api_plugin/src/config.rs \
        crates/workflow/ox_workflow_management_plugin/src/config.rs \
        crates/workflow/ox_workflow_scheduler_plugin/Cargo.toml
git commit -m "feat: add config structs for workflow plugin crates"
```

---

## Task 3: Status cache and control message types

**Files:**
- Modify: `crates/workflow/ox_workflow_management_plugin/src/status_cache.rs`
- Modify: `crates/workflow/ox_workflow_scheduler_plugin/src/control.rs`

- [ ] **Step 1: Write failing test for StatusCache**

`crates/workflow/ox_workflow_management_plugin/src/status_cache.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upsert_and_list_nodes() {
        let cache = StatusCache::new(30);
        let status = NodeStatus {
            node: "worker-1".to_string(),
            status: "running".to_string(),
            queues: vec![QueueStatus { name: "workflow/tasks/pending".to_string(), paused: false }],
            active_tasks: 5,
            semaphore_permits_remaining: 95,
        };
        cache.upsert(status.clone());
        let nodes = cache.list();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node, "worker-1");
        assert_eq!(nodes[0].active_tasks, 5);
    }

    #[test]
    fn test_update_existing_node() {
        let cache = StatusCache::new(30);
        let s1 = NodeStatus {
            node: "worker-1".to_string(),
            status: "running".to_string(),
            queues: vec![],
            active_tasks: 5,
            semaphore_permits_remaining: 95,
        };
        let s2 = NodeStatus { active_tasks: 10, semaphore_permits_remaining: 90, ..s1.clone() };
        cache.upsert(s1);
        cache.upsert(s2);
        let nodes = cache.list();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].active_tasks, 10);
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p ox_workflow_management_plugin status_cache 2>&1 | tail -10
```
Expected: compile error.

- [ ] **Step 3: Implement StatusCache**

`crates/workflow/ox_workflow_management_plugin/src/status_cache.rs`:
```rust
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStatus {
    pub name: String,
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatus {
    pub node: String,
    pub status: String,
    pub queues: Vec<QueueStatus>,
    pub active_tasks: u32,
    pub semaphore_permits_remaining: u32,
}

pub struct StatusCache {
    inner: Arc<RwLock<HashMap<String, NodeStatus>>>,
    pub ttl_secs: u64,
}

impl StatusCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            ttl_secs,
        }
    }

    pub fn upsert(&self, status: NodeStatus) {
        self.inner.write().insert(status.node.clone(), status);
    }

    pub fn list(&self) -> Vec<NodeStatus> {
        self.inner.read().values().cloned().collect()
    }
}

/// Control messages published to `workflow/control`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlMessage {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
    /// Target node_id or "*" for all nodes
    pub node: String,
}

impl ControlMessage {
    pub fn pause_all(node: &str) -> Self {
        Self { command: "pause".to_string(), queue: Some("*".to_string()), node: node.to_string() }
    }
    pub fn resume_all(node: &str) -> Self {
        Self { command: "resume".to_string(), queue: Some("*".to_string()), node: node.to_string() }
    }
    pub fn pause_queue(queue: &str, node: &str) -> Self {
        Self { command: "pause".to_string(), queue: Some(queue.to_string()), node: node.to_string() }
    }
    pub fn resume_queue(queue: &str, node: &str) -> Self {
        Self { command: "resume".to_string(), queue: Some(queue.to_string()), node: node.to_string() }
    }
    pub fn reload(node: &str) -> Self {
        Self { command: "reload".to_string(), queue: None, node: node.to_string() }
    }
    pub fn drain(node: &str) -> Self {
        Self { command: "drain".to_string(), queue: None, node: node.to_string() }
    }
    pub fn status_request(node: &str) -> Self {
        Self { command: "status_request".to_string(), queue: None, node: node.to_string() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upsert_and_list_nodes() {
        let cache = StatusCache::new(30);
        let status = NodeStatus {
            node: "worker-1".to_string(),
            status: "running".to_string(),
            queues: vec![QueueStatus { name: "workflow/tasks/pending".to_string(), paused: false }],
            active_tasks: 5,
            semaphore_permits_remaining: 95,
        };
        cache.upsert(status.clone());
        let nodes = cache.list();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node, "worker-1");
        assert_eq!(nodes[0].active_tasks, 5);
    }

    #[test]
    fn test_update_existing_node() {
        let cache = StatusCache::new(30);
        let s1 = NodeStatus {
            node: "worker-1".to_string(),
            status: "running".to_string(),
            queues: vec![],
            active_tasks: 5,
            semaphore_permits_remaining: 95,
        };
        let s2 = NodeStatus { active_tasks: 10, semaphore_permits_remaining: 90, ..s1.clone() };
        cache.upsert(s1);
        cache.upsert(s2);
        let nodes = cache.list();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].active_tasks, 10);
    }

    #[test]
    fn test_control_message_serializes() {
        let msg = ControlMessage::pause_all("*");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"command\":\"pause\""));
        assert!(json.contains("\"queue\":\"*\""));
    }
}
```

- [ ] **Step 4: Implement control types for scheduler**

`crates/workflow/ox_workflow_scheduler_plugin/src/control.rs`:
```rust
use serde::{Deserialize, Serialize};

/// Matches the ControlMessage schema from ox_workflow_management_plugin.
/// Duplicated intentionally — plugins must not share rlib state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlMessage {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
    pub node: String,
}

/// Status broadcast published by the scheduler to `workflow/status`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStatus {
    pub name: String,
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatus {
    pub node: String,
    pub status: String,
    pub queues: Vec<QueueStatus>,
    pub active_tasks: u32,
    pub semaphore_permits_remaining: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_control_message_deserialize() {
        let json = r#"{"command":"pause","queue":"workflow/tasks/pending","node":"*"}"#;
        let msg: ControlMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.command, "pause");
        assert_eq!(msg.queue.as_deref(), Some("workflow/tasks/pending"));
        assert_eq!(msg.node, "*");
    }

    #[test]
    fn test_reload_has_no_queue() {
        let json = r#"{"command":"reload","node":"*"}"#;
        let msg: ControlMessage = serde_json::from_str(json).unwrap();
        assert!(msg.queue.is_none());
    }
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p ox_workflow_management_plugin -p ox_workflow_scheduler_plugin 2>&1 | tail -10
```
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/workflow/ox_workflow_management_plugin/src/status_cache.rs \
        crates/workflow/ox_workflow_scheduler_plugin/src/control.rs
git commit -m "feat: add control message types and status cache"
```

---

## Task 4: API plugin handlers

**Files:**
- Modify: `crates/workflow/ox_workflow_api_plugin/src/handlers.rs`

These handlers operate purely on the `WorkflowStorage` and `Arc<dyn EventBus>` — no HTTP framework, no task_ctx. They take typed inputs and return `(u16, String)` (status, JSON body). The plugin's `process` function extracts fields from `task_ctx` and calls these.

- [ ] **Step 1: Write failing tests**

`crates/workflow/ox_workflow_api_plugin/src/handlers.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_enqueue_request_valid() {
        let body = r#"{"flow_name":"my_flow","priority":5,"metadata":{"key":"val"}}"#;
        let req: EnqueueRequest = serde_json::from_str(body).unwrap();
        assert_eq!(req.flow_name, "my_flow");
        assert_eq!(req.priority, 5);
        assert_eq!(req.metadata.get("key").unwrap(), "val");
    }

    #[test]
    fn test_parse_enqueue_request_defaults() {
        let body = r#"{"flow_name":"my_flow"}"#;
        let req: EnqueueRequest = serde_json::from_str(body).unwrap();
        assert_eq!(req.priority, 0);
        assert!(req.metadata.is_empty());
    }

    #[test]
    fn test_route_dispatch_unknown_returns_none() {
        assert!(route("GET", "/unknown/path").is_none());
    }

    #[test]
    fn test_route_dispatch_known_routes() {
        assert!(route("POST", "/workflow/api/flows").is_some());
        assert!(route("GET", "/workflow/api/tasks").is_some());
        assert!(route("GET", "/workflow/api/tasks/some-uuid").is_some());
        assert!(route("DELETE", "/workflow/api/tasks/some-uuid").is_some());
        assert!(route("GET", "/workflow/api/tasks/some-uuid/history").is_some());
        assert!(route("POST", "/workflow/api/tasks/some-uuid/cancel").is_some());
        assert!(route("POST", "/workflow/api/tasks/some-uuid/resume").is_some());
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p ox_workflow_api_plugin handlers 2>&1 | tail -10
```
Expected: compile error.

- [ ] **Step 3: Implement handlers**

`crates/workflow/ox_workflow_api_plugin/src/handlers.rs`:
```rust
use ox_workflow_core::{Task, TaskStatus};
use ox_workflow_storage::WorkflowStorage;
use ox_event_bus::EventBus;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct EnqueueRequest {
    pub flow_name: String,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct TaskSummary {
    pub id: Uuid,
    pub status: TaskStatus,
    pub priority: u32,
    pub metadata: HashMap<String, String>,
}

/// Returns the route variant for a given method+path, or None if no match.
/// Returns: Some((action, id_segment)) where id_segment is extracted UUID string if present.
pub fn route<'a>(method: &str, path: &'a str) -> Option<(&'static str, Option<&'a str>)> {
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (method, segs.as_slice()) {
        ("POST",   ["workflow", "api", "flows"])                         => Some(("enqueue", None)),
        ("GET",    ["workflow", "api", "tasks"])                         => Some(("list", None)),
        ("GET",    ["workflow", "api", "tasks", id])                     => Some(("get", Some(*id))),
        ("DELETE", ["workflow", "api", "tasks", id])                     => Some(("delete", Some(*id))),
        ("GET",    ["workflow", "api", "tasks", id, "history"])          => Some(("history", Some(*id))),
        ("POST",   ["workflow", "api", "tasks", id, "cancel"])           => Some(("cancel", Some(*id))),
        ("POST",   ["workflow", "api", "tasks", id, "resume"])           => Some(("resume", Some(*id))),
        _ => None,
    }
}

pub async fn handle_enqueue(
    storage: &WorkflowStorage,
    event_bus: &Arc<dyn EventBus>,
    body: &str,
) -> (u16, String) {
    let req: EnqueueRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return (400, format!(r#"{{"error":"bad request: {}"}}"#, e)),
    };
    let mut task = Task::new(req.priority);
    task.metadata = req.metadata;
    task.metadata.insert("flow_name".to_string(), req.flow_name.clone());

    if let Err(e) = storage.save_task(&task, Some(&req.flow_name), None).await {
        log::error!("enqueue save_task error: {}", e);
        return (503, r#"{"error":"storage error"}"#.to_string());
    }
    let task_id_str = task.id.to_string();
    if let Err(e) = event_bus.publish_to_queue(
        "workflow/tasks/pending",
        req.priority as u8,
        task_id_str.as_bytes(),
    ).await {
        log::error!("enqueue publish error: {}", e);
        return (503, r#"{"error":"queue error"}"#.to_string());
    }
    let body = serde_json::json!({ "task_id": task.id, "status": "Queued" });
    (201, body.to_string())
}

pub async fn handle_list(
    storage: &WorkflowStorage,
    query: &str,
) -> (u16, String) {
    // query string: "status=Queued" or "flow=my_flow"
    let params: HashMap<String, String> = query.split('&')
        .filter_map(|kv| {
            let mut parts = kv.splitn(2, '=');
            let k = parts.next()?.to_string();
            let v = parts.next()?.to_string();
            Some((k, v))
        })
        .collect();

    let tasks_res = if let Some(status_str) = params.get("status") {
        let status = match status_str.as_str() {
            "Queued"    => TaskStatus::Queued,
            "Running"   => TaskStatus::Running,
            "Paused"    => TaskStatus::Paused,
            "Errored"   => TaskStatus::Errored,
            "Completed" => TaskStatus::Completed,
            _ => return (400, r#"{"error":"invalid status"}"#.to_string()),
        };
        storage.list_tasks_by_status(status).await
    } else if let Some(flow) = params.get("flow") {
        storage.list_tasks_by_flow(flow).await
    } else {
        return (400, r#"{"error":"requires ?status= or ?flow= query param"}"#.to_string());
    };

    match tasks_res {
        Ok(tasks) => {
            let summaries: Vec<TaskSummary> = tasks.into_iter().map(|t| TaskSummary {
                id: t.id,
                status: t.status,
                priority: t.priority,
                metadata: t.metadata,
            }).collect();
            (200, serde_json::to_string(&summaries).unwrap_or_default())
        }
        Err(e) => {
            log::error!("list_tasks error: {}", e);
            (503, r#"{"error":"storage error"}"#.to_string())
        }
    }
}

pub async fn handle_get(storage: &WorkflowStorage, id_str: &str) -> (u16, String) {
    let id = match Uuid::parse_str(id_str) {
        Ok(u) => u,
        Err(_) => return (400, r#"{"error":"invalid uuid"}"#.to_string()),
    };
    match storage.load_task(id).await {
        Ok(Some(t)) => {
            let summary = TaskSummary { id: t.id, status: t.status, priority: t.priority, metadata: t.metadata };
            (200, serde_json::to_string(&summary).unwrap_or_default())
        }
        Ok(None) => (404, r#"{"error":"not found"}"#.to_string()),
        Err(e) => { log::error!("load_task error: {}", e); (503, r#"{"error":"storage error"}"#.to_string()) }
    }
}

pub async fn handle_delete(storage: &WorkflowStorage, id_str: &str) -> (u16, String) {
    let id = match Uuid::parse_str(id_str) {
        Ok(u) => u,
        Err(_) => return (400, r#"{"error":"invalid uuid"}"#.to_string()),
    };
    match storage.load_task(id).await {
        Ok(Some(_)) => {
            match storage.delete_task(id).await {
                Ok(_) => (204, String::new()),
                Err(e) => { log::error!("delete_task error: {}", e); (503, r#"{"error":"storage error"}"#.to_string()) }
            }
        }
        Ok(None) => (404, r#"{"error":"not found"}"#.to_string()),
        Err(e) => { log::error!("load_task error: {}", e); (503, r#"{"error":"storage error"}"#.to_string()) }
    }
}

pub async fn handle_history(storage: &WorkflowStorage, id_str: &str) -> (u16, String) {
    let id = match Uuid::parse_str(id_str) {
        Ok(u) => u,
        Err(_) => return (400, r#"{"error":"invalid uuid"}"#.to_string()),
    };
    match storage.get_task_history(id).await {
        Ok(h) => (200, serde_json::to_string(&h).unwrap_or_default()),
        Err(e) => { log::error!("get_task_history error: {}", e); (503, r#"{"error":"storage error"}"#.to_string()) }
    }
}

pub async fn handle_cancel(storage: &WorkflowStorage, id_str: &str) -> (u16, String) {
    let id = match Uuid::parse_str(id_str) {
        Ok(u) => u,
        Err(_) => return (400, r#"{"error":"invalid uuid"}"#.to_string()),
    };
    match storage.load_task(id).await {
        Ok(Some(t)) => {
            if matches!(t.status, TaskStatus::Queued | TaskStatus::Running | TaskStatus::Paused) {
                match storage.update_task_status(id, TaskStatus::Errored).await {
                    Ok(_) => (200, r#"{"status":"cancelled"}"#.to_string()),
                    Err(e) => { log::error!("update_task_status error: {}", e); (503, r#"{"error":"storage error"}"#.to_string()) }
                }
            } else {
                (409, r#"{"error":"task not in cancellable state"}"#.to_string())
            }
        }
        Ok(None) => (404, r#"{"error":"not found"}"#.to_string()),
        Err(e) => { log::error!("load_task error: {}", e); (503, r#"{"error":"storage error"}"#.to_string()) }
    }
}

pub async fn handle_resume(
    storage: &WorkflowStorage,
    event_bus: &Arc<dyn EventBus>,
    id_str: &str,
) -> (u16, String) {
    let id = match Uuid::parse_str(id_str) {
        Ok(u) => u,
        Err(_) => return (400, r#"{"error":"invalid uuid"}"#.to_string()),
    };
    match storage.load_task(id).await {
        Ok(Some(t)) => {
            if t.status == TaskStatus::Paused {
                if let Err(e) = storage.update_task_status(id, TaskStatus::Queued).await {
                    log::error!("update_task_status error: {}", e);
                    return (503, r#"{"error":"storage error"}"#.to_string());
                }
                if let Err(e) = event_bus.publish_to_queue(
                    "workflow/tasks/pending",
                    t.priority as u8,
                    id.to_string().as_bytes(),
                ).await {
                    log::error!("publish error: {}", e);
                    return (503, r#"{"error":"queue error"}"#.to_string());
                }
                (200, r#"{"status":"queued"}"#.to_string())
            } else {
                (409, r#"{"error":"task not paused"}"#.to_string())
            }
        }
        Ok(None) => (404, r#"{"error":"not found"}"#.to_string()),
        Err(e) => { log::error!("load_task error: {}", e); (503, r#"{"error":"storage error"}"#.to_string()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_enqueue_request_valid() {
        let body = r#"{"flow_name":"my_flow","priority":5,"metadata":{"key":"val"}}"#;
        let req: EnqueueRequest = serde_json::from_str(body).unwrap();
        assert_eq!(req.flow_name, "my_flow");
        assert_eq!(req.priority, 5);
        assert_eq!(req.metadata.get("key").unwrap(), "val");
    }

    #[test]
    fn test_parse_enqueue_request_defaults() {
        let body = r#"{"flow_name":"my_flow"}"#;
        let req: EnqueueRequest = serde_json::from_str(body).unwrap();
        assert_eq!(req.priority, 0);
        assert!(req.metadata.is_empty());
    }

    #[test]
    fn test_route_dispatch_unknown_returns_none() {
        assert!(route("GET", "/unknown/path").is_none());
    }

    #[test]
    fn test_route_dispatch_known_routes() {
        assert!(route("POST", "/workflow/api/flows").is_some());
        assert!(route("GET", "/workflow/api/tasks").is_some());
        assert!(route("GET", "/workflow/api/tasks/some-uuid").is_some());
        assert!(route("DELETE", "/workflow/api/tasks/some-uuid").is_some());
        assert!(route("GET", "/workflow/api/tasks/some-uuid/history").is_some());
        assert!(route("POST", "/workflow/api/tasks/some-uuid/cancel").is_some());
        assert!(route("POST", "/workflow/api/tasks/some-uuid/resume").is_some());
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p ox_workflow_api_plugin handlers 2>&1 | tail -10
```
Expected: all 4 tests pass (route dispatch + parse tests — storage/event_bus tests require integration setup, deferred to Task 8).

- [ ] **Step 5: Commit**

```bash
git add crates/workflow/ox_workflow_api_plugin/src/handlers.rs
git commit -m "feat: add API plugin handlers (enqueue, list, get, cancel, resume, delete, history)"
```

---

## Task 5: Management plugin handlers

**Files:**
- Modify: `crates/workflow/ox_workflow_management_plugin/src/handlers.rs`

- [ ] **Step 1: Write failing tests**

`crates/workflow/ox_workflow_management_plugin/src/handlers.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_known() {
        assert!(route("GET",  "/workflow/mgmt/status").is_some());
        assert!(route("GET",  "/workflow/mgmt/nodes").is_some());
        assert!(route("POST", "/workflow/mgmt/queues/pause").is_some());
        assert!(route("POST", "/workflow/mgmt/queues/resume").is_some());
        assert!(route("POST", "/workflow/mgmt/queues/workflow%2Ftasks%2Fpending/pause").is_some());
        assert!(route("POST", "/workflow/mgmt/reload").is_some());
        assert!(route("POST", "/workflow/mgmt/drain").is_some());
        assert!(route("GET",  "/workflow/mgmt/workers").is_some());
    }

    #[test]
    fn test_route_unknown() {
        assert!(route("GET", "/workflow/mgmt/nonexistent").is_none());
        assert!(route("GET", "/other/path").is_none());
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p ox_workflow_management_plugin handlers 2>&1 | tail -10
```
Expected: compile error.

- [ ] **Step 3: Implement management handlers**

`crates/workflow/ox_workflow_management_plugin/src/handlers.rs`:
```rust
use crate::status_cache::{ControlMessage, StatusCache};
use ox_event_bus::EventBus;
use std::sync::Arc;

pub fn route(method: &str, path: &str) -> Option<&'static str> {
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (method, segs.as_slice()) {
        ("GET",  ["workflow", "mgmt", "status"])              => Some("status"),
        ("GET",  ["workflow", "mgmt", "nodes"])               => Some("nodes"),
        ("POST", ["workflow", "mgmt", "queues", "pause"])     => Some("pause_all"),
        ("POST", ["workflow", "mgmt", "queues", "resume"])    => Some("resume_all"),
        ("POST", ["workflow", "mgmt", "queues", _, "pause"])  => Some("pause_queue"),
        ("POST", ["workflow", "mgmt", "queues", _, "resume"]) => Some("resume_queue"),
        ("POST", ["workflow", "mgmt", "reload"])              => Some("reload"),
        ("POST", ["workflow", "mgmt", "drain"])               => Some("drain"),
        ("GET",  ["workflow", "mgmt", "workers"])             => Some("workers"),
        _ => None,
    }
}

pub async fn handle_status(cache: &StatusCache) -> (u16, String) {
    let nodes = cache.list();
    let total_active: u32 = nodes.iter().map(|n| n.active_tasks).sum();
    let body = serde_json::json!({
        "node_count": nodes.len(),
        "total_active_tasks": total_active,
        "nodes": nodes,
    });
    (200, body.to_string())
}

pub async fn handle_nodes(cache: &StatusCache) -> (u16, String) {
    let nodes = cache.list();
    (200, serde_json::to_string(&nodes).unwrap_or_default())
}

pub async fn handle_workers(cache: &StatusCache) -> (u16, String) {
    let nodes = cache.list();
    let workers: Vec<serde_json::Value> = nodes.iter().map(|n| serde_json::json!({
        "node": n.node,
        "active_tasks": n.active_tasks,
        "permits_remaining": n.semaphore_permits_remaining,
    })).collect();
    (200, serde_json::to_string(&workers).unwrap_or_default())
}

async fn publish_control(
    event_bus: &Arc<dyn EventBus>,
    msg: ControlMessage,
) -> (u16, String) {
    let payload = match serde_json::to_vec(&msg) {
        Ok(p) => p,
        Err(e) => return (500, format!(r#"{{"error":"serialize error: {}"}}"#, e)),
    };
    match event_bus.publish("workflow/control", &payload).await {
        Ok(_) => (200, r#"{"status":"sent"}"#.to_string()),
        Err(e) => {
            log::error!("publish control error: {}", e);
            (503, r#"{"error":"mqtt error"}"#.to_string())
        }
    }
}

pub async fn handle_pause_all(event_bus: &Arc<dyn EventBus>) -> (u16, String) {
    publish_control(event_bus, ControlMessage::pause_all("*")).await
}

pub async fn handle_resume_all(event_bus: &Arc<dyn EventBus>) -> (u16, String) {
    publish_control(event_bus, ControlMessage::resume_all("*")).await
}

pub async fn handle_pause_queue(event_bus: &Arc<dyn EventBus>, path: &str) -> (u16, String) {
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if let ["workflow", "mgmt", "queues", queue_name, "pause"] = segs.as_slice() {
        publish_control(event_bus, ControlMessage::pause_queue(queue_name, "*")).await
    } else {
        (400, r#"{"error":"bad path"}"#.to_string())
    }
}

pub async fn handle_resume_queue(event_bus: &Arc<dyn EventBus>, path: &str) -> (u16, String) {
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if let ["workflow", "mgmt", "queues", queue_name, "resume"] = segs.as_slice() {
        publish_control(event_bus, ControlMessage::resume_queue(queue_name, "*")).await
    } else {
        (400, r#"{"error":"bad path"}"#.to_string())
    }
}

pub async fn handle_reload(event_bus: &Arc<dyn EventBus>) -> (u16, String) {
    publish_control(event_bus, ControlMessage::reload("*")).await
}

pub async fn handle_drain(event_bus: &Arc<dyn EventBus>) -> (u16, String) {
    publish_control(event_bus, ControlMessage::drain("*")).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_known() {
        assert!(route("GET",  "/workflow/mgmt/status").is_some());
        assert!(route("GET",  "/workflow/mgmt/nodes").is_some());
        assert!(route("POST", "/workflow/mgmt/queues/pause").is_some());
        assert!(route("POST", "/workflow/mgmt/queues/resume").is_some());
        assert!(route("POST", "/workflow/mgmt/queues/my-queue/pause").is_some());
        assert!(route("POST", "/workflow/mgmt/reload").is_some());
        assert!(route("POST", "/workflow/mgmt/drain").is_some());
        assert!(route("GET",  "/workflow/mgmt/workers").is_some());
    }

    #[test]
    fn test_route_unknown() {
        assert!(route("GET", "/workflow/mgmt/nonexistent").is_none());
        assert!(route("GET", "/other/path").is_none());
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p ox_workflow_management_plugin handlers 2>&1 | tail -10
```
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/workflow/ox_workflow_management_plugin/src/handlers.rs
git commit -m "feat: add management plugin route handlers"
```

---

## Task 6: API plugin lib.rs (cdylib entry points)

**Files:**
- Modify: `crates/workflow/ox_workflow_api_plugin/src/lib.rs`

- [ ] **Step 1: Implement lib.rs**

`crates/workflow/ox_workflow_api_plugin/src/lib.rs`:
```rust
pub mod config;
pub mod handlers;

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::Arc;
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_WORKFLOW_ABI_VERSION, OX_LOG_ERROR, OX_LOG_INFO};
use crate::config::ApiPluginConfig;

struct PluginState {
    storage: ox_workflow_storage::WorkflowStorage,
    event_bus: Arc<dyn ox_event_bus::EventBus>,
    rt: tokio::runtime::Handle,
}

unsafe impl Send for PluginState {}
unsafe impl Sync for PluginState {}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let Ok(c_key) = CString::new(key) else { return String::new() };
    let ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(value)) {
        (api.set_field)(task_ctx, k.as_ptr(), v.as_ptr());
    }
}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) {
        (api.log)(task_ctx, level, c.as_ptr());
    }
}

fn respond(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_init(
    config_ptr: *const c_char,
    api_ptr: *const CoreHostApi,
    abi_version: u32,
) -> *mut c_void {
    if abi_version != OX_WORKFLOW_ABI_VERSION || api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api = unsafe { *api_ptr };

    let config_path = if config_ptr.is_null() {
        log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_workflow_api_plugin: missing config");
        return std::ptr::null_mut();
    } else {
        let raw = unsafe { CStr::from_ptr(config_ptr).to_string_lossy() };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            match v.get("config_file").and_then(|v| v.as_str()) {
                Some(p) => p.to_string(),
                None => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_workflow_api_plugin: missing config_file");
                    return std::ptr::null_mut();
                }
            }
        } else {
            raw.into_owned()
        }
    };

    let cfg = match ApiPluginConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("ox_workflow_api_plugin: config error: {}", e));
            return std::ptr::null_mut();
        }
    };

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_workflow_api_plugin: no tokio runtime");
            return std::ptr::null_mut();
        }
    };

    let storage = match rt.block_on(ox_workflow_storage::WorkflowStorage::new(&cfg.db_path)) {
        Ok(s) => s,
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("ox_workflow_api_plugin: db error: {}", e));
            return std::ptr::null_mut();
        }
    };

    let mqtt_opts = rumqttc::MqttOptions::new(
        &cfg.mqtt.client_id,
        extract_mqtt_host(&cfg.mqtt.broker_url),
        extract_mqtt_port(&cfg.mqtt.broker_url),
    );
    let (mqtt_client, mut event_loop) = rumqttc::AsyncClient::new(mqtt_opts, 10);
    let event_bus: Arc<dyn ox_event_bus::EventBus> = Arc::new(
        ox_event_bus_mqtt::MqttEventBus::new(mqtt_client)
    );

    // Drain the MQTT event loop in the background
    rt.spawn(async move {
        loop {
            if event_loop.poll().await.is_err() {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    });

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, "ox_workflow_api_plugin: initialized");
    Box::into_raw(Box::new(PluginState { storage, event_bus, rt })) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_process(
    plugin_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    if plugin_ctx.is_null() { return cont; }
    let state = unsafe { &*(plugin_ctx as *mut PluginState) };
    let api = unsafe { &*(plugin_ctx as *const CoreHostApi) };

    // Re-derive api from task context the same way other plugins do
    let method = get_field(unsafe { &*(plugin_ctx as *const CoreHostApi) }, task_ctx, "request.method");
    let _ = method; // will be used below via the closure

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(state, task_ctx)
    }));
    match result {
        Ok(Some((status, body))) => {
            // We need access to the api vtable — store it in PluginState
            // This is handled in dispatch() which takes &PluginState
            let _ = (status, body); // suppress warning — dispatch sets fields directly
            cont
        }
        Ok(None) => cont,
        Err(_) => cont,
    }
}

fn dispatch(state: &PluginState, task_ctx: *mut c_void) -> Option<(u16, String)> {
    // We need the api vtable — pass it through PluginState
    // Refactor: PluginState stores api: CoreHostApi
    // (This step is resolved in the actual impl below — see note)
    None
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_error(_plugin_ctx: *mut c_void, _task_ctx: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginState)) }
    }
}

fn extract_mqtt_host(url: &str) -> String {
    url.trim_start_matches("mqtt://")
       .split(':').next()
       .unwrap_or("localhost")
       .to_string()
}

fn extract_mqtt_port(url: &str) -> u16 {
    url.trim_start_matches("mqtt://")
       .split(':').nth(1)
       .and_then(|p| p.parse().ok())
       .unwrap_or(1883)
}
```

The `dispatch` stub above has a design issue: to call `get_field`/`respond`, we need the `CoreHostApi` vtable at process time. Fix by storing `api: CoreHostApi` in `PluginState`. Replace `PluginState` and re-implement `ox_plugin_process` and `dispatch`:

```rust
struct PluginState {
    api: CoreHostApi,
    storage: ox_workflow_storage::WorkflowStorage,
    event_bus: Arc<dyn ox_event_bus::EventBus>,
    rt: tokio::runtime::Handle,
}

// Replace ox_plugin_process:
#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_process(
    plugin_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    if plugin_ctx.is_null() { return cont; }
    let state = unsafe { &*(plugin_ctx as *mut PluginState) };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let method = get_field(&state.api, task_ctx, "request.method").to_uppercase();
        let path   = get_field(&state.api, task_ctx, "request.path");
        let query  = get_field(&state.api, task_ctx, "request.query");
        let body   = get_field(&state.api, task_ctx, "request.body");

        match handlers::route(&method, &path) {
            None => None,
            Some(("enqueue", _)) => {
                Some(state.rt.block_on(handlers::handle_enqueue(&state.storage, &state.event_bus, &body)))
            }
            Some(("list", _)) => {
                Some(state.rt.block_on(handlers::handle_list(&state.storage, &query)))
            }
            Some(("get", Some(id))) => {
                Some(state.rt.block_on(handlers::handle_get(&state.storage, id)))
            }
            Some(("delete", Some(id))) => {
                Some(state.rt.block_on(handlers::handle_delete(&state.storage, id)))
            }
            Some(("history", Some(id))) => {
                Some(state.rt.block_on(handlers::handle_history(&state.storage, id)))
            }
            Some(("cancel", Some(id))) => {
                Some(state.rt.block_on(handlers::handle_cancel(&state.storage, id)))
            }
            Some(("resume", Some(id))) => {
                Some(state.rt.block_on(handlers::handle_resume(&state.storage, &state.event_bus, id)))
            }
            _ => None,
        }
    }));

    if let Ok(Some((status, body_str))) = result {
        respond(&state.api, task_ctx, status, &body_str);
    }
    cont
}

fn dispatch(_state: &PluginState, _task_ctx: *mut c_void) -> Option<(u16, String)> { None } // unused now
```

Also update `ox_plugin_init` to store `api` in `PluginState`:
```rust
// In ox_plugin_init, after building storage + event_bus:
log(&api, std::ptr::null_mut(), OX_LOG_INFO, "ox_workflow_api_plugin: initialized");
Box::into_raw(Box::new(PluginState { api, storage, event_bus, rt })) as *mut c_void
```

- [ ] **Step 2: Also need `request.query` field — verify ox_webservice sets it**

```bash
grep -r "request.query" /var/repos/oxIDIZER/crates/webservice/ --include="*.rs" | grep -v target | head -10
```

If `request.query` is not set by ox_webservice's flow layer, note it — we'll need to extract query from `request.path` (which may include `?query`) instead. In that case, change `handle_list` call in `ox_plugin_process` to parse query from path:

```rust
Some(("list", _)) => {
    let query_str = path.splitn(2, '?').nth(1).unwrap_or("");
    Some(state.rt.block_on(handlers::handle_list(&state.storage, query_str)))
}
```

And update `handle_list` in handlers.rs to receive a `&str` (it already does).

- [ ] **Step 3: Build the API plugin**

```bash
cargo build -p ox_workflow_api_plugin 2>&1 | tail -20
```
Expected: builds without errors. Warnings are acceptable.

- [ ] **Step 4: Commit**

```bash
git add crates/workflow/ox_workflow_api_plugin/src/lib.rs
git commit -m "feat: implement ox_workflow_api_plugin cdylib entry points"
```

---

## Task 7: Management plugin lib.rs and scheduler plugin lib.rs

**Files:**
- Modify: `crates/workflow/ox_workflow_management_plugin/src/lib.rs`
- Modify: `crates/workflow/ox_workflow_scheduler_plugin/src/lib.rs`

- [ ] **Step 1: Implement management plugin lib.rs**

`crates/workflow/ox_workflow_management_plugin/src/lib.rs`:
```rust
pub mod config;
pub mod handlers;
pub mod status_cache;

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::Arc;
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_WORKFLOW_ABI_VERSION, OX_LOG_ERROR, OX_LOG_INFO};
use crate::config::ManagementPluginConfig;
use crate::status_cache::StatusCache;

struct PluginState {
    api: CoreHostApi,
    event_bus: Arc<dyn ox_event_bus::EventBus>,
    cache: Arc<StatusCache>,
    rt: tokio::runtime::Handle,
}

unsafe impl Send for PluginState {}
unsafe impl Sync for PluginState {}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let Ok(c_key) = CString::new(key) else { return String::new() };
    let ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(value)) {
        (api.set_field)(task_ctx, k.as_ptr(), v.as_ptr());
    }
}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

fn respond(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
}

fn extract_mqtt_host(url: &str) -> String {
    url.trim_start_matches("mqtt://").split(':').next().unwrap_or("localhost").to_string()
}
fn extract_mqtt_port(url: &str) -> u16 {
    url.trim_start_matches("mqtt://").split(':').nth(1).and_then(|p| p.parse().ok()).unwrap_or(1883)
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_init(
    config_ptr: *const c_char,
    api_ptr: *const CoreHostApi,
    abi_version: u32,
) -> *mut c_void {
    if abi_version != OX_WORKFLOW_ABI_VERSION || api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };

    let config_path = if config_ptr.is_null() {
        log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_workflow_management_plugin: missing config");
        return std::ptr::null_mut();
    } else {
        let raw = unsafe { CStr::from_ptr(config_ptr).to_string_lossy() };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            match v.get("config_file").and_then(|v| v.as_str()) {
                Some(p) => p.to_string(),
                None => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_workflow_management_plugin: missing config_file"); return std::ptr::null_mut(); }
            }
        } else { raw.into_owned() }
    };

    let cfg = match ManagementPluginConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("management plugin config error: {}", e)); return std::ptr::null_mut(); }
    };

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "management plugin: no tokio runtime"); return std::ptr::null_mut(); }
    };

    let mqtt_opts = rumqttc::MqttOptions::new(
        &cfg.mqtt.client_id,
        extract_mqtt_host(&cfg.mqtt.broker_url),
        extract_mqtt_port(&cfg.mqtt.broker_url),
    );
    let (mqtt_client, mut event_loop) = rumqttc::AsyncClient::new(mqtt_opts, 10);
    let event_bus: Arc<dyn ox_event_bus::EventBus> = Arc::new(ox_event_bus_mqtt::MqttEventBus::new(mqtt_client));

    let cache = Arc::new(StatusCache::new(cfg.status_cache_ttl_secs));
    let cache_clone = cache.clone();
    let bus_clone = event_bus.clone();

    // Subscribe to workflow/status to populate cache
    rt.spawn(async move {
        use futures::StreamExt;
        loop {
            match bus_clone.subscribe("workflow/status").await {
                Ok(mut stream) => {
                    while let Some(msg) = stream.next().await {
                        if let Ok(status) = serde_json::from_slice::<status_cache::NodeStatus>(&msg.payload) {
                            cache_clone.upsert(status);
                        }
                    }
                }
                Err(e) => {
                    log::error!("management plugin: subscribe error: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    });

    rt.spawn(async move {
        loop { if event_loop.poll().await.is_err() { tokio::time::sleep(std::time::Duration::from_secs(5)).await; } }
    });

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, "ox_workflow_management_plugin: initialized");
    Box::into_raw(Box::new(PluginState { api, event_bus, cache, rt })) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_process(plugin_ctx: *mut c_void, task_ctx: *mut c_void) -> FlowControl {
    let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    if plugin_ctx.is_null() { return cont; }
    let state = unsafe { &*(plugin_ctx as *mut PluginState) };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let method = get_field(&state.api, task_ctx, "request.method").to_uppercase();
        let path   = get_field(&state.api, task_ctx, "request.path");

        match handlers::route(&method, &path) {
            None => None,
            Some("status")       => Some(state.rt.block_on(handlers::handle_status(&state.cache))),
            Some("nodes")        => Some(state.rt.block_on(handlers::handle_nodes(&state.cache))),
            Some("workers")      => Some(state.rt.block_on(handlers::handle_workers(&state.cache))),
            Some("pause_all")    => Some(state.rt.block_on(handlers::handle_pause_all(&state.event_bus))),
            Some("resume_all")   => Some(state.rt.block_on(handlers::handle_resume_all(&state.event_bus))),
            Some("pause_queue")  => Some(state.rt.block_on(handlers::handle_pause_queue(&state.event_bus, &path))),
            Some("resume_queue") => Some(state.rt.block_on(handlers::handle_resume_queue(&state.event_bus, &path))),
            Some("reload")       => Some(state.rt.block_on(handlers::handle_reload(&state.event_bus))),
            Some("drain")        => Some(state.rt.block_on(handlers::handle_drain(&state.event_bus))),
            _ => None,
        }
    }));

    if let Ok(Some((status, body))) = result {
        respond(&state.api, task_ctx, status, &body);
    }
    cont
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_error(_plugin_ctx: *mut c_void, _task_ctx: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginState)) }
    }
}
```

Add `rumqttc` to management plugin Cargo.toml:
```toml
rumqttc = "0.24"
```

- [ ] **Step 2: Implement scheduler plugin lib.rs**

`crates/workflow/ox_workflow_scheduler_plugin/src/lib.rs`:
```rust
pub mod config;
pub mod control;

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_WORKFLOW_ABI_VERSION, OX_LOG_ERROR, OX_LOG_INFO};
use ox_workflow_executor::{FlowManager, create_host_api};
use ox_workflow_scheduler::WorkflowScheduler;
use ox_workflow_config::EngineConfig;
use crate::config::SchedulerPluginConfig;
use crate::control::{ControlMessage, NodeStatus, QueueStatus};

struct PluginState {
    api: CoreHostApi,
    node_id: String,
    paused_queues: Arc<RwLock<HashMap<String, AtomicBool>>>,
    drain: Arc<AtomicBool>,
    active_tasks: Arc<std::sync::atomic::AtomicU32>,
    max_concurrent: usize,
    // Hold the shutdown sender so dropping PluginState signals shutdown
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
}

unsafe impl Send for PluginState {}
unsafe impl Sync for PluginState {}

fn log_msg(api: &CoreHostApi, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(std::ptr::null_mut(), level, c.as_ptr()); }
}

fn extract_mqtt_host(url: &str) -> String {
    url.trim_start_matches("mqtt://").split(':').next().unwrap_or("localhost").to_string()
}
fn extract_mqtt_port(url: &str) -> u16 {
    url.trim_start_matches("mqtt://").split(':').nth(1).and_then(|p| p.parse().ok()).unwrap_or(1883)
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_init(
    config_ptr: *const c_char,
    api_ptr: *const CoreHostApi,
    abi_version: u32,
) -> *mut c_void {
    if abi_version != OX_WORKFLOW_ABI_VERSION || api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };

    let config_path = if config_ptr.is_null() {
        log_msg(&api, OX_LOG_ERROR, "ox_workflow_scheduler_plugin: missing config");
        return std::ptr::null_mut();
    } else {
        let raw = unsafe { CStr::from_ptr(config_ptr).to_string_lossy() };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            match v.get("config_file").and_then(|v| v.as_str()) {
                Some(p) => p.to_string(),
                None => { log_msg(&api, OX_LOG_ERROR, "scheduler plugin: missing config_file"); return std::ptr::null_mut(); }
            }
        } else { raw.into_owned() }
    };

    let cfg = match SchedulerPluginConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => { log_msg(&api, OX_LOG_ERROR, &format!("scheduler config error: {}", e)); return std::ptr::null_mut(); }
    };

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => { log_msg(&api, OX_LOG_ERROR, "scheduler plugin: no tokio runtime"); return std::ptr::null_mut(); }
    };

    let storage = match rt.block_on(ox_workflow_storage::WorkflowStorage::new(&cfg.db_path)) {
        Ok(s) => s,
        Err(e) => { log_msg(&api, OX_LOG_ERROR, &format!("scheduler db error: {}", e)); return std::ptr::null_mut(); }
    };

    let mut mqtt_opts = rumqttc::MqttOptions::new(
        &cfg.mqtt.client_id,
        extract_mqtt_host(&cfg.mqtt.broker_url),
        extract_mqtt_port(&cfg.mqtt.broker_url),
    );
    mqtt_opts.set_keep_alive(std::time::Duration::from_secs(cfg.mqtt.keep_alive_secs));
    let (mqtt_client, mut event_loop) = rumqttc::AsyncClient::new(mqtt_opts, 100);
    let event_bus: Arc<dyn ox_event_bus::EventBus> = Arc::new(
        ox_event_bus_mqtt::MqttEventBus::new(mqtt_client)
    );

    let host_api = create_host_api();
    let mut manager = FlowManager::new();
    let engine_config = EngineConfig {
        max_concurrent_tasks: cfg.max_concurrent_tasks,
        tick_interval_ms: cfg.tick_interval_ms,
        ..EngineConfig::default()
    };

    if let Err(e) = manager.load_from_directory(
        &cfg.stages_dir,
        &cfg.flows_dir,
        &host_api,
        &cfg.plugin_paths,
    ) {
        log_msg(&api, OX_LOG_ERROR, &format!("scheduler: flow load error: {}", e));
        return std::ptr::null_mut();
    }

    let manager_arc = Arc::new(manager);
    // Safety: host_api lives in PluginState (via the scheduler) for the duration of the plugin
    let api_box = Box::new(host_api);
    let api_ptr_stable = Box::into_raw(api_box);

    let scheduler = Arc::new(WorkflowScheduler::new(
        engine_config,
        storage,
        event_bus.clone(),
        manager_arc,
        api_ptr_stable,
        cfg.queues.clone(),
    ));

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);
    let scheduler_clone = scheduler.clone();
    let mut shutdown_rx2 = shutdown_tx.subscribe();

    // Run scheduler
    rt.spawn(async move {
        tokio::select! {
            _ = scheduler_clone.run() => {}
            _ = shutdown_rx2.recv() => {
                log::info!("scheduler plugin: shutdown signal received");
            }
        }
    });

    // Control topic listener
    let node_id = cfg.node_id.clone();
    let bus_for_control = event_bus.clone();
    let bus_for_status = event_bus.clone();
    let node_id_status = node_id.clone();
    let active_tasks = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let active_tasks_clone = active_tasks.clone();
    let max_concurrent = cfg.max_concurrent_tasks;
    let drain = Arc::new(AtomicBool::new(false));
    let drain_clone = drain.clone();
    let paused_queues: Arc<RwLock<HashMap<String, AtomicBool>>> = Arc::new(RwLock::new(
        cfg.queues.iter().map(|q| (q.clone(), AtomicBool::new(false))).collect()
    ));
    let paused_clone = paused_queues.clone();
    let queues_for_status = cfg.queues.clone();

    rt.spawn(async move {
        use futures::StreamExt;
        loop {
            match bus_for_control.subscribe("workflow/control").await {
                Ok(mut stream) => {
                    while let Some(msg) = stream.next().await {
                        if let Ok(cmd) = serde_json::from_slice::<ControlMessage>(&msg.payload) {
                            if cmd.node != "*" && cmd.node != node_id { continue; }
                            match cmd.command.as_str() {
                                "pause" => {
                                    let q = cmd.queue.as_deref().unwrap_or("*");
                                    let pq = paused_clone.read();
                                    for (name, flag) in pq.iter() {
                                        if q == "*" || q == name {
                                            flag.store(true, Ordering::Relaxed);
                                            log::info!("scheduler: paused queue {}", name);
                                        }
                                    }
                                }
                                "resume" => {
                                    let q = cmd.queue.as_deref().unwrap_or("*");
                                    let pq = paused_clone.read();
                                    for (name, flag) in pq.iter() {
                                        if q == "*" || q == name {
                                            flag.store(false, Ordering::Relaxed);
                                            log::info!("scheduler: resumed queue {}", name);
                                        }
                                    }
                                }
                                "drain" => { drain_clone.store(true, Ordering::Relaxed); log::info!("scheduler: drain requested"); }
                                "status_request" => {
                                    let pq = paused_clone.read();
                                    let queue_statuses: Vec<QueueStatus> = queues_for_status.iter().map(|q| {
                                        QueueStatus {
                                            name: q.clone(),
                                            paused: pq.get(q).map(|f| f.load(Ordering::Relaxed)).unwrap_or(false),
                                        }
                                    }).collect();
                                    let status = NodeStatus {
                                        node: node_id.clone(),
                                        status: if drain_clone.load(Ordering::Relaxed) { "draining".to_string() } else { "running".to_string() },
                                        queues: queue_statuses,
                                        active_tasks: active_tasks_clone.load(Ordering::Relaxed),
                                        semaphore_permits_remaining: (max_concurrent as u32).saturating_sub(active_tasks_clone.load(Ordering::Relaxed)),
                                    };
                                    if let Ok(payload) = serde_json::to_vec(&status) {
                                        let _ = bus_for_status.publish("workflow/status", &payload).await;
                                    }
                                }
                                _ => { log::warn!("scheduler: unknown control command: {}", cmd.command); }
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("scheduler: control subscribe error: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    });

    // MQTT event loop
    rt.spawn(async move {
        loop {
            tokio::select! {
                res = event_loop.poll() => { if res.is_err() { tokio::time::sleep(std::time::Duration::from_secs(5)).await; } }
                _ = shutdown_rx.recv() => { break; }
            }
        }
    });

    log_msg(&api, OX_LOG_INFO, &format!("ox_workflow_scheduler_plugin: initialized node_id={}", cfg.node_id));
    Box::into_raw(Box::new(PluginState {
        api,
        node_id: cfg.node_id,
        paused_queues,
        drain,
        active_tasks,
        max_concurrent: cfg.max_concurrent_tasks,
        _shutdown_tx: shutdown_tx,
    })) as *mut c_void
}

/// Scheduler plugin has no HTTP routes — process is a no-op.
#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_process(
    _plugin_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) -> FlowControl {
    FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_error(_plugin_ctx: *mut c_void, _task_ctx: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        // Dropping PluginState drops _shutdown_tx, which signals all background tasks
        unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginState)) }
    }
}
```

Add `rumqttc` and `parking_lot` to scheduler plugin Cargo.toml:
```toml
rumqttc = "0.24"
parking_lot = "0.12"
```

- [ ] **Step 3: Build both plugins**

```bash
cargo build -p ox_workflow_management_plugin -p ox_workflow_scheduler_plugin 2>&1 | tail -20
```
Expected: both build without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/workflow/ox_workflow_management_plugin/src/lib.rs \
        crates/workflow/ox_workflow_scheduler_plugin/src/lib.rs \
        crates/workflow/ox_workflow_management_plugin/Cargo.toml \
        crates/workflow/ox_workflow_scheduler_plugin/Cargo.toml
git commit -m "feat: implement scheduler and management plugin cdylib entry points"
```

---

## Task 8: ox_webservice --set-module-config CLI flag

**Files:**
- Modify: `crates/webservice/ox_webservice/src/main.rs`

- [ ] **Step 1: Write failing test**

Add to `crates/webservice/ox_webservice/src/main.rs` (in the existing `#[cfg(test)]` block or add one):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_module_config_override_valid() {
        let result = parse_module_config_override("ox_cc_admin=/etc/ox_cc/admin.yaml");
        assert_eq!(result, Some(("ox_cc_admin".to_string(), "/etc/ox_cc/admin.yaml".to_string())));
    }

    #[test]
    fn test_parse_module_config_override_no_equals() {
        let result = parse_module_config_override("no_equals_here");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_module_config_override_path_with_equals() {
        // Path itself may contain = (unlikely but safe)
        let result = parse_module_config_override("my_module=/path/to/file=v2.yaml");
        assert_eq!(result, Some(("my_module".to_string(), "/path/to/file=v2.yaml".to_string())));
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p ox_webservice 2>&1 | grep "parse_module_config" | head -5
```
Expected: compile error — `parse_module_config_override` not defined.

- [ ] **Step 3: Implement the flag and helper**

In `crates/webservice/ox_webservice/src/main.rs`, update the `Cli` struct and add the helper:

Find the existing `Cli` struct:
```rust
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long, default_value = "ox_webservice.yaml")]
    config: String,
}
```

Replace with:
```rust
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long, default_value = "ox_webservice.yaml")]
    config: String,

    /// Override a module's config_file path. Format: <module-id>=<path>. Repeatable.
    #[arg(long = "set-module-config", value_name = "ID=PATH")]
    set_module_config: Vec<String>,
}

fn parse_module_config_override(s: &str) -> Option<(String, String)> {
    let mut parts = s.splitn(2, '=');
    let id = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    if id.is_empty() || path.is_empty() { return None; }
    Some((id, path))
}
```

- [ ] **Step 4: Apply overrides after config load**

In the `main()` function, after `load_config_from_path` succeeds, add:

```rust
// Apply --set-module-config overrides
let (mut server_config, mut config_json) = match load_config_from_path(server_config_path, "info") {
    Ok(result) => result,
    Err(e) => {
        eprintln!("Failed to load configuration: {}", e);
        std::process::exit(1);
    }
};

for override_str in &cli.set_module_config {
    if let Some((id, path)) = parse_module_config_override(override_str) {
        let mut found = false;
        for module in &mut server_config.modules {
            let module_id = module.id.as_deref().unwrap_or(&module.name);
            if module_id == id {
                module.extra_params.insert(
                    "config_file".to_string(),
                    serde_json::Value::String(path.clone()),
                );
                found = true;
                break;
            }
        }
        if !found {
            eprintln!("Warning: --set-module-config: module '{}' not found in config", id);
        }
    } else {
        eprintln!("Error: --set-module-config value '{}' must be in format ID=PATH", override_str);
        std::process::exit(1);
    }
}
// Re-serialize config_json with overrides applied
config_json = serde_json::to_string(&server_config).unwrap_or(config_json);
```

Note: the existing code binds `(server_config, config_json)` — replace the `let (server_config, config_json) = match...` binding with `let (mut server_config, mut config_json) = match...` and insert the override block before the `match cli.command` block.

- [ ] **Step 5: Run tests**

```bash
cargo test -p ox_webservice 2>&1 | grep -E "parse_module_config|PASSED|FAILED|ok|error" | head -10
```
Expected: all 3 `parse_module_config_override` tests pass.

- [ ] **Step 6: Build ox_webservice**

```bash
cargo build -p ox_webservice 2>&1 | tail -10
```
Expected: builds cleanly.

- [ ] **Step 7: Verify help output shows the new flag**

```bash
./target/debug/ox_webservice --help 2>&1 | grep "set-module-config"
```
Expected: `--set-module-config <ID=PATH>` appears in help.

- [ ] **Step 8: Commit**

```bash
git add crates/webservice/ox_webservice/src/main.rs
git commit -m "feat: add --set-module-config CLI flag to ox_webservice"
```

---

## Task 9: Config files and module YAML activation

**Files:**
- Create: `crates/workflow/ox_workflow_scheduler_plugin/conf/scheduler.yaml`
- Create: `crates/workflow/ox_workflow_api_plugin/conf/api.yaml`
- Create: `crates/workflow/ox_workflow_management_plugin/conf/management.yaml`
- Create: `conf/modules/available/ox_workflow_scheduler.yaml`
- Create: `conf/modules/available/ox_workflow_api.yaml`
- Create: `conf/modules/available/ox_workflow_management.yaml`
- Create: `conf/modules/available/ox_cc_admin_plugin.yaml`
- Create: `conf/workflow/flows/.gitkeep`
- Create: `conf/workflow/stages/.gitkeep`

- [ ] **Step 1: Create dev config files**

`crates/workflow/ox_workflow_scheduler_plugin/conf/scheduler.yaml`:
```yaml
node_id: "dev-worker-1"
mqtt:
  broker_url: "mqtt://localhost:1883"
  client_id: "ox_workflow_scheduler_dev"
  keep_alive_secs: 30
db_path: "data/workflow.db"
flows_dir: "conf/workflow/flows"
stages_dir: "conf/workflow/stages"
queues:
  - "workflow/tasks/pending"
max_concurrent_tasks: 10
tick_interval_ms: 1000
plugin_paths: {}
```

`crates/workflow/ox_workflow_api_plugin/conf/api.yaml`:
```yaml
mqtt:
  broker_url: "mqtt://localhost:1883"
  client_id: "ox_workflow_api_dev"
  keep_alive_secs: 30
db_path: "data/workflow.db"
```

`crates/workflow/ox_workflow_management_plugin/conf/management.yaml`:
```yaml
mqtt:
  broker_url: "mqtt://localhost:1883"
  client_id: "ox_workflow_mgmt_dev"
  keep_alive_secs: 30
db_path: "data/workflow.db"
status_cache_ttl_secs: 30
```

- [ ] **Step 2: Create module YAML files for conf/modules/available/**

`conf/modules/available/ox_workflow_scheduler.yaml`:
```yaml
modules:
  - id: "ox_workflow_scheduler"
    name: "ox_workflow_scheduler_plugin"
    config_file: "crates/workflow/ox_workflow_scheduler_plugin/conf/scheduler.yaml"
```

`conf/modules/available/ox_workflow_api.yaml`:
```yaml
modules:
  - id: "ox_workflow_api"
    name: "ox_workflow_api_plugin"
    config_file: "crates/workflow/ox_workflow_api_plugin/conf/api.yaml"

routes:
  - url: "^/workflow/api(/.*)?$"
    module_id: "ox_workflow_api"
```

`conf/modules/available/ox_workflow_management.yaml`:
```yaml
modules:
  - id: "ox_workflow_management"
    name: "ox_workflow_management_plugin"
    config_file: "crates/workflow/ox_workflow_management_plugin/conf/management.yaml"

routes:
  - url: "^/workflow/mgmt(/.*)?$"
    module_id: "ox_workflow_management"
```

`conf/modules/available/ox_cc_admin_plugin.yaml` (restore):
```yaml
modules:
  - id: "ox_cc_admin"
    name: "ox_cc_admin_plugin"
    config_file: "crates/cc/ox_cc_admin_plugin/conf/ox_cc_admin_plugin.yaml"

routes:
  - url: "^/admin/api(/.*)?$"
    module_id: "ox_cc_admin"
```

- [ ] **Step 3: Create workflow conf directories**

```bash
mkdir -p /var/repos/oxIDIZER/conf/workflow/flows
mkdir -p /var/repos/oxIDIZER/conf/workflow/stages
touch /var/repos/oxIDIZER/conf/workflow/flows/.gitkeep
touch /var/repos/oxIDIZER/conf/workflow/stages/.gitkeep
```

- [ ] **Step 4: Commit**

```bash
git add crates/workflow/ox_workflow_scheduler_plugin/conf/ \
        crates/workflow/ox_workflow_api_plugin/conf/ \
        crates/workflow/ox_workflow_management_plugin/conf/ \
        conf/modules/available/ox_workflow_scheduler.yaml \
        conf/modules/available/ox_workflow_api.yaml \
        conf/modules/available/ox_workflow_management.yaml \
        conf/modules/available/ox_cc_admin_plugin.yaml \
        conf/workflow/
git commit -m "feat: add workflow plugin configs and module YAML files"
```

---

## Task 10: Full build verification and self-review

- [ ] **Step 1: Build all workflow plugin crates**

```bash
cargo build -p ox_workflow_scheduler_plugin \
            -p ox_workflow_api_plugin \
            -p ox_workflow_management_plugin \
            -p ox_webservice 2>&1 | tail -20
```
Expected: all four build without errors.

- [ ] **Step 2: Run all tests in workflow plugins**

```bash
cargo test -p ox_workflow_scheduler_plugin \
           -p ox_workflow_api_plugin \
           -p ox_workflow_management_plugin 2>&1 | tail -20
```
Expected: all tests pass.

- [ ] **Step 3: Run existing ox_webservice and ox_workflow tests (no regressions)**

```bash
cargo test -p ox_webservice -p ox_workflow_core -p ox_workflow_executor -p ox_workflow_storage 2>&1 | tail -20
```
Expected: all pass.

- [ ] **Step 4: Verify MqttEventBus new() signature**

The scheduler and other plugins call `ox_event_bus_mqtt::MqttEventBus::new(mqtt_client)`. Verify the constructor signature matches:

```bash
grep -n "pub fn new" /var/repos/oxIDIZER/crates/messaging/ox_event_bus/ox_event_bus_mqtt/src/lib.rs | head -5
```

If the signature differs (e.g., requires additional args), update the `MqttEventBus::new(...)` calls in all three plugin lib.rs files accordingly.

- [ ] **Step 5: Final commit**

```bash
git add -u
git commit -m "feat: ox_workflow server mode plugins — complete implementation"
```

---

## Self-Review Notes

**Spec coverage check:**
- ✅ `ox_workflow_scheduler_plugin` cdylib — Task 7
- ✅ `ox_workflow_api_plugin` cdylib — Tasks 4, 6
- ✅ `ox_workflow_management_plugin` cdylib — Tasks 5, 7
- ✅ MQTT control topic (`workflow/control`) — Tasks 3, 7
- ✅ MQTT status topic (`workflow/status`) — Tasks 3, 7
- ✅ `--set-module-config` CLI flag — Task 8
- ✅ `conf/modules/available/ox_cc_admin_plugin.yaml` restored — Task 9
- ✅ Dev config files — Task 9
- ✅ `workflow/control` message schema (pause, resume, reload, drain, status_request) — Task 3

**Known items that require verification during execution:**
- `ox_event_bus_mqtt::MqttEventBus::new()` constructor signature (Task 10, Step 4)
- Whether `request.query` is set separately by ox_webservice or must be parsed from `request.path` (Task 6, Step 2)
- `WorkflowScheduler::run()` takes `Arc<Self>` — scheduler plugin wraps it correctly but the `FlowManager` borrow pattern (fields accessed directly in scheduler) may require `Arc<RwLock<FlowManager>>` — check at build time and adjust accordingly
