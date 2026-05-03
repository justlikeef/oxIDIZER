# ox_cert_renew

Renews an existing certificate, preserving subject and SANs, issuing a new serial and a
fresh validity window. Optionally marks the old certificate revoked as `Superseded`.

---

## Phase

`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/certificates/{serial}/renew` | Renew certificate by serial |

---

## Config Reference

```rust
pub struct CertRenewConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub auto_revoke_on_renew: bool,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub extensions: ExtensionsConfig,
}
```

| Field | Default | Description |
|---|---|---|
| `tenant_id` | required | Tenant |
| `auto_revoke_on_renew` | `false` | If `true`, revoke the old cert with reason `Superseded` |
| `ca_intermediate_key_id` | required | Key ID in KeyStore for signing |
| `ca_intermediate_cert_path` | required | Path to intermediate CA PEM |
| `ca_root_cert_path` | required | Path to root CA PEM |

---

## Request Body

```json
{
  "csr":              "-----BEGIN CERTIFICATE REQUEST-----\n...",
  "validity_seconds": 31536000
}
```

Both fields are optional.

- If `csr` is omitted: the existing subject DN and SANs are reused with the same public key.
- If `csr` is provided: its public key may differ from the original (re-key renewal).
  Subject and SANs in the new CSR replace those in the original certificate.
- If `validity_seconds` is omitted: the original cert's validity window length is reused.

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
| Serial not found | 404 | `NOT_FOUND` |
| Certificate already revoked | 409 | `ALREADY_REVOKED` |
| Certificate pending approval | 409 | `INVALID_REQUEST` |
| CA not initialized | 503 | `CA_NOT_READY` |
| Storage failure | 500 | `INTERNAL_ERROR` |

---

## Implementation Notes

- The new certificate receives a fresh UUID v4 serial.
- `enrollment_protocol` is copied from the original certificate record.
- The full issuance pipeline (CertBuilder, chain building, audit event) runs identically
  to `ox_cert_issue` after subject/SAN resolution.
- The `auto_revoke_on_renew = true` option is recommended in production to ensure that
  the old certificate is included in the next CRL.
