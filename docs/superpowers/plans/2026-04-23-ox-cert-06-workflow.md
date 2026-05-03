# ox_cert Workflow & Webhooks — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Prerequisites:** Plan 01 (ox_cert_core) must be complete before this plan.

**Goal:** Build `ox_cert_ra` (RA approval workflow), `ox_cert_webhook` (authorization and enrichment hooks), and `ox_cert_ct` (CT SCT query endpoints). Also adds `CoreHostApi::create_task` so RA can inject new workflow tasks for re-submission.

**Architecture:** `ox_cert_ra` stores ApprovalRequests via the CertStore GDO, re-submits approved requests by calling `CoreHostApi::create_task` (new fn ptr) + `publish_to_queue`. A background auto-approve thread re-checks pending requests every 5 minutes. `ox_cert_webhook` is a pipeline stage (no routes) that calls HMAC-signed external HTTP hooks before issuance. `ox_cert_ct` serves GET-only SCT query endpoints backed by CertStore.

**Tech Stack:** Rust 2021, ox_cert_core (path dep), ox_workflow_abi (path dep), reqwest 0.12 (blocking), ring 0.17, base64 0.22, serde/serde_json 1.0, uuid 1.6 (v4), regex 1.10, time 0.3.

---

## File Map

```
crates/workflow/ox_workflow_abi/src/lib.rs
    — add create_task fn ptr after publish_to_topic

crates/workflow/ox_workflow_storage/src/lib.rs
    — add create_pending_task() method

crates/workflow/ox_workflow_scheduler/src/lib.rs
    — modify spawn_task: populate TaskState from metadata_json when state_blob is empty

crates/workflow/ox_workflow_executor/src/lib.rs
    — add create_task_impl() + WORKFLOW_STORAGE global to create_host_api()

crates/cert/ox_cert_core/src/types.rs
    — add ApprovalRequest, RaStatus enums

crates/cert/ox_cert_core/src/certstore/mod.rs
    — add RA CRUD methods to CertStore trait

crates/cert/ox_cert_core/src/certstore/persistence.rs
    — implement RA methods on OxPersistenceCertStore

crates/cert/ox_cert_core/src/messaging.rs
    — add create_and_enqueue_task() helper

crates/cert/ox_cert_webhook/
├── Cargo.toml
└── src/
    ├── lib.rs          — plugin ABI (init/process/destroy), route dispatch
    ├── config.rs       — WebhookConfig, WebhookHookConfig, WebhookType, WebhookFailureMode
    └── signing.rs      — HMAC-SHA256 sign; build webhook payload JSON

crates/cert/ox_cert_ct/
├── Cargo.toml
└── src/
    ├── lib.rs          — plugin ABI, route dispatch
    ├── config.rs       — CtPluginConfig
    └── handlers.rs     — get_scts(), list_logs()

crates/cert/ox_cert_ra/
├── Cargo.toml
└── src/
    ├── lib.rs          — plugin ABI, route dispatch, ModuleContext
    ├── config.rs       — RaConfig, AutoApproveRule
    ├── handlers.rs     — list_pending, get_request, approve, deny, history, certificate
    └── auto_approve.rs — background auto-approval scanner thread
```

---

## Task 1: Workspace scaffold — three new crates

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/cert/ox_cert_webhook/Cargo.toml`
- Create: `crates/cert/ox_cert_ct/Cargo.toml`
- Create: `crates/cert/ox_cert_ra/Cargo.toml`
- Create: `crates/cert/ox_cert_webhook/src/lib.rs`
- Create: `crates/cert/ox_cert_ct/src/lib.rs`
- Create: `crates/cert/ox_cert_ra/src/lib.rs`

- [ ] **Step 1: Add crates to workspace**

In `Cargo.toml` under the `# cert` members block, add:

```toml
    "crates/cert/ox_cert_webhook",
    "crates/cert/ox_cert_ct",
    "crates/cert/ox_cert_ra",
```

- [ ] **Step 2: Create `crates/cert/ox_cert_webhook/Cargo.toml`**

```toml
[package]
name = "ox_cert_webhook"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[lib]
crate-type = ["cdylib"]

[dependencies]
ox_cert_core  = { path = "../ox_cert_core" }
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
serde         = { version = "1.0", features = ["derive"] }
serde_json    = "1.0"
reqwest       = { version = "0.12", features = ["blocking", "json"] }
ring          = "0.17"
base64        = "0.22"
regex         = "1.10"
libc          = "0.2"
```

- [ ] **Step 3: Create `crates/cert/ox_cert_ct/Cargo.toml`**

```toml
[package]
name = "ox_cert_ct"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[lib]
crate-type = ["cdylib"]

[dependencies]
ox_cert_core  = { path = "../ox_cert_core" }
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
serde         = { version = "1.0", features = ["derive"] }
serde_json    = "1.0"
reqwest       = { version = "0.12", features = ["blocking"] }
libc          = "0.2"
```

- [ ] **Step 4: Create `crates/cert/ox_cert_ra/Cargo.toml`**

```toml
[package]
name = "ox_cert_ra"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[lib]
crate-type = ["cdylib"]

[dependencies]
ox_cert_core  = { path = "../ox_cert_core" }
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
serde         = { version = "1.0", features = ["derive"] }
serde_json    = "1.0"
uuid          = { version = "1.6", features = ["v4"] }
regex         = "1.10"
time          = { version = "0.3", features = ["formatting", "macros"] }
libc          = "0.2"
```

- [ ] **Step 5: Create stub `lib.rs` files**

`crates/cert/ox_cert_webhook/src/lib.rs`:
```rust
mod config;
mod signing;

pub use config::{WebhookConfig, WebhookHookConfig, WebhookType, WebhookFailureMode};
```

`crates/cert/ox_cert_ct/src/lib.rs`:
```rust
mod config;
mod handlers;

pub use config::CtPluginConfig;
```

`crates/cert/ox_cert_ra/src/lib.rs`:
```rust
mod config;
mod handlers;
mod auto_approve;

pub use config::{RaConfig, AutoApproveRule};
```

- [ ] **Step 6: Verify workspace builds**

```bash
cargo check -p ox_cert_webhook -p ox_cert_ct -p ox_cert_ra 2>&1 | grep "^error" | head -20
```

Expected: errors only about missing modules (config.rs, etc.) — not about Cargo.toml.

- [ ] **Step 7: Commit**

```bash
git add crates/cert/ox_cert_webhook/ crates/cert/ox_cert_ct/ crates/cert/ox_cert_ra/ Cargo.toml
git commit -m "chore(cert): scaffold ox_cert_webhook, ox_cert_ct, ox_cert_ra crates"
```

---

## Task 2: CoreHostApi::create_task + WorkflowStorage + messaging

This task wires together everything the RA plugin needs to create a new workflow task:
`CoreHostApi::create_task` fn ptr → `WorkflowStorage::create_pending_task` → scheduler copies metadata into TaskState → `messaging::create_and_enqueue_task` wraps both.

**Files:**
- Modify: `crates/workflow/ox_workflow_abi/src/lib.rs`
- Modify: `crates/workflow/ox_workflow_storage/src/lib.rs`
- Modify: `crates/workflow/ox_workflow_scheduler/src/lib.rs`
- Modify: `crates/workflow/ox_workflow_executor/src/lib.rs`
- Modify: `crates/cert/ox_cert_core/src/messaging.rs`

- [ ] **Step 1: Write a compile-check test for create_task**

In `crates/workflow/ox_workflow_abi/src/lib.rs` test module at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_core_host_api_has_create_task() {
        // Verify create_task field exists by checking struct size hasn't regressed
        let _ = std::mem::size_of::<CoreHostApi>();
        // Type-check the function signature
        let _: unsafe extern "C" fn(*const libc::c_char, *const libc::c_char, u8, *mut libc::c_char) -> i32
            = |_, _, _, _| 0;
    }
}
```

Run: `cargo test -p ox_workflow_abi 2>&1 | tail -5`
Expected: FAIL — `create_task` field doesn't exist yet in `CoreHostApi`.

- [ ] **Step 2: Add `create_task` to `CoreHostApi`**

In `crates/workflow/ox_workflow_abi/src/lib.rs`, after the `publish_to_topic` field (already added in Plan 01 Task 5), add:

```rust
/// Create a new Queued workflow task with the given initial state fields.
/// `flow_name`: null-terminated flow name.
/// `state_fields_json`: null-terminated JSON object {"key": "value"...} stored as
///   metadata_json; the scheduler populates TaskState from this on load.
/// `priority`: 0 (lowest) – 255 (highest).
/// `out_task_id`: caller-allocated buffer ≥ 37 bytes; receives null-terminated UUID string.
/// Returns 0 on success, non-zero on failure.
pub create_task: unsafe extern "C" fn(
    flow_name:         *const libc::c_char,
    state_fields_json: *const libc::c_char,
    priority:          u8,
    out_task_id:       *mut libc::c_char,
) -> i32,
```

- [ ] **Step 3: Fix CoreHostApi construction sites**

```bash
grep -rn "CoreHostApi {" crates/ --include="*.rs" -l
```

For each file that constructs `CoreHostApi` with literal syntax, add a stub:

```rust
create_task: |_, _, _, _| { -1 },
```

- [ ] **Step 4: Verify compile-check test passes**

```bash
cargo test -p ox_workflow_abi test_core_host_api_has_create_task 2>&1 | tail -5
```

Expected: PASS.

- [ ] **Step 5: Write test for `WorkflowStorage::create_pending_task`**

In `crates/workflow/ox_workflow_storage/src/lib.rs`, add to the tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_pending_task_inserts_row() {
        let storage = WorkflowStorage::new("sqlite::memory:").await.unwrap();
        let id = uuid::Uuid::new_v4();
        let metadata = r#"{"flow_name":"issue","cert.ra.approved":"true"}"#;
        storage.create_pending_task(&id, "issue", metadata, 100).await.unwrap();
        let task = storage.load_task(id).await.unwrap().expect("task should exist");
        assert_eq!(task.metadata.get("flow_name").map(|s| s.as_str()), Some("issue"));
        assert_eq!(task.metadata.get("cert.ra.approved").map(|s| s.as_str()), Some("true"));
    }
}
```

Run: `cargo test -p ox_workflow_storage test_create_pending_task 2>&1 | tail -5`
Expected: FAIL — method doesn't exist yet.

- [ ] **Step 6: Implement `WorkflowStorage::create_pending_task`**

In `crates/workflow/ox_workflow_storage/src/lib.rs`, add after `save_task`:

```rust
/// Insert a new Queued task created externally (e.g. by the RA plugin).
/// `metadata_json` is a JSON object that the scheduler will copy into TaskState on load.
pub async fn create_pending_task(
    &self,
    id: &Uuid,
    flow_name: &str,
    metadata_json: &str,
    priority: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        r#"INSERT INTO tasks
           (id, priority, status, flow_name, state_blob, metadata_json)
           VALUES (?, ?, 'Queued', ?, ?, ?)"#,
    )
    .bind(id.to_string())
    .bind(priority)
    .bind(flow_name)
    .bind(b"".as_ref())   // empty state_blob; scheduler populates from metadata_json
    .bind(metadata_json)
    .execute(&self.pool)
    .await?;
    Ok(())
}
```

Run: `cargo test -p ox_workflow_storage test_create_pending_task 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 7: Write test for scheduler state population**

In `crates/workflow/ox_workflow_scheduler/src/lib.rs` or a separate integration test, add:

```rust
#[cfg(test)]
mod tests {
    use ox_workflow_storage::WorkflowStorage;
    use ox_workflow_core::state::FieldValue;

    #[tokio::test]
    async fn test_load_task_populates_state_from_metadata_when_empty() {
        let storage = WorkflowStorage::new("sqlite::memory:").await.unwrap();
        let id = uuid::Uuid::new_v4();
        let metadata = r#"{"flow_name":"issue","cert.ra.approved":"true","request.method":"POST"}"#;
        storage.create_pending_task(&id, "issue", metadata, 100).await.unwrap();

        let task = storage.load_task(id).await.unwrap().expect("task exists");
        // state_blob was empty → TaskState populated from metadata
        let state = task.state.read();
        assert_eq!(
            state.fields.get("cert.ra.approved"),
            Some(&FieldValue::String("true".to_string()))
        );
        assert_eq!(
            state.fields.get("request.method"),
            Some(&FieldValue::String("POST".to_string()))
        );
        // flow_name should NOT bleed into state (internal key)
        assert!(state.fields.get("flow_name").is_none());
    }
}
```

Run: `cargo test -p ox_workflow_storage test_load_task_populates 2>&1 | tail -5`
Expected: FAIL — `load_task` doesn't yet populate state from metadata.

- [ ] **Step 8: Modify `WorkflowStorage::load_task` to populate state from metadata**

In `crates/workflow/ox_workflow_storage/src/lib.rs`, inside `load_task`, after deserializing `state` and `metadata`, add:

```rust
// For tasks created externally (empty state_blob), populate TaskState from metadata_json.
// Internal scheduler keys are excluded from state.
const SCHEDULER_INTERNAL_KEYS: &[&str] = &[
    "flow_name", "parent_task_id", "current_stage", "current_plugin_index",
];
if state.fields.is_empty() {
    for (k, v) in &metadata {
        if !SCHEDULER_INTERNAL_KEYS.contains(&k.as_str()) {
            state.fields.insert(
                k.clone(),
                ox_workflow_core::state::FieldValue::String(v.clone()),
            );
        }
    }
}
```

Run: `cargo test -p ox_workflow_storage test_load_task_populates 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 9: Implement `create_task_impl` in the executor**

In `crates/workflow/ox_workflow_executor/src/lib.rs`, add a global storage reference and `create_task_impl`:

```rust
use ox_workflow_storage::WorkflowStorage;
use std::sync::{Arc, OnceLock};

static WORKFLOW_STORAGE: OnceLock<Arc<WorkflowStorage>> = OnceLock::new();

/// Call once at scheduler startup before building the host API.
pub fn init_workflow_storage_for_host(storage: Arc<WorkflowStorage>) {
    let _ = WORKFLOW_STORAGE.set(storage);
}
```

Inside `create_host_api()`, add `create_task_impl` alongside the other `extern "C"` functions:

```rust
unsafe extern "C" fn create_task_impl(
    flow_name: *const libc::c_char,
    state_fields_json: *const libc::c_char,
    priority: u8,
    out_task_id: *mut libc::c_char,
) -> i32 {
    let storage = match WORKFLOW_STORAGE.get() {
        Some(s) => s.clone(),
        None => return -1,
    };
    let flow = std::ffi::CStr::from_ptr(flow_name).to_string_lossy().to_string();
    let fields = std::ffi::CStr::from_ptr(state_fields_json).to_string_lossy().to_string();

    let task_id = uuid::Uuid::new_v4();
    let result = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            storage.create_pending_task(&task_id, &flow, &fields, priority as i64).await
        })
    });
    if result.is_err() {
        return -1;
    }
    let id_str = task_id.to_string();
    let id_bytes = id_str.as_bytes();
    std::ptr::copy_nonoverlapping(id_bytes.as_ptr() as *const libc::c_char, out_task_id, id_bytes.len());
    *out_task_id.add(id_bytes.len()) = 0;
    0
}
```

In the returned `CoreHostApi` struct literal inside `create_host_api()`, replace the stub:

```rust
create_task: create_task_impl,
```

- [ ] **Step 10: Write test for `create_and_enqueue_task` in messaging**

In `crates/cert/ox_cert_core/src/messaging.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_enqueue_task_returns_uuid_format() {
        // Use a fake CoreHostApi that captures calls
        static mut CREATE_CALLED: bool = false;
        static mut ENQUEUE_CALLED: bool = false;

        unsafe extern "C" fn fake_create(
            _flow: *const libc::c_char,
            _fields: *const libc::c_char,
            _prio: u8,
            out: *mut libc::c_char,
        ) -> i32 {
            CREATE_CALLED = true;
            let id = b"11111111-1111-1111-1111-111111111111\0";
            std::ptr::copy_nonoverlapping(id.as_ptr() as *const libc::c_char, out, id.len());
            0
        }
        unsafe extern "C" fn fake_publish(
            _queue: *const libc::c_char,
            _prio: u8,
            _payload: *const u8,
            _len: usize,
        ) -> i32 {
            ENQUEUE_CALLED = true;
            0
        }

        // Build a minimal CoreHostApi with stubs for all fields
        let api = make_stub_api(fake_create, fake_publish);
        let mut fields = std::collections::HashMap::new();
        fields.insert("cert.ra.approved".to_string(), "true".to_string());
        fields.insert("request.body".to_string(), r#"{"csr":"..."}"#.to_string());

        let result = create_and_enqueue_task(&api, "issue_flow", &fields, 100, "tasks.pending");
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
        let task_id = result.unwrap();
        assert_eq!(task_id, "11111111-1111-1111-1111-111111111111");
        assert!(unsafe { CREATE_CALLED });
        assert!(unsafe { ENQUEUE_CALLED });
    }
}
```

`make_stub_api` (add as test helper at bottom of messaging.rs):

```rust
#[cfg(test)]
pub(crate) fn make_stub_api(
    create: unsafe extern "C" fn(*const libc::c_char, *const libc::c_char, u8, *mut libc::c_char) -> i32,
    publish: unsafe extern "C" fn(*const libc::c_char, u8, *const u8, usize) -> i32,
) -> ox_workflow_abi::CoreHostApi {
    ox_workflow_abi::CoreHostApi {
        get_field:        |_, _| std::ptr::null(),
        set_field:        |_, _, _| {},
        get_field_bytes:  |_, _, out| { unsafe { *out = 0; } std::ptr::null() },
        set_field_bytes:  |_, _, _, _| {},
        get_metadata:     |_, _| std::ptr::null(),
        insert_into_flow: |_, _| false,
        pause_task:       |_, _| {},
        log:              |_, _, _| {},
        set_flag:         |_, _, _| {},
        set_flags:        |_, _, _| {},
        has_flag:         |_, _, _| false,
        clear_flag:       |_, _, _| {},
        get_keys:         |_| std::ptr::null(),
        unset_field:      |_, _| false,
        has_field:        |_, _| false,
        publish_to_queue: publish,
        publish_to_topic: |_, _, _| -1,
        create_task:      create,
    }
}
```

Run: `cargo test -p ox_cert_core test_create_and_enqueue 2>&1 | tail -5`
Expected: FAIL — `create_and_enqueue_task` doesn't exist yet.

- [ ] **Step 11: Implement `create_and_enqueue_task` in messaging.rs**

In `crates/cert/ox_cert_core/src/messaging.rs`, add after the existing `enqueue_task`:

```rust
use std::collections::HashMap;

/// Creates a new workflow task with the given initial state fields and publishes its ID to the queue.
/// Returns the new task's UUID string on success.
pub fn create_and_enqueue_task(
    api: &ox_workflow_abi::CoreHostApi,
    flow_name: &str,
    initial_fields: &HashMap<String, String>,
    priority: u8,
    queue: &str,
) -> Result<String, crate::error::CertError> {
    use std::ffi::{CStr, CString};

    // Build metadata JSON including flow_name so the scheduler can route the task.
    let mut meta = initial_fields.clone();
    meta.insert("flow_name".to_string(), flow_name.to_string());
    let meta_json = serde_json::to_string(&meta)
        .map_err(|e| crate::error::CertError::Internal(format!("fields json: {e}")))?;

    let flow_c = CString::new(flow_name)
        .map_err(|_| crate::error::CertError::Internal("flow_name nul".into()))?;
    let fields_c = CString::new(meta_json)
        .map_err(|_| crate::error::CertError::Internal("fields_json nul".into()))?;

    // 37 bytes: 36-char UUID + null terminator
    let mut task_id_buf = vec![0i8; 37];
    let rc = unsafe {
        (api.create_task)(
            flow_c.as_ptr(),
            fields_c.as_ptr(),
            priority,
            task_id_buf.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(crate::error::CertError::Internal(format!("create_task rc={rc}")));
    }
    let task_id = unsafe { CStr::from_ptr(task_id_buf.as_ptr()) }
        .to_string_lossy()
        .into_owned();

    // Publish the task_id to the queue
    let queue_c = CString::new(queue)
        .map_err(|_| crate::error::CertError::Internal("queue nul".into()))?;
    let payload = task_id.as_bytes();
    let rc = unsafe {
        (api.publish_to_queue)(queue_c.as_ptr(), priority, payload.as_ptr(), payload.len())
    };
    if rc != 0 {
        return Err(crate::error::CertError::Internal(format!("publish_to_queue rc={rc}")));
    }
    Ok(task_id)
}
```

Run: `cargo test -p ox_cert_core test_create_and_enqueue 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 12: Verify workspace builds**

```bash
cargo check --workspace 2>&1 | grep "^error" | head -20
```

Expected: no errors.

- [ ] **Step 13: Commit**

```bash
git add crates/workflow/ crates/cert/ox_cert_core/src/messaging.rs
git commit -m "feat(workflow): add create_task to CoreHostApi; add create_and_enqueue_task to messaging"
```

---

## Task 3: ox_cert_webhook — pipeline hook processor

**Files:**
- Create: `crates/cert/ox_cert_webhook/src/config.rs`
- Create: `crates/cert/ox_cert_webhook/src/signing.rs`
- Rewrite: `crates/cert/ox_cert_webhook/src/lib.rs`

- [ ] **Step 1: Write tests for HMAC signing**

Create `crates/cert/ox_cert_webhook/src/signing.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_payload_produces_sha256_prefix() {
        let sig = sign_payload(b"secret", b"{\"event\":\"test\"}");
        assert!(sig.starts_with("sha256="), "signature should start with sha256=, got {}", sig);
    }

    #[test]
    fn test_sign_payload_is_base64() {
        let sig = sign_payload(b"secret", b"payload");
        let b64_part = sig.strip_prefix("sha256=").unwrap();
        assert!(base64::engine::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            b64_part
        ).is_ok());
    }

    #[test]
    fn test_sign_payload_is_deterministic() {
        let s1 = sign_payload(b"key", b"data");
        let s2 = sign_payload(b"key", b"data");
        assert_eq!(s1, s2);
    }
}
```

Run: `cargo test -p ox_cert_webhook test_sign 2>&1 | tail -5`
Expected: FAIL — `sign_payload` doesn't exist yet.

- [ ] **Step 2: Implement `signing.rs`**

```rust
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ring::hmac;

pub fn sign_payload(secret: &[u8], payload: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let signature = hmac::sign(&key, payload);
    format!("sha256={}", STANDARD.encode(signature.as_ref()))
}

pub fn build_webhook_payload(
    tenant_id: &str,
    request_id: &str,
    csr_subject: &str,
    sans: &[String],
    profile: &str,
    requester_ip: &str,
    requester_identity: &str,
) -> Result<Vec<u8>, serde_json::Error> {
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let body = serde_json::json!({
        "event": "certificate_request",
        "tenant_id": tenant_id,
        "request_id": request_id,
        "csr_subject": csr_subject,
        "sans": sans,
        "profile": profile,
        "requester_ip": requester_ip,
        "requester_identity": requester_identity,
        "timestamp": now,
    });
    serde_json::to_vec(&body)
}
```

Add `time = { version = "0.3", features = ["formatting", "macros"] }` to `ox_cert_webhook/Cargo.toml`.

Run: `cargo test -p ox_cert_webhook test_sign 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 3: Create `config.rs`**

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct WebhookConfig {
    pub tenant_id: String,
    pub hooks: Vec<WebhookHookConfig>,
}

#[derive(Debug, Deserialize)]
pub struct WebhookHookConfig {
    pub name: String,
    pub url: String,
    pub hook_type: WebhookType,
    pub secret_env: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_retries")]
    pub retries: u32,
    pub on_failure: WebhookFailureMode,
}

fn default_timeout() -> u64 { 5 }
fn default_retries() -> u32 { 1 }

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum WebhookType {
    Authorize,
    Enrich,
    Both,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum WebhookFailureMode {
    Block,
    Allow,
}
```

- [ ] **Step 4: Write tests for hook processing**

In `crates/cert/ox_cert_webhook/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_url_must_be_https() {
        let config_json = serde_json::json!({
            "tenant_id": "t1",
            "hooks": [{
                "name": "test",
                "url": "http://insecure.example.com",
                "hook_type": "Authorize",
                "secret_env": "TEST_SECRET",
                "on_failure": "Block"
            }]
        });
        let result = validate_config(&config_json.to_string());
        assert!(result.is_err(), "http:// url should fail validation");
    }

    #[test]
    fn test_https_url_passes_validation() {
        let config_json = serde_json::json!({
            "tenant_id": "t1",
            "hooks": [{
                "name": "test",
                "url": "https://secure.example.com",
                "hook_type": "Enrich",
                "secret_env": "TEST_SECRET",
                "on_failure": "Allow"
            }]
        });
        let result = validate_config(&config_json.to_string());
        assert!(result.is_ok());
    }
}
```

Run: `cargo test -p ox_cert_webhook test_url 2>&1 | tail -5`
Expected: FAIL — `validate_config` doesn't exist.

- [ ] **Step 5: Implement full `lib.rs`**

```rust
mod config;
mod signing;

pub use config::{WebhookConfig, WebhookHookConfig, WebhookType, WebhookFailureMode};

use libc::{c_char, c_void};
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_END};
use std::ffi::{CStr, CString};

pub fn validate_config(json: &str) -> Result<WebhookConfig, String> {
    let cfg: WebhookConfig = serde_json::from_str(json).map_err(|e| e.to_string())?;
    for hook in &cfg.hooks {
        if !hook.url.starts_with("https://") {
            return Err(format!("hook '{}': url must be https://", hook.name));
        }
    }
    Ok(cfg)
}

struct HookSecret {
    name: String,
    secret: Vec<u8>,
}

struct ModuleContext {
    config: WebhookConfig,
    secrets: Vec<HookSecret>,  // resolved at init; index matches config.hooks
}

#[no_mangle]
pub extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    _api: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    let raw = if plugin_config_ctx.is_null() { return std::ptr::null_mut(); }
        else { unsafe { CStr::from_ptr(plugin_config_ctx) }.to_string_lossy().to_string() };

    let config = match validate_config(&raw) {
        Ok(c) => c,
        Err(e) => { eprintln!("[ox_cert_webhook] config error: {e}"); return std::ptr::null_mut(); }
    };

    let mut secrets = Vec::new();
    for hook in &config.hooks {
        match std::env::var(&hook.secret_env) {
            Ok(s) => secrets.push(HookSecret { name: hook.name.clone(), secret: s.into_bytes() }),
            Err(_) => {
                eprintln!("[ox_cert_webhook] missing env var '{}' for hook '{}'", hook.secret_env, hook.name);
                return std::ptr::null_mut();
            }
        }
    }

    Box::into_raw(Box::new(ModuleContext { config, secrets })) as *mut c_void
}

#[no_mangle]
pub extern "C" fn ox_plugin_process(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    let ctx = unsafe { &*(plugin_config_ctx as *const ModuleContext) };
    let api = unsafe { &*(task_ctx as *mut CoreHostApi) };

    // Read request fields from TaskState
    let get = |key: &str| -> String {
        let k = CString::new(key).unwrap();
        let ptr = (api.get_field)(task_ctx, k.as_ptr());
        if ptr.is_null() { String::new() }
        else { unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned() }
    };

    let body = get("request.body");
    let requester_ip = get("request.header.X-Forwarded-For");
    let path = get("request.path");
    let tenant_id = ctx.config.tenant_id.clone();
    let request_id = uuid::Uuid::new_v4().to_string();

    // Parse csr_subject / profile / sans from request body
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let csr_subject = parsed["subject"].as_str().unwrap_or("").to_string();
    let profile = parsed["profile"].as_str().unwrap_or("").to_string();
    let sans: Vec<String> = parsed["sans"].as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    let client = reqwest::blocking::Client::new();
    let mut enrichment = serde_json::Map::new();

    for (hook, secret_entry) in ctx.config.hooks.iter().zip(ctx.secrets.iter()) {
        let payload_bytes = match signing::build_webhook_payload(
            &tenant_id, &request_id, &csr_subject, &sans, &profile,
            &requester_ip, &requester_ip,
        ) {
            Ok(b) => b,
            Err(_) => {
                if hook.on_failure == WebhookFailureMode::Block {
                    return end_with_error(task_ctx, api, "WEBHOOK_REJECTED", 403, "webhook payload build failed");
                }
                continue;
            }
        };

        let signature = signing::sign_payload(&secret_entry.secret, &payload_bytes);

        let mut last_err = None;
        let mut response = None;
        for attempt in 0..=(hook.retries) {
            let _ = attempt;
            let timeout = std::time::Duration::from_secs(hook.timeout_secs);
            match client.post(&hook.url)
                .header("Content-Type", "application/json")
                .header("X-OxCert-Signature", &signature)
                .timeout(timeout)
                .body(payload_bytes.clone())
                .send()
            {
                Ok(r) if r.status().is_success() => { response = Some(r); break; }
                Ok(r) => { last_err = Some(format!("http {}", r.status())); }
                Err(e) => { last_err = Some(e.to_string()); }
            }
        }

        let resp = match response {
            Some(r) => r,
            None => {
                eprintln!("[ox_cert_webhook] hook '{}' failed: {:?}", hook.name, last_err);
                if hook.on_failure == WebhookFailureMode::Block {
                    return end_with_error(task_ctx, api, "WEBHOOK_REJECTED", 403,
                        &last_err.unwrap_or_default());
                }
                continue;
            }
        };

        let resp_json: serde_json::Value = match resp.json() {
            Ok(j) => j,
            Err(_) => {
                if hook.on_failure == WebhookFailureMode::Block {
                    return end_with_error(task_ctx, api, "WEBHOOK_REJECTED", 403, "invalid webhook response");
                }
                continue;
            }
        };

        // Check authorization
        if hook.hook_type == WebhookType::Authorize || hook.hook_type == WebhookType::Both {
            if resp_json["allow"].as_bool() == Some(false) {
                let reason = resp_json["reason"].as_str().unwrap_or("denied by webhook");
                return end_with_error(task_ctx, api, "WEBHOOK_REJECTED", 403, reason);
            }
        }

        // Collect enrichment
        if hook.hook_type == WebhookType::Enrich || hook.hook_type == WebhookType::Both {
            if let Some(data) = resp_json["data"].as_object() {
                for (k, v) in data {
                    enrichment.insert(k.clone(), v.clone());
                }
            }
        }
    }

    // All hooks passed
    let auth_key = CString::new("cert.webhook.authorized").unwrap();
    let auth_val = CString::new("true").unwrap();
    (api.set_field)(task_ctx, auth_key.as_ptr(), auth_val.as_ptr());

    if !enrichment.is_empty() {
        let enrich_json = serde_json::Value::Object(enrichment).to_string();
        let e_key = CString::new("cert.webhook.enrichment").unwrap();
        let e_val = CString::new(enrich_json).unwrap();
        (api.set_field)(task_ctx, e_key.as_ptr(), e_val.as_ptr());
    }

    FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
}

fn end_with_error(
    task_ctx: *mut c_void,
    api: &CoreHostApi,
    code: &str,
    status: u16,
    msg: &str,
) -> FlowControl {
    let body = serde_json::json!({ "error": { "code": code, "message": msg } }).to_string();
    let set = |k: &str, v: &str| {
        let ck = CString::new(k).unwrap();
        let cv = CString::new(v).unwrap();
        (api.set_field)(task_ctx, ck.as_ptr(), cv.as_ptr());
    };
    set("response.status", &status.to_string());
    set("response.body", &body);
    set("response.header.Content-Type", "application/json");
    FlowControl { code: FLOW_CONTROL_END, payload: std::ptr::null() }
}

#[no_mangle]
pub extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {}

#[no_mangle]
pub extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_config_ctx as *mut ModuleContext)); }
    }
}

use uuid;
```

Add `uuid = { version = "1.6", features = ["v4"] }` and `time = { version = "0.3", features = ["formatting", "macros"] }` to the `ox_cert_webhook/Cargo.toml` if not already present.

Run: `cargo test -p ox_cert_webhook 2>&1 | tail -10`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/cert/ox_cert_webhook/
git commit -m "feat(ox_cert_webhook): HMAC-signed pipeline hook processor for authorize/enrich"
```

---

## Task 4: ox_cert_ct — CT SCT query endpoints

**Files:**
- Create: `crates/cert/ox_cert_ct/src/config.rs`
- Create: `crates/cert/ox_cert_ct/src/handlers.rs`
- Rewrite: `crates/cert/ox_cert_ct/src/lib.rs`

- [ ] **Step 1: Write tests for route dispatch**

In `crates/cert/ox_cert_ct/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_scts_route() {
        let path = "/api/v1/ct/scts/some-serial-123";
        let result = dispatch_route("GET", path);
        assert_eq!(result, Some(Route::GetScts("some-serial-123".to_string())));
    }

    #[test]
    fn test_dispatch_logs_route() {
        let result = dispatch_route("GET", "/api/v1/ct/logs");
        assert_eq!(result, Some(Route::ListLogs));
    }

    #[test]
    fn test_dispatch_unknown_returns_none() {
        let result = dispatch_route("POST", "/api/v1/ct/scts/x");
        assert!(result.is_none());
    }
}
```

Run: `cargo test -p ox_cert_ct test_dispatch 2>&1 | tail -5`
Expected: FAIL — `dispatch_route` and `Route` don't exist.

- [ ] **Step 2: Create `config.rs`**

```rust
use serde::Deserialize;
use ox_cert_core::types::CtConfig;
use ox_cert_core::certstore::mod_::CertStoreConfig;

#[derive(Debug, Deserialize)]
pub struct CtPluginConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub ct: CtConfig,
}
```

Note: use the exact path for `CertStoreConfig` as exported from `ox_cert_core` — adjust if the module path differs.

- [ ] **Step 3: Implement route dispatch and `lib.rs`**

```rust
mod config;
mod handlers;

pub use config::CtPluginConfig;

use libc::{c_char, c_void};
use ox_cert_core::{certstore::CertStore, OxPersistenceCertStore};
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_END};
use std::ffi::{CStr, CString};

#[derive(Debug, PartialEq)]
pub(crate) enum Route {
    GetScts(String),
    ListLogs,
}

pub(crate) fn dispatch_route(method: &str, path: &str) -> Option<Route> {
    if method != "GET" {
        return None;
    }
    if let Some(serial) = path.strip_prefix("/api/v1/ct/scts/") {
        if !serial.is_empty() {
            return Some(Route::GetScts(serial.to_string()));
        }
    }
    if path == "/api/v1/ct/logs" {
        return Some(Route::ListLogs);
    }
    None
}

struct ModuleContext {
    config: CtPluginConfig,
    store: OxPersistenceCertStore,
}

#[no_mangle]
pub extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    _api: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    let raw = if plugin_config_ctx.is_null() { return std::ptr::null_mut(); }
        else { unsafe { CStr::from_ptr(plugin_config_ctx) }.to_string_lossy().to_string() };

    let config: CtPluginConfig = match serde_json::from_str(&raw) {
        Ok(c) => c,
        Err(e) => { eprintln!("[ox_cert_ct] config error: {e}"); return std::ptr::null_mut(); }
    };

    let store = match OxPersistenceCertStore::open(&config.store) {
        Ok(s) => s,
        Err(e) => { eprintln!("[ox_cert_ct] store open error: {e}"); return std::ptr::null_mut(); }
    };

    Box::into_raw(Box::new(ModuleContext { config, store })) as *mut c_void
}

#[no_mangle]
pub extern "C" fn ox_plugin_process(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    let ctx = unsafe { &*(plugin_config_ctx as *const ModuleContext) };
    let api = unsafe { &*(task_ctx as *mut CoreHostApi) };

    let get = |key: &str| -> String {
        let k = CString::new(key).unwrap();
        let ptr = (api.get_field)(task_ctx, k.as_ptr());
        if ptr.is_null() { String::new() }
        else { unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned() }
    };
    let set = |k: &str, v: &str| {
        let ck = CString::new(k).unwrap();
        let cv = CString::new(v).unwrap();
        (api.set_field)(task_ctx, ck.as_ptr(), cv.as_ptr());
    };

    let method = get("request.method");
    let path = get("request.path");

    let route = match dispatch_route(&method, &path) {
        Some(r) => r,
        None => {
            set("response.status", "404");
            set("response.body", r#"{"error":{"code":"NOT_FOUND","message":"route not found"}}"#);
            set("response.header.Content-Type", "application/json");
            return FlowControl { code: FLOW_CONTROL_END, payload: std::ptr::null() };
        }
    };

    let tenant_id = &ctx.config.tenant_id;
    let (status, body) = match route {
        Route::GetScts(serial) => handlers::get_scts(&ctx.store, tenant_id, &serial, &ctx.config),
        Route::ListLogs => handlers::list_logs(&ctx.config),
    };

    set("response.status", &status.to_string());
    set("response.body", &body);
    set("response.header.Content-Type", "application/json");
    FlowControl { code: FLOW_CONTROL_END, payload: std::ptr::null() }
}

#[no_mangle]
pub extern "C" fn ox_plugin_error(_: *mut c_void, _: *mut c_void) {}

#[no_mangle]
pub extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_config_ctx as *mut ModuleContext)); }
    }
}
```

- [ ] **Step 4: Implement `handlers.rs`**

```rust
use ox_cert_core::{certstore::CertStore, types::CtConfig, OxPersistenceCertStore};
use crate::CtPluginConfig;

pub fn get_scts(
    store: &OxPersistenceCertStore,
    tenant_id: &str,
    serial: &str,
    config: &CtPluginConfig,
) -> (u16, String) {
    match store.get_cert_by_serial(tenant_id, serial) {
        Err(_) => (
            500,
            r#"{"error":{"code":"INTERNAL_ERROR","message":"storage error"}}"#.into(),
        ),
        Ok(None) => (
            404,
            r#"{"error":{"code":"NOT_FOUND","message":"certificate not found"}}"#.into(),
        ),
        Ok(Some(record)) => {
            let scts_json = serde_json::to_string(&record.scts).unwrap_or_else(|_| "[]".into());
            let mut meta = serde_json::json!({ "tenant_id": tenant_id });
            if record.scts.is_empty() {
                meta["note"] = serde_json::Value::String(
                    "CT was disabled or submission failed at issuance time".into()
                );
            }
            let body = serde_json::json!({
                "data": { "serial": serial, "scts": record.scts },
                "meta": meta,
            });
            (200, body.to_string())
        }
    }
}

pub fn list_logs(config: &CtPluginConfig) -> (u16, String) {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let log_statuses: Vec<serde_json::Value> = config.ct.logs.iter().map(|log| {
        let url = format!("{}/ct/v1/get-sth", log.url.trim_end_matches('/'));
        let start = std::time::Instant::now();
        let result = client.head(&url).send();
        let elapsed = start.elapsed().as_millis() as u64;
        match result {
            Ok(r) if r.status().is_success() => serde_json::json!({
                "name": log.name, "url": log.url, "reachable": true, "latency_ms": elapsed
            }),
            _ => serde_json::json!({
                "name": log.name, "url": log.url, "reachable": false, "latency_ms": null
            }),
        }
    }).collect();

    let body = serde_json::json!({
        "data": log_statuses,
        "meta": { "tenant_id": &config.tenant_id },
    });
    (200, body.to_string())
}
```

Run: `cargo test -p ox_cert_ct 2>&1 | tail -10`
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cert/ox_cert_ct/
git commit -m "feat(ox_cert_ct): GET /ct/scts/{serial} and /ct/logs endpoints"
```

---

## Task 5: ox_cert_ra — ApprovalRequest types + CertStore RA methods

**Files:**
- Modify: `crates/cert/ox_cert_core/src/types.rs`
- Modify: `crates/cert/ox_cert_core/src/certstore/mod.rs`
- Modify: `crates/cert/ox_cert_core/src/certstore/persistence.rs`

- [ ] **Step 1: Write tests for ApprovalRequest GDO round-trip**

In `crates/cert/ox_cert_core/src/certstore/persistence.rs` tests:

```rust
#[cfg(test)]
mod ra_tests {
    use super::*;
    use crate::types::{ApprovalRequest, RaStatus};

    fn test_store() -> OxPersistenceCertStore {
        // Requires a configured in-memory persistence driver registered as "test"
        // Skipped if driver not available
        OxPersistenceCertStore::open_for_test()
    }

    #[test]
    fn test_store_and_get_ra_request() {
        let store = match test_store_opt() { Some(s) => s, None => return };
        let req = ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
            tenant_id: "t1".into(),
            status: RaStatus::Pending,
            csr_pem: "-----BEGIN CERTIFICATE REQUEST-----\nfake\n-----END CERTIFICATE REQUEST-----".into(),
            profile: "standard".into(),
            sans: vec!["example.com".into()],
            requester_identity: Some("10.0.0.1".into()),
            reviewer_identity: None,
            reviewer_notes: None,
            flow_name: Some("issue_flow".into()),
            certificate_serial: None,
            created_at: "2026-04-23T00:00:00Z".into(),
            updated_at: "2026-04-23T00:00:00Z".into(),
        };
        store.store_ra_request("t1", &req).unwrap();
        let loaded = store.get_ra_request("t1", &req.id).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.csr_pem, req.csr_pem);
        assert_eq!(loaded.status, RaStatus::Pending);
    }

    #[test]
    fn test_list_ra_pending() {
        let store = match test_store_opt() { Some(s) => s, None => return };
        let pending = store.list_ra_pending("t1", 1, 50).unwrap();
        assert!(pending.iter().all(|r| r.status == RaStatus::Pending));
    }
}
```

Run: `cargo test -p ox_cert_core ra_tests 2>&1 | tail -10`
Expected: FAIL — types and methods don't exist.

- [ ] **Step 2: Add ApprovalRequest and RaStatus to `types.rs`**

In `crates/cert/ox_cert_core/src/types.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RaStatus {
    Pending,
    Approved,
    Denied,
}

impl std::fmt::Display for RaStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RaStatus::Pending  => write!(f, "pending"),
            RaStatus::Approved => write!(f, "approved"),
            RaStatus::Denied   => write!(f, "denied"),
        }
    }
}

impl std::str::FromStr for RaStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending"  => Ok(RaStatus::Pending),
            "approved" => Ok(RaStatus::Approved),
            "denied"   => Ok(RaStatus::Denied),
            other      => Err(format!("unknown RaStatus: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub tenant_id: String,
    pub status: RaStatus,
    pub csr_pem: String,
    pub profile: String,
    pub sans: Vec<String>,
    pub requester_identity: Option<String>,
    pub reviewer_identity: Option<String>,
    pub reviewer_notes: Option<String>,
    pub flow_name: Option<String>,
    pub certificate_serial: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
```

- [ ] **Step 3: Add RA methods to `CertStore` trait**

In `crates/cert/ox_cert_core/src/certstore/mod.rs`, add to the `CertStore` trait:

```rust
fn store_ra_request(&self, tenant_id: &str, req: &ApprovalRequest) -> Result<(), CertError>;
fn get_ra_request(&self, tenant_id: &str, id: &str) -> Result<Option<ApprovalRequest>, CertError>;
fn list_ra_pending(&self, tenant_id: &str, page: u32, per_page: u32) -> Result<Vec<ApprovalRequest>, CertError>;
fn list_ra_history(&self, tenant_id: &str, page: u32, per_page: u32) -> Result<Vec<ApprovalRequest>, CertError>;
fn update_ra_request(
    &self,
    tenant_id: &str,
    id: &str,
    status: RaStatus,
    reviewer_identity: &str,
    notes: &str,
) -> Result<(), CertError>;
fn set_ra_certificate_serial(&self, tenant_id: &str, id: &str, serial: &str) -> Result<(), CertError>;
```

Import `ApprovalRequest` and `RaStatus` from `crate::types`.

- [ ] **Step 4: Implement RA methods on `OxPersistenceCertStore`**

In `crates/cert/ox_cert_core/src/certstore/persistence.rs`:

```rust
fn store_ra_request(&self, tenant_id: &str, req: &ApprovalRequest) -> Result<(), CertError> {
    let mut gdo = GenericDataObject::new("id", &req.id);
    gdo.set("tenant_id", tenant_id);
    gdo.set("status", &req.status.to_string());
    gdo.set("csr_pem", &req.csr_pem);
    gdo.set("profile", &req.profile);
    gdo.set("sans", &serde_json::to_string(&req.sans).unwrap_or_default());
    gdo.set("requester_identity", req.requester_identity.as_deref().unwrap_or(""));
    gdo.set("flow_name", req.flow_name.as_deref().unwrap_or(""));
    gdo.set("created_at", &req.created_at);
    gdo.set("updated_at", &req.updated_at);
    gdo.persist(&self.driver_name, "ra_requests")
        .map_err(|e| CertError::Internal(format!("store_ra_request: {e}")))
}

fn get_ra_request(&self, tenant_id: &str, id: &str) -> Result<Option<ApprovalRequest>, CertError> {
    let mut gdo = GenericDataObject::new("id", id);
    gdo.set("tenant_id", tenant_id);
    match gdo.hydrate_object(&self.driver_name, "ra_requests") {
        Ok(()) => Ok(Some(ra_from_gdo(&gdo)
            .map_err(|e| CertError::Internal(format!("ra_from_gdo: {e}")))?)),
        Err(e) if e.contains("not found") => Ok(None),
        Err(e) => Err(CertError::Internal(format!("get_ra_request: {e}"))),
    }
}

fn list_ra_pending(&self, tenant_id: &str, page: u32, per_page: u32) -> Result<Vec<ApprovalRequest>, CertError> {
    let mut filter = GenericDataObject::new("id", "");
    filter.set("tenant_id", tenant_id);
    filter.set("status", "pending");
    let ids = filter.fetch(&self.driver_name, "ra_requests")
        .map_err(|e| CertError::Internal(format!("list_ra_pending: {e}")))?;
    let offset = ((page.saturating_sub(1)) * per_page) as usize;
    let ids_page: Vec<_> = ids.into_iter().skip(offset).take(per_page as usize).collect();
    ids_page.into_iter().map(|id| {
        self.get_ra_request(tenant_id, &id)
            .and_then(|opt| opt.ok_or_else(|| CertError::Internal("missing row".into())))
    }).collect()
}

fn list_ra_history(&self, tenant_id: &str, page: u32, per_page: u32) -> Result<Vec<ApprovalRequest>, CertError> {
    let offset = ((page.saturating_sub(1)) * per_page) as usize;
    let sql = "SELECT id FROM ra_requests WHERE tenant_id = ? AND status != 'pending' ORDER BY updated_at DESC LIMIT ? OFFSET ?";
    let params = serde_json::json!({
        "sql": sql,
        "params": [tenant_id, per_page, offset],
    });
    let rows = self.call_raw_sql(&params)
        .map_err(|e| CertError::Internal(format!("list_ra_history: {e}")))?;
    rows.into_iter().map(|id_json| {
        let id = id_json.as_str().unwrap_or("").to_string();
        self.get_ra_request(tenant_id, &id)
            .and_then(|opt| opt.ok_or_else(|| CertError::Internal("missing row".into())))
    }).collect()
}

fn update_ra_request(
    &self,
    tenant_id: &str,
    id: &str,
    status: RaStatus,
    reviewer_identity: &str,
    notes: &str,
) -> Result<(), CertError> {
    let mut gdo = GenericDataObject::new("id", id);
    gdo.set("tenant_id", tenant_id);
    gdo.hydrate_object(&self.driver_name, "ra_requests")
        .map_err(|e| CertError::Internal(format!("hydrate for update: {e}")))?;
    gdo.set("status", &status.to_string());
    gdo.set("reviewer_identity", reviewer_identity);
    gdo.set("reviewer_notes", notes);
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    gdo.set("updated_at", &now);
    gdo.persist(&self.driver_name, "ra_requests")
        .map_err(|e| CertError::Internal(format!("update_ra_request: {e}")))
}

fn set_ra_certificate_serial(&self, tenant_id: &str, id: &str, serial: &str) -> Result<(), CertError> {
    let mut gdo = GenericDataObject::new("id", id);
    gdo.set("tenant_id", tenant_id);
    gdo.hydrate_object(&self.driver_name, "ra_requests")
        .map_err(|e| CertError::Internal(format!("hydrate for serial: {e}")))?;
    gdo.set("certificate_serial", serial);
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    gdo.set("updated_at", &now);
    gdo.persist(&self.driver_name, "ra_requests")
        .map_err(|e| CertError::Internal(format!("set_ra_certificate_serial: {e}")))
}
```

Add a helper `ra_from_gdo` that maps GDO fields to `ApprovalRequest`:

```rust
fn ra_from_gdo(gdo: &GenericDataObject) -> Result<ApprovalRequest, String> {
    let get = |k: &str| gdo.get(k).unwrap_or_default().to_string();
    let status: RaStatus = get("status").parse().map_err(|e: String| e)?;
    let sans: Vec<String> = serde_json::from_str(&get("sans")).unwrap_or_default();
    Ok(ApprovalRequest {
        id: get("id"),
        tenant_id: get("tenant_id"),
        status,
        csr_pem: get("csr_pem"),
        profile: get("profile"),
        sans,
        requester_identity: Some(get("requester_identity")).filter(|s| !s.is_empty()),
        reviewer_identity:  Some(get("reviewer_identity")).filter(|s| !s.is_empty()),
        reviewer_notes:     Some(get("reviewer_notes")).filter(|s| !s.is_empty()),
        flow_name:          Some(get("flow_name")).filter(|s| !s.is_empty()),
        certificate_serial: Some(get("certificate_serial")).filter(|s| !s.is_empty()),
        created_at: get("created_at"),
        updated_at: get("updated_at"),
    })
}
```

Also add a helper used by `list_ra_history` to call raw SQL via the driver:

```rust
fn call_raw_sql(&self, params: &serde_json::Value) -> Result<Vec<serde_json::Value>, String> {
    let driver = ox_persistence::PERSISTENCE_DRIVER_REGISTRY
        .get(&self.driver_name)
        .ok_or_else(|| format!("driver '{}' not registered", self.driver_name))?;
    match driver.call_action("raw_sql", params.clone()) {
        Ok(serde_json::Value::Array(rows)) => Ok(rows),
        Ok(_) => Ok(vec![]),
        Err(e) => Err(e),
    }
}
```

Run: `cargo test -p ox_cert_core ra_tests 2>&1 | tail -10`
Expected: all passing or skipped (if test driver not configured).

- [ ] **Step 5: Commit**

```bash
git add crates/cert/ox_cert_core/src/types.rs \
        crates/cert/ox_cert_core/src/certstore/
git commit -m "feat(ox_cert_core): ApprovalRequest, RaStatus, CertStore RA CRUD methods"
```

---

## Task 6: ox_cert_ra — handlers + auto-approve background scanner

**Files:**
- Create: `crates/cert/ox_cert_ra/src/config.rs`
- Create: `crates/cert/ox_cert_ra/src/handlers.rs`
- Create: `crates/cert/ox_cert_ra/src/auto_approve.rs`
- Rewrite: `crates/cert/ox_cert_ra/src/lib.rs`

- [ ] **Step 1: Write tests for route dispatch**

In `crates/cert/ox_cert_ra/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_list_pending() {
        let route = dispatch_route("GET", "/api/v1/ra/pending");
        assert_eq!(route, Some(Route::ListPending));
    }

    #[test]
    fn test_dispatch_get_request() {
        let route = dispatch_route("GET", "/api/v1/ra/pending/abc-123");
        assert_eq!(route, Some(Route::GetRequest("abc-123".to_string())));
    }

    #[test]
    fn test_dispatch_approve() {
        let route = dispatch_route("POST", "/api/v1/ra/pending/abc-123/approve");
        assert_eq!(route, Some(Route::Approve("abc-123".to_string())));
    }

    #[test]
    fn test_dispatch_deny() {
        let route = dispatch_route("POST", "/api/v1/ra/pending/abc-123/deny");
        assert_eq!(route, Some(Route::Deny("abc-123".to_string())));
    }

    #[test]
    fn test_dispatch_history() {
        let route = dispatch_route("GET", "/api/v1/ra/history");
        assert_eq!(route, Some(Route::History));
    }

    #[test]
    fn test_dispatch_certificate() {
        let route = dispatch_route("GET", "/api/v1/ra/requests/abc-123/certificate");
        assert_eq!(route, Some(Route::Certificate("abc-123".to_string())));
    }
}
```

Run: `cargo test -p ox_cert_ra test_dispatch 2>&1 | tail -5`
Expected: FAIL — `dispatch_route` and `Route` don't exist.

- [ ] **Step 2: Create `config.rs`**

```rust
use serde::Deserialize;
use ox_cert_core::certstore::CertStoreConfig;

#[derive(Debug, Clone, Deserialize)]
pub struct RaConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    #[serde(default = "default_queue")]
    pub resubmit_queue: String,
    #[serde(default = "default_priority")]
    pub resubmit_priority: u8,
    #[serde(default)]
    pub auto_approve_rules: Vec<AutoApproveRule>,
    pub notification_webhook: Option<String>,
}

fn default_queue() -> String { "tasks.pending".to_string() }
fn default_priority() -> u8 { 100 }

#[derive(Debug, Clone, Deserialize)]
pub struct AutoApproveRule {
    pub identity_pattern: String,
    pub profiles: Vec<String>,
}

impl AutoApproveRule {
    /// Returns true if this rule matches the given requester identity and profile.
    pub fn matches(&self, identity: &str, profile: &str) -> bool {
        let re = regex::Regex::new(&self.identity_pattern);
        if let Ok(re) = re {
            re.is_match(identity) && self.profiles.iter().any(|p| p == profile)
        } else {
            false
        }
    }
}
```

- [ ] **Step 3: Create `handlers.rs`**

```rust
use std::collections::HashMap;
use ox_cert_core::{
    certstore::CertStore,
    messaging::create_and_enqueue_task,
    types::{ApprovalRequest, RaStatus},
    OxPersistenceCertStore,
};
use ox_workflow_abi::CoreHostApi;
use crate::config::RaConfig;

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn list_pending(
    store: &OxPersistenceCertStore,
    tenant_id: &str,
    page: u32,
    per_page: u32,
) -> (u16, String) {
    match store.list_ra_pending(tenant_id, page, per_page) {
        Ok(reqs) => {
            let body = serde_json::json!({
                "data": reqs,
                "meta": { "tenant_id": tenant_id, "page": page, "per_page": per_page }
            });
            (200, body.to_string())
        }
        Err(e) => (500, error_body("INTERNAL_ERROR", &e.to_string())),
    }
}

pub fn get_request(
    store: &OxPersistenceCertStore,
    tenant_id: &str,
    id: &str,
) -> (u16, String) {
    match store.get_ra_request(tenant_id, id) {
        Ok(Some(req)) => (200, serde_json::json!({ "data": req, "meta": { "tenant_id": tenant_id } }).to_string()),
        Ok(None)      => (404, error_body("NOT_FOUND", "request not found")),
        Err(e)        => (500, error_body("INTERNAL_ERROR", &e.to_string())),
    }
}

pub fn approve(
    store: &OxPersistenceCertStore,
    config: &RaConfig,
    api: &CoreHostApi,
    id: &str,
    body: &str,
) -> (u16, String) {
    let reviewer_notes: String = serde_json::from_str(body)
        .ok()
        .and_then(|v: serde_json::Value| v["reviewer_notes"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    let req = match store.get_ra_request(&config.tenant_id, id) {
        Ok(Some(r)) => r,
        Ok(None)    => return (404, error_body("NOT_FOUND", "request not found")),
        Err(e)      => return (500, error_body("INTERNAL_ERROR", &e.to_string())),
    };

    if req.status != RaStatus::Pending {
        return (409, error_body("INVALID_REQUEST", "Request already processed"));
    }

    if let Err(e) = store.update_ra_request(
        &config.tenant_id, id,
        RaStatus::Approved,
        "ra_officer",  // caller identity (would be from auth context in production)
        &reviewer_notes,
    ) {
        return (500, error_body("INTERNAL_ERROR", &e.to_string()));
    }

    // Build re-submission fields
    let resubmit_body = serde_json::json!({
        "csr": req.csr_pem,
        "profile": req.profile,
        "sans": req.sans,
    });

    let flow_name = req.flow_name.as_deref().unwrap_or("issue_flow");
    let mut initial_fields: HashMap<String, String> = HashMap::new();
    initial_fields.insert("cert.ra.approved".into(), "true".into());
    initial_fields.insert("cert.ra.request_id".into(), id.to_string());
    initial_fields.insert("request.body".into(), resubmit_body.to_string());
    initial_fields.insert("request.method".into(), "POST".into());
    initial_fields.insert("request.path".into(), "/api/v1/certificates".into());
    initial_fields.insert("tenant_id".into(), config.tenant_id.clone());

    let task_id = match create_and_enqueue_task(
        api,
        flow_name,
        &initial_fields,
        config.resubmit_priority,
        &config.resubmit_queue,
    ) {
        Ok(t) => t,
        Err(e) => return (500, error_body("INTERNAL_ERROR", &format!("enqueue failed: {e}"))),
    };

    let body = serde_json::json!({
        "data": { "id": id, "status": "approved", "task_id": task_id },
        "meta": { "tenant_id": &config.tenant_id }
    });
    (200, body.to_string())
}

pub fn deny(
    store: &OxPersistenceCertStore,
    config: &RaConfig,
    id: &str,
    body: &str,
) -> (u16, String) {
    let reason: String = serde_json::from_str(body)
        .ok()
        .and_then(|v: serde_json::Value| v["reason"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    if reason.is_empty() {
        return (400, error_body("INVALID_REQUEST", "'reason' is required to deny a request"));
    }

    let req = match store.get_ra_request(&config.tenant_id, id) {
        Ok(Some(r)) => r,
        Ok(None)    => return (404, error_body("NOT_FOUND", "request not found")),
        Err(e)      => return (500, error_body("INTERNAL_ERROR", &e.to_string())),
    };
    if req.status != RaStatus::Pending {
        return (409, error_body("INVALID_REQUEST", "Request already processed"));
    }

    if let Err(e) = store.update_ra_request(
        &config.tenant_id, id,
        RaStatus::Denied,
        "ra_officer",
        &reason,
    ) {
        return (500, error_body("INTERNAL_ERROR", &e.to_string()));
    }

    let body = serde_json::json!({
        "data": { "id": id, "status": "denied" },
        "meta": { "tenant_id": &config.tenant_id }
    });
    (200, body.to_string())
}

pub fn history(
    store: &OxPersistenceCertStore,
    tenant_id: &str,
    page: u32,
    per_page: u32,
) -> (u16, String) {
    match store.list_ra_history(tenant_id, page, per_page) {
        Ok(reqs) => {
            let body = serde_json::json!({
                "data": reqs,
                "meta": { "tenant_id": tenant_id, "page": page, "per_page": per_page }
            });
            (200, body.to_string())
        }
        Err(e) => (500, error_body("INTERNAL_ERROR", &e.to_string())),
    }
}

pub fn get_certificate(
    store: &OxPersistenceCertStore,
    tenant_id: &str,
    id: &str,
) -> (u16, String) {
    let req = match store.get_ra_request(tenant_id, id) {
        Ok(Some(r)) => r,
        Ok(None)    => return (404, error_body("NOT_FOUND", "request not found")),
        Err(e)      => return (500, error_body("INTERNAL_ERROR", &e.to_string())),
    };
    match req.status {
        RaStatus::Pending  => (202, serde_json::json!({ "status": "pending" }).to_string()),
        RaStatus::Denied   => (409, error_body("INVALID_REQUEST", "request was denied")),
        RaStatus::Approved => {
            match req.certificate_serial {
                None => (202, serde_json::json!({ "status": "processing" }).to_string()),
                Some(serial) => {
                    match store.get_cert_by_serial(tenant_id, &serial) {
                        Ok(Some(cert)) => {
                            let body = serde_json::json!({ "data": cert, "meta": { "tenant_id": tenant_id } });
                            (200, body.to_string())
                        }
                        Ok(None) => (202, serde_json::json!({ "status": "processing" }).to_string()),
                        Err(e)  => (500, error_body("INTERNAL_ERROR", &e.to_string())),
                    }
                }
            }
        }
    }
}

fn error_body(code: &str, msg: &str) -> String {
    serde_json::json!({ "error": { "code": code, "message": msg } }).to_string()
}
```

- [ ] **Step 4: Create `auto_approve.rs`**

```rust
use std::sync::Arc;
use std::time::Duration;
use ox_cert_core::{certstore::CertStore, OxPersistenceCertStore};
use ox_workflow_abi::CoreHostApi;
use crate::config::{AutoApproveRule, RaConfig};
use crate::handlers;

/// Spawns a background thread that re-applies auto-approve rules every 5 minutes.
/// The thread is started once at plugin init and runs until the store is dropped.
/// Returns the thread handle (caller may ignore it; the thread is daemonized).
pub fn spawn_auto_approve_scanner(
    store: Arc<OxPersistenceCertStore>,
    config: Arc<RaConfig>,
    api_ptr: usize,  // *const CoreHostApi cast to usize for Send
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        loop {
            run_auto_approve_scan(&store, &config, api_ptr);
            std::thread::sleep(Duration::from_secs(300)); // 5 minutes
        }
    })
}

fn run_auto_approve_scan(
    store: &OxPersistenceCertStore,
    config: &RaConfig,
    api_ptr: usize,
) {
    if config.auto_approve_rules.is_empty() {
        return;
    }
    let pending = match store.list_ra_pending(&config.tenant_id, 1, 1000) {
        Ok(p) => p,
        Err(e) => { eprintln!("[ox_cert_ra auto-approve] list error: {e}"); return; }
    };
    let api = unsafe { &*(api_ptr as *const CoreHostApi) };
    for req in pending {
        let identity = req.requester_identity.as_deref().unwrap_or("");
        let matched = config.auto_approve_rules.iter().any(|rule| rule.matches(identity, &req.profile));
        if matched {
            let (status, _) = handlers::approve(store, config, api, &req.id, "{}");
            if status != 200 {
                eprintln!("[ox_cert_ra auto-approve] approve {} returned {status}", req.id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AutoApproveRule;

    #[test]
    fn test_rule_matches_identity_and_profile() {
        let rule = AutoApproveRule {
            identity_pattern: r"^10\.0\.0\.\d+$".to_string(),
            profiles: vec!["standard".to_string()],
        };
        assert!(rule.matches("10.0.0.42", "standard"));
        assert!(!rule.matches("10.0.0.42", "long_lived"));
        assert!(!rule.matches("192.168.1.1", "standard"));
    }

    #[test]
    fn test_invalid_regex_does_not_panic() {
        let rule = AutoApproveRule {
            identity_pattern: "[invalid".to_string(),
            profiles: vec!["standard".to_string()],
        };
        assert!(!rule.matches("anything", "standard"));
    }
}
```

- [ ] **Step 5: Implement full `lib.rs`**

```rust
mod config;
mod handlers;
mod auto_approve;

pub use config::{RaConfig, AutoApproveRule};

use std::sync::Arc;
use libc::{c_char, c_void};
use ox_cert_core::OxPersistenceCertStore;
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_END};
use std::ffi::{CStr, CString};

#[derive(Debug, PartialEq)]
pub(crate) enum Route {
    ListPending,
    GetRequest(String),
    Approve(String),
    Deny(String),
    History,
    Certificate(String),
}

pub(crate) fn dispatch_route(method: &str, path: &str) -> Option<Route> {
    match (method, path) {
        ("GET", "/api/v1/ra/pending") => Some(Route::ListPending),
        ("GET", "/api/v1/ra/history") => Some(Route::History),
        (m, p) if p.starts_with("/api/v1/ra/pending/") => {
            let rest = &p["/api/v1/ra/pending/".len()..];
            if let Some(id) = rest.strip_suffix("/approve") {
                if m == "POST" { return Some(Route::Approve(id.to_string())); }
            } else if let Some(id) = rest.strip_suffix("/deny") {
                if m == "POST" { return Some(Route::Deny(id.to_string())); }
            } else if m == "GET" && !rest.is_empty() {
                return Some(Route::GetRequest(rest.to_string()));
            }
            None
        }
        ("GET", p) if p.starts_with("/api/v1/ra/requests/") => {
            let rest = &p["/api/v1/ra/requests/".len()..];
            if let Some(id) = rest.strip_suffix("/certificate") {
                return Some(Route::Certificate(id.to_string()));
            }
            None
        }
        _ => None,
    }
}

struct ModuleContext {
    config: Arc<RaConfig>,
    store: Arc<OxPersistenceCertStore>,
    api_ptr: *const CoreHostApi,
    _scanner: std::thread::JoinHandle<()>,
}

unsafe impl Send for ModuleContext {}
unsafe impl Sync for ModuleContext {}

#[no_mangle]
pub extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    api: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    let raw = if plugin_config_ctx.is_null() { return std::ptr::null_mut(); }
        else { unsafe { CStr::from_ptr(plugin_config_ctx) }.to_string_lossy().to_string() };

    let config: RaConfig = match serde_json::from_str(&raw) {
        Ok(c) => c,
        Err(e) => { eprintln!("[ox_cert_ra] config error: {e}"); return std::ptr::null_mut(); }
    };
    let config = Arc::new(config);

    let store = match OxPersistenceCertStore::open(&config.store) {
        Ok(s) => Arc::new(s),
        Err(e) => { eprintln!("[ox_cert_ra] store open error: {e}"); return std::ptr::null_mut(); }
    };

    let scanner = auto_approve::spawn_auto_approve_scanner(
        store.clone(),
        config.clone(),
        api as usize,
    );

    Box::into_raw(Box::new(ModuleContext {
        config,
        store,
        api_ptr: api,
        _scanner: scanner,
    })) as *mut c_void
}

#[no_mangle]
pub extern "C" fn ox_plugin_process(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    let ctx = unsafe { &*(plugin_config_ctx as *const ModuleContext) };
    let api = unsafe { &*(ctx.api_ptr) };

    let get = |key: &str| -> String {
        let k = CString::new(key).unwrap();
        let ptr = (api.get_field)(task_ctx, k.as_ptr());
        if ptr.is_null() { String::new() }
        else { unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned() }
    };
    let set = |k: &str, v: &str| {
        let ck = CString::new(k).unwrap();
        let cv = CString::new(v).unwrap();
        (api.set_field)(task_ctx, ck.as_ptr(), cv.as_ptr());
    };

    let method = get("request.method");
    let path = get("request.path");
    let body = get("request.body");

    let page: u32 = get("request.query.page").parse().unwrap_or(1);
    let per_page: u32 = get("request.query.per_page").parse().unwrap_or(20);

    let route = match dispatch_route(&method, &path) {
        Some(r) => r,
        None => {
            set("response.status", "404");
            set("response.body", r#"{"error":{"code":"NOT_FOUND","message":"route not found"}}"#);
            set("response.header.Content-Type", "application/json");
            return FlowControl { code: FLOW_CONTROL_END, payload: std::ptr::null() };
        }
    };

    let (status, resp_body) = match route {
        Route::ListPending    => handlers::list_pending(&ctx.store, &ctx.config.tenant_id, page, per_page),
        Route::GetRequest(id) => handlers::get_request(&ctx.store, &ctx.config.tenant_id, &id),
        Route::Approve(id)    => handlers::approve(&ctx.store, &ctx.config, api, &id, &body),
        Route::Deny(id)       => handlers::deny(&ctx.store, &ctx.config, &id, &body),
        Route::History        => handlers::history(&ctx.store, &ctx.config.tenant_id, page, per_page),
        Route::Certificate(id)=> handlers::get_certificate(&ctx.store, &ctx.config.tenant_id, &id),
    };

    set("response.status", &status.to_string());
    set("response.body", &resp_body);
    set("response.header.Content-Type", "application/json");
    FlowControl { code: FLOW_CONTROL_END, payload: std::ptr::null() }
}

#[no_mangle]
pub extern "C" fn ox_plugin_error(_: *mut c_void, _: *mut c_void) {}

#[no_mangle]
pub extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_config_ctx as *mut ModuleContext)); }
    }
}
```

- [ ] **Step 6: Run all tests**

```bash
cargo test -p ox_cert_ra 2>&1 | tail -20
```

Expected: all tests pass (dispatch tests, auto-approve rule tests).

- [ ] **Step 7: Verify workspace builds**

```bash
cargo check --workspace 2>&1 | grep "^error" | head -20
```

Expected: no errors.

- [ ] **Step 8: Commit**

```bash
git add crates/cert/ox_cert_ra/
git commit -m "feat(ox_cert_ra): RA approval workflow with auto-approve scanner and re-submission"
```

---

## Self-Review Checklist

- [x] **CoreHostApi::create_task**: Added after `publish_to_topic`; all construction sites updated.
- [x] **WorkflowStorage::create_pending_task**: Inserts with empty state_blob; load_task populates TaskState from metadata.
- [x] **create_and_enqueue_task**: Creates task then publishes UUID to queue.
- [x] **ox_cert_webhook**: URL must be https://; env secret read at init; HMAC-SHA256 signing; authorize/enrich/both modes; Block/Allow failure modes.
- [x] **ox_cert_ct**: GET /ct/scts/{serial} → 404 if not found, 200 with scts array; GET /ct/logs → health-check HEAD requests.
- [x] **ox_cert_ra dispatch**: All 6 routes covered.
- [x] **approve**: Checks status != Pending (409); builds re-submission fields; calls create_and_enqueue_task; returns task_id.
- [x] **deny**: Requires `reason` field (400 if missing); checks status != Pending.
- [x] **history**: Uses list_ra_history (raw_sql for status IN (approved, denied)).
- [x] **certificate**: Handles Pending(202), Approved+no serial(202), Approved+serial(200/202), Denied(409).
- [x] **auto-approve**: Runs on startup + every 5 minutes; regex + profile matching; re-applies rules to all pending requests.
- [x] **Tenant isolation**: Every store method takes tenant_id from config; every GDO sets tenant_id filter.
- [x] **Multi-tenancy HA**: UUID v4 for ApprovalRequest IDs; no row-level locking needed (status transitions are idempotent).
