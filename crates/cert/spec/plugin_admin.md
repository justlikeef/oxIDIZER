# ox_cert_admin

**Purpose:** Administrative API for certificate lifecycle management, CA operations,
key rollover, audit log access, and SCEP challenge provisioning.

---

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/certificates` | List/search certificates (paginated, filterable) |
| `GET` | `/api/v1/certificates/{serial}` | Get single cert details + full PEM chain |
| `GET` | `/api/v1/certificates/expiring` | Certs expiring within `?days=N` (default 30) |
| `GET` | `/api/v1/audit` | Audit log with pagination and filtering |
| `GET` | `/api/v1/ca` | CA info (root + intermediate cert details, rollover status) |
| `POST` | `/api/v1/ca/rollover` | Initiate intermediate CA key rollover |
| `POST` | `/api/v1/ca/rollover/commit` | Commit rollover; retire old key |
| `POST` | `/api/v1/ca/rollover/abort` | Abort rollover; discard new key |
| `GET` | `/api/v1/ca/cross-sign` | List cross-signed certificates |
| `POST` | `/api/v1/ca/cross-sign` | Issue cross-signed cert for external CA |
| `GET` | `/api/v1/ssh/ca` | SSH CA public keys (user + host) |
| `POST` | `/api/v1/scep/challenges` | Provision a SCEP challenge password |
| `GET` | `/api/v1/scep/challenges` | List active (unused, unexpired) SCEP challenges |
| `DELETE` | `/api/v1/scep/challenges/{id}` | Revoke a SCEP challenge before use |
| `GET` | `/api/v1/tenants` | List tenants (super-admin only) |
| `POST` | `/api/v1/tenants` | Create tenant (super-admin only) |
| `DELETE` | `/api/v1/tenants/{tenant_id}` | Deactivate tenant (super-admin only) |
| `POST` | `/api/v1/est/credentials` | Provision EST HTTP Basic one-time credential |
| `GET` | `/api/v1/est/credentials` | List active EST credentials (hashes only, no plaintext) |
| `DELETE` | `/api/v1/est/credentials/{id}` | Revoke an EST credential before use |

Route registration: `"GET,POST,DELETE /api/v1/certificates,GET,POST /api/v1/audit,GET,POST /api/v1/ca,GET /api/v1/ssh,GET,POST,DELETE /api/v1/scep,GET,POST,DELETE /api/v1/tenants"`.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `KeyStore`, `CertStore`, all types, `CertBuilder`, `CertError` |
| `x509-parser` | Parse CA cert PEM for rollover info and cross-signing |
| `rcgen` | Issue cross-signed certificates |
| `serde` / `serde_json` | Request/response serialization |
| `uuid` (v4) | SCEP challenge IDs, cross-cert serials |
| `rand` | Generate SCEP challenge passwords |
| `bcrypt` | Hash SCEP challenge passwords at rest |
| `time` | Expiry computation |

---

## Config

```rust
#[derive(Debug, Deserialize)]
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

## Processing (by endpoint)

### `GET /api/v1/certificates`

Query parameters:
- `subject_cn`, `san`, `status`, `profile`, `not_after_before`, `not_after_after`,
  `enrollment_protocol`, `offset`, `limit`, `sort`, `order`

1. Build `CertFilter` from query params.
2. `store.list_certs(tenant_id, &filter, &pagination)`.
3. Return paginated JSON response.

### `GET /api/v1/certificates/{serial}`

1. `store.get_cert_by_serial(tenant_id, serial)` → 404 if not found.
2. Build full chain PEM via `ChainBuilder`.
3. Return JSON with `certificate` (leaf PEM), `chain` (full PEM), all metadata fields.

### `GET /api/v1/certificates/expiring`

1. Parse `?days=N` (default 30; max 365).
2. `store.list_expiring(tenant_id, days)`.
3. Return paginated list.

### `GET /api/v1/audit`

Query parameters: `action`, `serial`, `actor`, `from`, `to`, `offset`, `limit`.

1. Build `AuditFilter`. `store.get_audit_log(tenant_id, &filter, &pagination)`.
2. Return paginated JSON.

### `GET /api/v1/ca`

1. `store.get_active_ca_key(tenant_id)` → load intermediate `CaKeyRecord`.
2. Parse intermediate + root cert PEMs.
3. Return:
   ```json
   {
     "intermediate": {
       "subject_dn": "...", "not_before": "...", "not_after": "...",
       "key_type": "ecc-p384", "status": "active", "days_until_expiry": 3287
     },
     "root": { ... },
     "rollover_active": false
   }
   ```

### `POST /api/v1/ca/rollover`

Initiates dual-signing rollover:

1. Assert no rollover already in progress (`store.get_active_ca_key` status check).
2. Generate new intermediate key:
   `key_store.generate_key(tenant_id, new_key_id, key_type, false)`.
3. Build new intermediate CA cert signed by root CA key.
4. Store new `CaKeyRecord { status: Active }`.
5. Update old record to `status: Retiring`.
6. `store.store_audit_event(CaRollover)`.
7. Return new CA cert details.

### `POST /api/v1/ca/rollover/commit`

1. Assert rollover is in progress (a `Retiring` key exists).
2. Update retiring key to `Retired` status.
3. `store.store_audit_event(CaRolloverCommit)`.

### `POST /api/v1/ca/rollover/abort`

1. Assert rollover is in progress.
2. Delete new (Active) key record from store. Update Retiring key back to Active.
3. Optionally delete the generated key from `KeyStore`.
4. `store.store_audit_event(CaRolloverAbort)`.

### `POST /api/v1/ca/cross-sign`

Input: `{ "csr_pem": "-----BEGIN CERTIFICATE REQUEST-----\n..." }`

1. Parse the external CA's CSR.
2. Verify it is a CA cert request (`is_ca = true` in basic constraints).
3. Build a cross-signed cert: subject from the external CA's CSR, signed by the
   intermediate CA key, with `CA: true` and `pathLen = 0`.
4. Store as `CertificateRecord` with profile `ca_intermediate`.
5. `store.store_audit_event(CrossSign)`.
6. Return signed cert PEM.

### `GET /api/v1/ssh/ca`

1. Load user CA and host CA public keys from `KeyStore`.
2. Format as OpenSSH authorized_keys lines.
3. Return JSON: `{ "user_ca": "ssh-ed25519 AAAA...", "host_ca": "ssh-ed25519 AAAA..." }`.

### `POST /api/v1/scep/challenges`

1. Generate 16-character random alphanumeric password.
2. bcrypt-hash the password (work factor 12).
3. Store `ScepChallenge { id: uuid, tenant_id, password_hash, used: false, expires_at: now + challenge_ttl }`.
4. Return `{ "id": "uuid", "password": "plain-text-password", "expires_at": "..." }`.
   The plaintext password is returned only once; it is not stored.

### `GET /api/v1/scep/challenges`

Return list of all `ScepChallenge` rows where `used = false AND expires_at > now`, omitting `password_hash`.

### `DELETE /api/v1/scep/challenges/{id}`

Mark challenge as used (consumed) to prevent its use: `store.consume_scep_challenge` is
not appropriate here (it checks by hash). Instead, use a direct update:
`UPDATE scep_challenges SET used = true WHERE id = $1 AND tenant_id = $2`.
This is executed via `store`'s `call_action("raw_sql", ...)`.

### Tenant Management (`/api/v1/tenants/*`)

Tenant management is gated by `ox_webservice` permission management to a `super_admin`
role. These endpoints operate across all tenants and do not require a `tenant_id` in
the URL path.

**`GET /api/v1/tenants`**: List all rows in a `tenants` table (new table, added to
migrations):
```sql
CREATE TABLE IF NOT EXISTS tenants (
    tenant_id    TEXT PRIMARY KEY,
    display_name TEXT,
    status       TEXT DEFAULT 'active',  -- 'active' | 'inactive'
    created_at   TIMESTAMP NOT NULL
);
```

**`POST /api/v1/tenants`**: Insert a new tenant row; return `{ tenant_id, status }`.

**`DELETE /api/v1/tenants/{tenant_id}`**: Set `status = 'inactive'`. Does not delete
data rows. A future background purge job (not in scope) removes inactive tenant data.

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| `serial` not found | 404 | `NOT_FOUND` |
| Rollover already in progress | 409 | `INVALID_REQUEST` |
| No rollover to commit/abort | 409 | `INVALID_REQUEST` |
| Cross-sign CSR is not a CA request | 400 | `INVALID_CSR` |
| Root key unavailable (rollover) | 503 | `CA_NOT_READY` |
| Tenant already exists | 409 | `INVALID_REQUEST` |
| Tenant not found | 404 | `TENANT_NOT_FOUND` |
| Storage failure | 500 | `INTERNAL_ERROR` |
