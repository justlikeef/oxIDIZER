# ox_cert Core Issuance Plugins — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Prerequisite:** Plans 00 and 01 must be complete before this plan.

**Goal:** Build four cdylib plugins that form the core certificate lifecycle: `ox_cert_ca_init` (CA key bootstrap), `ox_cert_issue` (issuance), `ox_cert_renew` (renewal), and `ox_cert_revoke` (revocation).

**Architecture:** Each plugin is an independent cdylib exporting `ox_plugin_init`, `ox_plugin_process`, and `ox_plugin_destroy`. They share no in-process state — each opens its own `SoftwareKeyStore` and `OxPersistenceCertStore` from config. `ox_cert_ca_init` runs in phase `PreEarlyRequest` (background init); the others run in phase `Content` for specific route patterns.

**Tech Stack:** Rust 2021 cdylib, ox_cert_core (path dep), ox_workflow_abi (path dep), serde_json, uuid 1.6, time 0.3, x509-parser 0.16, reqwest 0.12 (blocking, for RA webhook).

---

## Plugin ABI Pattern

Every plugin follows this structure. All four plugins in this plan use it.

```rust
// src/lib.rs
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_END};
use std::ffi::c_void;

pub struct ModuleContext {
    store: Box<dyn ox_cert_core::CertStore>,
    key_store: ox_cert_core::keystore::software::SoftwareKeyStore,
    config: PluginConfig,
    // + any plugin-specific fields
}

#[no_mangle]
pub extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const libc::c_char,
    _api: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    let config: PluginConfig = match ox_cert_core::messaging::parse_config(plugin_config_ctx) {
        Ok(c) => c,
        Err(e) => { eprintln!("ox_cert_xxx init error: {e}"); return std::ptr::null_mut(); }
    };
    let store = match ox_cert_core::certstore::OxPersistenceCertStore::open(&config.store) {
        Ok(s) => s,
        Err(e) => { eprintln!("store open: {e}"); return std::ptr::null_mut(); }
    };
    Box::into_raw(Box::new(ModuleContext { store: Box::new(store), config })) as *mut c_void
}

#[no_mangle]
pub extern "C" fn ox_plugin_process(
    plugin_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    let ctx = unsafe { &*(plugin_ctx as *mut ModuleContext) };
    let api = /* passed via thread-local or stored at init */;
    match process(ctx, task_ctx, api) {
        Ok(fc) => fc,
        Err(e) => {
            set_field(task_ctx, "cert.error.code", e.error_code());
            set_field(task_ctx, "cert.error.message", &e.to_string());
            FlowControl { code: FLOW_CONTROL_END, payload: std::ptr::null() }
        }
    }
}

#[no_mangle]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_ctx as *mut ModuleContext)) };
    }
}
```

Helper: store the `CoreHostApi` pointer in `ModuleContext` at init time for use in process.

---

## File Map

```
crates/cert/
├── ox_cert_ca_init/
│   ├── Cargo.toml
│   └── src/lib.rs
├── ox_cert_issue/
│   ├── Cargo.toml
│   └── src/lib.rs
├── ox_cert_renew/
│   ├── Cargo.toml
│   └── src/lib.rs
└── ox_cert_revoke/
    ├── Cargo.toml
    └── src/lib.rs

Cargo.toml (workspace root)
    — add all four new crates
```

All four `Cargo.toml` files share the same dependency set (adjust per plugin):

```toml
[package]
name = "ox_cert_ca_init"   # (or issue, renew, revoke)
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[lib]
crate-type = ["cdylib"]

[dependencies]
ox_cert_core    = { path = "../ox_cert_core" }
ox_workflow_abi = { path = "../../../workflow/ox_workflow_abi" }
serde           = { version = "1.0", features = ["derive"] }
serde_json      = "1.0"
uuid            = { version = "1.6", features = ["v4"] }
time            = { version = "0.3", features = ["serde"] }
libc            = "0.2"
x509-parser     = "0.16"
rcgen           = { version = "0.13", features = ["pem"] }
```

---

## Task 1: Workspace + Cargo.toml for all four plugins

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/cert/ox_cert_ca_init/Cargo.toml`
- Create: `crates/cert/ox_cert_issue/Cargo.toml`
- Create: `crates/cert/ox_cert_renew/Cargo.toml`
- Create: `crates/cert/ox_cert_revoke/Cargo.toml`

- [ ] **Step 1: Add workspace members**

```toml
    # cert plugins — issuance
    "crates/cert/ox_cert_ca_init",
    "crates/cert/ox_cert_issue",
    "crates/cert/ox_cert_renew",
    "crates/cert/ox_cert_revoke",
```

- [ ] **Step 2: Create Cargo.toml for each plugin** using the template above with correct `name`.

- [ ] **Step 3: Create stub `src/lib.rs` for each**

```rust
// Minimal stub — just the ABI exports so it compiles
use libc::c_void;
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE};

#[no_mangle] pub extern "C" fn ox_plugin_init(_: *const libc::c_char, _: *const CoreHostApi, _: u32) -> *mut c_void { std::ptr::null_mut() }
#[no_mangle] pub extern "C" fn ox_plugin_process(_: *mut c_void, _: *mut c_void) -> FlowControl { FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() } }
#[no_mangle] pub extern "C" fn ox_plugin_destroy(_: *mut c_void) {}
```

- [ ] **Step 4: Verify all compile**

```bash
cargo build -p ox_cert_ca_init -p ox_cert_issue -p ox_cert_renew -p ox_cert_revoke 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/cert/ox_cert_ca_init/ crates/cert/ox_cert_issue/ crates/cert/ox_cert_renew/ crates/cert/ox_cert_revoke/
git commit -m "feat(ox_cert): scaffold four issuance plugin crates"
```

---

## Task 2: ox_cert_ca_init

**Spec:** `spec/plugin_ca_init.md`
**Phase:** `PreEarlyRequest`
**Routes:** None (background init only)

Config:

```rust
#[derive(Debug, serde::Deserialize)]
pub struct CaInitConfig {
    pub tenant_id: String,
    pub store: ox_cert_core::CertStoreConfig,
    pub keystore: ox_cert_core::KeyStoreConfig,
    pub ca_root_key_id: String,
    pub ca_root_cert_path: String,
    pub ca_root_subject: String,
    pub ca_root_key_type: ox_cert_core::KeyType,
    pub ca_root_validity_years: u32,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_intermediate_subject: String,
    pub ca_intermediate_key_type: ox_cert_core::KeyType,
    pub ca_intermediate_validity_years: u32,
    pub ca_intermediate_path_length: Option<u32>,
    pub auto_generate: bool,
    pub extensions: ox_cert_core::ExtensionsConfig,
}
```

**Processing** (`ox_plugin_init`):

1. Parse config.
2. Open `CertStore` and `SoftwareKeyStore`.
3. If `auto_generate` is true:
   a. Check `key_store.key_exists(tenant_id, root_key_id)`. If not: generate root key + self-signed CA cert, write cert to `ca_root_cert_path`, store `CaKeyRecord` with `status: Active`.
   b. Same for intermediate: generate key + CA cert signed by root, write cert to `ca_intermediate_cert_path`.
4. Set `cert.ca.ready = "true"` in a TaskState-equivalent (via `ModuleContext` flag).
5. `ox_plugin_process` returns `FLOW_CONTROL_CONTINUE` immediately (no request handling).

- [ ] **Step 1: Write integration test**

```rust
#[test]
fn test_ca_init_creates_keys_on_first_run() {
    let dir = TempDir::new().unwrap();
    let cert_dir = TempDir::new().unwrap();
    let config_json = serde_json::json!({
        "tenant_id": "test",
        "store": { "driver": "test_sqlite" },
        "keystore": { "store_type": "Software", "key_dir": dir.path() },
        "ca_root_key_id": "root",
        "ca_root_cert_path": cert_dir.path().join("root.crt").to_str().unwrap(),
        "ca_root_subject": "CN=Test Root CA",
        "ca_root_key_type": "EcP384",
        "ca_root_validity_years": 25,
        "ca_intermediate_key_id": "intermediate",
        "ca_intermediate_cert_path": cert_dir.path().join("intermediate.crt").to_str().unwrap(),
        "ca_intermediate_subject": "CN=Test Intermediate CA",
        "ca_intermediate_key_type": "EcP384",
        "ca_intermediate_validity_years": 10,
        "ca_intermediate_path_length": 0,
        "auto_generate": true,
        "extensions": {}
    }).to_string();

    // Simulate ox_plugin_init
    let ctx = simulate_init(&config_json);
    assert!(!ctx.is_null(), "init must succeed");

    // Verify files were created
    assert!(cert_dir.path().join("root.crt").exists());
    assert!(cert_dir.path().join("intermediate.crt").exists());
    // Verify keys exist in store
    let ks = open_key_store(dir.path());
    assert!(ks.key_exists("test", "root").unwrap());
    assert!(ks.key_exists("test", "intermediate").unwrap());
}

#[test]
fn test_ca_init_idempotent_no_overwrite() {
    // Run init twice — second run must not replace the first CA cert
    // (check CaKeyRecord remains the same)
}
```

- [ ] **Step 2: Implement `ox_plugin_init` for ca_init**

The init function calls `ensure_ca_keys(config, store, key_store)`:

```rust
fn ensure_ca_keys(config: &CaInitConfig, store: &dyn CertStore, key_store: &SoftwareKeyStore) -> Result<(), CertError> {
    // Root CA
    let root_exists = key_store.key_exists(&config.tenant_id, &config.ca_root_key_id)?;
    if !root_exists || /* no CaKeyRecord in store */ {
        key_store.generate_key(&config.tenant_id, &config.ca_root_key_id, config.ca_root_key_type.clone(), false)?;
        // Build self-signed root cert using CertBuilder
        let root_cert = build_self_signed_ca(/* ... */)?;
        std::fs::write(&config.ca_root_cert_path, &root_cert.pem)?;
        store.store_ca_key(&config.tenant_id, &CaKeyRecord { /* ... status: Active */ })?;
    }
    // Intermediate CA (signed by root)
    // ... same pattern ...
    Ok(())
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p ox_cert_ca_init 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add crates/cert/ox_cert_ca_init/
git commit -m "feat(ox_cert_ca_init): CA key bootstrap with auto_generate"
```

---

## Task 3: ox_cert_issue

**Spec:** `spec/plugin_issue.md`
**Phase:** `Content`
**Route:** `POST /api/v1/certificates`

Config:

```rust
#[derive(Debug, serde::Deserialize)]
pub struct IssueConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub default_profile: String,
    pub profiles: std::collections::HashMap<String, EnrollmentProfile>,
    pub policy: IssuancePolicyConfig,
    pub extensions: ExtensionsConfig,
    pub ct: Option<CtConfig>,
    pub ra_notification_webhook: Option<String>,
    pub server_keygen: bool,
}
```

**Processing (21 steps — see spec/plugin_issue.md):**

1. Read `request.method` / `request.path` — skip if not `POST /api/v1/certificates`.
2. Parse request body JSON: `{ csr, profile, sans? }`.
3. If `cert.ra.approved == "true"` (RA re-submission): skip RA check.
4. Parse and validate CSR with `x509-parser`.
5. Select `EnrollmentProfile` (from body `profile` or `default_profile`).
6. Run `IssuancePolicy::validate_csr`.
7. Apply webhook enrichment from `cert.webhook.enrichment` TaskState field.
8. Check if RA approval required (profile flag OR policy flag). If yes and not already approved: store `ApprovalRequest`, set 202 response, `FLOW_CONTROL_END`.
9. Generate UUID v4 serial.
10. Build `CertBuilder`, sign with intermediate CA key.
11. If CT enabled: `ox_cert_core::ct::submit_to_ct_logs(...)`.
12. Store `CertificateRecord` in `CertStore`.
13. `store.store_audit_event(Issue)`.
14. Set `cert.issued.serial`, `cert.issued.not_after`, `cert.issued.scts` in TaskState.
15. Set `response.status = "201"`, `response.body` = JSON envelope, `response.header.Content-Type = "application/json"`.
16. Return `FLOW_CONTROL_CONTINUE`.

- [ ] **Step 1: Write integration test**

```rust
#[test]
fn test_issue_basic_cert() {
    // Set up: register test driver, run ca_init to create CA keys
    // Build a CSR using rcgen
    // Call process() with request.body set
    // Verify response.status == "201", cert is in store

    let csr_pem = make_test_csr("api.example.com");
    let body = serde_json::json!({
        "csr": csr_pem,
        "profile": "standard"
    }).to_string();

    let task = TestTaskState::new();
    task.set("request.method", "POST");
    task.set("request.path", "/api/v1/certificates");
    task.set("request.body", &body);

    let fc = call_process(&mut ctx, &task);
    assert_eq!(fc.code, FLOW_CONTROL_CONTINUE);
    assert_eq!(task.get("response.status"), "201");

    let response: serde_json::Value = serde_json::from_str(&task.get("response.body")).unwrap();
    let serial = response["data"]["serial"].as_str().unwrap();
    // Verify cert exists in store
    let cert = ctx.store.get_cert_by_serial("test", serial).unwrap();
    assert!(cert.is_some());
}

#[test]
fn test_issue_policy_violation_rejected() {
    let csr_pem = make_test_csr("evil.test"); // blocked domain
    // ...
    // Expect response.status == "403", error code POLICY_VIOLATION
}
```

- [ ] **Step 2: Implement process() for ox_cert_issue**

Follow the 21-step processing from `spec/plugin_issue.md`. Key pieces:

```rust
fn process(ctx: &ModuleContext, task: &TaskState) -> Result<FlowControl, CertError> {
    // 1. Route guard
    if task.get("request.method") != "POST" || task.get("request.path") != "/api/v1/certificates" {
        return Ok(FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() });
    }
    // 2. Parse body
    let body: IssueRequest = serde_json::from_str(&task.get("request.body"))
        .map_err(|e| CertError::InvalidCsr(format!("bad request: {e}")))?;
    // 3. RA bypass check
    let ra_approved = task.get("cert.ra.approved") == "true";
    // 4. Parse CSR
    let csr_info = parse_csr(&body.csr)?;
    // 5. Select profile
    let profile = ctx.config.profiles.get(&body.profile.unwrap_or(ctx.config.default_profile.clone()))
        .ok_or_else(|| CertError::ConfigError(format!("unknown profile")))?;
    // 6. Policy validation
    if !ra_approved { ctx.policy.validate_csr(&csr_info)?; }
    // 7. Apply webhook enrichment (additional_sans, subject_ou)
    apply_enrichment(&mut csr_info, &task.get("cert.webhook.enrichment"))?;
    // 8. RA check
    if !ra_approved && profile.require_ra_approval {
        let req = store_approval_request(&ctx, &csr_info, &body)?;
        set_response(task, 202, json!({"data": {"status": "pending", "request_id": req.id}}));
        return Ok(FLOW_CONTROL_END);
    }
    // 9. UUID serial
    let serial = uuid::Uuid::new_v4().to_string();
    // 10. Sign cert
    let issuer_cert = load_issuer_cert(&ctx.config.ca_intermediate_cert_path)?;
    let built = CertBuilder::new(profile)
        .subject(DistinguishedName { common_name: csr_info.subject_cn.clone(), ..Default::default() })
        .validity(/* from profile.validity_seconds */)
        .build_and_sign(&issuer_cert, &ctx.key_store, &ctx.config.tenant_id, &ctx.config.ca_intermediate_key_id, &ctx.config.extensions)?;
    // 11. CT submission
    let scts = if let Some(ct) = &ctx.config.ct {
        ox_cert_core::ct::submit_to_ct_logs(&built.der, &issuer_cert_der, ct)
            .unwrap_or_default()  // on_failure=warn → ignore
    } else { vec![] };
    // 12. Store
    let record = CertificateRecord { serial: serial.clone(), /* ... */ };
    ctx.store.store_cert(&ctx.config.tenant_id, &record)?;
    // 13. Audit
    ctx.store.store_audit_event(&ctx.config.tenant_id, &AuditEvent { action: AuditAction::Issue, /* ... */ })?;
    // 14-15. Set response
    set_response(task, 201, build_response_body(&record));
    Ok(FLOW_CONTROL_CONTINUE)
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p ox_cert_issue 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add crates/cert/ox_cert_issue/
git commit -m "feat(ox_cert_issue): certificate issuance with CSR validation, policy, CT, RA check"
```

---

## Task 4: ox_cert_renew

**Spec:** `spec/plugin_renew.md`
**Route:** `POST /api/v1/certificates/{serial}/renew`

Config adds:
```rust
pub auto_revoke_on_renew: bool,
pub rekey_allowed: bool,  // if false, require same public key
```

**Processing:**

1. Parse `{serial}` from path.
2. `store.get_cert_by_serial(tenant_id, serial)` → 404 if not found.
3. Assert `status == Active` → 409 if already revoked.
4. Parse optional `csr` from body (re-key) or reuse existing `csr_pem`.
5. Parse/validate CSR, apply same policy as issue.
6. Sign new cert (same profile as original).
7. If `auto_revoke_on_renew`: call `store.mark_revoked` on original cert with reason `Superseded`.
8. Store new cert, audit.
9. Return 201 with new cert.

- [ ] **Step 1: Write tests**

```rust
#[test]
fn test_renew_active_cert() {
    // Issue a cert first, then renew it
    // Verify new cert has different serial
    // If auto_revoke: original cert status == Revoked
}

#[test]
fn test_renew_already_revoked_fails() {
    // Revoke cert, then try to renew → expect 409
}
```

- [ ] **Step 2: Implement and run tests**

```bash
cargo test -p ox_cert_renew 2>&1 | tail -5
```

- [ ] **Step 3: Commit**

```bash
git add crates/cert/ox_cert_renew/
git commit -m "feat(ox_cert_renew): certificate renewal with optional auto-revoke"
```

---

## Task 5: ox_cert_revoke

**Spec:** `spec/plugin_revoke.md`
**Route:** `POST /api/v1/certificates/{serial}/revoke`

**Processing:**

1. Parse `{serial}` from path.
2. `store.get_cert_by_serial` → 404 if not found.
3. Check `status != Revoked` → 409 `ALREADY_REVOKED` if already revoked.
4. Parse `reason` from body (default: `Unspecified`).
5. `store.mark_revoked(tenant_id, serial, reason, now)`.
6. `store.store_audit_event(Revoke)`.
7. Return 200 `{"data": {"serial": "...", "status": "revoked"}}`.

- [ ] **Step 1: Write tests**

```rust
#[test]
fn test_revoke_active_cert() {
    // Store a cert, revoke it, verify status
}
#[test]
fn test_revoke_already_revoked_returns_409() { /* ... */ }
#[test]
fn test_revoke_not_found_returns_404() { /* ... */ }
#[test]
fn test_revoke_with_explicit_reason() {
    // Body: {"reason": "KeyCompromise"}
    // Verify cert.revocation_reason == KeyCompromise
}
```

- [ ] **Step 2: Implement**

```rust
fn process(ctx: &ModuleContext, task: &TaskState) -> Result<FlowControl, CertError> {
    let serial = extract_serial_from_path(&task.get("request.path"), "revoke")?;
    let cert = ctx.store.get_cert_by_serial(&ctx.config.tenant_id, &serial)?
        .ok_or_else(|| CertError::NotFound(serial.clone()))?;
    if cert.status == CertStatus::Revoked {
        return Err(CertError::AlreadyRevoked(serial));
    }
    let body: RevokeRequest = serde_json::from_str(&task.get("request.body")).unwrap_or_default();
    let reason = body.reason.unwrap_or(RevocationReason::Unspecified);
    ctx.store.mark_revoked(&ctx.config.tenant_id, &serial, reason, time::OffsetDateTime::now_utc())?;
    ctx.store.store_audit_event(&ctx.config.tenant_id, &AuditEvent { action: AuditAction::Revoke, serial: Some(serial.clone()), /* ... */ })?;
    set_response(task, 200, json!({"data": {"serial": serial, "status": "revoked"}}));
    Ok(FLOW_CONTROL_CONTINUE)
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p ox_cert_revoke 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add crates/cert/ox_cert_revoke/
git commit -m "feat(ox_cert_revoke): certificate revocation with reason code"
```

---

## Self-Review Checklist

- [x] **ABI exports**: All four plugins export `ox_plugin_init`, `ox_plugin_process`, `ox_plugin_destroy` with correct `extern "C"` and `#[no_mangle]`.
- [x] **Config parsing**: Uses `ox_cert_core::messaging::parse_config()` — null-safe, logs error on failure.
- [x] **Plugin init failure**: Returns `null_mut()` on any init error; `ox_plugin_process` is never called.
- [x] **Tenant isolation**: Every store call uses `config.tenant_id`.
- [x] **RA re-submission bypass**: `cert.ra.approved == "true"` in TaskState skips the RA check in issue.
- [x] **Error response format**: Errors set `cert.error.code` / `cert.error.message` and return `FLOW_CONTROL_END`.
- [x] **UUID serial**: Generated with `uuid::Uuid::new_v4().to_string()`.
- [x] **CT on_failure=warn**: CT errors are logged, not returned, when `on_failure = Warn`.
- [x] **auto_revoke_on_renew**: Renew marks old cert `Revoked` with `Superseded` reason when flag is set.
- [x] **Audit events**: All three lifecycle operations (Issue, Renew, Revoke) emit audit events.
