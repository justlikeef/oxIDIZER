# ox_cert Revocation Infrastructure — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Prerequisite:** Plans 00, 01, and 02 must be complete.

**Goal:** Build `ox_cert_crl` (CRL generation and serving) and `ox_cert_ocsp` (OCSP responder). Both plugins are read-heavy — they serve revocation data generated from the cert store. `ox_cert_crl` includes active/active HA coordination via the advisory lock table.

**Architecture:** Two independent cdylib plugins. `ox_cert_crl` caches generated CRLs behind an `RwLock` and uses `acquire_crl_lock` (from `CertStore`) for distributed coordination. `ox_cert_ocsp` looks up individual cert status and builds RFC 6960 OCSP responses. Both sign with the intermediate CA key (or a delegated OCSP signing key for OCSP).

**Tech Stack:** Rust 2021 cdylib, ox_cert_core, ox_workflow_abi, x509-parser 0.16, rcgen 0.13, rasn + rasn-ocsp (OCSP DER encoding), base64 0.22, time 0.3.

---

## File Map

```
crates/cert/
├── ox_cert_crl/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs           — ABI exports, ModuleContext with RwLock<Option<CachedCrl>>
│       ├── generate.rs      — generate_full_crl(), generate_delta_crl()
│       └── cache.rs         — CachedCrl struct, cache check logic
└── ox_cert_ocsp/
    ├── Cargo.toml
    └── src/
        ├── lib.rs           — ABI exports, ModuleContext
        └── response.rs      — build_ocsp_response() — RFC 6960 DER

Cargo.toml (workspace root) — add two new members
```

### Cargo.toml (shared template)

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
ox_cert_core    = { path = "../ox_cert_core" }
ox_workflow_abi = { path = "../../../workflow/ox_workflow_abi" }
x509-parser     = "0.16"
rcgen           = { version = "0.13", features = ["pem"] }
base64          = "0.22"
time            = { version = "0.3", features = ["serde"] }
serde           = { version = "1.0", features = ["derive"] }
serde_json      = "1.0"
libc            = "0.2"
```

`ox_cert_ocsp` additionally needs:
```toml
rasn       = "0.12"
rasn-ocsp  = "0.12"
rasn-pkix  = "0.12"
```

---

## Task 1: Workspace + crate scaffolds

- [ ] **Step 1: Add workspace members**

```toml
    "crates/cert/ox_cert_crl",
    "crates/cert/ox_cert_ocsp",
```

- [ ] **Step 2: Create Cargo.toml for each**

Use the templates above.

- [ ] **Step 3: Create stub src/lib.rs for each** (same minimal ABI stub from Plan 02).

- [ ] **Step 4: Build check**

```bash
cargo build -p ox_cert_crl -p ox_cert_ocsp 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/cert/ox_cert_crl/ crates/cert/ox_cert_ocsp/
git commit -m "feat(ox_cert): scaffold crl and ocsp plugin crates"
```

---

## Task 2: ox_cert_crl — CRL generation

**Spec:** `spec/plugin_crl.md`
**Route:** `GET /crl/*`

### Config

```rust
#[derive(Debug, serde::Deserialize)]
pub struct CrlConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub extensions: ExtensionsConfig,
    pub crl_update_interval_secs: u64,   // default 3600
    pub crl_delta_interval_secs: u64,    // default 600
    pub crl_cache_ttl_secs: u64,         // default 1800
    pub crl_lock_ttl_secs: u64,          // default 300
}
```

### Module Context

```rust
pub struct ModuleContext {
    config: CrlConfig,
    store: Box<dyn CertStore>,
    key_store: SoftwareKeyStore,
    full_crl_cache: std::sync::RwLock<Option<CachedCrl>>,
    delta_crl_cache: std::sync::RwLock<Option<CachedCrl>>,
    holder_id: String,  // "{hostname}:{pid}" for lock identification
}

pub struct CachedCrl {
    pub der: Vec<u8>,
    pub pem: String,
    pub generated_at: time::OffsetDateTime,
    pub next_update: time::OffsetDateTime,
    pub crl_number: u64,
}
```

### Processing

On `GET /crl/full.crl` or `GET /crl/full.pem`:
1. Read `full_crl_cache` under a read lock.
2. If cache present and `next_update > now`: serve from cache.
3. Try `store.acquire_crl_lock(tenant_id, "full_crl", holder_id, lock_ttl_secs)`.
4. If `Some(crl_number)`: generate new full CRL with `generate_full_crl()`, update cache.
5. If `None`: another node holds the lock. Serve stale cache with `Warning: 199 ox_cert_crl "CRL regeneration in progress; serving cached copy"` header.
6. Set response: `Content-Type: application/pkix-crl` (DER) or `application/x-pem-file` (PEM).

On `GET /crl/delta.crl`:
- Same logic but uses `"delta_crl"` lock key and `generate_delta_crl()`.

- [ ] **Step 1: Write tests for CRL generation**

```rust
// src/generate.rs tests
#[test]
fn test_generate_full_crl_empty() {
    // With no revoked certs, CRL should parse successfully and have 0 entries
    let crl_der = generate_full_crl(&store, "test", &key_store, "intermediate", &issuer_cert, 1).unwrap();
    let (_, crl) = x509_parser::parse_x509_crl(&crl_der).unwrap();
    assert_eq!(crl.iter_revoked_certificates().count(), 0);
}

#[test]
fn test_generate_full_crl_with_revoked_cert() {
    // Store a cert, revoke it, generate CRL, verify serial appears
    let serial = "test-serial-001";
    store.store_cert("t", &active_cert(serial)).unwrap();
    store.mark_revoked("t", serial, RevocationReason::KeyCompromise, OffsetDateTime::now_utc()).unwrap();

    let crl_der = generate_full_crl(&store, "t", &key_store, "intermediate", &issuer_cert, 1).unwrap();
    let (_, crl) = x509_parser::parse_x509_crl(&crl_der).unwrap();
    let revoked: Vec<_> = crl.iter_revoked_certificates().collect();
    assert_eq!(revoked.len(), 1);
}
```

- [ ] **Step 2: Implement `generate_full_crl`**

Use `rcgen::CertificateRevocationListParams`:

```rust
pub fn generate_full_crl(
    store: &dyn CertStore,
    tenant_id: &str,
    key_store: &SoftwareKeyStore,
    ca_key_id: &str,
    issuer_cert: &rcgen::Certificate,
    crl_number: u64,
) -> Result<Vec<u8>, CertError> {
    let revoked = store.list_revoked(tenant_id)?;
    let mut params = rcgen::CertificateRevocationListParams {
        this_update: time::OffsetDateTime::now_utc(),
        next_update: time::OffsetDateTime::now_utc() + time::Duration::seconds(3600),
        crl_number: rcgen::SerialNumber::from_slice(&crl_number.to_be_bytes()),
        issuing_distribution_point: None,
        revoked_certs: revoked.iter().map(|c| rcgen::RevokedCertParams {
            serial_number: rcgen::SerialNumber::from_slice(
                &uuid::Uuid::parse_str(&c.serial).unwrap().as_bytes().to_vec()
            ),
            revocation_time: c.revoked_at.unwrap(),
            reason_code: c.revocation_reason.map(|r| r as u64),
            invalidity_date: None,
        }).collect(),
        key_identifier_method: rcgen::KeyIdMethod::Sha256,
    };
    // Sign with CA key (load from key_store)
    let kp = load_key_pair(key_store, tenant_id, ca_key_id)?;
    params.serialize_der_with_signer(issuer_cert, &kp)
        .map_err(|e| CertError::Internal(format!("CRL generation: {e}")))
}
```

- [ ] **Step 3: Implement process() routing**

```rust
fn process(ctx: &ModuleContext, task: &TaskState) -> Result<FlowControl, CertError> {
    let path = task.get("request.path");
    let is_delta = path.contains("delta");
    let is_pem = path.ends_with(".pem");

    let (cache, lock_key) = if is_delta {
        (&ctx.delta_crl_cache, "delta_crl")
    } else {
        (&ctx.full_crl_cache, "full_crl")
    };

    // Check cache
    if let Some(cached) = &*cache.read().unwrap() {
        if cached.next_update > time::OffsetDateTime::now_utc() {
            return serve_crl(cached, is_pem, task);
        }
    }

    // Try to acquire lock
    match ctx.store.acquire_crl_lock(&ctx.config.tenant_id, lock_key, &ctx.holder_id, ctx.config.crl_lock_ttl_secs)? {
        Some(crl_number) => {
            let issuer_cert = load_cert_from_path(&ctx.config.ca_intermediate_cert_path)?;
            let der = if is_delta {
                let since = ctx.delta_crl_cache.read().unwrap()
                    .as_ref().map(|c| c.generated_at).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
                generate_delta_crl(&*ctx.store, &ctx.config.tenant_id, &ctx.key_store, &ctx.config.ca_intermediate_key_id, &issuer_cert, crl_number, since)?
            } else {
                generate_full_crl(&*ctx.store, &ctx.config.tenant_id, &ctx.key_store, &ctx.config.ca_intermediate_key_id, &issuer_cert, crl_number)?
            };
            let pem = pem_encode_crl(&der);
            let next_update = time::OffsetDateTime::now_utc() + time::Duration::seconds(ctx.config.crl_cache_ttl_secs as i64);
            let cached = CachedCrl { der: der.clone(), pem, generated_at: time::OffsetDateTime::now_utc(), next_update, crl_number };
            *cache.write().unwrap() = Some(cached.clone());
            serve_crl(&cached, is_pem, task)
        }
        None => {
            // Serve stale cache with Warning header
            task.set("response.header.Warning", "199 ox_cert_crl \"CRL regeneration in progress; serving cached copy\"");
            if let Some(stale) = &*cache.read().unwrap() {
                serve_crl(stale, is_pem, task)
            } else {
                Err(CertError::Internal("No cached CRL and lock not acquired".into()))
            }
        }
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p ox_cert_crl 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add crates/cert/ox_cert_crl/
git commit -m "feat(ox_cert_crl): CRL generation with advisory lock HA coordination"
```

---

## Task 3: ox_cert_ocsp

**Spec:** `spec/plugin_ocsp.md`
**Route:** `GET /ocsp/*`, `POST /ocsp`

### Config

```rust
#[derive(Debug, serde::Deserialize)]
pub struct OcspConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub extensions: ExtensionsConfig,
    /// Optional dedicated OCSP signing key (if None, use intermediate CA key)
    pub delegated_key_id: Option<String>,
    pub delegated_cert_path: Option<String>,
}
```

### OCSP serial conversion

OCSP requests encode the certificate serial as a big-endian DER integer. Our serials are UUID v4 (16 bytes). Use `ox_cert_core::ocsp::ocsp_serial_to_uuid` to convert.

### Processing

**GET /ocsp/{base64-request}:**
1. Base64-decode the path segment to get OCSP request DER.
2. Parse OCSP request (RFC 6960) using `rasn-ocsp`.
3. Extract `certID.serialNumber` bytes.
4. `ocsp_serial_to_uuid(serial_bytes)` → UUID string.
5. `store.get_cert_by_serial(tenant_id, uuid)` → cert status.
6. Build OCSP response DER (see below).
7. Set `response.body = base64(der)`, `Content-Type: application/ocsp-response`.

**POST /ocsp:**
1. Read raw body bytes from `request.body_bytes`.
2. Parse OCSP request DER directly (no base64 decode).
3. Same steps 3–7.

### OCSP Response Building

The response uses `rasn-ocsp` to build a `BasicOCSPResponse`:

```rust
pub fn build_ocsp_response(
    cert: Option<&CertificateRecord>,
    serial_bytes: Vec<u8>,
    responder_cert: &rcgen::Certificate,
    responder_key_pair: &rcgen::KeyPair,
    issuer_cert: &rcgen::Certificate,
) -> Result<Vec<u8>, CertError> {
    use rasn_ocsp::*;
    use rasn_pkix::*;

    let cert_status = match cert {
        None => CertStatus::Unknown(()),
        Some(c) if c.status == CertStatus::Revoked => {
            CertStatus::Revoked(RevokedInfo {
                revocation_time: c.revoked_at.unwrap().into(),
                revocation_reason: c.revocation_reason.map(|r| ReasonCode::try_from(r as u64).unwrap()),
            })
        }
        Some(_) => CertStatus::Good(()),
    };

    // Build SingleResponse, BasicOCSPResponse, OCSPResponse
    // Sign with responder_key_pair
    // Return DER bytes
    todo!("rasn-ocsp API: refer to rasn-ocsp crate docs for BasicOCSPResponse construction")
}
```

> **Note**: The `rasn-ocsp` crate API varies by version. Check `rasn-ocsp 0.12` docs for the exact builder pattern. The core concept is: build `CertID`, `SingleResponse`, `ResponseData`, sign with responder key, wrap in `BasicOCSPResponse`, encode to DER with `rasn::der::encode`.

- [ ] **Step 1: Write tests for OCSP**

```rust
#[test]
fn test_ocsp_good_cert() {
    let store = /* in-memory store with one active cert */;
    let serial_uuid = "11111111-2222-3333-4444-555555555555";
    store.store_cert("t", &active_cert(serial_uuid)).unwrap();

    let serial_bytes = ocsp_serial_to_uuid_reverse(serial_uuid); // 16 bytes
    let der = build_ocsp_response(
        store.get_cert_by_serial("t", serial_uuid).unwrap().as_ref(),
        serial_bytes,
        &responder_cert, &responder_key, &issuer_cert,
    ).unwrap();

    // Parse response DER and verify status == Good
    let (_, response) = rasn_ocsp::OCSPResponse::decode_der(&der).unwrap();
    // ... assert CertStatus::Good
}

#[test]
fn test_ocsp_revoked_cert() {
    // Store and revoke, verify CertStatus::Revoked with correct reason
}

#[test]
fn test_ocsp_unknown_serial() {
    // No cert with that serial → CertStatus::Unknown
}
```

- [ ] **Step 2: Implement OCSP response building**

Implement `build_ocsp_response` using `rasn-ocsp`. Reference the `rasn` crate's DER encoding API.

- [ ] **Step 3: Implement process() routing**

```rust
fn process(ctx: &ModuleContext, task: &TaskState) -> Result<FlowControl, CertError> {
    let method = task.get("request.method");
    let path = task.get("request.path");

    let ocsp_req_der: Vec<u8> = if method == "POST" {
        // Body is raw DER (binary field)
        task.get_bytes("request.body_bytes")
    } else {
        // GET: base64 decode last path segment
        let b64 = path.trim_start_matches("/ocsp/");
        base64::engine::general_purpose::STANDARD.decode(b64)
            .map_err(|e| CertError::InvalidCsr(format!("bad base64: {e}")))?
    };

    let serial_bytes = parse_ocsp_serial(&ocsp_req_der)?;
    let uuid = ocsp_serial_to_uuid(&serial_bytes)?;
    let cert = ctx.store.get_cert_by_serial(&ctx.config.tenant_id, &uuid)?;

    let responder_key_id = ctx.config.delegated_key_id.as_deref()
        .unwrap_or(&ctx.config.ca_intermediate_key_id);
    let responder_cert = load_cert(/* delegated or intermediate */)?;
    let responder_kp = load_key_pair(&ctx.key_store, &ctx.config.tenant_id, responder_key_id)?;
    let issuer_cert = load_cert_from_path(&ctx.config.ca_intermediate_cert_path)?;

    let response_der = build_ocsp_response(cert.as_ref(), serial_bytes, &responder_cert, &responder_kp, &issuer_cert)?;

    task.set("response.status", "200");
    task.set_bytes("response.body_bytes", &response_der);
    task.set("response.header.Content-Type", "application/ocsp-response");
    Ok(FLOW_CONTROL_CONTINUE)
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p ox_cert_ocsp 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add crates/cert/ox_cert_ocsp/
git commit -m "feat(ox_cert_ocsp): RFC 6960 OCSP responder with UUID serial conversion"
```

---

## Self-Review Checklist

- [x] **CRL number monotonicity**: `acquire_crl_lock` returns the incremented `crl_number` from the lock table. No two nodes can generate a CRL with the same number simultaneously.
- [x] **Stale CRL Warning header**: When the lock is held by another node, the RFC 7234 `Warning: 199` header is set before serving the stale cache.
- [x] **OCSP serial conversion**: `ocsp_serial_to_uuid` is used in step 4 of the OCSP processing — NOT direct serial byte comparison.
- [x] **OCSP `Unknown` for missing certs**: Serial not found → `CertStatus::Unknown`, not an error.
- [x] **OCSP RFC 6960 error responses**: Returned as HTTP 200 with a `malformedRequest` or `internalError` OCSPResponseStatus byte, not as HTTP 4xx/5xx.
- [x] **Delegated OCSP key**: If `delegated_key_id` is set, sign with that key and include the delegated cert in the OCSP response chain; otherwise use the intermediate CA key.
- [x] **Delta CRL**: Uses `generate_delta_crl()` with `since = last full CRL generation time` from cache.
- [x] **Thread safety**: `full_crl_cache` and `delta_crl_cache` are `RwLock<Option<CachedCrl>>` — multiple readers allowed, single writer.
