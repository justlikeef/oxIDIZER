# ox_cert_ocsp

**Purpose:** OCSP responder (RFC 6960). Returns the revocation status of queried
certificates, signed by the CA or a delegated OCSP responder key.

---

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/ocsp/{encoded-request}` | HTTP GET form (base64url-encoded DER request in path) |
| `POST` | `/ocsp` | HTTP POST form (DER request in body) |

The route registration string `"GET /ocsp/*,POST /ocsp"` covers both forms.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `KeyStore`, `CertStore`, `CertError` |
| `x509-parser` | Decode OCSP request DER |
| `rcgen` | Build OCSP response and sign |
| `ring` | ECDSA/RSA signing for response |
| `base64` | URL-decode path parameter for GET form |
| `serde_json` | Config deserialization |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct OcspConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    /// Key ID of the OCSP signing key. Use "intermediate" to sign with the intermediate
    /// CA key directly, or provide a dedicated delegated OCSP signing key ID.
    pub responder_key_id: String,
    /// If set, this is a delegated OCSP responder: the cert at this path is included in
    /// the response to let clients verify the delegation chain.
    pub delegated_cert_path: Option<String>,
    /// Cache-Control max-age for OCSP responses in seconds (default: 3600).
    pub max_age_secs: u64,
    /// nextUpdate offset from now in seconds (default: 86400 = 24 hours).
    pub next_update_secs: u64,
}
```

### Delegated OCSP Responder

When `delegated_cert_path` is set, `ox_cert_ca_init` (or the admin API) must have
previously issued a certificate with:
- Extended Key Usage: `OCSPSigning` (OID 1.3.6.1.5.5.7.3.9)
- The `id-pkix-ocsp-nocheck` extension (OID 1.3.6.1.5.5.7.48.1.5) to exempt it from
  revocation checking.

The delegated cert's private key is stored in `KeyStore` under `responder_key_id`. The
cert PEM is loaded from `delegated_cert_path` at init time.

---

## Input TaskState Fields

| Field | Description |
|---|---|
| `request.method` | `GET` or `POST` |
| `request.path` | Full path (for GET: last segment is base64url-encoded OCSP request) |
| `request.body` | DER-encoded OCSP request (POST only) |
| `request.header.Content-Type` | `application/ocsp-request` (POST) |

---

## Processing

1. Determine input form: GET (decode path segment from base64url) or POST (read body DER).
2. Parse OCSP request DER using `x509-parser`. Extract queried serial(s) (may be batch).
3. For each queried serial:
   a. Convert from OCSP wire format to UUID string using
      `ox_cert_core::ocsp::ocsp_serial_to_uuid(serial_bytes)`. Return a
      `malformedRequest` OCSP error if conversion fails (serial is not 16 bytes).
   b. `store.get_cert_by_serial(tenant_id, &uuid_str)`.
   c. If not found: status = `unknown`.
   c. If found and `status == Revoked`: status = `revoked`, include `revoked_at` and
      `revocation_reason` in the OCSP single response.
   d. Otherwise: status = `good`.
4. Build OCSP response:
   - `responseStatus = successful`
   - `producedAt = now`
   - `thisUpdate = now`
   - `nextUpdate = now + next_update_secs`
   - Each `SingleResponse` carries the status and timestamps.
5. Sign the response with `KeyStore::sign(tenant_id, responder_key_id, algorithm, tbs)`.
6. If delegated responder: include the delegated cert in the `certs` field of the response.
7. Set response DER bytes via `set_field_bytes` on the task context.
8. Set `response.header.Content-Type = "application/ocsp-response"`.
9. Set `response.header.Cache-Control = "max-age={max_age_secs}"`.
10. Set `response.status = "200"`.

---

## Output TaskState Fields

| Field | Value |
|---|---|
| `response.status` | `"200"` |
| `response.body` (binary) | DER-encoded OCSP response |
| `response.header.Content-Type` | `application/ocsp-response` |
| `response.header.Cache-Control` | `max-age={max_age_secs}` |

---

## Error Cases

| Condition | HTTP | Behaviour |
|---|---|---|
| Malformed OCSP request DER | 400 | Return `malformedRequest` OCSP error response |
| Signing key not found | 503 | Return `internalError` OCSP error response |
| Storage failure | 500 | Return `tryLater` OCSP error response |

OCSP errors are themselves valid OCSP responses with `responseStatus ≠ successful`; they
are always returned with HTTP 200 per RFC 6960 §4.2.1.

---

## Notes

- Serial number format: OCSP requests encode serials as big-endian DER integers. UUID
  serials are exactly 16 bytes in binary form. `ox_cert_core::ocsp::ocsp_serial_to_uuid`
  converts to the TEXT UUID format stored in the database.
  `ox_cert_core::ocsp::uuid_to_ocsp_serial` converts back for building OCSP responses.
- Nonce extension (RFC 6960 §4.4.1): If the request contains an OCSP nonce, copy it
  into the response. Configurable: `ocsp.include_nonce: true` (default).
- OCSP stapling: Certificates issued by `ox_cert_issue` embed the OCSP responder URL
  in their AIA extension, enabling TLS stacks to staple the response automatically.
