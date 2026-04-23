# ox_cert_issue

**Purpose:** Issues a new X.509 certificate from a submitted CSR or an auto-generated key pair.

---

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/certificates` | Issue a new certificate |

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | All shared types, `KeyStore`, `CertStore`, `CertBuilder`, `IssuancePolicy`, `CertError` |
| `x509-parser` | Parse and validate the submitted CSR |
| `ring` | CSR signature verification |
| `uuid` (v4) | Generate UUID v4 serial |
| `serde` / `serde_json` | Request/response serialization |
| `base64` | PEM body decode |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct CertIssueConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub default_profile: String,
    pub policy: IssuancePolicyConfig,
    pub ct: Option<CtConfig>,
    pub extensions: ExtensionsConfig,
    /// Key ID of the intermediate CA signing key.
    pub ca_intermediate_key_id: String,
    /// Path to the intermediate CA cert PEM (for chain building).
    pub ca_intermediate_cert_path: String,
    /// Path to the root CA cert PEM (for chain building).
    pub ca_root_cert_path: String,
}
```

---

## Input TaskState Fields

| Field | Description |
|---|---|
| `request.body` | JSON: `{ csr, profile?, validity_seconds?, key_type?, sans? }` |
| `request.header.Content-Type` | `application/json` or `application/pkcs10` (PEM CSR directly) |
| `cert.ra.approved` | `"true"` if an RA officer pre-approved this request (set by `ox_cert_ra`) |
| `cert.webhook.authorized` | `"true"` if `ox_cert_webhook` ran and authorized (must be present if webhook plugin is in pipeline) |
| `cert.webhook.enrichment` | JSON object of enrichment fields from `ox_cert_webhook` |

### Request Body Schema

```json
{
  "csr":              "-----BEGIN CERTIFICATE REQUEST-----\n...",
  "profile":          "standard",
  "validity_seconds": 31536000,
  "sans":             ["example.com", "www.example.com"],
  "key_type":         "ecc-p256"
}
```

`csr` is required. All other fields are optional; `profile` defaults to `default_profile`
from config. If `sans` is provided alongside a CSR, the provided list overrides the CSR's
SAN extension (enrichment use case). `key_type` is only used when the request body omits
a `csr` and the server generates a key pair on behalf of the caller (server-keygen mode).

---

## Processing

1. Read `request.body`. If `Content-Type: application/pkcs10`, treat body as PEM CSR directly.
2. Parse and DER-decode the CSR using `x509-parser`.
3. Verify the CSR's self-signature.
4. Build `CsrInfo` from parsed CSR fields.
5. Resolve effective profile: use `request.body.profile` if set, else `default_profile`.
6. Load `EnrollmentProfile` for that name; return 400 if unknown.
7. Apply `IssuancePolicy::validate_csr(&csr_info)`. On violation → `PolicyViolation` → 403.
8. If profile has `require_ra_approval`:
   - Check `cert.ra.approved == "true"`. If not set:
     a. Build `ApprovalRequest` with UUID id.
     b. `store.store_ra_request(tenant_id, &request)`.
     c. Write `AuditEvent { action: Issue, ... }` with status pending.
     d. Set `response.status = "202"`, `response.body = {request_id, status: "pending_approval"}`.
     e. Return `FLOW_CONTROL_END`.
9. If `cert.webhook.authorized` is present and `!= "true"` → 403 (webhook already fired
   `FLOW_CONTROL_END`; this is a safety check only).
10. Apply enrichment: if `cert.webhook.enrichment` is set, parse the JSON object and:
    - Append `additional_sans` (string array) to the effective SAN list.
    - Apply `custom_extensions` (array of `{oid, critical, value_hex}`) via `CertBuilder::add_extension`.
    - If `subject_ou` is present, merge it into the `DistinguishedName.organizational_unit`.
11. Build `ValidityPeriod`: `not_before = now`, `not_after = now + effective_validity_seconds`.
12. Generate UUID v4 serial: `uuid::Uuid::new_v4().to_string()`.
13. Build `CertBuilder` from profile, subject (from CSR), SANs, validity.
14. Call `builder.build_and_sign(issuer_cert, key_store, tenant_id, ca_intermediate_key_id, &extensions_config)`.
15. If `ct.enabled`:
    - Call `ox_cert_core::ct::submit_to_ct_logs(tbs_cert_der, issuer_der, &ct_config)`.
    - On failure: if `on_failure = block` → `CtError` → 502. If `warn` → log and continue with empty SCTs.
    - Embed SCTs into the final certificate via the `1.3.6.1.4.1.11129.2.4.2` extension.
16. Build full chain PEM via `ChainBuilder::full_chain_pem`.
17. Construct `CertificateRecord` with `enrollment_protocol = Rest` (or the protocol detected
    from context — set to `Acme`/`Est`/`Scep` by those plugins, which call `ox_cert_issue`
    logic directly via the shared library, not via HTTP).
18. `store.store_cert(tenant_id, &record)`.
19. `store.store_audit_event(tenant_id, &AuditEvent { action: Issue, serial, actor, ... })`.
20. Set `response.status = "201"`, `response.header.Content-Type = "application/json"`.
21. Set `response.body` to JSON response envelope.

---

## Output TaskState Fields

| Field | Value |
|---|---|
| `response.status` | `"201"` |
| `response.body` | JSON: `{ data: { serial, subject_cn, not_before, not_after, profile, certificate, chain, scts[] }, meta: { tenant_id, request_id } }` |
| `cert.issued.serial` | UUID string of the issued cert |
| `cert.issued.not_after` | RFC3339 expiry timestamp |

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| Missing or unparseable `csr` field | 400 | `INVALID_CSR` |
| CSR self-signature invalid | 400 | `INVALID_CSR` |
| Unknown profile | 400 | `INVALID_REQUEST` |
| Policy violation (domain, key strength, SAN count, wildcard) | 403 | `POLICY_VIOLATION` |
| RA approval required but not present | 202 | `RA_APPROVAL_REQUIRED` |
| CA not initialized (key not found) | 503 | `CA_NOT_READY` |
| CT failure with `on_failure = block` | 502 | `CT_FAILURE` |
| Storage failure | 500 | `INTERNAL_ERROR` |

---

## Server-Keygen Mode

When the request body includes `key_type` but no `csr`, the plugin generates a key pair:
1. Generate key via `KeyStore::generate_key(tenant_id, serial, key_type, false)` where
   `serial` is the UUID chosen for this cert (key_id = serial ensures uniqueness).
2. Build a self-signed CSR internally from the generated public key and the SANs/subject
   in the request body.
3. Proceed from step 5 above.
4. Encrypt the private key for storage: derive encryption key via HKDF, encrypt with
   AES-256-GCM, store `base64(nonce||ct||tag)` in `private_key_encrypted`.

Server-keygen mode is required for PKCS#12 export (`ox_cert_p12`). CSR-based certs have
`private_key_encrypted = NULL` — no export is possible.
