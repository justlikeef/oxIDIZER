use std::ffi::{c_char, c_void, CStr, CString};
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_ERROR, OX_LOG_INFO, OX_LOG_ERROR, OX_LOG_DEBUG,
};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyBytes, PyModule};

const MODULE_NAME: &str = "ox_webservice_wsgi";

#[derive(Deserialize, Debug, Clone)]
struct Config {
    config_file: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct AppConfig {
    python_path: String,
    module: String,
    #[serde(default = "default_callable")]
    callable: String,
}

fn default_callable() -> String { "application".to_string() }

struct WsgiContext {
    config: AppConfig,
    app: PyObject,
    api: CoreHostApi,
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let res_ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if res_ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(res_ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    let c_key = CString::new(key).unwrap();
    let c_val = CString::new(value).unwrap();
    (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };

    let params_str = if !plugin_config_ctx.is_null() {
        unsafe { CStr::from_ptr(plugin_config_ctx).to_string_lossy().to_string() }
    } else { "{}".to_string() };

    let config: Config = match serde_json::from_str(&params_str) {
        Ok(c) => c,
        Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to parse params: {}", e)); return std::ptr::null_mut(); }
    };

    let path = std::path::Path::new(&config.config_file);
    let app_config: AppConfig = match ox_fileproc::process_file(path, 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to parse config: {}", e)); return std::ptr::null_mut(); }
        },
        Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to load config: {}", e)); return std::ptr::null_mut(); }
    };

    pyo3::prepare_freethreaded_python();

    let app = match Python::with_gil(|py| -> PyResult<PyObject> {
        let sys = py.import("sys")?;
        let path: pyo3::Bound<'_, pyo3::types::PyList> = sys.getattr("path")?.downcast()?.clone();
        path.insert(0, &app_config.python_path)?;
        let app_module = PyModule::import(py, app_config.module.as_str())?;
        let callable = app_module.getattr(app_config.callable.as_str())?;
        if !callable.is_callable() {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>("App is not callable"));
        }
        Ok(callable.into())
    }) {
        Ok(obj) => obj,
        Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to load Python app: {}", e)); return std::ptr::null_mut(); }
    };

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, &format!("{} initialized with module: {}", MODULE_NAME, app_config.module));

    let ctx = Box::new(WsgiContext { config: app_config, app, api });
    Box::into_raw(ctx) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_process(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    if plugin_config_ctx.is_null() {
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }
    let context = unsafe { &*(plugin_config_ctx as *mut WsgiContext) };
    let api = &context.api;

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Python::with_gil(|py| -> PyResult<(i32, Vec<u8>, Vec<(String, String)>)> {
            let environ = PyDict::new(py);
            environ.set_item("REQUEST_METHOD", get_field(api, task_ctx, "request.method").to_uppercase())?;
            environ.set_item("SCRIPT_NAME", "")?;
            environ.set_item("PATH_INFO", get_field(api, task_ctx, "request.path"))?;
            environ.set_item("QUERY_STRING", get_field(api, task_ctx, "request.query"))?;
            environ.set_item("SERVER_NAME", "oxidizer")?;
            environ.set_item("SERVER_PORT", "80")?;
            environ.set_item("SERVER_PROTOCOL", "HTTP/1.1")?;
            environ.set_item("wsgi.version", (1, 0))?;
            environ.set_item("wsgi.url_scheme", get_field(api, task_ctx, "request.protocol"))?;
            environ.set_item("wsgi.multithread", true)?;
            environ.set_item("wsgi.multiprocess", false)?;
            environ.set_item("wsgi.run_once", false)?;

            let io_module = py.import("io")?;
            let body_str = get_field(api, task_ctx, "request.body");
            let body_bytes = body_str.as_bytes();
            let stream = io_module.call_method1("BytesIO", (PyBytes::new(py, body_bytes),))?;
            environ.set_item("wsgi.input", stream)?;

            let sys_mod = py.import("sys")?;
            environ.set_item("wsgi.errors", sys_mod.getattr("stderr")?)?;

            let locals = PyDict::new(py);
            locals.set_item("environ", environ)?;
            locals.set_item("app", &context.app)?;

            let code = CString::new(r#"
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
"#).unwrap();
            py.run(&code, Some(&locals), Some(&locals))?;

            let sc: i32 = locals.get_item("status_code")?.ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing status"))?.extract::<Vec<i32>>()?[0];
            let hd: Vec<(String, String)> = locals.get_item("headers")?.ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing headers"))?.extract()?;
            let body: Vec<u8> = locals.get_item("body")?.ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing body"))?.extract()?;

            Ok((sc, body, hd))
        })
    }));

    match result {
        Ok(Ok((status, body, headers))) => {
            set_field(api, task_ctx, "response.status", &status.to_string());
            set_field(api, task_ctx, "response.body", &String::from_utf8_lossy(&body));
            for (k, v) in headers {
                let header_key = format!("response.header.{}", k);
                set_field(api, task_ctx, &header_key, &v);
            }
            FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
        }
        Ok(Err(e)) => {
            log(api, task_ctx, OX_LOG_ERROR, &format!("WSGI Error: {}", e));
            set_field(api, task_ctx, "response.status", "500");
            FlowControl { code: FLOW_CONTROL_ERROR, payload: std::ptr::null() }
        }
        Err(e) => {
            log(api, task_ctx, OX_LOG_ERROR, &format!("WSGI Panic: {:?}", e));
            set_field(api, task_ctx, "response.status", "500");
            FlowControl { code: FLOW_CONTROL_ERROR, payload: std::ptr::null() }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        let _ = Box::from_raw(plugin_config_ctx as *mut WsgiContext);
    }
}
