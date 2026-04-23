# ox_cert_ct

**Purpose:** Certificate Transparency log management. Serves SCT query endpoints for
issued certificates. Issuance-time SCT submission is handled by the library function
`ox_cert_core::ct::submit_to_ct_logs()` — not by this plugin.

---

## Design Clarification

CT submission during certificate issuance is a *library call*, not a pipeline stage.
`ox_cert_issue` calls `ox_cert_core::ct::submit_to_ct_logs(tbs_cert_der, issuer_cert_der,
&ct_config)` directly and embeds the returned SCTs into the signed certificate.

This plugin (`ox_cert_ct`) serves only:
- Query endpoints to retrieve stored SCTs for issued certificates.
- No issuance logic is duplicated here.

Do NOT add `ox_cert_ct` before `ox_cert_issue` in the pipeline. It is an independent
endpoint-only plugin.

---

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/ct/scts/{serial}` | Return SCTs for a specific certificate serial |
| `GET` | `/api/v1/ct/logs` | List configured CT logs and their status |

Route registration: `"GET /api/v1/ct/*"`.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `CertStore`, `Sct`, `CtConfig`, `CertError` |
| `reqwest` (blocking) | Health-check outbound calls to CT logs (for `/ct/logs` status) |
| `serde` / `serde_json` | Response serialization |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct CtPluginConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub ct: CtConfig,
}
```

---

## Processing

### `GET /api/v1/ct/scts/{serial}`

1. Extract `serial` from path.
2. `store.get_cert_by_serial(tenant_id, serial)` → 404 if not found.
3. Return `record.scts` as JSON:
   ```json
   {
     "data": {
       "serial": "uuid",
       "scts": [
         { "log_id": "base64", "log_name": "Google Argon", "timestamp": "...", "signature": "base64" }
       ]
     },
     "meta": { "tenant_id": "acme-corp", "request_id": "uuid" }
   }
   ```
4. If `record.scts` is empty: return 200 with empty array and note in `meta`:
   `"note": "CT was disabled or submission failed at issuance time"`.

### `GET /api/v1/ct/logs`

1. For each configured log in `ct.logs`:
   a. Make a HEAD request to `{log_url}/ct/v1/get-sth` with a short timeout (3s).
   b. Record latency and whether the response was 2xx.
2. Return JSON:
   ```json
   {
     "data": [
       { "name": "Google Argon", "url": "...", "reachable": true, "latency_ms": 45 },
       { "name": "Cloudflare Nimbus", "url": "...", "reachable": false, "latency_ms": null }
     ],
     "meta": { "tenant_id": "acme-corp" }
   }
   ```

---

## CT Library (`ox_cert_core::ct`)

This section describes the library function called by `ox_cert_issue`, not the plugin.

```rust
/// Submit a pre-certificate to configured CT logs and collect SCTs.
/// Called by ox_cert_issue before final signing when ct.enabled = true.
pub fn submit_to_ct_logs(
    tbs_cert_der: &[u8],
    issuer_cert_der: &[u8],
    config: &CtConfig,
) -> Result<Vec<Sct>, CertError> {
    // 1. Build pre-certificate: copy TBS cert, add poison extension
    //    (OID 1.3.6.1.4.1.11129.2.4.3, critical, empty value).
    // 2. Build "add-pre-chain" request body:
    //    { "chain": [base64(pre_cert_der), base64(issuer_cert_der)] }
    // 3. POST to each log's /ct/v1/add-pre-chain with config.timeout.
    // 4. Collect responses. Each success returns:
    //    { "sct_version": 0, "id": "base64", "timestamp": u64, "signature": "base64" }
    // 5. Convert to Vec<Sct>.
    // 6. If len(scts) < config.min_scts:
    //    match config.on_failure {
    //        Block => return Err(CertError::CtError("insufficient SCTs")),
    //        Warn  => log warning and return partial Sct vec,
    //    }
    // 7. Return scts.
}
```

SCTs are embedded in the final signed certificate as an X.509 extension with OID
`1.3.6.1.4.1.11129.2.4.2` (RFC 6962 §3.3). `CertBuilder` accepts them via
`add_extension(CustomExtension { oid, critical: false, value: sct_list_der })`.

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| `serial` not found | 404 | `NOT_FOUND` |
| Storage failure | 500 | `INTERNAL_ERROR` |

CT log reachability errors in `/ct/logs` are represented in the response body, not as
HTTP errors.
