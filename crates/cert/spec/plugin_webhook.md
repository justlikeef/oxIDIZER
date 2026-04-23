# ox_cert_webhook

**Purpose:** Calls external services during certificate issuance for authorization
decisions and certificate attribute enrichment. Must run as a pipeline stage *before*
`ox_cert_issue`.

---

## Phase
`Content` (ordered before `ox_cert_issue` in pipeline config)

## Routes
None. This plugin processes every request that reaches it in the pipeline; route
filtering is handled by the pipeline router upstream.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `CertError` |
| `reqwest` (blocking) | Outbound HTTPS calls to webhook endpoints |
| `ring` | HMAC-SHA256 request signing |
| `base64` | Encode HMAC signature |
| `serde` / `serde_json` | Webhook payload and response serialization |
| `regex` | Match request paths to route scope |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct WebhookConfig {
    pub tenant_id: String,
    pub hooks: Vec<WebhookHookConfig>,
}

#[derive(Debug, Deserialize)]
pub struct WebhookHookConfig {
    pub name: String,
    pub url: String,                              // Must be HTTPS
    pub hook_type: WebhookType,
    /// Environment variable name holding the HMAC-SHA256 signing secret.
    pub secret_env: String,
    pub timeout_secs: u64,                        // default: 5
    pub retries: u32,                             // default: 1
    pub on_failure: WebhookFailureMode,
}

#[derive(Debug, Deserialize)]
pub enum WebhookType {
    /// Returns { allow: bool, reason: String }
    Authorize,
    /// Returns { data: { field: value, ... } }
    Enrich,
    /// Combines both: returns { allow: bool, reason: String, data: {...} }
    Both,
}

#[derive(Debug, Deserialize)]
pub enum WebhookFailureMode {
    Block,   // Treat webhook failure as denial
    Allow,   // Treat webhook failure as pass-through
}
```

---

## Input TaskState Fields

| Field | Description |
|---|---|
| `request.body` | The raw enrollment request body (read-only reference) |
| `request.header.X-Forwarded-For` | Requester IP |
| `request.path` | Request path |

---

## Processing

For each configured hook (in order):

1. Build webhook payload from TaskState:
   ```json
   {
     "event":               "certificate_request",
     "tenant_id":           "acme-corp",
     "request_id":          "uuid",
     "csr_subject":         "CN=example.com,O=ACME Corp",
     "sans":                ["example.com", "www.example.com"],
     "profile":             "standard",
     "requester_ip":        "10.0.0.42",
     "requester_identity":  "10.0.0.42",
     "timestamp":           "2026-04-22T10:00:00Z"
   }
   ```

2. Serialize payload to JSON. Sign with HMAC-SHA256:
   - Read secret from `std::env::var(&config.secret_env)`. Fail to start if missing.
   - `HMAC-SHA256(key=secret, message=payload_json_bytes)`.
   - Set `X-OxCert-Signature: sha256={base64(hmac)}` header.

3. POST to `hook.url` with `Content-Type: application/json`, timeout, and retries.

4. On network failure or non-2xx response:
   - `on_failure = Block`: set `cert.webhook.authorized = "false"`, log the error,
     set error response, return `FLOW_CONTROL_END`.
   - `on_failure = Allow`: log warning, skip this hook, continue.

5. Parse response body:
   - **Authorize / Both:** read `allow` field.
     - If `allow == false`: set `cert.webhook.authorized = "false"`, set 403 response
       with `reason` from webhook, return `FLOW_CONTROL_END`.
   - **Enrich / Both:** read `data` field (JSON object).
     - Merge into `cert.webhook.enrichment` TaskState field as JSON string.
     - `ox_cert_issue` reads this field and applies enriched SANs and custom extensions.

6. After all hooks pass: set `cert.webhook.authorized = "true"`, return
   `FLOW_CONTROL_CONTINUE`.

---

## Enrichment Data Format

`cert.webhook.enrichment` is a JSON object merged from all enrichment hooks:

```json
{
  "additional_sans": ["internal.example.com"],
  "custom_extensions": [
    { "oid": "1.3.6.1.4.1.99999.1", "critical": false, "value_hex": "0101ff" }
  ],
  "subject_ou": "Engineering"
}
```

`ox_cert_issue` reads:
- `additional_sans`: appended to the SAN list from the CSR.
- `custom_extensions`: passed to `CertBuilder::add_extension`.
- `subject_ou`: merged into the distinguished name.

---

## Output TaskState Fields

| Field | Value |
|---|---|
| `cert.webhook.authorized` | `"true"` if all authorize hooks passed; `"false"` if blocked |
| `cert.webhook.enrichment` | JSON object of merged enrichment fields |

---

## Security Notes

- Webhook URLs must be HTTPS. If a URL starts with `http://`, the plugin logs an error
  and refuses to start.
- The HMAC secret is read from an environment variable at init time and never logged.
- Request bodies may contain CSR data including public keys; webhook endpoints should be
  treated as trusted internal services and protected accordingly.

---

## Error Cases

| Condition | Behaviour |
|---|---|
| `secret_env` not set in environment | `ox_plugin_init` returns null; server fails to start |
| Hook URL is HTTP (not HTTPS) | `ox_plugin_init` returns null; server fails to start |
| Network timeout (`on_failure = block`) | 403 `WEBHOOK_REJECTED` |
| Non-2xx response (`on_failure = block`) | 403 `WEBHOOK_REJECTED` |
| `allow: false` in response | 403 `WEBHOOK_REJECTED` with reason from response |
