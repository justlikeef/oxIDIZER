use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE,
    OX_LOG_INFO, OX_LOG_ERROR,
};
use ox_persistence::{OxBuffer, DriverMetadata};
use std::ffi::{c_char, c_void, c_int, CStr, CString};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use libloading::{Library, Symbol};
use std::path::Path;

type InitFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type DestroyFn = unsafe extern "C" fn(*mut c_void);
type PersistFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type RestoreFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> OxBuffer;
type FetchFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> OxBuffer;
type FreeBufferFn = unsafe extern "C" fn(OxBuffer);
type GetMetadataFn = unsafe extern "C" fn() -> *mut c_char;

struct LoadedDriver {
    library: Library,
    context: *mut c_void,
    destroy_fn: DestroyFn,
    persist_fn: PersistFn,
    restore_fn: RestoreFn,
    fetch_fn: FetchFn,
    free_buffer_fn: FreeBufferFn,
    metadata: DriverMetadata,
}

unsafe impl Send for LoadedDriver {}
unsafe impl Sync for LoadedDriver {}

impl LoadedDriver {
    unsafe fn load(path: &str) -> Result<Self, String> {
        let lib = Library::new(path).map_err(|e| format!("Failed to load library: {}", e))?;
        let init: InitFn = *lib.get(b"ox_driver_init").map_err(|e| e.to_string())?;
        let destroy: DestroyFn = *lib.get(b"ox_driver_destroy").map_err(|e| e.to_string())?;
        let persist: PersistFn = *lib.get(b"ox_driver_persist").map_err(|e| e.to_string())?;
        let restore: RestoreFn = *lib.get(b"ox_driver_restore").map_err(|e| e.to_string())?;
        let fetch: FetchFn = *lib.get(b"ox_driver_fetch").map_err(|e| e.to_string())?;
        let free_buf: FreeBufferFn = *lib.get(b"ox_driver_free_buffer").map_err(|e| e.to_string())?;
        let get_meta: GetMetadataFn = *lib.get(b"ox_driver_get_driver_metadata").map_err(|e| e.to_string())?;
        let config = CString::new("{}").unwrap();
        let ctx = init(config.as_ptr());
        let meta_ptr = get_meta();
        let meta_str = CStr::from_ptr(meta_ptr).to_string_lossy();
        let metadata: DriverMetadata = serde_json::from_str(&meta_str).map_err(|e| e.to_string())?;
        Ok(LoadedDriver { library: lib, context: ctx, destroy_fn: destroy, persist_fn: persist, restore_fn: restore, fetch_fn: fetch, free_buffer_fn: free_buf, metadata })
    }
}

impl Drop for LoadedDriver {
    fn drop(&mut self) { unsafe { (self.destroy_fn)(self.context); } }
}

struct DriverManager { drivers: HashMap<String, LoadedDriver> }

lazy_static::lazy_static! {
    static ref DRIVER_MANAGER: Mutex<DriverManager> = Mutex::new(DriverManager { drivers: HashMap::new() });
}

pub struct ModuleContext { api: CoreHostApi }

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let p = (api.get_field)(task_ctx, c_key.as_ptr());
    if p.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    let c_key = CString::new(key).unwrap();
    let c_val = CString::new(value).unwrap();
    (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

fn json_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    _plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };
    log(&api, std::ptr::null_mut(), OX_LOG_INFO, "ox_data_broker initialized");
    let ctx = Box::new(ModuleContext { api });
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
    let ctx = unsafe { &*(plugin_config_ctx as *mut ModuleContext) };
    let api = &ctx.api;

    let request_path = get_field(api, task_ctx, "request.path");
    let method = get_field(api, task_ctx, "request.method").to_uppercase();

    if request_path == "/drivers" && method == "GET" {
        let manager = DRIVER_MANAGER.lock().unwrap();
        let meta_list: Vec<&DriverMetadata> = manager.drivers.values().map(|d| &d.metadata).collect();
        let json = serde_json::to_string(&meta_list).unwrap_or_default();
        json_response(api, task_ctx, 200, &json);
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    if request_path == "/drivers/reload" && method == "POST" {
        use ox_fileproc::process_file;
        use ox_persistence::DriversList;

        let mut manager = DRIVER_MANAGER.lock().unwrap();
        let drivers_path = Path::new("conf/drivers.yaml");
        let mut loaded_count = 0;
        let mut errors = Vec::new();

        if drivers_path.exists() {
            match process_file(drivers_path, 5) {
                Ok(val) => {
                    if let Ok(list) = serde_json::from_value::<DriversList>(val) {
                        for driver_conf in list.drivers {
                            if driver_conf.state == "enabled" {
                                let dir_path = if !driver_conf.library_path.is_empty() {
                                    Path::new(&driver_conf.library_path).to_path_buf()
                                } else {
                                    Path::new("target/debug").to_path_buf()
                                };
                                #[cfg(target_os = "linux")] let filename = format!("lib{}.so", driver_conf.name);
                                #[cfg(target_os = "macos")] let filename = format!("lib{}.dylib", driver_conf.name);
                                #[cfg(target_os = "windows")] let filename = format!("{}.dll", driver_conf.name);
                                #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))] let filename = format!("lib{}.so", driver_conf.name);
                                let lib_path = dir_path.join(filename);
                                let lib_path_str = lib_path.to_string_lossy().to_string();
                                match unsafe { LoadedDriver::load(&lib_path_str) } {
                                    Ok(loaded) => {
                                        let name = loaded.metadata.name.clone();
                                        manager.drivers.insert(name.clone(), loaded);
                                        log(api, task_ctx, OX_LOG_INFO, &format!("Loaded driver: {}", name));
                                        loaded_count += 1;
                                    }
                                    Err(e) => errors.push(format!("Failed '{}': {}", driver_conf.name, e)),
                                }
                            }
                        }
                    }
                }
                Err(e) => errors.push(format!("Config error: {}", e)),
            }
        } else {
            errors.push("drivers.yaml not found".to_string());
        }

        let status: u16 = if errors.is_empty() { 200 } else { 207 };
        let msg = serde_json::json!({"loaded": loaded_count, "errors": errors}).to_string();
        json_response(api, task_ctx, status, &msg);
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    if request_path.starts_with("/data/") {
        let parts: Vec<&str> = request_path.split('/').collect();
        if parts.len() < 4 {
            json_response(api, task_ctx, 400, r#"{"error":"Invalid path"}"#);
            return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }
        let driver_name = parts[2];
        let operation = parts[3];
        let body = get_field(api, task_ctx, "request.body");
        let manager = DRIVER_MANAGER.lock().unwrap();
        if let Some(driver) = manager.drivers.get(driver_name) {
            let location = CString::new("data.csv").unwrap();
            match (method.as_ref(), operation) {
                ("POST", "persist") => unsafe {
                    let data_c = CString::new(body).unwrap_or_default();
                    let result = (driver.persist_fn)(driver.context, data_c.as_ptr(), location.as_ptr());
                    json_response(api, task_ctx, if result == 0 { 200 } else { 500 }, if result == 0 { r#"{"ok":true}"# } else { r#"{"error":"persist failed"}"# });
                },
                ("GET", "restore") => unsafe {
                    let id_c = CString::new(body).unwrap_or_default();
                    let buf = (driver.restore_fn)(driver.context, location.as_ptr(), id_c.as_ptr());
                    let json = buf.to_string();
                    (driver.free_buffer_fn)(buf);
                    json_response(api, task_ctx, 200, &json);
                },
                ("POST", "fetch") => unsafe {
                    let filter_c = CString::new(body).unwrap_or_default();
                    let buf = (driver.fetch_fn)(driver.context, filter_c.as_ptr(), location.as_ptr());
                    let json = buf.to_string();
                    (driver.free_buffer_fn)(buf);
                    json_response(api, task_ctx, 200, &json);
                },
                _ => { json_response(api, task_ctx, 405, r#"{"error":"Method not allowed"}"#); }
            }
        } else {
            json_response(api, task_ctx, 404, &format!("{{\"error\":\"Driver {} not found\"}}", driver_name));
        }
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    json_response(api, task_ctx, 404, r#"{"error":"Not found"}"#);
    FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        let _ = Box::from_raw(plugin_config_ctx as *mut ModuleContext);
    }
}