
use std::error::Error;
use std::fmt;


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
    Json, Router,
    response::{IntoResponse, Html},
    extract::ConnectInfo,
};
use tower::ServiceExt;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::net::TcpListener;
use sysinfo::{System};
use tera::Tera;
use log::{info, debug, trace, error, warn};
use futures::future::BoxFuture;

use libloading::{Library, Symbol};
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::future::Future;
use once_cell::sync::Lazy;


use regex::Regex;
use std::panic;

use ox_webservice_api::{
    ModuleConfig, WebServiceContext, LogCallback, LogLevel, InitializeModuleFn, Phase, HandlerResult,
    RequestContext, ModuleInterface, GetRequestMethodFn, GetRequestPathFn, GetRequestQueryFn,
    GetRequestHeaderFn, GetRequestHeadersFn, GetRequestBodyFn, GetSourceIpFn, SetRequestPathFn,
    SetRequestHeaderFn, GetResponseStatusFn, GetResponseHeaderFn, SetResponseStatusFn,
    SetResponseHeaderFn, SetResponseBodyFn,
};

static GLOBAL_TERA: Lazy<Arc<Tera>> = Lazy::new(|| {
    Arc::new(Tera::new("content/**/*.html").expect("Failed to parse Tera templates"))
});

// --- Pipeline State ---
struct PipelineState {
    // Request data
    request_method: String,
    request_path: String,
    request_query: String,
    request_headers: HeaderMap,
    request_body: Vec<u8>,
    source_ip: SocketAddr,

    // Response data
    status_code: u16,
    response_headers: HeaderMap,
    response_body: Vec<u8>,
}

// --- FFI Helper Functions ---
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_method_c(context_ptr: *mut RequestContext) -> *mut c_char {
    let state = unsafe { &*(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    CString::new(state.request_method.as_str()).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_path_c(context_ptr: *mut RequestContext) -> *mut c_char {
    let state = unsafe { &*(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    CString::new(state.request_path.as_str()).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_query_c(context_ptr: *mut RequestContext) -> *mut c_char {
    let state = unsafe { &*(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    CString::new(state.request_query.as_str()).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_header_c(context_ptr: *mut RequestContext, key_ptr: *const c_char) -> *mut c_char {
    let state = unsafe { &*(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    let key = unsafe { CStr::from_ptr(key_ptr).to_str().unwrap() };
    match state.request_headers.get(key) {
        Some(value) => CString::new(value.to_str().unwrap_or("")).unwrap().into_raw(),
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_headers_c(context_ptr: *mut RequestContext) -> *mut c_char {
    let state = unsafe { &*(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    let headers: HashMap<String, String> = state.request_headers.iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let json = serde_json::to_string(&headers).unwrap();
    CString::new(json).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_body_c(context_ptr: *mut RequestContext) -> *mut c_char {
    let state = unsafe { &*(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    CString::new(state.request_body.as_slice()).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_source_ip_c(context_ptr: *mut RequestContext) -> *mut c_char {
    let state = unsafe { &*(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    CString::new(state.source_ip.to_string()).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_request_path_c(context_ptr: *mut RequestContext, path_ptr: *const c_char) {
    let state = unsafe { &mut *(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    let path = unsafe { CStr::from_ptr(path_ptr).to_str().unwrap() };
    state.request_path = path.to_string();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_request_header_c(context_ptr: *mut RequestContext, key_ptr: *const c_char, value_ptr: *const c_char) {
    let state = unsafe { &mut *(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    let key = unsafe { CStr::from_ptr(key_ptr).to_str().unwrap() };
    let value = unsafe { CStr::from_ptr(value_ptr).to_str().unwrap() };
    state.request_headers.insert(axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap(), axum::http::HeaderValue::from_str(value).unwrap());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_source_ip_c(context_ptr: *mut RequestContext, ip_ptr: *const c_char) {
    let state = unsafe { &mut *(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    let ip_str = unsafe { CStr::from_ptr(ip_ptr).to_str().unwrap() };
    match ip_str.parse::<SocketAddr>() {
        Ok(addr) => {
            state.source_ip = addr;
        }
        Err(e) => {
            error!("Failed to parse IP address '{}': {}", ip_str, e);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_response_status_c(context_ptr: *mut RequestContext) -> u16 {
    let state = unsafe { &*(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    state.status_code
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_response_header_c(context_ptr: *mut RequestContext, key_ptr: *const c_char) -> *mut c_char {
    let state = unsafe { &*(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    let key = unsafe { CStr::from_ptr(key_ptr).to_str().unwrap() };
    match state.response_headers.get(key) {
        Some(value) => CString::new(value.to_str().unwrap_or("")).unwrap().into_raw(),
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_status_c(context_ptr: *mut RequestContext, status_code: u16) {
    let state = unsafe { &mut *(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    state.status_code = status_code;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_header_c(context_ptr: *mut RequestContext, key_ptr: *const c_char, value_ptr: *const c_char) {
    let state = unsafe { &mut *(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    let key = unsafe { CStr::from_ptr(key_ptr).to_str().unwrap() };
    let value = unsafe { CStr::from_ptr(value_ptr).to_str().unwrap() };
    state.response_headers.insert(axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap(), axum::http::HeaderValue::from_str(value).unwrap());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_body_c(context_ptr: *mut RequestContext, body_ptr: *const u8, body_len: usize) {
    let state = unsafe { &mut *(*context_ptr).pipeline_state_ptr.cast::<PipelineState>() };
    let body_slice = unsafe { std::slice::from_raw_parts(body_ptr, body_len) };
    state.response_body = body_slice.to_vec();
}


#[derive(Debug, Deserialize)]
struct TemplateQuery {
    name: String,
}


#[derive(Debug, Deserialize)]
struct ServerConfig {
    #[serde(default)] // Allow modules to be optional in the config file
    modules: Vec<ModuleConfig>,
    log4rs_config: String,
    server: ServerDetails, // New field
}

#[derive(Debug, Deserialize)]
struct ServerDetails {
    port: u16,
    bind_address: String,
}

struct LoadedModule {
    _library: Arc<Library>, // Keep the library loaded as an Arc
    module_name: String,
    module_interface: Box<ModuleInterface>, // Store the ModuleInterface
    module_config: ModuleConfig,
}

#[derive(Clone)]
struct PipelineExecutor {
    // This will eventually hold the modules grouped by phase
    phases: HashMap<Phase, Vec<Arc<LoadedModule>>>,
    // Add other fields needed for pipeline execution here
}

impl PipelineExecutor {
    fn new(
        phases: HashMap<Phase, Vec<Arc<LoadedModule>>>,
    ) -> Self {
        PipelineExecutor {
            phases,
        }
    }

    async fn execute_pipeline(self: Arc<Self>, source_ip: SocketAddr, req: Request<Body>) -> Response<Body> {
        const PHASES: &[Phase] = &[
            Phase::PreEarlyRequest,
            Phase::EarlyRequest,
            Phase::PostEarlyRequest,
            Phase::PreAuthentication,
            Phase::Authentication,
            Phase::PostAuthentication,
            Phase::PreAuthorization,
            Phase::Authorization,
            Phase::PostAuthorization,
            Phase::PreContent,
            Phase::Content,
            Phase::PostContent,
            Phase::PreAccounting,
            Phase::Accounting,
            Phase::PostAccounting,
            Phase::PreErrorHandling,
            Phase::ErrorHandling,
            Phase::PostErrorHandling,
            Phase::PreLateRequest,
            Phase::LateRequest,
            Phase::PostLateRequest,
        ];

        let (parts, body) = req.into_parts();
        let body_bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap().to_vec();

        let mut state = PipelineState {
            request_method: parts.method.to_string(),
            request_path: parts.uri.path().to_string(),
            request_query: parts.uri.query().unwrap_or("").to_string(),
            request_headers: parts.headers.clone(),
            request_body: body_bytes,
            source_ip,
            status_code: 200,
            response_headers: HeaderMap::new(),
            response_body: Vec::new(),
        };

        let mut request_context = RequestContext {
            pipeline_state_ptr: &mut state as *mut _ as *mut c_void,
        };

        let mut content_was_handled = false; // Internal flag for the server
        let mut current_phase_index = 0;
        while current_phase_index < PHASES.len() {
            let current_phase = &PHASES[current_phase_index];
            if let Some(modules) = self.phases.get(current_phase) {
                info!("Executing phase: {:?}", current_phase);
                let mut jumped_to_next_phase = false;

                for module in modules {
                    // URI matching logic
                    if let Some(uris) = &module.module_config.uris {
                        let state = unsafe { &*request_context.pipeline_state_ptr.cast::<PipelineState>() };
                        let full_uri = if state.request_query.is_empty() {
                            state.request_path.clone()
                        } else {
                            format!("{}?{}", state.request_path, state.request_query)
                        };
                        let mut matched = false;
                        for uri_pattern in uris {
                            match Regex::new(uri_pattern) {
                                Ok(regex) => {
                                    if regex.is_match(&full_uri) {
                                        matched = true;
                                        break;
                                    }
                                }
                                Err(e) => {
                                    error!("Invalid regex pattern '{}' for module '{}': {}", uri_pattern, module.module_name, e);
                                }
                            }
                        }
                        if !matched {
                            info!("Request URI '{}' did not match any URI patterns for module '{}'. Skipping module.", full_uri, module.module_name);
                            continue; // Skip this module
                        }
                    }

                    let module_interface = &module.module_interface;
                    let handler_result = unsafe {
                        (module_interface.handler_fn)(module_interface.instance_ptr, &mut request_context as *mut _)
                    };

                    match handler_result {
                        HandlerResult::UnmodifiedContinue => {}
                        HandlerResult::ModifiedContinue => {
                            if *current_phase == Phase::Content {
                                content_was_handled = true;
                            }
                        }
                        HandlerResult::UnmodifiedNextPhase => {
                            jumped_to_next_phase = true;
                            break;
                        }
                        HandlerResult::ModifiedNextPhase => {
                            if *current_phase == Phase::Content {
                                content_was_handled = true;
                            }
                            jumped_to_next_phase = true;
                            break;
                        }
                        HandlerResult::UnmodifiedJumpToError | HandlerResult::ModifiedJumpToError => {
                            if *current_phase == Phase::Content {
                                content_was_handled = true;
                            }
                            current_phase_index = PHASES.iter().position(|&p| p == Phase::PreErrorHandling).unwrap_or(PHASES.len());
                            jumped_to_next_phase = true;
                            break;
                        }
                        HandlerResult::HaltProcessing => { // Changed from Stop
                            state.status_code = 500;
                            return Response::builder()
                                .status(StatusCode::from_u16(state.status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
                                .body(Body::from("Pipeline stopped by module due to fatal error."))
                                .unwrap();
                        }
                    }
                }

                if jumped_to_next_phase {
                    continue;
                }
            }

            if *current_phase == Phase::Content && !content_was_handled {
                info!("No content module handled the request. Setting status to 404.");
                state.status_code = 404;
                current_phase_index = PHASES.iter().position(|&p| p == Phase::PreErrorHandling).unwrap_or(PHASES.len());
                continue;
            }

            current_phase_index += 1;
        }

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
    /// Path to the server configuration file
    #[arg(short, long, default_value = "ox_webservice.yaml")]
    config: String,

    /// Comma-separated list of modules to load, overriding config file
    #[arg(short, long, value_delimiter = ',')]
    modules: Option<Vec<String>>,

    /// Pass parameters to modules, e.g., -p ox_content:error_path=www/error_cli
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

// Implement the C-style logging callback function
#[unsafe(no_mangle)]
pub unsafe extern "C" fn log_callback(level: LogLevel, message: *const c_char) {
    let message = unsafe { CStr::from_ptr(message).to_string_lossy() };
    match level {
        LogLevel::Error => error!("{}", message),
        LogLevel::Warn => warn!("{}", message),
        LogLevel::Info => info!("{}", message),
        LogLevel::Debug => debug!("{}", message),
        LogLevel::Trace => trace!("{}", message),
    }
}


#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // --- Load Server Configuration ---
    let server_config_path = Path::new(&cli.config);
    let mut server_config: ServerConfig = match load_config_from_path(server_config_path, "info") {
        Ok(config) => config,
        Err(e) => {
            // Can't use logger here, as it's not initialized yet.
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    // Initialize log4rs, which will take over from env_logger
    match log4rs::init_file(&server_config.log4rs_config, Default::default()) {
        Ok(_) => info!("log4rs initialized successfully, taking over from temporary logger."),
        Err(e) => {
            eprintln!("Failed to initialize log4rs from {}: {}. Exiting.", server_config.log4rs_config, e);
            std::process::exit(1);
        }
    }

    info!("Starting ox_webservice...");
    debug!("CLI arguments: {:?}", cli);


    // --- Override/Supplement Modules from CLI ---
    if let Some(cli_modules) = cli.modules {
        info!("Overriding modules from CLI: {:?}", cli_modules);
        server_config.modules = cli_modules
            .into_iter()
            .map(|name| {
                ModuleConfig {
                    name,
                    params: None,
                    error_path: None,
                    phase: ox_webservice_api::Phase::Content, // Default phase
                    priority: 0, // Default priority
                    uris: None, // Default uris
                }
            })
            .collect();
    }

    // --- Merge CLI parameters into module configs ---
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

    info!("Major process state: Initializing modules.");
    // --- Dynamically Load Modules and Register Routes ---
    let mut loaded_modules: Vec<LoadedModule> = Vec::new();

    for module_config in server_config.modules.iter_mut() {
        // Create a new WebServiceContext for each module
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

                    // Prepare the initialization data
                    let module_params_json = serde_json::to_string(&module_config.params.clone().unwrap_or_default())
                        .expect("Failed to serialize module params to JSON");
                    let module_params_cstring = CString::new(module_params_json)
                        .expect("Failed to create CString from module params JSON");
                    let module_params_json_ptr = module_params_cstring.as_ptr();

                    let module_interface_ptr = initialize_module_fn(
                        module_params_json_ptr,
                        render_template_ffi,
                        log_callback,
                        get_request_method_c,
                        get_request_path_c,
                        get_request_query_c,
                        get_request_header_c,
                        get_request_headers_c,
                        get_request_body_c,
                        get_source_ip_c,
                        set_request_path_c,
                        set_request_header_c,
                        set_source_ip_c,
                        get_response_status_c,
                        get_response_header_c,
                        set_response_status_c,
                        set_response_header_c,
                        set_response_body_c,
                    );

                    if module_interface_ptr.is_null() {
                        error!("initialize_module for '{}' returned a null pointer. Module not loaded.", module_name);
                        continue; // Skip this module
                    }
                    let module_interface = Box::from_raw(module_interface_ptr);

                    let current_library = Arc::new(library); // Wrap library in Arc for sharing

                    loaded_modules.push(LoadedModule {
                        _library: current_library.clone(),
                        module_name: module_name.clone(),
                        module_interface, // Store the ModuleInterface
                        module_config: module_config.clone(),
                    });
                }
            }
            Err(e) => {
                error!("Failed to load module {}: {}", module_name, e);
            }
        }
    }

    // --- Group Modules by Phase and Sort by Priority ---
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

    let app = Router::new()
        .route("/", get(|| async {"Hello from ox_webservice - Pipeline active."}))
        .layer(axum::middleware::from_fn(move |ConnectInfo(source_ip): ConnectInfo<SocketAddr>, req, _next| {
            let executor_clone = pipeline_executor.clone();
            Box::pin(async move {
                executor_clone.execute_pipeline(source_ip, req).await
            })
        }));

    // Start the server
    let addr = SocketAddr::from(([127, 0, 0, 1], server_config.server.port));
    let listener = TcpListener::bind(&addr).await.unwrap();

    info!("listening on {}", addr);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
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

// Custom debug macro
#[macro_export]
macro_rules! debug_println {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            println!($($arg)*);
        }
    };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn render_template_ffi(template_name_ptr: *mut c_char, data_ptr: *mut c_char) -> *mut c_char {
    let template_name = unsafe { CStr::from_ptr(template_name_ptr).to_str().unwrap() };
    let data_json = unsafe { CStr::from_ptr(data_ptr).to_str().unwrap() };

    debug!("render_template_ffi called for template: {}", template_name);
    trace!("render_template_ffi data: {}", data_json);

    let context = match serde_json::from_str(data_json) {
        Ok(ctx) => tera::Context::from_value(ctx).unwrap(),
        Err(e) => {
            error!("Failed to parse JSON data for template rendering: {}", e);
            let error_html = format!("<h1>Internal Server Error</h1><p>Failed to parse template data.</p><p>Error: {}</p>", e);
            return CString::new(error_html).unwrap().into_raw();
        }
    };

    let tera_instance = &GLOBAL_TERA;

    match tera_instance.render(template_name, &context) {
        Ok(rendered_html) => {
            CString::new(rendered_html).unwrap().into_raw()
        },
        Err(e) => {
            error!("Failed to render template '{}': {}", template_name, e);
            let error_html = format!(
                "<h1>Error Rendering Page</h1><p>Template '{}' not found or could not be rendered.</p><p>Error: {}</p>",
                template_name, e
            );
            CString::new(error_html).unwrap().into_raw()
        }
    }
}
