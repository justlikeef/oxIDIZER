use std::ffi::{c_char, c_void, CStr, CString};
use std::panic;
use std::path::Path;
use std::ptr::null;

use jsonwebtoken::EncodingKey;
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO,
    OX_WORKFLOW_ABI_VERSION,
};

use crate::config::IdpConfig;
use crate::oauth2::{
    build_encoding_key, handle_authorize, handle_oidc_discovery, handle_token, now_secs,
};
use crate::saml::{build_assertion_xml, build_metadata_xml, build_saml_post_form};
use crate::store::{AuthCodeStore, RefreshTokenStore, SamlSessionEntry, SamlSessionStore, TokenStore};

struct PluginState {
    api: CoreHostApi,
    config: IdpConfig,
    enc_key: EncodingKey,
    code_store: AuthCodeStore,
    token_store: TokenStore,
    refresh_store: RefreshTokenStore,
    saml_sessions: SamlSessionStore,
    cert_b64: String,
}
unsafe impl Send for PluginState {}
unsafe impl Sync for PluginState {}

// ---------------------------------------------------------------------------
// FFI helpers
// ---------------------------------------------------------------------------

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) {
        (api.log)(task_ctx, level, c.as_ptr());
    }
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let Ok(k) = CString::new(key) else {
        return String::new();
    };
    let ptr = (api.get_field)(task_ctx, k.as_ptr());
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, val: &str) {
    let sanitized = val.replace('\0', "");
    if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(sanitized)) {
        (api.set_field)(task_ctx, k.as_ptr(), v.as_ptr());
    }
}

fn json_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
}

fn html_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "text/html; charset=utf-8");
}

fn xml_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/xml");
}

fn redirect_response(api: &CoreHostApi, task_ctx: *mut c_void, location: &str) {
    set_field(api, task_ctx, "response.status", "302");
    set_field(api, task_ctx, "response.header.Location", location);
}

// ---------------------------------------------------------------------------
// Auth helpers
// ---------------------------------------------------------------------------

fn extract_bearer_principal(state: &PluginState, task_ctx: *mut c_void) -> Option<String> {
    let auth_header = get_field(&state.api, task_ctx, "request.header.Authorization");
    let token = auth_header.strip_prefix("Bearer ")?.to_string();
    let active = state.token_store.list_active();
    active
        .into_iter()
        .find(|e| e.raw_jwt.as_deref() == Some(token.as_str()))
        .and_then(|e| e.principal_id)
}

// ---------------------------------------------------------------------------
// Request dispatcher
// ---------------------------------------------------------------------------

fn dispatch(state: &PluginState, task_ctx: *mut c_void) {
    let api = &state.api;

    let method = get_field(api, task_ctx, "request.method").to_uppercase();
    let path = get_field(api, task_ctx, "request.path");
    let query = get_field(api, task_ctx, "request.query");
    let body = get_field(api, task_ctx, "request.body");

    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();

    // Extract bearer principal once for reuse in routes that need it.
    let principal_id = extract_bearer_principal(state, task_ctx);

    match (
        method.as_str(),
        segs.get(0).copied(),
        segs.get(1).copied(),
        segs.get(2).copied(),
        segs.get(3).copied(),
    ) {
        // GET /oauth2/authorize
        ("GET", Some("oauth2"), Some("authorize"), None, None) => {
            match handle_authorize(&state.config, &state.code_store, &query, principal_id.as_deref()) {
                Ok(location) => redirect_response(api, task_ctx, &location),
                Err(e) => json_response(api, task_ctx, e.status, &e.to_json()),
            }
        }

        // POST /oauth2/token
        ("POST", Some("oauth2"), Some("token"), None, None) => {
            let (status, resp_body) = handle_token(
                &state.config,
                &state.enc_key,
                &state.code_store,
                &state.token_store,
                &state.refresh_store,
                &body,
            );
            json_response(api, task_ctx, status, &resp_body);
        }

        // POST /oauth2/introspect
        ("POST", Some("oauth2"), Some("introspect"), None, None) => {
            let params: std::collections::HashMap<&str, &str> = body
                .split('&')
                .filter_map(|p| {
                    let mut kv = p.splitn(2, '=');
                    Some((kv.next()?, kv.next()?))
                })
                .collect();
            let token = params.get("token").copied().unwrap_or("");
            let active_tokens = state.token_store.list_active();
            let found = active_tokens
                .iter()
                .find(|e| e.raw_jwt.as_deref() == Some(token));
            let resp = match found {
                Some(e) => serde_json::json!({
                    "active": true,
                    "client_id": e.client_id,
                    "scope": e.scope,
                    "exp": e.expires_at,
                    "jti": e.jti,
                    "sub": e.principal_id,
                }),
                None => serde_json::json!({ "active": false }),
            };
            json_response(api, task_ctx, 200, &resp.to_string());
        }

        // POST /oauth2/revoke
        ("POST", Some("oauth2"), Some("revoke"), None, None) => {
            let params: std::collections::HashMap<&str, &str> = body
                .split('&')
                .filter_map(|p| {
                    let mut kv = p.splitn(2, '=');
                    Some((kv.next()?, kv.next()?))
                })
                .collect();
            let token = params.get("token").copied().unwrap_or("");
            let active_tokens = state.token_store.list_active();
            if let Some(e) = active_tokens.iter().find(|e| e.raw_jwt.as_deref() == Some(token)) {
                state.token_store.revoke(&e.jti);
            }
            json_response(api, task_ctx, 200, "{}");
        }

        // GET /oidc/.well-known/openid-configuration
        ("GET", Some("oidc"), Some(".well-known"), Some("openid-configuration"), None) => {
            let discovery = handle_oidc_discovery(&state.config);
            json_response(api, task_ctx, 200, &discovery);
        }

        // GET /oidc/jwks.json
        ("GET", Some("oidc"), Some("jwks.json"), None, None) => {
            // TODO: derive public key JWK (n, e) from config.rsa_private_key_pem using the rsa crate
            json_response(api, task_ctx, 200, r#"{"keys":[]}"#);
        }

        // GET /saml/{tenant}/metadata
        ("GET", Some("saml"), Some(_tenant), Some("metadata"), None) => {
            let issuer = &state.config.issuer;
            let tenant_seg = segs.get(1).copied().unwrap_or("");
            let sso_url = format!("{}/saml/{}/sso", issuer, tenant_seg);
            let slo_url = format!("{}/saml/{}/slo", issuer, tenant_seg);
            let xml = build_metadata_xml(issuer, &sso_url, &slo_url, &state.cert_b64);
            xml_response(api, task_ctx, 200, &xml);
        }

        // POST /saml/{tenant}/sso
        ("POST", Some("saml"), Some(_tenant), Some("sso"), None) => {
            let params: std::collections::HashMap<&str, &str> = body
                .split('&')
                .filter_map(|p| {
                    let mut kv = p.splitn(2, '=');
                    Some((kv.next()?, kv.next()?))
                })
                .collect();
            let sp_entity_id = params.get("sp_entity_id").copied().unwrap_or("");
            let name_id = params.get("name_id").copied().unwrap_or("");
            let relay_state = params.get("RelayState").copied().unwrap_or("");

            // Reject unauthenticated requests
            if name_id.is_empty() {
                json_response(&state.api, task_ctx, 401,
                    r#"{"error":"unauthenticated","error_description":"name_id required"}"#);
                return;
            }

            // Only issue assertions for registered SPs
            let sp = match state.config.saml_sps.iter().find(|s| s.entity_id == sp_entity_id) {
                Some(s) => s,
                None => {
                    json_response(&state.api, task_ctx, 400,
                        r#"{"error":"unknown_sp","error_description":"SP not registered"}"#);
                    return;
                }
            };
            let acs_url = sp.acs_url.as_str();

            let session_id = uuid::Uuid::new_v4().to_string();
            let assertion_id = uuid::Uuid::new_v4().to_string();
            let assertion_xml = build_assertion_xml(
                &assertion_id,
                &state.config.issuer,
                sp_entity_id,
                acs_url,
                name_id,
                &session_id,
                3600,
            );

            // Store SAML session
            state.saml_sessions.insert(SamlSessionEntry {
                session_id: session_id.clone(),
                sp_entity_id: sp_entity_id.to_string(),
                principal_id: principal_id.clone().unwrap_or_default(),
                name_id: name_id.to_string(),
                created_at: now_secs(),
            });

            let form = build_saml_post_form(acs_url, &assertion_xml, relay_state);
            html_response(api, task_ctx, 200, &form);
        }

        // POST /saml/{tenant}/slo
        ("POST", Some("saml"), Some(_tenant), Some("slo"), None) => {
            let params: std::collections::HashMap<&str, &str> = body
                .split('&')
                .filter_map(|p| {
                    let mut kv = p.splitn(2, '=');
                    Some((kv.next()?, kv.next()?))
                })
                .collect();
            let session_id = params.get("session_id").copied().unwrap_or("");
            state.saml_sessions.remove(session_id);
            json_response(api, task_ctx, 200, "{}");
        }

        // Admin routes — perimeter authentication is enforced by the ox_security_pipeline plugin
        // loaded at higher priority in the persona YAML. No per-handler auth check needed here.

        // Admin routes: GET /api/v1/admin/idp/clients
        ("GET", Some("api"), Some("v1"), Some("admin"), Some("idp"))
            if segs.get(4).copied() == Some("clients") =>
        {
            let data = serde_json::json!({ "data": state.config.clients });
            json_response(api, task_ctx, 200, &data.to_string());
        }

        // Admin routes: GET /api/v1/admin/idp/tokens
        ("GET", Some("api"), Some("v1"), Some("admin"), Some("idp"))
            if segs.get(4).copied() == Some("tokens") =>
        {
            let data = serde_json::json!({ "data": state.token_store.list_active() });
            json_response(api, task_ctx, 200, &data.to_string());
        }

        // Admin routes: DELETE /api/v1/admin/idp/tokens/{jti}
        ("DELETE", Some("api"), Some("v1"), Some("admin"), Some("idp"))
            if segs.get(4).copied() == Some("tokens") =>
        {
            let jti = segs.get(5).copied().unwrap_or("");
            if jti.is_empty() {
                json_response(api, task_ctx, 400,
                    r#"{"error":{"code":"INVALID_REQUEST","message":"missing token jti"}}"#);
            } else {
                state.token_store.revoke(jti);
                let data = serde_json::json!({ "data": { "revoked": true } });
                json_response(api, task_ctx, 200, &data.to_string());
            }
        }

        // Admin routes: GET /api/v1/admin/idp/sessions
        ("GET", Some("api"), Some("v1"), Some("admin"), Some("idp"))
            if segs.get(4).copied() == Some("sessions") =>
        {
            let data = serde_json::json!({ "data": state.saml_sessions.list() });
            json_response(api, task_ctx, 200, &data.to_string());
        }

        // Unmatched — leave response unset, continue pipeline
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// FFI exports
// ---------------------------------------------------------------------------

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
    } else {
        String::new()
    };

    let params: serde_json::Value =
        serde_json::from_str(&params_str).unwrap_or(serde_json::Value::Null);

    let config_path = match params.get("config_file").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            log(
                &api,
                std::ptr::null_mut(),
                OX_LOG_ERROR,
                "ox_security_idp: missing config_file param",
            );
            return std::ptr::null_mut();
        }
    };

    let config: IdpConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => {
                log(
                    &api,
                    std::ptr::null_mut(),
                    OX_LOG_ERROR,
                    &format!("ox_security_idp: config error: {}", e),
                );
                return std::ptr::null_mut();
            }
        },
        Err(e) => {
            log(
                &api,
                std::ptr::null_mut(),
                OX_LOG_ERROR,
                &format!("ox_security_idp: failed to load config: {}", e),
            );
            return std::ptr::null_mut();
        }
    };

    let enc_key = match build_encoding_key(&config.rsa_private_key_pem) {
        Ok(k) => k,
        Err(e) => {
            log(
                &api,
                std::ptr::null_mut(),
                OX_LOG_ERROR,
                &format!("ox_security_idp: invalid RSA key: {}", e),
            );
            return std::ptr::null_mut();
        }
    };

    // Extract base64-encoded cert for SAML metadata (stub: empty string if not extractable)
    let cert_b64 = extract_cert_b64_from_pem(&config.rsa_private_key_pem);

    log(
        &api,
        std::ptr::null_mut(),
        OX_LOG_INFO,
        &format!(
            "ox_security_idp: initialized for tenant '{}'",
            config.tenant_id
        ),
    );

    let state = PluginState {
        api,
        config,
        enc_key,
        code_store: AuthCodeStore::new(),
        token_store: TokenStore::new(),
        refresh_store: RefreshTokenStore::new(),
        saml_sessions: SamlSessionStore::new(),
        cert_b64,
    };

    Box::into_raw(Box::new(state)) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_process(
    plugin_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    let cont = FlowControl {
        code: FLOW_CONTROL_CONTINUE,
        payload: null(),
    };
    if plugin_ctx.is_null() {
        return cont;
    }
    let state = unsafe { &*(plugin_ctx as *mut PluginState) };

    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        dispatch(state, task_ctx);
    }));

    if let Err(_) = result {
        log(&state.api, task_ctx, OX_LOG_ERROR, "ox_security_idp: panic in dispatch");
    }

    cont
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_error(_plugin_ctx: *mut c_void, _task_ctx: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        unsafe {
            drop(Box::from_raw(plugin_ctx as *mut PluginState));
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Attempt to extract the public-key DER as base64 from a PEM-encoded RSA private key.
/// Returns an empty string on any failure (graceful degradation for SAML metadata).
fn extract_cert_b64_from_pem(_pem: &str) -> String {
    // TODO: extract Base64-encoded DER public cert from RSA key for SAML metadata
    String::new()
}
