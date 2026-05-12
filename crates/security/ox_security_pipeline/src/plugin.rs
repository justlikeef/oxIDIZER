use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::net::{IpAddr, Ipv4Addr};
use std::panic;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use ox_security_accounting::MemoryAccountingDriver;
use ox_security_core::{
    AccountingDriver, AuthDriver, AuthPipelineContext, AuthResult, AuthzDriver, AuthzResult,
    Credentials, GroupId, Principal, PrincipalId, TenantId, AuthSource,
};
use crate::{SecurityPipeline, SecurityPipelineBuilder};
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO,
    OX_WORKFLOW_ABI_VERSION,
};

// Config
#[derive(Debug, Deserialize)]
struct ApiKeyEntry {
    key: String,
    principal_id: Uuid,
    display_name: String,
    #[serde(default)]
    groups: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PipelinePluginConfig {
    tenant_id: String,
    #[serde(default)]
    api_keys: Vec<ApiKeyEntry>,
}

// In-memory API key auth driver
struct ApiKeyAuthDriverSimple {
    keys: HashMap<String, (Uuid, String, Vec<String>, String)>,
}

#[async_trait]
impl AuthDriver for ApiKeyAuthDriverSimple {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let key_val = match credentials {
            Credentials::ApiKey { key } => key.expose_secret().to_string(),
            _ => return AuthResult::Continue,
        };
        match self.keys.get(&key_val) {
            Some((pid, name, groups, tenant)) => AuthResult::Authenticated(Principal {
                id: PrincipalId::from_uuid(*pid),
                display_name: name.clone(),
                source: AuthSource::Local,
                groups: groups.iter().map(|g| GroupId::new(g)).collect(),
                tenant_id: TenantId::from(tenant.as_str()),
                session_id: None,
            }),
            None => AuthResult::Continue,
        }
    }
}

// In-memory grant store + authz driver
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GrantEntry {
    pub principal_id: String,
    pub operation: String,
    pub resource_pattern: Option<String>,
}

struct InMemoryAuthzDriver {
    grants: Arc<Mutex<Vec<GrantEntry>>>,
}

#[async_trait]
impl AuthzDriver for InMemoryAuthzDriver {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        let pid = principal.id.as_uuid().to_string();
        let store = self.grants.lock().unwrap_or_else(|p| p.into_inner());
        for entry in store.iter().filter(|e| e.principal_id == pid && e.operation == operation) {
            match &entry.resource_pattern {
                None => return AuthzResult::Allow,
                Some(pat) if pat.ends_with("/*") => {
                    // prefix of "" (from pat "/*") matches all paths — equivalent to resource_pattern: None
                    let prefix = &pat[..pat.len() - 2];
                    if path.starts_with(prefix) { return AuthzResult::Allow; }
                }
                Some(pat) if pat == path => return AuthzResult::Allow,
                _ => {}
            }
        }
        AuthzResult::Continue
    }
}

// Plugin state
struct PluginState {
    api: CoreHostApi,
    config: PipelinePluginConfig,
    pipeline: SecurityPipeline,
    memory_accounting: Arc<MemoryAccountingDriver>,
    grant_store: Arc<Mutex<Vec<GrantEntry>>>,
    runtime: tokio::runtime::Runtime,
}
unsafe impl Send for PluginState {}
unsafe impl Sync for PluginState {}

fn log_msg(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let Ok(k) = CString::new(key) else { return String::new() };
    let ptr = (api.get_field)(task_ctx, k.as_ptr());
    if ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, val: &str) {
    if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(val)) {
        (api.set_field)(task_ctx, k.as_ptr(), v.as_ptr());
    }
}

fn json_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
}

fn handle_authenticate(state: &PluginState, task_ctx: *mut c_void, body: &str) {
    #[derive(Deserialize)]
    struct AuthBody { api_key: String }
    let parsed: AuthBody = match serde_json::from_str(body) {
        Ok(b) => b,
        Err(_) => {
            json_response(&state.api, task_ctx, 400,
                r#"{"error":{"code":"INVALID_REQUEST","message":"expected {\"api_key\":\"...\"}"}}"#);
            return;
        }
    };
    let creds = Credentials::ApiKey { key: secrecy::SecretString::new(parsed.api_key) };
    let mut ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: state.config.tenant_id.parse().unwrap_or_else(|_| TenantId::from("default")),
        source_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
    };
    match state.runtime.block_on(state.pipeline.authenticate(&creds, &mut ctx)) {
        Ok(principal) => {
            let resp = serde_json::json!({
                "data": {
                    "principal_id": principal.id.as_uuid().to_string(),
                    "display_name": principal.display_name,
                    "tenant_id": principal.tenant_id.as_str(),
                    "groups": principal.groups.iter().map(|g| g.as_str()).collect::<Vec<_>>(),
                }
            });
            json_response(&state.api, task_ctx, 200, &resp.to_string());
        }
        Err(e) => {
            let resp = serde_json::json!({"error":{"code":"AUTH_FAILED","message":e.to_string()}});
            json_response(&state.api, task_ctx, 401, &resp.to_string());
        }
    }
}

fn handle_health(state: &PluginState, task_ctx: *mut c_void) {
    json_response(&state.api, task_ctx, 200,
        r#"{"data":{"status":"ok","service":"ox_security_pipeline"}}"#);
}

fn handle_accounting_events(state: &PluginState, task_ctx: *mut c_void) {
    let events = state.memory_accounting.events();
    let parsed: Vec<serde_json::Value> = events.iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect();
    let len = parsed.len();
    let resp = serde_json::json!({"data": parsed, "meta": {"count": len}});
    json_response(&state.api, task_ctx, 200, &resp.to_string());
}

fn handle_grants_list(state: &PluginState, task_ctx: *mut c_void) {
    let grants = state.grant_store.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let len = grants.len();
    let resp = serde_json::json!({"data": grants, "meta": {"count": len}});
    json_response(&state.api, task_ctx, 200, &resp.to_string());
}

fn handle_grants_create(state: &PluginState, task_ctx: *mut c_void, body: &str) {
    let entry: GrantEntry = match serde_json::from_str(body) {
        Ok(e) => e,
        Err(_) => {
            json_response(&state.api, task_ctx, 400,
                r#"{"error":{"code":"INVALID_REQUEST","message":"expected {\"principal_id\":\"...\",\"operation\":\"...\",\"resource_pattern\":null}"}}"#);
            return;
        }
    };
    if entry.principal_id.is_empty() || entry.operation.is_empty() {
        json_response(&state.api, task_ctx, 400,
            r#"{"error":{"code":"INVALID_REQUEST","message":"principal_id and operation are required"}}"#);
        return;
    }
    state.grant_store.lock().unwrap_or_else(|p| p.into_inner()).push(entry.clone());
    let resp = serde_json::json!({"data": entry});
    json_response(&state.api, task_ctx, 201, &resp.to_string());
}

fn handle_grants_delete(state: &PluginState, task_ctx: *mut c_void, principal_id: &str, operation: &str) {
    let mut store = state.grant_store.lock().unwrap_or_else(|p| p.into_inner());
    let before = store.len();
    store.retain(|e| !(e.principal_id == principal_id && e.operation == operation));
    let removed = before - store.len();
    let resp = serde_json::json!({"data": {"removed": removed}});
    json_response(&state.api, task_ctx, 200, &resp.to_string());
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
    let params_str = if config_ptr.is_null() { String::new() } else {
        unsafe { CStr::from_ptr(config_ptr).to_string_lossy().to_string() }
    };
    let params: serde_json::Value = serde_json::from_str(&params_str)
        .unwrap_or(serde_json::Value::Null);
    let config_path = match params.get("config_file").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            log_msg(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                "ox_security_pipeline: missing config_file param");
            return std::ptr::null_mut();
        }
    };
    let config: PipelinePluginConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => {
                log_msg(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_security_pipeline: config error: {}", e));
                return std::ptr::null_mut();
            }
        },
        Err(e) => {
            log_msg(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_security_pipeline: failed to load config: {}", e));
            return std::ptr::null_mut();
        }
    };

    let key_map: HashMap<String, (Uuid, String, Vec<String>, String)> = config.api_keys.iter()
        .map(|e| (e.key.clone(), (e.principal_id, e.display_name.clone(), e.groups.clone(), config.tenant_id.clone())))
        .collect();
    let grant_store: Arc<Mutex<Vec<GrantEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let authz_driver = Arc::new(InMemoryAuthzDriver { grants: grant_store.clone() });
    let memory_accounting = Arc::new(MemoryAccountingDriver::new());

    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            log_msg(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_security_pipeline: runtime error: {}", e));
            return std::ptr::null_mut();
        }
    };

    let pipeline = SecurityPipelineBuilder::new()
        .auth(Arc::new(ApiKeyAuthDriverSimple { keys: key_map }))
        .authz(authz_driver)
        .accounting(memory_accounting.clone() as Arc<dyn AccountingDriver>)
        .build();

    log_msg(&api, std::ptr::null_mut(), OX_LOG_INFO,
        &format!("ox_security_pipeline: initialized for tenant '{}'", config.tenant_id));

    Box::into_raw(Box::new(PluginState { api, config, pipeline, memory_accounting, grant_store, runtime })) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_process(plugin_ctx: *mut c_void, task_ctx: *mut c_void) -> FlowControl {
    let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    if plugin_ctx.is_null() { return cont; }
    let state = unsafe { &*(plugin_ctx as *mut PluginState) };
    panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let method = get_field(&state.api, task_ctx, "request.method").to_uppercase();
        let path   = get_field(&state.api, task_ctx, "request.path");
        let body   = get_field(&state.api, task_ctx, "request.body");
        let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        // Admin routes (/api/v1/admin/*) rely on the host workflow engine for perimeter
        // authentication — the persona YAML routes admin URLs only after the pipeline
        // authenticates the request. No per-handler auth check is needed here.
        match (method.as_str(), segs.get(0).copied(), segs.get(1).copied(),
               segs.get(2).copied(), segs.get(3).copied(), segs.get(4).copied()) {
            ("POST", Some("api"), Some("v1"), Some("security"), Some("authenticate"), None) => {
                handle_authenticate(state, task_ctx, &body);
            }
            ("GET", Some("api"), Some("v1"), Some("security"), Some("health"), None) => {
                handle_health(state, task_ctx);
            }
            ("GET", Some("api"), Some("v1"), Some("admin"), Some("accounting"), Some("events")) => {
                handle_accounting_events(state, task_ctx);
            }
            ("GET", Some("api"), Some("v1"), Some("admin"), Some("authz"), Some("grants")) => {
                handle_grants_list(state, task_ctx);
            }
            ("POST", Some("api"), Some("v1"), Some("admin"), Some("authz"), Some("grants")) => {
                handle_grants_create(state, task_ctx, &body);
            }
            ("DELETE", Some("api"), Some("v1"), Some("admin"), Some("authz"), Some("grants")) => {
                // segs[5] = principal_id, segs[6] = operation
                let pid = segs.get(5).copied().unwrap_or("");
                let op  = segs.get(6).copied().unwrap_or("");
                if pid.is_empty() || op.is_empty() {
                    json_response(&state.api, task_ctx, 400,
                        r#"{"error":{"code":"INVALID_REQUEST","message":"DELETE /api/v1/admin/authz/grants/{principal_id}/{operation} requires both path segments"}}"#);
                } else {
                    handle_grants_delete(state, task_ctx, pid, op);
                }
            }
            _ => {}
        }
        cont
    })).unwrap_or_else(|_| {
        log_msg(&state.api, task_ctx, OX_LOG_ERROR, "ox_security_pipeline: panic in process");
        cont
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_error(_ctx: *mut c_void, _task: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginState)); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No-op stubs for CoreHostApi function pointers — avoids UB from zeroed fn ptrs.
    extern "C" fn stub_get_field(_: *mut c_void, _: *const c_char) -> *const c_char { std::ptr::null() }
    extern "C" fn stub_set_field(_: *mut c_void, _: *const c_char, _: *const c_char) {}
    extern "C" fn stub_get_field_bytes(_: *mut c_void, _: *const c_char, len: *mut usize) -> *const u8 {
        if !len.is_null() { unsafe { *len = 0 }; }
        std::ptr::null()
    }
    extern "C" fn stub_set_field_bytes(_: *mut c_void, _: *const c_char, _: *const u8, _: usize) {}
    extern "C" fn stub_get_metadata(_: *mut c_void, _: *const c_char) -> *const c_char { std::ptr::null() }
    extern "C" fn stub_insert_into_flow(_: *mut c_void, _: *const c_char) -> bool { false }
    extern "C" fn stub_pause_task(_: *mut c_void, _: *const c_char) {}
    extern "C" fn stub_log(_: *mut c_void, _: u8, _: *const c_char) {}
    extern "C" fn stub_set_flag(_: *mut c_void, _: *const c_char, _: u8) {}
    extern "C" fn stub_set_flags(_: *mut c_void, _: *const *const c_char, _: u8) {}
    extern "C" fn stub_has_flag(_: *mut c_void, _: *const c_char, _: u8) -> bool { false }
    extern "C" fn stub_clear_flag(_: *mut c_void, _: *const c_char, _: u8) {}
    extern "C" fn stub_get_keys(_: *mut c_void) -> *const c_char { std::ptr::null() }
    extern "C" fn stub_unset_field(_: *mut c_void, _: *const c_char) -> bool { false }
    extern "C" fn stub_has_field(_: *mut c_void, _: *const c_char) -> bool { false }

    fn dummy_api() -> CoreHostApi {
        CoreHostApi {
            get_field: stub_get_field,
            set_field: stub_set_field,
            get_field_bytes: stub_get_field_bytes,
            set_field_bytes: stub_set_field_bytes,
            get_metadata: stub_get_metadata,
            insert_into_flow: stub_insert_into_flow,
            pause_task: stub_pause_task,
            log: stub_log,
            set_flag: stub_set_flag,
            set_flags: stub_set_flags,
            has_flag: stub_has_flag,
            clear_flag: stub_clear_flag,
            get_keys: stub_get_keys,
            unset_field: stub_unset_field,
            has_field: stub_has_field,
        }
    }

    fn make_config() -> PipelinePluginConfig {
        PipelinePluginConfig {
            tenant_id: "test".to_string(),
            api_keys: vec![ApiKeyEntry {
                key: "testkey".to_string(),
                principal_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                display_name: "Test User".to_string(),
                groups: vec!["admins".to_string()],
            }],
        }
    }

    fn make_test_principal(pid_str: &str, uuid_str: &str) -> Principal {
        Principal {
            id: PrincipalId::from_uuid(Uuid::parse_str(uuid_str).unwrap()),
            display_name: pid_str.to_string(),
            source: AuthSource::Local,
            groups: vec![],
            tenant_id: TenantId::from("test"),
            session_id: None,
        }
    }

    fn make_state() -> PluginState {
        let config = make_config();
        let key_map: HashMap<String, (Uuid, String, Vec<String>, String)> = config.api_keys.iter()
            .map(|e| (e.key.clone(), (e.principal_id, e.display_name.clone(), e.groups.clone(), config.tenant_id.clone())))
            .collect();
        let grant_store: Arc<Mutex<Vec<GrantEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let memory_accounting = Arc::new(MemoryAccountingDriver::new());
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let pipeline = SecurityPipelineBuilder::new()
            .auth(Arc::new(ApiKeyAuthDriverSimple { keys: key_map }))
            .authz(Arc::new(InMemoryAuthzDriver { grants: grant_store.clone() }))
            .accounting(memory_accounting.clone() as Arc<dyn AccountingDriver>)
            .build();
        PluginState {
            api: dummy_api(),
            config,
            pipeline,
            memory_accounting,
            grant_store,
            runtime,
        }
    }

    #[test]
    fn test_config_parses() {
        let json = r#"{"tenant_id":"default","api_keys":[{"key":"testkey","principal_id":"00000000-0000-0000-0000-000000000001","display_name":"Test User","groups":["admins"]}]}"#;
        let cfg: PipelinePluginConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.tenant_id, "default");
        assert_eq!(cfg.api_keys.len(), 1);
    }

    #[test]
    fn test_pipeline_authenticates_valid_api_key() {
        let state = make_state();
        let creds = Credentials::ApiKey { key: secrecy::SecretString::new("testkey".to_string()) };
        let mut ctx = AuthPipelineContext {
            partial_principal: None,
            tenant_id: "test".parse().unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        let result = state.runtime.block_on(state.pipeline.authenticate(&creds, &mut ctx));
        assert!(result.is_ok());
        assert_eq!(result.unwrap().display_name, "Test User");
    }

    #[test]
    fn test_pipeline_rejects_unknown_api_key() {
        let state = make_state();
        let creds = Credentials::ApiKey { key: secrecy::SecretString::new("badkey".to_string()) };
        let mut ctx = AuthPipelineContext {
            partial_principal: None,
            tenant_id: "test".parse().unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        assert!(state.runtime.block_on(state.pipeline.authenticate(&creds, &mut ctx)).is_err());
    }

    #[test]
    fn test_grant_store_crud() {
        let state = make_state();
        let entry = GrantEntry { principal_id: "pid1".to_string(), operation: "read".to_string(), resource_pattern: Some("files/*".to_string()) };
        state.grant_store.lock().unwrap().push(entry);
        assert_eq!(state.grant_store.lock().unwrap().len(), 1);
        state.grant_store.lock().unwrap().retain(|e| !(e.principal_id == "pid1" && e.operation == "read"));
        assert!(state.grant_store.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_authz_driver_wildcard_patterns() {
        // Grant entries use the UUID string form — matching what principal.id.as_uuid().to_string() returns.
        let store: Arc<Mutex<Vec<GrantEntry>>> = Arc::new(Mutex::new(vec![
            GrantEntry { principal_id: "00000000-0000-0000-0000-000000000001".into(), operation: "read".into(), resource_pattern: Some("files/*".into()) },
            GrantEntry { principal_id: "00000000-0000-0000-0000-000000000002".into(), operation: "write".into(), resource_pattern: None },
        ]));
        let driver = InMemoryAuthzDriver { grants: store };
        let principal_1 = make_test_principal("pid1", "00000000-0000-0000-0000-000000000001");
        let principal_2 = make_test_principal("pid2", "00000000-0000-0000-0000-000000000002");

        assert_eq!(driver.check(&principal_1, "files/readme.txt", "read").await, AuthzResult::Allow);
        assert_eq!(driver.check(&principal_1, "other/file.txt", "read").await, AuthzResult::Continue);
        assert_eq!(driver.check(&principal_2, "any/path", "write").await, AuthzResult::Allow);
        assert_eq!(driver.check(&principal_1, "files/readme.txt", "write").await, AuthzResult::Continue);
    }
}
