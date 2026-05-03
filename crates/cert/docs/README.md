# ox_cert — Certificate Authority Server

ox_cert is a modular, multi-tenant, active/active-HA Certificate Authority built as a
family of `ox_webservice` plugins on the `ox_workflow` ABI. Every discrete CA function is
its own plugin, composed into pipelines via standard pipeline YAML. No plugin hardcodes
a dependency on another; communication is through shared `TaskState` fields or the shared
`CertStore` database.

---

## Crate Layout

```
crates/cert/
├── ox_cert_core/          # [LIB] Shared types, KeyStore, CertStore, CertBuilder
├── ox_cert_ca_init/       # [PLUGIN] CA hierarchy init at startup
├── ox_cert_issue/         # [PLUGIN] Issue X.509 certificates
├── ox_cert_renew/         # [PLUGIN] Renew existing certificates
├── ox_cert_revoke/        # [PLUGIN] Revoke certificates
├── ox_cert_ocsp/          # [PLUGIN] OCSP responder
├── ox_cert_crl/           # [PLUGIN] CRL generation and serving
├── ox_cert_acme/          # [PLUGIN] ACME (RFC 8555) server
├── ox_cert_ssh/           # [PLUGIN] SSH Certificate Authority
├── ox_cert_ra/            # [PLUGIN] Registration Authority (approval workflow)
├── ox_cert_webhook/       # [PLUGIN] External authorization and enrichment hooks
├── ox_cert_notify/        # [PLUGIN] Expiration notifications (background)
├── ox_cert_p12/           # [PLUGIN] PKCS#12 export
├── ox_cert_health/        # [PLUGIN] Health and readiness probes
└── ox_cert_admin/         # [PLUGIN] Administrative API
```

---

## Design Principles

1. **One function per plugin.** Each plugin does exactly one thing. `ox_cert_issue` issues
   certs; `ox_cert_revoke` revokes them; `ox_cert_ocsp` answers status queries. There is
   no monolithic CA server — the pipeline is the CA.

2. **Composable via pipeline YAML.** Plugins are wired together in named stages. The
   order of stages determines behavior. No source-code dependency exists between plugins.

3. **Shared state via `ox_cert_core`.** A non-plugin library crate provides every common
   type, crypto helper, storage trait, and CA key-material interface. All plugins link
   against it.

4. **Full multi-tenancy from the start.** Every `KeyStore` and `CertStore` operation
   carries a `tenant_id`. Each tenant has its own CA hierarchy, policy set, and storage
   partition.

5. **Active/active HA.** Multiple CA nodes may serve write traffic simultaneously. Serials
   are UUID v4 (collision-safe by construction). CRL number sequencing uses an advisory
   lock table in the shared database.

6. **Persistence via `ox_data`.** All structured data goes through `GenericDataObject` and
   `DataObjectManager`. No plugin owns a raw connection string outside `CertStoreConfig`.

---

## Multi-Tenancy

Every `KeyStore` and `CertStore` method takes `tenant_id: &str`.

For the software keystore, keys live at `{key_dir}/{tenant_id}/{key_id}.key.pem`.  
For PKCS#11 keystores, the label is `{tenant_id}:{key_id}`.  
Every database row carries `tenant_id` as a column; all queries filter on it.

Tenant lifecycle (create, deactivate, list) is managed via `ox_cert_admin`. Deleting a
tenant marks it inactive — data rows are preserved until a background purge runs.

---

## Pipeline Composition Model

A CA deployment is a `ox_workflow` pipeline YAML that loads cert plugins as modules.
Each module entry specifies a phase, a shared library path, a route pattern (for
request-driven plugins), and a `params` block that becomes the plugin's config JSON.

Phases used by cert plugins:

| Phase | Used by |
|---|---|
| `PreEarlyRequest` | `ox_cert_ca_init` (startup only), `ox_cert_notify` (background) |
| `Content` | All request-handling plugins |

Pipeline evaluation: the host routes an incoming request to the first `Content` plugin
whose `route` pattern matches. Within a stage, plugins receive the same `TaskState` and
may read fields written by upstream plugins. A plugin may terminate the pipeline early
by returning `FLOW_CONTROL_END`.

Key ordering constraint: `ox_cert_webhook` must appear before `ox_cert_issue` in the
pipeline so authorization and enrichment data are in `TaskState` before issuance runs.

---

## Certificate Profiles

Five built-in profiles control validity, key type, extensions, and RA requirements:

| Profile | Validity | Typical Use | RA Required |
|---|---|---|---|
| `short_lived` | 30 s – 60 min | Service mesh mTLS, ephemeral workloads | No |
| `standard` | 1 d – 1 y | Web servers, API endpoints | No |
| `long_lived` | 1 – 10 y | Infrastructure, IoT devices | Yes |
| `ca_intermediate` | 5 – 20 y | Subordinate CA | Yes |
| `ca_root` | 20 – 30 y | Root CA (offline) | Yes |

Each profile carries policy OIDs, CPS URI, name constraints, path length, AIA/CDP URLs,
domain allow/block lists, and wildcard permission.

---

## TaskState Field Namespaces

All `ox_cert_*` plugins use these namespaced conventions for `TaskState` fields:

| Prefix | Owner |
|---|---|
| `request.*` | Host (pre-injected): path, method, body, headers |
| `response.*` | Plugin output: status, body, response headers |
| `cert.ca.*` | `ox_cert_ca_init`: readiness, issuer CN, tenant ID |
| `cert.issued.*` | `ox_cert_issue`: serial, not_after, SCTs |
| `cert.acme.*` | `ox_cert_acme`: order ID, account ID, challenge context |
| `cert.ra.*` | `ox_cert_ra`: request ID, approval flag |
| `cert.ssh.*` | `ox_cert_ssh`: cert type, principals |
| `cert.webhook.*` | `ox_cert_webhook`: authorized flag, enrichment JSON |
| `cert.notify.*` | `ox_cert_notify`: expiring count, last run |
| `cert.health.*` | `ox_cert_health`: status, CA key OK flag |
| `cert.error.*` | Any cert plugin: error code, message |

---

## REST API Conventions

### Response Envelope

```json
{ "data": { ... }, "meta": { "request_id": "uuid", "tenant_id": "acme-corp" } }
```

List responses add `total`, `offset`, `limit` to `meta`.  
Error responses use `"error"` instead of `"data"`:

```json
{ "error": { "code": "POLICY_VIOLATION", "message": "Domain blocked" }, "meta": { ... } }
```

### Standard Error Codes

| Code | HTTP | Meaning |
|---|---|---|
| `INVALID_CSR` | 400 | CSR parse or signature failure |
| `INVALID_REQUEST` | 400 | Malformed request body |
| `POLICY_VIOLATION` | 403 | Issuance policy blocked |
| `NOT_FOUND` | 404 | Certificate or resource not found |
| `ALREADY_REVOKED` | 409 | Certificate already revoked |
| `RA_APPROVAL_REQUIRED` | 202 | Queued for manual approval |
| `CA_NOT_READY` | 503 | CA keys not loaded or HSM unreachable |
| `WEBHOOK_REJECTED` | 403 | Authorization webhook denied |
| `CT_FAILURE` | 502 | CT log submission failed (if `on_failure = block`) |
| `TENANT_NOT_FOUND` | 404 | Unknown tenant ID |
| `INTERNAL_ERROR` | 500 | Unexpected server error |

### Content Negotiation

| `Accept` | Response format |
|---|---|
| `application/json` (default) | JSON envelope |
| `application/pem-certificate-chain` | PEM chain text |
| `application/pkix-cert` | DER binary |
| `application/pkcs7-mime` | PKCS#7 (EST) |
| `application/ocsp-response` | DER OCSP response binary |

---

## Active/Active HA Details

**Serial numbers:** UUID v4 strings. Stored as TEXT. The 16 UUID bytes fit the RFC 5280
≤20-byte serial limit. Collision probability is negligible under any realistic issuance
volume.

**CRL numbers:** Must monotonically increase per RFC 5280 §5.2.3. Coordination uses an
advisory lock table in the shared database exposed through `CertStore::acquire_crl_lock`
and `CertStore::release_crl_lock`. Only the node that holds the lock regenerates the CRL
for a given interval; other nodes serve the cached copy with a `Warning` header.

**SSH serials:** u64 per the OpenSSH wire format. Assigned via an atomic
`UPDATE ... RETURNING` on the `ssh_serial_counter` table — safe under concurrent writers.

---

## Plugin Reference Index

| Plugin | Phase | Purpose |
|---|---|---|
| `ox_cert_core` | library | Shared types, crypto, storage |
| `ox_cert_ca_init` | PreEarlyRequest | CA hierarchy init |
| `ox_cert_issue` | Content | Issue X.509 certs |
| `ox_cert_renew` | Content | Renew certs |
| `ox_cert_revoke` | Content | Revoke certs |
| `ox_cert_ocsp` | Content | OCSP responder |
| `ox_cert_crl` | Content | CRL generation and serving |
| `ox_cert_acme` | Content | ACME server |
| `ox_cert_ssh` | Content | SSH CA |
| `ox_cert_ra` | Content | Approval workflow |
| `ox_cert_webhook` | Content | Authorization and enrichment |
| `ox_cert_notify` | PreEarlyRequest | Expiry notifications |
| `ox_cert_p12` | Content | PKCS#12 export |
| `ox_cert_health` | Content | Health probes |
| `ox_cert_admin` | Content | Admin API |

See individual plugin `docs/README.md` files and `crates/cert/docs/architecture.md` for
deeper detail.
