use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct WebhookConfig {
    pub tenant_id: String,
    pub hooks: Vec<WebhookHookConfig>,
}

#[derive(Debug, Deserialize)]
pub struct WebhookHookConfig {
    pub name: String,
    pub url: String,
    pub hook_type: WebhookType,
    pub secret_env: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_retries")]
    pub retries: u32,
    pub on_failure: WebhookFailureMode,
}

fn default_timeout() -> u64 { 5 }
fn default_retries() -> u32 { 1 }

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum WebhookType {
    Authorize,
    Enrich,
    Both,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum WebhookFailureMode {
    Block,
    Allow,
}

/// A resolved hook — secret bytes pre-loaded at init time.
struct ResolvedHook {
    cfg: WebhookHookConfig,
    secret: Vec<u8>,
}

pub struct WebhookState {
    hooks: Vec<ResolvedHook>,
    tenant_id: String,
}

impl WebhookState {
    pub fn from_config(cfg: WebhookConfig) -> Result<Self, String> {
        let mut hooks = Vec::with_capacity(cfg.hooks.len());
        for hook in cfg.hooks {
            if !hook.url.starts_with("https://") {
                return Err(format!("hook '{}': URL must be HTTPS", hook.name));
            }
            let secret = std::env::var(&hook.secret_env)
                .map_err(|_| format!("hook '{}': env var '{}' not set", hook.name, hook.secret_env))?;
            hooks.push(ResolvedHook { cfg: hook, secret: secret.into_bytes() });
        }
        Ok(Self { hooks, tenant_id: cfg.tenant_id })
    }
}

pub enum ProcessResult {
    Continue,
    Block { http_status: u16, body: String },
}

pub fn process_hooks(state: &WebhookState, request_body: &str, requester_ip: &str) -> ProcessResult {
    let request_id = Uuid::new_v4().to_string();
    let timestamp = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();

    let base_payload = serde_json::json!({
        "event": "certificate_request",
        "tenant_id": state.tenant_id,
        "request_id": request_id,
        "requester_ip": requester_ip,
        "requester_identity": requester_ip,
        "request_body": request_body,
        "timestamp": timestamp,
    });

    let payload_json = base_payload.to_string();

    for hook in &state.hooks {
        let result = call_hook(hook, &payload_json);
        match result {
            Ok(body) => {
                if hook.cfg.hook_type == WebhookType::Authorize || hook.cfg.hook_type == WebhookType::Both {
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    if v.get("allow").and_then(|a| a.as_bool()) == Some(false) {
                        let reason = v.get("reason")
                            .and_then(|r| r.as_str())
                            .unwrap_or("webhook rejected request");
                        return ProcessResult::Block {
                            http_status: 403,
                            body: serde_json::json!({
                                "error": { "code": "WEBHOOK_REJECTED", "message": reason },
                                "meta": { "tenant_id": state.tenant_id }
                            }).to_string(),
                        };
                    }
                }
            }
            Err(e) => {
                match hook.cfg.on_failure {
                    WebhookFailureMode::Block => {
                        return ProcessResult::Block {
                            http_status: 403,
                            body: serde_json::json!({
                                "error": { "code": "WEBHOOK_REJECTED", "message": e },
                                "meta": { "tenant_id": state.tenant_id }
                            }).to_string(),
                        };
                    }
                    WebhookFailureMode::Allow => {}
                }
            }
        }
    }

    ProcessResult::Continue
}

fn call_hook(hook: &ResolvedHook, payload: &str) -> Result<String, String> {
    let signature = hmac_sha256_sign(&hook.secret, payload.as_bytes());
    let sig_header = format!("sha256={}", signature);

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(hook.cfg.timeout_secs))
        .build();

    let mut last_err = String::new();
    for _ in 0..=hook.cfg.retries {
        match agent.post(&hook.cfg.url)
            .set("Content-Type", "application/json")
            .set("X-OxCert-Signature", &sig_header)
            .send_string(payload)
        {
            Ok(resp) => {
                return resp.into_string().map_err(|e| e.to_string());
            }
            Err(e) => { last_err = e.to_string(); }
        }
    }
    Err(last_err)
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
    use std::panic;
    use ox_workflow_abi::{
        CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO,
        OX_WORKFLOW_ABI_VERSION,
    };

    struct PluginCtx {
        api: CoreHostApi,
        state: WebhookState,
    }
    unsafe impl Send for PluginCtx {}
    unsafe impl Sync for PluginCtx {}

    fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
        if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
    }

    fn get(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
        let Ok(k) = CString::new(key) else { return String::new() };
        let ptr = (api.get_field)(task_ctx, k.as_ptr());
        if ptr.is_null() { return String::new(); }
        unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
    }

    fn set(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, val: &str) {
        if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(val)) {
            (api.set_field)(task_ctx, k.as_ptr(), v.as_ptr());
        }
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
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_webhook: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let cfg: WebhookConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_webhook: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_webhook: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        let tenant_id = cfg.tenant_id.clone();
        let state = match WebhookState::from_config(cfg) {
            Ok(s) => s,
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_webhook: init error: {}", e));
                return std::ptr::null_mut();
            }
        };
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_webhook: initialized for tenant '{}'", tenant_id));
        Box::into_raw(Box::new(PluginCtx { api, state })) as *mut c_void
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_process(
        plugin_ctx: *mut c_void,
        task_ctx: *mut c_void,
    ) -> FlowControl {
        use ox_workflow_abi::FLOW_CONTROL_END;
        let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        let end = FlowControl { code: FLOW_CONTROL_END, payload: std::ptr::null() };
        if plugin_ctx.is_null() { return cont; }
        let ctx = unsafe { &*(plugin_ctx as *mut PluginCtx) };

        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let body = get(&ctx.api, task_ctx, "request.body");
            let ip = get(&ctx.api, task_ctx, "request.header.X-Forwarded-For");

            match process_hooks(&ctx.state, &body, &ip) {
                ProcessResult::Continue => {
                    set(&ctx.api, task_ctx, "cert.webhook.authorized", "true");
                    cont
                }
                ProcessResult::Block { http_status, body } => {
                    set(&ctx.api, task_ctx, "cert.webhook.authorized", "false");
                    set(&ctx.api, task_ctx, "response.status", &http_status.to_string());
                    set(&ctx.api, task_ctx, "response.body", &body);
                    set(&ctx.api, task_ctx, "response.header.Content-Type", "application/json");
                    end
                }
            }
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&ctx.api, task_ctx, OX_LOG_ERROR, "ox_cert_webhook: panic");
                cont
            }
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_error(_ctx: *mut c_void, _task: *mut c_void) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
        if !plugin_ctx.is_null() {
            unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginCtx)); }
        }
    }
}
