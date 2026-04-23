# ox_cert — Certificate Authority Server

A modular, multi-tenant, active/active-HA Certificate Authority built as a family of
`ox_webservice` plugins on the `ox_workflow` ABI. Each discrete CA function is its own
plugin, composed into workflows via standard pipeline YAML.

## How to Read This Spec

This file is the **index and overview**. It covers design principles, crate layout,
global conventions (TaskState fields, REST standards, certificate profiles), the pipeline
YAML example, and all resolved design decisions.

**Detailed specifications live under `spec/`:**

| File | Contents |
|---|---|
| [spec/core.md](spec/core.md) | `ox_cert_core`: all Rust type/struct definitions, `KeyStore` and `CertStore` trait signatures, `CertBuilder`, `SshCertBuilder`, `IssuancePolicy`, `CertError`, `ox_persistence` integration, private key encryption, `CoreHostApi` extensions, migration strategy, and the full storage schema |
| [spec/plugin_*.md](spec/) | One file per plugin: routes, config struct, processing steps, TaskState fields, error cases, crate dependencies |

---

## Design Principles

1. **One function per plugin** — each plugin does exactly one thing and communicates via
   `TaskState` fields or the shared `CertStore`.
2. **Composable via pipeline YAML** — plugins are wired together in stages; no hard-coded
   inter-plugin dependencies.
3. **Shared state via `ox_cert_core`** — a non-plugin library crate providing all common
   types, crypto helpers, storage traits, and the CA key-material interface.
4. **Full multi-tenancy** — every `KeyStore` and `CertStore` operation carries a
   `tenant_id`. Each tenant has its own CA hierarchy, policy set, and storage partition.
5. **Active/active HA** — multiple CA nodes may serve write traffic simultaneously.
   Serials are UUID v4 (collision-safe). CRL number sequencing uses an advisory lock table.
6. **Persistence via `ox_persistence`** — all structured data goes through the
   `PersistenceDriver` abstraction. No plugin owns a raw database connection string
   outside the `CertStoreConfig`.

---

## Crate Layout

```
crates/cert/
├── CERTSERVERSPEC.md                          # This file — overview and index
├── spec/
│   ├── core.md                                # ox_cert_core: all types, traits, storage
│   ├── plugin_ca_init.md
│   ├── plugin_issue.md
│   ├── plugin_renew.md
│   ├── plugin_revoke.md
│   ├── plugin_ocsp.md
│   ├── plugin_crl.md
│   ├── plugin_acme.md
│   ├── plugin_est.md
│   ├── plugin_scep.md
│   ├── plugin_ssh.md
│   ├── plugin_ra.md
│   ├── plugin_webhook.md
│   ├── plugin_notify.md
│   ├── plugin_p12.md
│   ├── plugin_health.md
│   ├── plugin_admin.md
│   ├── plugin_ct.md
│   └── plugin_ad_autoenroll.md
│
├── ox_cert_core/                              # [LIB] See spec/core.md
├── ox_cert_ca_init/                           # [PLUGIN] See spec/plugin_ca_init.md
├── ox_cert_issue/                             # [PLUGIN] See spec/plugin_issue.md
├── ox_cert_renew/                             # [PLUGIN] See spec/plugin_renew.md
├── ox_cert_revoke/                            # [PLUGIN] See spec/plugin_revoke.md
├── ox_cert_ocsp/                              # [PLUGIN] See spec/plugin_ocsp.md
├── ox_cert_crl/                               # [PLUGIN] See spec/plugin_crl.md
├── ox_cert_acme/                              # [PLUGIN] See spec/plugin_acme.md
├── ox_cert_acme_challenge_http/               # [PLUGIN] (spec in plugin_acme.md)
├── ox_cert_acme_challenge_dns/                # [PLUGIN] (spec in plugin_acme.md)
├── ox_cert_est/                               # [PLUGIN] See spec/plugin_est.md
├── ox_cert_scep/                              # [PLUGIN] See spec/plugin_scep.md
├── ox_cert_ad_autoenroll/                     # [PLUGIN] See spec/plugin_ad_autoenroll.md
├── ox_cert_ssh/                               # [PLUGIN] See spec/plugin_ssh.md
├── ox_cert_ra/                                # [PLUGIN] See spec/plugin_ra.md
├── ox_cert_webhook/                           # [PLUGIN] See spec/plugin_webhook.md
├── ox_cert_notify/                            # [PLUGIN] See spec/plugin_notify.md
├── ox_cert_p12/                               # [PLUGIN] See spec/plugin_p12.md
├── ox_cert_health/                            # [PLUGIN] See spec/plugin_health.md
├── ox_cert_admin/                             # [PLUGIN] See spec/plugin_admin.md
└── ox_cert_ct/                                # [PLUGIN] See spec/plugin_ct.md
```

---

## Global Conventions

### TaskState Field Namespaces

All `ox_cert_*` plugins use the following namespaced field conventions:

| Prefix | Owner | Example |
|---|---|---|
| `request.*` | Host (pre-injected) | `request.path`, `request.method`, `request.body` |
| `response.*` | Plugin (output) | `response.status`, `response.body`, `response.header.Content-Type` |
| `cert.ca.*` | `ox_cert_ca_init` | `cert.ca.ready`, `cert.ca.issuer_cn`, `cert.ca.tenant_id` |
| `cert.issued.*` | `ox_cert_issue` | `cert.issued.serial`, `cert.issued.not_after`, `cert.issued.scts` |
| `cert.acme.*` | `ox_cert_acme` | `cert.acme.order_id`, `cert.acme.account_id` |
| `cert.ra.*` | `ox_cert_ra` | `cert.ra.request_id`, `cert.ra.approved` |
| `cert.ssh.*` | `ox_cert_ssh` | `cert.ssh.cert_type`, `cert.ssh.principals` |
| `cert.webhook.*` | `ox_cert_webhook` | `cert.webhook.authorized`, `cert.webhook.enrichment` |
| `cert.notify.*` | `ox_cert_notify` | `cert.notify.expiring_count`, `cert.notify.last_run` |
| `cert.health.*` | `ox_cert_health` | `cert.health.status`, `cert.health.ca_key_ok` |
| `cert.error.*` | Any cert plugin | `cert.error.code`, `cert.error.message` |

### REST API Standards

#### Response Envelope

```json
{ "data": { ... },  "meta": { "request_id": "uuid", "tenant_id": "acme-corp" } }
```
List:
```json
{ "data": [...], "meta": { "total": 1234, "offset": 0, "limit": 50, "request_id": "uuid", "tenant_id": "acme-corp" } }
```
Error:
```json
{ "error": { "code": "POLICY_VIOLATION", "message": "Domain blocked", "details": {} }, "meta": { "request_id": "uuid", "tenant_id": "acme-corp" } }
```

#### Error Codes

| Code | HTTP | Meaning |
|---|---|---|
| `INVALID_CSR` | 400 | CSR parsing or signature verification failed |
| `INVALID_REQUEST` | 400 | Malformed request body |
| `POLICY_VIOLATION` | 403 | Issuance policy blocked the request |
| `NOT_FOUND` | 404 | Certificate/resource not found |
| `ALREADY_REVOKED` | 409 | Certificate already revoked |
| `RA_APPROVAL_REQUIRED` | 202 | Queued for RA approval |
| `CA_NOT_READY` | 503 | CA keys not loaded / HSM unreachable |
| `WEBHOOK_REJECTED` | 403 | Authorization webhook denied |
| `CT_FAILURE` | 502 | CT log submission failed (if `on_failure = block`) |
| `TENANT_NOT_FOUND` | 404 | Tenant ID unknown |
| `INTERNAL_ERROR` | 500 | Unexpected server error |

#### Pagination Query Parameters

- `offset` — starting position (default: `0`)
- `limit` — max results per page (default: `50`, max: `1000`)
- `sort` — field name (default: `created_at`)
- `order` — `asc` or `desc` (default: `desc`)

#### Content Negotiation

| Accept Header | Response Format |
|---|---|
| `application/json` (default) | JSON response envelope |
| `application/pem-certificate-chain` | PEM chain text |
| `application/pkix-cert` | DER certificate binary |
| `application/pkcs7-mime` | PKCS#7 (EST responses) |
| `application/ocsp-response` | DER OCSP response binary |

---

## Certificate Profiles

| Profile | Validity | Typical Use | Key Types | Key Usage | Max SANs | Wildcards | RA Required |
|---|---|---|---|---|---|---|---|
| `short_lived` | 30 s – 60 min | Service mesh mTLS, ephemeral workloads | ECC P-256 preferred | Digital Signature | 10 | No | No |
| `standard` | 1 d – 1 y | Web servers, API endpoints | RSA-2048+, ECC P-256/P-384 | Digital Signature, Key Encipherment | 100 | Yes | No |
| `long_lived` | 1 – 10 y | Infrastructure, IoT devices | RSA-4096, ECC P-384/P-521 | Digital Signature, Key Encipherment | 50 | Configurable | Yes |
| `ca_intermediate` | 5 – 20 y | Subordinate CA | RSA-4096, ECC P-384 | Certificate Sign, CRL Sign | N/A | N/A | Yes |
| `ca_root` | 20 – 30 y | Root CA (offline) | RSA-4096, ECC P-521 | Certificate Sign, CRL Sign | N/A | N/A | Yes |

Each profile carries: policy OIDs, CPS URI, name constraints, path length, AIA URLs, CDP URLs, domain allowlist/blocklist.

---

## Pipeline Configuration Example

```yaml
pipeline:
  phases:
    - Content: "ox_pipeline_router"

modules:
  # --- Init / Background ---
  - name: ox_cert_ca_init
    phase: PreEarlyRequest
    lib: libox_cert_ca_init.so
    params:
      tenant_id: acme-corp
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      ca.root.key_path: /etc/ox_cert/keys/acme-corp/root.key
      ca.root.cert_path: /etc/ox_cert/ca/acme-corp/root.crt
      ca.root.key_type: ecc-p384
      ca.root.validity_years: 25
      ca.root.subject: "CN=ACME Root CA,O=ACME Corp,C=US"
      ca.intermediate.key_path: /etc/ox_cert/keys/acme-corp/intermediate.key
      ca.intermediate.cert_path: /etc/ox_cert/ca/acme-corp/intermediate.crt
      ca.intermediate.key_type: ecc-p384
      ca.intermediate.validity_years: 10
      ca.intermediate.subject: "CN=ACME Intermediate CA,O=ACME Corp,C=US"
      ca.intermediate.name_constraints.permitted_dns: [".example.com", ".internal.local"]
      ca.intermediate.path_length: 0
      auto_generate: true
      extensions.aia.ocsp_url: "http://ocsp.example.com/ocsp"
      extensions.aia.ca_issuer_url: "http://pki.example.com/ca/acme-corp/intermediate.crt"
      extensions.cdp.url: "http://pki.example.com/crl/acme-corp"
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"

  - name: ox_cert_notify
    phase: PreEarlyRequest
    lib: libox_cert_notify.so
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      notify.schedule: "0 8 * * *"
      notify.thresholds_days: [90, 60, 30, 14, 7, 1]
      notify.include_ca_certs: true
      notify.channels:
        - type: webhook
          url: "https://hooks.example.com/cert-expiry"
          secret_env: OX_NOTIFY_WEBHOOK_SECRET
        - type: mqtt
          topic: "ox/cert/expiring"

  # --- Webhook enrichment / authorization (before issue) ---
  - name: ox_cert_webhook
    phase: Content
    lib: libox_cert_webhook.so
    params:
      tenant_id: acme-corp
      webhook.hooks:
        - name: inventory-check
          url: "https://cmdb.example.com/api/cert-authorize"
          type: authorize
          secret_env: OX_WEBHOOK_HMAC_SECRET
          timeout: "5s"
          on_failure: block
        - name: enrich-ou
          url: "https://cmdb.example.com/api/cert-enrich"
          type: enrich
          secret_env: OX_WEBHOOK_HMAC_SECRET
          timeout: "3s"
          on_failure: allow

  # --- Enrollment ---
  - name: ox_cert_issue
    phase: Content
    lib: libox_cert_issue.so
    route: "POST /api/v1/certificates"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys
      default_profile: standard
      policy.domain_allowlist: [".*\\.example\\.com$", ".*\\.internal\\.local$"]
      policy.domain_blocklist: [".*\\.test$"]
      policy.wildcard_allowed: true
      policy.min_rsa_bits: 2048
      ct.enabled: true
      ct.min_scts: 2
      ct.on_failure: warn
      ct.logs:
        - name: "Google Argon"
          url: "https://ct.googleapis.com/logs/argon2025h1"
          public_key_b64: "<base64-encoded-key>"

  - name: ox_cert_renew
    phase: Content
    lib: libox_cert_renew.so
    route: "POST /api/v1/certificates/*/renew"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys
      auto_revoke_on_renew: true

  - name: ox_cert_revoke
    phase: Content
    lib: libox_cert_revoke.so
    route: "POST /api/v1/certificates/*/revoke"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"

  # --- ACME ---
  - name: ox_cert_acme
    phase: Content
    lib: libox_cert_acme.so
    route: "GET,HEAD,POST /acme/*"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys
      acme.tos_url: "https://example.com/tos"
      acme.external_account_required: false
      acme.rate_limit.orders_per_account_per_hour: 50
      acme.rate_limit.certs_per_domain_per_week: 5

  - name: ox_cert_acme_challenge_http
    phase: Content
    lib: libox_cert_acme_challenge_http.so
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      acme.http01.timeout: "10s"
      acme.http01.retries: 3

  - name: ox_cert_acme_challenge_dns
    phase: Content
    lib: libox_cert_acme_challenge_dns.so
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      acme.dns01.resolver: "8.8.8.8:53"
      acme.dns01.propagation_delay: "30s"

  # --- EST ---
  - name: ox_cert_est
    phase: Content
    lib: libox_cert_est.so
    route: "GET,POST /.well-known/est/*"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys
      est.require_client_cert: true
      est.basic_auth_enabled: false
      est.labels:
        iot: short_lived
        server: standard

  # --- SCEP ---
  - name: ox_cert_scep
    phase: Content
    lib: libox_cert_scep.so
    route: "GET,POST /scep"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys
      scep.challenge_ttl: "1h"
      scep.encryption_algorithm: aes-256-cbc

  # --- AD Auto-Enrollment (interface only; Kerberos implementation deferred) ---
  - name: ox_cert_ad_autoenroll
    phase: Content
    lib: libox_cert_ad_autoenroll.so
    route: "GET,POST /certsrv/*"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys
      ad.domain: "corp.example.com"
      ad.ldap_uri: "ldaps://dc.corp.example.com"
      ad.auth_mode: client_cert          # client_cert | kerberos (kerberos = deferred)
      ad.templates:
        - name: Machine
          oid: "1.3.6.1.4.1.311.21.8.1"
          key_type: rsa-2048
          validity: "1y"
          autoenroll_group: "Domain Computers"

  # --- SSH CA ---
  - name: ox_cert_ssh
    phase: Content
    lib: libox_cert_ssh.so
    route: "GET,POST /api/v1/ssh/*"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys
      ssh.user_ca.key_id: ssh-user-ca
      ssh.user_ca.key_type: ed25519
      ssh.host_ca.key_id: ssh-host-ca
      ssh.host_ca.key_type: ed25519
      ssh.user.default_validity: "16h"
      ssh.host.default_validity: "720h"
      ssh.user.allowed_principals: ["*"]
      ssh.host.allowed_principals: ["*.example.com", "*.internal.local"]
      ssh.user.default_extensions:
        permit-pty: ""
        permit-port-forwarding: ""
        permit-agent-forwarding: ""

  # --- RA ---
  - name: ox_cert_ra
    phase: Content
    lib: libox_cert_ra.so
    route: "GET,POST /api/v1/ra/*"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      ra.auto_approve_rules:
        - identity_pattern: ".*@example\\.com$"
          profiles: [short_lived, standard]
      ra.notification_webhook: "https://hooks.example.com/ra-pending"
      ra.resubmit_queue: "tasks.pending"

  # --- Revocation / Status ---
  - name: ox_cert_ocsp
    phase: Content
    lib: libox_cert_ocsp.so
    route: "GET /ocsp/*,POST /ocsp"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys
      ocsp.responder_key_id: intermediate    # use intermediate CA key; or set ocsp.delegated_key_id

  - name: ox_cert_crl
    phase: Content
    lib: libox_cert_crl.so
    route: "GET /crl/*"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys
      crl.update_interval: "1h"
      crl.delta_interval: "10m"
      crl.cache_ttl: "30m"
      crl.lock_ttl_secs: 300

  # --- Export / Admin / Health ---
  - name: ox_cert_p12
    phase: Content
    lib: libox_cert_p12.so
    route: "GET /api/v1/certificates/*.p12,POST /api/v1/certificates/*.p12"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys
      p12.encryption: aes256

  - name: ox_cert_admin
    phase: Content
    lib: libox_cert_admin.so
    route: "GET,POST /api/v1/certificates,GET /api/v1/audit,GET,POST /api/v1/ca,GET /api/v1/ssh"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys

  - name: ox_cert_health
    phase: Content
    lib: libox_cert_health.so
    route: "GET /healthz,GET /readyz,GET /api/v1/health"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
      keystore.type: software
      keystore.passphrase_env: OX_CA_KEY_PASS
      keystore.key_dir: /etc/ox_cert/keys
      health.ca_cert_warn_days: 365
      health.crl_staleness_threshold: "2h"

  # --- CT query endpoints (issuance-time submission is a library call in ox_cert_core) ---
  - name: ox_cert_ct
    phase: Content
    lib: libox_cert_ct.so
    route: "GET /api/v1/ct/*"
    params:
      tenant_id: acme-corp
      store.driver: postgresql
      store.url: "postgresql://ca@db:5432/ox_cert"
```

---

## Resolved Design Decisions

| # | Decision |
|---|---|
| 1 | **Multi-tenancy:** Full multi-tenant from the start. `tenant_id` is a required field on every `KeyStore` and `CertStore` call and a column on every table. |
| 2 | **HA model:** Active/active. UUID v4 serials (collision-safe by construction). CRL number sequencing uses an advisory lock table via `CertStore::acquire_crl_lock`. |
| 3 | **Persistence:** All structured data goes through `ox_persistence` (`PersistenceDriver` + `GenericDataObject`). `CertStore` is a trait; `OxPersistenceCertStore` is the concrete implementation. |
| 4 | **CA key sharing:** Each plugin loads the CA key independently via `KeyStore`. `ox_cert_ca_init` validates and optionally generates keys at startup; subsequent plugins open their own `KeyStore` handle using the same config. No shared Arc across plugins. |
| 5 | **Serial numbers:** UUID v4 (`uuid::Uuid::new_v4()`), stored as TEXT. The 16 UUID bytes satisfy the RFC 5280 ≤20 byte serial requirement. SSH certs retain u64 serials per the OpenSSH specification. |
| 6 | **Certificate Transparency:** Issuance-time SCT submission is a library call in `ox_cert_core::ct::submit_to_ct_logs()` — not a pipeline stage. `ox_cert_ct` plugin exists solely to serve SCT query endpoints (`GET /api/v1/ct/scts/{serial}`). |
| 7 | **RA re-submission:** After approval, `ox_cert_ra` creates a new `Task` record in workflow storage with the stored CSR and `cert.ra.approved = true` in task metadata, then publishes the task UUID to the `tasks.pending` queue via `CoreHostApi::publish_to_queue`. The workflow scheduler picks it up and re-runs the standard issuance pipeline. This requires two extensions to `CoreHostApi` (see spec/core.md). |
| 8 | **Private key storage:** Server-generated keys stored in `certificates.private_key_encrypted` as `base64(nonce[12] \|\| AES-256-GCM-ciphertext \|\| tag[16])`. Encryption key derived via HKDF-SHA-256 from `OX_CA_KEY_PASS` with info=`"ox_cert:private_key_enc_v1"` and salt=`tenant_id`. |
| 9 | **Authorization:** All endpoint authorization is handled by `ox_webservice` URL-based permission management. Cert plugins trust any request that reaches them. |
| 10 | **Rate limiting:** ACME-specific rate limits embedded in `ox_cert_acme`. REST API rate limiting handled by a general-purpose `ox_webservice` rate-limit plugin upstream in the pipeline. |
| 11 | **OCSP responder key:** Configurable — either the intermediate CA key (`ocsp.responder_key_id: intermediate`) or a dedicated delegated OCSP signing certificate (`ocsp.delegated_key_id`). See spec/plugin_ocsp.md. |
| 12 | **AD auto-enrollment:** Interface fully specified (routes, config, data shapes). Kerberos/SPNEGO authentication implementation deferred pending a dedicated FFI spike against MIT-krb5. Client-certificate auth mode is implemented. See spec/plugin_ad_autoenroll.md. |
| 13 | **Webhook secrets:** Always via environment variable reference (`secret_env: ENV_VAR_NAME`). Inline secret values are never accepted in YAML config. |
| 14 | **Backup/DR:** Left to external tooling (PostgreSQL `pg_dump`, filesystem snapshots for SQLite). `ox_cert_admin` does not expose a backup API. |
