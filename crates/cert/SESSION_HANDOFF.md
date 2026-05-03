# ox_cert Refactoring Session Handoff

## Goal
Build the Certificate Authority server (`ox_cert`) as a family of `ox_workflow` plugins backed by
the `ox_data` GDO persistence layer. All shared types, traits, and crypto helpers live in
`ox_cert_core`; individual CA functions are separate `cdylib` plugins.

## What Has Been Completed

### `ox_cert_core` — fully implemented
Located at `crates/cert/ox_cert_core/`.

- **Schema registration** (`lib.rs`): `register_schemas()` registers 8 `DataObjectSchema`s.
- **Model types** (`model.rs`): all CA structs, enums, config types.
  - `IssuancePolicy::validate_csr()` — validates SAN count, wildcards, allowlist/blocklist, RSA key size.
- **`CertStore` trait + `OxPersistenceCertStore`** (`store.rs`): full trait, `open()`, `migrate()`.
- **`KeyStore` trait + `SoftwareKeyStore`** (`keystore.rs`): including `load_key_pem()` for CA init/issue.
- **`CertBuilder`** (`builder.rs`):
  - `self_sign()`, `sign_with_issuer()`, `sign_csr()` (with optional `override_sans`), `parse_csr()`.
  - `issuer_params_from_cert_pem()` — rebuilds `rcgen::CertificateParams` from an existing CA cert
    PEM (x509-parser) for use as an issuer when `CertificateParams::from_ca_cert_pem` is unavailable in rcgen 0.14.
- **Unit tests** (`tests.rs`): 13 passing, 7 ignored (real persistence backend), 0 failing.

### Plugins implemented

| Crate | Purpose | Status |
|---|---|---|
| `ox_cert_ca_init` | Load/generate root+intermediate CA hierarchy at startup | ✅ |
| `ox_cert_issue` | `POST /api/v1/certificates` — CSR-based and server-keygen issuance | ✅ |
| `ox_cert_revoke` | `POST /api/v1/certificates/{serial}/revoke` | ✅ |
| `ox_cert_health` | `GET /healthz`, `/readyz`, `/api/v1/health` | ✅ |

All 5 crates (`ox_cert_core` + 4 plugins) compile with zero errors and zero warnings.

## Architecture Notes

- **`issuer_params_from_cert_pem`**: x509-parser is used to extract the subject DN from the
  intermediate CA cert PEM; the DN is then pushed into `rcgen::CertificateParams`. This
  reconstructs the issuer context because rcgen 0.14 has no `CertificateParams::from_ca_cert_pem`.
- **`sign_csr` override_sans**: The `sign_csr` function accepts `Option<&[SanType]>` to replace
  the CSR's SANs (used for request-body override and webhook enrichment in `ox_cert_issue`).
- **DataObjectManager placeholder**: Store roundtrip tests are `#[ignore]`d pending a real driver.
- **Server-keygen key storage**: Keys generated server-side use the cert serial as the key_id.
  The plugin does NOT yet write `private_key_encrypted` — that requires HKDF passphrase config.
  This is noted as a TODO.

## Next Steps

Remaining plugins (priority order):

1. **`ox_cert_crl`** (`spec/plugin_crl.md`) — CRL generation and serving.
   - Routes: `GET /crl/{tenant_id}.crl`
   - Requires `CertStore::list_revoked()` method (not yet in store.rs)
   - Uses `rcgen` or `x509-parser` to build the CRL DER

2. **`ox_cert_ocsp`** (`spec/plugin_ocsp.md`) — OCSP responder.
   - Routes: `GET /ocsp/*`, `POST /ocsp`
   - Requires `CertStore::get_cert_by_serial()` (already exists) and signing logic

3. **`ox_cert_admin`** (`spec/plugin_admin.md`) — Admin API for cert listing, CA management.
   - Routes: `GET /api/v1/certificates`, `GET /api/v1/audit`, `GET,POST /api/v1/ca`
   - Requires `CertStore::list_certs()` and `list_expiring()` methods (not yet in store.rs)

4. **`ox_cert_acme`** (`spec/plugin_acme.md`) — ACME v2 protocol.

5. **`ox_cert_ssh`** (`spec/plugin_ssh.md`) — SSH certificate signing.

6. **Wire real persistence**: Once `ox_persistence_driver_db_*` is available, remove `#[ignore]`
   from store roundtrip tests and verify end-to-end issuance.

## Missing `CertStore` Methods
The following store methods are referenced by upcoming plugins but not yet implemented:

| Method | Needed by |
|---|---|
| `list_certs(tenant_id, offset, limit)` | `ox_cert_admin` |
| `list_expiring(tenant_id, days)` | `ox_cert_admin`, `ox_cert_health` (stub) |
| `list_revoked(tenant_id)` | `ox_cert_crl` |
| `acquire_crl_lock(tenant_id)` | `ox_cert_crl` |
| `release_crl_lock(tenant_id)` | `ox_cert_crl` |

## Key Design Decisions (from CERTSERVERSPEC.md)

| # | Decision |
|---|---|
| 1 | Multi-tenancy: `tenant_id` required on every KeyStore/CertStore call |
| 2 | HA: active/active, UUID v4 serials |
| 3 | Persistence: all data through GDO/DataObjectManager |
| 4 | No shared Arc across plugins — each plugin opens its own KeyStore handle |
| 5 | Serial numbers: UUID v4 stored as TEXT |
| 8 | Private keys: base64(nonce[12] \|\| AES-256-GCM || tag[16]) in `private_key_encrypted` |
