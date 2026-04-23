# ox_cert_notify

**Purpose:** Proactive certificate expiration notifications and lifecycle event alerts,
delivered via webhook, MQTT, or email on a configurable cron schedule.

---

## Phase
`PreEarlyRequest` — background/scheduled module. Not request-driven.

## Routes
None. `ox_plugin_process` returns `FLOW_CONTROL_CONTINUE` immediately.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `CertStore`, `NotificationRecord`, `AuditEvent`, `CertError` |
| `cron` | Parse cron expressions (e.g., `"0 8 * * *"`) |
| `reqwest` (blocking) | Webhook delivery |
| `lettre` | SMTP email delivery (optional; feature-gated) |
| `serde` / `serde_json` | Notification payload serialization |
| `time` | Schedule evaluation, expiry threshold computation |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct NotifyConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    /// Cron expression for when to run notifications (e.g., "0 8 * * *" = daily at 08:00 UTC).
    pub schedule: String,
    /// Days-before-expiry thresholds to notify at (e.g., [90, 60, 30, 14, 7, 1]).
    pub thresholds_days: Vec<u32>,
    /// Also notify for CA cert expiry.
    pub include_ca_certs: bool,
    pub channels: Vec<NotifyChannelConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotifyChannelConfig {
    Webhook {
        url: String,
        /// Env var name holding the HMAC-SHA256 signing secret (optional).
        secret_env: Option<String>,
        timeout_secs: u64,          // default: 10
    },
    Mqtt {
        topic: String,
    },
    Email {
        smtp_host: String,
        smtp_port: u16,
        from: String,
        to: Vec<String>,
        /// Env var name holding SMTP password (optional for unauthenticated relay).
        password_env: Option<String>,
    },
}
```

---

## ModuleContext

```rust
pub struct NotifyContext {
    api: CoreHostApi,
    store: Arc<dyn CertStore>,
    config: NotifyConfig,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    _handle: Option<std::thread::JoinHandle<()>>,
}
```

---

## Processing (background thread)

On `ox_plugin_init`:
1. Parse the cron expression with the `cron` crate into a `Schedule`.
2. Spawn a background thread that loops:
   ```
   loop:
       next = schedule.upcoming(Utc).next()
       sleep until next
       if shutdown: break
       run_notification_sweep()
   ```

### `run_notification_sweep()`

For each threshold `t` in `config.thresholds_days`:

1. `store.list_expiring(tenant_id, t)` — certs expiring within `t` days.
2. For each cert:
   a. `store.was_notification_sent(tenant_id, &cert.serial, t)` — skip if already sent.
   b. Build notification payload:
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
   c. Deliver to each configured channel (see below).
   d. `store.store_notification(tenant_id, &NotificationRecord { ... status: Sent/Failed })`.
   e. `store.store_audit_event(...)` for audit trail.

3. If `include_ca_certs`: repeat for CA cert records from `ca_keys` table where
   `status = "active"` and `not_after` is within `t` days. Uses a raw_sql call to
   `CertStore` since `list_expiring` targets the `certificates` table. Payload includes
   `"event": "ca_cert_expiring"`.

4. `store.update_status_expired(tenant_id)` — bulk-mark certs past `not_after` as Expired.

### Channel Delivery

**Webhook:**
1. POST payload JSON to `url`.
2. If `secret_env` is set: sign with HMAC-SHA256, set `X-OxCert-Signature` header.
3. Log failure; do not retry (will retry on next sweep if `was_notification_sent` was
   not recorded).

**MQTT:**
1. Publish payload JSON to configured `topic` using `ox_messaging_mqtt` client
   (or a local MQTT client if `CoreHostApi::publish_to_topic` is available).
2. Fire-and-forget.

**Email:**
1. Build email via `lettre`: subject `"Certificate Expiring: {subject_cn} in {days} days"`.
2. Deliver via SMTP. Log failure.

---

## Deduplication

`store.was_notification_sent(tenant_id, serial, threshold_days)` checks the
`notification_log` table for a row with matching `(tenant_id, serial, threshold_days)`
and `status = "sent"`. Sent within the past `threshold_days / 2` days qualifies as a
duplicate. This prevents re-sending if the sweep runs multiple times (e.g., after
a server restart during the same day).

---

## Error Cases

| Condition | Behaviour |
|---|---|
| Invalid cron expression | `ox_plugin_init` returns null; server fails to start |
| Webhook delivery failure | Logged as `WARN`; `NotificationRecord.status = Failed` |
| MQTT publish failure | Logged as `WARN`; `NotificationRecord.status = Failed` |
| Email delivery failure | Logged as `WARN`; `NotificationRecord.status = Failed` |
| Storage failure | Logged as `ERROR`; sweep continues for other certs |
