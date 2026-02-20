use ox_fileproc::{process_file, RawFile};
use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use std::ffi::{CString, CStr};
use libc::{c_char, c_void};
use ox_webservice_api::{
    ModuleInterface, PipelineState, HandlerResult,
    LogCallback, AllocFn, AllocStrFn,
    ModuleStatus, FlowControl, ReturnParameters, LogLevel, CoreHostApi,
};
use serde_json::Value;
use ox_pipeline_plugin::PipelineContext;
use ox_persistence::{ConfiguredDriver, DriversList};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use bumpalo::Bump;

const MODULE_NAME: &str = "ox_persistence_driver_installer";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstallerConfig {
    pub drivers_file: String,
    pub driver_root: String,
}

impl Default for InstallerConfig {
    fn default() -> Self {
        Self {
            drivers_file: "/var/repos/oxIDIZER/ox_persistence/conf/drivers.yaml".to_string(),
            driver_root: "/var/repos/oxIDIZER/conf/drivers".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StatusInfo {
    pub status: String, // "installing", "success", "failed", "pending"
    pub progress: u32,
    pub message: String,
    pub package_name: String,
}

pub struct DriverInstaller {
    config: InstallerConfig,
    module_id: String,
    api: &'static CoreHostApi,
    status: Arc<RwLock<HashMap<String, StatusInfo>>>,
}

impl DriverInstaller {
    pub fn new(api: &'static CoreHostApi, config: InstallerConfig, module_id: String) -> Self {
        Self { 
            api, 
            config, 
            module_id,
            status: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn log(&self, ctx: &PipelineContext, level: LogLevel, message: String) {
        if let (Ok(c_mid), Ok(c_msg)) = (CString::new(self.module_id.clone()), CString::new(message)) {
            unsafe {
                (self.api.log_callback)(level, c_mid.as_ptr(), c_msg.as_ptr());
            }
        }
    }

    fn update_status(&self, ctx: &PipelineContext, package_name: &str, status: &str, progress: u32, message: &str) {
        if let Ok(mut lock) = self.status.write() {
            self.log(ctx, LogLevel::Info, format!("DEBUG: update_status: package={}, status={}, msg={}", package_name, status, message));
            lock.insert(package_name.to_string(), StatusInfo {
                status: status.to_string(),
                progress,
                message: message.to_string(),
                package_name: package_name.to_string(),
            });
        }
    }

    pub fn handle_status(&self, ctx: &PipelineContext) -> HandlerResult {
        let package_name = ctx.get("installer.package_name")
            .or(ctx.get("package"))
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        
        self.log(ctx, LogLevel::Info, format!("DEBUG: DriverManager::handle_status entered for package: {}", package_name));
        
        let status_json = if package_name.is_empty() {
            let lock = self.status.read().unwrap_or_else(|e| e.into_inner());
            self.log(ctx, LogLevel::Info, format!("DEBUG: handle_status: returning all status, count={}", lock.len()));
            serde_json::to_value(&*lock).unwrap_or(Value::Null)
        } else {
            let lock = self.status.read().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = lock.get(&package_name) {
                self.log(ctx, LogLevel::Info, format!("DEBUG: handle_status: found status for {}: {}", package_name, s.status));
                serde_json::to_value(s).unwrap_or(Value::Null)
            } else {
                self.log(ctx, LogLevel::Info, format!("DEBUG: handle_status: status NOT found for {}. Available: {:?}", package_name, lock.keys()));
                Value::Null
            }
        };

        let response_json = serde_json::json!({
            "result": "success",
            "status": status_json
        });

        let _ = ctx.set("response.status", serde_json::json!(200));
        let _ = ctx.set("response.body", Value::String(response_json.to_string()));
        let _ = ctx.set("response.headers.Content-Type", Value::String("application/json".to_string()));
        
        HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        }
    }

    fn installer_response(&self, ctx: &PipelineContext, status: &str, message: &str) -> HandlerResult {
        // Construct standard JSON response for installer
        let mut response = serde_json::Map::new();
        response.insert("result".to_string(), Value::String(status.to_string()));
        
        if status == "error" {
             response.insert("message".to_string(), Value::String(message.to_string()));
        } else {
             let mut inner_status = serde_json::Map::new();
             inner_status.insert("status".to_string(), Value::String(status.to_string()));
             inner_status.insert("message".to_string(), Value::String(message.to_string()));
             response.insert("status".to_string(), Value::Object(inner_status));
        }

        let _ = ctx.set("response.body", Value::Object(response));
        let _ = ctx.set("response.status", Value::Number(200.into())); // Helper returns 200 even for errors, as per user request

        HandlerResult {
            status: ModuleStatus::Modified, // Or Unmodified? Installer usually modifies state via side effects, but response is just JSON. Stick to Modified if we set response.
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        }
    }

    fn load_configured_drivers(&self) -> Result<DriversList, String> {
        let path = Path::new(&self.config.drivers_file);
        if !path.exists() {
             return Ok(DriversList { drivers: Vec::new() });
        }
        let val = process_file(path, 5).map_err(|e| e.to_string())?;
        serde_json::from_value(val).map_err(|e| e.to_string())
    }

    fn upsert_configured_driver(&self, driver: ConfiguredDriver) -> Result<(), String> {
        let path = Path::new(&self.config.drivers_file);
        
        // Ensure file exists with basic structure if missing
        if !path.exists() {
             if let Some(parent) = path.parent() {
                 fs::create_dir_all(parent).map_err(|e| e.to_string())?;
             }
             fs::write(path, "drivers: []\n").map_err(|e| e.to_string())?;
        }

        let mut raw = RawFile::open(path).map_err(|e| e.to_string())?;
        let query = format!("drivers[id=\"{}\"]", driver.id);
        
        // Serialize partial driver object to YAML, then trim "---" and newlines
        let yaml_str = serde_yaml::to_string(&driver).map_err(|e| e.to_string())?;
        let clean_yaml = yaml_str.trim_start_matches("---\n").trim();

        // Check if driver exists
        let existing_span = raw.find(&query).next().map(|c| c.span.clone());

        if let Some(span) = existing_span {
             // Update existing entry
             raw.update(span, clean_yaml);
        } else {
             // Append new entry
             // 1. Find keys list span
             let drivers_info = raw.find("drivers").next().map(|c| (c.span.clone(), c.value().trim() == "[]"));

             if let Some((span, is_empty_flow)) = drivers_info {
                 let indented_body = clean_yaml.replace("\n", "\n    ");
                 let new_entry = format!("\n  - {}", indented_body);
                 
                 if is_empty_flow {
                     raw.update(span, &new_entry);
                 } else {
                     raw.update(span.end..span.end, &new_entry);
                 }
             } else {
                 return Err("drivers key not found in config".to_string());
             }
        }
        
        raw.save().map_err(|e| e.to_string())
    }

    pub fn handle_install(&self, ctx: &PipelineContext) -> HandlerResult {
        // Assume package_path is a directory as per new flow
        let manifest = ctx.get("installer.manifest").unwrap_or(Value::Null);
        let package_path_val = ctx.get("installer.package_path").unwrap_or(Value::Null);
        let package_path = package_path_val.as_str().map(|s| s.to_string()).unwrap_or_default();
        
        let package_name = manifest.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
        self.log(ctx, LogLevel::Info, format!("DEBUG: DriverInstaller::handle_install entered. path={}", package_path));
        self.update_status(ctx, package_name, "installing", 0, "Starting installation");

        if package_path.is_empty() {
             self.update_status(ctx, package_name, "failed", 0, "Missing package_path");
            return self.installer_response(ctx, "error", "Missing installer.package_path");
        }

        // Look for ox_module.yaml in the extracted directory
        let path = Path::new(&package_path);
        let config_path = path.join("ox_module.yaml");
        let mut driver_config: Option<ConfiguredDriver> = None;
        let mut library_dest = String::new();

        if config_path.exists() {
             self.log(ctx, LogLevel::Info, format!("Found ox_module.yaml at {}", config_path.display()));
             if let Ok(content) = fs::read_to_string(&config_path) {
                if let Ok(module_yaml) = serde_yaml::from_str::<serde_json::Value>(&content) {
                     if let Some(driver_code) = module_yaml.get("driver") {
                         let id = driver_code.get("id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                         let lib = driver_code.get("library").and_then(|v| v.as_str()).unwrap_or("").to_string();
                         
                         let lib_src = path.join(&lib);
                         let lib_dest = Path::new(&self.config.driver_root).join(&lib);
                         
                         if let Some(parent) = lib_dest.parent() {
                             let _ = fs::create_dir_all(parent);
                         }

                         self.log(ctx, LogLevel::Info, format!("Copying library {} -> {}", lib_src.display(), lib_dest.display()));
                         if let Err(e) = fs::copy(&lib_src, &lib_dest) {
                             self.update_status(ctx, package_name, "failed", 0, &format!("Failed to copy library: {}", e));
                             return self.installer_response(ctx, "error", &format!("Failed to copy library: {}", e));
                         }
                         library_dest = lib_dest.to_string_lossy().to_string();

                         let mut cfg = ConfiguredDriver {
                             id: id.clone(),
                              friendly_name: None,
                             name: driver_code.get("config").and_then(|c| c.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
                             library_path: "".to_string(), 
                             state: driver_code.get("config").and_then(|c| c.get("state")).and_then(|v| v.as_str()).unwrap_or("enabled").to_string(),
                         };
                         driver_config = Some(cfg);
                         self.update_status(ctx, package_name, "installing", 50, "Library copied and config parsed from ox_module.yaml");
                     }
                }
             }
        } else {
            self.update_status(ctx, package_name, "failed", 0, "ox_module.yaml not found in package");
            return self.installer_response(ctx, "error", "ox_module.yaml not found in package");
        }

        if let Some(mut new_driver) = driver_config {
             if !library_dest.is_empty() {
                 new_driver.library_path = Path::new(&library_dest).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
             }

             if let Err(e) = self.upsert_configured_driver(new_driver) {
                  self.update_status(ctx, package_name, "failed", 0, &format!("Failed to save drivers config: {}", e));
                  return self.installer_response(ctx, "error", &format!("Failed to save drivers config: {}", e));
             }
             
             self.update_status(ctx, package_name, "success", 100, "Installation complete");
             return self.installer_response(ctx, "success", "Driver installed successfully");
        }

        self.update_status(ctx, package_name, "failed", 0, "No valid driver configuration found");
        self.installer_response(ctx, "error", "No valid driver configuration found")
    }

    pub fn process_request(&self, ctx: &PipelineContext) -> HandlerResult {
        // We only expect to be called for installation
        let action = ctx.get("installer.action").and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default();
        if action == "install" {
            return self.handle_install(ctx);
        } else if action == "status" {
            return self.handle_status(ctx);
        }
        
        // Also support direct paths if needed, e.g. for status polling
        let path = ctx.get("request.path").and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default();
        if path.ends_with("/packages/installer/status") {
             return self.handle_status(ctx);
        }

        HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        }
    }

}

#[no_mangle]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    module_id_ptr: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = &*api_ptr;

    let module_params_json = if !module_params_json_ptr.is_null() {
        CStr::from_ptr(module_params_json_ptr).to_str().unwrap_or("{}")
    } else { "{}" };

    let mut config: InstallerConfig = serde_json::from_str(module_params_json).unwrap_or(InstallerConfig::default());
    
    let module_id = if !module_id_ptr.is_null() {
        CStr::from_ptr(module_id_ptr).to_string_lossy().to_string()
    } else {
        MODULE_NAME.to_string()
    };

    let module = Box::new(DriverInstaller::new(api, config, module_id));
    let instance_ptr = Box::into_raw(module) as *mut c_void;

    Box::into_raw(Box::new(ModuleInterface {
        instance_ptr,
        handler_fn: process_request_c_wrapper,
        log_callback: api.log_callback,
        get_config: get_config_c,
    }))
}

#[no_mangle]
pub unsafe extern "C" fn process_request_c_wrapper(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    _log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena_ptr: *const c_void,
) -> HandlerResult {
    if instance_ptr.is_null() {
        return HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        };
    }

    let module = &*(instance_ptr as *mut DriverInstaller);
    let pipeline_state = &mut *pipeline_state_ptr;
    let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;
    let ctx = PipelineContext::new(
        module.api, 
        pipeline_state_ptr as *mut c_void, 
        arena_ptr
    );

    module.process_request(&ctx)
}

#[no_mangle]
pub unsafe extern "C" fn get_config_c(
    instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    if instance_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let module = &*(instance_ptr as *mut DriverInstaller);
    let json = serde_json::to_string(&module.config).unwrap_or("{}".to_string());
    let json_cstring = CString::new(json).unwrap_or_else(|_| CString::new("{}").unwrap());
    alloc_fn(arena, json_cstring.as_ptr())
}
