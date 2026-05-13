# Security Pipeline FFI Plugin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `ox_security_pipeline` a workflow-engine FFI plugin (`cdylib`) that exposes an authenticate endpoint and per-crate admin routes for authz grants and accounting events.

**Architecture:** The crate gains `crate-type = ["cdylib", "rlib"]` and a new `src/plugin.rs` module. Plugin state holds a `SecurityPipeline` (composed of an in-plugin `ApiKeyAuthDriverSimple`, an `InMemoryAuthzDriver`, and a `MemoryAccountingDriver`) plus a `tokio::runtime::Runtime` for blocking on async pipeline calls. Routes: `POST /api/v1/security/authenticate`, `GET /api/v1/security/health`, `GET /api/v1/admin/accounting/events`, `GET|POST|DELETE /api/v1/admin/authz/grants`.

**Tech Stack:** `ox_workflow_abi`, `ox_fileproc`, `ox_security_pipeline`, `ox_security_core`, `ox_security_accounting::MemoryAccountingDriver`, `async-trait`, `secrecy`, `serde_json`, `tokio`, `uuid`

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `crates/security/ox_security_pipeline/Cargo.toml` | Add `cdylib`, add new deps |
| Modify | `crates/security/ox_security_pipeline/src/lib.rs` | Export `pub mod plugin` |
| Create | `crates/security/ox_security_pipeline/src/plugin.rs` | Full FFI ABI + all handlers |
| Create | `crates/security/ox_security_pipeline/conf/plugin.yaml` | Plugin config template |
| Create | `personas/security/modules/available/ox_security_pipeline.yaml` | Persona module file |

---

### Task 1: Update Cargo.toml and lib.rs

**Files:**
- Modify: `crates/security/ox_security_pipeline/Cargo.toml`
- Modify: `crates/security/ox_security_pipeline/src/lib.rs`

- [ ] **Step 1: Update Cargo.toml**

Replace contents of `crates/security/ox_security_pipeline/Cargo.toml`:

```toml
[package]
name = "ox_security_pipeline"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
ox_security_core       = { path = "../ox_security_core" }
ox_security_auth       = { path = "../ox_security_auth" }
ox_security_authz      = { path = "../ox_security_authz" }
ox_security_accounting = { path = "../ox_security_accounting" }
ox_workflow_abi        = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc            = { path = "../../util/ox_fileproc" }
async-trait            = "0.1"
thiserror              = "1"
chrono                 = { version = "0.4", features = ["serde"] }
serde                  = { version = "1", features = ["derive"] }
serde_json             = "1"
secrecy                = { version = "0.8" }
tokio                  = { version = "1", features = ["rt", "sync"] }
uuid                   = { version = "1", features = ["v4"] }

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt"] }
secrecy = { version = "0.8", features = ["serde"] }
```

- [ ] **Step 2: Add plugin module to lib.rs**

Append to `crates/security/ox_security_pipeline/src/lib.rs`:

```rust
pub mod plugin;
```

- [ ] **Step 3: Verify it compiles**

```bash
cd /var/repos/oxIDIZER
cargo build -p ox_security_pipeline 2>&1 | tail -5
```

Expected: no errors (plugin.rs doesn't exist yet, so expect "file not found" for plugin — that's OK, just the module declaration for now).

Actually, creating an empty `plugin.rs` first:
```bash
touch crates/security/ox_security_pipeline/src/plugin.rs
cargo build -p ox_security_pipeline 2>&1 | tail -5
```
Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/security/ox_security_pipeline/Cargo.toml crates/security/ox_security_pipeline/src/lib.rs crates/security/ox_security_pipeline/src/plugin.rs
git commit -m "feat(security-pipeline): add cdylib target and plugin module scaffold"
```

---

### Task 2: Implement plugin.rs — config + in-memory drivers

**Files:**
- Create: `crates/security/ox_security_pipeline/src/plugin.rs`

- [ ] **Step 1: Write failing test for config parsing**

At the bottom of `src/plugin.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_parses() {
        let yaml = r#"
tenant_id: "default"
api_keys:
  - key: "testkey"
    principal_id: "00000000-0000-0000-0000-000000000001"
    display_name: "Test User"
    groups: ["admins"]
"#;
        let v: serde_json::Value = serde_yaml::from_str(yaml)
            .unwrap_or_else(|_| serde_json::from_str(yaml).unwrap());
        let cfg: PipelinePluginConfig = serde_json::from_value(v).unwrap();
        assert_eq!(cfg.tenant_id, "default");
        assert_eq!(cfg.api_keys.len(), 1);
        assert_eq!(cfg.api_keys[0].display_name, "Test User");
    }
}
```

- [ ] **Step 2: Run test — expect fail (PipelinePluginConfig not defined)**

```bash
cargo test -p ox_security_pipeline plugin::tests::test_config_parses 2>&1 | tail -10
```
Expected: compile error "PipelinePluginConfig not found"

- [ ] **Step 3: Write config types and in-memory drivers**

Write the full `src/plugin.rs`:

```rust
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::net::{IpAddr, Ipv4Addr};
use std::panic;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use secrecy::ExposeSecret;
use serde::Deserialize;
use uuid::Uuid;

use ox_security_accounting::MemoryAccountingDriver;
use ox_security_authz::PermissionGrant;
use ox_security_core::{
    AccountingDriver, AuthDriver, AuthPipelineContext, AuthResult, AuthzDriver, AuthzResult,
    Credentials, GroupId, Principal, PrincipalId, TenantId,
    AuthSource,
};
use ox_security_pipeline::{SecurityPipeline, SecurityPipelineBuilder};
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO,
    OX_WORKFLOW_ABI_VERSION,
};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ApiKeyEntry {
    key: String,
    principal_id: Uuid,
    display_name: String,
    #[serde(default)]
    groups: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PipelinePluginConfig {
    tenant_id: String,
    #[serde(default)]
    api_keys: Vec<ApiKeyEntry>,
}

// ---------------------------------------------------------------------------
// In-memory API key auth driver
// ---------------------------------------------------------------------------

struct ApiKeyAuthDriverSimple {
    // key -> (principal_id, display_name, groups, tenant_id)
    keys: HashMap<String, (Uuid, String, Vec<String>, String)>,
}

#[async_trait]
impl AuthDriver for ApiKeyAuthDriverSimple {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let key_val = match credentials {
            Credentials::ApiKey { key } => key.expose_secret().to_string(),
            _ => return AuthResult::Continue,
        };
        match self.keys.get(&key_val) {
            Some((pid, name, groups, tenant)) => AuthResult::Authenticated(Principal {
                id: PrincipalId::from_uuid(*pid),
                display_name: name.clone(),
                source: AuthSource::Local,
                groups: groups.iter().map(|g| GroupId::new(g)).collect(),
                tenant_id: TenantId::from(tenant.as_str()),
                session_id: None,
            }),
            None => AuthResult::Continue,
        }
    }
}

// ---------------------------------------------------------------------------
// In-memory authz driver
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GrantEntry {
    pub principal_id: String,
    pub operation: String,
    pub resource_pattern: Option<String>,
}

struct InMemoryAuthzDriver {
    grants: Arc<Mutex<Vec<GrantEntry>>>,
}

#[async_trait]
impl AuthzDriver for InMemoryAuthzDriver {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        let pid = principal.id.as_uuid().to_string();
        let store = self.grants.lock().unwrap_or_else(|p| p.into_inner());
        for entry in store.iter().filter(|e| e.principal_id == pid && e.operation == operation) {
            match &entry.resource_pattern {
                None => return AuthzResult::Allow,
                Some(pat) if pat.ends_with("/*") => {
                    let prefix = &pat[..pat.len() - 2];
                    if path.starts_with(prefix) {
                        return AuthzResult::Allow;
                    }
                }
                Some(pat) if pat == path => return AuthzResult::Allow,
                _ => {}
            }
        }
        AuthzResult::Continue
    }
}

// ---------------------------------------------------------------------------
// Plugin state
// ---------------------------------------------------------------------------

struct PluginState {
    api: CoreHostApi,
    config: PipelinePluginConfig,
    pipeline: SecurityPipeline,
    memory_accounting: Arc<MemoryAccountingDriver>,
    grant_store: Arc<Mutex<Vec<GrantEntry>>>,
    runtime: tokio::runtime::Runtime,
}
unsafe impl Send for PluginState {}
unsafe impl Sync for PluginState {}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) {
        (api.log)(task_ctx, level, c.as_ptr());
    }
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let Ok(k) = CString::new(key) else { return String::new() };
    let ptr = (api.get_field)(task_ctx, k.as_ptr());
    if ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, val: &str) {
    if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(val)) {
        (api.set_field)(task_ctx, k.as_ptr(), v.as_ptr());
    }
}

fn json_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

fn handle_authenticate(state: &PluginState, task_ctx: *mut c_void, body: &str) {
    #[derive(Deserialize)]
    struct AuthBody { api_key: String }
    let parsed: AuthBody = match serde_json::from_str(body) {
        Ok(b) => b,
        Err(_) => {
            json_response(&state.api, task_ctx, 400,
                r#"{"error":{"code":"INVALID_REQUEST","message":"expected {\"api_key\":\"...\"}"}}"#);
            return;
        }
    };
    let creds = Credentials::ApiKey {
        key: secrecy::SecretString::new(parsed.api_key),
    };
    let mut ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: state.config.tenant_id.parse().unwrap_or_else(|_| TenantId::from("default")),
        source_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
    };
    match state.runtime.block_on(state.pipeline.authenticate(&creds, &mut ctx)) {
        Ok(principal) => {
            let body = serde_json::json!({
                "data": {
                    "principal_id": principal.id.as_uuid().to_string(),
                    "display_name": principal.display_name,
                    "tenant_id": principal.tenant_id.as_str(),
                    "groups": principal.groups.iter().map(|g| g.as_str()).collect::<Vec<_>>(),
                }
            });
            json_response(&state.api, task_ctx, 200, &body.to_string());
        }
        Err(e) => {
            let body = serde_json::json!({"error":{"code":"AUTH_FAILED","message":e.to_string()}});
            json_response(&state.api, task_ctx, 401, &body.to_string());
        }
    }
}

fn handle_health(state: &PluginState, task_ctx: *mut c_void) {
    json_response(&state.api, task_ctx, 200,
        r#"{"data":{"status":"ok","service":"ox_security_pipeline"}}"#);
}

fn handle_accounting_events(state: &PluginState, task_ctx: *mut c_void) {
    let events = state.memory_accounting.events();
    let parsed: Vec<serde_json::Value> = events.iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect();
    let body = serde_json::json!({"data": parsed, "meta": {"count": parsed.len()}});
    // Note: parsed moved above; recalculate length
    let events2 = state.memory_accounting.events();
    let parsed2: Vec<serde_json::Value> = events2.iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect();
    let len = parsed2.len();
    let body = serde_json::json!({"data": parsed2, "meta": {"count": len}});
    json_response(&state.api, task_ctx, 200, &body.to_string());
}

fn handle_grants_list(state: &PluginState, task_ctx: *mut c_void) {
    let grants = state.grant_store.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let body = serde_json::json!({"data": grants, "meta": {"count": grants.len()}});
    json_response(&state.api, task_ctx, 200, &body.to_string());
}

fn handle_grants_create(state: &PluginState, task_ctx: *mut c_void, body: &str) {
    let entry: GrantEntry = match serde_json::from_str(body) {
        Ok(e) => e,
        Err(_) => {
            json_response(&state.api, task_ctx, 400,
                r#"{"error":{"code":"INVALID_REQUEST","message":"expected {\"principal_id\":\"...\",\"operation\":\"...\",\"resource_pattern\":null}"}}"#);
            return;
        }
    };
    if entry.principal_id.is_empty() || entry.operation.is_empty() {
        json_response(&state.api, task_ctx, 400,
            r#"{"error":{"code":"INVALID_REQUEST","message":"principal_id and operation are required"}}"#);
        return;
    }
    state.grant_store.lock().unwrap_or_else(|p| p.into_inner()).push(entry.clone());
    let body = serde_json::json!({"data": entry});
    json_response(&state.api, task_ctx, 201, &body.to_string());
}

fn handle_grants_delete(state: &PluginState, task_ctx: *mut c_void, principal_id: &str, operation: &str) {
    let mut store = state.grant_store.lock().unwrap_or_else(|p| p.into_inner());
    let before = store.len();
    store.retain(|e| !(e.principal_id == principal_id && e.operation == operation));
    let removed = before - store.len();
    let body = serde_json::json!({"data": {"removed": removed}});
    json_response(&state.api, task_ctx, 200, &body.to_string());
}

// ---------------------------------------------------------------------------
// FFI ABI
// ---------------------------------------------------------------------------

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
    let params_str = if config_ptr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(config_ptr).to_string_lossy().to_string() }
    };
    let params: serde_json::Value = serde_json::from_str(&params_str)
        .unwrap_or(serde_json::Value::Null);
    let config_path = match params.get("config_file").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                "ox_security_pipeline: missing config_file param");
            return std::ptr::null_mut();
        }
    };
    let config: PipelinePluginConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_security_pipeline: config error: {}", e));
                return std::ptr::null_mut();
            }
        },
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_security_pipeline: failed to load config: {}", e));
            return std::ptr::null_mut();
        }
    };

    let key_map: HashMap<String, (Uuid, String, Vec<String>, String)> = config.api_keys.iter()
        .map(|e| (e.key.clone(), (e.principal_id, e.display_name.clone(), e.groups.clone(), config.tenant_id.clone())))
        .collect();
    let auth_driver = Arc::new(ApiKeyAuthDriverSimple { keys: key_map });

    let grant_store: Arc<Mutex<Vec<GrantEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let grant_store_clone = grant_store.clone();
    let authz_driver = Arc::new(InMemoryAuthzDriver { grants: grant_store_clone });

    let memory_accounting = Arc::new(MemoryAccountingDriver::new());
    let accounting_driver: Arc<dyn AccountingDriver> = memory_accounting.clone();

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_security_pipeline: failed to create runtime: {}", e));
            return std::ptr::null_mut();
        }
    };

    let pipeline = SecurityPipelineBuilder::new()
        .auth(auth_driver)
        .authz(authz_driver)
        .accounting(accounting_driver)
        .build();

    log(&api, std::ptr::null_mut(), OX_LOG_INFO,
        &format!("ox_security_pipeline: initialized for tenant '{}'", config.tenant_id));

    Box::into_raw(Box::new(PluginState {
        api,
        config,
        pipeline,
        memory_accounting,
        grant_store,
        runtime,
    })) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_process(
    plugin_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    if plugin_ctx.is_null() { return cont; }
    let state = unsafe { &*(plugin_ctx as *mut PluginState) };

    panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let method = get_field(&state.api, task_ctx, "request.method").to_uppercase();
        let path   = get_field(&state.api, task_ctx, "request.path");
        let body   = get_field(&state.api, task_ctx, "request.body");

        let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();

        match (method.as_str(), segs.get(0).copied(), segs.get(1).copied(),
               segs.get(2).copied(), segs.get(3).copied(), segs.get(4).copied()) {
            // POST /api/v1/security/authenticate
            ("POST", Some("api"), Some("v1"), Some("security"), Some("authenticate"), None) => {
                handle_authenticate(state, task_ctx, &body);
            }
            // GET /api/v1/security/health
            ("GET", Some("api"), Some("v1"), Some("security"), Some("health"), None) => {
                handle_health(state, task_ctx);
            }
            // GET /api/v1/admin/accounting/events
            ("GET", Some("api"), Some("v1"), Some("admin"), Some("accounting"), Some("events")) => {
                handle_accounting_events(state, task_ctx);
            }
            // GET /api/v1/admin/authz/grants
            ("GET", Some("api"), Some("v1"), Some("admin"), Some("authz"), Some("grants")) => {
                handle_grants_list(state, task_ctx);
            }
            // POST /api/v1/admin/authz/grants
            ("POST", Some("api"), Some("v1"), Some("admin"), Some("authz"), Some("grants")) => {
                handle_grants_create(state, task_ctx, &body);
            }
            // DELETE /api/v1/admin/authz/grants/{principal_id}/{operation}
            ("DELETE", Some("api"), Some("v1"), Some("admin"), Some("authz"), Some("grants")) => {
                // path segment 6 = principal_id, 7 = operation
                let pid = segs.get(5).copied().unwrap_or("");
                let op  = segs.get(6).copied().unwrap_or("");
                handle_grants_delete(state, task_ctx, pid, op);
            }
            _ => { /* not our route */ }
        }
        cont
    }))
    .unwrap_or_else(|_| {
        log(&state.api, task_ctx, OX_LOG_ERROR, "ox_security_pipeline: panic in process");
        cont
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_error(_ctx: *mut c_void, _task: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginState)); }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> PipelinePluginConfig {
        PipelinePluginConfig {
            tenant_id: "test".to_string(),
            api_keys: vec![ApiKeyEntry {
                key: "testkey".to_string(),
                principal_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                display_name: "Test User".to_string(),
                groups: vec!["admins".to_string()],
            }],
        }
    }

    fn make_state() -> PluginState {
        let config = make_config();
        let key_map: HashMap<String, (Uuid, String, Vec<String>, String)> = config.api_keys.iter()
            .map(|e| (e.key.clone(), (e.principal_id, e.display_name.clone(), e.groups.clone(), config.tenant_id.clone())))
            .collect();
        let grant_store: Arc<Mutex<Vec<GrantEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let memory_accounting = Arc::new(MemoryAccountingDriver::new());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let pipeline = SecurityPipelineBuilder::new()
            .auth(Arc::new(ApiKeyAuthDriverSimple { keys: key_map }))
            .authz(Arc::new(InMemoryAuthzDriver { grants: grant_store.clone() }))
            .accounting(memory_accounting.clone())
            .build();
        // Using a dummy CoreHostApi is not practical in unit tests without the workflow engine.
        // These tests focus on the logic functions; plugin.rs FFI is tested via integration tests.
        // For now, just verify the pipeline builds and auth works.
        PluginState {
            api: unsafe { std::mem::zeroed() }, // not called in logic tests
            config,
            pipeline,
            memory_accounting,
            grant_store,
            runtime,
        }
    }

    #[test]
    fn test_config_parses() {
        let json = r#"{"tenant_id":"default","api_keys":[{"key":"testkey","principal_id":"00000000-0000-0000-0000-000000000001","display_name":"Test User","groups":["admins"]}]}"#;
        let cfg: PipelinePluginConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.tenant_id, "default");
        assert_eq!(cfg.api_keys.len(), 1);
    }

    #[test]
    fn test_pipeline_authenticates_valid_api_key() {
        let state = make_state();
        let creds = Credentials::ApiKey {
            key: secrecy::SecretString::new("testkey".to_string()),
        };
        let mut ctx = AuthPipelineContext {
            partial_principal: None,
            tenant_id: "test".parse().unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        let result = state.runtime.block_on(state.pipeline.authenticate(&creds, &mut ctx));
        assert!(result.is_ok());
        let principal = result.unwrap();
        assert_eq!(principal.display_name, "Test User");
    }

    #[test]
    fn test_pipeline_rejects_unknown_api_key() {
        let state = make_state();
        let creds = Credentials::ApiKey {
            key: secrecy::SecretString::new("badkey".to_string()),
        };
        let mut ctx = AuthPipelineContext {
            partial_principal: None,
            tenant_id: "test".parse().unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        let result = state.runtime.block_on(state.pipeline.authenticate(&creds, &mut ctx));
        assert!(result.is_err());
    }

    #[test]
    fn test_grant_store_crud() {
        let state = make_state();
        let entry = GrantEntry {
            principal_id: "test-pid".to_string(),
            operation: "read".to_string(),
            resource_pattern: Some("files/*".to_string()),
        };
        state.grant_store.lock().unwrap().push(entry);
        let grants = state.grant_store.lock().unwrap().clone();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].operation, "read");

        // Remove it
        state.grant_store.lock().unwrap()
            .retain(|e| !(e.principal_id == "test-pid" && e.operation == "read"));
        let grants = state.grant_store.lock().unwrap().clone();
        assert!(grants.is_empty());
    }
}
```

> **Note on `PrincipalId::from_uuid` and `TenantId::from`:** Check if these methods exist. If `PrincipalId` only has `new()` (which generates a new UUID), you'll need to add a `pub fn from_uuid(u: Uuid) -> Self { Self(u) }` to `ox_security_core/src/types.rs`. Similarly, if `TenantId::from(&str)` doesn't exist, add `impl From<&str> for TenantId { fn from(s: &str) -> Self { Self(s.to_string()) } }`. Check the types.rs file before attempting to compile.

- [ ] **Step 4: Run tests — expect compile errors first**

```bash
cargo test -p ox_security_pipeline plugin::tests 2>&1 | tail -20
```

Expected: compile errors about missing methods. Fix them.

- [ ] **Step 5: Fix missing constructors in ox_security_core if needed**

Check `crates/security/ox_security_core/src/types.rs`. If `PrincipalId::from_uuid` is missing, add:

```rust
impl PrincipalId {
    pub fn from_uuid(u: Uuid) -> Self { Self(u) }
}
```

If `TenantId::From<&str>` is missing, add:
```rust
impl From<&str> for TenantId {
    fn from(s: &str) -> Self { Self(s.to_string()) }
}
```

- [ ] **Step 6: Run tests until they pass**

```bash
cargo test -p ox_security_pipeline plugin::tests 2>&1 | tail -20
```

Expected: all 4 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/security/ox_security_pipeline/src/plugin.rs crates/security/ox_security_core/src/types.rs
git commit -m "feat(security-pipeline-plugin): implement FFI plugin with authenticate, health, grants, and accounting admin endpoints (4 tests)"
```

---

### Task 3: Create config template and persona YAML

**Files:**
- Create: `crates/security/ox_security_pipeline/conf/plugin.yaml`
- Create: `personas/security/modules/available/ox_security_pipeline.yaml`

- [ ] **Step 1: Create plugin config template**

Create `crates/security/ox_security_pipeline/conf/plugin.yaml`:

```yaml
tenant_id: "default"
api_keys:
  - key: "changeme-replace-with-strong-secret"
    principal_id: "00000000-0000-0000-0000-000000000001"
    display_name: "Administrator"
    groups:
      - "admins"
```

- [ ] **Step 2: Create personas/security directory and module YAML**

```bash
mkdir -p personas/security/modules/available
```

Create `personas/security/modules/available/ox_security_pipeline.yaml`:

```yaml
modules:
  - id: "security_pipeline"
    name: "ox_security_pipeline"
    phase: Content
    params:
      config_file: "${{OX_BASE}}/crates/security/ox_security_pipeline/conf/plugin.yaml"

routes:
  - url: "^/api/v1/security/authenticate$"
    module_id: "security_pipeline"
    priority: 100
  - url: "^/api/v1/security/health$"
    module_id: "security_pipeline"
    priority: 100
  - url: "^/api/v1/admin/accounting(/.*)?$"
    module_id: "security_pipeline"
    priority: 150
  - url: "^/api/v1/admin/authz(/.*)?$"
    module_id: "security_pipeline"
    priority: 150
```

- [ ] **Step 3: Verify full build compiles cleanly**

```bash
cargo build -p ox_security_pipeline 2>&1 | tail -5
```

Expected: builds without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/security/ox_security_pipeline/conf/ personas/security/
git commit -m "feat(security-pipeline-plugin): add plugin config template and persona YAML"
```
