use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Key types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyType {
    Rsa2048,
    Rsa3072,
    Rsa4096,
    EcP256,
    EcP384,
    EcP521,
    Ed25519,
}

// ---------------------------------------------------------------------------
// Certificate types
// ---------------------------------------------------------------------------

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
    Unspecified          = 0,
    KeyCompromise        = 1,
    CaCompromise         = 2,
    AffiliationChanged   = 3,
    Superseded           = 4,
    CessationOfOperation = 5,
    CertificateHold      = 6,
    RemoveFromCrl        = 8,
    PrivilegeWithdrawn   = 9,
    AaCompromise         = 10,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnrollmentProtocol {
    Rest,
    Acme,
    Est,
    Scep,
    Ad,
    Ssh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sct {
    pub log_id: String,
    pub log_name: String,
    pub timestamp: time::OffsetDateTime,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateRecord {
    pub serial: String,
    pub tenant_id: String,
    pub subject_cn: String,
    pub subject_dn: String,
    pub sans: Vec<String>,
    pub issuer_dn: String,
    pub not_before: time::OffsetDateTime,
    pub not_after: time::OffsetDateTime,
    pub key_type: String,
    pub profile: String,
    pub pem: String,
    pub csr_pem: Option<String>,
    pub private_key_encrypted: Option<String>,
    pub status: CertStatus,
    pub revoked_at: Option<time::OffsetDateTime>,
    pub revocation_reason: Option<RevocationReason>,
    pub scts: Vec<Sct>,
    pub policy_oids: Vec<String>,
    pub enrollment_protocol: Option<EnrollmentProtocol>,
    pub created_at: time::OffsetDateTime,
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
    pub limit: u64,
}

impl Default for Pagination {
    fn default() -> Self {
        Self { offset: 0, limit: 50 }
    }
}

#[derive(Debug, Clone)]
pub struct PagedResult<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

// ---------------------------------------------------------------------------
// SSH types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshCertType {
    User,
    Host,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshCertRecord {
    pub serial: u64,
    pub tenant_id: String,
    pub cert_type: SshCertType,
    pub key_id: String,
    pub principals: Vec<String>,
    pub public_key: String,
    pub signing_key_fingerprint: String,
    pub valid_after: time::OffsetDateTime,
    pub valid_before: time::OffsetDateTime,
    pub critical_options: HashMap<String, String>,
    pub extensions: HashMap<String, String>,
    pub certificate: String,
    pub created_at: time::OffsetDateTime,
}

// ---------------------------------------------------------------------------
// CA Key types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaKeyStatus {
    Active,
    Retiring,
    Retired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameConstraints {
    pub permitted_dns: Vec<String>,
    pub excluded_dns: Vec<String>,
    pub permitted_ip: Vec<String>,
    pub excluded_ip: Vec<String>,
    pub permitted_email: Vec<String>,
    pub excluded_email: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaKeyRecord {
    pub id: String,
    pub tenant_id: String,
    pub key_type: KeyType,
    pub cert_pem: String,
    pub key_ref: String,
    pub status: CaKeyStatus,
    pub not_before: time::OffsetDateTime,
    pub not_after: time::OffsetDateTime,
    pub name_constraints: Option<NameConstraints>,
    pub path_length: Option<u32>,
    pub created_at: time::OffsetDateTime,
}

// ---------------------------------------------------------------------------
// ACME types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcmeAccountStatus {
    Valid,
    Deactivated,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcmeAccount {
    pub id: String,
    pub tenant_id: String,
    pub jwk: String,
    pub contact: Vec<String>,
    pub status: AcmeAccountStatus,
    pub eab_kid: Option<String>,
    pub created_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcmeOrderStatus {
    Pending,
    Ready,
    Processing,
    Valid,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcmeIdentifier {
    pub identifier_type: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcmeOrder {
    pub id: String,
    pub tenant_id: String,
    pub account_id: String,
    pub status: AcmeOrderStatus,
    pub identifiers: Vec<AcmeIdentifier>,
    pub not_before: Option<time::OffsetDateTime>,
    pub not_after: Option<time::OffsetDateTime>,
    pub certificate_serial: Option<String>,
    pub expires: time::OffsetDateTime,
    pub created_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcmeAuthzStatus {
    Pending,
    Valid,
    Invalid,
    Deactivated,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChallengeType {
    Http01,
    Dns01,
    TlsAlpn01,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcmeChallengeStatus {
    Pending,
    Processing,
    Valid,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcmeChallenge {
    pub id: String,
    pub challenge_type: ChallengeType,
    pub token: String,
    pub status: AcmeChallengeStatus,
    pub validated_at: Option<time::OffsetDateTime>,
    pub error: Option<String>,
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

// ---------------------------------------------------------------------------
// RA types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub tenant_id: String,
    pub csr_pem: String,
    pub requester_identity: String,
    pub profile: String,
    pub sans: Vec<String>,
    pub status: ApprovalStatus,
    pub reviewer: Option<String>,
    pub review_notes: Option<String>,
    pub reviewed_at: Option<time::OffsetDateTime>,
    pub certificate_serial: Option<String>,
    pub created_at: time::OffsetDateTime,
}

// ---------------------------------------------------------------------------
// SCEP types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScepChallenge {
    pub id: String,
    pub tenant_id: String,
    pub password_hash: String,
    pub used: bool,
    pub expires_at: time::OffsetDateTime,
    pub created_at: time::OffsetDateTime,
}

// ---------------------------------------------------------------------------
// Notification types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationChannel {
    Webhook,
    Mqtt,
    Email,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationStatus {
    Sent,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationRecord {
    pub id: i64,
    pub tenant_id: String,
    pub serial: String,
    pub threshold_days: u32,
    pub channel: NotificationChannel,
    pub sent_at: time::OffsetDateTime,
    pub status: NotificationStatus,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Audit types
// ---------------------------------------------------------------------------

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,
    pub tenant_id: String,
    pub timestamp: time::OffsetDateTime,
    pub action: AuditAction,
    pub serial: Option<String>,
    pub actor: String,
    pub details: serde_json::Value,
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
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum SanType {
    Dns(String),
    Ip(IpAddr),
    Email(String),
    Uri(String),
}

// ---------------------------------------------------------------------------
// Validity / Extension types
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
    pub value: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Enrollment profile / policy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentProfile {
    pub name: String,
    pub validity_seconds: u64,
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

#[derive(Debug, Clone, Deserialize)]
pub struct IssuancePolicyConfig {
    pub domain_allowlist: Vec<String>,
    pub domain_blocklist: Vec<String>,
    pub max_san_count: usize,
    pub wildcard_allowed: bool,
    pub min_rsa_bits: u32,
    pub require_ra_approval: bool,
}

/// Runtime form of IssuancePolicy — regex objects are not serializable.
pub struct IssuancePolicy {
    pub domain_allowlist: Vec<regex::Regex>,
    pub domain_blocklist: Vec<regex::Regex>,
    pub max_san_count: usize,
    pub wildcard_allowed: bool,
    pub min_rsa_bits: u32,
    pub require_ra_approval: bool,
}

impl IssuancePolicy {
    pub fn from_config(cfg: &IssuancePolicyConfig) -> Result<Self, regex::Error> {
        let allowlist = cfg.domain_allowlist.iter()
            .map(|s| regex::Regex::new(s))
            .collect::<Result<Vec<_>, _>>()?;
        let blocklist = cfg.domain_blocklist.iter()
            .map(|s| regex::Regex::new(s))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            domain_allowlist: allowlist,
            domain_blocklist: blocklist,
            max_san_count: cfg.max_san_count,
            wildcard_allowed: cfg.wildcard_allowed,
            min_rsa_bits: cfg.min_rsa_bits,
            require_ra_approval: cfg.require_ra_approval,
        })
    }

    /// Validate a CSR against this policy. Returns an error message string on violation.
    pub fn validate_csr(&self, csr: &CsrInfo) -> Result<(), String> {
        if csr.sans.len() > self.max_san_count {
            return Err(format!(
                "SAN count {} exceeds policy maximum {}",
                csr.sans.len(),
                self.max_san_count
            ));
        }

        for san in &csr.sans {
            if let SanType::Dns(name) = san {
                if !self.wildcard_allowed && name.starts_with("*.") {
                    return Err(format!("wildcard SAN '{}' not permitted by policy", name));
                }
                for re in &self.domain_blocklist {
                    if re.is_match(name) {
                        return Err(format!("domain '{}' is blocked by policy", name));
                    }
                }
                if !self.domain_allowlist.is_empty() {
                    let allowed = self.domain_allowlist.iter().any(|re| re.is_match(name));
                    if !allowed {
                        return Err(format!("domain '{}' not in policy allowlist", name));
                    }
                }
            }
        }

        // RSA minimum key size
        if matches!(csr.key_type, KeyType::Rsa2048 | KeyType::Rsa3072 | KeyType::Rsa4096)
            && csr.key_bits < self.min_rsa_bits
        {
            return Err(format!(
                "RSA key size {} bits is below policy minimum {}",
                csr.key_bits, self.min_rsa_bits
            ));
        }

        Ok(())
    }
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
// CA Key set (active + retiring during rollover)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CaKeySet {
    pub tenant_id: String,
    pub active: CaKeyRecord,
    pub retiring: Option<CaKeyRecord>,
}

// ---------------------------------------------------------------------------
// KeyStore config types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct KeyStoreConfig {
    pub store_type: KeyStoreType,
    pub key_dir: Option<PathBuf>,
    pub passphrase_env: Option<String>,
    pub pkcs11_module: Option<PathBuf>,
    pub pkcs11_slot: Option<u64>,
    pub pkcs11_pin_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyStoreType {
    Software,
    Pkcs11,
}

#[derive(Debug, Clone)]
pub enum SigningAlgorithm {
    Sha256WithRsa,
    Sha384WithRsa,
    Sha512WithRsa,
    EcdsaWithSha256,
    EcdsaWithSha384,
    EcdsaWithSha512,
    Ed25519,
}

#[derive(Debug, Clone)]
pub struct KeyInfo {
    pub key_id: String,
    pub tenant_id: String,
    pub key_type: KeyType,
    pub created_at: time::OffsetDateTime,
}

// ---------------------------------------------------------------------------
// CertStore config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct CertStoreConfig {
    pub driver: String,
    pub path: Option<String>,
    pub url: Option<String>,
}

impl CertStoreConfig {
    pub fn db_path(&self) -> &str {
        self.path.as_deref().unwrap_or("/var/lib/ox_webservice/cert.db")
    }
}

// ---------------------------------------------------------------------------
// CT types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct CtConfig {
    pub enabled: bool,
    pub logs: Vec<CtLogConfig>,
    pub min_scts: usize,
    pub timeout_secs: u64,
    pub on_failure: CtFailureMode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CtLogConfig {
    pub name: String,
    pub url: String,
    pub public_key_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CtFailureMode {
    Block,
    Warn,
}

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
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub message: Option<String>,
}
