# ox_cert_ca_init

Initializes or loads the CA hierarchy (root + intermediate) at server startup. Validates
keys, optionally generates them, and ensures the CA is ready before any issuance plugins
run.

---

## Phase

`PreEarlyRequest` — runs once during server startup, not per-request.

## Routes

None. `ox_plugin_process` returns `FLOW_CONTROL_CONTINUE` immediately.

---

## Config Reference

```rust
pub struct CaInitConfig {
    pub tenant_id: String,
    pub keystore: KeyStoreConfig,
    pub ca: CaHierarchyConfig,
    pub auto_generate: bool,
    pub extensions: ExtensionsConfig,
    pub store: CertStoreConfig,
}

pub struct CaHierarchyConfig {
    pub root: CaCertConfig,
    pub intermediate: CaCertConfig,
}

pub struct CaCertConfig {
    pub key_path: String,              // used as key_id in KeyStore
    pub cert_path: String,             // on-disk PEM cert file path
    pub key_type: String,              // "ecc-p384" | "rsa-4096" | "ecc-p521"
    pub validity_years: u32,
    pub subject: String,               // full DN string, e.g. "CN=...,O=...,C=US"
    pub name_constraints: Option<NameConstraintsConfig>,
    pub path_length: Option<u32>,
}
```

| Field | Default | Description |
|---|---|---|
| `tenant_id` | required | Tenant this CA serves |
| `auto_generate` | `false` | Generate keys and certs if missing; safe to leave `true` in production |
| `keystore.type` | required | `software` or `pkcs11` |
| `keystore.passphrase_env` | required (software) | Env var name for PKCS#8 passphrase |
| `keystore.key_dir` | required (software) | Base directory for key files |
| `ca.root.key_type` | required | Key algorithm: `ecc-p384`, `rsa-4096`, `ecc-p521` |
| `ca.root.validity_years` | required | Root CA certificate validity in years (typically 25) |
| `ca.intermediate.path_length` | `0` | Path length constraint on the intermediate cert |
| `ca.intermediate.name_constraints.permitted_dns` | `[]` | List of permitted DNS name suffixes |
| `extensions.aia.ocsp_url` | optional | OCSP URL embedded in issued certs |
| `extensions.aia.ca_issuer_url` | optional | CA issuer URL embedded in issued certs |
| `extensions.cdp.url` | optional | CRL distribution point URL |
| `store.driver` | required | `postgresql` or `sqlite` |
| `store.url` | required (postgres) | PostgreSQL connection URL |
| `store.path` | required (sqlite) | SQLite file path |

---

## What It Does at Startup

1. Opens `CertStore` and calls `migrate()` — applies schema migrations idempotently.
2. Opens `KeyStore` using the configured keystore type.
3. **Root CA:**
   - If key exists: loads and validates the cert PEM; logs a warning if expiring within
     90 days, an error if already expired.
   - If key missing and `auto_generate = true`: generates the key pair, self-signs the
     root CA cert, writes PEM to `cert_path`, stores `CaKeyRecord` in the database.
   - If key missing and `auto_generate = false`: returns null from `ox_plugin_init` and
     the server fails to start.
4. **Intermediate CA:** same logic, but the certificate is signed by the root key instead
   of self-signed. Name constraints and path length are applied from config.
5. Logs a CA hierarchy summary: tenant, subject DNs, key types, expiry dates.

---

## Error Cases

| Condition | Behavior |
|---|---|
| Key missing, `auto_generate = false` | Returns null; server fails to start |
| `CertStore::migrate()` fails | Returns null; server fails to start |
| `KeyStore::generate_key()` fails | Returns null; server fails to start |
| CA cert expired | Logs `ERROR`; continues (operator must act) |
| CA cert expiring within 90 days | Logs `WARN` |
| Root key file missing (after intermediate is signed) | Logs `WARN`; continues |

---

## Implementation Notes

- `auto_generate: true` is safe to leave enabled permanently. The `overwrite: false`
  parameter to `KeyStore::generate_key()` means generation is a no-op if the key exists.
- The loaded `KeyStore` handle is not stored in the plugin context — each downstream
  plugin opens its own handle independently (design decision #4 in CERTSERVERSPEC.md).
- For CA key rollover (adding a new intermediate while the old one expires), use the
  `POST /api/v1/ca/rollover` endpoint in `ox_cert_admin`. Do not modify `ox_cert_ca_init`
  config while a rollover is in progress.
- Name constraints are only applied to the intermediate CA certificate. The root CA is
  unconstrained.
