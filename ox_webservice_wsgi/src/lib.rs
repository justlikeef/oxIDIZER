use std::ffi::{c_char, c_void, CStr, CString};
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ox_webservice_api::{
    ModuleInterface, HandlerResult, ModuleStatus, FlowControl, ReturnParameters, 
    LogCallback, AllocStrFn, CoreHostApi, LogLevel, PipelineState, AllocFn
};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyBytes, PyModule, PyTuple};

#[derive(Deserialize, Debug, Clone)]
struct Config {
    /// Path to the configuration file
    config_file: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct AppConfig {
    /// Path to the directory containing the python application
    python_path: String,
    /// Name of the python module to import (e.g., "app" for app.py)
    module: String,
    /// Name of the callable entry point (default: "application")
    #[serde(default = "default_callable")]
    callable: String,
}

fn default_callable() -> String {
    "application".to_string()
}

// Custom context to hold the Python application object
struct WsgiModuleContext {
    config: AppConfig,
    // The compiled Python application callable
    app: PyObject, 
    // module_id: String, // Removed unused field
    api: &'static CoreHostApi,
}

#[no_mangle]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    module_id: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api = unsafe { &*api_ptr };

    let module_id_str = if !module_id.is_null() {
        unsafe { CStr::from_ptr(module_id).to_string_lossy().to_string() }
    } else {
        "ox_webservice_wsgi".to_string()
    };

    let _ = ox_webservice_api::init_logging(api.log_callback, &module_id_str);

    let params_str = if !module_params_json_ptr.is_null() {
        unsafe { CStr::from_ptr(module_params_json_ptr).to_string_lossy().to_string() }
    } else {
        "{}".to_string()
    };
    
    let config: Config = match serde_json::from_str(&params_str) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to parse params: {}", e);
            return std::ptr::null_mut();
        }
    };

    // Load AppConfig from file
    let path = std::path::Path::new(&config.config_file);
    let app_config: AppConfig = match ox_fileproc::process_file(path, 5) {
        Ok(value) => {
            match serde_json::from_value(value) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Failed to parse config file {}: {}", config.config_file, e);
                    return std::ptr::null_mut();
                }
            }
        },
        Err(e) => {
             log::error!("Failed to load config file {}: {}", config.config_file, e);
             return std::ptr::null_mut();
        }
    };

    // Initialize Python
    // Note: In a real server with multiple WSGI modules, we must ensure 
    // pyo3::prepare_freethreaded_python is called once, or rely on auto-init.
    pyo3::prepare_freethreaded_python();

    let app = match Python::with_gil(|py| -> PyResult<PyObject> {
        let sys = py.import("sys")?;
        let path: Bound<'_, PyList> = sys.getattr("path")?.downcast()?.clone();
        path.insert(0, &app_config.python_path)?;

        let app_module = PyModule::import(py, app_config.module.as_str())?;
        let app_callable = app_module.getattr(app_config.callable.as_str())?;
        
        if !app_callable.is_callable() {
             return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>("App object is not callable"));
        }
        
        Ok(app_callable.into())
    }) {
        Ok(obj) => obj,
        Err(e) => {
            log::error!("Failed to load python application: {}", e);
            return std::ptr::null_mut();
        }
    };

    let ctx = Box::new(WsgiModuleContext {
        config: app_config,
        app,
        // module_id: module_id_str,
        api,
    });

    let interface = Box::new(ModuleInterface {
        instance_ptr: Box::into_raw(ctx) as *mut c_void,
        handler_fn: process_request,
        log_callback: api.log_callback,
        get_config: get_config,
    });

    Box::into_raw(interface)
}

#[no_mangle]
pub unsafe extern "C" fn process_request(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    _log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena_ptr: *const c_void,
) -> HandlerResult {
    // Use the API logger as requested
    log::debug!("WSGI: Entered process_request");
    
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        log::debug!("WSGI: Inside catch_unwind");

        if instance_ptr.is_null() || pipeline_state_ptr.is_null() {
            log::error!("WSGI: Null pointers passed to handler");
            return HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Halt,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
            };
        }

        let context = unsafe { &*(instance_ptr as *mut WsgiModuleContext) };
        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const bumpalo::Bump as *const c_void;
        
        log::debug!("WSGI: Creating PipelineContext");
        let ctx = unsafe { ox_pipeline_plugin::PipelineContext::new(
            context.api, 
            pipeline_state_ptr as *mut c_void, 
            arena_ptr
        ) };
        log::debug!("WSGI: PipelineContext created");

        // Prepare WSGI Environment
        let python_result = Python::with_gil(|py| -> PyResult<(i32, Vec<u8>, Vec<(String, String)>)> {
            log::debug!("WSGI: Acquired GIL");
            let environ = PyDict::new(py);
            
            // Basic WSGI inputs
            environ.set_item("REQUEST_METHOD", ctx.get("request.method").and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or("GET".to_string()))?;
            environ.set_item("SCRIPT_NAME", "")?; // Root
            environ.set_item("PATH_INFO", ctx.get("request.path").and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or("/".to_string()))?;
            environ.set_item("QUERY_STRING", ctx.get("request.query_string").and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or("".to_string()))?;
            environ.set_item("SERVER_NAME", "oxidizer")?; 
            environ.set_item("SERVER_PORT", "80")?; // TODO: Get actual port
            environ.set_item("SERVER_PROTOCOL", "HTTP/1.1")?;
            
            environ.set_item("wsgi.version", (1, 0))?;
            environ.set_item("wsgi.url_scheme", "http")?;
            environ.set_item("wsgi.multithread", true)?;
            environ.set_item("wsgi.multiprocess", false)?;
            environ.set_item("wsgi.run_once", false)?;
            
            // Input stream (body) 
            let io_module = py.import("io")?;
            let body_val = ctx.get("request.body");
            let body_bytes = body_val.as_ref().and_then(|v| v.as_str()).map(|s| s.as_bytes()).unwrap_or(&[]);
            let stream = io_module.call_method1("BytesIO", (PyBytes::new(py, body_bytes),))?;
            environ.set_item("wsgi.input", stream)?;
            
            environ.set_item("wsgi.errors", sys_stderr(py)?)?;
            
            let locals = PyDict::new(py);
            locals.set_item("environ", environ)?;
            locals.set_item("app", &context.app)?;
            
            let py_code = r#"
status_code = [500]
headers = []
output_chunks = []

def start_response(status, response_headers, exc_info=None):
    status_code[0] = int(status.split(' ')[0])
    headers.extend(response_headers)
    return output_chunks.append

iterable = app(environ, start_response)
try:
    for data in iterable:
        output_chunks.append(data)
finally:
    if hasattr(iterable, 'close'):
        iterable.close()
    
body = b''.join(output_chunks)
"#;
            let c_code = CString::new(py_code).unwrap();
            py.run(&c_code, Some(&locals), Some(&locals))?;
            
            // Extract results
            let sc_item = locals.get_item("status_code")?
                 .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing status_code"))?;
            let sc: i32 = sc_item.extract::<Vec<i32>>()?[0];
                
            let hd_item = locals.get_item("headers")?
                 .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing headers"))?;
            let hd_list = hd_item.extract::<Vec<(String, String)>>()?;
                
            let body_item = locals.get_item("body")?
                 .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing body"))?;
            let body = body_item.extract::<Vec<u8>>()?;
            
            Ok((sc, body, hd_list))
        });

        match python_result {
            Ok((status, body, headers)) => {
                let _ = ctx.set("response.status", serde_json::json!(status));
                // Direct access to pipeline_state is available:
                pipeline_state.response_body = body; // Direct access!

                for (k, v) in headers {
                    pipeline_state.response_headers.insert(
                        axum::http::HeaderName::try_from(k.as_str()).unwrap_or(axum::http::header::CONTENT_TYPE),
                        axum::http::HeaderValue::from_str(&v).unwrap_or(axum::http::HeaderValue::from_static(""))
                    );
                }
                
                HandlerResult {
                    status: ModuleStatus::Modified,
                    flow_control: FlowControl::Continue,
                    return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
                }
            },
            Err(e) => {
                log::error!("WSGI Execution Error: {}", e);
                 HandlerResult {
                    status: ModuleStatus::Unmodified, 
                    flow_control: FlowControl::Halt, 
                    return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
                }
            }
        }
    }));

    match result {
        Ok(handler_result) => handler_result,
        Err(e) => {
            log::error!("WSGI Module Panicked: {:?}", e);
            HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Halt,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
            }
        }
    }
}

// Helper for stderr
fn sys_stderr(py: Python<'_>) -> PyResult<PyObject> {
    let sys = py.import("sys")?;
    Ok(sys.getattr("stderr")?.into())
}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_config(
    instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    if instance_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let context = unsafe { &*(instance_ptr as *mut WsgiModuleContext) };
    
    let json = serde_json::to_string(&context.config).unwrap_or("{}".to_string());
    let json_cstring = CString::new(json).unwrap_or_else(|_| CString::new("{}").unwrap());
    unsafe { alloc_fn(arena, json_cstring.as_ptr()) }
}

