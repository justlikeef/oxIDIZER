use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::io::Cursor;
use std::collections::HashMap;
use std::sync::RwLock;
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_ERROR,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yaml;
use multipart::server::Multipart;
use prost::Message;

const MODULE_NAME: &str = "ox_package_manager";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(default = "Config::default_staging_directory")]
    staging_directory: String,
    #[serde(default = "Config::default_allowed_extensions")]
    allowed_extensions: Vec<String>,
    #[serde(default = "Config::default_installers_config")]
    installers_config: String,
    #[serde(default = "Config::default_max_dependency_depth")]
    max_dependency_depth: u32,
    #[serde(default = "Config::default_manifests_directory")]
    manifests_directory: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            staging_directory: Self::default_staging_directory(),
            allowed_extensions: Self::default_allowed_extensions(),
            installers_config: Self::default_installers_config(),
            max_dependency_depth: Self::default_max_dependency_depth(),
            manifests_directory: Self::default_manifests_directory(),
        }
    }
}

impl Config {
    fn default_staging_directory() -> String {
        "/tmp/ox_staging".to_string()
    }

    fn default_allowed_extensions() -> Vec<String> {
        vec![
            ".tar.gz".to_string(),
            ".tgz".to_string(),
            ".zip".to_string(),
            ".tar.bz2".to_string(),
            ".tbz2".to_string(),
        ]
    }

    fn default_installers_config() -> String {
        "/var/repos/oxIDIZER/ox_package_manager/conf/installers.yaml".to_string()
    }

    fn default_manifests_directory() -> String {
        "/var/repos/oxIDIZER/ox_package_manager/staged/installed".to_string()
    }

    fn default_max_dependency_depth() -> u32 {
        10
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResourceRecord {
    #[serde(alias = "type")]
    pub resource_type: String, // e.g. "module_config", "config_file", "content"
    pub filename: String,
}

/// Prost-compatible representation of PackageMetadata for binary task-state encoding.
/// Fields that are not directly prost-compatible (Vec<ResourceRecord>, HashMap) are
/// stored as their JSON-encoded strings.
#[derive(Clone, PartialEq, prost::Message)]
pub struct PackageMetadataProto {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub version: String,
    #[prost(string, tag = "3")]
    pub description: String,
    #[prost(string, tag = "4")]
    pub package_type: String,
    /// JSON-encoded Vec<ResourceRecord>
    #[prost(string, tag = "5")]
    pub resources_json: String,
    #[prost(string, tag = "6")]
    pub filename: String,
    #[prost(uint64, tag = "7")]
    pub size: u64,
    #[prost(string, repeated, tag = "8")]
    pub dependencies: Vec<String>,
    /// JSON-encoded HashMap<String, String>
    #[prost(string, tag = "9")]
    pub installer_handlers_json: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PackageMetadata {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "PackageMetadata::default_package_type")]
    pub package_type: String, // e.g. "module", "persistence_drivers"
    #[serde(default)]
    pub resources: Vec<ResourceRecord>,
    // Fields added for the list API
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub installer_handlers: HashMap<String, String>, // "package_type" -> "module_id" (usually self, or specific ID)
}

impl PackageMetadata {
    fn default_package_type() -> String {
        "module".to_string()
    }
}



pub struct OxModule {
    api: CoreHostApi,
    config: Config,
    module_id: String,
    installers: RwLock<HashMap<String, String>>,
}

impl OxModule {
    pub fn new(api: CoreHostApi, config: Config, module_id: String) -> Self {
        // Ensure staging directory exists
        if let Err(_e) = std::fs::create_dir_all(&config.staging_directory) {}
        if let Err(_e) = std::fs::create_dir_all(&config.manifests_directory) {}

        let installers = Self::load_installers_from_config(&config.installers_config);

        Self {
            api,
            config,
            module_id,
            installers: RwLock::new(installers),
        }
    }

    fn log(&self, level: u8, message: String) {
        if let Ok(c_msg) = CString::new(message) {
            (self.api.log)(std::ptr::null_mut(), level, c_msg.as_ptr());
        }
    }

    fn error_response(&self, api: &CoreHostApi, task_ctx: *mut c_void, status: u16, message: &str) -> FlowControl {
        let response_json = serde_json::json!({
            "result": "error",
            "message": message
        });
        // Return the actual error status code.
        set_field(api, task_ctx, "response.status", &serde_json::json!(status).to_string());
        set_field(api, task_ctx, "response.body", &response_json.to_string());
        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    }

    fn load_installers_from_config(path_str: &str) -> HashMap<String, String> {
        let path = Path::new(path_str);
        if !path.exists() {
            // Ensure directory exists
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Return default for fresh install
            let mut default_map = HashMap::new();
            default_map.insert("module".to_string(), "package_manager".to_string());
            return default_map;
        }

        match ox_fileproc::process_file(path, 10) {
            Ok(value) => {
                serde_json::from_value::<HashMap<String, String>>(value).unwrap_or_else(|_| {
                    let mut m = HashMap::new();
                    m.insert("module".to_string(), "package_manager".to_string());
                    m
                })
            }
            Err(_) => {
                let mut m = HashMap::new();
                m.insert("module".to_string(), "package_manager".to_string());
                m
            }
        }
    }

    fn save_installers(&self) -> Result<(), String> {
        let path = Path::new(&self.config.installers_config);
        let installers = self.installers.read().map_err(|e| e.to_string())?;
        let yaml = serde_yaml::to_string(&*installers).map_err(|e| e.to_string())?;
        std::fs::write(path, yaml).map_err(|e| e.to_string())
    }

    fn handle_register(&self, api: &CoreHostApi, task_ctx: *mut c_void) -> FlowControl {
        self.log(ox_workflow_abi::OX_LOG_INFO, "DEBUG: handle_register called".to_string());
        let mut body_str = String::new();
        if let Some(v) = get_field_val(api, task_ctx, "request.payload") {
            if let Some(s) = v.as_str() { body_str = s.to_string(); }
        }
        if body_str.is_empty() {
             if let Some(v) = get_field_val(api, task_ctx, "request.body_path") {
                  if let Some(path) = v.as_str() {
                       if let Ok(s) = std::fs::read_to_string(path) {
                            body_str = s;
                       }
                  }
             }
        }

        self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: Registration payload: {}", body_str));

        let json: Value = serde_json::from_str(&body_str).unwrap_or(Value::Null);
        let pkg_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let module_id = json.get("module_id").and_then(|v| v.as_str()).unwrap_or("");

        if pkg_type.is_empty() || module_id.is_empty() {
             self.log(ox_workflow_abi::OX_LOG_ERROR, format!("DEBUG: Missing required fields: type='{}', module_id='{}'", pkg_type, module_id));
             let response_json = serde_json::json!({ "result": "error", "message": "type and module_id are required" });
             set_field(api, task_ctx, "response.status", &400.to_string());
             set_field(api, task_ctx, "response.body", &response_json.to_string());
             return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }

        {
            let mut installers = match self.installers.write() {
                Ok(lock) => lock,
                Err(_) => return self.error_response(api, task_ctx, 500, "Internal error: installers lock poisoned"),
            };
            installers.insert(pkg_type.to_string(), module_id.to_string());
        }

        if let Err(e) = self.save_installers() {
             let response_json = serde_json::json!({ "result": "error", "message": format!("Failed to save installers: {}", e) });
             set_field(api, task_ctx, "response.status", &500.to_string());
             set_field(api, task_ctx, "response.body", &response_json.to_string());
             return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }

        let response_json = serde_json::json!({ "result": "success", "message": format!("Registered {} to handle {}", module_id, pkg_type) });
        set_field(api, task_ctx, "response.status", &200.to_string());
        set_field(api, task_ctx, "response.body", &response_json.to_string());
        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    }

    fn handle_deregister(&self, api: &CoreHostApi, task_ctx: *mut c_void) -> FlowControl {
        let mut body_str = String::new();
        if let Some(v) = get_field_val(api, task_ctx, "request.payload") {
            if let Some(s) = v.as_str() { body_str = s.to_string(); }
        }
        if body_str.is_empty() {
             if let Some(v) = get_field_val(api, task_ctx, "request.body_path") {
                  if let Some(path) = v.as_str() {
                       if let Ok(s) = std::fs::read_to_string(path) {
                            body_str = s;
                       }
                  }
             }
        }

        let json: Value = serde_json::from_str(&body_str).unwrap_or(Value::Null);
        let pkg_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if pkg_type.is_empty() {
             let response_json = serde_json::json!({ "result": "error", "message": "type is required" });
             set_field(api, task_ctx, "response.status", &400.to_string());
             set_field(api, task_ctx, "response.body", &response_json.to_string());
             return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }

        {
            let mut installers = match self.installers.write() {
                Ok(lock) => lock,
                Err(_) => return self.error_response(api, task_ctx, 500, "Internal error: installers lock poisoned"),
            };
            installers.remove(pkg_type);
        }

        if let Err(e) = self.save_installers() {
             let response_json = serde_json::json!({ "result": "error", "message": format!("Failed to save installers: {}", e) });
             set_field(api, task_ctx, "response.status", &500.to_string());
             set_field(api, task_ctx, "response.body", &response_json.to_string());
             return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }

        let response_json = serde_json::json!({ "result": "success", "message": format!("Deregistered handler for {}", pkg_type) });
        set_field(api, task_ctx, "response.status", &200.to_string());
        set_field(api, task_ctx, "response.body", &response_json.to_string());
        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    }


    fn handle_status(&self, api: &CoreHostApi, task_ctx: *mut c_void) -> FlowControl {
        let package_name = get_field_val(api, task_ctx, "package")
            .or(get_field_val(api, task_ctx, "installer.package_name"))
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();

        let response_json = if package_name.is_empty() {
             serde_json::json!({ "result": "success", "message": "Package Manager is alive. Specify 'package' for specific status." })
        } else {
             // For now, PM aggregation is minimal. 
             // We return if it's in installed/ database or not.
             let staging_path = PathBuf::from(&self.config.staging_directory);
             let manifest_path = staging_path.join("installed").join(format!("{}.json", package_name));
             if manifest_path.exists() {
                 serde_json::json!({ "result": "success", "status": "installed" })
             } else {
                 serde_json::json!({ "result": "success", "status": "unknown or in-progress" })
             }
        };

        set_field(api, task_ctx, "response.status", &200.to_string());
        set_field(api, task_ctx, "response.body", &response_json.to_string());
        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    }

    pub fn process_request(&self, task_ctx: *mut c_void) -> FlowControl {
        let api = &self.api;

        let verb = { let v = get_field(api, task_ctx, "request.verb"); if v.is_empty() { get_field(api, task_ctx, "request.method") } else { v } }.to_lowercase();
        let resource = { let r = get_field(api, task_ctx, "request.resource"); if r.is_empty() { get_field(api, task_ctx, "request.path") } else { r } };
        let installer_action = get_field(api, task_ctx, "installer.action");
        
        if resource.contains("installer") || !installer_action.is_empty() { self.log(ox_workflow_abi::OX_LOG_INFO, format!("pm: resource={}, action={}", resource, installer_action)); }

        if resource.contains("/packages/installer/status") {
            return self.handle_status(api, task_ctx);
        }

        // Handle delegated installer actions
        if let Some(action_val) = get_field_val(api, task_ctx, "installer.action") {
            if let Some(action) = action_val.as_str() {
                 if action == "subscribe" || action == "register" {
                      return self.handle_register(api, task_ctx);
                 }
                 if action == "unsubscribe" || action == "deregister" {
                      return self.handle_deregister(api, task_ctx);
                 }
                 if action == "install" {
                      return self.handle_install(api, task_ctx);
                 }
                 if action == "uninstall" {
                      return self.handle_uninstall(api, task_ctx);
                 }
            }
        }

        self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG_REQUEST_GENERIC: verb='{}', resource='{}'", verb, resource));

        if (verb == "create" || verb == "post") && resource.contains("upload") {
            return self.handle_upload(api, task_ctx);
        }

        if verb == "get" {
            if resource.ends_with("/packages/list/") || resource.ends_with("/packages/list") {
                return self.handle_list_staged(api, task_ctx);
            }
            if resource.ends_with("/packages/list/installed") {
                return self.handle_list_installed(api, task_ctx);
            }
            if resource.ends_with("/packages/installed/package") {
                return self.handle_get_installed_package(api, task_ctx);
            }
        }
        if (verb == "create" || verb == "post") && (resource.ends_with("/packages/upload/") || resource.ends_with("/packages/upload")) {
             return self.handle_upload(api, task_ctx);
        }
        if (verb == "get" || verb == "create" || verb == "post") && (resource.ends_with("/packages/install/") || resource.ends_with("/packages/install")) {
            return self.handle_install(api, task_ctx);
        }
        if (verb == "get" || verb == "create" || verb == "post") && (resource.ends_with("/packages/uninstall/") || resource.ends_with("/packages/uninstall")) {
            return self.handle_uninstall(api, task_ctx);
        }

        if (verb == "create" || verb == "post") && resource.contains("installer/subscribe") {
            return self.handle_register(api, task_ctx);
        }

        if (verb == "create" || verb == "post") && resource.contains("installer/unsubscribe") {
            return self.handle_deregister(api, task_ctx);
        }

        // Default: Ignore
        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    }

    fn handle_install(&self, api: &CoreHostApi, task_ctx: *mut c_void) -> FlowControl {
        let mut body_str = String::new();
        if let Some(v) = get_field_val(api, task_ctx, "request.payload") {
            if let Some(s) = v.as_str() {
                body_str = s.to_string();
            }
        }
        
        if body_str.is_empty() {
            body_str = get_field(api, task_ctx, "request.body");
        }
        if body_str.is_empty() {
             if let Some(v) = get_field_val(api, task_ctx, "request.body_path") {
                  if let Some(path) = v.as_str() {
                       if let Ok(s) = std::fs::read_to_string(path) {
                            body_str = s;
                       }
                  }
             }
        }

        self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: handle_install body_str: {}", body_str));
            
        let json: Value = serde_json::from_str(&body_str).unwrap_or(Value::Null);
        let mut filename = json.get("filename").or(json.get("package")).and_then(|v| v.as_str()).unwrap_or("").to_string();

        if filename.is_empty() {
             // Fallback to form-encoded parse if it looks like one
             if body_str.contains("package=") {
                  for part in body_str.split('&') {
                       if part.starts_with("package=") {
                            filename = part.replace("package=", "");
                            break;
                       }
                  }
             } else if body_str.contains("filename=") {
                  for part in body_str.split('&') {
                       if part.starts_with("filename=") {
                            filename = part.replace("filename=", "");
                            break;
                       }
                  }
             }
        }

        self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: handle_install filename: {}", filename));

        if filename.is_empty() {
             let response_json = serde_json::json!({ "result": "error", "message": "Filename or package is required" });
             set_field(api, task_ctx, "response.status", &400.to_string());
             set_field(api, task_ctx, "response.body", &response_json.to_string());
             return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }

        let staging_path = PathBuf::from(&self.config.staging_directory);
        let source_path = staging_path.join(&filename);

        self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: handle_install source_path: {:?}", source_path));
        if !source_path.exists() {
             self.log(ox_workflow_abi::OX_LOG_ERROR, format!("DEBUG: handle_install source_path DOES NOT EXIST: {:?}", source_path));
             let response_json = serde_json::json!({ "result": "error", "message": format!("Package file not found: {:?}", source_path) });
             set_field(api, task_ctx, "response.status", &404.to_string());
             set_field(api, task_ctx, "response.body", &response_json.to_string());
             return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }

        // 1. Extract metadata
        self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: handle_install calling extract_metadata_from_archive"));
        let metadata = match self.extract_metadata_from_archive(&source_path) {
            Ok(m) => m,
            Err(e) => {
                let response_json = serde_json::json!({ "result": "error", "message": format!("Failed to read metadata: {}", e) });
                set_field(api, task_ctx, "response.status", &500.to_string());
                set_field(api, task_ctx, "response.body", &response_json.to_string());
                return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
            }
        };

        // 2. Resolve dependencies recursively
        self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: handle_install metadata extracted: {}. Dependencies: {:?}", metadata.name, metadata.dependencies));
        let mut visited = vec![metadata.name.clone()];
        if let Err(e) = self.resolve_dependencies(api, task_ctx, &metadata, 0, &mut visited) {
            let response_json = serde_json::json!({ "result": "error", "message": format!("Dependency resolution failed: {}", e) });
            set_field(api, task_ctx, "response.status", &409.to_string());
            set_field(api, task_ctx, "response.body", &response_json.to_string());
            return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }

        // 3. Perform primary installation
        self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: handle_install dependencies resolved. Calling perform_install for {}", filename));
        if let Err(e) = self.perform_install(api, task_ctx, &filename, &metadata) {
            let response_json = serde_json::json!({ "result": "error", "message": format!("Installation failed: {}", e) });
            set_field(api, task_ctx, "response.status", &500.to_string());
            set_field(api, task_ctx, "response.body", &response_json.to_string());
            return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }

        let response_json = serde_json::json!({ "result": "success", "message": "Package and dependencies installed successfully" });
        set_field(api, task_ctx, "response.status", &200.to_string());
        set_field(api, task_ctx, "response.body", &response_json.to_string());
        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    }

    // ... (helper methods)

    fn extract_metadata_from_archive(&self, file_path: &PathBuf) -> Result<PackageMetadata, String> {
        let filename = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        let file = std::fs::File::open(file_path).map_err(|e| format!("Failed to open package: {}", e))?;

        let fs_meta = std::fs::metadata(file_path).map_err(|e| format!("Failed to get file metadata: {}", e))?;
        let mut metadata = PackageMetadata {
            name: "unknown".to_string(),
            version: "0.0.0".to_string(),
            description: "".to_string(),
            package_type: "module".to_string(),
            resources: Vec::new(),
            dependencies: Vec::new(),
            installer_handlers: HashMap::new(),
            filename: filename.clone(),
            size: fs_meta.len(),
        };

        // Simplified approach: scan archive for manifest files and parse them.
        let mut manifest_content: Option<Vec<u8>> = None;
        let mut is_yaml = false;

        if filename.ends_with(".zip") {
             let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Failed to read zip: {}", e))?;
             
             // Try JSON
             {
                 if let Ok(mut json_file) = archive.by_name("ox_package.json") {
                     let mut buf = Vec::new();
                     let _ = std::io::Read::read_to_end(&mut json_file, &mut buf);
                     manifest_content = Some(buf);
                 }
             }

             // Try YAML if JSON missing
             if manifest_content.is_none() {
                 if let Ok(mut yaml_file) = archive.by_name("ox_package.yaml") {
                     let mut buf = Vec::new();
                     let _ = std::io::Read::read_to_end(&mut yaml_file, &mut buf);
                     manifest_content = Some(buf);
                     is_yaml = true;
                 }
             }
        } else if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
            let tar = flate2::read::GzDecoder::new(file);
            let mut archive = tar::Archive::new(tar);
            for entry in archive.entries().map_err(|e| format!("Tar error: {}", e))? {
                let mut entry = entry.map_err(|e| format!("Entry error: {}", e))?;
                let path = match entry.path() {
                    Ok(p) => p.to_string_lossy().to_string(),
                    Err(_) => continue,
                };
                // Normalize path
                let normalized = if path.starts_with("./") { &path[2..] } else { &path };
                
                if normalized == "ox_package.json" {
                     let mut buf = Vec::new();
                     let _ = std::io::Read::read_to_end(&mut entry, &mut buf);
                     manifest_content = Some(buf);
                     break;
                } else if normalized == "ox_package.yaml" {
                     let mut buf = Vec::new();
                     let _ = std::io::Read::read_to_end(&mut entry, &mut buf);
                     manifest_content = Some(buf);
                     is_yaml = true;
                     break;
                }
            }
        } else if filename.ends_with(".tar.bz2") || filename.ends_with(".tbz2") {
            let tar = bzip2::read::BzDecoder::new(file);
            let mut archive = tar::Archive::new(tar);
            for entry in archive.entries().map_err(|e| format!("Tar error: {}", e))? {
                let mut entry = entry.map_err(|e| format!("Entry error: {}", e))?;
                let path = entry.path().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "unknown".to_string());
                let normalized = if path.starts_with("./") { &path[2..] } else { &path };

                if normalized == "ox_package.json" {
                     let mut buf = Vec::new();
                     let _ = std::io::Read::read_to_end(&mut entry, &mut buf);
                     manifest_content = Some(buf);
                     break;
                } else if normalized == "ox_package.yaml" {
                     let mut buf = Vec::new();
                     let _ = std::io::Read::read_to_end(&mut entry, &mut buf);
                     manifest_content = Some(buf);
                     is_yaml = true;
                     break;
                }
            }
        }

        if let Some(content) = manifest_content {
             if is_yaml {
                 if let Ok(yaml) = serde_yaml::from_slice::<Value>(&content) {
                     if let Some(n) = yaml.get("name").and_then(|v| v.as_str()) { metadata.name = n.to_string(); }
                     if let Some(v) = yaml.get("version").and_then(|v| v.as_str()) { metadata.version = v.to_string(); }
                     if let Some(d) = yaml.get("description").and_then(|v| v.as_str()) { metadata.description = d.to_string(); }
                     if let Some(t) = yaml.get("package_type").and_then(|v| v.as_str()) { metadata.package_type = t.to_string(); }
                     if let Some(r) = yaml.get("resources").and_then(|v| v.as_array()) {
                         metadata.resources = r.iter().filter_map(|rv| {
                             serde_json::from_value::<ResourceRecord>(rv.clone()).ok()
                         }).collect();
                     }
                     if let Some(deps) = yaml.get("dependencies").and_then(|v| v.as_array()) {
                         metadata.dependencies = deps.iter().filter_map(|dv| dv.as_str().map(|s| s.to_string())).collect();
                     }
                     if let Some(handlers) = yaml.get("installer_handlers").and_then(|v| v.as_object()) {
                         for (k, v) in handlers {
                             if let Some(mod_id) = v.as_str() {
                                 metadata.installer_handlers.insert(k.clone(), mod_id.to_string());
                             }
                         }
                     }
                 }
             } else {
                 if let Ok(json) = serde_json::from_slice::<Value>(&content) {
                     if let Some(n) = json.get("name").and_then(|v| v.as_str()) { metadata.name = n.to_string(); }
                     if let Some(v) = json.get("version").and_then(|v| v.as_str()) { metadata.version = v.to_string(); }
                     if let Some(d) = json.get("description").and_then(|v| v.as_str()) { metadata.description = d.to_string(); }
                     if let Some(t) = json.get("package_type").and_then(|v| v.as_str()) { metadata.package_type = t.to_string(); }
                     if let Some(r) = json.get("resources").and_then(|v| v.as_array()) {
                         metadata.resources = r.iter().filter_map(|rv| {
                             serde_json::from_value::<ResourceRecord>(rv.clone()).ok()
                         }).collect();
                     }
                     if let Some(deps) = json.get("dependencies").and_then(|v| v.as_array()) {
                         metadata.dependencies = deps.iter().filter_map(|dv| dv.as_str().map(|s| s.to_string())).collect();
                     }
                     if let Some(handlers) = json.get("installer_handlers").and_then(|v| v.as_object()) {
                         for (k, v) in handlers {
                             if let Some(mod_id) = v.as_str() {
                                 metadata.installer_handlers.insert(k.clone(), mod_id.to_string());
                             }
                         }
                     }
                 }
             }
             Ok(metadata)
        } else {
             Err("Manifest not found".to_string())
        }
    }

    fn verify_package_manifest(&self, file_path: &PathBuf) -> Result<(), String> {
        let filename = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        let file = std::fs::File::open(file_path).map_err(|e| format!("Failed to open package for verification: {}", e))?;

        let has_manifest = |name: &str| name == "ox_package.json" || name == "ox_package.yaml";

        if filename.ends_with(".zip") {
             let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Failed to read zip directory: {}", e))?;
             let has_json = archive.by_name("ox_package.json").is_ok();
             let has_yaml = archive.by_name("ox_package.yaml").is_ok();
             if !has_json && !has_yaml {
                 return Err("'ox_package.json' or 'ox_package.yaml' not found in zip archive".to_string());
             }
        } else if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
            let tar = flate2::read::GzDecoder::new(file);
            let mut archive = tar::Archive::new(tar);
            let mut found = false;
            for entry in archive.entries().map_err(|e| format!("Failed to read tar entries: {}", e))? {
                let entry = entry.map_err(|e| format!("Bad entry: {}", e))?;
                if let Ok(path) = entry.path() {
                    let path_str = path.to_string_lossy();
                    
                    // Normalize path: strip leading "./" if present
                    let normalized = if path_str.starts_with("./") {
                        &path_str[2..]
                    } else {
                        &path_str
                    };
                    
                    if has_manifest(normalized) {
                        found = true;
                        break;
                    }
                }
            }
            if !found { 
                return Err("Manifest not found in tar.gz archive".to_string()); 
            }
        } else if filename.ends_with(".tar.bz2") || filename.ends_with(".tbz2") {
            let tar = bzip2::read::BzDecoder::new(file);
            let mut archive = tar::Archive::new(tar);
            let mut found = false;
             for entry in archive.entries().map_err(|e| format!("Failed to read tar entries: {}", e))? {
                let entry = entry.map_err(|e| format!("Bad entry: {}", e))?;
                if let Ok(path) = entry.path() {
                    let path_str = path.to_string_lossy();
                    // Normalize path: strip leading "./" if present
                    let normalized = if path_str.starts_with("./") {
                        &path_str[2..]
                    } else {
                        &path_str
                    };
                    
                     if has_manifest(normalized) {
                        found = true;
                        break;
                    }
                }
            }
            if !found { return Err("Manifest not found in tar.bz2 archive".to_string()); }
        } else {
             return Err("Unsupported format for verification".to_string());
        }
        Ok(())
    }

    fn extract_package(&self, file_path: &PathBuf, target_dir: &PathBuf) -> Result<(), String> {
        let filename = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        let file = std::fs::File::open(file_path).map_err(|e| format!("Failed to open package: {}", e))?;

        if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
            let tar = flate2::read::GzDecoder::new(file);
            let mut archive = tar::Archive::new(tar);
            archive.unpack(target_dir).map_err(|e| format!("Failed to unpack tar.gz: {}", e))?;
        } else if filename.ends_with(".tar.bz2") || filename.ends_with(".tbz2") {
            let tar = bzip2::read::BzDecoder::new(file);
            let mut archive = tar::Archive::new(tar);
            archive.unpack(target_dir).map_err(|e| format!("Failed to unpack tar.bz2: {}", e))?;
        } else if filename.ends_with(".zip") {
             let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Failed to open zip: {}", e))?;
             archive.extract(target_dir).map_err(|e| format!("Failed to unpack zip: {}", e))?;
        } else {
            return Err("Unsupported file format".to_string());
        }
        Ok(())
    }

    fn on_file_upload_completion(&self, file_path: PathBuf, filename: String) -> Result<(String, bool), String> {
         // Validation 1: Extension Check
         let valid_ext = self.config.allowed_extensions.iter().any(|ext| filename.to_lowercase().ends_with(ext));
         if !valid_ext {
             let _ = std::fs::remove_file(&file_path);
             return Err(format!("Invalid file extension. Allowed: {:?}", self.config.allowed_extensions));
         }

         // Validation 2: Verify Manifest (Pre-Check)
         if let Err(e) = self.verify_package_manifest(&file_path) {
             let _ = std::fs::remove_file(&file_path);
             return Err(format!("Invalid Package Manifest: {}", e));
         }

         // Extraction setup
         let stem = std::path::Path::new(&filename)
            .file_stem().and_then(|s| s.to_str()).unwrap_or("package");
         let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
         let staging_folder_name = format!("{}_{}", stem, timestamp);
         let staging_path = PathBuf::from(&self.config.staging_directory).join("extracted").join(&staging_folder_name);

         if let Err(e) = std::fs::create_dir_all(&staging_path) {
             let _ = std::fs::remove_file(&file_path);
             return Err(format!("Failed to create staging directory: {}", e));
         }

         // Extract
         if let Err(e) = self.extract_package(&file_path, &staging_path) {
             let _ = std::fs::remove_dir_all(&staging_path);
             let _ = std::fs::remove_file(&file_path);
             return Err(format!("Extraction failed: {}", e));
         }

         // 4. Extract Metadata
         let json_path = staging_path.join("ox_package.json");
         let yaml_path = staging_path.join("ox_package.yaml");
         
         let mut metadata = PackageMetadata {
             name: "unknown".to_string(),
             version: "0.0.0".to_string(),
             description: "".to_string(),
             package_type: PackageMetadata::default_package_type(),
             resources: Vec::new(),
             dependencies: Vec::new(),
             installer_handlers: HashMap::new(),
             filename: filename.clone(),
             size: std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0),
         };

         if let Ok(file) = std::fs::File::open(&json_path) {
             if let Ok(json) = serde_json::from_reader::<_, Value>(file) {
                 if let Some(n) = json.get("name").and_then(|v| v.as_str()) { metadata.name = n.to_string(); }
                 if let Some(v) = json.get("version").and_then(|v| v.as_str()) { metadata.version = v.to_string(); }
                 if let Some(d) = json.get("description").and_then(|v| v.as_str()) { metadata.description = d.to_string(); }
             }
         } else if let Ok(file) = std::fs::File::open(&yaml_path) {
             if let Ok(yaml) = serde_yaml::from_reader::<_, Value>(file) {
                 if let Some(n) = yaml.get("name").and_then(|v| v.as_str()) { metadata.name = n.to_string(); }
                 if let Some(v) = yaml.get("version").and_then(|v| v.as_str()) { metadata.version = v.to_string(); }
                 if let Some(d) = yaml.get("description").and_then(|v| v.as_str()) { metadata.description = d.to_string(); }
             }
         }

         // Cleanup: Remove expanded files. Retain uploaded archive.
         let _ = std::fs::remove_dir_all(&staging_path);
         
         Ok((staging_path.to_string_lossy().to_string(), true))
    }

    fn handle_list_staged(&self, api: &CoreHostApi, task_ctx: *mut c_void) -> FlowControl {
        let mut packages = Vec::new();
        let staging_dir = PathBuf::from(&self.config.staging_directory);

        if let Ok(entries) = std::fs::read_dir(&staging_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
                
                // Check against allowed extensions
                let is_package = self.config.allowed_extensions.iter().any(|ext| filename.to_lowercase().ends_with(ext));
                
                if is_package {
                    // Extract metadata on demand
                    if let Ok(mut meta) = self.extract_metadata_from_archive(&path) {
                        meta.filename = filename.clone();
                        meta.size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        packages.push(meta);
                    }
                }
            }
        }

        // Encode staged package list as protobuf and store in binary task-state field.
        let proto_packages: Vec<PackageMetadataProto> = packages.iter().map(|m| PackageMetadataProto {
            name: m.name.clone(),
            version: m.version.clone(),
            description: m.description.clone(),
            package_type: m.package_type.clone(),
            resources_json: serde_json::to_string(&m.resources).unwrap_or_default(),
            filename: m.filename.clone(),
            size: m.size,
            dependencies: m.dependencies.clone(),
            installer_handlers_json: serde_json::to_string(&m.installer_handlers).unwrap_or_default(),
        }).collect();
        // Encode each proto entry and concatenate length-delimited
        let mut all_proto_bytes: Vec<u8> = Vec::new();
        for p in &proto_packages {
            let mut buf = Vec::new();
            if p.encode(&mut buf).is_ok() {
                let len = buf.len() as u32;
                all_proto_bytes.extend_from_slice(&len.to_le_bytes());
                all_proto_bytes.extend_from_slice(&buf);
            }
        }
        if !all_proto_bytes.is_empty() {
            set_field_bytes_data(api, task_ctx, "data.staged_packages_proto", &all_proto_bytes);
        }

        let response_json = serde_json::json!({
            "result": "success",
            "packages": packages
        });

        set_field(api, task_ctx, "response.status", &200.to_string());
        set_field(api, task_ctx, "response.type", &"application/json");
        set_field(api, task_ctx, "response.body", &response_json.to_string());

        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    }

    fn handle_list_installed(&self, api: &CoreHostApi, task_ctx: *mut c_void) -> FlowControl {
        let mut packages = Vec::new();
        let manifests_dir = PathBuf::from(&self.config.manifests_directory);

        // Filter by type if provided
        let type_filter = get_field_val(api, task_ctx, "request.query.type")
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        if let Ok(entries) = std::fs::read_dir(&manifests_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(file) = std::fs::File::open(&path) {
                        if let Ok(meta) = serde_json::from_reader::<_, PackageMetadata>(file) {
                             if let Some(ref t) = type_filter {
                                 if &meta.package_type != t {
                                     continue;
                                 }
                             }
                             packages.push(meta);
                        }
                    }
                }
            }
        }

        let response_json = serde_json::json!({
            "result": "success",
            "packages": packages
        });

        set_field(api, task_ctx, "response.status", &200.to_string());
        set_field(api, task_ctx, "response.type", &"application/json");
        set_field(api, task_ctx, "response.body", &response_json.to_string());

        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    }

    fn handle_get_installed_package(&self, api: &CoreHostApi, task_ctx: *mut c_void) -> FlowControl {
        let name = get_field_val(api, task_ctx, "request.query.name")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        
        if name.is_empty() {
             return self.error_response(api, task_ctx, 400, "Missing 'name' query parameter");
        }

        let manifest_path = PathBuf::from(&self.config.manifests_directory).join(format!("{}.json", name));
        if !manifest_path.exists() {
             return self.error_response(api, task_ctx, 404, &format!("Package '{}' not found", name));
        }

        if let Ok(content) = std::fs::read_to_string(&manifest_path) {
             set_field(api, task_ctx, "response.status", &200.to_string());
             set_field(api, task_ctx, "response.type", &"application/json");
             set_field(api, task_ctx, "response.body", &serde_json::Value::String(content).to_string());
             
             FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
        } else {
             self.error_response(api, task_ctx, 500, "Failed to read package manifest")
        }
    }

    fn collect_packages_to_uninstall(&self, target_pkg: String, to_remove: &mut Vec<String>) -> Result<(), String> {
         if to_remove.contains(&target_pkg) {
             return Ok(());
         }
         
         // 1. Get info about target package to know what types it handles (if any)
         // Actually, we need to scan ALL installed packages to see if their package_type is handled by the target package.
         
         // Add target first to prevent cycles
         to_remove.push(target_pkg.clone());

         let manifests_dir = PathBuf::from(&self.config.manifests_directory);
         let mut child_packages = Vec::new(); // Packages handled by this one

         // Re-read target metadata to see if it claims any handlers
         // Note: Metadata tells us what IT handles.
         let target_manifest_path = manifests_dir.join(format!("{}.json", target_pkg));
         if !target_manifest_path.exists() {
              return Ok(()); // Already gone?
         }
         
         // We need to know what types this package handles.
         let target_meta: PackageMetadata = match serde_json::from_reader(std::fs::File::open(&target_manifest_path).map_err(|e| e.to_string())?) {
             Ok(m) => m,
             Err(_) => return Ok(()), // Ignore read errors
         };
         
         // For each type handled, look for installed packages of that type
         for (handled_type, _) in &target_meta.installer_handlers {
              if let Ok(entries) = std::fs::read_dir(&manifests_dir) {
                  for entry in entries.flatten() {
                      let path = entry.path();
                      if path.extension().and_then(|s| s.to_str()) == Some("json") {
                          if let Ok(child_meta) = serde_json::from_reader::<_, PackageMetadata>(std::fs::File::open(&path).unwrap()) {
                               if &child_meta.package_type == handled_type {
                                   child_packages.push(child_meta.name);
                               }
                          }
                      }
                  }
              }
         }

         for child in child_packages {
             self.collect_packages_to_uninstall(child, to_remove)?;
         }
         
         Ok(())
    }

    fn handle_uninstall(&self, api: &CoreHostApi, task_ctx: *mut c_void) -> FlowControl {
        let mut body_str = String::new();
        if let Some(v) = get_field_val(api, task_ctx, "request.payload") {
            if let Some(s) = v.as_str() { body_str = s.to_string(); }
        } else if let Some(v) = get_field_val(api, task_ctx, "request.body_path") { 
             if let Some(path) = v.as_str() {
                  if let Ok(s) = std::fs::read_to_string(path) {
                       body_str = s;
                  }
             }
        }

        let json: Value = serde_json::from_str(&body_str).unwrap_or(Value::Null);
        let package_name = json.get("package").and_then(|v| v.as_str()).unwrap_or("");
        
        if package_name.is_empty() {
             return self.error_response(api, task_ctx, 400, "Missing 'package' parameter");
        }

        let mut to_remove = Vec::new();
        if let Err(e) = self.collect_packages_to_uninstall(package_name.to_string(), &mut to_remove) {
             return self.error_response(api, task_ctx, 500, &format!("Failed to resolve dependencies for uninstall: {}", e));
        }

        self.log(ox_workflow_abi::OX_LOG_INFO, format!("Uninstalling packages: {:?}", to_remove));

        // Perform removal
        let manifests_dir = PathBuf::from(&self.config.manifests_directory);
        let installed_archives = PathBuf::from(&self.config.staging_directory).join("installed_archives");

        for pkg in &to_remove {
             // 1. Remove manifest
             let manifest_path = manifests_dir.join(format!("{}.json", pkg));
             
             // Check if it has handlers to unregister BEFORE deleting manifest
             if let Ok(meta) = serde_json::from_reader::<_, PackageMetadata>(std::fs::File::open(&manifest_path).unwrap()) {
                  if !meta.installer_handlers.is_empty() {
                       if let Ok(mut installers) = self.installers.write() {
                           for (k, _) in &meta.installer_handlers {
                               installers.remove(k);
                           }
                       }
                       // Save installers after loop
                       let _ = self.save_installers();
                  }
             }
             
             let _ = std::fs::remove_file(manifest_path);

             // 2. Remove archive (best effort, extension unknown)
             for ext in &self.config.allowed_extensions {
                  let archive_path = installed_archives.join(format!("{}{}", pkg, ext));
                  if archive_path.exists() {
                       let _ = std::fs::remove_file(archive_path);
                  }
             }
        }
        
        let response_json = serde_json::json!({ 
            "result": "success", 
            "message": format!("Uninstalled {} packages: {:?}", to_remove.len(), to_remove),
            "uninstalled": to_remove
        });
        
        set_field(api, task_ctx, "response.status", &200.to_string());
        set_field(api, task_ctx, "response.body", &response_json.to_string());

        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    }

    fn handle_upload(&self, api: &CoreHostApi, task_ctx: *mut c_void) -> FlowControl {
        let request_body_str = get_field(api, task_ctx, "request.body");
        let request_body = request_body_str.as_bytes().to_vec();
        self.log(ox_workflow_abi::OX_LOG_INFO, "Processing upload request...".to_string());

        // Check Content-Type (try both mixed-case and lowercase, since HTTP/2 uses lowercase)
        let content_type = {
            let ct = get_field(api, task_ctx, "request.header.content-type");
            if ct.is_empty() { get_field(api, task_ctx, "request.header.Content-Type") } else { ct }
        };
        
        self.log(ox_workflow_abi::OX_LOG_INFO, format!("Content-Type: {}", content_type));

        if !content_type.contains("multipart/form-data") {
        set_field(api, task_ctx, "response.status", &400.to_string());
        set_field(api, task_ctx, "response.type", &"application/json");
        set_field(api, task_ctx, "response.body", &"Invalid Content-Type");

        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

        let boundary_param = "boundary=";
        let boundary_idx = match content_type.find(boundary_param) {
        Some(i) => i + boundary_param.len(),
        None => {
             set_field(api, task_ctx, "response.status", &400.to_string());
              return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
         }
        };
        let boundary_full = &content_type[boundary_idx..];
        let boundary = boundary_full.split(';').next().unwrap_or(boundary_full).trim().trim_matches('"');
 
        let body_reader: Box<dyn std::io::Read> = if let Some(path_val) = get_field_val(api, task_ctx, "request.body_path") {
             if let Some(path) = path_val.as_str() {
                  match std::fs::File::open(path) {
                       Ok(f) => Box::new(f),
                       Err(e) => {
                            set_field(api, task_ctx, "response.status", &500.to_string());
                            set_field(api, task_ctx, "response.body", &serde_json::Value::String(format!("Failed to open request body file: {}", e)).to_string());
                            return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
                       }
                  }
             } else {
                  Box::new(Cursor::new(&request_body))
             }
        } else {
             Box::new(Cursor::new(&request_body))
        };

        let mut multipart = Multipart::with_body(body_reader, boundary);
        
        let mut upload_error: Option<String> = None;
        let mut processed_files = Vec::new();

        loop {
            match multipart.read_entry() {
                Ok(Some(mut entry)) => {
                    let name = entry.headers.name.as_ref().to_string();
                    if name == "package" || name == "file" {
                        let filename = entry.headers.filename.clone().unwrap_or("unknown.blob".to_string());
                        let target_path = PathBuf::from(&self.config.staging_directory).join(&filename);
                        
                        match std::fs::File::create(&target_path) {
                            Ok(mut file) => {
                                 if let Err(e) = std::io::copy(&mut entry.data, &mut file) {
                                      upload_error = Some(format!("Failed to copy content: {}", e));
                                      break; 
                                 } else {
                                      // === Trigger on_file_upload_completion ===
                                      match self.on_file_upload_completion(target_path.clone(), filename.clone()) {
                                          Ok((extraction_path, extracted)) => {
                                              processed_files.push(serde_json::json!({
                                                  "filename": filename,
                                                  "extracted": extracted,
                                                  "extraction_path": extraction_path
                                              }));
                                          },
                                          Err(e) => {
                                              // Validation failed
                                              upload_error = Some(e);
                                              break;
                                          }
                                      }
                                 }
                            },
                            Err(e) => {
                                 upload_error = Some(format!("Unable to write to staging directory: {}", e));
                                 break; 
                            }
                        }
                    }
                },
                Ok(None) => break, 
                Err(e) => {
                    upload_error = Some(format!("Multipart error: {}", e));
                    break;
                }
            }
        }

        if let Some(err_msg) = upload_error {
          let response_json = serde_json::json!({
              "result": "error",
              "message": err_msg
          });
          set_field(api, task_ctx, "response.status", &200.to_string());
          set_field(api, task_ctx, "response.type", &"application/json");
          set_field(api, task_ctx, "response.body", &response_json.to_string());

     } else {
          let response_json = serde_json::json!({
              "result": "success",
              "files": processed_files
          });
          set_field(api, task_ctx, "response.status", &200.to_string());
          set_field(api, task_ctx, "response.type", &"application/json");
          set_field(api, task_ctx, "response.body", &response_json.to_string());
     }

        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    }

    fn resolve_dependencies(&self, api: &CoreHostApi, task_ctx: *mut c_void, metadata: &PackageMetadata, depth: u32, visited: &mut Vec<String>) -> Result<(), String> {
        if depth > self.config.max_dependency_depth {
            return Err(format!("Maximum dependency depth reached ({})", self.config.max_dependency_depth));
        }

        for dep in &metadata.dependencies {
            if visited.contains(dep) {
                self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: Cyclical dependency detected or already visited: {}. Skipping.", dep));
                continue;
            }
            visited.push(dep.clone());

            // Check if installed
            let installed_path = PathBuf::from(&self.config.manifests_directory).join(format!("{}.json", dep));
            if installed_path.exists() {
                self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: Dependency '{}' already installed.", dep));
                continue;
            }

            // Look in staged
            let staging_path = PathBuf::from(&self.config.staging_directory);
            let mut dep_archive_path = None;
            
            // Search for archive starting with dep name
            if let Ok(entries) = std::fs::read_dir(&staging_path) {
                for entry in entries.flatten() {
                    let file_name = entry.file_name().to_string_lossy().to_string();
                    if file_name.starts_with(dep) {
                        for ext in &self.config.allowed_extensions {
                            if file_name.ends_with(ext) {
                                dep_archive_path = Some(entry.path());
                                break;
                            }
                        }
                    }
                    if dep_archive_path.is_some() { break; }
                }
            }

            match dep_archive_path {
                Some(path) => {
                    self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: Found staged archive for dependency '{}': {:?}", dep, path));
                    let dep_metadata = self.extract_metadata_from_archive(&path)?;
                    
                    // Recursive call
                    self.resolve_dependencies(api, task_ctx, &dep_metadata, depth + 1, visited)?;

                    // Now install it
                    let dep_filename = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    self.perform_install(api, task_ctx, &dep_filename, &dep_metadata)?;
                },
                None => {
                    return Err(format!("Dependency '{}' not found in installed or staged.", dep));
                }
            }
        }
        Ok(())
    }

    fn perform_install(&self, api: &CoreHostApi, task_ctx: *mut c_void, filename: &str, metadata: &PackageMetadata) -> Result<(), String> {
         self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: perform_install for {}", filename));
         
         let staging_path = PathBuf::from(&self.config.staging_directory);
         let source_path = staging_path.join(filename);
         
         // 1. Resolve installer
         let installer_id = {
             let installers = match self.installers.read() {
                 Ok(lock) => lock,
                 Err(_) => return Err("Internal error: installers lock poisoned".to_string()),
             };
             installers.get(&metadata.package_type).cloned()
         };

         let installer_id = match installer_id {
             Some(id) => id,
             None => return Err(format!("No installer registered for type: {}", metadata.package_type)),
         };

         // 2. Handle Installation
         if installer_id == self.module_id || installer_id == "package_manager" {
              // Self-installation for "module" packages
              let installed_archives = staging_path.join("installed_archives");
              let _ = std::fs::create_dir_all(&installed_archives);

              let dest_path = installed_archives.join(filename);
              std::fs::rename(&source_path, &dest_path).map_err(|e| format!("Failed to move package: {}", e))?;

              // Build metadata json and save to installed/
               let manifest_json = serde_json::to_string_pretty(metadata).unwrap_or_default();
               let manifest_path = PathBuf::from(&self.config.manifests_directory).join(format!("{}.json", metadata.name));
              std::fs::write(&manifest_path, &manifest_json).map_err(|e| format!("Failed to save manifest: {}", e))?;

              // Also write a protobuf-encoded binary alongside the JSON manifest.
              let proto = PackageMetadataProto {
                  name: metadata.name.clone(),
                  version: metadata.version.clone(),
                  description: metadata.description.clone(),
                  package_type: metadata.package_type.clone(),
                  resources_json: serde_json::to_string(&metadata.resources).unwrap_or_default(),
                  filename: metadata.filename.clone(),
                  size: metadata.size,
                  dependencies: metadata.dependencies.clone(),
                  installer_handlers_json: serde_json::to_string(&metadata.installer_handlers).unwrap_or_default(),
              };
              let mut proto_bytes = Vec::new();
              if proto.encode(&mut proto_bytes).is_ok() {
                  let proto_path = PathBuf::from(&self.config.manifests_directory).join(format!("{}.proto.bin", metadata.name));
                  let _ = std::fs::write(proto_path, &proto_bytes);
              }

              // Clean up meta file if it exists
              let meta_path = staging_path.join(format!("{}.meta", filename));
              if meta_path.exists() {
                  let _ = std::fs::remove_file(meta_path);
              }
         } else {
              // Delegated installation
              let temp_dir = staging_path.join(format!("tmp_{}", filename));
              let _ = std::fs::remove_dir_all(&temp_dir); // Clean start
              std::fs::create_dir_all(&temp_dir).map_err(|e| format!("Failed to create temp directory: {}", e))?;

              self.extract_package(&source_path, &temp_dir)?;

              // Dispatch to installer
              set_field(api, task_ctx, "installer.package_path", &Value::String(temp_dir.to_string_lossy().to_string()).to_string());
              set_field(api, task_ctx, "installer.action", &Value::String("install".to_string()).to_string());
              set_field(api, task_ctx, "installer.manifest", &serde_json::to_value(&metadata).unwrap_or(Value::Null).to_string());
              set_field(api, task_ctx, "installer.package_name", &Value::String(metadata.name.clone()).to_string());

               self.log(ox_workflow_abi::OX_LOG_INFO, format!("DEBUG: Delegating installation to {}", installer_id));
               // TODO: cross-module dispatch not yet supported in workflow ABI
               let _ = std::fs::remove_dir_all(&temp_dir);
               return Err(format!("Delegated installation to installer '{}' is not yet supported in the workflow ABI", installer_id));
          }
          Ok(())
     }
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let p = (api.get_field)(task_ctx, c_key.as_ptr());
    if p.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
}

fn get_field_val(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> Option<serde_json::Value> {
    let v = get_field(api, task_ctx, key);
    if v.is_empty() { None } else { Some(serde_json::Value::String(v)) }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    let c_key = CString::new(key).unwrap();
    let c_val = CString::new(value).unwrap();
    (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
}

fn get_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> Option<Vec<u8>> {
    let c_key = CString::new(key).unwrap();
    let mut len: usize = 0;
    let ptr = (api.get_field_bytes)(task_ctx, c_key.as_ptr(), &mut len as *mut usize);
    if ptr.is_null() || len == 0 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

fn set_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, data: &[u8]) {
    let c_key = CString::new(key).unwrap();
    (api.set_field_bytes)(task_ctx, c_key.as_ptr(), data.as_ptr(), data.len());
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

    let params_value: serde_json::Value = serde_json::from_str(&params_str).unwrap_or(serde_json::Value::Null);
    let mut config: Config = serde_json::from_str(&params_str).unwrap_or_default();

    if let Some(config_path) = params_value.get("config_file").and_then(|v| v.as_str()) {
        if let Ok(file_content) = std::fs::read_to_string(config_path) {
            if let Ok(loaded) = serde_yaml::from_str::<Config>(&file_content) {
                config = loaded;
            } else if let Ok(loaded) = serde_json::from_str::<Config>(&file_content) {
                config = loaded;
            }
        }
    }

    let module = OxModule::new(api, config, MODULE_NAME.to_string());
    let _ = std::fs::create_dir_all(&module.config.staging_directory);
    let _ = std::fs::create_dir_all(&module.config.manifests_directory);

    Box::into_raw(Box::new(module)) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_process(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    if plugin_config_ctx.is_null() {
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }
    let module = unsafe { &*(plugin_config_ctx as *mut OxModule) };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        module.process_request(task_ctx)
    }));
    match result {
        Ok(fc) => fc,
        Err(_) => {
            module.log(ox_workflow_abi::OX_LOG_ERROR, "Panic in ox_package_manager".to_string());
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
        let _ = Box::from_raw(plugin_config_ctx as *mut OxModule);
    }
}
