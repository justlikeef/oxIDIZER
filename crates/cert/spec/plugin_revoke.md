# ox_cert_revoke

**Purpose:** Revokes a certificate by serial number, recording the reason and timestamp.

---

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/certificates/{serial}/revoke` | Revoke certificate by serial |

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `CertStore`, `RevocationReason`, `AuditEvent`, `CertError` |
| `serde` / `serde_json` | Request/response serialization |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct CertRevokeConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
}
```

---

## Input TaskState Fields

| Field | Description |
|---|---|
| `request.path.serial` | UUID serial of the certificate to revoke |
| `request.body` | JSON: `{ reason }` |

### Request Body Schema

```json
{
  "reason": "key_compromise"
}
```

`reason` must be one of: `unspecified`, `key_compromise`, `ca_compromise`,
`affiliation_changed`, `superseded`, `cessation_of_operation`, `certificate_hold`,
`privilege_withdrawn`. Defaults to `unspecified` if omitted.

---

## Processing

1. Extract `serial` from `request.path.serial`.
2. Parse `reason` from request body; deserialize to `RevocationReason`. Return 400 on
   unknown reason string.
3. `store.get_cert_by_serial(tenant_id, serial)` → 404 if not found.
4. If `status == Revoked` → 409 `ALREADY_REVOKED`.
5. `store.mark_revoked(tenant_id, serial, reason, now)`.
6. `store.store_audit_event(tenant_id, AuditEvent { action: Revoke, serial, actor, details: {reason} })`.
7. Set response fields.

---

## Output TaskState Fields

| Field | Value |
|---|---|
| `response.status` | `"200"` |
| `response.body` | JSON: `{ data: { serial, revoked_at, reason }, meta: { tenant_id, request_id } }` |

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| `serial` not found | 404 | `NOT_FOUND` |
| Already revoked | 409 | `ALREADY_REVOKED` |
| Unknown reason string | 400 | `INVALID_REQUEST` |
| Storage failure | 500 | `INTERNAL_ERROR` |
