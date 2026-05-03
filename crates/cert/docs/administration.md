# ox_cert Administration Guide

This guide is for operators deploying and maintaining ox_cert CA server instances.

---

## First-Time Setup

### 1. Choose a Database Backend

ox_cert requires a shared relational database reachable by all CA nodes.

**PostgreSQL** (recommended for production and multi-node deployments):
```yaml
store.driver: postgresql
store.url: "postgresql://ca_user:password@db.example.com:5432/ox_cert"
```

**SQLite** (single-node or development only):
```yaml
store.driver: sqlite
store.path: /var/lib/ox_cert/ca.db
```

Schema migrations run automatically at startup via `CertStore::migrate()`. They are
guarded with `IF NOT EXISTS`, so startup is always safe even if migrations already ran.

### 2. Create a Directory Structure

```
/etc/pki/ox_cert/
├── keys/
│   └── acme-corp/          # one subdirectory per tenant_id
│       ├── root.key.pem
│       └── intermediate.key.pem
├── ca/
│   └── acme-corp/
│       ├── root.crt
│       └── intermediate.crt
└── pipeline.yaml           # the ox_workflow pipeline configuration
```

Key directory permissions should be `700`, owned by the service account.

### 3. Configure CA Initialization

Add `ox_cert_ca_init` as a `PreEarlyRequest` module in your pipeline YAML. On first
startup with `auto_generate: true`, it generates the root and intermediate CA key pairs,
self-signs the root, cross-signs the intermediate, and writes certificate PEM files to
the configured paths.

```yaml
- name: ox_cert_ca_init
  phase: PreEarlyRequest
  lib: libox_cert_ca_init.so
  params:
    tenant_id: acme-corp
    auto_generate: true
    keystore.type: software
    keystore.passphrase_env: OX_CA_KEY_PASS
    ca.root.key_path: /etc/pki/ox_cert/private/acme-corp/root.key.pem
    ca.root.cert_path: /etc/pki/ox_cert/ca/acme-corp/root.crt
    ca.root.key_type: ecc-p384
    ca.root.validity_years: 25
    ca.root.subject: "CN=ACME Root CA,O=ACME Corp,C=US"
    ca.intermediate.key_path: /etc/pki/ox_cert/private/acme-corp/intermediate.key.pem
    ca.intermediate.cert_path: /etc/pki/ox_cert/ca/acme-corp/intermediate.crt
    ca.intermediate.key_type: ecc-p384
    ca.intermediate.validity_years: 10
    ca.intermediate.subject: "CN=ACME Intermediate CA,O=ACME Corp,C=US"
    ca.intermediate.path_length: 0
    extensions.aia.ocsp_url: "http://ocsp.example.com/ocsp"
    extensions.aia.ca_issuer_url: "http://pki.example.com/ca/acme-corp/intermediate.crt"
    extensions.cdp.url: "http://pki.example.com/crl/acme-corp"
    store.driver: postgresql
    store.url: "postgresql://ca@db:5432/ox_cert"
```

After the first successful startup, `auto_generate: true` is safe to leave enabled —
generation is skipped when the key already exists (the `overwrite: false` flag).

### 4. Secure the Root CA Key

After the intermediate CA certificate is signed, the root CA private key is no longer
needed for day-to-day operations. Move `root.key.pem` to offline storage. If
`ca.root.key_path` resolves to a missing file at startup, the server logs a `WARN` but
continues — the root key is only needed for intermediate CA rollover.

### 5. Set Required Environment Variables

| Variable | Required by | Purpose |
|---|---|---|
| `OX_CA_KEY_PASS` | All plugins using KeyStore | Passphrase for software private key files; used as IKM for private key encryption |
| `OX_NOTIFY_WEBHOOK_SECRET` | `ox_cert_notify` | HMAC signing secret for webhook notifications |
| `OX_WEBHOOK_HMAC_SECRET` | `ox_cert_webhook` | HMAC signing secret for authorization/enrichment hooks |

Webhook secrets are always referenced by environment variable name in the config (`secret_env: VAR_NAME`). Inline secrets are not accepted.

---

## Keystore Options

### Software Keystore (default)

Keys are stored as PKCS#8 PEM files encrypted with the `OX_CA_KEY_PASS` passphrase.

```yaml
keystore.type: software
keystore.passphrase_env: OX_CA_KEY_PASS
keystore.key_dir: /etc/pki/ox_cert/private
```

File layout: `{key_dir}/{tenant_id}/{key_id}.key.pem`

### PKCS#11 (HSM)

```yaml
keystore.type: pkcs11
keystore.pkcs11_module: /usr/lib/softhsm/libsofthsm2.so
keystore.pkcs11_slot: 0
keystore.pkcs11_pin_env: OX_HSM_PIN
```

Key labels in the HSM take the form `{tenant_id}:{key_id}`. Set the HSM PIN via
environment variable referenced in `pkcs11_pin_env`.

---

## Config File Structure

The pipeline YAML is loaded by the `ox_workflow` host. Use YAML anchors to avoid
repeating common parameters across modules:

```yaml
# Shared store config
_store: &store
  store.driver: postgresql
  store.url: "postgresql://ca@db:5432/ox_cert"

_keystore: &keystore
  keystore.type: software
  keystore.passphrase_env: OX_CA_KEY_PASS
  keystore.key_dir: /etc/pki/ox_cert/private

pipeline:
  phases:
    - Content: "ox_pipeline_router"

modules:
  - name: ox_cert_ca_init
    phase: PreEarlyRequest
    lib: libox_cert_ca_init.so
    params: { <<: [*store, *keystore], tenant_id: acme-corp, auto_generate: true, ... }

  - name: ox_cert_issue
    phase: Content
    lib: libox_cert_issue.so
    route: "POST /api/v1/certificates"
    params: { <<: [*store, *keystore], tenant_id: acme-corp, default_profile: standard, ... }
```

Full pipeline example with all standard plugins is in `CERTSERVERSPEC.md`.

---

## Plugin Activation

Plugins are activated by including them as `modules:` entries in the pipeline YAML. To
disable a plugin, remove or comment out its entry — no symlinks, no separate enable/disable
mechanism.

Conventionally, shared library files are placed in a directory such as
`/opt/ox/lib/cert/` and referenced by the `lib:` key in the module entry.

---

## Log Level

Log level is controlled via the `ox_workflow` host configuration. All cert plugins use
the host's structured logger. Common log patterns:

| Level | When |
|---|---|
| `ERROR` | Plugin init failure, storage failure, signing failure |
| `WARN` | CA cert expiry within 90 days, CRL staleness, root key missing |
| `INFO` | Certificate issued, renewed, revoked; CA init summary |
| `DEBUG` | Per-request routing, policy evaluation details |

---

## Health Checking

ox_cert exposes three health endpoints via `ox_cert_health`:

| Path | Use |
|---|---|
| `GET /healthz` | Kubernetes liveness probe — always 200 if the process responds |
| `GET /readyz` | Kubernetes readiness probe — 200 if healthy or degraded; 503 if unhealthy |
| `GET /api/v1/health` | Detailed JSON with per-check results (always 200) |

The health check inspects: CA key accessibility, database connectivity, CRL freshness,
and CA/root cert validity. The `ca_cert_warn_days` and `crl_staleness_threshold`
parameters control when degraded status is triggered.

Example health check from command line:
```bash
curl -s http://localhost:8443/api/v1/health | jq .data.status
```

---

## Common Operations

### Issue a Certificate

```bash
curl -X POST https://ca.example.com/api/v1/certificates \
  -H "Content-Type: application/json" \
  -d '{
    "csr": "-----BEGIN CERTIFICATE REQUEST-----\n...",
    "profile": "standard",
    "sans": ["api.example.com"]
  }'
```

Response includes `data.serial` (UUID), `data.certificate` (PEM), and `data.chain`.

### Renew a Certificate

```bash
curl -X POST https://ca.example.com/api/v1/certificates/{serial}/renew \
  -H "Content-Type: application/json" \
  -d '{"validity_seconds": 31536000}'
```

### Revoke a Certificate

```bash
curl -X POST https://ca.example.com/api/v1/certificates/{serial}/revoke \
  -H "Content-Type: application/json" \
  -d '{"reason": "key_compromise"}'
```

Valid reason codes: `unspecified`, `key_compromise`, `ca_compromise`,
`affiliation_changed`, `superseded`, `cessation_of_operation`, `certificate_hold`,
`privilege_withdrawn`.

### Query Revocation Status (OCSP)

```bash
# POST form
curl -X POST https://ca.example.com/ocsp \
  -H "Content-Type: application/ocsp-request" \
  --data-binary @request.der

# GET form (base64url-encoded DER in path)
curl https://ca.example.com/ocsp/{base64url-encoded-request}
```

### Fetch the CRL

```bash
# DER format
curl https://ca.example.com/crl -o crl.der

# PEM format
curl https://ca.example.com/crl.pem
```

---

## CA Key Rollover

To rotate the intermediate CA key while the CA remains operational:

```bash
# Initiate rollover — generates new intermediate key, begins dual-signing
curl -X POST https://ca.example.com/api/v1/ca/rollover

# After all old certs have been replaced or expired, commit
curl -X POST https://ca.example.com/api/v1/ca/rollover/commit

# Or abort if you want to discard the new key
curl -X POST https://ca.example.com/api/v1/ca/rollover/abort
```

Do not change `ox_cert_ca_init` config while a rollover is in progress.

---

## Certificate Expiry Monitoring

`ox_cert_notify` runs on a cron schedule and sends notifications for certificates
approaching expiry. Default thresholds: 90, 60, 30, 14, 7, and 1 day.

Notifications are deduplicated: a notification at a given threshold is not resent unless
the previous one was sent more than `threshold / 2` days ago.

Configure the schedule and channels in the pipeline:

```yaml
- name: ox_cert_notify
  phase: PreEarlyRequest
  lib: libox_cert_notify.so
  params:
    tenant_id: acme-corp
    store.driver: postgresql
    store.url: "..."
    notify.schedule: "0 8 * * *"
    notify.thresholds_days: [90, 60, 30, 14, 7, 1]
    notify.include_ca_certs: true
    notify.channels:
      - type: webhook
        url: "https://hooks.example.com/cert-expiry"
        secret_env: OX_NOTIFY_WEBHOOK_SECRET
```

To check certificates expiring soon via the admin API:

```bash
curl "https://ca.example.com/api/v1/certificates/expiring?days=30"
```

---

## Multi-Tenant Operations

Tenant management requires `super_admin` permissions on `ox_cert_admin`:

```bash
# List tenants
curl https://ca.example.com/api/v1/tenants

# Create tenant
curl -X POST https://ca.example.com/api/v1/tenants \
  -d '{"tenant_id": "new-corp", "display_name": "New Corp"}'

# Deactivate tenant (data preserved, not deleted)
curl -X DELETE https://ca.example.com/api/v1/tenants/new-corp
```

Each tenant requires its own `ox_cert_ca_init` module entry in the pipeline with
`tenant_id` set to the tenant's ID.

---

## Backup and Disaster Recovery

ox_cert does not expose a backup API. Use the database's native tooling:

**PostgreSQL:**
```bash
pg_dump ox_cert > ox_cert_$(date +%Y%m%d).sql
```

**SQLite:**
```bash
sqlite3 /var/lib/ox_cert/ca.db ".backup /backups/ca_$(date +%Y%m%d).db"
```

CA private key files under `{key_dir}/` must also be backed up separately. For
software keystores, back up the entire `key_dir` tree with appropriate encryption.
For HSM-backed keystores, follow your HSM vendor's backup procedures.
