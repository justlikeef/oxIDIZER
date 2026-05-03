# ox_cert_webhook

Calls external HTTPS services during certificate issuance for authorization decisions and
certificate attribute enrichment. Must be placed before `ox_cert_issue` in the pipeline.

---

## Phase

`Content` — must be ordered before `ox_cert_issue`.

## Routes

None. This plugin processes every request that reaches it in the pipeline; it does not
match specific routes.

---

## Config Reference

```rust
pub struct WebhookConfig {
    pub tenant_id: String,
    pub hooks: Vec<WebhookHookConfig>,
}

pub struct WebhookHookConfig {
    pub name: String,
    pub url: String,                       // Must be HTTPS
    pub hook_type: WebhookType,            // Authorize | Enrich | Both
    pub secret_env: String,                // Env var name for HMAC-SHA256 signing secret
    pub timeout_secs: u64,                 // default: 5
    pub retries: u32,                      // default: 1
    pub on_failure: WebhookFailureMode,    // Block | Allow
}
```

| Field | Default | Description |
|---|---|---|
| `url` | required | HTTPS URL only. HTTP URLs cause startup failure. |
| `hook_type` | required | `Authorize` (allow/deny), `Enrich` (add data), or `Both` |
| `secret_env` | required | Environment variable holding HMAC-SHA256 secret |
| `timeout_secs` | `5` | Per-request timeout |
| `retries` | `1` | Retry count on network failure |
| `on_failure` | required | `Block` — treat hook failure as denial; `Allow` — pass through |

---

## Hook Payload

Every hook receives:
```json
{
  "event":              "certificate_request",
  "tenant_id":          "acme-corp",
  "request_id":         "uuid",
  "csr_subject":        "CN=example.com,O=ACME Corp",
  "sans":               ["example.com", "www.example.com"],
  "profile":            "standard",
  "requester_ip":       "10.0.0.42",
  "requester_identity": "10.0.0.42",
  "timestamp":          "2026-04-22T10:00:00Z"
}
```

The payload is HMAC-SHA256 signed. The signature is set as
`X-OxCert-Signature: sha256={base64(hmac)}` on the request.

---

## Hook Response

**Authorize hooks** return: `{ "allow": true/false, "reason": "..." }`

**Enrich hooks** return: `{ "data": { "additional_sans": [...], "custom_extensions": [...], "subject_ou": "..." } }`

**Both hooks** combine: `{ "allow": true, "reason": "...", "data": { ... } }`

---

## Output TaskState Fields

| Field | Value |
|---|---|
| `cert.webhook.authorized` | `"true"` if all authorize hooks passed; `"false"` if any blocked |
| `cert.webhook.enrichment` | JSON object merged from all enrich hooks |

`cert.webhook.enrichment` format:
```json
{
  "additional_sans":    ["internal.example.com"],
  "custom_extensions":  [{ "oid": "1.3.6.1.4.1.99999.1", "critical": false, "value_hex": "0101ff" }],
  "subject_ou":         "Engineering"
}
```

`ox_cert_issue` reads this field and applies: `additional_sans` are appended to the SAN
list; `custom_extensions` are added via `CertBuilder::add_extension`; `subject_ou` is
merged into the distinguished name.

---

## Error Cases

| Condition | Behavior |
|---|---|
| `secret_env` not set | Startup failure; returns null from `ox_plugin_init` |
| Any hook URL is HTTP (not HTTPS) | Startup failure |
| Network timeout, `on_failure = block` | 403 `WEBHOOK_REJECTED` |
| Non-2xx response, `on_failure = block` | 403 `WEBHOOK_REJECTED` |
| `allow: false` in response | 403 `WEBHOOK_REJECTED` with reason |
| Network error, `on_failure = allow` | Log warning; continue pipeline |

---

## Security Notes

- HMAC secret is read from the environment at startup and never logged.
- Webhook URLs must be HTTPS. The check is at plugin init time.
- Request bodies contain CSR public key data. Webhook endpoints should be treated as
  trusted internal services and protected with network-level controls.
