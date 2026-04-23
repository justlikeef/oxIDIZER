# ox_cert_ad_autoenroll

**Purpose:** Active Directory certificate auto-enrollment via MS CEP/CES (Certificate
Enrollment Policy/Service) protocols. Allows domain-joined Windows machines and AD users
to automatically enroll for certificates using Group Policy.

---

## Implementation Status

**Interface: fully specified. Implementation: partially deferred.**

| Authentication Mode | Status |
|---|---|
| Client certificate (mTLS) | Implementable — uses existing TLS stack |
| Kerberos/SPNEGO | **Deferred** — requires a dedicated FFI spike against `libgssapi` / MIT-krb5 |

The YAML config flag `ad.auth_mode: client_cert` enables the implementable path.
`ad.auth_mode: kerberos` will be unavailable until the Kerberos FFI implementation is
complete. Setting it logs an error and causes `ox_plugin_init` to return null.

---

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/certsrv/mscep/mscep.dll` | CEP: SCEP policy endpoint (queries capabilities) |
| `POST` | `/certsrv/mscep/mscep.dll` | SCEP enrollment via CEP |
| `GET` | `/certsrv/mscep_admin/mscep.dll` | Admin SCEP endpoint (challenge password mgmt) |
| `GET` | `/certsrv/CertEnroll/{template}.json` | CES: enrollment policy (JSON, for modern clients) |
| `POST` | `/certsrv/CertEnroll/{template}` | CES: CSR submission and certificate issuance |
| `GET` | `/.well-known/est/{label}/cacerts` | Alternative EST-based AD enrollment (RFC 7030) |

Route registration: `"GET,POST /certsrv/*"`.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `KeyStore`, `CertStore`, `CertBuilder`, `CertError` |
| `x509-parser` | Parse CSRs from CEP/CES requests |
| `cms` | Wrap SCEP responses in CMS |
| `uuid` (v4) | Serial generation |
| `serde` / `serde_json` | Config and JSON policy response serialization |
| `base64` | Encode/decode SCEP message bodies |
| `quick-xml` | Parse/generate SOAP envelopes for CES WS-Trust |
| `ring` | Cryptographic operations |

**Kerberos/SPNEGO (deferred):**
| `libgssapi` (FFI) | GSSAPI token negotiation — implementation deferred |
| `mit-krb5` (FFI) | Kerberos ticket validation — implementation deferred |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct AdAutoenrollConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub extensions: ExtensionsConfig,
    /// Authentication mode. "client_cert" is implemented; "kerberos" is deferred.
    pub auth_mode: AdAuthMode,
    /// Active Directory domain (e.g., "corp.example.com").
    pub domain: String,
    /// LDAP URI for identity validation (optional; used with Kerberos mode).
    pub ldap_uri: Option<String>,
    pub templates: Vec<AdTemplate>,
    pub scep_challenge_ttl: std::time::Duration,   // default: 1h
}

#[derive(Debug, Deserialize)]
pub enum AdAuthMode { ClientCert, Kerberos }

#[derive(Debug, Deserialize)]
pub struct AdTemplate {
    pub name: String,
    /// Microsoft template OID (e.g., "1.3.6.1.4.1.311.21.8.1").
    pub oid: String,
    pub key_type: String,          // "rsa-2048" | "ecc-p256"
    pub validity: String,          // e.g., "1y"
    /// AD group whose members are allowed to enroll for this template.
    pub autoenroll_group: String,
    /// ox_cert_core profile to use for this template.
    pub profile: String,
}
```

---

## Authentication (Client Certificate Mode)

When `auth_mode = client_cert`:
1. `ox_webservice` TLS layer enforces client certificate requirement for `/certsrv/*`.
2. The verified client cert DN is available in TaskState as
   `request.tls.client_cert_dn`.
3. The plugin uses this DN as `requester_identity` for policy and audit.
4. Group membership validation is not enforced in this mode (the client cert itself
   is the authorization token). Group-based enforcement requires Kerberos mode.

---

## Authentication (Kerberos Mode — Deferred)

When `auth_mode = kerberos` (not yet implemented):

The plugin would:
1. Check `request.header.Authorization` for `Negotiate` scheme.
2. Extract the SPNEGO token (base64-encoded GSSAPI token).
3. Pass to `gss_accept_sec_context()` via `libgssapi` FFI.
4. On success: extract the client's Kerberos principal (e.g., `machine$@CORP.EXAMPLE.COM`).
5. Query LDAP to validate group membership for the requested template.
6. Proceed with enrollment.

**Implementation note:** This requires a Kerberos keytab file for the service principal
(e.g., `HTTP/certsrv.example.com@CORP.EXAMPLE.COM`) accessible at startup. The keytab
path would be added to config as `ad.keytab_path`. A dedicated implementation spike is
required before starting this work.

---

## CEP: Certificate Enrollment Policy

### `GET /certsrv/mscep/mscep.dll` (GetCACaps / GetCACert)

Delegates to the SCEP operation logic (same as `ox_cert_scep`). Supported capabilities:
`POSTPKIOperation`, `SHA-256`, `AES`, `Renewal`.

### Enrollment Policy JSON (`GET /certsrv/CertEnroll/{template}.json`)

Returns a JSON enrollment policy document listing available templates:

```json
{
  "policyId": "{template-oid}",
  "friendlyName": "Machine",
  "hashAlgorithmList": ["SHA256"],
  "keySpec": 1,
  "minimalKeyLength": 2048,
  "policyAttributes": {
    "templateOID": "1.3.6.1.4.1.311.21.8.1",
    "validityPeriod": "P1Y"
  }
}
```

---

## CES: Certificate Enrollment Service

### `POST /certsrv/CertEnroll/{template}` (CSR submission)

1. Authenticate client.
2. Parse template name from path; look up `AdTemplate` in config.
3. Parse request body: SOAP envelope containing a base64-encoded PKCS#10 CSR or
   PKCS#7 signed request.
4. Extract CSR DER. Verify.
5. Apply template policy: key type, key size, validity.
6. Issue cert via `CertBuilder` with profile from template config.
7. Add Microsoft-specific extensions:
   - Template OID extension (OID `1.3.6.1.4.1.311.21.7`): encodes template name + OID.
   - Application Policies extension (OID `1.3.6.1.4.1.311.21.10`): maps to EKU.
8. Store `CertificateRecord` with `enrollment_protocol = Ad`.
9. Store audit event.
10. Wrap cert in SOAP response envelope. Return 200.

---

## Data Shapes

### `AdEnrollmentRequest` (internal, not stored)

```rust
struct AdEnrollmentRequest {
    template_name: String,
    csr_der: Vec<u8>,
    requester_identity: String,
    auth_mode_used: AdAuthMode,
}
```

---

## Error Cases

| Condition | HTTP | Behaviour |
|---|---|---|
| `auth_mode = kerberos` at startup | — | `ox_plugin_init` returns null |
| No client cert (client_cert mode) | 401 | SOAP fault: `AuthenticationFailure` |
| Unknown template in path | 404 | SOAP fault: `PolicyNotFound` |
| Invalid CSR | 400 | SOAP fault: `InvalidArgument` |
| Policy violation | 403 | SOAP fault: `AccessDenied` |
| CA not ready | 503 | SOAP fault: `SystemError` |
| Storage failure | 500 | SOAP fault: `SystemError` |

SOAP faults follow the WS-Trust error format. HTTP-level errors are used for
non-SOAP requests (e.g., the JSON policy endpoint).

---

## Kerberos Implementation Spike Checklist

Before implementing Kerberos mode, the following must be resolved:

- [ ] Evaluate `libgssapi` crate for Rust FFI bindings to `libgssapi_krb5.so`.
- [ ] Confirm service principal keytab setup procedure for `ox_webservice` deployments.
- [ ] Evaluate whether `ldap3` crate is sufficient for group membership queries.
- [ ] Confirm that SPNEGO token negotiation can be performed in a blocking Rust context
      (no async required in the plugin ABI).
- [ ] Test against a Windows Server 2022 Active Directory with Group Policy auto-enrollment.
