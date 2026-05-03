# ox_cert_admin

Administrative API for certificate lifecycle management, CA operations, key rollover,
audit log access, and SCEP/EST credential provisioning.

---

## Phase

`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/certificates` | List/search certificates (paginated, filterable) |
| `GET` | `/api/v1/certificates/{serial}` | Get single cert details + full PEM chain |
| `GET` | `/api/v1/certificates/expiring` | Certs expiring within `?days=N` (default 30) |
| `GET` | `/api/v1/audit` | Audit log (paginated, filterable) |
| `GET` | `/api/v1/ca` | CA info (root + intermediate cert details, rollover status) |
| `POST` | `/api/v1/ca/rollover` | Initiate intermediate CA key rollover |
| `POST` | `/api/v1/ca/rollover/commit` | Commit rollover; retire old key |
| `POST` | `/api/v1/ca/rollover/abort` | Abort rollover; discard new key |
| `GET` | `/api/v1/ca/cross-sign` | List cross-signed certificates |
| `POST` | `/api/v1/ca/cross-sign` | Issue cross-signed cert for external CA |
| `GET` | `/api/v1/ssh/ca` | SSH CA public keys (user + host) |
| `POST` | `/api/v1/scep/challenges` | Provision a SCEP challenge password |
| `GET` | `/api/v1/scep/challenges` | List active (unused, unexpired) SCEP challenges |
| `DELETE` | `/api/v1/scep/challenges/{id}` | Revoke a SCEP challenge |
| `GET` | `/api/v1/tenants` | List tenants (super-admin only) |
| `POST` | `/api/v1/tenants` | Create tenant (super-admin only) |
| `DELETE` | `/api/v1/tenants/{tenant_id}` | Deactivate tenant (super-admin only) |
| `POST` | `/api/v1/est/credentials` | Provision EST HTTP Basic credential |
| `GET` | `/api/v1/est/credentials` | List active EST credentials (hashes only) |
| `DELETE` | `/api/v1/est/credentials/{id}` | Revoke an EST credential |

---

## Config Reference

```rust
pub struct AdminConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub extensions: ExtensionsConfig,
}
```

---

## Certificate List / Search

`GET /api/v1/certificates` accepts query parameters:

| Parameter | Description |
|---|---|
| `subject_cn` | Filter by Common Name |
| `san` | Filter by SAN value |
| `status` | `active`, `revoked`, `expired`, `pending_approval` |
| `profile` | Filter by profile name |
| `not_after_before` | Expiring before this RFC3339 timestamp |
| `not_after_after` | Expiring after this RFC3339 timestamp |
| `enrollment_protocol` | `rest`, `acme`, `est`, `scep`, `ad`, `ssh` |
| `offset`, `limit`, `sort`, `order` | Pagination and sorting |

---

## CA Rollover

The rollover process keeps the CA operational throughout:

1. **Initiate** (`POST /api/v1/ca/rollover`): Generates a new intermediate key pair.
   Signs the new intermediate cert with the root CA key. Stores new key as `Active`;
   marks the old key as `Retiring`. Both keys sign certificates during the transition.
2. **Commit** (`POST /api/v1/ca/rollover/commit`): Marks the retiring key as `Retired`.
   The old intermediate CA is no longer used for issuance.
3. **Abort** (`POST /api/v1/ca/rollover/abort`): Deletes the new key record, restores
   the retiring key to `Active`.

The root CA key must be accessible during rollover initiation. If the root key file was
moved offline after initial setup, it must be temporarily restored.

---

## SCEP Challenge Provisioning

```bash
# Create a challenge password (returned plaintext once; not stored)
curl -X POST https://ca.example.com/api/v1/scep/challenges
# Returns: { "id": "uuid", "password": "A1B2C3...", "expires_at": "..." }

# List active challenges (hashes only, no plaintext)
curl https://ca.example.com/api/v1/scep/challenges

# Revoke a challenge before use
curl -X DELETE https://ca.example.com/api/v1/scep/challenges/{id}
```

Challenge passwords are 16 random alphanumeric characters. They are bcrypt-hashed before
storage. The plaintext is returned only once in the create response.

---

## Tenant Management (super-admin)

Tenant management requires a `super_admin` role enforced by `ox_webservice` permission
management. These endpoints operate across all tenants.

Deactivating a tenant (`DELETE /api/v1/tenants/{tenant_id}`) sets `status = inactive`.
Data rows are preserved — a future background purge removes inactive tenant data.

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| Serial not found | 404 | `NOT_FOUND` |
| Rollover already in progress | 409 | `INVALID_REQUEST` |
| No rollover to commit/abort | 409 | `INVALID_REQUEST` |
| Cross-sign CSR is not a CA request | 400 | `INVALID_CSR` |
| Root key unavailable for rollover | 503 | `CA_NOT_READY` |
| Tenant already exists | 409 | `INVALID_REQUEST` |
| Tenant not found | 404 | `TENANT_NOT_FOUND` |
| Storage failure | 500 | `INTERNAL_ERROR` |
