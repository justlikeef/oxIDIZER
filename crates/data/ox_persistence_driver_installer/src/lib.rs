use ox_fileproc::{process_file, RawFile};
use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};
use std::ffi::{c_char, c_void, CStr, CString};
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_INFO,
};
use serde_json::Value;
use ox_persistence::{ConfiguredDriver, DriversList};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;

const MODULE_NAME: &str = "ox_persistence_driver_installer";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstallerConfig {
    pub drivers_file: String,
    pub driver_root: String,
}

impl Default for InstallerConfig {
    fn default() -> Self {
        Self {
            drivers_file: "conf/drivers.yaml".to_string(),
            driver_root: "conf/drivers".to_string(),
        }
    }
}

/// Proto-compatible mirror of InstallerConfig
#[derive(prost::Message, Clone)]
pub struct InstallerConfigProto {
    #[prost(string, tag = "1")]
    pub drivers_file: String,
    #[prost(string, tag = "2")]
    pub driver_root: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[derive(prost::Message)]
pub struct StatusInfo {
    #[prost(string, tag = "1")]
    pub status: String,
    #[prost(uint32, tag = "2")]
    pub progress: u32,
    #[prost(string, tag = "3")]
    pub message: String,
    #[prost(string, tag = "4")]
    pub package_name: String,
}

pub struct ModuleContext {
    config: InstallerConfig,
    api: CoreHostApi,
    status: Arc<RwLock<HashMap<String, StatusInfo>>>,
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

fn get_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> Option<Vec<u8>> {
    let c_key = CString::new(key).unwrap();
    let mut len: usize = 0;
    let ptr = (api.get_field_bytes)(task_ctx, c_key.as_ptr(), &mut len as *mut usize);
    if ptr.is_null() || len == 0 { return None; }
    Some(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

fn set_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, data: &[u8]) {
    let c_key = CString::new(key).unwrap();
    (api.set_field_bytes)(task_ctx, c_key.as_ptr(), data.as_ptr(), data.len());
}

fn json_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
}

impl ModuleContext {
    #[allow(dead_code)]
    fn load_configured_drivers(&self) -> Result<DriversList, String> {
        let path = Path::new(&self.config.drivers_file);
        if !path.exists() { return Ok(DriversList { drivers: Vec::new() }); }
        let val = process_file(path, 5).map_err(|e| e.to_string())?;
        serde_json::from_value(val).map_err(|e| e.to_string())
    }

    fn upsert_configured_driver(&self, driver: ConfiguredDriver) -> Result<(), String> {
        let path = Path::new(&self.config.drivers_file);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(path, "drivers: []\n").map_err(|e| e.to_string())?;
        }

        let mut raw = RawFile::open(path).map_err(|e| e.to_string())?;
        let query = format!("drivers[id=\"{}\"]", driver.id);
        let yaml_str = serde_yaml::to_string(&driver).map_err(|e| e.to_string())?;
        let clean_yaml = yaml_str.trim_start_matches("---\n").trim();

        let existing_span = raw.find(&query).next().map(|c| c.span.clone());
        if let Some(span) = existing_span {
            raw.update(span, clean_yaml);
        } else {
            let drivers_info = raw.find("drivers").next().map(|c| (c.span.clone(), c.value().trim() == "[]"));
            if let Some((span, is_empty_flow)) = drivers_info {
                let indented = clean_yaml.replace("\n", "\n    ");
                let new_entry = format!("\n  - {}", indented);
                if is_empty_flow { raw.update(span, &new_entry); } else { raw.update(span.end..span.end, &new_entry); }
            } else {
                return Err("drivers key not found".to_string());
            }
        }
        raw.save().map_err(|e| e.to_string())
    }

    fn handle_install(&self, api: &CoreHostApi, task_ctx: *mut c_void) {
        let manifest_str = get_field(api, task_ctx, "installer.manifest");
        let package_path = get_field(api, task_ctx, "installer.package_path");

        let manifest: Value = serde_json::from_str(&manifest_str).unwrap_or(Value::Null);
        let _package_name = manifest.get("name").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();

        if package_path.is_empty() {
            let err = serde_json::json!({"result": "error", "message": "Missing installer.package_path"}).to_string();
            return json_response(api, task_ctx, 400, &err);
        }

        let path = Path::new(&package_path);
        let config_path = path.join("ox_module.yaml");

        if !config_path.exists() {
            let err = serde_json::json!({"result": "error", "message": "ox_module.yaml not found"}).to_string();
            return json_response(api, task_ctx, 400, &err);
        }

        let content = match fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(e) => {
                let err = serde_json::json!({"result": "error", "message": format!("Cannot read ox_module.yaml: {}", e)}).to_string();
                return json_response(api, task_ctx, 500, &err);
            }
        };

        let module_yaml: Value = serde_yaml::from_str(&content).unwrap_or(Value::Null);
        if let Some(driver_code) = module_yaml.get("driver") {
            let id = driver_code.get("id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
            let lib = driver_code.get("library").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let lib_src = path.join(&lib);
            let lib_dest = Path::new(&self.config.driver_root).join(&lib);

            if let Some(parent) = lib_dest.parent() { let _ = fs::create_dir_all(parent); }
            if let Err(e) = fs::copy(&lib_src, &lib_dest) {
                let err = serde_json::json!({"result": "error", "message": format!("Copy failed: {}", e)}).to_string();
                return json_response(api, task_ctx, 500, &err);
            }

            let new_driver = ConfiguredDriver {
                id: id.clone(),
                friendly_name: None,
                name: driver_code.get("config").and_then(|c| c.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
                library_path: lib_dest.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                state: "enabled".to_string(),
            };

            if let Err(e) = self.upsert_configured_driver(new_driver) {
                let err = serde_json::json!({"result": "error", "message": format!("Failed to save config: {}", e)}).to_string();
                return json_response(api, task_ctx, 500, &err);
            }

            log(api, task_ctx, OX_LOG_INFO, &format!("Driver {} installed successfully", id));
            let ok = serde_json::json!({"result": "success", "message": "Driver installed"}).to_string();
            json_response(api, task_ctx, 200, &ok);
        } else {
            let err = serde_json::json!({"result": "error", "message": "No valid driver configuration found"}).to_string();
            json_response(api, task_ctx, 400, &err);
        }
    }

    fn handle_status(&self, api: &CoreHostApi, task_ctx: *mut c_void) {
        let lock = self.status.read().unwrap();
        let json = serde_json::to_string(&*lock).unwrap_or("{}".to_string());
        let body = serde_json::json!({"result": "success", "status": serde_json::from_str::<Value>(&json).unwrap_or(Value::Null)}).to_string();
        json_response(api, task_ctx, 200, &body);
    }
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

    let config: InstallerConfig = serde_json::from_str(&params_str).unwrap_or_default();
    log(&api, std::ptr::null_mut(), OX_LOG_INFO, &format!("{} initialized", MODULE_NAME));

    let ctx = Box::new(ModuleContext {
        config,
        api,
        status: Arc::new(RwLock::new(HashMap::new())),
    });
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
    let context = unsafe { &*(plugin_config_ctx as *mut ModuleContext) };
    let api = &context.api;

    let action = get_field(api, task_ctx, "installer.action");
    let path = get_field(api, task_ctx, "request.path");

    if action == "install" || path.ends_with("/install") {
        context.handle_install(api, task_ctx);
    } else if action == "status" || path.ends_with("/status") {
        context.handle_status(api, task_ctx);
    }

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
