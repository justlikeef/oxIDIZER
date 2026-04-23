# ox_cert_ca_init

**Purpose:** Initializes or loads the CA hierarchy (root + intermediate) at server startup.
Validates keys, optionally generates them, and makes the CA ready for issuance.

---

## Phase
`PreEarlyRequest` — runs once at server startup. Not request-driven.

## Routes
None. This is an init-only module; `ox_plugin_process` is a no-op.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | All shared types, `KeyStore`, `CertStore`, `CertBuilder`, `CertError` |
| `rcgen` | Root and intermediate CA cert generation |
| `x509-parser` | Load and validate existing CA cert PEM |
| `pem` | PEM decode |
| `serde` / `serde_json` | Config deserialization |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct CaInitConfig {
    pub tenant_id: String,
    pub keystore: KeyStoreConfig,
    pub ca: CaHierarchyConfig,
    pub auto_generate: bool,
    pub extensions: ExtensionsConfig,
    pub store: CertStoreConfig,
}

#[derive(Debug, Deserialize)]
pub struct CaHierarchyConfig {
    pub root: CaCertConfig,
    pub intermediate: CaCertConfig,
}

#[derive(Debug, Deserialize)]
pub struct CaCertConfig {
    pub key_path: String,           // used as key_id in KeyStore
    pub cert_path: String,          // path to PEM cert file on disk
    pub key_type: String,           // "ecc-p384" | "rsa-4096"
    pub validity_years: u32,
    pub subject: String,            // full DN string
    pub name_constraints: Option<NameConstraintsConfig>,
    pub path_length: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct NameConstraintsConfig {
    pub permitted_dns: Vec<String>,
    pub excluded_dns: Vec<String>,
}
```

---

## ModuleContext

```rust
pub struct CaInitContext {
    api: CoreHostApi,
    config: CaInitConfig,
    store: Box<dyn CertStore>,
}
```

The loaded `KeyStore` is not held in context — each downstream plugin opens its own
`KeyStore` handle using the same config (see design decision #4 in CERTSERVERSPEC.md).

---

## Processing (ox_plugin_init)

1. Parse config from `plugin_config_ctx` via `ox_cert_core::parse_config::<CaInitConfig>`.
2. Open `CertStore` and call `migrate()`.
3. Open `KeyStore` using `config.keystore`.
4. **Root CA:**
   a. Call `key_store.key_exists(tenant_id, root_key_id)`.
   b. If exists: load root cert PEM from `ca.root.cert_path`; parse with `x509-parser`;
      validate `not_after > now + 30 days`; log key fingerprint.
   c. If missing and `auto_generate = true`: generate key via
      `key_store.generate_key(tenant_id, root_key_id, key_type, false)`; build self-signed
      root CA cert via `CertBuilder`; write cert PEM to `ca.root.cert_path`; persist
      `CaKeyRecord` with `status = Active` to `CertStore`.
   d. If missing and `auto_generate = false`: log error and return null from
      `ox_plugin_init` (server will not start).
5. **Intermediate CA:** Same logic, signed by root key instead of self-signed.
   Name constraints and path length applied from config.
6. Set `cert.ca.ready = "true"` as a persistent flag in a well-known TaskState-equivalent
   location (host-level metadata) so downstream plugins can assert CA readiness at
   request time if needed. (In practice, plugins simply attempt `KeyStore::key_exists`
   and return `CertError::CaNotReady` if missing.)
7. Log CA hierarchy summary: tenant, subject DNs, key types, expiry dates.

## Processing (ox_plugin_process)

Returns `FLOW_CONTROL_CONTINUE` immediately. No request handling.

## Processing (ox_plugin_destroy)

No-op. `KeyStore` and `CertStore` handles drop automatically.

---

## TaskState Fields Set

None. CA state is held in the database (`ca_keys` table) and `KeyStore` filesystem.

---

## Error Cases

| Condition | Behaviour |
|---|---|
| Key missing and `auto_generate = false` | Returns null from `ox_plugin_init`; server fails to start |
| CA cert expired | Logs `ERROR`; continues with warning (does not block startup) |
| CA cert expiring within 90 days | Logs `WARN` |
| `KeyStore::generate_key` fails | Returns null; server fails to start |
| `CertStore::migrate` fails | Returns null; server fails to start |

---

## Notes

- `auto_generate: true` is safe to leave enabled in production: if the key already exists,
  generation is skipped (the `overwrite: false` parameter to `generate_key`).
- The root CA private key should be stored offline after the intermediate CA is signed.
  If `ca.root.key_path` resolves to a missing file, startup logs `WARN` but proceeds,
  as the root key is not needed for day-to-day issuance.
- For CA key rollover (adding a new intermediate while the old one expires), use the
  `POST /api/v1/ca/rollover` endpoint in `ox_cert_admin`. Do not change `ca_init` config
  while a rollover is in progress.
