# ox_cert_crl

Generates and serves Certificate Revocation Lists (RFC 5280 §5). Supports full and delta
CRLs, in-process caching, active/active HA coordination via the advisory lock table, and
optional background pre-generation.

---

## Phase

`Content` (request handler) + optional background thread (if `background_pregenerate = true`)

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/crl` | Full CRL (DER) |
| `GET` | `/crl.pem` | Full CRL (PEM) |
| `GET` | `/crl/delta` | Delta CRL (DER) |
| `GET` | `/crl/delta.pem` | Delta CRL (PEM) |

Route registration: `"GET /crl/*"`.

---

## Config Reference

```rust
pub struct CrlConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub update_interval: Duration,      // e.g. "1h"
    pub delta_interval: Duration,       // e.g. "10m"
    pub cache_ttl: Duration,            // e.g. "30m"
    pub lock_ttl_secs: u64,
    pub node_id: Option<String>,        // defaults to "{hostname}:{pid}"
    pub background_pregenerate: bool,
}
```

| Field | Default | Description |
|---|---|---|
| `update_interval` | required | Full CRL regeneration interval (e.g., `"1h"`) |
| `delta_interval` | required | Delta CRL regeneration interval (e.g., `"10m"`) |
| `cache_ttl` | required | In-process cache TTL before re-reading DB |
| `lock_ttl_secs` | required | Advisory lock TTL. Must exceed worst-case CRL generation time. |
| `node_id` | auto | Node identifier for lock ownership; defaults to `{hostname}:{pid}` |
| `background_pregenerate` | `false` | Spawn background thread to pre-generate CRLs |

---

## Caching and HA Behavior

CRLs are cached in the plugin context behind a `RwLock<Option<CachedCrl>>`. On each
request:

- If cache is valid (`next_update > now`): serve immediately from cache.
- If cache is stale: attempt to acquire the advisory lock from `CertStore`.
  - Lock acquired: regenerate CRL, update cache, release lock.
  - Lock not acquired (another node is generating): serve stale cache with
    `Warning: 199 ox_cert_crl "CRL regeneration in progress; serving cached copy"`.
  - No cache and lock held: return 503 `CA_NOT_READY`.

This ensures only one node generates a CRL per interval while all nodes can serve
cached copies.

---

## Delta CRL

The delta CRL includes only revocations since the last full CRL. It carries the
`deltaCRLIndicator` extension (RFC 5280 §5.2.4) pointing to the base CRL number. Delta
CRL lock key is `"delta_crl"` (separate from `"full_crl"`).

---

## Background Pre-Generation

When `background_pregenerate = true`, a background thread wakes at
`min(update_interval, delta_interval) / 2` and regenerates whichever CRL is within 60s
(full) or 30s (delta) of its `next_update`. This prevents first-request latency spikes
and keeps caches warm across all nodes.

---

## Error Cases

| Condition | HTTP | Behavior |
|---|---|---|
| Lock held by another node, cache stale | 200 | Serve cache with `Warning` header |
| No cache, lock held | 503 | `CA_NOT_READY` |
| Signing key unavailable | 503 | `CA_NOT_READY` |
| Storage failure | 500 | `INTERNAL_ERROR` |

---

## Implementation Notes

- CRL numbers are monotonically increasing (RFC 5280 §5.2.3 requirement). The advisory
  lock table provides each node that acquires the lock with the next CRL number. This
  ensures monotonicity even under active/active deployment.
- `lock_ttl_secs` should be set to at least 2x the expected CRL generation time to avoid
  false expiry under load. A value of 300 (5 minutes) is suitable for most deployments.
