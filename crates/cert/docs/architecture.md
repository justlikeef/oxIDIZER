# ox_cert Architecture

This document describes how the ox_cert system is structured for plugin developers and
anyone integrating with or extending it.

---

## Plugin Pipeline Model

ox_cert is not a monolithic CA server. It is a set of `cdylib` plugins loaded and composed
by the `ox_workflow` host. The host provides:

- An HTTP listener and routing engine
- A `TaskState` key/value store for per-request context
- A plugin lifecycle (`ox_plugin_init`, `ox_plugin_process`, `ox_plugin_destroy`)
- A `CoreHostApi` struct with function pointers for host services

Each plugin implements three C-ABI functions:

```rust
// Called once at startup; returns an opaque context pointer (or null on failure)
#[no_mangle]
pub extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    api: *const CoreHostApi,
    abi_version: u32,
) -> *mut c_void

// Called for every request that reaches this plugin in the pipeline
#[no_mangle]
pub extern "C" fn ox_plugin_process(
    plugin_ctx: *mut c_void,
    task_ctx: *mut TaskContext,
) -> FlowControl

// Called at shutdown; free plugin_ctx memory
#[no_mangle]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void)
```

`FlowControl` return values:
- `FLOW_CONTROL_CONTINUE` — pass the request to the next stage
- `FLOW_CONTROL_END` — stop the pipeline; `response.*` fields are already set

---

## TaskState Field Namespaces

`TaskState` is a flat string key/value store. All cert plugins use namespaced keys:

| Prefix | Owner | Notable fields |
|---|---|---|
| `request.*` | Host | `request.path`, `request.method`, `request.body`, `request.path.{param}`, `request.query.{param}`, `request.header.{Name}` |
| `response.*` | Plugin output | `response.status`, `response.body`, `response.header.{Name}` |
| `cert.ca.*` | `ox_cert_ca_init` | `cert.ca.ready`, `cert.ca.issuer_cn`, `cert.ca.tenant_id` |
| `cert.issued.*` | `ox_cert_issue` | `cert.issued.serial`, `cert.issued.not_after`, `cert.issued.scts` |
| `cert.acme.*` | `ox_cert_acme` | `cert.acme.order_id`, `cert.acme.account_id`, challenge fields |
| `cert.ra.*` | `ox_cert_ra` | `cert.ra.request_id`, `cert.ra.approved` |
| `cert.ssh.*` | `ox_cert_ssh` | `cert.ssh.cert_type`, `cert.ssh.principals` |
| `cert.webhook.*` | `ox_cert_webhook` | `cert.webhook.authorized`, `cert.webhook.enrichment` |
| `cert.notify.*` | `ox_cert_notify` | `cert.notify.expiring_count`, `cert.notify.last_run` |
| `cert.health.*` | `ox_cert_health` | `cert.health.status`, `cert.health.ca_key_ok` |
| `cert.error.*` | Any cert plugin | `cert.error.code`, `cert.error.message` |

All field values are strings. JSON objects (e.g., `cert.webhook.enrichment`) are stored
as JSON-encoded strings and must be parsed by consuming plugins.

---

## REST Response/Error Conventions

All cert REST endpoints share a common envelope:

```json
// Success (single object)
{ "data": { ... }, "meta": { "request_id": "uuid", "tenant_id": "acme-corp" } }

// Success (list)
{ "data": [...], "meta": { "total": 1234, "offset": 0, "limit": 50, "request_id": "uuid", "tenant_id": "acme-corp" } }

// Error
{ "error": { "code": "POLICY_VIOLATION", "message": "Domain blocked", "details": {} }, "meta": { "request_id": "uuid", "tenant_id": "acme-corp" } }
```

OCSP errors deviate from this: they are always valid DER-encoded OCSP responses with
`responseStatus != successful`, returned with HTTP 200 per RFC 6960 §4.2.1.

ACME errors use RFC 8555 problem documents:
```json
{ "type": "urn:ietf:params:acme:error:badNonce", "detail": "Nonce already used" }
```

---

## Certificate Profiles

Profiles are defined in `ox_cert_core` and carried by `EnrollmentProfile`. Each profile
specifies:

| Field | Controls |
|---|---|
| `validity_seconds` | Default cert lifetime |
| `key_usage` | X.509 Key Usage extension values |
| `extended_key_usage` | X.509 EKU extension values |
| `allowed_key_types` | Set of `KeyType` variants accepted |
| `max_san_count` | Upper bound on SANs |
| `wildcard_allowed` | Whether `*.example.com` SANs are permitted |
| `require_ra_approval` | Whether RA approval is mandatory |
| `policy_oids` | Certificate policy OIDs embedded in issued certs |
| `cps_uri` | Certification Practice Statement URL |
| `name_constraints` | RFC 5280 §4.2.1.10 permitted/excluded name constraints |
| `path_length` | `basicConstraints` path length for CA certs |
| `is_ca` | Whether to set CA=true in `basicConstraints` |

---

## Pipeline YAML Shape

A minimal issuance pipeline looks like:

```yaml
pipeline:
  phases:
    - Content: "ox_pipeline_router"

modules:
  - name: ox_cert_ca_init
    phase: PreEarlyRequest
    lib: libox_cert_ca_init.so
    params:
      tenant_id: acme-corp
      auto_generate: true
      # ... keystore, ca, store, extensions config

  - name: ox_cert_webhook          # must come before issue
    phase: Content
    lib: libox_cert_webhook.so
    params:
      tenant_id: acme-corp
      hooks:
        - name: authorize
          url: "https://cmdb.example.com/api/cert-authorize"
          type: authorize
          secret_env: OX_WEBHOOK_HMAC_SECRET
          on_failure: block

  - name: ox_cert_issue
    phase: Content
    lib: libox_cert_issue.so
    route: "POST /api/v1/certificates"
    params:
      tenant_id: acme-corp
      default_profile: standard
      # ... store, keystore, policy, ct config
```

Plugins without a `route:` key are invoked for every request that reaches their stage.
Plugins with a `route:` key are only invoked when the path and method match.

---

## `ox_cert_core` Library

The library crate that all plugins link against. Key exports:

### KeyStore Trait

Abstraction over software (PKCS#8 PEM) and PKCS#11 (HSM) key material. All signing
operations go through this trait. The concrete implementation is chosen at runtime by
`KeyStoreConfig.store_type`.

Key method signatures:
- `sign(tenant_id, key_id, algorithm, data) -> Result<Vec<u8>, CertError>`
- `public_key(tenant_id, key_id) -> Result<Vec<u8>, CertError>` (DER SPKI)
- `generate_key(tenant_id, key_id, key_type, overwrite) -> Result<(), CertError>`

### CertStore Trait

Persistence abstraction for all certificate data. The concrete implementation
(`OxPersistenceCertStore`) delegates to `ox_data_object_manager`. Every method takes
`tenant_id: &str`.

Each plugin opens its own `CertStore` handle during `ox_plugin_init` and calls
`migrate()` — idempotent, safe to call every startup.

### CertBuilder

Wraps `rcgen` to produce X.509 v3 certificates. Automatically injects:
- AIA (OCSP URL + CA issuer URL)
- CDP (CRL URL)
- SKI (Subject Key Identifier)
- AKI (Authority Key Identifier)
- Policy OIDs and CPS URI

### SshCertBuilder

Builds OpenSSH native binary certificates (not X.509). Uses `ssh-key` crate. Produces
`SshCertRecord` with the full base64 certificate.

### ChainBuilder / Pkcs12Builder

`ChainBuilder` assembles PEM chains from leaf + intermediate + root cert PEM files.  
`Pkcs12Builder` bundles cert + key + chain into a password-protected `.p12` file.

### CT Submission

`ox_cert_core::ct::submit_to_ct_logs()` is called by `ox_cert_issue` at signing time.
This is a library call, not a pipeline stage. The `ox_cert_ct` plugin exists only to
serve SCT query endpoints.

### CoreHostApi Extensions

Two function pointers required on `CoreHostApi` for RA re-submission:
- `publish_to_queue(queue_id, priority, payload, payload_len) -> i32`
- `publish_to_topic(topic, payload, payload_len) -> i32`

The `ox_cert_core::enqueue_task(api, task_id, priority)` helper wraps `publish_to_queue`
for the common case.

---

## Private Key Encryption

Server-generated private keys (for PKCS#12 export) are stored encrypted in the database
column `certificates.private_key_encrypted`.

| Parameter | Value |
|---|---|
| Algorithm | AES-256-GCM |
| Key derivation | HKDF-SHA-256: IKM=`OX_CA_KEY_PASS`, salt=`tenant_id`, info=`"ox_cert:private_key_enc_v1"`, 32 bytes output |
| Nonce | 12 random bytes per key |
| Wire format | `base64(nonce[12] || ciphertext || tag[16])` stored as TEXT |

Only nodes with `OX_CA_KEY_PASS` can decrypt stored keys. P12 export (`ox_cert_p12`)
requires the same passphrase used during issuance.

---

## RA Re-Submission Mechanism

When an RA officer approves a pending request, `ox_cert_ra` does not make an HTTP call.
Instead it uses the workflow task queue:

1. Builds a re-submission request body JSON from the stored `ApprovalRequest`.
2. Creates a new workflow `Task` record with metadata:
   - `cert.ra.approved = "true"`
   - `cert.ra.request_id = <approval request UUID>`
   - `request.body = <re-submission JSON>`
   - `request.method = "POST"`, `request.path = "/api/v1/certificates"`
3. Publishes the task UUID to `tasks.pending` via `CoreHostApi::publish_to_queue`.
4. The workflow scheduler picks up the task and runs the standard issuance pipeline.
5. `ox_cert_issue` reads `cert.ra.approved == "true"` and skips the RA approval check.

---

## Background Plugin Pattern

Plugins that run scheduled work (`ox_cert_notify`, `ox_cert_crl` when
`background_pregenerate = true`) spawn a background thread in `ox_plugin_init`:

```rust
pub struct PluginContext {
    store: Arc<dyn CertStore>,
    config: MyConfig,
    shutdown: Arc<AtomicBool>,
    _handle: Option<JoinHandle<()>>,
}
```

`ox_plugin_process` returns `FLOW_CONTROL_CONTINUE` immediately — the background thread
operates independently of the request pipeline. `ox_plugin_destroy` sets the shutdown
flag and joins the thread.

Background threads use `CertStore` directly; they do not use `CoreHostApi` (which is
per-request context).

---

## Config Parsing Pattern

All cert plugins parse their JSON config in `ox_plugin_init` using the shared helper:

```rust
let config: MyPluginConfig = ox_cert_core::parse_config::<MyPluginConfig>(plugin_config_ctx)?;
```

This handles null pointer checks, `CStr` conversion, JSON deserialization, and structured
error logging. Plugin configs are plain `serde::Deserialize` structs.

---

## Database Schema (GDO Tables)

All data is stored as `GenericDataObject` instances with these logical schemas:

| Table | Primary Key | Key Columns |
|---|---|---|
| `certificate` | `serial` (TEXT, UUID v4) | `tenant_id`, `status`, `not_after`, `enrollment_protocol` |
| `ssh_certificate` | `serial` (BIGINT, u64) | `tenant_id`, `cert_type`, `principals` |
| `ca_key` | `id` (TEXT) | `tenant_id`, `status` (active/retiring/retired) |
| `acme_account` | `id` (TEXT, UUID) | `tenant_id`, `jwk`, `status` |
| `acme_order` | `id` (TEXT, UUID) | `tenant_id`, `account_id`, `status` |
| `acme_authorization` | `id` (TEXT, UUID) | `tenant_id`, `order_id`, `status` |
| `ra_request` | `id` (TEXT, UUID) | `tenant_id`, `status`, `certificate_serial` |
| `audit_log` | `id` (BIGINT, auto) | `tenant_id`, `action`, `serial`, `actor` |
| `scep_challenges` | `id` (TEXT, UUID) | `tenant_id`, `used`, `expires_at` |
| `notification_log` | `id` (BIGINT, auto) | `tenant_id`, `serial`, `threshold_days`, `status` |
| `crl_generation_locks` | composite | `tenant_id`, `lock_key`, `holder_id`, `expires_at` |
| `ssh_serial_counter` | `tenant_id` | `next_serial` (BIGINT) |
| `tenants` | `tenant_id` | `status` (active/inactive) |

Migrations run on every `CertStore::open()` — they are `IF NOT EXISTS` guarded.
