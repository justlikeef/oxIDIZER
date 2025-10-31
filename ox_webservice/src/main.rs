use axum::{routing::get, Json, Router, response::IntoResponse};
use clap::Parser;
use serde::Deserialize;
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
use log::{info, debug, trace, error};
use env_logger::Env;

use libloading::{Library, Symbol};

use std::panic;

use ox_webservice::{ModuleEndpoints, ModuleEndpoint, WebServiceContext};

static mut GLOBAL_TERA: Option<Arc<Tera>> = None;

#[derive(Debug, Deserialize)]
struct TemplateQuery {
    name: String,
}


#[derive(Debug, Deserialize)]
struct ServerConfig {
    port: u16,
    #[serde(default)] // Allow modules to be optional in the config file
    modules: Vec<ModuleConfig>,
    #[serde(default)] // Allow log_output_path to be optional
    log_output_path: Option<String>,
    #[serde(default = "default_log_level")]
    log_level: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Deserialize, Clone)]
struct ModuleConfig {
    name: String,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    error_path: Option<String>,
}


// A wrapper for a dynamically loaded module
struct LoadedModule {
    _library: Arc<Library>, // Keep the library loaded as an Arc
    module_name: String,
    endpoints: Vec<ModuleEndpoint>,
    module_config: ModuleConfig,
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

    /// Path to a file to redirect stdout and stderr
    #[arg(long)]
    log_output_path: Option<String>,

    /// Set the logging level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Path to the error templates for ox_content module
    #[arg(long)]
    ox_content_error_path: Option<String>,
}


#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Initialize logger based on CLI log level
    env_logger::Builder::from_env(Env::default().default_filter_or(&cli.log_level)).init();

    info!("Starting ox_webservice...");
    debug!("CLI arguments: {:?}", cli);

    // --- Configure Logging ---
    let log_file_path = cli.log_output_path.clone(); // Clone for potential use after config load

    // Set up custom panic hook
    if let Some(path) = &log_file_path {
        let path_clone = path.clone();
        panic::set_hook(Box::new(move |panic_info| {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .append(true)
                .open(&path_clone)
                .expect(&format!("Failed to open log file in panic hook: {}", path_clone));
            
            writeln!(file, "PANIC: {}", panic_info).expect("Failed to write panic info to log file");
            let _ = file.flush(); // Attempt to flush
        }));
    }


    // --- Load Server Configuration ---
    let server_config_path = Path::new(&cli.config);
    let mut server_config: ServerConfig = load_config_from_path(server_config_path, &cli.log_level);

    // --- Apply CLI Log Output Path Override ---
    // CLI takes precedence over config file for log output
    if log_file_path.is_some() {
        server_config.log_output_path = log_file_path;
    }

    // --- Redirect output if log_output_path is specified ---
    // Temporarily commented out for debugging
    // if let Some(path) = &server_config.log_output_path {
    //     info!("Redirecting output to {}", path);
    //     let file = OpenOptions::new()
    //         .create(true)
    //         .write(true)
    //         .append(true)
    //         .open(&path)
    //         .expect(&format!("Failed to open log file: {}", path));
    //     
    //     let file_handle = file.as_raw_handle();
    //
    //     unsafe {
    //         // Redirect stdout
    //         SetStdHandle(STD_OUTPUT_HANDLE, file_handle as isize);
    //         // Redirect stderr
    //         SetStdHandle(STD_ERROR_HANDLE, file_handle as isize);
    //     }
    //     info!("Output redirected to {}", path);
    // }

    // --- Override/Supplement Modules from CLI ---
    if let Some(cli_modules) = cli.modules {
        info!("Overriding modules from CLI: {:?}", cli_modules);
        server_config.modules = cli_modules
            .into_iter()
            .map(|name| {
                let mut module_config = ModuleConfig {
                    name,
                    params: None,
                    error_path: None,
                };
                // Apply CLI specific overrides
                if module_config.name == "ox_content" {
                    module_config.error_path = cli.ox_content_error_path.clone();
                }
                module_config
            })
            .collect();
    }

    // --- Initialize Tera --- 
    let tera_instance = match Tera::new("templates/**/*.html") {
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
    let mut specific_endpoints: Vec<(Arc<LoadedModule>, ModuleEndpoint)> = Vec::new();
    let mut wildcard_endpoints: Vec<(Arc<LoadedModule>, ModuleEndpoint)> = Vec::new();

    // Collect module names for WebServiceContext
    let module_names: Vec<String> = server_config.modules.iter().map(|m| m.name.clone()).collect();

    // System Info for WebServiceContext
    let os_info = format!("{} {}", System::name().unwrap_or_else(|| "Unknown".to_string()), System::os_version().unwrap_or_else(|| "Unknown".to_string()));
    let hostname = System::host_name().unwrap_or_else(|| "Unknown".to_string());
    let running_directory = std::env::current_dir().unwrap().to_str().unwrap().to_string();
    let addr = SocketAddr::from(([127, 0, 0, 1], server_config.port));

    let initial_context = WebServiceContext {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_date: env!("VERGEN_BUILD_TIMESTAMP").to_string(),
        running_directory: running_directory.clone(),
        config_file_location: cli.config.clone(),
        loaded_modules: module_names.clone(),
        hostname: hostname.clone(),
        os_info: os_info.clone(),
        total_memory_gb: 0.0, // Will be updated by the module if needed
        available_memory_gb: 0.0, // Will be updated by the module if needed
        total_disk_gb: 0.0, // Will be updated by the module if needed
        available_disk_gb: 0.0, // Will be updated by the module if needed
        server_port: server_config.port,
        bound_ip: addr.ip().to_string(),
        render_template_fn: Some(render_template_ffi),
    };

    for module_config in server_config.modules.into_iter() {
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
                    let initialize_module_fn: Symbol<unsafe extern "C" fn(*mut c_char) -> *mut c_void> = library
                        .get(b"initialize_module")
                        .expect(&format!("Failed to find 'initialize_module' in {}", module_name));

                    // Prepare the initialization data
                    let init_data = ox_webservice::InitializationData {
                        context: initial_context.clone(), // Clone the context
                        params: module_config.params.clone().unwrap_or(Value::Null),
                    };

                    let init_data_json = serde_json::to_string(&init_data).unwrap();
                    let init_data_cstring = CString::new(init_data_json).unwrap();
                    
                    let module_endpoints_ptr = initialize_module_fn(init_data_cstring.into_raw());
                    let boxed_module_endpoints = Box::from_raw(module_endpoints_ptr as *mut ModuleEndpoints);
                    let module_endpoints = *boxed_module_endpoints;

                    let current_library = Arc::new(library); // Wrap library in Arc for sharing

                    let loaded_module = Arc::new(LoadedModule {
                        _library: current_library.clone(),
                        module_name: module_name.clone(),
                        endpoints: module_endpoints.endpoints.clone(),
                        module_config: module_config.clone(),
                    });

                    for endpoint in module_endpoints.endpoints {
                        info!("Registering route: /{}/{}", module_name, endpoint.path);

                        if endpoint.path == "*" {
                            wildcard_endpoints.push((loaded_module.clone(), endpoint));
                        } else {
                            specific_endpoints.push((loaded_module.clone(), endpoint));
                        }
                    }

                    loaded_modules.push(loaded_module);
                }
            }
            Err(e) => {
                error!("Failed to load module {}: {}", module_name, e);
            }
        }
    }

    info!("Major process state: Registering specific module endpoints.");
    // Sort wildcard endpoints by priority (lowest to highest)
    wildcard_endpoints.sort_by_key(|(_, endpoint)| endpoint.priority);

    // Register all specific module endpoints with Axum
    for (loaded_module, endpoint) in specific_endpoints {
        let full_url = format!("/{}", endpoint.path);
        let handler_fn_clone = endpoint.handler;
        let handler_library_clone = loaded_module._library.clone(); // Get the library from loaded_module
        let module_name_clone = loaded_module.module_name.clone(); // Get the module name from loaded_module

        let axum_handler = move |req: axum::http::Request<axum::body::Body>| {
            let _handler_library_clone = handler_library_clone.clone(); // Use the cloned library
            let _handler_module_name_clone = module_name_clone.clone(); // Use the cloned module name
            async move {
                let path = req.uri().path().to_string();
                let request_json = serde_json::json!({ "path": path }).to_string();
                let request_cstring = CString::new(request_json).unwrap();

                unsafe {
                    let response_ptr = handler_fn_clone(request_cstring.into_raw());
                    println!("DEBUG: response_ptr: {:?}", response_ptr);

                    let c_str = CStr::from_ptr(response_ptr);
                    println!("DEBUG: c_str: {:?}", c_str);
                    let response_json = c_str.to_str().expect("Failed to convert CStr to &str");
                    println!("DEBUG: response_json in ox_webservice: {}", response_json);

                    let _ = CString::from_raw(response_ptr as *mut c_char);

                    Json(serde_json::from_str::<serde_json::Value>(response_json).unwrap()).into_response()
                }
            }
        };
        app = app.route(&full_url, get(axum_handler));
    }

    info!("Major process state: Registering fallback handler for wildcard routes.");
    // Register a fallback handler for wildcard routes
    let shared_wildcard_endpoints = Arc::new(wildcard_endpoints);
    app = app.fallback(move |req: axum::http::Request<axum::body::Body>| {
        let path = req.uri().path().to_string();
        let wildcard_endpoints_clone = shared_wildcard_endpoints.clone();
        async move {
            for (loaded_module, endpoint) in wildcard_endpoints_clone.iter() {
                // Check if the path matches the wildcard handler (which is always true for a fallback)
                // and then call the handler.
                let handler_fn_clone = endpoint.handler;
                let _handler_library_clone = loaded_module._library.clone(); // Get the library from loaded_module
                let _handler_module_name_clone = loaded_module.module_name.clone(); // Get the module name from loaded_module

                let dummy_request = CString::new(format!("{{\"path\":\"{}\"}}", path)).unwrap();
                unsafe {
                    let response_ptr = handler_fn_clone(dummy_request.into_raw());

                    let c_str = CStr::from_ptr(response_ptr);
                    let response_json = c_str.to_str().expect("Failed to convert CStr to &str");

                    let _ = CString::from_raw(response_ptr as *mut c_char);

                    return Json(serde_json::from_str::<serde_json::Value>(response_json).unwrap()).into_response();
                }
            }
            // If no wildcard handler returns a response, then it's a 404
            (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response()
        }
    });

    // We need to drop the CString after the loop, but before `axum::serve`
    // to ensure the pointer is valid during module initialization.
    // However, `initial_context_cstring` is dropped at the end of the `main` function scope.
    // This is fine as `initialize_module` is expected to copy the data it needs.

    // Run it
    info!("listening on {}", addr);
    println!("DEBUG: Test println from ox_webservice main.rs");
    let listener = TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
fn load_config_from_path(path: &Path, cli_log_level: &str) -> ServerConfig {
    debug!("Loading config from: {:?}", path);
    trace!("File extension: {:?}", path.extension());

    if !path.exists() {
        error!("Configuration file not found at {:?}", path);
        std::process::exit(1);
    }

    let mut file = File::open(path)
        .expect(&format!("Failed to open server configuration file: {:?}", path));
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .expect(&format!("Could not read server configuration file: {:?}", path));

    debug!("Content read from config file: \n{}", contents);

    if cli_log_level == "trace" {
        trace!("Parsed config file content:\n{}", contents);
    } else if cli_log_level == "debug" {
        debug!("Parsed config file content:\n{}", contents);
    }

    match path.extension().and_then(|s| s.to_str()) {
        Some("yaml") | Some("yml") => {
            debug!("Parsing as YAML");
            serde_yaml::from_str(&contents).expect("Could not parse YAML server config")
        }
        Some("json") => {
            debug!("Parsing as JSON");
            serde_json::from_str(&contents).expect("Could not parse JSON server config")
        }
        Some("toml") => {
            debug!("Parsing as TOML");
            toml::from_str(&contents).expect("Could not parse TOML server config")
        }
        Some("xml") => {
            debug!("Parsing as XML");
            serde_xml_rs::from_str(&contents).expect("Could not parse XML server config")
        }
        _ => {
            error!("Unsupported server config file format: {:?}. Exiting.", path.extension());
            std::process::exit(1);
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