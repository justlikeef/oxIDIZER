use ox_cert_core::{
    issuer_params_from_cert_pem,
    model::{CertStoreConfig, KeyStoreConfig, RevocationReason as OxRevocationReason},
    open_keystore,
    store::{CertStore, OxPersistenceCertStore},
    CertError,
};
use rcgen::{
    CertificateRevocationListParams, KeyIdMethod, RevokedCertParams,
    RevocationReason as RcgenRevocationReason, SerialNumber, Issuer,
};
use serde::Deserialize;
use std::sync::{Arc, RwLock};
use time::OffsetDateTime;

#[derive(Debug, Deserialize)]
pub struct CrlConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    #[serde(default = "default_update_interval")]
    pub crl_update_interval_secs: u64,
    #[serde(default = "default_delta_interval")]
    pub crl_delta_interval_secs: u64,
    #[serde(default = "default_cache_ttl")]
    pub crl_cache_ttl_secs: u64,
    #[serde(default = "default_lock_ttl")]
    pub crl_lock_ttl_secs: u64,
    pub node_id: Option<String>,
    #[serde(default)]
    pub background_pregenerate: bool,
}

fn default_update_interval() -> u64 { 3600 }
fn default_delta_interval() -> u64 { 600 }
fn default_cache_ttl() -> u64 { 1800 }
fn default_lock_ttl() -> u64 { 300 }

pub struct CachedCrl {
    pub der: Vec<u8>,
    pub pem: String,
    pub next_update: OffsetDateTime,
    pub crl_number: u64,
}

pub struct CrlContext {
    config: CrlConfig,
    full_crl: Arc<RwLock<Option<CachedCrl>>>,
    delta_crl: Arc<RwLock<Option<CachedCrl>>>,
    holder_id: String,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
}
unsafe impl Send for CrlContext {}
unsafe impl Sync for CrlContext {}

impl CrlContext {
    pub fn new(config: CrlConfig) -> Arc<Self> {
        let holder_id = config.node_id.clone().unwrap_or_else(|| {
            format!("{}:{}", hostname(), std::process::id())
        });
        let full_crl = Arc::new(RwLock::new(None));
        let delta_crl = Arc::new(RwLock::new(None));
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let ctx = Arc::new(Self {
            config,
            full_crl,
            delta_crl,
            holder_id,
            shutdown,
        });

        if ctx.config.background_pregenerate {
            let ctx_clone = Arc::clone(&ctx);
            let full_clone = Arc::clone(&ctx.full_crl);
            let delta_clone = Arc::clone(&ctx.delta_crl);
            let update_secs = ctx.config.crl_update_interval_secs;
            let delta_secs = ctx.config.crl_delta_interval_secs;
            let sd = Arc::clone(&ctx.shutdown);
            std::thread::spawn(move || {
                bg_loop(ctx_clone, update_secs, delta_secs, full_clone, delta_clone, sd);
            });
        }

        ctx
    }
}

impl Drop for CrlContext {
    fn drop(&mut self) {
        self.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string())
}

fn bg_loop(
    ctx: Arc<CrlContext>,
    update_secs: u64,
    delta_secs: u64,
    full_crl: Arc<RwLock<Option<CachedCrl>>>,
    delta_crl: Arc<RwLock<Option<CachedCrl>>>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
) {
    let sleep_secs = (update_secs.min(delta_secs) / 2).max(30);
    loop {
        std::thread::sleep(std::time::Duration::from_secs(sleep_secs));
        if shutdown.load(std::sync::atomic::Ordering::Relaxed) { break; }

        let store = match OxPersistenceCertStore::open(ctx.config.store.db_path()) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let now = OffsetDateTime::now_utc();
        let should_full = {
            let guard = full_crl.read().unwrap();
            guard.as_ref().map(|c| c.next_update - ::time::Duration::seconds(60) < now).unwrap_or(true)
        };
        if should_full {
            if let Ok(cached) = generate_crl_with_store(&ctx, &store, update_secs, false) {
                *full_crl.write().unwrap() = Some(cached);
            }
        }

        let should_delta = {
            let guard = delta_crl.read().unwrap();
            guard.as_ref().map(|c| c.next_update - ::time::Duration::seconds(30) < now).unwrap_or(true)
        };
        if should_delta {
            if let Ok(cached) = generate_crl_with_store(&ctx, &store, delta_secs, true) {
                *delta_crl.write().unwrap() = Some(cached);
            }
        }
    }
}

fn ox_reason_to_rcgen(r: OxRevocationReason) -> RcgenRevocationReason {
    match r {
        OxRevocationReason::Unspecified => RcgenRevocationReason::Unspecified,
        OxRevocationReason::KeyCompromise => RcgenRevocationReason::KeyCompromise,
        OxRevocationReason::CaCompromise => RcgenRevocationReason::CaCompromise,
        OxRevocationReason::AffiliationChanged => RcgenRevocationReason::AffiliationChanged,
        OxRevocationReason::Superseded => RcgenRevocationReason::Superseded,
        OxRevocationReason::CessationOfOperation => RcgenRevocationReason::CessationOfOperation,
        OxRevocationReason::CertificateHold => RcgenRevocationReason::CertificateHold,
        OxRevocationReason::RemoveFromCrl => RcgenRevocationReason::RemoveFromCrl,
        OxRevocationReason::PrivilegeWithdrawn => RcgenRevocationReason::PrivilegeWithdrawn,
        OxRevocationReason::AaCompromise => RcgenRevocationReason::AaCompromise,
    }
}

pub fn handle_crl_request(ctx: &CrlContext, path: &str) -> CrlResponse {
    let is_delta = path.contains("delta");
    let is_pem = path.ends_with(".pem");

    let cache = if is_delta { &ctx.delta_crl } else { &ctx.full_crl };
    let lock_key = if is_delta { "delta_crl" } else { "full_crl" };
    let update_secs = if is_delta { ctx.config.crl_delta_interval_secs } else { ctx.config.crl_update_interval_secs };

    // Check cache freshness
    {
        let guard = cache.read().unwrap();
        if let Some(cached) = guard.as_ref() {
            if cached.next_update > OffsetDateTime::now_utc() {
                return serve_cached(cached, is_pem, false);
            }
        }
    }

    // Try to acquire lock and regenerate
    let store = match OxPersistenceCertStore::open(ctx.config.store.db_path()) {
        Ok(s) => s,
        Err(e) => return CrlResponse {
            http_status: 500,
            content_type: "application/json".to_string(),
            body: format!("{{\"error\":\"INTERNAL_ERROR\",\"message\":\"{}\"}}", e).into_bytes(),
            warning: None,
        },
    };

    match store.acquire_crl_lock(&ctx.config.tenant_id, lock_key, &ctx.holder_id, ctx.config.crl_lock_ttl_secs) {
        Ok(Some(_crl_number)) => {
            // We got the lock, regenerate
            match generate_crl_with_store(ctx, &store, update_secs, is_delta) {
                Ok(cached) => {
                    let response = serve_cached(&cached, is_pem, false);
                    *cache.write().unwrap() = Some(cached);
                    let _ = store.release_crl_lock(&ctx.config.tenant_id, lock_key, &ctx.holder_id);
                    response
                }
                Err(e) => CrlResponse {
                    http_status: 500,
                    content_type: "application/json".to_string(),
                    body: format!("{{\"error\":\"INTERNAL_ERROR\",\"message\":\"{}\"}}", e).into_bytes(),
                    warning: None,
                },
            }
        }
        Ok(None) => {
            // Another node holds the lock
            let guard = cache.read().unwrap();
            if let Some(cached) = guard.as_ref() {
                serve_cached(cached, is_pem, true)
            } else {
                CrlResponse {
                    http_status: 503,
                    content_type: "application/json".to_string(),
                    body: b"{\"error\":\"CA_NOT_READY\",\"message\":\"CRL not yet generated\"}".to_vec(),
                    warning: None,
                }
            }
        }
        Err(e) => {
            // Lock error — try from cache
            let guard = cache.read().unwrap();
            if let Some(cached) = guard.as_ref() {
                serve_cached(cached, is_pem, true)
            } else {
                CrlResponse {
                    http_status: 500,
                    content_type: "application/json".to_string(),
                    body: format!("{{\"error\":\"INTERNAL_ERROR\",\"message\":\"{}\"}}", e).into_bytes(),
                    warning: None,
                }
            }
        }
    }
}

fn generate_crl_with_store(
    ctx: &CrlContext,
    store: &OxPersistenceCertStore,
    next_update_secs: u64,
    is_delta: bool,
) -> Result<CachedCrl, CertError> {
    let tenant = &ctx.config.tenant_id;
    let now = OffsetDateTime::now_utc();

    let revoked = if is_delta {
        let since = now - ::time::Duration::seconds(next_update_secs as i64 * 2);
        store.list_revoked_since(tenant, since)?
    } else {
        store.list_revoked(tenant)?
    };

    let ca_cert_pem = std::fs::read_to_string(&ctx.config.ca_intermediate_cert_path)
        .map_err(|e| CertError::Internal(format!("CA cert: {}", e)))?;
    let issuer_params = issuer_params_from_cert_pem(&ca_cert_pem)?;

    let ks = open_keystore(&ctx.config.keystore)?;
    let ca_key_pem = ks.load_key_pem(tenant, &ctx.config.ca_intermediate_key_id)?;
    let ca_keypair = rcgen::KeyPair::from_pem(&ca_key_pem)
        .map_err(|e| CertError::Crypto(e.to_string()))?;

    let revoked_certs: Vec<RevokedCertParams> = revoked.iter().filter_map(|c| {
        let serial_bytes = uuid::Uuid::parse_str(&c.serial).ok()?.as_bytes().to_vec();
        let revocation_time = c.revoked_at.unwrap_or(now);
        let reason_code = c.revocation_reason.map(ox_reason_to_rcgen);
        Some(RevokedCertParams {
            serial_number: SerialNumber::from_slice(&serial_bytes),
            revocation_time,
            reason_code,
            invalidity_date: None,
        })
    }).collect();

    let crl_number = now.unix_timestamp() as u64;
    let next_update = now + ::time::Duration::seconds(next_update_secs as i64);

    let params = CertificateRevocationListParams {
        this_update: now,
        next_update,
        crl_number: SerialNumber::from_slice(&crl_number.to_be_bytes()),
        issuing_distribution_point: None,
        revoked_certs,
        key_identifier_method: KeyIdMethod::Sha256,
    };

    let issuer = Issuer::new(issuer_params, ca_keypair);
    let crl = params.signed_by(&issuer)
        .map_err(|e| CertError::Internal(format!("CRL sign: {}", e)))?;

    let der = crl.der().to_vec();
    let pem = crl.pem()
        .map_err(|e| CertError::Internal(format!("CRL PEM: {}", e)))?;

    Ok(CachedCrl { der, pem, next_update, crl_number })
}

pub struct CrlResponse {
    pub http_status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub warning: Option<String>,
}

fn serve_cached(cached: &CachedCrl, is_pem: bool, stale: bool) -> CrlResponse {
    let warning = if stale {
        Some(r#"199 ox_cert_crl "CRL regeneration in progress; serving cached copy""#.to_string())
    } else {
        None
    };
    if is_pem {
        CrlResponse {
            http_status: 200,
            content_type: "application/x-pem-file".to_string(),
            body: cached.pem.as_bytes().to_vec(),
            warning,
        }
    } else {
        CrlResponse {
            http_status: 200,
            content_type: "application/pkix-crl".to_string(),
            body: cached.der.clone(),
            warning,
        }
    }
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

    struct PluginState {
        api: CoreHostApi,
        ctx: Arc<CrlContext>,
    }
    unsafe impl Send for PluginState {}
    unsafe impl Sync for PluginState {}

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

    fn set_bytes(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, data: &[u8]) {
        if let Ok(k) = CString::new(key) {
            (api.set_field_bytes)(task_ctx, k.as_ptr(), data.as_ptr(), data.len());
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
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_crl: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: CrlConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_crl: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_crl: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        let tenant_id = config.tenant_id.clone();
        let ctx = CrlContext::new(config);
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_crl: initialized for tenant '{}'", tenant_id));
        Box::into_raw(Box::new(PluginState { api, ctx })) as *mut c_void
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_process(
        plugin_ctx: *mut c_void,
        task_ctx: *mut c_void,
    ) -> FlowControl {
        let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        if plugin_ctx.is_null() { return cont; }
        let state = unsafe { &*(plugin_ctx as *mut PluginState) };

        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let method = get(&state.api, task_ctx, "request.method").to_uppercase();
            let path = get(&state.api, task_ctx, "request.path");

            if method != "GET" || !path.starts_with("/crl") { return cont; }

            let response = handle_crl_request(&state.ctx, &path);

            set(&state.api, task_ctx, "response.status", &response.http_status.to_string());
            set(&state.api, task_ctx, "response.header.Content-Type", &response.content_type);
            if let Some(w) = &response.warning {
                set(&state.api, task_ctx, "response.header.Warning", w);
            }

            if response.content_type.starts_with("application/json") {
                if let Ok(s) = std::str::from_utf8(&response.body) {
                    set(&state.api, task_ctx, "response.body", s);
                }
            } else {
                set_bytes(&state.api, task_ctx, "response.body", &response.body);
            }
            cont
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_crl: panic");
                cont
            }
        }
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
