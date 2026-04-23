# ox_cert_health

**Purpose:** CA-specific health and readiness probes — checks CA key accessibility,
database connectivity, CRL freshness, CA cert validity, and storage capacity.

---

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/healthz` | Kubernetes liveness probe (always 200 if process running) |
| `GET` | `/readyz` | Kubernetes readiness probe (200 only if all subsystems ready) |
| `GET` | `/api/v1/health` | Detailed JSON health status with per-check results |

Route registration: `"GET /healthz,GET /readyz,GET /api/v1/health"`.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `KeyStore`, `CertStore`, `HealthStatus`, `CheckResult`, `CertError` |
| `serde_json` | Response serialization |
| `time` | CA cert expiry computation |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct HealthConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    /// Warn (degraded) if CA cert expires within this many days (default: 365).
    pub ca_cert_warn_days: u32,
    /// Maximum age of the current CRL before status becomes degraded (e.g., "2h").
    pub crl_staleness_threshold: std::time::Duration,
}
```

---

## Checks Performed

| Check | Key | Criteria |
|---|---|---|
| **CA key** | `ca_key` | `KeyStore::key_exists(tenant_id, intermediate_key_id)` returns Ok(true). Latency measured. |
| **Database** | `database` | `CertStore` connection alive: run trivial `store.list_expiring(tenant_id, 0)` with limit 1. Latency measured. |
| **CRL freshness** | `crl_fresh` | Query `crl_generation_locks` for `lock_key="full_crl"`; check `expires_at > now - staleness_threshold`. |
| **CA cert validity** | `ca_cert_valid` | Parse intermediate CA cert PEM; check `not_after`. |
| **Root cert validity** | `root_cert_valid` | Parse root CA cert PEM; check `not_after`. |

### Status Roll-Up

| Condition | Overall Status |
|---|---|
| All checks pass | `healthy` |
| `ca_key` or `database` fails | `unhealthy` |
| `crl_fresh` fails or any CA cert expires within `ca_cert_warn_days` | `degraded` |
| CA cert already expired | `unhealthy` |

---

## Processing

### `GET /healthz`

Always returns HTTP 200 with body `"ok"`. This is a liveness probe — it only fails
if the process itself is not responding.

### `GET /readyz`

1. Run all checks in parallel (spawn threads or use blocking tasks).
2. If overall status is `healthy` or `degraded`: return HTTP 200.
3. If overall status is `unhealthy`: return HTTP 503.
4. Body: `{ "status": "ready" }` or `{ "status": "not_ready", "reason": "..." }`.

### `GET /api/v1/health`

1. Run all checks.
2. Build `HealthStatus` struct.
3. Return HTTP 200 (always, even when unhealthy — the status is in the body).

```json
{
  "data": {
    "status": "degraded",
    "tenant_id": "acme-corp",
    "checks": {
      "ca_key":        { "ok": true,  "latency_ms": 2 },
      "database":      { "ok": true,  "latency_ms": 5 },
      "crl_fresh":     { "ok": false, "message": "CRL last updated 3h ago; threshold 2h" },
      "ca_cert_valid": { "ok": true,  "message": "Expires in 287 days" },
      "root_cert_valid": { "ok": true, "message": "Expires in 8921 days" }
    }
  },
  "meta": { "tenant_id": "acme-corp", "request_id": "uuid" }
}
```

---

## Error Cases

No error responses — health endpoints always return a response (200 or 503 for readyz).
Individual check failures are represented in the JSON body, not as HTTP error codes.
