# ox_cert_acme (+ challenge plugins)

Covers three crates: `ox_cert_acme`, `ox_cert_acme_challenge_http`,
`ox_cert_acme_challenge_dns`.

---

# ox_cert_acme

**Purpose:** ACME protocol server (RFC 8555). Manages accounts, orders, authorizations,
and certificate finalization.

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/acme/directory` | ACME directory document |
| `HEAD` | `/acme/new-nonce` | Consume nonce (HEAD, no body) |
| `POST` | `/acme/new-nonce` | Consume nonce (POST, no body) |
| `POST` | `/acme/new-account` | Account registration or lookup |
| `POST` | `/acme/new-order` | Create a new order |
| `POST` | `/acme/order/{id}` | Get order status |
| `POST` | `/acme/order/{id}/finalize` | Submit CSR and finalize |
| `POST` | `/acme/authz/{id}` | Get authorization details |
| `POST` | `/acme/challenge/{id}` | Trigger challenge validation |
| `POST` | `/acme/cert/{id}` | Download issued certificate |
| `POST` | `/acme/revoke-cert` | Revoke via ACME |

Route registration: `"GET,HEAD,POST /acme/*"`.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | All shared types, `KeyStore`, `CertStore`, `CertError`, `CertBuilder` |
| `josekit` | JWS verification (RFC 7515), JWK parsing (RFC 7517) |
| `uuid` (v4) | Account/order/authz IDs and nonces |
| `base64` | base64url encoding/decoding |
| `sha2` | Key thumbprint (SHA-256) for challenge token |
| `serde` / `serde_json` | ACME JSON protocol |
| `time` | Order/authz expiry |

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct AcmeConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub extensions: ExtensionsConfig,
    /// URL for ACME Terms of Service.
    pub tos_url: Option<String>,
    /// If true, require External Account Binding for new accounts.
    pub external_account_required: bool,
    pub nonce_store: NonceStoreType,          // Memory | Database
    pub rate_limit: AcmeRateLimitConfig,
}

#[derive(Debug, Deserialize)]
pub enum NonceStoreType { Memory, Database }

#[derive(Debug, Deserialize)]
pub struct AcmeRateLimitConfig {
    pub orders_per_account_per_hour: u32,
    pub certs_per_domain_per_week: u32,
}
```

## Nonce Management

```rust
struct NonceStore {
    nonces: RwLock<std::collections::HashMap<String, std::time::Instant>>,
    ttl: std::time::Duration,  // default: 1 hour
}
```

When `nonce_store = Database`: nonces are stored in the `acme_nonces` table
(see core.md storage schema). A background thread in `ox_cert_acme`'s `ModuleContext`
runs every 5 minutes and executes:
`DELETE FROM acme_nonces WHERE tenant_id = $1 AND expires_at < NOW()`
via `store`'s `call_action("raw_sql", ...)`. Additionally, each nonce-consuming request
performs lazy cleanup of up to 100 expired rows before responding, bounding table growth
even if the background thread is delayed.

Every response includes a fresh `Replay-Nonce` header. Nonces are single-use; a request
with a previously consumed nonce is rejected with `badNonce`.

## JWS Verification

All POST requests carry a JWS body (RFC 7515). Verification:
1. Parse the JWS compact serialization from the request body.
2. Extract the `kid` (account URL) or `jwk` (new-account) from the protected header.
3. For `kid`: load `AcmeAccount` from store; use `account.jwk` as the verification key.
4. For `jwk`: use the embedded JWK directly (only valid for `new-account`).
5. Verify the JWS signature using `josekit`.
6. Verify the `nonce` in the protected header is present and unused.
7. Verify the `url` in the protected header matches the request URL.

## Order Lifecycle

```
pending → ready (all authz valid) → processing (finalize called) → valid (cert issued)
                                                                  → invalid (failed)
```

## Processing (by endpoint)

### `GET /acme/directory`
Returns the ACME directory JSON (no JWS required):
```json
{
  "newNonce":   "https://ca.example.com/acme/new-nonce",
  "newAccount": "https://ca.example.com/acme/new-account",
  "newOrder":   "https://ca.example.com/acme/new-order",
  "revokeCert": "https://ca.example.com/acme/revoke-cert",
  "meta": { "termsOfService": "...", "externalAccountRequired": false }
}
```

### `HEAD /acme/new-nonce` or `POST /acme/new-nonce`
1. Generate `uuid::Uuid::new_v4().to_string()` as nonce.
2. Store nonce in `NonceStore` with TTL.
3. Set `Replay-Nonce` header. Return 200 (HEAD) or 204 (POST).

### `POST /acme/new-account`
1. Verify JWS (embedded `jwk`).
2. If account with this JWK already exists: return existing account (RFC 8555 §7.3.1).
3. If `external_account_required` and no EAB: return `externalAccountRequired` error.
4. Create `AcmeAccount` with new UUID, store it.
5. Return 201 with account URL in `Location` header.

### `POST /acme/new-order`
1. Verify JWS (account `kid`).
2. Parse `identifiers` from payload — list of `{type: "dns", value: "example.com"}`.
3. Apply rate limits; return `rateLimited` error if exceeded.
4. Validate each identifier against `IssuancePolicy` allowlist/blocklist.
5. Create `AcmeOrder` (status: `pending`) and one `AcmeAuthorization` per identifier.
6. Each authorization: status `pending`, one `http-01` challenge and one `dns-01` challenge.
7. Store order and authorizations. Return 201 with order URL.

### `POST /acme/order/{id}/finalize`
1. Verify JWS. Load order; assert status == `ready` (all authz valid).
2. Parse CSR from payload `csr` field (base64url DER).
3. Verify CSR matches order identifiers.
4. Set order status `processing`. Store.
5. Issue certificate via `ox_cert_core::CertBuilder` (same pipeline as `ox_cert_issue`).
   Set `enrollment_protocol = Acme`.
6. Store cert; set `order.certificate_serial = serial`; set order status `valid`.
7. Return order object with `certificate` URL.

### `POST /acme/cert/{id}`
1. Load order by id. Assert `status == valid`.
2. Load `CertificateRecord` by `certificate_serial`.
3. Return full chain PEM with `Content-Type: application/pem-certificate-chain`.

### `POST /acme/challenge/{id}`
1. Load authorization by challenge id.
2. Set challenge status `processing`. Store.
3. Set `cert.acme.challenge.id`, `cert.acme.challenge.type`, `cert.acme.challenge.token`,
   `cert.acme.challenge.thumbprint` in TaskState.
4. Return `FLOW_CONTROL_CONTINUE` — the challenge plugin downstream handles validation.

### `POST /acme/revoke-cert`
1. Verify JWS. Extract cert DER from payload `certificate` field.
2. Parse cert DER; extract serial UUID.
3. Look up cert in store. Revoke with reason `Unspecified` (or reason from payload).

## Error Cases

All ACME errors are returned as RFC 8555 problem documents:
```json
{ "type": "urn:ietf:params:acme:error:badNonce", "detail": "Nonce already used" }
```

| Condition | HTTP | ACME error type |
|---|---|---|
| Invalid JWS | 400 | `malformed` |
| Nonce missing or used | 400 | `badNonce` |
| Rate limit exceeded | 429 | `rateLimited` |
| Order not ready for finalize | 403 | `orderNotReady` |
| CSR identifiers mismatch | 400 | `badCSR` |
| Policy violation | 403 | `rejectedIdentifier` |

---

# ox_cert_acme_challenge_http

**Purpose:** Validates ACME HTTP-01 challenges by performing an outbound HTTP GET.

## Phase
`Content` (runs after `ox_cert_acme` sets challenge context in TaskState)

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `CertStore`, `CertError` |
| `reqwest` (blocking) | Outbound HTTP GET to validate challenge |
| `sha2` | Key thumbprint computation |
| `base64` | base64url encode thumbprint |

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct AcmeChallengeHttpConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub timeout_secs: u64,   // default: 10
    pub retries: u32,        // default: 3
}
```

## Processing

1. Read `cert.acme.challenge.type` from TaskState. If not `http-01`: return
   `FLOW_CONTROL_CONTINUE` (not our challenge type).
2. Read `cert.acme.challenge.token` and `cert.acme.challenge.thumbprint`.
3. Read `cert.acme.challenge.id` and load `AcmeAuthorization` from store.
4. Construct validation URL:
   `http://{identifier_value}/.well-known/acme-challenge/{token}`
5. HTTP GET with configured timeout and retries.
6. Verify response body equals `{token}.{thumbprint}`.
7. If valid: update authorization status to `valid` in store.
8. If invalid or timeout: update authorization status to `invalid` in store.
9. Return `FLOW_CONTROL_CONTINUE`.

---

# ox_cert_acme_challenge_dns

**Purpose:** Validates ACME DNS-01 challenges via DNS TXT record lookup.

## Phase
`Content` (runs after `ox_cert_acme` sets challenge context in TaskState)

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `CertStore`, `CertError` |
| `trust-dns-resolver` | Async DNS TXT lookup |
| `sha2` | SHA-256 of `{token}.{thumbprint}` |
| `base64` | base64url encode the digest |
| `serde` / `serde_json` | Config, provider API bodies |

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct AcmeChallengesDnsConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    /// DNS resolver address (e.g., "8.8.8.8:53"). If None, uses system resolver.
    pub resolver: Option<String>,
    pub timeout_secs: u64,          // default: 10
    /// Seconds to wait before querying (for DNS propagation).
    pub propagation_delay_secs: u64, // default: 30
    pub auto_provision: Option<DnsProviderConfig>,
}

#[derive(Debug, Deserialize)]
pub struct DnsProviderConfig {
    pub provider: String,           // "route53" | "cloudflare"
    pub credentials_env: String,    // env var containing JSON credentials
}
```

## Processing

1. Read `cert.acme.challenge.type`. If not `dns-01`: return `FLOW_CONTROL_CONTINUE`.
2. Read token and thumbprint from TaskState.
3. Compute expected TXT value: `base64url(SHA-256("{token}.{thumbprint}"))`.
4. If `auto_provision` is set: call DNS provider API to create
   `_acme-challenge.{identifier_value}` TXT record.
5. Sleep `propagation_delay_secs`.
6. Perform DNS TXT lookup for `_acme-challenge.{identifier_value}`.
7. Check if any returned TXT value matches the expected value.
8. Update authorization status (valid or invalid) in store.
9. Return `FLOW_CONTROL_CONTINUE`.
