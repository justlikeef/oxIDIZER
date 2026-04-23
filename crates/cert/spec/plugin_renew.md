# ox_cert_renew

**Purpose:** Renews an existing certificate, preserving subject and SANs, issuing a new
serial and fresh validity window.

---

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/certificates/{serial}/renew` | Renew certificate by serial |

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | All shared types, `KeyStore`, `CertStore`, `CertBuilder`, `CertError` |
| `x509-parser` | Parse existing cert PEM to extract subject/SANs |
| `uuid` (v4) | Generate new UUID v4 serial |
| `serde` / `serde_json` | Request/response serialization |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct CertRenewConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    /// If true, marks the old certificate revoked with reason Superseded upon renewal.
    pub auto_revoke_on_renew: bool,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub extensions: ExtensionsConfig,
}
```

---

## Input TaskState Fields

| Field | Description |
|---|---|
| `request.path.serial` | UUID serial of the certificate to renew |
| `request.body` | Optional JSON: `{ csr?, validity_seconds? }` |

### Request Body Schema

```json
{
  "csr":              "-----BEGIN CERTIFICATE REQUEST-----\n...",
  "validity_seconds": 31536000
}
```

Both fields are optional. If `csr` is omitted, the existing subject and SANs are reused
with the same public key. If `csr` is provided, it must contain the same public key as
the original certificate (to prove key possession); the subject/SANs in the new CSR
replace those in the original.

---

## Processing

1. Extract `serial` from `request.path.serial`.
2. `store.get_cert_by_serial(tenant_id, serial)` → 404 if not found.
3. If `status == Revoked` → 409 `ALREADY_REVOKED`.
4. If `status == PendingApproval` → 409 with message "Cannot renew a cert pending approval".
5. Parse the existing cert PEM via `x509-parser` to extract subject DN and SANs.
6. If request body contains `csr`: parse and verify it. Confirm public key matches the
   original. If key differs, accept the new key (re-key renewal). Merge SANs from CSR.
7. Resolve validity: use `validity_seconds` from request if provided, else reuse the
   original cert's validity window length.
8. Generate new UUID v4 serial.
9. Build and sign new cert via `CertBuilder` (same flow as `ox_cert_issue` steps 11–16).
10. Construct new `CertificateRecord`. Copy `enrollment_protocol` from original.
11. `store.store_cert(tenant_id, &new_record)`.
12. If `auto_revoke_on_renew`: `store.mark_revoked(tenant_id, old_serial, Superseded, now)`.
13. `store.store_audit_event(...)` with action `Renew`, details include both old and new serial.
14. Set response fields.

---

## Output TaskState Fields

| Field | Value |
|---|---|
| `response.status` | `"201"` |
| `response.body` | Same shape as `ox_cert_issue` response |

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| `serial` not found | 404 | `NOT_FOUND` |
| Certificate already revoked | 409 | `ALREADY_REVOKED` |
| Submitted CSR key does not match original (strict re-key disabled) | 400 | `INVALID_CSR` |
| CA not initialized | 503 | `CA_NOT_READY` |
| Storage failure | 500 | `INTERNAL_ERROR` |
