# ox_cert_est

**Purpose:** Enrollment over Secure Transport server (RFC 7030). HTTPS-native enrollment
for IoT devices, network equipment, and enterprise endpoints.

---

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/.well-known/est/cacerts` | Return CA certificate chain (no auth) |
| `POST` | `/.well-known/est/simpleenroll` | Initial enrollment (CSR submission) |
| `POST` | `/.well-known/est/simplereenroll` | Re-enrollment / renewal |
| `POST` | `/.well-known/est/serverkeygen` | Server-side key generation + enrollment |
| `GET` | `/.well-known/est/csrattrs` | Return required CSR attribute OIDs |
| `POST` | `/.well-known/est/{label}/simpleenroll` | Per-label (profile) enrollment |
| `POST` | `/.well-known/est/{label}/simplereenroll` | Per-label re-enrollment |
| `POST` | `/.well-known/est/{label}/serverkeygen` | Per-label server keygen |

Route registration: `"GET,POST /.well-known/est/*"`.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | All shared types, `KeyStore`, `CertStore`, `CertBuilder`, `CertError` |
| `x509-parser` | Parse incoming PKCS#10 CSR (`application/pkcs10`) |
| `cms` | Wrap response in PKCS#7 CMS `certs-only` |
| `base64` | Base64-encode PKCS#7 response per RFC 7030 §4.1 |
| `uuid` (v4) | Serial generation |
| `serde_json` | Config deserialization |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct EstConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub extensions: ExtensionsConfig,
    /// Require mutual TLS (client certificate) for enrollment endpoints.
    pub require_client_cert: bool,
    /// Allow HTTP Basic auth as fallback (RFC 7030 §3.2.3).
    pub basic_auth_enabled: bool,
    /// OIDs of required CSR attributes, returned by /csrattrs.
    pub csr_attrs: Vec<String>,
    /// Map of label → profile name. Unlabelled requests use "default_profile".
    pub labels: std::collections::HashMap<String, String>,
    pub default_profile: String,
}
```

---

## Authentication

EST endpoints (except `/cacerts`) require authentication:

1. **Mutual TLS (primary):** Client presents a certificate during TLS handshake. The
   `ox_webservice` TLS layer exposes the verified client cert DN in a TaskState field
   (`request.tls.client_cert_dn`). EST plugin reads this as `requester_identity`.
2. **HTTP Basic (fallback):** When `basic_auth_enabled = true`, the plugin reads
   `request.header.Authorization` (Basic scheme) and validates against one-time
   passwords stored in an `est_credentials` table (format: `(tenant_id, username,
   password_hash, expires_at)`). These are provisioned via the admin API.
3. If neither auth method succeeds: return 401 with `WWW-Authenticate` header.

---

## Processing

### `GET /.well-known/est/cacerts`
1. Load root and intermediate cert PEMs.
2. Encode as PKCS#7 `certs-only` CMS message (DER).
3. Base64-encode the DER.
4. Return with `Content-Type: application/pkcs7-mime; smime-type=certs-only`,
   `Content-Transfer-Encoding: base64`.

### `POST /.well-known/est/simpleenroll` (and per-label variant)
1. Authenticate client (mTLS or Basic).
2. Resolve profile from label (default if unlabelled).
3. Decode request body: base64-decoded PKCS#10 CSR DER
   (`Content-Type: application/pkcs10`).
4. Parse and verify CSR. Build `CsrInfo`.
5. Apply `IssuancePolicy::validate_csr`. On violation → 403.
6. Check RA approval requirement. If required and not pre-approved:
   store `ApprovalRequest`; return 202 with `Retry-After: 60` header.
7. Generate UUID serial; build and sign cert via `CertBuilder`.
8. Store `CertificateRecord` with `enrollment_protocol = Est`.
9. Store audit event.
10. Wrap issued cert in PKCS#7 `certs-only` CMS DER; base64-encode.
11. Return 200 with `Content-Type: application/pkcs7-mime; smime-type=certs-only`.

### `POST /.well-known/est/simplereenroll`
Same as `simpleenroll` except:
- Client must present the existing certificate via mTLS.
- Preserve subject and SANs from the existing cert unless overridden in CSR.
- Set `enrollment_protocol = Est`.

### `POST /.well-known/est/serverkeygen`
1. Authenticate. Resolve profile.
2. Generate key pair via `KeyStore::generate_key` (key_id = new UUID serial).
3. Build CSR internally from generated key + SANs in request body.
4. Sign cert as in `simpleenroll`.
5. Encrypt private key: PKCS#8 DER, wrapped in a CMS `EnvelopedData` encrypted with the
   client's public key (extracted from their TLS client cert).
6. Bundle: return a multipart response with cert PKCS#7 + encrypted private key, or
   a single CMS `certs-only` response if the key transport is handled separately.

### `GET /.well-known/est/csrattrs`
Return a PKCS#10 CSR Attributes DER (sequence of OIDs) from `config.csr_attrs`,
base64-encoded. `Content-Type: application/csrattrs`.

---

## Error Cases

| Condition | HTTP | Description |
|---|---|---|
| No client cert and Basic disabled | 401 | Unauthorized |
| Bad Basic credentials | 401 | Unauthorized |
| Invalid CSR | 400 | CSR parse or signature failed |
| Policy violation | 403 | Issuance policy blocked |
| RA approval pending | 202 | `Retry-After` header set |
| CA not ready | 503 | Signing key unavailable |
| Storage failure | 500 | Internal error |
