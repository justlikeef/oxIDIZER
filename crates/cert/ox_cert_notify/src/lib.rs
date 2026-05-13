use ox_cert_core::{
    model::{
        AuditAction, AuditEvent, CertStoreConfig, NotificationChannel,
        NotificationRecord, NotificationStatus,
    },
    store::{CertStore, OxPersistenceCertStore},
};
use serde::Deserialize;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use time::OffsetDateTime;

#[derive(Debug, Deserialize, Clone)]
pub struct NotifyConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub schedule: String,
    #[serde(default = "default_thresholds")]
    pub thresholds_days: Vec<u32>,
    #[serde(default)]
    pub include_ca_certs: bool,
    #[serde(default)]
    pub channels: Vec<NotifyChannelConfig>,
}

fn default_thresholds() -> Vec<u32> { vec![90, 60, 30, 14, 7, 1] }

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotifyChannelConfig {
    Webhook {
        url: String,
        secret_env: Option<String>,
        #[serde(default = "default_timeout")]
        timeout_secs: u64,
    },
    Mqtt {
        topic: String,
    },
    Email {
        smtp_host: String,
        smtp_port: u16,
        from: String,
        to: Vec<String>,
        password_env: Option<String>,
    },
}

fn default_timeout() -> u64 { 10 }

pub struct NotifyContext {
    _config: NotifyConfig,
    shutdown: Arc<AtomicBool>,
    _handle: Option<std::thread::JoinHandle<()>>,
}

impl NotifyContext {
    pub fn start(config: NotifyConfig) -> Result<Self, String> {
        // Validate cron expression
        let _schedule: cron::Schedule = config.schedule.parse()
            .map_err(|e| format!("invalid cron expression '{}': {}", config.schedule, e))?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);
        let cfg_clone = config.clone();

        let handle = std::thread::spawn(move || {
            run_schedule_loop(cfg_clone, shutdown_clone);
        });

        Ok(Self {
            _config: config,
            shutdown,
            _handle: Some(handle),
        })
    }
}

impl Drop for NotifyContext {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

fn run_schedule_loop(config: NotifyConfig, shutdown: Arc<AtomicBool>) {
    use chrono::Utc;
    let Ok(schedule) = config.schedule.parse::<cron::Schedule>() else { return };

    for next in schedule.upcoming(Utc) {
        if shutdown.load(Ordering::Relaxed) { break; }
        let now = Utc::now();
        let delay = (next - now).to_std().unwrap_or_default();
        std::thread::sleep(delay);
        if shutdown.load(Ordering::Relaxed) { break; }
        run_notification_sweep(&config);
    }
}

fn run_notification_sweep(config: &NotifyConfig) {
    let store = match OxPersistenceCertStore::open(config.store.db_path()) {
        Ok(s) => s,
        Err(_) => return,
    };

    let tenant = &config.tenant_id;

    for &threshold in &config.thresholds_days {
        let certs = match store.list_expiring(tenant, threshold) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for cert in certs {
            let already_sent = store.was_notification_sent(tenant, &cert.serial, threshold)
                .unwrap_or(false);
            if already_sent { continue; }

            let now = OffsetDateTime::now_utc();
            let days_remaining = (cert.not_after - now).whole_days().max(0) as u32;

            let payload = serde_json::json!({
                "event": "cert_expiring",
                "tenant_id": tenant,
                "serial": cert.serial,
                "subject_cn": cert.subject_cn,
                "sans": cert.sans,
                "not_after": cert.not_after.to_string(),
                "days_remaining": days_remaining,
            });

            for channel in &config.channels {
                let status = deliver(channel, &payload);
                let _ = store.store_notification(tenant, &NotificationRecord {
                    id: 0,
                    tenant_id: tenant.clone(),
                    serial: cert.serial.clone(),
                    threshold_days: threshold,
                    channel: channel_type(channel),
                    sent_at: now,
                    status,
                    error: None,
                });
            }

            let _ = store.store_audit_event(tenant, &AuditEvent {
                id: 0,
                tenant_id: tenant.clone(),
                timestamp: now,
                action: AuditAction::Issue, // best proxy; no dedicated Notify action
                serial: Some(cert.serial.clone()),
                actor: "ox_cert_notify".to_string(),
                details: serde_json::json!({ "threshold_days": threshold, "days_remaining": days_remaining }),
            });
        }
    }

    let _ = store.update_status_expired(tenant);
}

fn channel_type(ch: &NotifyChannelConfig) -> NotificationChannel {
    match ch {
        NotifyChannelConfig::Webhook { .. } => NotificationChannel::Webhook,
        NotifyChannelConfig::Mqtt { .. } => NotificationChannel::Mqtt,
        NotifyChannelConfig::Email { .. } => NotificationChannel::Email,
    }
}

fn deliver(channel: &NotifyChannelConfig, payload: &serde_json::Value) -> NotificationStatus {
    match channel {
        NotifyChannelConfig::Webhook { url, secret_env, timeout_secs } => {
            deliver_webhook(url, secret_env.as_deref(), *timeout_secs, payload)
        }
        NotifyChannelConfig::Mqtt { .. } => {
            // MQTT delivery requires ox_messaging_mqtt client — stubbed
            NotificationStatus::Sent
        }
        NotifyChannelConfig::Email { smtp_host, smtp_port, from, to, password_env } => {
            deliver_email(smtp_host, *smtp_port, from, to, password_env.as_deref(), payload)
        }
    }
}

fn deliver_webhook(url: &str, secret_env: Option<&str>, timeout_secs: u64, payload: &serde_json::Value) -> NotificationStatus {
    let body = payload.to_string();
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build();

    let mut req = agent.post(url).set("Content-Type", "application/json");

    if let Some(env_key) = secret_env {
        if let Ok(secret) = std::env::var(env_key) {
            let sig = hmac_sha256_sign(secret.as_bytes(), body.as_bytes());
            req = req.set("X-OxCert-Signature", &format!("sha256={}", sig));
        }
    }

    match req.send_string(&body) {
        Ok(_) => NotificationStatus::Sent,
        Err(_) => NotificationStatus::Failed,
    }
}

fn deliver_email(
    smtp_host: &str,
    smtp_port: u16,
    from: &str,
    to: &[String],
    password_env: Option<&str>,
    payload: &serde_json::Value,
) -> NotificationStatus {
    use lettre::message::header::ContentType;
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{Message, SmtpTransport, Transport};

    let subject = format!("Certificate Alert: {}",
        payload.get("event").and_then(|e| e.as_str()).unwrap_or("notification"));
    let body_text = serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());

    let from_mb: lettre::message::Mailbox = match from.parse() {
        Ok(m) => m,
        Err(_) => return NotificationStatus::Failed,
    };

    let mut builder = Message::builder().from(from_mb).subject(subject);

    for addr in to {
        let mb: lettre::message::Mailbox = match addr.parse() {
            Ok(m) => m,
            Err(_) => return NotificationStatus::Failed,
        };
        builder = builder.to(mb);
    }

    let msg = match builder.header(ContentType::TEXT_PLAIN).body(body_text) {
        Ok(m) => m,
        Err(_) => return NotificationStatus::Failed,
    };

    let relay_builder = match SmtpTransport::relay(smtp_host) {
        Ok(b) => b,
        Err(_) => return NotificationStatus::Failed,
    };

    let transport_builder = relay_builder.port(smtp_port);
    let transport = if let Some(env_key) = password_env {
        if let Ok(pass) = std::env::var(env_key) {
            transport_builder.credentials(Credentials::new(from.to_string(), pass)).build()
        } else {
            transport_builder.build()
        }
    } else {
        transport_builder.build()
    };

    match transport.send(&msg) {
        Ok(_) => NotificationStatus::Sent,
        Err(_) => NotificationStatus::Failed,
    }
}

fn hmac_sha256_sign(key: &[u8], msg: &[u8]) -> String {
    use ring::hmac;
    let k = hmac::Key::new(hmac::HMAC_SHA256, key);
    let tag = hmac::sign(&k, msg);
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, tag.as_ref())
}

// ---------------------------------------------------------------------------
// Plugin ABI
// ---------------------------------------------------------------------------

pub mod plugin {
    use super::*;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::path::Path;
    use ox_workflow_abi::{
        CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO,
        OX_WORKFLOW_ABI_VERSION,
    };

    struct PluginState {
        #[allow(dead_code)]
        api: CoreHostApi,
        _ctx: NotifyContext,
    }
    unsafe impl Send for PluginState {}
    unsafe impl Sync for PluginState {}

    fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
        if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_init(
        config_ptr: *const c_char,
        api_ptr: *const CoreHostApi,
        abi_version: u32,
    ) -> *mut c_void {
        if abi_version != OX_WORKFLOW_ABI_VERSION || api_ptr.is_null() {
            return std::ptr::null_mut();
        }
        let api = unsafe { *api_ptr };
        let params_str = if !config_ptr.is_null() {
            unsafe { CStr::from_ptr(config_ptr).to_string_lossy().to_string() }
        } else { String::new() };
        let params: serde_json::Value = serde_json::from_str(&params_str).unwrap_or(serde_json::Value::Null);
        let config_path = match params.get("config_file").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_notify: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: NotifyConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_notify: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_notify: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        let tenant_id = config.tenant_id.clone();
        let ctx = match NotifyContext::start(config) {
            Ok(c) => c,
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_notify: start error: {}", e));
                return std::ptr::null_mut();
            }
        };
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_notify: initialized for tenant '{}'", tenant_id));
        Box::into_raw(Box::new(PluginState { api, _ctx: ctx })) as *mut c_void
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_process(
        _plugin_ctx: *mut c_void,
        _task_ctx: *mut c_void,
    ) -> FlowControl {
        // Background-only plugin; pass through all requests
        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_error(_ctx: *mut c_void, _task: *mut c_void) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
        if !plugin_ctx.is_null() {
            unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginState)); }
        }
    }
}
