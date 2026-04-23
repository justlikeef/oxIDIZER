# ox_cert_scep

**Purpose:** Simple Certificate Enrollment Protocol server (RFC 8894). Supports network
equipment (Cisco, Juniper, etc.) and MDM platforms that use SCEP for certificate enrollment.

---

## Phase
`Content`

## Routes

| Method | Path | Query | Description |
|---|---|---|---|
| `GET` | `/scep` | `operation=GetCACert` | Return CA certificate(s) |
| `GET` | `/scep` | `operation=GetCACaps` | Return CA capabilities |
| `POST` | `/scep` | `operation=PKIOperation` | Process PKCS#7 enrollment envelope |
| `GET` | `/scep` | `operation=GetNextCACert` | Return next CA cert during rollover |

Route registration: `"GET,POST /scep"`. The plugin dispatches internally on the
`operation` query parameter.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | All shared types, `KeyStore`, `CertStore`, `CertBuilder`, `CertError` |
| `cms` | CMS SignedData and EnvelopedData parsing and construction |
| `x509-parser` | Parse PKCS#10 CSR from unwrapped CMS envelope |
| `ring` | RSA decryption (EnvelopedData), PKCS#1 v1.5 signing for response |
| `sha2` | SHA-256 challenge password hashing |
| `bcrypt` | Challenge password hashing at rest |
| `uuid` (v4) | Serial generation |
| `base64` | Decode/encode SCEP message bodies |
| `serde_json` | Config deserialization |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct ScepConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub extensions: ExtensionsConfig,
    /// TTL for challenge passwords (default: 1h).
    pub challenge_ttl: std::time::Duration,
    /// Symmetric encryption algorithm for CMS EnvelopedData responses.
    pub encryption_algorithm: ScepEncryptionAlgorithm,
    /// Signing algorithm for SCEP responses.
    pub signing_algorithm: ScepSigningAlgorithm,
    /// Key ID for the SCEP RA encryption key (not the CA signing key).
    /// Generated automatically on first init if missing.
    pub encryption_key_id: String,
    pub encryption_key_type: KeyType,  // default: Rsa2048
}

#[derive(Debug, Deserialize)]
pub enum ScepEncryptionAlgorithm { Aes256Cbc, TripleDes }

#[derive(Debug, Deserialize)]
pub enum ScepSigningAlgorithm { Sha256, Sha512 }
```

---

## Encryption Key Initialization

On `ox_plugin_init`, the plugin checks
`key_store.key_exists(tenant_id, &config.encryption_key_id)`. If missing, it calls
`key_store.generate_key(tenant_id, &config.encryption_key_id, Rsa2048, false)`.
This key is the RA encryption key used to unwrap client-encrypted `EnvelopedData` —
it is completely separate from the CA signing key chain.

---

## Processing (by operation)

### `GetCACert`

1. Load intermediate CA cert PEM from `ca_intermediate_cert_path`.
2. If a retiring CA cert exists (`CaKeySet.retiring`), return a degenerate PKCS#7
   `certs-only` CMS containing both certs (per RFC 8894 §4.1.2).
3. Otherwise return the single intermediate cert DER.
4. `Content-Type: application/x-x509-ca-cert` (single cert) or
   `application/x-x509-ca-ra-cert` (chain).

### `GetCACaps`

Return a newline-separated list of supported capabilities:
```
POSTPKIOperation
SHA-256
SHA-512
AES
DES3
Renewal
GetNextCACert
```
`Content-Type: text/plain`.

### `PKIOperation` (POST)

1. Base64-decode the request body to get the SCEP message DER.
2. Parse outer CMS `SignedData`. Verify signature against the client's self-signed cert
   included in the message (initial enrollment) or the existing client cert (renewal).
3. Decrypt inner CMS `EnvelopedData` using the SCEP encryption key
   (`KeyStore::sign` with the RA encryption key in decrypt mode, or via a direct
   `KeyStore` RSA decryption call — to be resolved at implementation time based on
   `KeyStore` extension for raw RSA decrypt).
4. Extract the inner `PKCSReq` (PKCS#10 CSR).
5. Validate the challenge password:
   a. Hash the presented password.
   b. `store.consume_scep_challenge(tenant_id, &password_hash)`. Returns `true` if a
      matching, unexpired, unconsumed challenge exists (and atomically marks it used).
   c. If `false`: return a SCEP failure response (`badRequest`).
6. Parse and verify the CSR.
7. Apply `IssuancePolicy::validate_csr`. On violation: return SCEP failure response.
8. Generate UUID serial; build and sign cert via `CertBuilder`.
9. Store `CertificateRecord` with `enrollment_protocol = Scep`.
10. Store audit event.
11. Wrap issued cert in a PKCS#7 CMS `SignedData` wrapping a PKCS#7 `EnvelopedData`
    (encrypted with the client's public key from the CSR).
12. Sign the outer `SignedData` with the CA intermediate key.
13. Return the DER-encoded CMS message with
    `Content-Type: application/x-pki-message`.

### `GetNextCACert`

1. Load the active `CaKeySet` from `CertStore`.
2. If `retiring` is Some: return the new (active) CA cert — this is the "next" cert
   during rollover.
3. If `retiring` is None: return the current active CA cert.
4. Wrap in PKCS#7 `certs-only`.

---

## Challenge Password Lifecycle

Challenge passwords are provisioned via the `ox_cert_admin` API:
`POST /api/v1/scep/challenges` (see plugin_admin.md). The admin endpoint:
1. Generates a random one-time password (16 printable chars).
2. Hashes it with bcrypt (work factor 12).
3. Stores `ScepChallenge { id, tenant_id, password_hash, used: false, expires_at }`.
4. Returns the plaintext password to the admin caller (only time it is visible).

---

## Error Cases

| Condition | HTTP | SCEP Response |
|---|---|---|
| Invalid CMS outer signature | 400 | `failInfo: badRequest` |
| Bad or expired challenge password | 400 | `failInfo: badRequest` |
| Invalid CSR | 400 | `failInfo: badRequest` |
| Policy violation | 403 | `failInfo: badRequest` |
| CA not ready | 503 | `failInfo: systemFailure` |
| Storage failure | 500 | `failInfo: systemFailure` |

SCEP failures are CMS-signed `failInfo` responses with HTTP 200 (SCEP convention).
The HTTP status codes above apply only to non-SCEP protocol errors (e.g., malformed body
before CMS parsing).
