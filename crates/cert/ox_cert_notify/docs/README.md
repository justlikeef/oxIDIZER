# ox_cert_notify

Proactive certificate expiration notifications and lifecycle alerts, delivered via
webhook, MQTT, or email on a configurable cron schedule. Runs entirely in a background
thread — not request-driven.

---

## Phase

`PreEarlyRequest` — background/scheduled. `ox_plugin_process` returns `FLOW_CONTROL_CONTINUE` immediately.

## Routes

None.

---

## Config Reference

```rust
pub struct NotifyConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub schedule: String,               // cron expression, e.g. "0 8 * * *"
    pub thresholds_days: Vec<u32>,      // e.g. [90, 60, 30, 14, 7, 1]
    pub include_ca_certs: bool,
    pub channels: Vec<NotifyChannelConfig>,
}

pub enum NotifyChannelConfig {
    Webhook { url, secret_env, timeout_secs },
    Mqtt { topic },
    Email { smtp_host, smtp_port, from, to, password_env },
}
```

| Field | Default | Description |
|---|---|---|
| `schedule` | required | Cron expression for sweep timing (UTC) |
| `thresholds_days` | required | Days-before-expiry thresholds to notify at |
| `include_ca_certs` | `false` | Also notify for CA cert expiry |
| `channels` | required | List of delivery channels |

Invalid cron expressions cause `ox_plugin_init` to return null and the server fails to start.

---

## Sweep Logic

On each scheduled run:

1. For each threshold `t` in `thresholds_days`:
   - Query `store.list_expiring(tenant_id, t)` for certs expiring within `t` days.
   - For each cert, check `store.was_notification_sent(tenant_id, serial, t)`. Skip if
     already sent within the last `t/2` days (deduplication).
   - Build payload and deliver to all configured channels.
   - Store `NotificationRecord` with `status = Sent` or `Failed`.
2. If `include_ca_certs`: repeat for active CA keys approaching expiry.
3. Call `store.update_status_expired(tenant_id)` to bulk-mark past-expiry certs.

---

## Notification Payload

```json
{
  "event":          "cert_expiring",
  "tenant_id":      "acme-corp",
  "serial":         "uuid",
  "subject_cn":     "example.com",
  "sans":           ["example.com"],
  "not_after":      "2026-05-22T00:00:00Z",
  "days_remaining": 30
}
```

CA cert notifications use `"event": "ca_cert_expiring"`.

---

## Channel Delivery

**Webhook:** POST with optional HMAC-SHA256 signature via `X-OxCert-Signature` header.
Fire-and-forget; failures are logged but not retried until the next sweep.

**MQTT:** Publish to configured topic via `CoreHostApi::publish_to_topic` (if available)
or a local MQTT client.

**Email:** Delivered via SMTP using `lettre`. Subject line:
`"Certificate Expiring: {subject_cn} in {days} days"`.

---

## Error Cases

| Condition | Behavior |
|---|---|
| Invalid cron expression | Startup failure |
| Webhook delivery failure | Log `WARN`; record `status = Failed` |
| MQTT publish failure | Log `WARN`; record `status = Failed` |
| Email delivery failure | Log `WARN`; record `status = Failed` |
| Storage failure during sweep | Log `ERROR`; sweep continues for remaining certs |

---

## Implementation Notes

- The background thread is started in `ox_plugin_init` with a shutdown `AtomicBool`.
  `ox_plugin_destroy` sets the flag and joins the thread.
- The sweep restarts cleanly after a server restart — deduplication prevents duplicate
  notifications even if a threshold boundary was crossed during downtime.
- Email delivery is feature-gated (`lettre` feature); the plugin builds without it if
  SMTP support is not needed.
