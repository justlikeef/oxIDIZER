# ox_cert_health

CA-specific health and readiness probes. Checks CA key accessibility, database
connectivity, CRL freshness, and CA certificate validity. Designed for use with
Kubernetes liveness/readiness probes and monitoring systems.

---

## Phase

`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/healthz` | Kubernetes liveness probe (always 200) |
| `GET` | `/readyz` | Kubernetes readiness probe (200 or 503) |
| `GET` | `/api/v1/health` | Detailed per-check JSON status (always 200) |

Route registration: `"GET /healthz,GET /readyz,GET /api/v1/health"`.

---

## Config Reference

```rust
pub struct HealthConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub ca_cert_warn_days: u32,                    // default: 365
    pub crl_staleness_threshold: Duration,          // e.g. "2h"
}
```

| Field | Default | Description |
|---|---|---|
| `ca_cert_warn_days` | `365` | Days before CA cert expiry to set status to `degraded` |
| `crl_staleness_threshold` | required | Maximum acceptable age of the current CRL |

---

## Checks Performed

| Check key | What it tests |
|---|---|
| `ca_key` | `KeyStore::key_exists(tenant_id, intermediate_key_id)` — key accessible and latency measured |
| `database` | `CertStore` connection alive via `list_expiring(tenant_id, 0)` with limit 1 |
| `crl_fresh` | CRL generation lock table timestamp — CRL regenerated within `crl_staleness_threshold` |
| `ca_cert_valid` | Intermediate CA cert PEM parsed; `not_after` evaluated |
| `root_cert_valid` | Root CA cert PEM parsed; `not_after` evaluated |

---

## Status Roll-Up

| Condition | Overall Status |
|---|---|
| All checks pass | `healthy` |
| `ca_key` or `database` fails | `unhealthy` |
| `crl_fresh` fails | `degraded` |
| CA cert expiring within `ca_cert_warn_days` | `degraded` |
| CA cert already expired | `unhealthy` |

---

## Endpoint Behavior

**`GET /healthz`:** Always returns HTTP 200 with body `"ok"`. Liveness — only fails if
the process is not responding.

**`GET /readyz`:** Returns 200 if status is `healthy` or `degraded`; 503 if `unhealthy`.
Body: `{"status": "ready"}` or `{"status": "not_ready", "reason": "..."}`.

**`GET /api/v1/health`:** Always returns HTTP 200. Body:

```json
{
  "data": {
    "status": "degraded",
    "tenant_id": "acme-corp",
    "checks": {
      "ca_key":          { "ok": true,  "latency_ms": 2 },
      "database":        { "ok": true,  "latency_ms": 5 },
      "crl_fresh":       { "ok": false, "message": "CRL last updated 3h ago; threshold 2h" },
      "ca_cert_valid":   { "ok": true,  "message": "Expires in 287 days" },
      "root_cert_valid": { "ok": true,  "message": "Expires in 8921 days" }
    }
  },
  "meta": { "tenant_id": "acme-corp", "request_id": "uuid" }
}
```

---

## Implementation Notes

- Health checks run in parallel on each request for minimum latency.
- There are no error HTTP responses from this plugin — all outcomes are represented in
  the response body or the 200/503 status code of `/readyz`.
- The Kubernetes liveness probe should point to `/healthz`. Readiness probe should point
  to `/readyz`. Monitoring systems should use `/api/v1/health` for alerting detail.
