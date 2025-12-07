use std::error::Error;
use std::fmt;
use bumpalo::Bump;


#[derive(Debug)]
enum ConfigError {
    NotFound,
    ReadError(std::io::Error),
    ParseError(String),
    UnsupportedFormat,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ConfigError::NotFound => write!(f, "Configuration file not found"),
            ConfigError::ReadError(e) => write!(f, "Error reading configuration file: {}", e),
            ConfigError::ParseError(e) => write!(f, "Error parsing configuration file: {}", e),
            ConfigError::UnsupportedFormat => write!(f, "Unsupported configuration file format"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ConfigError::ReadError(e) => Some(e),
            _ => None,
        }
    }
}

use axum::{
    body::Body,
    http::{Request, Response, StatusCode, HeaderMap},
    routing::get,
    extract::ConnectInfo,
    Router,
};
use clap::Parser;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::{CString, c_void, CStr};
use std::fs::File;
use std::io::{Read, BufReader};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::net::TcpListener;
use tera::Tera;
use axum_server::bind_rustls;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::{certs, pkcs8_private_keys};
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use rustls::server::{ClientHello, ResolvesServerCert, ResolvesServerCertUsingSni};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig as RustlsServerConfig;
use axum_server::tls_rustls::RustlsConfig;
use log::{info, debug, trace, error, warn};

use ox_webservice_api::{
    ModuleConfig, InitializeModuleFn, Phase, HandlerResult,
    ModuleInterface, WebServiceApiV1, UriMatcher,
    PipelineState,
    LogLevel,
};

use libloading::{Library, Symbol};
use once_cell::sync::Lazy;
use regex::Regex;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn alloc_str_c(arena: *const c_void, s: *const libc::c_char) -> *mut libc::c_char { unsafe {
    let arena = &*(arena as *const Bump);
    let s = CStr::from_ptr(s).to_str().unwrap();
    let c_string = CString::new(s).unwrap();
    let ptr = c_string.as_ptr() as *mut libc::c_char;
    let len = c_string.as_bytes_with_nul().len();
    let allocated = arena.alloc_slice_copy(std::slice::from_raw_parts(ptr as *const u8, len));
    allocated.as_mut_ptr() as *mut libc::c_char
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn alloc_raw_c(arena: *mut c_void, size: usize, align: usize) -> *mut c_void { unsafe {
    let arena = &mut *(arena as *mut Bump);
    let layout = std::alloc::Layout::from_size_align(size, align).unwrap();
    arena.alloc_layout(layout).as_ptr() as *mut c_void
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_module_context_value_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let key = CStr::from_ptr(key).to_str().unwrap();
    let pipeline_state = &mut *pipeline_state_ptr;
    let value = pipeline_state.module_context.read().unwrap().get(key).cloned();
    if let Some(value) = value {
        let value_str = value.to_string();
        alloc_fn(arena, CString::new(value_str).unwrap().as_ptr())
    } else {
        std::ptr::null_mut()
    }
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_module_context_value_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    value_json: *const libc::c_char,
) { unsafe {
    let key = CStr::from_ptr(key).to_str().unwrap().to_string();
    let value_json = CStr::from_ptr(value_json).to_str().unwrap();
    let value: Value = serde_json::from_str(value_json).unwrap();
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.module_context.write().unwrap().insert(key, value);
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_method_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    alloc_fn(
        arena,
        CString::new(pipeline_state.request_method.as_str())
            .unwrap()
            .as_ptr(),
    )
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_path_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    alloc_fn(
        arena,
        CString::new(pipeline_state.request_path.as_str())
            .unwrap()
            .as_ptr(),
    )
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_query_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    alloc_fn(
        arena,
        CString::new(pipeline_state.request_query.as_str())
            .unwrap()
            .as_ptr(),
    )
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_header_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let key = CStr::from_ptr(key).to_str().unwrap();
    let pipeline_state = &*pipeline_state_ptr;
    if let Some(value) = pipeline_state.request_headers.get(key) {
        alloc_fn(
            arena,
            CString::new(value.to_str().unwrap()).unwrap().as_ptr(),
        )
    } else {
        std::ptr::null_mut()
    }
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_headers_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    let headers: HashMap<String, String> = pipeline_state
        .request_headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
        .collect();
    let headers_json = serde_json::to_string(&headers).unwrap();
    alloc_fn(arena, CString::new(headers_json).unwrap().as_ptr())
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_body_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    let body_str = String::from_utf8_lossy(&pipeline_state.request_body);
    alloc_fn(arena, CString::new(body_str.as_ref()).unwrap().as_ptr())
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_source_ip_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    alloc_fn(
        arena,
        CString::new(pipeline_state.source_ip.to_string())
            .unwrap()
            .as_ptr(),
    )
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_request_path_c(
    pipeline_state_ptr: *mut PipelineState,
    path: *const libc::c_char,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.request_path = CStr::from_ptr(path).to_str().unwrap().to_string();
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_request_header_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    value: *const libc::c_char,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    let key = CStr::from_ptr(key).to_str().unwrap();
    let value = CStr::from_ptr(value).to_str().unwrap();
    pipeline_state
        .request_headers
        .insert(key, value.parse().unwrap());
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_source_ip_c(
    pipeline_state_ptr: *mut PipelineState,
    ip: *const libc::c_char,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.source_ip = CStr::from_ptr(ip).to_str().unwrap().parse().unwrap();
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_response_status_c(pipeline_state_ptr: *mut PipelineState) -> u16 { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    pipeline_state.status_code
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_status_c(
    pipeline_state_ptr: *mut PipelineState,
    status_code: u16,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.status_code = status_code;
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_header_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    value: *const libc::c_char,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    let key = CStr::from_ptr(key).to_str().unwrap();
    let value = CStr::from_ptr(value).to_str().unwrap();
    pipeline_state
        .response_headers
        .insert(key, value.parse().unwrap());
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_body_c(
    pipeline_state_ptr: *mut PipelineState,
    body: *const u8,
    body_len: usize,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.response_body = std::slice::from_raw_parts(body, body_len).to_vec();
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_response_header_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let key = CStr::from_ptr(key).to_str().unwrap();
    let pipeline_state = &*pipeline_state_ptr;
    if let Some(value) = pipeline_state.response_headers.get(key) {
        alloc_fn(
            arena,
            CString::new(value.to_str().unwrap()).unwrap().as_ptr(),
        )
    } else {
        std::ptr::null_mut()
    }
}}



static GLOBAL_TERA: Lazy<Arc<Tera>> = Lazy::new(|| {
    Arc::new(Tera::new("content/**/*.html").expect("Failed to parse Tera templates"))
});


#[derive(Debug, Deserialize)]
struct UrlRoute {
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    hostname: Option<String>,
    url: String,
    module_id: String,
}

#[derive(Debug, Deserialize)]
struct ServerConfig {
    #[serde(default)]
    urls: Vec<UrlRoute>,
    #[serde(default)]
    modules: Vec<ModuleConfig>,
    log4rs_config: String,
    servers: Vec<ServerDetails>,
}

#[derive(Debug, Deserialize, Clone)]
struct HostDetails {
    name: String,
    tls_cert_path: Option<String>,
    tls_key_path: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct ServerDetails {
    protocol: String,
    port: u16,
    bind_address: String,
    hosts: Vec<HostDetails>,
}

#[derive(Debug)]
struct CustomCertResolver {
    sni_resolver: ResolvesServerCertUsingSni,
    default_cert: Option<Arc<CertifiedKey>>,
}

impl ResolvesServerCert for CustomCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        // First, try to resolve using the SNI-based resolver
        if let Some(cert) = self.sni_resolver.resolve(client_hello) {
            return Some(cert);
        }
        // If no specific certificate was found, return the default one
        self.default_cert.clone()
    }
}

struct LoadedModule {
    _library: Arc<Library>,
    module_name: String,
    module_interface: Box<ModuleInterface>,
    module_config: ModuleConfig,
}

#[derive(Clone)]
struct PipelineExecutor {
    phases: HashMap<Phase, Vec<Arc<LoadedModule>>>
}

impl PipelineExecutor {
    fn new(phases: HashMap<Phase, Vec<Arc<LoadedModule>>>) -> Self {
        PipelineExecutor { phases }
    }

    async fn execute_pipeline(self: Arc<Self>, source_ip: SocketAddr, req: Request<Body>, protocol: String) -> Response<Body> {
        const PHASES: &[Phase] = &[
            Phase::PreEarlyRequest, Phase::EarlyRequest, Phase::PostEarlyRequest, Phase::PreAuthentication,
            Phase::Authentication, Phase::PostAuthentication, Phase::PreAuthorization, Phase::Authorization,
            Phase::PostAuthorization, Phase::PreContent, Phase::Content, Phase::PostContent, Phase::PreAccounting,
            Phase::Accounting, Phase::PostAccounting, Phase::PreErrorHandling, Phase::ErrorHandling,
            Phase::PostErrorHandling, Phase::PreLateRequest, Phase::LateRequest, Phase::PostLateRequest,
        ];

        let (parts, body) = req.into_parts();
        let body_bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap().to_vec();

        let mut state = PipelineState {
            arena: Bump::new(),
            protocol,
            request_method: parts.method.to_string(),
            request_path: parts.uri.path().to_string(),
            request_query: parts.uri.query().unwrap_or("").to_string(),
            request_headers: parts.headers.clone(),
            request_body: body_bytes,
            source_ip,
            status_code: 200,
            response_headers: HeaderMap::new(),
            response_body: Vec::new(),
            module_context: Arc::new(RwLock::new(HashMap::new()))
        };

        state.module_context.write().unwrap().insert("module_name".to_string(), Value::String("NONE".to_string()));
        state.module_context.write().unwrap().insert("module_context".to_string(), Value::String("No context".to_string()));

        let mut content_was_handled = false;
        let mut current_phase_index = 0;
        while current_phase_index < PHASES.len() {
            let current_phase = &PHASES[current_phase_index];
            debug!("Executing phase: {:?}, Body: {:?}", current_phase, String::from_utf8_lossy(&state.response_body));
            if let Some(modules) = self.phases.get(current_phase) {
                info!("Executing phase: {:?}", current_phase);
                let mut jumped_to_next_phase = false;

                for module in modules {
                    if let Some(uris) = &module.module_config.uris {
                        let full_uri = if state.request_query.is_empty() {
                            state.request_path.clone()
                        } else {
                            format!("{}?{}", state.request_path, state.request_query)
                        };
                        
                        let hostname = state.request_headers.get(axum::http::header::HOST)
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.split(':').next().unwrap_or("").to_string());

                        let mut matched = false;
                        for uri_matcher in uris {
                            let protocol_pattern = uri_matcher.protocol.as_deref().unwrap_or(".*");
                            let hostname_pattern = uri_matcher.hostname.as_deref().unwrap_or(".*");

                            let protocol_regex = match Regex::new(protocol_pattern) {
                                Ok(re) => re,
                                Err(e) => {
                                    error!("Invalid regex for protocol '{}': {}", protocol_pattern, e);
                                    continue;
                                }
                            };

                            let hostname_regex = match Regex::new(hostname_pattern) {
                                Ok(re) => re,
                                Err(e) => {
                                    error!("Invalid regex for hostname '{}': {}", hostname_pattern, e);
                                    continue;
                                }
                            };

                            if !protocol_regex.is_match(&state.protocol) {
                                continue;
                            }

                            if !hostname_regex.is_match(hostname.as_deref().unwrap_or("")) {
                                continue;
                            }
                            
                            match Regex::new(&uri_matcher.path) {
                                Ok(regex) => {
                                    if regex.is_match(&full_uri) {
                                        matched = true;
                                        break;
                                    }
                                }
                                Err(e) => {
                                    error!("Invalid regex pattern '{}' for module '{}': {}", uri_matcher.path, module.module_name, e);
                                }
                            }
                        }

                        if !matched {
                            info!("Request URI '{}' did not match any URI patterns for module '{}'. Skipping module.", full_uri, module.module_name);
                            continue;
                        }
                    }

                    let module_interface = &module.module_interface;
                    let handler_result = unsafe {
                        (module_interface.handler_fn)(
                            module_interface.instance_ptr,
                            &mut state as *mut _,
                            module_interface.log_callback,
                            alloc_raw_c,
                            &state.arena as *const Bump as *const c_void,
                        )
                    };

                    match handler_result {
                        HandlerResult::ModifiedContinue | HandlerResult::ModifiedNextPhase | HandlerResult::ModifiedJumpToError => {
                            let mut module_context_write_guard = state.module_context.write().unwrap();
                            module_context_write_guard.insert("module_name".to_string(), Value::String(module.module_name.clone()));
                            module_context_write_guard.insert("module_context".to_string(), Value::String("{\"status\":\"modified\"}".to_string()));
                        },
                        _ => {}
                    }

                    match handler_result {
                        HandlerResult::UnmodifiedContinue => {} // Do nothing, continue to next module or phase
                        HandlerResult::ModifiedContinue => {
                            if *current_phase == Phase::Content {
                                content_was_handled = true;
                            }
                        }
                        HandlerResult::UnmodifiedNextPhase => {
                            jumped_to_next_phase = true;
                            break; // Break from module loop, move to next phase
                        }
                        HandlerResult::ModifiedNextPhase => {
                            if *current_phase == Phase::Content {
                                content_was_handled = true;
                            }
                            jumped_to_next_phase = true;
                            break; // Break from module loop, move to next phase
                        }
                        HandlerResult::UnmodifiedJumpToError | HandlerResult::ModifiedJumpToError => {
                            if *current_phase == Phase::Content {
                                content_was_handled = true;
                            }
                            current_phase_index = PHASES.iter().position(|&p| p == Phase::PreErrorHandling).unwrap_or(PHASES.len());
                            jumped_to_next_phase = true;
                            break; // Break from module loop, jump to error handling phase
                        }
                        HandlerResult::HaltProcessing => {
                            state.status_code = 500;
                            return Response::builder()
                                .status(StatusCode::from_u16(state.status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
                                .body(Body::from("Pipeline stopped by module due to fatal error."))
                                .unwrap();
                        }
                    }
                }

                if jumped_to_next_phase {
                    continue; // Continue to the next phase
                }
            }

            if *current_phase == Phase::Content && !content_was_handled {
                info!("No content module handled the request. Setting status to 500.");
                state.status_code = 500;
                let mut module_context_write_guard = state.module_context.write().unwrap();
                module_context_write_guard.insert("module_name".to_string(), Value::String("NONE".to_string()));
                module_context_write_guard.insert("module_context".to_string(), Value::String("No context module matched".to_string()));
                current_phase_index = PHASES.iter().position(|&p| p == Phase::PreErrorHandling).unwrap_or(PHASES.len());
                continue;
            }

            current_phase_index += 1;
        }

        debug!("Final Response State: Status: {}, Body: {:?}, Headers: {:?}", state.status_code, String::from_utf8_lossy(&state.response_body), state.response_headers);
        let mut response = Response::builder()
            .status(StatusCode::from_u16(state.status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
        
        for (key, value) in state.response_headers.iter() {
            response = response.header(key, value);
        }

        response.body(Body::from(state.response_body)).unwrap()
    }
}


unsafe impl Send for LoadedModule {}
unsafe impl Sync for LoadedModule {}


#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "ox_webservice.yaml")]
    config: String,
    #[arg(short, long, value_delimiter = ',')]
    modules: Option<Vec<String>>,
    #[arg(short = 'p', long, value_parser = parse_key_val, action = clap::ArgAction::Append)]
    module_params: Vec<(String, Value)>,
}

fn parse_key_val(s: &str) -> Result<(String, Value), String> {
    let pos = s.find(':').ok_or_else(|| format!("invalid KEY:VALUE format: no `:` found in `{}`", s))?;
    let key = s[..pos].to_string();
    let rest = &s[pos + 1..];
    let pos = rest.find('=').ok_or_else(|| format!("invalid PARAM=VALUE format: no `=` found in `{}`", rest))?;
    let param_key = rest[..pos].to_string();
    let value = rest[pos + 1..].to_string();
    Ok((key, serde_json::json!({ param_key: value })))
}


#[tokio::main]
async fn main() {
    let _ = CryptoProvider::install_default(aws_lc_rs::default_provider());
    let cli = Cli::parse();

    let server_config_path = Path::new(&cli.config);
    let mut server_config: ServerConfig = match load_config_from_path(server_config_path, "info") {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    match log4rs::init_file(&server_config.log4rs_config, Default::default()) {
        Ok(_) => info!("log4rs initialized successfully, taking over from temporary logger."),
        Err(e) => {
            eprintln!("Failed to initialize log4rs from {}: {}. Exiting.", server_config.log4rs_config, e);
            std::process::exit(1);
        }
    }

    info!("Starting ox_webservice...");
    debug!("CLI arguments: {:?}", cli);

    if let Some(cli_modules) = cli.modules {
        info!("Overriding modules from CLI: {:?}", cli_modules);
        server_config.modules = cli_modules
            .into_iter()
            .map(|name| {
                ModuleConfig {
                    id: None,
                    name,
                    params: None,
                    error_path: None,
                    phase: ox_webservice_api::Phase::Content,
                    priority: 0,
                    uris: None,
                }
            })
            .collect();
    }

    for (module_name, cli_params) in &cli.module_params {
        if let Some(module_config) = server_config.modules.iter_mut().find(|m| &m.name == module_name) {
            if let Some(existing_params) = &mut module_config.params {
                if let Value::Object(map) = existing_params {
                    if let Value::Object(cli_map) = cli_params {
                        for (k, v) in cli_map {
                            map.insert(k.clone(), v.clone());
                        }
                    }
                }
            } else {
                module_config.params = Some(cli_params.clone());
            }
        }
    }

    // Ensure GLOBAL_TERA is initialized
    Lazy::force(&GLOBAL_TERA);

    // --- Merge top-level URLs into module configs ---
    let mut modules_by_id: HashMap<String, &mut ModuleConfig> = HashMap::new();
    for m_ref in server_config.modules.iter_mut() {
        if let Some(id) = &m_ref.id {
            modules_by_id.insert(id.clone(), m_ref);
        }
    }

    for route in &server_config.urls {
        if let Some(module) = modules_by_id.get_mut(&route.module_id) {
            let uris = module.uris.get_or_insert_with(Vec::new);
            uris.push(UriMatcher {
                protocol: route.protocol.clone(),
                hostname: route.hostname.clone(),
                path: route.url.clone(),
            });
            info!("Routing top-level URL '{}' (protocol: {:?}, hostname: {:?}) to module with id '{}'", route.url, route.protocol, route.hostname, route.module_id);
        } else {
            warn!("URL '{}' specifies a module_id '{}' that was not found.", route.url, route.module_id);
        }
    }

    info!("Major process state: Initializing modules.");
    let mut loaded_modules: Vec<LoadedModule> = Vec::new();

// --- Create the WebServiceApiV1 instance ---
    let api = WebServiceApiV1 {
        log_callback: host_log_callback,
        alloc_str: alloc_str_c,
        alloc_raw: alloc_raw_c,
        get_module_context_value: get_module_context_value_c,
        set_module_context_value: set_module_context_value_c,
        get_request_method: get_request_method_c,
        get_request_path: get_request_path_c,
        get_request_query: get_request_query_c,
        get_request_header: get_request_header_c,
        get_request_headers: get_request_headers_c,
        get_request_body: get_request_body_c,
        get_source_ip: get_source_ip_c,
        set_request_path: set_request_path_c,
        set_request_header: set_request_header_c,
        set_source_ip: set_source_ip_c,
        get_response_status: get_response_status_c,
        set_response_status: set_response_status_c,
        set_response_header: set_response_header_c,
        set_response_body: set_response_body_c,
        get_response_header: get_response_header_c,
    };

    for module_config in server_config.modules.iter_mut() {
        let module_name = &module_config.name;
        info!("Attempting to load module: {}", module_name);
        let library_file_name = if cfg!(target_os = "windows") {
            format!("{}.dll", module_name)
        } else if cfg!(target_os = "macos") {
            format!("lib{}.dylib", module_name)
        } else {
            format!("lib{}.so", module_name)
        };

        let library_path = PathBuf::from(library_file_name);

        debug!("Attempting to load module: {} from {:?}", module_name, library_path);

        match unsafe { Library::new(&library_path) } {
            Ok(library) => {
                info!("Successfully loaded module: {}", module_name);
                unsafe {
                    let initialize_module_fn: Symbol<InitializeModuleFn> = library
                        .get(b"initialize_module")
                        .expect(&format!("Failed to find 'initialize_module' in {}", module_name));

                    let module_params_json = serde_json::to_string(&module_config.params.clone().unwrap_or_default())
                        .expect("Failed to serialize module params to JSON");
                    let module_params_cstring = CString::new(module_params_json)
                        .expect("Failed to create CString from module params JSON");
                    let module_params_json_ptr = module_params_cstring.as_ptr();

                    let module_interface_ptr = initialize_module_fn(
                        module_params_json_ptr,
                        &api as *const WebServiceApiV1,
                    );

                    if module_interface_ptr.is_null() {
                        error!("initialize_module for '{}' returned a null pointer. Module not loaded.", module_name);
                        continue;
                    }
                    let module_interface = Box::from_raw(module_interface_ptr);

                    let current_library = Arc::new(library);

                    loaded_modules.push(LoadedModule {
                        _library: current_library.clone(),
                        module_name: module_name.clone(),
                        module_interface,
                        module_config: module_config.clone(),
                    });
                }
            }
            Err(e) => {
                error!("Failed to load module {}: {}", module_name, e);
            }
        }
    }

    let mut phases: HashMap<Phase, Vec<Arc<LoadedModule>>> = HashMap::new();
    for module in loaded_modules {
        phases.entry(module.module_config.phase).or_default().push(Arc::new(module));
    }

    for phase_modules in phases.values_mut() {
        phase_modules.sort_by_key(|m| m.module_config.priority);
    }

    let pipeline_executor = Arc::new(
        PipelineExecutor::new(phases)
    );

    let mut tasks = vec![];

    for server in server_config.servers {
        let app = Router::new()
            .route("/", get(|| async {"Hello from ox_webservice - Pipeline active."}))
            .layer(axum::middleware::from_fn({ 
                let executor_clone = Arc::clone(&pipeline_executor);
                let server_protocol = server.protocol.clone();
                move |ConnectInfo(source_ip): ConnectInfo<SocketAddr>, req, _next| {
                    let executor_clone = executor_clone.clone();
                    let protocol_clone = server_protocol.clone();
                    async move {
                        executor_clone.execute_pipeline(source_ip, req, protocol_clone).await
                    }
                }
            }));

        let bind_address: IpAddr = server.bind_address.parse().unwrap_or_else(|_| {
            error!("Invalid bind address '{}', defaulting to 127.0.0.1", server.bind_address);
            "127.0.0.1".parse().unwrap()
        });
        
        let addr = SocketAddr::new(bind_address, server.port);

        let listener_future = async move {
            if server.protocol == "https" {
                let mut sni_resolver = ResolvesServerCertUsingSni::new();
                let mut default_certificate: Option<Arc<CertifiedKey>> = None;
                let mut host_names = Vec::new();

                for host in server.hosts.iter() {
                    if let (Some(cert_path), Some(key_path)) = (host.tls_cert_path.as_ref(), host.tls_key_path.as_ref()) {
                        let cert_file = match File::open(&cert_path) {
                            Ok(file) => file,
                            Err(e) => {
                                error!("Failed to open certificate file '{}' for host {}: {}. Skipping host.", cert_path, host.name, e);
                                continue;
                            }
                        };
                        let key_file = match File::open(&key_path) {
                            Ok(file) => file,
                            Err(e) => {
                                error!("Failed to open key file '{}' for host {}: {}. Skipping host.", key_path, host.name, e);
                                continue;
                            }
                        };
                        let cert_file_content = &mut BufReader::new(cert_file);
                        let key_file_content = &mut BufReader::new(key_file);

                        let cert_chain: Vec<CertificateDer> = certs(cert_file_content)
                            .filter_map(|cert_res| cert_res.ok())
                            .collect();
                        
                        let mut keys: Vec<PrivateKeyDer> = pkcs8_private_keys(key_file_content)
                            .filter_map(|key_res| key_res.ok())
                            .map(Into::into)
                            .collect();
                        
                        if keys.is_empty() {
                            error!("No private key found in key file for host {}", host.name);
                            continue;
                        }
                        
                        let private_key = keys.remove(0);
                        let signing_key = match rustls::crypto::aws_lc_rs::sign::any_supported_type(&private_key) {
                            Ok(key) => key,
                            Err(e) => {
                                error!("Invalid private key for host {}: {}. Skipping host.", host.name, e);
                                continue;
                            }
                        };
                        let certified_key = CertifiedKey::new(cert_chain, signing_key);

                        if host.name == ".*" {
                            if default_certificate.is_some() {
                                warn!("Multiple default HTTPS configurations found for server on port {}. Using the last one.", server.port);
                            }
                            default_certificate = Some(Arc::new(certified_key));
                            info!("Found default HTTPS configuration for server on port {}", server.port);
                        } else {
                            if let Err(e) = sni_resolver.add(&host.name, certified_key) {
                                error!("Failed to add certificate for host '{}': {}. This is likely due to an invalid DNS name. Skipping host.", host.name, e);
                                continue;
                            }
                            host_names.push(host.name.as_str());
                        }
                    }
                }

                if !host_names.is_empty() || default_certificate.is_some() {
                    let resolver = CustomCertResolver {
                        sni_resolver,
                        default_cert: default_certificate,
                    };

                    // Capture this value before resolver is moved
                    let has_default_cert = resolver.default_cert.is_some();

                    let tls_config = RustlsServerConfig::builder()
                        .with_no_client_auth()
                        .with_cert_resolver(Arc::new(resolver));
                    
                    let rustls_config = RustlsConfig::from_config(Arc::new(tls_config));
                    
                    let mut listening_on = host_names.join(", ");
                    if has_default_cert {
                        if listening_on.is_empty() {
                            listening_on = "default (.*)".to_string();
                        } else {
                            listening_on.push_str(", and default (.*)");
                        }
                    }

                    info!("{} server listening on https://{}:{}", listening_on, addr.ip(), addr.port());
                    bind_rustls(addr, rustls_config)
                        .serve(app.into_make_service_with_connect_info::<SocketAddr>()) 
                        .await
                        .unwrap();
                } else {
                    error!("HTTPS protocol specified for server on port {} but no hosts with valid certificates are configured.", server.port);
                }
            } else {
                let listener = TcpListener::bind(&addr).await.unwrap();
                let host_names: Vec<&str> = server.hosts.iter().map(|h| h.name.as_str()).collect();
                info!("{} server listening on http://{}:{}", host_names.join(", "), addr.ip(), addr.port());
                axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
            }
        };
        tasks.push(tokio::spawn(listener_future));
    }

    // Wait for all server tasks to complete
    for task in tasks {
        task.await.unwrap();
    }
}
fn load_config_from_path(path: &Path, cli_log_level: &str) -> Result<ServerConfig, ConfigError> {
    debug!("Loading config from: {:?}", path);
    trace!("File extension: {:?}", path.extension());

    if !path.exists() {
        error!("Configuration file not found at {:?}", path);
        return Err(ConfigError::NotFound);
    }

    let mut file = File::open(path)
        .map_err(ConfigError::ReadError)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(ConfigError::ReadError)?;

    debug!("Content read from config file: \n{}", contents);

    if cli_log_level == "trace" {
        trace!("Parsed config file content:\n{}", contents);
    } else if cli_log_level == "debug" {
        debug!("Parsed config file content:\n{}", contents);
    }

    match path.extension().and_then(|s| s.to_str()) {
        Some("yaml") | Some("yml") => {
            debug!("Parsing as YAML");
            serde_yaml::from_str(&contents).map_err(|e| ConfigError::ParseError(e.to_string()))
        }
        Some("json") => {
            debug!("Parsing as JSON");
            serde_json::from_str(&contents).map_err(|e| ConfigError::ParseError(e.to_string()))
        }
        Some("toml") => {
            debug!("Parsing as TOML");
            toml::from_str(&contents).map_err(|e| ConfigError::ParseError(e.to_string()))
        }
        Some("xml") => {
            debug!("Parsing as XML");
            serde_xml_rs::from_str(&contents).map_err(|e| ConfigError::ParseError(e.to_string()))
        }
        _ => {
            error!("Unsupported server config file format: {:?}. Exiting.", path.extension());
            Err(ConfigError::UnsupportedFormat)
        }
    }
}

// #[unsafe(no_mangle)] - Not needed as this is passed via pointer, not dlsym'd
pub unsafe extern "C" fn host_log_callback(level: LogLevel, module: *const libc::c_char, message: *const libc::c_char) {
    let module_str = unsafe { CStr::from_ptr(module).to_string_lossy() };
    let message_str = unsafe { CStr::from_ptr(message).to_string_lossy() };
    
    match level {
        LogLevel::Error => error!(target: &module_str, "{}", message_str),
        LogLevel::Warn => warn!(target: &module_str, "{}", message_str),
        LogLevel::Info => info!(target: &module_str, "{}", message_str),
        LogLevel::Debug => debug!(target: &module_str, "{}", message_str),
        LogLevel::Trace => trace!(target: &module_str, "{}", message_str),
    }
}