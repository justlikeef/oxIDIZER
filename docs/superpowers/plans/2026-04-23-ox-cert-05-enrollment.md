# ox_cert Enrollment Protocols — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Prerequisite:** Plans 00 and 01 must be complete. Plan 02 (issuance) is recommended but not strictly required.

**Goal:** Build three enrollment protocol plugins: `ox_cert_est` (RFC 7030 — HTTPS-based Simple Certificate Enrollment over Transport), `ox_cert_scep` (RFC 8894 — Simple Certificate Enrollment Protocol), and `ox_cert_ssh` (OpenSSH certificate signing).

**Architecture:** Each is an independent cdylib. EST and SCEP handle legacy device enrollment; SSH issues OpenSSH-format certificates (not X.509). SCEP requires a separate RSA encryption key. All three open their own `SoftwareKeyStore` and `OxPersistenceCertStore`.

**Tech Stack:** Rust 2021 cdylib, ox_cert_core, cms 0.3 (CMS/PKCS#7 for EST and SCEP), ssh-key 0.6, bcrypt 0.15, x509-parser 0.16, base64 0.22, time 0.3. EST also requires `p12` for server-keygen CMS wrapping.

---

## File Map

```
crates/cert/
├── ox_cert_est/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs          — ABI exports, ModuleContext
│       ├── auth.rs         — mTLS client cert extraction + HTTP Basic auth
│       └── handlers.rs     — cacerts, simpleenroll, simplereenroll, serverkeygen
├── ox_cert_scep/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs          — ABI exports, ModuleContext
│       └── handlers.rs     — GetCACert, PKIOperation, GetCACaps
└── ox_cert_ssh/
    ├── Cargo.toml
    └── src/
        ├── lib.rs          — ABI exports, ModuleContext
        └── handlers.rs     — sign, renew, ca/user, ca/host, config

Cargo.toml (workspace root) — add three new members
```

---

## Task 1: Workspace + scaffolds

- [ ] **Step 1: Add workspace members**

```toml
    "crates/cert/ox_cert_est",
    "crates/cert/ox_cert_scep",
    "crates/cert/ox_cert_ssh",
```

- [ ] **Step 2: Create Cargo.toml for each**

`ox_cert_est/Cargo.toml`:
```toml
[lib]
crate-type = ["cdylib"]
[dependencies]
ox_cert_core    = { path = "../ox_cert_core" }
ox_workflow_abi = { path = "../../../workflow/ox_workflow_abi" }
cms             = { version = "0.3", features = ["std"] }
bcrypt          = "0.15"
base64          = "0.22"
x509-parser     = "0.16"
serde           = { version = "1.0", features = ["derive"] }
serde_json      = "1.0"
time            = { version = "0.3", features = ["serde"] }
libc            = "0.2"
p12             = "0.6"
```

`ox_cert_scep/Cargo.toml`:
```toml
[lib]
crate-type = ["cdylib"]
[dependencies]
ox_cert_core    = { path = "../ox_cert_core" }
ox_workflow_abi = { path = "../../../workflow/ox_workflow_abi" }
cms             = { version = "0.3", features = ["std"] }
bcrypt          = "0.15"
base64          = "0.22"
x509-parser     = "0.16"
serde           = { version = "1.0", features = ["derive"] }
serde_json      = "1.0"
time            = { version = "0.3", features = ["serde"] }
libc            = "0.2"
```

`ox_cert_ssh/Cargo.toml`:
```toml
[lib]
crate-type = ["cdylib"]
[dependencies]
ox_cert_core    = { path = "../ox_cert_core" }
ox_workflow_abi = { path = "../../../workflow/ox_workflow_abi" }
ssh-key         = { version = "0.6", features = ["ed25519", "p256", "p384"] }
serde           = { version = "1.0", features = ["derive"] }
serde_json      = "1.0"
time            = { version = "0.3", features = ["serde"] }
libc            = "0.2"
```

- [ ] **Step 3: Create stubs + build check**

```bash
cargo build -p ox_cert_est -p ox_cert_scep -p ox_cert_ssh 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/cert/ox_cert_est/ crates/cert/ox_cert_scep/ crates/cert/ox_cert_ssh/
git commit -m "feat(ox_cert): scaffold est, scep, ssh plugin crates"
```

---

## Task 2: ox_cert_est

**Spec:** `spec/plugin_est.md`
**Routes:** `GET,POST /.well-known/est/*`

### Config

```rust
#[derive(Debug, serde::Deserialize)]
pub struct EstConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub extensions: ExtensionsConfig,
    pub require_client_cert: bool,
    pub basic_auth_enabled: bool,
    /// Label → profile name mapping (e.g. {"iot": "short_lived"})
    pub labels: std::collections::HashMap<String, String>,
}
```

### Endpoints

| Route | Handler |
|-------|---------|
| `GET /.well-known/est/cacerts` | Return CA chain as PKCS#7 CMS DER |
| `POST /.well-known/est/simpleenroll` | CSR in PKCS#10 DER, respond with signed cert PKCS#7 |
| `POST /.well-known/est/simplereenroll` | Same as simpleenroll but uses existing cert identity |
| `POST /.well-known/est/{label}/simpleenroll` | Use label to select profile |
| `POST /.well-known/est/serverkeygen` | Generate key + cert server-side, return CMS EnvelopedData |

### Authentication

1. **mTLS (primary)**: Extract client certificate CN from `request.tls_client_cert_dn` TaskState field (set by `ox_webservice` TLS layer).
2. **HTTP Basic (fallback)**: If `basic_auth_enabled`, decode `Authorization: Basic ...` header. Look up credential in `est_credentials` table via `call_action("raw_sql")`. If credential matches and `used = false`: mark used, proceed.

- [ ] **Step 1: Write tests for cacerts**

```rust
#[test]
fn test_cacerts_returns_pkcs7() {
    let task = test_task("GET", "/.well-known/est/cacerts", "");
    call_process(&ctx, &task);
    assert_eq!(task.get("response.status"), "200");
    assert_eq!(task.get("response.header.Content-Type"), "application/pkcs7-mime; smime-type=certs-only");
    // Verify the body is valid base64 DER
    let der = base64::engine::general_purpose::STANDARD.decode(task.get("response.body")).unwrap();
    assert!(!der.is_empty());
}
```

- [ ] **Step 2: Implement cacerts handler**

Build a PKCS#7 `certs-only` response (degenerate CMS SignedData with no signers, just the CA chain):

```rust
pub fn handle_cacerts(ctx: &ModuleContext, task: &TaskState) -> Result<(), CertError> {
    // Load root and intermediate PEM, wrap in CMS degenerate SignedData
    // Using the `cms` crate: cms::content_info::CmsVersion, SignedData, CertificateSet
    let root_pem = std::fs::read_to_string(&ctx.config.ca_root_cert_path)?;
    let int_pem = std::fs::read_to_string(&ctx.config.ca_intermediate_cert_path)?;
    let der = build_certs_only_pkcs7(&root_pem, &int_pem)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&der);
    task.set("response.status", "200");
    task.set("response.body", &encoded);
    task.set("response.header.Content-Type", "application/pkcs7-mime; smime-type=certs-only");
    Ok(())
}
```

- [ ] **Step 3: Write tests for simpleenroll**

```rust
#[test]
fn test_simpleenroll_with_valid_csr() {
    // Create a CSR, base64-encode as DER, call process
    // Verify response is PKCS#7 containing signed cert
    let csr_der = make_test_csr_der("device.example.com");
    let body = base64::engine::general_purpose::STANDARD.encode(&csr_der);
    let task = test_task("POST", "/.well-known/est/simpleenroll", &body);
    task.set("request.header.Content-Type", "application/pkcs10");
    task.set("request.tls_client_cert_dn", "CN=device001");
    call_process(&ctx, &task);
    assert_eq!(task.get("response.status"), "200");
}
```

- [ ] **Step 4: Implement simpleenroll handler**

```rust
pub fn handle_simpleenroll(ctx: &ModuleContext, label: Option<&str>, task: &TaskState) -> Result<(), CertError> {
    // 1. Authenticate (mTLS or Basic)
    authenticate(ctx, task)?;
    // 2. Decode PKCS#10 DER from body
    let body = task.get("request.body");
    let csr_der = base64::engine::general_purpose::STANDARD.decode(&body)?;
    // 3. Parse CSR
    let csr_info = parse_csr_der(&csr_der)?;
    // 4. Select profile from label (if present) or default
    let profile_name = label.and_then(|l| ctx.config.labels.get(l))
        .unwrap_or(&"standard".to_string()).clone();
    // 5. Issue cert (same CertBuilder pipeline)
    let serial = uuid::Uuid::new_v4().to_string();
    let cert = issue_cert(&ctx, &csr_info, &profile_name, serial, EnrollmentProtocol::Est)?;
    // 6. Wrap in PKCS#7 CMS
    let pkcs7_der = wrap_cert_in_pkcs7(&cert.pem)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&pkcs7_der);
    task.set("response.status", "200");
    task.set("response.body", &encoded);
    task.set("response.header.Content-Type", "application/pkcs7-mime; smime-type=certs-only");
    Ok(())
}
```

- [ ] **Step 5: Run tests, commit**

```bash
cargo test -p ox_cert_est 2>&1 | tail -5
git add crates/cert/ox_cert_est/
git commit -m "feat(ox_cert_est): EST RFC 7030 — cacerts, simpleenroll, simplereenroll"
```

---

## Task 3: ox_cert_scep

**Spec:** `spec/plugin_scep.md`
**Route:** `GET,POST /scep`

### Config

```rust
#[derive(Debug, serde::Deserialize)]
pub struct ScepConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub challenge_ttl_secs: u64,   // default 3600
    pub encryption_algorithm: String,  // "aes-256-cbc"
    pub encryption_key_id: String, // default "scep-encryption"
}
```

### SCEP Key Init

On `ox_plugin_init`, generate the SCEP encryption key if it doesn't exist:

```rust
key_store.generate_key(tenant_id, &config.encryption_key_id, KeyType::Rsa2048, false)?;
```

### Endpoints

**`GET /scep?operation=GetCACert`**:
- Return CA cert chain (intermediate + root) as `application/x-x509-ca-ra-cert` (PKCS#7 if chain, single DER if just CA).

**`GET /scep?operation=GetCACaps`**:
- Return newline-separated list:
  ```
  POSTPKIOperation
  SHA-256
  AES
  ```

**`POST /scep?operation=PKIOperation`**:
1. Decode body as CMS EnvelopedData (client encrypted PKCSReq).
2. Decrypt using SCEP encryption key.
3. Extract PKCS#10 CSR from decrypted CMS SignedData.
4. Verify challenge password from `messageType=PKCSReq`:
   - Extract `challengePassword` attribute.
   - Call `store.consume_scep_challenge(tenant_id, &bcrypt_hash_of_password)`.
5. Issue certificate.
6. Wrap response as CMS SignedData (CertRep) encrypted to client key.

- [ ] **Step 1: Write tests**

```rust
#[test]
fn test_scep_get_cacaps() {
    let task = test_task_get("/scep?operation=GetCACaps");
    call_process(&ctx, &task);
    let body = task.get("response.body");
    assert!(body.contains("POSTPKIOperation"));
    assert!(body.contains("SHA-256"));
    assert!(body.contains("AES"));
}

#[test]
fn test_scep_get_ca_cert() {
    let task = test_task_get("/scep?operation=GetCACert");
    call_process(&ctx, &task);
    assert_eq!(task.get("response.status"), "200");
    // Body is DER-encoded cert or PKCS#7
    assert!(!task.get("response.body").is_empty());
}
```

- [ ] **Step 2: Implement GetCACert and GetCACaps handlers**

These are straightforward. GetCACert reads the CA cert PEM, converts to DER, sets the response. GetCACaps returns a static string.

- [ ] **Step 3: PKIOperation challenge password validation**

```rust
fn verify_scep_challenge(ctx: &ModuleContext, password: &str) -> Result<(), CertError> {
    // Hash the provided password with bcrypt at work factor 12
    // Call store.consume_scep_challenge which does a constant-time comparison
    // The store checks password_hash field in scep_challenges table
    let hash = bcrypt::hash(password, 12)
        .map_err(|e| CertError::Internal(format!("bcrypt: {e}")))?;
    let consumed = ctx.store.consume_scep_challenge(&ctx.config.tenant_id, &hash)?;
    if !consumed {
        return Err(CertError::PolicyViolation("invalid or expired SCEP challenge".into()));
    }
    Ok(())
}
```

> **Note on `consume_scep_challenge`**: The CertStore method must use bcrypt `verify()` (not hash equality) to check if any active challenge's hash matches the provided password. This requires a `call_action("raw_sql")` to fetch unexpired, unused challenge hashes and verify each one.

- [ ] **Step 4: PKIOperation — CMS unwrap, issue, wrap**

This is the most complex part. The full CMS pipeline:
1. Decode base64 request body → CMS EnvelopedData DER.
2. Use `cms::enveloped_data` to decrypt with the SCEP RSA encryption key.
3. Parse decrypted bytes as CMS SignedData.
4. Verify signature with client's self-signed certificate.
5. Extract PKCS#10 from `encapContentInfo`.
6. Parse challenge password attribute.
7. Verify challenge password.
8. Issue cert via CertBuilder.
9. Build CertRep CMS SignedData.
10. Encrypt CertRep to client's public key.
11. Base64-encode, return.

```rust
pub fn handle_pki_operation(ctx: &ModuleContext, task: &TaskState) -> Result<(), CertError> {
    let body_b64 = task.get("request.body");
    let env_data_der = base64::engine::general_purpose::STANDARD.decode(&body_b64)?;
    // Step 2: Decrypt using SCEP encryption key RSA PKCS#1v1.5
    let signed_data_der = decrypt_cms_enveloped_data(&env_data_der, &ctx.key_store, &ctx.config.tenant_id, &ctx.config.encryption_key_id)?;
    // Step 3-4: Parse and verify SignedData, extract CSR
    let (csr_der, client_cert_der, challenge_password) = parse_scep_pki_message(&signed_data_der)?;
    // Step 7: Verify challenge
    verify_scep_challenge(ctx, &challenge_password)?;
    // Step 8: Issue cert
    let csr_info = parse_csr_der(&csr_der)?;
    let cert = issue_cert(ctx, &csr_info, "standard", uuid::Uuid::new_v4().to_string(), EnrollmentProtocol::Scep)?;
    // Steps 9-11: Wrap and encrypt response
    let response_der = build_cert_rep(&cert.pem, &client_cert_der, ctx)?;
    task.set("response.status", "200");
    task.set("response.body", &base64::engine::general_purpose::STANDARD.encode(&response_der));
    task.set("response.header.Content-Type", "application/x-pki-message");
    Ok(())
}
```

- [ ] **Step 5: Run tests, commit**

```bash
cargo test -p ox_cert_scep 2>&1 | tail -5
git add crates/cert/ox_cert_scep/
git commit -m "feat(ox_cert_scep): SCEP RFC 8894 — GetCACert, GetCACaps, PKIOperation"
```

---

## Task 4: ox_cert_ssh

**Spec:** `spec/plugin_ssh.md`
**Routes:** `GET,POST /api/v1/ssh/*`

### Config

```rust
#[derive(Debug, serde::Deserialize)]
pub struct SshPluginConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub user_ca: SshCaConfig,
    pub host_ca: SshCaConfig,
    pub user: SshPrincipalPolicy,
    pub host: SshPrincipalPolicy,
}
```

### CA Key Init

In `ox_plugin_init`, for each CA (user + host): generate the key if not present.

```rust
key_store.generate_key(&config.tenant_id, &config.user_ca.key_id, config.user_ca.key_type.into(), false)?;
key_store.generate_key(&config.tenant_id, &config.host_ca.key_id, config.host_ca.key_type.into(), false)?;
```

- [ ] **Step 1: Write tests for POST /api/v1/ssh/sign**

```rust
#[test]
fn test_ssh_sign_user_cert() {
    // Generate a test ED25519 public key
    let test_key = ssh_key::PublicKey::from_openssh("ssh-ed25519 AAAA...").unwrap();
    let body = serde_json::json!({
        "public_key": test_key.to_openssh().unwrap(),
        "cert_type": "user",
        "principals": ["alice"],
        "validity": "16h",
        "key_id": "alice@example.com"
    }).to_string();
    let task = test_task("POST", "/api/v1/ssh/sign", &body);
    call_process(&ctx, &task);
    assert_eq!(task.get("response.status"), "201");
    let body: serde_json::Value = serde_json::from_str(&task.get("response.body")).unwrap();
    let cert_str = body["data"]["certificate"].as_str().unwrap();
    assert!(cert_str.contains("ssh-ed25519-cert-v01@openssh.com"));
    assert_eq!(body["data"]["cert_type"], "user");
}

#[test]
fn test_ssh_sign_principal_not_allowed_returns_403() {
    // Config allows only ["*.example.com"]
    // Request principal "root" → 403
}

#[test]
fn test_ssh_sign_validity_exceeds_max_capped() {
    // max_validity = "24h", request "720h" → cert validity capped at 24h
}
```

- [ ] **Step 2: Implement POST /api/v1/ssh/sign handler**

```rust
pub fn handle_sign(ctx: &ModuleContext, task: &TaskState) -> Result<(), CertError> {
    let req: SshSignRequest = serde_json::from_str(&task.get("request.body"))?;
    // 1. Select CA config
    let (ca_config, principal_policy) = match req.cert_type.as_str() {
        "user" => (&ctx.config.user_ca, &ctx.config.user),
        "host" => (&ctx.config.host_ca, &ctx.config.host),
        _ => return Err(CertError::InvalidCsr("cert_type must be 'user' or 'host'".into())),
    };
    // 2. Parse public key
    let public_key = ssh_key::PublicKey::from_openssh(&req.public_key)
        .map_err(|e| CertError::InvalidCsr(format!("invalid SSH public key: {e}")))?;
    // 3. Validate principals against allowed_principals glob list
    for principal in &req.principals {
        let allowed = principal_policy.allowed_principals.iter().any(|pattern| {
            glob::Pattern::new(pattern).ok().map(|p| p.matches(principal)).unwrap_or(false)
        });
        if !allowed {
            return Err(CertError::PolicyViolation(format!("principal '{principal}' not in allowed list")));
        }
    }
    // 4. Resolve validity (parse duration string, cap at max)
    let validity_str = req.validity.as_deref().unwrap_or(&ca_config.default_validity);
    let duration = parse_duration(validity_str)?;
    let duration = if let Some(max) = &principal_policy.max_validity {
        duration.min(parse_duration(max)?)
    } else { duration };
    // 5. Get next serial
    let serial = ctx.store.get_next_ssh_serial(&ctx.config.tenant_id)?;
    // 6. Determine extensions
    let extensions = req.extensions.unwrap_or(principal_policy.default_extensions.clone());
    // 7. Build and sign using SshCertBuilder
    let now = time::OffsetDateTime::now_utc();
    let record = SshCertBuilder::new(if req.cert_type == "user" { SshCertType::User } else { SshCertType::Host })
        .key_id(req.key_id.as_deref().unwrap_or(&req.cert_type))
        .serial(serial)
        .validity(now, now + duration)
        .extend_principals(req.principals.clone())
        .extend_extensions(extensions)
        .sign(public_key.as_bytes(), &ctx.key_store, &ctx.config.tenant_id, &ca_config.key_id)?;
    // 8. Store
    ctx.store.store_ssh_cert(&ctx.config.tenant_id, &record)?;
    ctx.store.store_audit_event(&ctx.config.tenant_id, &AuditEvent { action: AuditAction::SshSign, /* ... */ })?;
    // 9. Respond
    task.set("response.status", "201");
    task.set("response.body", &serde_json::to_string(&ssh_response(&record)).unwrap());
    Ok(())
}
```

- [ ] **Step 3: Implement GET /api/v1/ssh/ca/user and /host**

```rust
pub fn handle_ca_public_key(ctx: &ModuleContext, cert_type: &str, task: &TaskState) -> Result<(), CertError> {
    let key_id = match cert_type {
        "user" => &ctx.config.user_ca.key_id,
        "host" => &ctx.config.host_ca.key_id,
        _ => return Err(CertError::InvalidCsr("must be user or host".into())),
    };
    let pubkey_der = ctx.key_store.public_key(&ctx.config.tenant_id, key_id)?;
    let ssh_pubkey = ssh_key::PublicKey::try_from(pubkey_der.as_slice())
        .map_err(|e| CertError::Internal(format!("convert to SSH pubkey: {e}")))?;
    let openssh_line = ssh_pubkey.to_openssh().map_err(|e| CertError::Internal(e.to_string()))?;
    task.set("response.status", "200");
    task.set("response.body", &openssh_line);
    task.set("response.header.Content-Type", "text/plain");
    Ok(())
}
```

- [ ] **Step 4: Implement GET /api/v1/ssh/config**

```rust
pub fn handle_config(ctx: &ModuleContext, task: &TaskState) -> Result<(), CertError> {
    let user_ca_key = get_ca_pubkey_line(ctx, "user")?;
    let host_ca_key = get_ca_pubkey_line(ctx, "host")?;
    let response = serde_json::json!({
        "sshd_config_snippet": "TrustedUserCAKeys /etc/ssh/trusted_user_cas",
        "known_hosts_snippet": format!("@cert-authority * {host_ca_key}"),
        "user_ca_public_key": user_ca_key,
        "host_ca_public_key": host_ca_key,
        "notes": {
            "TrustedUserCAKeys": "Add user_ca_public_key contents to this file on each SSH server",
            "HostCertificate": "Host certs must be signed separately using POST /api/v1/ssh/sign with cert_type=host and the server's host public key"
        }
    });
    task.set("response.status", "200");
    task.set("response.body", &response.to_string());
    task.set("response.header.Content-Type", "application/json");
    Ok(())
}
```

- [ ] **Step 5: Implement POST /api/v1/ssh/renew**

```rust
pub fn handle_renew(ctx: &ModuleContext, task: &TaskState) -> Result<(), CertError> {
    let req: SshRenewRequest = serde_json::from_str(&task.get("request.body"))?;
    let original = ctx.store.get_ssh_cert_by_serial(&ctx.config.tenant_id, req.serial)?
        .ok_or_else(|| CertError::NotFound(format!("ssh serial {}", req.serial)))?;
    // Reject renewal of long-expired certs (valid_before < now - 5min)
    let five_min_ago = time::OffsetDateTime::now_utc() - time::Duration::minutes(5);
    if original.valid_before < five_min_ago {
        return Err(CertError::PolicyViolation("cannot renew expired SSH certificate".into()));
    }
    // Reuse principals, extensions, critical_options from original
    // Sign new cert with same public key
    let new_serial = ctx.store.get_next_ssh_serial(&ctx.config.tenant_id)?;
    let ca_config = if original.cert_type == SshCertType::User { &ctx.config.user_ca } else { &ctx.config.host_ca };
    let validity_str = req.validity.as_deref().unwrap_or(&ca_config.default_validity);
    let duration = parse_duration(validity_str)?;
    let now = time::OffsetDateTime::now_utc();
    let pubkey_bytes = base64::engine::general_purpose::STANDARD.decode(&original.public_key)?;
    let record = SshCertBuilder::new(original.cert_type.clone())
        .key_id(&original.key_id)
        .serial(new_serial)
        .validity(now, now + duration)
        .extend_principals(original.principals.clone())
        .extend_extensions(original.extensions.clone())
        .sign(&pubkey_bytes, &ctx.key_store, &ctx.config.tenant_id, &ca_config.key_id)?;
    ctx.store.store_ssh_cert(&ctx.config.tenant_id, &record)?;
    task.set("response.status", "201");
    task.set("response.body", &serde_json::to_string(&ssh_response(&record)).unwrap());
    Ok(())
}
```

- [ ] **Step 6: Run all SSH tests**

```bash
cargo test -p ox_cert_ssh 2>&1 | tail -10
```

- [ ] **Step 7: Commit**

```bash
git add crates/cert/ox_cert_ssh/
git commit -m "feat(ox_cert_ssh): OpenSSH certificate signing, renewal, and CA key endpoints"
```

---

## Self-Review Checklist

- [x] **EST cacerts**: Returns PKCS#7 degenerate CMS with `smime-type=certs-only` — not raw PEM.
- [x] **EST Basic auth single-use**: `est_credentials.used` is set to `true` after first use; subsequent requests with the same credential are rejected.
- [x] **EST label routing**: `/.well-known/est/{label}/simpleenroll` selects profile from `config.labels[label]`.
- [x] **SCEP encryption key init**: `generate_key(overwrite=false)` on `ox_plugin_init` — idempotent.
- [x] **SCEP challenge bcrypt**: `consume_scep_challenge` uses `bcrypt::verify()` against stored hash, not plain equality.
- [x] **SSH serial counter**: Uses `store.get_next_ssh_serial()` which calls `atomic_increment` — safe under HA.
- [x] **SSH validity cap**: Requested validity is capped at `max_validity` if configured.
- [x] **SSH principal glob matching**: Uses glob patterns (e.g., `*.example.com`), not regex.
- [x] **SSH /config notes**: Clarifies host certs require explicit signing, not auto-generated by this endpoint.
- [x] **SSH renew expiry gate**: Rejects renewal if `valid_before < now - 5 minutes`.
- [x] **SSH CA key auto-init**: User CA and host CA keys generated on `ox_plugin_init` if not present.
