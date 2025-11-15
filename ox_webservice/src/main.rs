
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

use axum::{routing::get, Json, Router, response::{IntoResponse, Html}};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap; // Keep this for now, might be needed later
use std::ffi::{CStr, CString, c_char, c_void};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;
use sysinfo::{System};
use tera::Tera;
use log::{info, debug, trace, error, warn}; // Added warn
use env_logger::Env;
use futures::future::BoxFuture;

use libloading::{Library, Symbol};

use std::panic;

use ox_webservice_api::{ModuleConfig, WebServiceContext, SendableWebServiceHandler, CErrorHandler, CErrorHandlerFn, LogCallback, LogLevel, ErrorHandlerFactory, InitializeModuleFn}; // Added LogCallback, LogLevel, ErrorHandlerFactory, InitializeModuleFn
use axum::http::StatusCode;
use regex::Regex;

static mut GLOBAL_TERA: Option<Arc<Tera>> = None;



#[derive(Debug, Deserialize)]
struct TemplateQuery {
    name: String,
}


#[derive(Debug, Deserialize)]
struct ServerConfig {
    #[serde(default)] // Allow modules to be optional in the config file
    modules: Vec<ModuleConfig>,
    log4rs_config: String,
    #[serde(default)]
    error_handler: Option<ModuleConfig>,
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
    handler: SendableWebServiceHandler, // Store the handler directly
    module_config: ModuleConfig,
}

struct LoadedErrorHandler {
    _library: Arc<Library>,
    c_error_handler: CErrorHandler,
    destroy_fn: unsafe extern "C" fn(*mut CErrorHandler),
}

impl Drop for LoadedErrorHandler {
    fn drop(&mut self) {
        info!("Dropping LoadedErrorHandler for module: {:?}", self.c_error_handler.instance_ptr);
        unsafe {
            (self.destroy_fn)(&mut self.c_error_handler);
        }
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
#[no_mangle]
pub unsafe extern "C" fn log_callback(level: LogLevel, message: *const c_char) {
    let message = CStr::from_ptr(message).to_string_lossy();
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

        // --- Initialize Tera ---
        let tera_instance = match Tera::new("content/**/*.html") {
            Ok(t) => {
                info!("Tera templates initialized successfully.");
                t
            },
            Err(e) => {
                error!("Parsing error(s) in Tera templates: {}", e);
                ::std::process::exit(1);
            }
        };

    unsafe {
        GLOBAL_TERA = Some(Arc::new(tera_instance));
    }

    info!("Major process state: Initializing modules.");
    // --- Dynamically Load Modules and Register Routes ---
    let mut app = Router::new();
    let mut loaded_modules: Vec<Arc<LoadedModule>> = Vec::new();
    let mut error_handler_module: Option<Arc<LoadedErrorHandler>> = None;

    let addr = SocketAddr::from(([127, 0, 0, 1], server_config.server.port));



    if let Some(eh_module_config) = &server_config.error_handler {
        let module_name = &eh_module_config.name;
        info!("Attempting to load error handler module: {}", module_name);
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
                let current_library = Arc::new(library); // Wrap library in Arc for sharing
                unsafe {
                    let create_error_handler_fn: Symbol<ErrorHandlerFactory> = current_library
                        .get(b"create_error_handler")
                        .expect(&format!("Failed to find 'create_error_handler' in error handler module {}", module_name));

                    let destroy_error_handler_fn: Symbol<unsafe extern "C" fn(*mut CErrorHandler)> = current_library
                        .get(b"destroy_error_handler")
                        .expect(&format!("Failed to find 'destroy_error_handler' in error handler module {}", module_name));

                    let c_error_handler_ptr = create_error_handler_fn(&*eh_module_config as *const ModuleConfig as *mut c_void, log_callback);
                    if c_error_handler_ptr.is_null() {
                        error!("create_error_handler returned a null pointer. Error handler not loaded.");
                    } else {
                        let boxed_c_error_handler = Box::from_raw(c_error_handler_ptr);
                        error_handler_module = Some(Arc::new(LoadedErrorHandler {
                            _library: current_library.clone(),
                            c_error_handler: *boxed_c_error_handler,
                            destroy_fn: *destroy_error_handler_fn,
                        }));
                    }
                }
            }
            Err(e) => {
                error!("Failed to load module {}: {}", module_name, e);
            }
        }
    }

    for module_config in server_config.modules.iter_mut().filter(|m| {
        if let Some(eh_config) = &server_config.error_handler {
            m.name != eh_config.name
        } else {
            true
        }
    }) {
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
                    let module_config_ptr = &*module_config as *const ModuleConfig as *mut c_void;
                    let module_handler = initialize_module_fn(module_config_ptr, render_template_ffi, log_callback);

                    let current_library = Arc::new(library); // Wrap library in Arc for sharing

                    let loaded_module = Arc::new(LoadedModule {
                        _library: current_library.clone(),
                        module_name: module_name.clone(),
                        handler: module_handler, // Store the handler directly
                        module_config: module_config.clone(),
                    });

                    loaded_modules.push(loaded_module);
                }
            }
            Err(e) => {
                error!("Failed to load module {}: {}", module_name, e);
            }
        }
    }

    // Share the error_handler_module across all specific endpoint handlers and the fallback
    let shared_error_handler_module = error_handler_module.clone();

    // Store loaded modules with their regex patterns and priorities
    #[derive(Clone)]
    struct RouteEntry {
        module: Arc<LoadedModule>,
        regex: Regex,
        priority: u16,
    }

    let mut route_entries: Vec<RouteEntry> = Vec::new();

    for module_config in server_config.modules.iter() {
        if let Some(urls) = &module_config.params.as_ref().and_then(|p| p.get("urls")).and_then(|u| u.as_array()) {
            for url_value in urls.iter() {
                if let Some(url_str) = url_value.as_str() {
                    let regex = Regex::new(url_str).expect("Invalid regex in module URL configuration");
                    if let Some(loaded_module) = loaded_modules.iter().find(|m| m.module_name == module_config.name) { // Corrected: m.name instead of m.module_name
                        route_entries.push(RouteEntry {
                            module: loaded_module.clone(),
                            regex,
                            priority: module_config.params.as_ref().and_then(|p| p.get("priority")).and_then(|pr| pr.as_u64()).unwrap_or(0) as u16,
                        });
                    }
                }
            }
        }
    }

    // Sort route entries by priority (lowest to highest)
    route_entries.sort_by_key(|entry| entry.priority);

    let shared_route_entries = Arc::new(route_entries);
    let shared_error_handler_module_for_fallback = shared_error_handler_module.clone();

    app = app.fallback(move |req: axum::http::Request<axum::body::Body>| {
        let path = req.uri().path().to_string();
        let route_entries_clone = shared_route_entries.clone();
        let error_handler_for_fallback_clone = shared_error_handler_module_for_fallback.clone();

        async move {
            for entry in route_entries_clone.iter() {
                debug!("Checking module: {} with regex: {} against path: {}", entry.module.module_name, entry.regex.as_str(), path);
                if entry.regex.is_match(&path) {
                    debug!("Module {} regex matched for path: {}", entry.module.module_name, path);
                    let handler_fn_for_closure = entry.module.handler;
                    let request_json = serde_json::json!({ "path": path }).to_string();
                    let request_cstring = CString::new(request_json).unwrap();
                    let path_for_error_handler = path.clone(); // Clone path here for use in error handler

                    let handler_result = tokio::task::spawn_blocking(move || {
                        call_module_handler(handler_fn_for_closure, request_cstring, path.clone())
                    }).await.unwrap();

                    match handler_result {
                        Ok(json_value) => {
                            let status_code = json_value.get("status").and_then(|s| s.as_u64()).unwrap_or(200) as u16;
                            debug!("Extracted status code: {}", status_code);
                            let body = json_value.get("body").and_then(|b| b.as_str()).unwrap_or("").to_string();
                            let content_type = json_value.get("headers").and_then(|h| h.get("Content-Type")).and_then(|ct| ct.as_str()).unwrap_or("application/json").to_string();

                            let mut response = axum::response::Response::builder()
                                .status(StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK))
                                .header(axum::http::header::CONTENT_TYPE, content_type);
                            
                            if let Some(headers_map) = json_value.get("headers").and_then(|h| h.as_object()) {
                                for (key, value) in headers_map {
                                    if key != "Content-Type" {
                                        if let Some(header_value) = value.as_str() {
                                            response = response.header(key, header_value);
                                        }
                                    }
                                }
                            }

                            return response.body(axum::body::Body::from(body)).unwrap().into_response();
                        },
            Err(e) => {
                error!("Module returned an error: {}", e);
                let status_code = e.get("status").and_then(|s| s.as_u64()).unwrap_or(500) as u16;
                let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                let message = e.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
                let module_name = entry.module.module_name.clone();
                let module_context = e.get("module_context").and_then(|c| c.as_str()).unwrap_or("");

                // Start of handle_error_request logic
                if let Some(eh_wrapper_arc) = &error_handler_for_fallback_clone {
                    let eh = &eh_wrapper_arc.c_error_handler;
                    let status_code_u16 = status.as_u16();
                    let message_cstring = CString::new(message).expect("CString::new failed");
                            error!("ox_webservice: Module returning error: module_name = {}", module_name); // Temporarily added logging
                            let module_name_cstring = CString::new(module_name.clone()).expect("CString::new failed"); // This is being passed
                                    let params_json = serde_json::json!({
                                        "request_method": req.method().as_str(),
                                        "request_path": path_for_error_handler, // Use the cloned path
                                        "user_agent": req.headers().get("User-Agent").and_then(|h| h.to_str().ok()).unwrap_or("Unknown"),
                                        "timestamp": chrono::Utc::now().to_rfc3339(),
                                    }).to_string();                    let params_cstring = CString::new(params_json).expect("CString::new failed");
                    let module_context_cstring = CString::new(module_context).expect("CString::new failed");

                    error!("ox_webservice: Calling error handler handle_error_fn for status_code: {}", status_code_u16);
                    let response_ptr = unsafe {
                        (eh.handle_error_fn)(
                            eh.instance_ptr,
                            status_code_u16,
                            message_cstring.into_raw(),
                            module_name_cstring.into_raw(), // Pass module_name here
                            params_cstring.into_raw(),
                            module_context_cstring.into_raw(),
                        )
                    };

                    let html_response = if response_ptr.is_null() {
                        error!("handle_error_fn returned a null pointer. Returning generic error HTML.");
                        "<h1>Internal Server Error</h1><p>Error handler returned null.</p>".to_string()
                    } else {
                        let c_str = unsafe { CStr::from_ptr(response_ptr) };
                        let response = c_str.to_str().unwrap().to_string();
                        unsafe { CString::from_raw(response_ptr) }; // Deallocate the CString
                        response
                    };

                    return axum::response::Response::builder()
                        .status(status)
                        .header(axum::http::header::CONTENT_TYPE, "text/html")
                        .body(axum::body::Body::from(html_response))
                        .unwrap()
                        .into_response();
                } else {
                    let reason = status.canonical_reason().unwrap_or("Internal Server Error");
                    return (status, format!("{} {}", status.as_u16(), reason)).into_response();
                }
                // End of handle_error_request logic
            }
                    }
                }
            }

            // If no module handled the request, use the default 404 or error handler
            if let Some(eh_wrapper_arc) = &error_handler_for_fallback_clone {
                let eh = &eh_wrapper_arc.c_error_handler;
                let status_code_u16 = StatusCode::NOT_FOUND.as_u16();
                let message_cstring = CString::new("Not Found").unwrap();
                let module_name_cstring = CString::new("Unknown Module".to_string()).unwrap(); // Renamed to module_name_cstring
                let params_json = serde_json::json!({
                    "request_method": req.method().as_str(),
                    "request_path": path,
                    "user_agent": req.headers().get("User-Agent").and_then(|h| h.to_str().ok()).unwrap_or("Unknown"),
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                }).to_string();
                let params_cstring = CString::new(params_json).unwrap();

                let module_context_cstring = CString::new("").unwrap();
                error!("ox_webservice: Calling error handler handle_error_fn for status_code: {}", status_code_u16);
                let response_ptr = unsafe {
                    (eh.handle_error_fn)(
                        eh.instance_ptr,
                        status_code_u16,
                        message_cstring.into_raw(),
                        module_name_cstring.into_raw(), // Pass module_name here
                        params_cstring.into_raw(),
                        module_context_cstring.into_raw(),
                    )
                };

                let html_response = if response_ptr.is_null() {
                    error!("handle_error_fn returned a null pointer. Returning generic error HTML.");
                    "<h1>Internal Server Error</h1><p>Error handler returned null.</p>".to_string()
                } else {
                    let c_str = unsafe { CStr::from_ptr(response_ptr) };
                    let response = c_str.to_str().unwrap().to_string();
                    unsafe { CString::from_raw(response_ptr) }; // Deallocate the CString
                    response
                };

                axum::response::Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header(axum::http::header::CONTENT_TYPE, "text/html")
                    .body(axum::body::Body::from(html_response))
                    .unwrap()
                    .into_response()
            } else {
                let status = axum::http::StatusCode::NOT_FOUND;
                let reason = status.canonical_reason().unwrap_or("Not Found");
                return (status, format!("{} {}", status.as_u16(), reason)).into_response();
            }
        }
    });

    // We need to drop the CString after the loop, but before `axum::serve`
    // to ensure the pointer is valid during module initialization.
    // However, `initial_context_cstring` is dropped at the end of the `main` function scope.
    // This is fine as `initialize_module` is expected to copy the data it needs.

    // Run it
    info!("listening on {}", addr);

    let listener = TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
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

fn call_module_handler(
    handler_fn: SendableWebServiceHandler,
    request_cstring: CString,
    path: String,
) -> Result<serde_json::Value, serde_json::Value> {
    unsafe {
        let response_ptr = (handler_fn.0)(request_cstring.into_raw());

        if response_ptr.is_null() {
            error!("Handler returned a null pointer for path: {}", path);
            return Err(serde_json::json!({
                "status": 500,
                "message": format!("Handler for path '{}' returned a null response.", path)
            }));
        }

        let c_str = CStr::from_ptr(response_ptr as *mut c_char);
        let response_json_str = c_str.to_str().map_err(|e| format!("Failed to convert CStr to &str: {}", e))?.to_string();

        let _ = CString::from_raw(response_ptr as *mut c_char); // Deallocate the memory

        let json_value: serde_json::Value = serde_json::from_str(&response_json_str)
            .map_err(|e| format!("Failed to parse JSON from module handler: {}", e))?;

        // If the module's response contains a status code >= 400, treat it as an error
        if let Some(status_code) = json_value.get("status").and_then(|s| s.as_u64()) {
            if status_code >= 400 {
                return Err(json_value);
            }
        }

        Ok(json_value)
    }
}

#[no_mangle]
pub extern "C" fn render_template_ffi(template_name_ptr: *mut c_char, data_ptr: *mut c_char) -> *mut c_char {
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

    let tera_instance = unsafe {
        GLOBAL_TERA.as_ref().expect("GLOBAL_TERA not initialized")
    };

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
