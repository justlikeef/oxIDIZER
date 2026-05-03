# ox_cert_acme

ACME protocol server (RFC 8555). Manages accounts, orders, authorizations, and certificate
finalization. The challenge validation plugins (`ox_cert_acme_challenge_http` and
`ox_cert_acme_challenge_dns`) run as downstream pipeline stages.

---

## Phase

`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/acme/directory` | ACME directory document |
| `HEAD/POST` | `/acme/new-nonce` | Issue a fresh replay nonce |
| `POST` | `/acme/new-account` | Register or look up an account |
| `POST` | `/acme/new-order` | Create a new certificate order |
| `POST` | `/acme/order/{id}` | Get order status |
| `POST` | `/acme/order/{id}/finalize` | Submit CSR and finalize order |
| `POST` | `/acme/authz/{id}` | Get authorization details |
| `POST` | `/acme/challenge/{id}` | Trigger challenge validation |
| `POST` | `/acme/cert/{id}` | Download issued certificate |
| `POST` | `/acme/revoke-cert` | Revoke certificate via ACME |

Route registration: `"GET,HEAD,POST /acme/*"`.

---

## Config Reference

```rust
pub struct AcmeConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub extensions: ExtensionsConfig,
    pub tos_url: Option<String>,
    pub external_account_required: bool,
    pub nonce_store: NonceStoreType,      // Memory | Database
    pub rate_limit: AcmeRateLimitConfig,
}

pub struct AcmeRateLimitConfig {
    pub orders_per_account_per_hour: u32,
    pub certs_per_domain_per_week: u32,
}
```

| Field | Default | Description |
|---|---|---|
| `tos_url` | absent | Terms of Service URL included in directory document |
| `external_account_required` | `false` | Require External Account Binding for new accounts |
| `nonce_store` | `Memory` | `Memory` for single-node; `Database` for multi-node |
| `rate_limit.orders_per_account_per_hour` | required | Rate limit per ACME account |
| `rate_limit.certs_per_domain_per_week` | required | Rate limit per domain name |

---

## Order Lifecycle

```
pending → ready (all authz valid) → processing (finalize called) → valid (cert issued)
                                                                  → invalid (failed)
```

---

## Challenge Plugins

`ox_cert_acme` sets challenge context in `TaskState` and returns `FLOW_CONTROL_CONTINUE`.
Challenge validation is handled by downstream plugins:

**`ox_cert_acme_challenge_http`** — validates HTTP-01 challenges:
- Reads `cert.acme.challenge.type`; skips if not `http-01`.
- Makes an outbound HTTP GET to `http://{domain}/.well-known/acme-challenge/{token}`.
- Verifies response body equals `{token}.{thumbprint}`.
- Config: `timeout_secs` (default: 10), `retries` (default: 3).

**`ox_cert_acme_challenge_dns`** — validates DNS-01 challenges:
- Reads `cert.acme.challenge.type`; skips if not `dns-01`.
- Computes expected TXT value: `base64url(SHA-256("{token}.{thumbprint}"))`.
- Optionally auto-provisions the DNS TXT record via `auto_provision` (Route53/Cloudflare).
- Waits `propagation_delay_secs` (default: 30) before querying DNS.
- Config: `resolver`, `timeout_secs`, `propagation_delay_secs`, `auto_provision`.

---

## Nonce Management

Every ACME response includes a fresh `Replay-Nonce` header. Nonces are single-use; a
request using a previously-consumed nonce is rejected with `badNonce`.

- `nonce_store: Memory` — in-process hash map with 1-hour TTL. A background thread
  purges expired nonces every 5 minutes. Suitable for single-node deployments.
- `nonce_store: Database` — stored in `acme_nonces` table. Multi-node safe. Lazy cleanup
  of up to 100 expired rows per consuming request.

---

## Error Cases

All ACME errors are RFC 8555 problem documents:
```json
{ "type": "urn:ietf:params:acme:error:badNonce", "detail": "Nonce already used" }
```

| Condition | HTTP | ACME error |
|---|---|---|
| Invalid JWS signature | 400 | `malformed` |
| Nonce missing or already used | 400 | `badNonce` |
| Rate limit exceeded | 429 | `rateLimited` |
| Order not in `ready` state for finalize | 403 | `orderNotReady` |
| CSR identifiers do not match order | 400 | `badCSR` |
| Policy violation | 403 | `rejectedIdentifier` |

---

## Implementation Notes

- JWS verification (RFC 7515) uses the `josekit` crate. The `kid` header resolves the
  account public key from the database; the `jwk` header is only accepted for new-account
  requests.
- Certificate finalization uses the same `CertBuilder` pipeline as `ox_cert_issue`, with
  `enrollment_protocol = Acme`.
- `POST /acme/revoke-cert` works even without a `kid` — the account public key must
  match the certificate's public key, providing proof of possession.
