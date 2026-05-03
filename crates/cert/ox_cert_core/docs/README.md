# ox_cert_core

Shared library crate (not a plugin). All `ox_cert_*` plugins depend on it. Provides
every common type, crypto helper, storage trait, and the CA key-material interface.
Plugins never link against each other — they communicate through `TaskState` fields or
the shared `CertStore`.

---

## What It Exports

### KeyStore Trait and Implementations

Abstraction over signing key material with two concrete implementations:

- **Software:** PKCS#8 PEM files at `{key_dir}/{tenant_id}/{key_id}.key.pem`, encrypted
  with a passphrase from `OX_CA_KEY_PASS`.
- **PKCS#11:** HSM via `cryptoki`. Key labels take the form `{tenant_id}:{key_id}`.

Core methods: `sign()`, `public_key()` (DER SPKI), `generate_key()`, `key_exists()`,
`key_info()`, `delete_key()`.

All methods take `tenant_id: &str` explicitly — the keystore enforces the tenant boundary.

### CertStore Trait

Persistence abstraction for all certificate data, backed by `OxPersistenceCertStore`
which wraps `ox_data_object_manager`. Complete coverage of X.509 certs, SSH certs,
CA keys, ACME objects, RA requests, SCEP challenges, notifications, audit events, and
the CRL advisory lock table.

Every method takes `tenant_id: &str`. Calling `migrate()` on `open()` is always safe —
migrations use `IF NOT EXISTS` guards.

### CertBuilder

Wraps `rcgen` to produce X.509 v3 certificates. Accepts subject DN, SANs, validity,
key type, profile, and a `KeyStore` reference. Automatically injects standard extensions:
AIA (OCSP URL + CA issuer URL), CDP (CRL URL), SKI, AKI, policy OIDs, and CPS URI.

### SshCertBuilder

Builds OpenSSH native binary certificates (not X.509). Signs with the SSH CA key via
`KeyStore`. Returns `SshCertRecord` with the full base64 OpenSSH certificate.

### ChainBuilder / Pkcs12Builder

- `ChainBuilder::full_chain_pem()` — assembles leaf + intermediate + root PEM chain.
- `Pkcs12Builder::build()` — bundles cert + key + chain into a password-protected `.p12`.
  Supports both AES-256 and legacy 3DES encryption.

### CT Submission

`ox_cert_core::ct::submit_to_ct_logs()` handles issuance-time SCT submission. Called
directly by `ox_cert_issue` at signing time; not a pipeline stage. Embeds SCTs into the
issued certificate via extension OID `1.3.6.1.4.1.11129.2.4.2`.

### Error Types

`CertError` — unified error enum with HTTP status mapping. Variants cover every failure
mode in the cert system: `InvalidCsr`, `PolicyViolation`, `NotFound`, `AlreadyRevoked`,
`RaApprovalRequired`, `CaNotReady`, `WebhookRejected`, `CtFailure`, `TenantNotFound`,
`Internal`.

### Common Types

Full set of shared structs and enums used across all plugins:
`CertificateRecord`, `CertStatus`, `RevocationReason`, `EnrollmentProtocol`, `Sct`,
`DistinguishedName`, `SanType`, `ValidityPeriod`, `EnrollmentProfile`, `NameConstraints`,
`IssuancePolicy`, `CsrInfo`, `SshCertRecord`, `SshCertType`, `CaKeyRecord`, `CaKeySet`,
`AcmeAccount`, `AcmeOrder`, `AcmeAuthorization`, `AcmeChallenge`, `ApprovalRequest`,
`ScepChallenge`, `NotificationRecord`, `AuditEvent`, `HealthStatus`, `CheckResult`,
`PagedResult<T>`, filter structs.

### Config Parsing Helper

```rust
pub fn parse_config<T: DeserializeOwned>(raw: *const c_char) -> Result<T, CertError>
```

Handles null pointer check, `CStr` conversion, JSON deserialization, and structured error
logging. Every plugin calls this in `ox_plugin_init`.

### RA Queue Helper

```rust
pub fn enqueue_task(api: &CoreHostApi, task_id: &str, priority: u8) -> Result<(), CertError>
```

Wraps `CoreHostApi::publish_to_queue` for the `tasks.pending` queue. Used by
`ox_cert_ra` after approving a request.

---

## Key Design Decisions

**UUID serials:** `uuid::Uuid::new_v4().to_string()`. Stored as TEXT. 16 UUID bytes fit
the RFC 5280 ≤20-byte serial limit. Collision-safe without coordination.

**Private key encryption:** AES-256-GCM with HKDF-SHA-256 key derivation from
`OX_CA_KEY_PASS`. Wire format: `base64(nonce[12] || ciphertext || tag[16])`.

**CA key sharing:** Each plugin opens its own `KeyStore` handle independently. There is
no shared `Arc<KeyStore>` across plugins — `ox_cert_ca_init` validates at startup; all
other plugins open their own handle on demand.

**SSH serials:** u64 per the OpenSSH specification, not UUID. Generated via
`CertStore::get_next_ssh_serial()` which uses an atomic `UPDATE ... RETURNING`.

---

## Key Dependencies

| Crate | Purpose |
|---|---|
| `rcgen` | X.509 certificate generation |
| `ring` | RSA, ECDSA, AES-256-GCM, HKDF |
| `aws-lc-rs` | Optional: enables Ed448 support |
| `x509-parser` | Parse and validate CSRs and certs |
| `pem` | PEM encode/decode |
| `p12` | PKCS#12 bundle creation |
| `cryptoki` | PKCS#11 HSM interface |
| `uuid` (v4) | Serial number generation |
| `ssh-key` | OpenSSH certificate format |
| `regex` | Domain allow/block list matching |
| `hkdf`, `sha2` | Key derivation for private key encryption |
| `ox_data_object_manager` | `CertStore` backing implementation |
