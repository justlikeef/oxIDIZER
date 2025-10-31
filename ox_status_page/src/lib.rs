use libc::{c_char, c_void};
use std::ffi::{CStr, CString};
use minijinja::{Environment};
use serde_json;
use ox_webservice::{ModuleEndpoints, WebServiceHandler, WebServiceContext, ModuleEndpoint};

// The HTML template for the status page
const STATUS_PAGE_TEMPLATE: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>ox_webservice Status</title>
    <style>
        body { font-family: sans-serif; margin: 2em; }
        h1 { color: #333; }
        .info { background-color: #f0f0f0; padding: 1em; border-radius: 5px; margin-bottom: 1em; }
        .modules ul { list-style-type: none; padding: 0; }
        .modules li { background-color: #e0e0e0; margin: 0.5em 0; padding: 0.5em; border-radius: 3px; }
    </style>
</head>
<body>
    <h1>ox_webservice Status</h1>

    <div class="info">
        <h2>Service Information</h2>
        <p><strong>Version:</strong> {{ version }}</p>
        <p><strong>Build Date:</strong> {{ build_date }}</p>
        <p><strong>Running Directory:</strong> {{ running_directory }}</p>
        <p><strong>Config File Location:</strong> {{ config_file_location }}</p>
    </div>

    <div class="modules">
        <h2>Loaded Modules</h2>
        {% if loaded_modules %}
            <ul>
                {% for module in loaded_modules %}
                    <li>{{ module }}</li>
                {% endfor %}
            </ul>
        {% else %}
            <p>No modules loaded.</p>
        {% endif %}
    </div>
</body>
</html>
"#;

// FFI-compatible handler for the /status endpoint
#[no_mangle]
pub extern "C" fn status_page_handler(_request_ptr: *mut c_char) -> *mut c_char {
    // In a real scenario, _request_ptr would contain serialized request data
    // For the status page, we don't need request data, so we ignore it.

    // Get context information (this is a placeholder, actual context will be passed)
    let context = WebServiceContext {
        version: "0.1.0".to_string(), // Placeholder
        build_date: "Unknown".to_string(), // Placeholder
        running_directory: "Unknown".to_string(), // Placeholder
        config_file_location: "Unknown".to_string(), // Placeholder
        loaded_modules: vec![], // Placeholder
        hostname: "Unknown".to_string(),
        os_info: "Unknown".to_string(),
        total_memory_gb: 0.0,
        available_memory_gb: 0.0,
        total_disk_gb: 0.0,
        available_disk_gb: 0.0,
        server_port: 0,
        bound_ip: "0.0.0.0".to_string(),
    };

    let mut env = Environment::new();
    env.add_template("status_page", STATUS_PAGE_TEMPLATE).unwrap();

    let tmpl = env.get_template("status_page").unwrap();
    let rendered = tmpl.render(minijinja::context! {
        version => context.version,
        build_date => context.build_date,
        running_directory => context.running_directory,
        config_file_location => context.config_file_location,
        loaded_modules => context.loaded_modules,
    }).unwrap();

    CString::new(rendered).expect("Failed to create CString from rendered template").into_raw()
}

// This function will be called by ox_webservice to initialize the module
#[no_mangle]
pub extern "C" fn initialize_module() -> *mut c_void {
    let endpoints = vec![
        ModuleEndpoint { path: "/status".to_string(), handler: status_page_handler, priority: 0 },
    ];
    let boxed_endpoints = Box::new(ModuleEndpoints { endpoints });
    Box::into_raw(boxed_endpoints) as *mut c_void
}
