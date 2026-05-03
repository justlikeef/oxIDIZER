# ox_cert_core

Shared library crate (not a plugin). All `ox_cert_*` plugins depend on it. Contains every
common type, crypto helper, storage trait, and the CA key-material interface.

---

## Responsibilities

| Area | Details |
|---|---|
| **Key Management** | Load/generate RSA (2048, 3072, 4096), ECC (P-256, P-384, P-521), and EdDSA (Ed25519) keys. Wraps `rcgen` / `ring` / `aws-lc-rs`. |
| **Keystore Abstraction** | `KeyStore` trait with Software (PKCS#8 PEM, passphrase from env) and PKCS#11 (HSM via `cryptoki`) implementations. All signing goes through this trait. |
| **Certificate Builder** | `CertBuilder` wraps `rcgen`. Produces X.509 v3 DER/PEM. Automatically embeds AIA, CDP, SKI, AKI, policy OIDs. |
| **Extension Manager** | Injects standard extensions on every issued cert: AIA (OCSP URL + CA issuer URL), CDP (CRL URL), SKI, AKI. URLs from global config. |
| **Name Constraints** | RFC 5280 §4.2.1.10 name constraints on intermediate CAs — permitted/excluded DNS, IP, email. |
| **Path Length** | `basicConstraints` path length enforcement per CA profile. |
| **Certificate Policy Engine** | Policy OIDs and CPS URI qualifiers per profile. Supports policy mapping for cross-certified CAs. |
| **Certificate Profiles** | `short_lived`, `standard`, `long_lived`, `ca_intermediate`, `ca_root`. |
| **Issuance Policy** | Domain allowlist/blocklist, max SAN count, wildcard flag, min key strength, RA-required flag. |
| **Storage Trait** | `CertStore` — persistence abstraction backed by `ox_data_object_manager`. |
| **Serial Number Generation** | UUID v4 (`uuid::Uuid::new_v4()`). 16 bytes ≤ 20-byte RFC 5280 limit. |
| **CA Key Rollover** | Dual-signing during rotation: `CaKeySet` holds `active` + optional `retiring` key. |
| **SSH Certificate Builder** | OpenSSH user and host certs (not X.509). Ed25519 and ECDSA signing keys. |
| **CT Submission** | `ox_cert_core::ct::submit_to_ct_logs()` — called by `ox_cert_issue` at signing time. |
| **Chain Builder** | `ChainBuilder` — assembles PEM chains (issued + intermediate + root). |
| **PKCS#12 Builder** | `Pkcs12Builder` — bundles cert + key + chain into password-protected `.p12`. |
| **Private Key Encryption** | AES-256-GCM for server-generated keys at rest. |
| **Config Parsing Helper** | `ox_cert_core::parse_config::<T>()` — null check, CStr, JSON deserialization, logging. |
| **Common Types** | All shared structs and enums (see below). |
| **Error Types** | `CertError` — unified error enum with HTTP status mapping. |

---

## Key Dependencies

| Crate | Purpose |
|---|---|
| `rcgen` | X.509 certificate generation |
| `ring` | RSA, ECDSA, AES-256-GCM, HKDF, random |
| `aws-lc-rs` | Drop-in `ring` replacement; required if Ed448 support is needed |
| `x509-parser` | Parse and validate incoming CSRs and certificates |
| `pem` | PEM encode/decode |
| `p12` | PKCS#12 bundle creation |
| `cryptoki` | PKCS#11 interface for HSM |
| `uuid` (features: `v4`) | UUID v4 serial generation |
| `serde` / `serde_json` | Serialization |
| `time` | Validity period computation |
| `thiserror` | `CertError` derive |
| `base64` | Base64 encode/decode for key/cert data |
| `hkdf` | Key derivation for private key encryption |
| `sha2` | SHA-256 for HKDF, CT, fingerprints |
| `regex` | Domain allowlist/blocklist pattern matching |
| `ox_persistence` | lib | `ox_data_object`, `ox_type_converter` |
| `ox_data_object_manager` | lib | `ox_data_object`, `ox_persistence`, `ox_type_converter` |
| `GenericDataObject` | Core storage primitive for `CertStore` impl |

### EdDSA Note

`ring` supports Ed25519 but not Ed448. For Ed448 support, swap to `aws-lc-rs` as the
crypto backend. Ed25519 is the recommended default for SSH CA keys and short-lived certs.

---

## Multi-Tenancy Model

Every `KeyStore` and `CertStore` operation carries a `tenant_id: &str` parameter.

**Software KeyStore:** tenant_id maps to a subdirectory.
```
{key_dir}/{tenant_id}/{key_id}.key.pem
```

**PKCS#11 KeyStore:** tenant_id is prefixed onto the key label.
```
{tenant_id}:{key_id}
```

**CertStore:** tenant_id is included as a filter in every `fetch()` call and as an attribute
in every `GenericDataObject`. Migrations apply globally via `DataObjectManager` schemas;
data is partitioned by tenant_id at the GDO level.

Tenant lifecycle (create, delete, list) is managed by `ox_cert_admin`. Deleting a tenant
does not immediately purge rows; it marks the tenant inactive and schedules a background
purge via `ox_cert_notify`.

---

## Active/Active HA

### UUID Serials

`uuid::Uuid::new_v4().to_string()` produces a collision-safe 128-bit random identifier.
Stored as TEXT (`"xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx"`). Used directly as the X.509
serial number (16 bytes of UUID binary, ≤ 20-byte RFC 5280 limit).

### CRL Number Sequencing

CRL numbers must be monotonically increasing (RFC 5280 §5.2.3). Under active/active,
only one node must generate a CRL for a given interval. Coordination uses an advisory
lock table in the shared database via `ox_data_object_manager`.

`CertStore` exposes `acquire_crl_lock` and `release_crl_lock` (see trait definition below).

---

## Private Key Encryption at Rest

Server-generated keys (for PKCS#12 export) are stored encrypted in
`certificates.private_key_encrypted`.

| Parameter | Value |
|---|---|
| Algorithm | AES-256-GCM |
| Key derivation | HKDF-SHA-256: IKM=`OX_CA_KEY_PASS`, salt=`tenant_id.as_bytes()`, info=`b"ox_cert:private_key_enc_v1"`, output=32 bytes |
| Nonce | 12 random bytes, generated per-key |
| Wire format | `base64(nonce[12] \|\| ciphertext \|\| tag[16])` stored as TEXT |
| Crate | `ring::aead::AES_256_GCM` |

Decryption requires the same `OX_CA_KEY_PASS` environment variable that protects the CA
signing keys, so PKCS#12 export is only possible on nodes that have the CA passphrase.

---

## CoreHostApi Extensions

Two function pointers must be added to `ox_workflow_abi::CoreHostApi` to enable
`ox_cert_ra` to re-submit approved requests into the workflow scheduler:

```rust
/// Publish a raw byte payload to a named priority queue (e.g. "tasks.pending").
/// priority: 0 (lowest) – 255 (highest). Returns 0 on success, non-zero on error.
pub publish_to_queue: unsafe extern "C" fn(
    queue_id:    *const c_char,
    priority:    u8,
    payload:     *const u8,
    payload_len: usize,
) -> i32,

/// Publish a raw byte payload to a pub/sub topic.
/// Returns 0 on success, non-zero on error.
pub publish_to_topic: unsafe extern "C" fn(
    topic:       *const c_char,
    payload:     *const u8,
    payload_len: usize,
) -> i32,
```

Helper provided by `ox_cert_core`:
```rust
pub fn enqueue_task(api: &CoreHostApi, task_id: &str, priority: u8) -> Result<(), CertError> {
    let queue = CString::new("tasks.pending").unwrap();
    let payload = task_id.as_bytes();
    let rc = unsafe {
        (api.publish_to_queue)(queue.as_ptr(), priority, payload.as_ptr(), payload.len())
    };
    if rc == 0 { Ok(()) } else { Err(CertError::Internal(format!("enqueue_task rc={rc}"))) }
}
```

---

## Common Types

```rust
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Certificate types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateRecord {
    pub serial: String,                               // UUID v4 string
    pub tenant_id: String,
    pub subject_cn: String,
    pub subject_dn: String,
    pub sans: Vec<String>,
    pub issuer_dn: String,
    pub not_before: time::OffsetDateTime,
    pub not_after: time::OffsetDateTime,
    pub key_type: String,                             // "rsa-2048", "ecc-p256", etc.
    pub profile: String,
    pub pem: String,
    pub csr_pem: Option<String>,
    /// AES-256-GCM encrypted PKCS#8 key; only present for server-generated keys.
    pub private_key_encrypted: Option<String>,        // base64(nonce||ct||tag)
    pub status: CertStatus,
    pub revoked_at: Option<time::OffsetDateTime>,
    pub revocation_reason: Option<RevocationReason>,
    pub scts: Vec<Sct>,
    pub policy_oids: Vec<String>,
    pub enrollment_protocol: Option<EnrollmentProtocol>,
    pub created_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertStatus {
    Active,
    Revoked,
    Expired,
    PendingApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum RevocationReason {
    Unspecified         = 0,
    KeyCompromise       = 1,
    CaCompromise        = 2,
    AffiliationChanged  = 3,
    Superseded          = 4,
    CessationOfOperation = 5,
    CertificateHold     = 6,
    RemoveFromCrl       = 8,
    PrivilegeWithdrawn  = 9,
    AaCompromise        = 10,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnrollmentProtocol {
    Rest, Acme, Est, Scep, Ad, Ssh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sct {
    pub log_id: String,                               // base64 log ID
    pub log_name: String,
    pub timestamp: time::OffsetDateTime,
    pub signature: String,                            // base64 signature
}

// ---------------------------------------------------------------------------
// Distinguished Name / SAN
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DistinguishedName {
    pub common_name: String,
    pub organization: Option<String>,
    pub organizational_unit: Option<String>,
    pub country: Option<String>,
    pub state: Option<String>,
    pub locality: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SanType {
    Dns(String),
    Ip(std::net::IpAddr),
    Email(String),
    Uri(String),
}

// ---------------------------------------------------------------------------
// Validity / Profile / Policy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidityPeriod {
    pub not_before: time::OffsetDateTime,
    pub not_after: time::OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomExtension {
    pub oid: String,
    pub critical: bool,
    pub value: Vec<u8>,                               // DER-encoded
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentProfile {
    pub name: String,
    pub validity_seconds: u64,                        // default validity in seconds
    pub key_usage: Vec<String>,
    pub extended_key_usage: Vec<String>,
    pub allowed_key_types: Vec<KeyType>,
    pub max_san_count: usize,
    pub wildcard_allowed: bool,
    pub require_ra_approval: bool,
    pub policy_oids: Vec<String>,
    pub cps_uri: Option<String>,
    pub name_constraints: Option<NameConstraints>,
    pub path_length: Option<u32>,
    pub is_ca: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameConstraints {
    pub permitted_dns: Vec<String>,
    pub excluded_dns: Vec<String>,
    pub permitted_ip: Vec<String>,                    // CIDR notation
    pub excluded_ip: Vec<String>,
    pub permitted_email: Vec<String>,
    pub excluded_email: Vec<String>,
}

/// Policy rules enforced before signing.
#[derive(Debug, Clone)]
pub struct IssuancePolicy {
    pub domain_allowlist: Vec<regex::Regex>,          // SANs must match at least one
    pub domain_blocklist: Vec<regex::Regex>,          // SANs must not match any
    pub max_san_count: usize,
    pub wildcard_allowed: bool,
    pub min_rsa_bits: u32,
    pub require_ra_approval: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IssuancePolicyConfig {
    pub domain_allowlist: Vec<String>,                // regex strings
    pub domain_blocklist: Vec<String>,
    pub max_san_count: usize,
    pub wildcard_allowed: bool,
    pub min_rsa_bits: u32,
    pub require_ra_approval: bool,
}

/// Parsed CSR fields, validated before signing.
#[derive(Debug, Clone)]
pub struct CsrInfo {
    pub subject_dn: String,
    pub subject_cn: String,
    pub sans: Vec<SanType>,
    pub key_type: KeyType,
    pub key_bits: u32,
    pub public_key_der: Vec<u8>,
    pub raw_der: Vec<u8>,
}

// ---------------------------------------------------------------------------
// SSH types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshCertRecord {
    /// OpenSSH certificate serial (u64 per OpenSSH spec, not UUID).
    pub serial: u64,
    pub tenant_id: String,
    pub cert_type: SshCertType,
    pub key_id: String,
    pub principals: Vec<String>,
    pub public_key: String,                           // base64 OpenSSH public key
    pub signing_key_fingerprint: String,              // SHA-256 fingerprint of CA key
    pub valid_after: time::OffsetDateTime,
    pub valid_before: time::OffsetDateTime,
    pub critical_options: HashMap<String, String>,
    pub extensions: HashMap<String, String>,
    pub certificate: String,                          // full OpenSSH cert (base64)
    pub created_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshCertType { User, Host }

// ---------------------------------------------------------------------------
// CA Key types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaKeyRecord {
    pub id: String,                                   // e.g., "acme-corp:intermediate-2026"
    pub tenant_id: String,
    pub key_type: KeyType,
    pub cert_pem: String,
    pub key_ref: String,                              // file path or PKCS#11 label
    pub status: CaKeyStatus,
    pub not_before: time::OffsetDateTime,
    pub not_after: time::OffsetDateTime,
    pub name_constraints: Option<NameConstraints>,
    pub path_length: Option<u32>,
    pub created_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaKeyStatus { Active, Retiring, Retired }

/// Active CA key pair, with an optional retiring key during rollover.
#[derive(Debug, Clone)]
pub struct CaKeySet {
    pub tenant_id: String,
    pub active: CaKeyRecord,
    pub retiring: Option<CaKeyRecord>,
}

// ---------------------------------------------------------------------------
// ACME types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcmeAccount {
    pub id: String,
    pub tenant_id: String,
    pub jwk: String,                                  // JSON Web Key (RFC 7517)
    pub contact: Vec<String>,                         // mailto: URIs
    pub status: AcmeAccountStatus,
    pub eab_kid: Option<String>,
    pub created_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcmeAccountStatus { Valid, Deactivated, Revoked }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcmeOrder {
    pub id: String,
    pub tenant_id: String,
    pub account_id: String,
    pub status: AcmeOrderStatus,
    pub identifiers: Vec<AcmeIdentifier>,
    pub not_before: Option<time::OffsetDateTime>,
    pub not_after: Option<time::OffsetDateTime>,
    pub certificate_serial: Option<String>,           // UUID of issued cert
    pub expires: time::OffsetDateTime,
    pub created_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcmeOrderStatus { Pending, Ready, Processing, Valid, Invalid }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcmeIdentifier {
    pub identifier_type: String,                      // "dns"
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcmeAuthorization {
    pub id: String,
    pub tenant_id: String,
    pub order_id: String,
    pub identifier_type: String,
    pub identifier_value: String,
    pub status: AcmeAuthzStatus,
    pub challenges: Vec<AcmeChallenge>,
    pub expires: time::OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcmeAuthzStatus { Pending, Valid, Invalid, Deactivated, Expired, Revoked }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcmeChallenge {
    pub id: String,
    pub challenge_type: ChallengeType,
    pub token: String,
    pub status: AcmeChallengeStatus,
    pub validated_at: Option<time::OffsetDateTime>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChallengeType { Http01, Dns01, TlsAlpn01 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcmeChallengeStatus { Pending, Processing, Valid, Invalid }

// ---------------------------------------------------------------------------
// RA types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub tenant_id: String,
    pub csr_pem: String,
    pub requester_identity: String,                   // IP, CN, or email
    pub profile: String,
    pub sans: Vec<String>,
    pub status: ApprovalStatus,
    pub reviewer: Option<String>,
    pub review_notes: Option<String>,
    pub reviewed_at: Option<time::OffsetDateTime>,
    /// UUID of the issued certificate; set by ox_cert_issue after the re-submitted task completes.
    pub certificate_serial: Option<String>,
    pub created_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalStatus { Pending, Approved, Denied }

// ---------------------------------------------------------------------------
// SCEP types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScepChallenge {
    pub id: String,
    pub tenant_id: String,
    pub password_hash: String,                        // bcrypt or SHA-256 hex
    pub used: bool,
    pub expires_at: time::OffsetDateTime,
    pub created_at: time::OffsetDateTime,
}

// ---------------------------------------------------------------------------
// Notification types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationRecord {
    pub id: i64,                                      // auto-increment
    pub tenant_id: String,
    pub serial: String,
    pub threshold_days: u32,
    pub channel: NotificationChannel,
    pub sent_at: time::OffsetDateTime,
    pub status: NotificationStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationChannel { Webhook, Mqtt, Email }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationStatus { Sent, Failed }

// ---------------------------------------------------------------------------
// Audit types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,                                      // auto-increment
    pub tenant_id: String,
    pub timestamp: time::OffsetDateTime,
    pub action: AuditAction,
    pub serial: Option<String>,
    pub actor: String,                                // IP, account ID, CN, or role
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditAction {
    Issue, Renew, Revoke,
    AcmeOrder, AcmeFinalize, AcmeRevoke,
    RaApprove, RaDeny,
    CaInit, CaRollover, CaRolloverCommit, CaRolloverAbort,
    CrossSign,
    SshSign, SshRevoke,
    WebhookBlock,
    ScepEnroll, EstEnroll, AdEnroll,
    P12Export,
}

// ---------------------------------------------------------------------------
// Filter / Pagination types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CertFilter {
    pub subject_cn: Option<String>,
    pub san: Option<String>,
    pub status: Option<CertStatus>,
    pub profile: Option<String>,
    pub not_after_before: Option<time::OffsetDateTime>,
    pub not_after_after: Option<time::OffsetDateTime>,
    pub enrollment_protocol: Option<EnrollmentProtocol>,
}

#[derive(Debug, Clone, Default)]
pub struct SshCertFilter {
    pub cert_type: Option<SshCertType>,
    pub principal: Option<String>,
    pub valid_before_before: Option<time::OffsetDateTime>,
}

#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    pub action: Option<AuditAction>,
    pub serial: Option<String>,
    pub actor: Option<String>,
    pub from: Option<time::OffsetDateTime>,
    pub to: Option<time::OffsetDateTime>,
}

#[derive(Debug, Clone)]
pub struct Pagination {
    pub offset: u64,
    pub limit: u64,                                   // default 50, max 1000
}

#[derive(Debug, Clone)]
pub struct PagedResult<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

// ---------------------------------------------------------------------------
// CT types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct CtConfig {
    pub enabled: bool,
    pub logs: Vec<CtLogConfig>,
    pub min_scts: usize,                              // default 2
    pub timeout: std::time::Duration,
    pub on_failure: CtFailureMode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CtLogConfig {
    pub name: String,
    pub url: String,
    pub public_key_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum CtFailureMode { Block, Warn }

// ---------------------------------------------------------------------------
// Health types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
    pub status: HealthState,
    pub tenant_id: String,
    pub checks: HashMap<String, CheckResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum HealthState { Healthy, Degraded, Unhealthy }

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub message: Option<String>,
}
```

---

## Rust Trait Definitions

### `KeyStore` Trait

```rust
/// All signing operations go through this trait. Implementations are tenant-aware:
/// Software keystore uses {key_dir}/{tenant_id}/{key_id}.key.pem;
/// PKCS#11 uses label "{tenant_id}:{key_id}".
pub trait KeyStore: Send + Sync {
    fn open(config: &KeyStoreConfig) -> Result<Self, CertError> where Self: Sized;

    /// Sign data with the named key for the given tenant.
    fn sign(&self, tenant_id: &str, key_id: &str, algorithm: SigningAlgorithm, data: &[u8])
        -> Result<Vec<u8>, CertError>;

    /// DER-encoded SubjectPublicKeyInfo for the named key.
    fn public_key(&self, tenant_id: &str, key_id: &str) -> Result<Vec<u8>, CertError>;

    /// Generate a new key pair. No-op if key already exists and `overwrite` is false.
    fn generate_key(&self, tenant_id: &str, key_id: &str, key_type: KeyType, overwrite: bool)
        -> Result<(), CertError>;

    fn key_exists(&self, tenant_id: &str, key_id: &str) -> Result<bool, CertError>;
    fn key_info(&self, tenant_id: &str, key_id: &str) -> Result<KeyInfo, CertError>;
    fn delete_key(&self, tenant_id: &str, key_id: &str) -> Result<(), CertError>;
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyStoreConfig {
    pub store_type: KeyStoreType,
    // Software
    pub key_dir: Option<std::path::PathBuf>,
    pub passphrase_env: Option<String>,
    // PKCS#11
    pub pkcs11_module: Option<std::path::PathBuf>,
    pub pkcs11_slot: Option<u64>,
    pub pkcs11_pin_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub enum KeyStoreType { Software, Pkcs11 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyType {
    Rsa2048, Rsa3072, Rsa4096,
    EcP256, EcP384, EcP521,
    Ed25519,
}

#[derive(Debug, Clone)]
pub enum SigningAlgorithm {
    Sha256WithRsa, Sha384WithRsa, Sha512WithRsa,
    EcdsaWithSha256, EcdsaWithSha384, EcdsaWithSha512,
    Ed25519,
}

#[derive(Debug, Clone)]
pub struct KeyInfo {
    pub key_id: String,
    pub tenant_id: String,
    pub key_type: KeyType,
    pub created_at: time::OffsetDateTime,
}
```

---

### `CertStore` Trait

```rust
/// Persistence abstraction for all certificate data.
/// Backed by OxPersistenceCertStore (wraps ox_data_object_manager DataObjectManager).
/// Every method takes tenant_id explicitly; implementations enforce the partition.
pub trait CertStore: Send + Sync {
    fn open(config: &CertStoreConfig) -> Result<Self, CertError> where Self: Sized;

    /// Apply pending schema migrations. Safe to call on every startup (idempotent).
    fn migrate(&self) -> Result<(), CertError>;

    // --- X.509 Certificates ---
    fn store_cert(&self, tenant_id: &str, record: &CertificateRecord) -> Result<(), CertError>;
    fn get_cert_by_serial(&self, tenant_id: &str, serial: &str)
        -> Result<Option<CertificateRecord>, CertError>;
    fn get_certs_by_subject(&self, tenant_id: &str, subject_cn: &str)
        -> Result<Vec<CertificateRecord>, CertError>;
    fn list_certs(&self, tenant_id: &str, filter: &CertFilter, page: &Pagination)
        -> Result<PagedResult<CertificateRecord>, CertError>;
    fn mark_revoked(&self, tenant_id: &str, serial: &str, reason: RevocationReason,
        timestamp: time::OffsetDateTime) -> Result<(), CertError>;
    fn list_revoked(&self, tenant_id: &str) -> Result<Vec<CertificateRecord>, CertError>;
    fn list_revoked_since(&self, tenant_id: &str, since: time::OffsetDateTime)
        -> Result<Vec<CertificateRecord>, CertError>;
    fn list_expiring(&self, tenant_id: &str, within_days: u32)
        -> Result<Vec<CertificateRecord>, CertError>;
    fn update_status_expired(&self, tenant_id: &str) -> Result<u64, CertError>;

    // --- SSH Certificates ---
    fn store_ssh_cert(&self, tenant_id: &str, record: &SshCertRecord) -> Result<(), CertError>;
    fn get_ssh_cert_by_serial(&self, tenant_id: &str, serial: u64)
        -> Result<Option<SshCertRecord>, CertError>;
    fn list_ssh_certs(&self, tenant_id: &str, filter: &SshCertFilter, page: &Pagination)
        -> Result<PagedResult<SshCertRecord>, CertError>;
    fn get_next_ssh_serial(&self, tenant_id: &str) -> Result<u64, CertError>;

    // --- CA Keys ---
    fn store_ca_key(&self, tenant_id: &str, key: &CaKeyRecord) -> Result<(), CertError>;
    fn get_active_ca_key(&self, tenant_id: &str) -> Result<Option<CaKeyRecord>, CertError>;
    fn get_ca_key_by_id(&self, tenant_id: &str, id: &str)
        -> Result<Option<CaKeyRecord>, CertError>;
    fn update_ca_key_status(&self, tenant_id: &str, id: &str, status: CaKeyStatus)
        -> Result<(), CertError>;

    // --- ACME ---
    fn store_acme_account(&self, tenant_id: &str, account: &AcmeAccount) -> Result<(), CertError>;
    fn get_acme_account(&self, tenant_id: &str, id: &str)
        -> Result<Option<AcmeAccount>, CertError>;
    fn store_acme_order(&self, tenant_id: &str, order: &AcmeOrder) -> Result<(), CertError>;
    fn get_acme_order(&self, tenant_id: &str, id: &str) -> Result<Option<AcmeOrder>, CertError>;
    fn update_acme_order_status(&self, tenant_id: &str, id: &str, status: AcmeOrderStatus)
        -> Result<(), CertError>;
    fn store_acme_authorization(&self, tenant_id: &str, authz: &AcmeAuthorization)
        -> Result<(), CertError>;
    fn get_acme_authorization(&self, tenant_id: &str, id: &str)
        -> Result<Option<AcmeAuthorization>, CertError>;
    fn update_acme_authorization(&self, tenant_id: &str, authz: &AcmeAuthorization)
        -> Result<(), CertError>;

    // --- RA ---
    fn store_ra_request(&self, tenant_id: &str, request: &ApprovalRequest)
        -> Result<(), CertError>;
    fn get_ra_request(&self, tenant_id: &str, id: &str)
        -> Result<Option<ApprovalRequest>, CertError>;
    fn list_ra_pending(&self, tenant_id: &str, page: &Pagination)
        -> Result<PagedResult<ApprovalRequest>, CertError>;
    fn update_ra_request(&self, tenant_id: &str, id: &str, status: ApprovalStatus,
        reviewer: &str, notes: &str) -> Result<(), CertError>;

    // --- SCEP ---
    fn store_scep_challenge(&self, tenant_id: &str, challenge: &ScepChallenge)
        -> Result<(), CertError>;
    fn consume_scep_challenge(&self, tenant_id: &str, password_hash: &str)
        -> Result<bool, CertError>;

    // --- Notifications ---
    fn store_notification(&self, tenant_id: &str, notification: &NotificationRecord)
        -> Result<(), CertError>;
    fn was_notification_sent(&self, tenant_id: &str, serial: &str, threshold_days: u32)
        -> Result<bool, CertError>;

    // --- Audit ---
    fn store_audit_event(&self, tenant_id: &str, event: &AuditEvent) -> Result<(), CertError>;
    fn get_audit_log(&self, tenant_id: &str, filter: &AuditFilter, page: &Pagination)
        -> Result<PagedResult<AuditEvent>, CertError>;

    // --- CRL coordination (active/active HA) ---
    /// Try to acquire the CRL generation lock for this tenant+lock_key.
    /// Returns Ok(Some(crl_number)) if acquired, Ok(None) if another node holds a valid lock.
    fn acquire_crl_lock(&self, tenant_id: &str, lock_key: &str, holder_id: &str,
        ttl_secs: u64) -> Result<Option<u64>, CertError>;
    fn release_crl_lock(&self, tenant_id: &str, lock_key: &str, holder_id: &str)
        -> Result<(), CertError>;
}

#[derive(Debug, Clone, Deserialize)]
pub struct CertStoreConfig {
    pub driver: String,                               // "sqlite" | "postgresql"
    pub path: Option<String>,                         // SQLite file path
    pub url: Option<String>,                          // PostgreSQL connection URL
}
```

---

## Data Dictionary Schemas (GDO)

All `ox_cert` data is stored as `GenericDataObject` instances. The following schemas
must be registered in the `DataDictionary` at startup.

### `certificate` (X.509)
| Attribute | Type | Description |
|---|---|---|
| `serial` | TEXT | UUID v4 (Primary Key) |
| `tenant_id` | TEXT | Partition key |
| `subject_cn` | TEXT | |
| `subject_dn` | TEXT | |
| `sans` | JSON | List of `SanType` |
| `issuer_dn` | TEXT | |
| `not_before` | TIMESTAMP | |
| `not_after` | TIMESTAMP | |
| `key_type` | TEXT | `rsa-2048`, `ecc-p256`, etc. |
| `profile` | TEXT | |
| `pem` | TEXT | |
| `csr_pem` | TEXT | Optional |
| `private_key_encrypted` | TEXT | Optional (server-generated keys) |
| `status` | TEXT | `active` \| `revoked` \| `expired` \| `pending_approval` |
| `revoked_at` | TIMESTAMP | Optional |
| `revocation_reason` | INTEGER | Optional (RevocationReason enum) |
| `scts` | JSON | List of `Sct` objects |
| `policy_oids` | JSON | |
| `enrollment_protocol` | TEXT | `rest`, `acme`, `est`, `scep`, `ad`, `ssh` |
| `created_at` | TIMESTAMP | |

### `ssh_certificate`
| Attribute | Type | Description |
|---|---|---|
| `serial` | BIGINT | OpenSSH serial (u64) |
| `tenant_id` | TEXT | |
| `cert_type` | TEXT | `user` \| `host` |
| `key_id` | TEXT | |
| `principals` | JSON | List of strings |
| `public_key` | TEXT | |
| `signing_key_fingerprint` | TEXT | |
| `valid_after` | TIMESTAMP | |
| `valid_before` | TIMESTAMP | |
| `critical_options` | JSON | Map<String, String> |
| `extensions` | JSON | Map<String, String> |
| `certificate` | TEXT | Full OpenSSH cert (base64) |
| `created_at` | TIMESTAMP | |

### `ca_key`
| Attribute | Type | Description |
|---|---|---|
| `id` | TEXT | PK: e.g., `"acme-corp:intermediate-2026"` |
| `tenant_id` | TEXT | |
| `key_type` | TEXT | |
| `cert_pem` | TEXT | |
| `key_ref` | TEXT | File path or PKCS#11 label |
| `status` | TEXT | `active` \| `retiring` \| `retired` |
| `not_before` | TIMESTAMP | |
| `not_after` | TIMESTAMP | |
| `name_constraints` | JSON | |
| `path_length` | INTEGER | |
| `created_at` | TIMESTAMP | |

### `acme_account`
| Attribute | Type | Description |
|---|---|---|
| `id` | TEXT | PK |
| `tenant_id` | TEXT | |
| `jwk` | TEXT | JSON Web Key |
| `contact` | JSON | List of mailto: URIs |
| `status` | TEXT | `valid` \| `deactivated` \| `revoked` |
| `eab_kid` | TEXT | Optional |
| `created_at` | TIMESTAMP | |

### `acme_order`
| Attribute | Type | Description |
|---|---|---|
| `id` | TEXT | PK |
| `tenant_id` | TEXT | |
| `account_id` | TEXT | FK → acme_account.id |
| `status` | TEXT | |
| `identifiers` | JSON | List of `{type, value}` |
| `not_before` | TIMESTAMP | |
| `not_after` | TIMESTAMP | |
| `certificate_serial` | TEXT | |
| `expires` | TIMESTAMP | |
| `created_at` | TIMESTAMP | |

### `acme_authorization`
| Attribute | Type | Description |
|---|---|---|
| `id` | TEXT | PK |
| `tenant_id` | TEXT | |
| `order_id` | TEXT | FK → acme_order.id |
| `identifier_type` | TEXT | |
| `identifier_value` | TEXT | |
| `status` | TEXT | |
| `challenges` | JSON | List of `AcmeChallenge` |
| `expires` | TIMESTAMP | |

### `ra_request`
| Attribute | Type | Description |
|---|---|---|
| `id` | TEXT | PK |
| `tenant_id` | TEXT | |
| `csr_pem` | TEXT | |
| `requester_identity` | TEXT | |
| `profile` | TEXT | |
| `sans` | JSON | |
| `status` | TEXT | `pending` \| `approved` \| `denied` |
| `reviewer` | TEXT | |
| `review_notes` | TEXT | |
| `reviewed_at` | TIMESTAMP | |
| `certificate_serial` | TEXT | |
| `created_at` | TIMESTAMP | |

### `audit_log`
| Attribute | Type | Description |
|---|---|---|
| `id` | BIGINT | Auto-increment PK |
| `tenant_id` | TEXT | |
| `timestamp` | TIMESTAMP | |
| `action` | TEXT | |
| `serial` | TEXT | |
| `actor` | TEXT | |
| `details` | JSON | |

---

## Implementation Patterns

### Plugin Config Parsing

```rust
// In ox_plugin_init:
let params_str = unsafe { CStr::from_ptr(plugin_config_ctx) }
    .to_string_lossy().to_string();
let config: MyCertPluginConfig = serde_json::from_str(&params_str)
    .map_err(|e| /* log and return null */)?;
```

`ox_cert_core` helper:
```rust
pub fn parse_config<T: DeserializeOwned>(raw: *const c_char) -> Result<T, CertError>;
```

### Database Handle Sharing

Each plugin opens its own `CertStore` during `ox_plugin_init`, stores it in `ModuleContext`.
Config must include `store.driver` and either `store.path` (SQLite) or `store.url` (PostgreSQL).
`CertStore::migrate()` is called on every `open()` — safe due to `IF NOT EXISTS` guards.

Use YAML anchors to avoid config duplication:
```yaml
_store: &store
  store.driver: postgresql
  store.url: "postgresql://ca@db:5432/ox_cert"

modules:
  - name: ox_cert_issue
    params: { <<: *store, tenant_id: acme-corp, default_profile: standard }
  - name: ox_cert_revoke
    params: { <<: *store, tenant_id: acme-corp }
```

### Background / Scheduled Plugin Execution

Plugins needing scheduled work (`ox_cert_notify`, CRL pre-generation) spawn a background
thread in `ox_plugin_init`. The thread uses `CertStore` and external APIs directly —
it does NOT use `CoreHostApi` (which is per-request). `ox_plugin_process` returns
`FLOW_CONTROL_CONTINUE` immediately. `ox_plugin_destroy` signals the shutdown flag and
joins the thread.

```rust
pub struct NotifyContext {
    store: Arc<dyn CertStore>,
    config: NotifyConfig,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    _handle: Option<std::thread::JoinHandle<()>>,
}
```

### CRL Caching

`ox_cert_crl` caches generated CRLs in `ModuleContext` behind an `RwLock`:

```rust
struct CachedCrl {
    der: Vec<u8>,
    pem: String,
    generated_at: time::OffsetDateTime,
    next_update: time::OffsetDateTime,
    crl_number: u64,
}
```

On request: if the cached CRL's `next_update` is still in the future, serve from cache.
Otherwise, attempt to acquire the CRL lock. If acquired, regenerate; if not, serve stale
cache with a `Warning` header.

### ACME Nonce Management

```rust
struct NonceStore {
    nonces: std::sync::RwLock<HashMap<String, std::time::Instant>>,
    ttl: std::time::Duration,   // default: 1 hour
}
```

Background thread removes expired nonces every 5 minutes. For multi-node deployments,
`acme.nonce_store: database` switches to a DB-backed nonce table (adds `acme_nonces`
table: `nonce TEXT PK, tenant_id TEXT, expires_at TIMESTAMP`).

### SCEP Encryption Key

SCEP requires a separate RSA encryption key. Generated by `ox_cert_scep` on first init
via `KeyStore::generate_key()` with `key_id = "scep-encryption"`. Separate from the CA
signing chain.
