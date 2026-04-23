# ox_cert_crl

**Purpose:** Generates and serves Certificate Revocation Lists (RFC 5280 §5), with full
and delta CRL support and active/active HA coordination via the advisory lock table.

---

## Phase
`Content` (request handler) + optional background pre-generation thread.

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/crl` | Full CRL (DER) |
| `GET` | `/crl.pem` | Full CRL (PEM) |
| `GET` | `/crl/delta` | Delta CRL (DER) |
| `GET` | `/crl/delta.pem` | Delta CRL (PEM) |

Route registration: `"GET /crl/*"` (router dispatches on trailing segment).

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `KeyStore`, `CertStore`, `CertError`, advisory lock helpers |
| `x509-parser` | Build CRL structures |
| `rcgen` | Sign CRL with CA key |
| `ring` | Signing primitives |
| `pem` | PEM encode CRL DER |
| `serde_json` | Config deserialization |
| `time` | `now()`, next-update computation |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct CrlConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    /// Full CRL regeneration interval (e.g., "1h").
    pub update_interval: std::time::Duration,
    /// Delta CRL regeneration interval (e.g., "10m").
    pub delta_interval: std::time::Duration,
    /// In-process cache TTL before re-reading DB (e.g., "30m").
    pub cache_ttl: std::time::Duration,
    /// Advisory lock TTL in seconds. Must exceed worst-case CRL generation time.
    pub lock_ttl_secs: u64,
    /// Node identifier for lock ownership. Defaults to "{hostname}:{pid}".
    pub node_id: Option<String>,
    /// If true, spawn a background thread to pre-generate CRLs on the update interval.
    pub background_pregenerate: bool,
}
```

---

## ModuleContext

```rust
pub struct CrlContext {
    api: CoreHostApi,
    config: CrlConfig,
    store: Arc<dyn CertStore>,
    key_store: Arc<dyn KeyStore>,
    full_crl: Arc<RwLock<Option<CachedCrl>>>,
    delta_crl: Arc<RwLock<Option<CachedCrl>>>,
    _background: Option<std::thread::JoinHandle<()>>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
}

struct CachedCrl {
    der: Vec<u8>,
    pem: String,
    generated_at: time::OffsetDateTime,
    next_update: time::OffsetDateTime,
    crl_number: u64,
}
```

---

## Processing (request handler)

### Full CRL (`GET /crl` or `GET /crl.pem`)

1. Acquire read lock on `full_crl`.
2. If cache exists and `cached.next_update > now`: serve from cache.
3. Otherwise: drop read lock, call `generate_full_crl()` (see below).
4. Serialize to DER or PEM based on path suffix.
5. Set `response.header.Content-Type`:
   - DER: `application/pkix-crl`
   - PEM: `application/x-pem-file`
6. Set `response.header.Cache-Control: max-age={seconds until next_update}`.
7. Set `response.status = "200"`.

### Delta CRL (`GET /crl/delta` or `GET /crl/delta.pem`)

Same flow but against `delta_crl` cache and `generate_delta_crl()`.

---

## CRL Generation

### `generate_full_crl() -> Result<CachedCrl, CertError>`

1. Try `store.acquire_crl_lock(tenant_id, "full_crl", node_id, lock_ttl_secs)`.
2. If `Ok(None)`: another node holds the lock. Serve the stale cache and set an RFC 7234
   `Warning` response header:
   `Warning: 199 ox_cert_crl "CRL regeneration in progress; serving cached copy"`.
3. If `Ok(Some(crl_number))`: this node owns the lock.
4. `store.list_revoked(tenant_id)` — fetch all revoked cert records.
5. Construct CRL:
   - `version = v2`
   - `thisUpdate = now`
   - `nextUpdate = now + update_interval`
   - `cRLNumber = crl_number` (from lock table)
   - `deltaCRLIndicator` — absent (this is the full CRL)
   - One `RevokedCertificate` entry per record: serial (UUID bytes), revocation date,
     reason code extension.
6. Sign with `KeyStore::sign(tenant_id, ca_intermediate_key_id, algo, tbs_crl_der)`.
7. Wrap in CRL DER.
8. PEM-encode.
9. Update `full_crl` cache under write lock.
10. `store.release_crl_lock(tenant_id, "full_crl", node_id)`.
11. Return `CachedCrl`.

### `generate_delta_crl() -> Result<CachedCrl, CertError>`

Same as above but:
- Lock key: `"delta_crl"`.
- `store.list_revoked_since(tenant_id, last_full_crl_time)` — only certs revoked since
  the last full CRL.
- Includes `deltaCRLIndicator` extension (RFC 5280 §5.2.4) pointing to the base CRL number
  (read from `full_crl` cache).
- `nextUpdate = now + delta_interval`.

---

## Background Pre-Generation Thread

When `background_pregenerate = true`, the background thread loops:

```
loop:
    sleep(min(update_interval, delta_interval) / 2)
    if now > full_crl.next_update - 60s:
        generate_full_crl()
    if now > delta_crl.next_update - 30s:
        generate_delta_crl()
    if shutdown:
        break
```

This prevents first-request latency and keeps the cache warm across nodes.

---

## Error Cases

| Condition | HTTP | Behaviour |
|---|---|---|
| Lock held by another node | 200 | Serve stale cache with `Warning` header |
| No cache and lock held | 503 | `CA_NOT_READY`: CRL not yet generated |
| Signing key unavailable | 503 | `CA_NOT_READY` |
| Storage failure | 500 | `INTERNAL_ERROR` |
