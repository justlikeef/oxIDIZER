# ox_cert_ocsp

OCSP responder (RFC 6960). Returns the revocation status of queried certificates, signed
by the CA key or a dedicated delegated OCSP signing key.

---

## Phase

`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/ocsp/{encoded-request}` | HTTP GET form (base64url-encoded DER in path) |
| `POST` | `/ocsp` | HTTP POST form (DER request body) |

Route registration: `"GET /ocsp/*,POST /ocsp"`.

---

## Config Reference

```rust
pub struct OcspConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub responder_key_id: String,
    pub delegated_cert_path: Option<String>,
    pub max_age_secs: u64,         // default: 3600
    pub next_update_secs: u64,     // default: 86400 (24 hours)
}
```

| Field | Default | Description |
|---|---|---|
| `responder_key_id` | required | Key ID for OCSP signing. Use `"intermediate"` to sign with the CA key, or a dedicated delegated key ID. |
| `delegated_cert_path` | absent | If set, loads the delegated OCSP signing certificate from this path and includes it in responses |
| `max_age_secs` | `3600` | `Cache-Control: max-age` value in seconds |
| `next_update_secs` | `86400` | `nextUpdate` offset from now in OCSP responses |

---

## Delegated OCSP Responder

When `delegated_cert_path` is set, the signing certificate at that path must have:
- Extended Key Usage: `OCSPSigning` (OID `1.3.6.1.5.5.7.3.9`)
- `id-pkix-ocsp-nocheck` extension (OID `1.3.6.1.5.5.7.48.1.5`) to exempt it from
  revocation checking

The delegated cert is included in the `certs` field of every OCSP response so clients
can verify the delegation chain.

---

## Processing

1. Decode the OCSP request (base64url from path for GET, raw DER body for POST).
2. Parse the request; extract queried serials.
3. For each serial: look up in `CertStore` and determine status (good / revoked / unknown).
4. Build an OCSP response:
   - `responseStatus = successful`
   - `producedAt = now`, `thisUpdate = now`, `nextUpdate = now + next_update_secs`
   - One `SingleResponse` per queried serial
5. Sign the response with `KeyStore::sign()`.
6. Return DER-encoded response with `Content-Type: application/ocsp-response`.

---

## Error Cases

OCSP errors are returned as valid OCSP DER responses (HTTP 200 always, per RFC 6960
§4.2.1):

| Condition | OCSP error | HTTP |
|---|---|---|
| Malformed request DER | `malformedRequest` | 200 |
| Signing key not found | `internalError` | 200 |
| Storage failure | `tryLater` | 200 |

---

## Implementation Notes

- Serial format conversion: OCSP requests encode serials as big-endian DER integers.
  UUID serials are 16 bytes. `ox_cert_core::ocsp::ocsp_serial_to_uuid()` converts the
  wire format to the TEXT UUID stored in the database.
- Nonce extension: if the OCSP request includes a nonce (RFC 6960 §4.4.1), it is copied
  into the response. Configurable via `ocsp.include_nonce` (default: `true`).
- OCSP stapling: certificates issued by `ox_cert_issue` embed the OCSP responder URL in
  their AIA extension, enabling TLS stacks to staple responses automatically.
