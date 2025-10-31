use axum::{routing::get, Json, Router, response::IntoResponse};
use clap::Parser;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap};
use std::ffi::{CStr, CString, c_char, c_void};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc};
use tokio::net::TcpListener;
use sysinfo::{System};
use tera::{Tera, Context};
use axum::response::Html;
use axum::extract::{Query, Path as AxumPath};

use libloading::{Library, Symbol};

use std::panic;
use std::os::windows::io::AsRawHandle;


// Import necessary types from ox_webservice
use ox_webservice::{ModuleEndpoints, ModuleEndpoint, WebServiceContext};
use windows_sys::Win32::System::Console::{SetStdHandle, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE};

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
}


#[derive(Debug, Deserialize, Clone)]
struct ModuleConfig {
    name: String,
    #[serde(default)]
    params: Option<Value>,
}


// A wrapper for a dynamically loaded module
struct LoadedModule {
    _library: Arc<Library>, // Keep the library loaded as an Arc
    module_name: String,
    endpoints: Vec<ModuleEndpoint>,
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
}


#[tokio::main]
async fn main() {
    let cli = Cli::parse();

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
    let mut server_config: ServerConfig = load_config_from_path(server_config_path);

    // --- Apply CLI Log Output Path Override ---
    // CLI takes precedence over config file for log output
    if log_file_path.is_some() {
        server_config.log_output_path = log_file_path;
    }

    // --- Redirect output if log_output_path is specified ---
    if let Some(path) = &server_config.log_output_path {
        eprintln!("Redirecting output to {}", path);
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(&path)
            .expect(&format!("Failed to open log file: {}", path));
        
        let file_handle = file.as_raw_handle();

        unsafe {
            // Redirect stdout
            SetStdHandle(STD_OUTPUT_HANDLE, file_handle as isize);
            // Redirect stderr
            SetStdHandle(STD_ERROR_HANDLE, file_handle as isize);
        }
        println!("Output redirected to {}", path);
    }

    // --- Override/Supplement Modules from CLI ---
    if let Some(cli_modules) = cli.modules {
        server_config.modules = cli_modules
            .into_iter()
            .map(|name| ModuleConfig {
                name,
                params: None, // No params from CLI, consider how to support this if needed
            })
            .collect();
    }

    // --- Initialize Tera --- 
    let tera = match Tera::new("templates/**/*.html") {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Parsing error(s): {}", e);
            ::std::process::exit(1);
        }
    };
    let tera = Arc::new(tera); // Wrap Tera in Arc for sharing

    // --- Dynamically Load Modules and Register Routes ---
    let mut app = Router::new();
    let mut loaded_modules: HashMap<String, LoadedModule> = HashMap::new();
    let mut specific_endpoints: Vec<(String, ModuleEndpoint, Arc<Library>)> = Vec::new(); // (module_name, endpoint, library)
    let mut wildcard_endpoints: Vec<(String, ModuleEndpoint, Arc<Library>)> = Vec::new(); // (module_name, endpoint, library)

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
    };

    for module_config in &server_config.modules {
        let module_name = &module_config.name;
        println!("ox_webservice: Attempting to load module: {}", module_name);
        let library_file_name = if cfg!(target_os = "windows") {
            format!("{}.dll", module_name)
        } else if cfg!(target_os = "macos") {
            format!("lib{}.dylib", module_name)
        } else {
            format!("lib{}.so", module_name)
        };

        let library_path = PathBuf::from(library_file_name);

        println!("ox_webservice: Attempting to load module: {} from {:?}", module_name, library_path);

        match unsafe { Library::new(&library_path) } {
            Ok(library) => {
                println!("ox_webservice: Successfully loaded module: {}", module_name);
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

                    let current_module_name = module_name.clone();
                    let current_library = Arc::new(library); // Wrap library in Arc for sharing

                    let mut collected_endpoints = Vec::new();
                    for endpoint in module_endpoints.endpoints {
                        let full_url = format!("/{}/{}", current_module_name, endpoint.path);
                        println!("ox_webservice: Registering route: {}", full_url);

                        if endpoint.path == "*" {
                            wildcard_endpoints.push((current_module_name.clone(), endpoint.clone(), current_library.clone()));
                        } else {
                            specific_endpoints.push((current_module_name.clone(), endpoint.clone(), current_library.clone()));
                        }
                        collected_endpoints.push(endpoint);
                    }

                    loaded_modules.insert(
                        module_name.clone(),
                        LoadedModule {
                            _library: current_library, // Store the Arc directly
                            module_name: current_module_name.clone(),
                            endpoints: collected_endpoints,
                        },
                    );
                }
            }
            Err(e) => {
                eprintln!("ox_webservice: Failed to load module {}: {}", module_name, e);
            }
        }
    }

    // Sort wildcard endpoints by priority (lowest to highest)
    wildcard_endpoints.sort_by_key(|(_, endpoint, _)| endpoint.priority);

    // Register all specific module endpoints with Axum
    for (module_name, endpoint, handler_library) in specific_endpoints {
        let full_url = format!("/{}/{}", module_name, endpoint.path);
        let handler_fn_clone = endpoint.handler;

        let axum_handler = move || {
            let _handler_library_clone = handler_library.clone();
            let _handler_module_name_clone = module_name.clone();
            async move {
                let dummy_request = CString::new("{}".to_string()).unwrap();
                unsafe {
                    let response_ptr = handler_fn_clone(dummy_request.into_raw());

                    let c_str = CStr::from_ptr(response_ptr);
                    let response_json = c_str.to_str().expect("Failed to convert CStr to &str");

                    let _ = CString::from_raw(response_ptr as *mut c_char);

                    Json(serde_json::from_str::<serde_json::Value>(response_json).unwrap()).into_response()
                }
            }
        };
        app = app.route(&full_url, get(axum_handler));
    }

    // Register a fallback handler for wildcard routes
    let shared_wildcard_endpoints = Arc::new(wildcard_endpoints);
    app = app.fallback(move |req: axum::http::Request<axum::body::Body>| {
        let path = req.uri().path().to_string();
        let wildcard_endpoints_clone = shared_wildcard_endpoints.clone();
        async move {
            for (_module_name, endpoint, _handler_library) in wildcard_endpoints_clone.iter() {
                // Check if the path matches the wildcard handler (which is always true for a fallback)
                // and then call the handler.
                let handler_fn_clone = endpoint.handler;
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

    // Register Tera rendering route
    let tera_clone = tera.clone();
    app = app.route("/render_template", axum::routing::post(move |Query(template_query): Query<TemplateQuery>, Json(data): Json<serde_json::Value>| async move {
        let mut context = Context::new();
        if let Some(obj) = data.as_object() {
            for (key, value) in obj {
                context.insert(key, value);
            }
        }
        match tera_clone.render(&template_query.name, &context) {
            Ok(s) => Html(s),
            Err(e) => {
                eprintln!("Template rendering error: {}", e);
                Html(format!("<h1>Error rendering template: {}</h1>", e))
            }
        }
    }));

    // We need to drop the CString after the loop, but before `axum::serve`
    // to ensure the pointer is valid during module initialization.
    // However, `initial_context_cstring` is dropped at the end of the `main` function scope.
    // This is fine as `initialize_module` is expected to copy the data it needs.

    // Run it
    println!("listening on {}", addr);
    let listener = TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
fn load_config_from_path(path: &Path) -> ServerConfig {
    debug_println!("Loading config from: {:?}", path);
    debug_println!("File extension: {:?}", path.extension());

    if !path.exists() {
        eprintln!("Error: Configuration file not found at {:?}", path);
        std::process::exit(1);
    }

    let mut file = File::open(path)
        .expect(&format!("Failed to open server configuration file: {:?}", path));
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .expect(&format!("Could not read server configuration file: {:?}", path));

    match path.extension().and_then(|s| s.to_str()) {
        Some("yaml") | Some("yml") => {
            debug_println!("Parsing as YAML");
            serde_yaml::from_str(&contents).expect("Could not parse YAML server config")
        }
        Some("json") => {
            debug_println!("Parsing as JSON");
            serde_json::from_str(&contents).expect("Could not parse JSON server config")
        }
        Some("toml") => {
            debug_println!("Parsing as TOML");
            toml::from_str(&contents).expect("Could not parse TOML server config")
        }
        Some("xml") => {
            debug_println!("Parsing as XML");
            serde_xml_rs::from_str(&contents).expect("Could not parse XML server config")
        }
        _ => {
            eprintln!("Error: Unsupported server config file format: {:?}. Exiting.", path.extension());
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