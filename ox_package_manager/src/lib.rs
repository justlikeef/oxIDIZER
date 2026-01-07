use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::io::Cursor;
use ox_webservice_api::{
    AllocFn, AllocStrFn, HandlerResult, LogCallback, LogLevel, ModuleInterface, PipelineState, 
    ModuleStatus, FlowControl, ReturnParameters, CoreHostApi
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yaml;
use multipart::server::Multipart;
use bumpalo::Bump;

const MODULE_NAME: &str = "ox_package_manager";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(default = "Config::default_staging_directory")]
    staging_directory: String,
    #[serde(default = "Config::default_allowed_extensions")]
    allowed_extensions: Vec<String>,
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
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PackageMetadata {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    // Fields added for the list API
    #[serde(default)]
    filename: String,
    #[serde(default)]
    size: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            staging_directory: "/tmp/ox_staging".to_string(),
            allowed_extensions: vec![".tar.gz".to_string(), ".tgz".to_string(), ".zip".to_string(), ".tar.bz2".to_string(), ".tbz2".to_string()],
        }
    }
}

pub struct OxModule {
    api: &'static CoreHostApi,
    config: Config,
    module_id: String,
}

impl OxModule {
    pub fn new(api: &'static CoreHostApi, config: Config, module_id: String) -> Self {
        let _ = ox_webservice_api::init_logging(api.log_callback, &module_id);
        
        // Ensure staging directory exists
        if let Err(_e) = std::fs::create_dir_all(&config.staging_directory) {
             // FFI log is available via self.log if we had an instance, but here we don't.
        }

        Self {
            api,
            config,
            module_id,
        }
    }

    fn log(&self, level: LogLevel, message: String) {
        if let Ok(c_message) = CString::new(message) {
            let module_name = CString::new(self.module_id.clone()).unwrap_or(CString::new(MODULE_NAME).unwrap());
            unsafe {
                (self.api.log_callback)(level, module_name.as_ptr(), c_message.as_ptr());
            }
        }
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        if pipeline_state_ptr.is_null() {
            return HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
            };
        }

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

        let ctx = unsafe { ox_pipeline_plugin::PipelineContext::new(
            self.api, 
            pipeline_state_ptr as *mut c_void, 
            arena_ptr
        ) };

        let verb_json = ctx.get("request.verb");
        let resource_json = ctx.get("request.resource");

        let verb = verb_json.and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default();
        let resource = resource_json.and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default();
        
        self.log(LogLevel::Info, format!("DEBUG_REQUEST_GENERIC: verb='{}', resource='{}'", verb, resource));

        if verb == "create" && (resource == "upload" || resource == "upload/" || resource.ends_with("/upload") || resource.ends_with("/upload/")) {
            return self.handle_upload(&ctx, pipeline_state);
        }

        if verb == "get" && (resource == "list" || resource == "list/" || resource.ends_with("/list") || resource.ends_with("/list/")) {
            return self.handle_list_staged(&ctx);
        }

        if verb == "create" && (resource == "install" || resource == "install/" || resource.ends_with("/install") || resource.ends_with("/install/")) {
            return self.handle_install(&ctx);
        }

        // Default: Ignore
        HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
        }
    }

    fn handle_install(&self, ctx: &ox_pipeline_plugin::PipelineContext) -> HandlerResult {
        let mut body_str = String::new();
        if let Some(v) = ctx.get("request.payload") {
            if let Some(s) = v.as_str() {
                body_str = s.to_string();
            }
        }
        
        if body_str.is_empty() {
             if let Some(v) = ctx.get("request.body_path") {
                  if let Some(path) = v.as_str() {
                       if let Ok(s) = std::fs::read_to_string(path) {
                            body_str = s;
                       }
                  }
             }
        }
            
        let json: Value = serde_json::from_str(&body_str).unwrap_or(Value::Null);
        let filename = json.get("filename").and_then(|v| v.as_str()).unwrap_or("");

        if filename.is_empty() {
             // return self.json_response(400, "error", "Filename is required", None);
             let response_json = serde_json::json!({ "result": "error", "message": "Filename is required" });
             let _ = ctx.set("response.status", serde_json::json!(400));
             let _ = ctx.set("response.type", serde_json::Value::String("application/json".to_string()));
             let _ = ctx.set("response.body", serde_json::Value::String(response_json.to_string()));
             return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
        }

        let staging_path = PathBuf::from(&self.config.staging_directory);
        let source_path = staging_path.join(filename);
        let meta_path = staging_path.join(format!("{}.meta", filename));

        if !source_path.exists() {
             // return self.json_response(404, "error", "Package file not found", None);
             let response_json = serde_json::json!({ "result": "error", "message": "Package file not found" });
             
             let _ = ctx.set("response.status", serde_json::json!(404));
             let _ = ctx.set("response.type", serde_json::Value::String("application/json".to_string()));
             let _ = ctx.set("response.body", serde_json::Value::String(response_json.to_string()));

             return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
        }

        // define installed path (subdirectory 'installed' in staging root for now)
        let installed_dir = staging_path.join("installed");
        if let Err(e) = std::fs::create_dir_all(&installed_dir) {
             let response_json = serde_json::json!({ "result": "error", "message": format!("Failed to create installed directory: {}", e) });
             
             let _ = ctx.set("response.status", serde_json::json!(500));
             let _ = ctx.set("response.type", serde_json::Value::String("application/json".to_string()));
             let _ = ctx.set("response.body", serde_json::Value::String(response_json.to_string()));

             return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
        }

        let dest_path = installed_dir.join(filename);

        // Move the file
        if let Err(e) = std::fs::rename(&source_path, &dest_path) {
             let response_json = serde_json::json!({ "result": "error", "message": format!("Failed to move package: {}", e) });
             
             let _ = ctx.set("response.status", serde_json::json!(500));
             let _ = ctx.set("response.type", serde_json::Value::String("application/json".to_string()));
             let _ = ctx.set("response.body", serde_json::Value::String(response_json.to_string()));

             return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
        }

        // Clean up meta file if it exists
        if meta_path.exists() {
            let _ = std::fs::remove_file(meta_path);
        }

        // self.json_response(200, "success", "Package installed successfully", None)
        let response_json = serde_json::json!({ "result": "success", "message": "Package installed successfully" });
        
        let _ = ctx.set("response.status", serde_json::json!(200));
        let _ = ctx.set("response.type", serde_json::Value::String("application/json".to_string()));
        let _ = ctx.set("response.body", serde_json::Value::String(response_json.to_string()));

        HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } }
    }

    // ... (helper methods)

    fn extract_metadata_from_archive(&self, file_path: &PathBuf) -> Result<PackageMetadata, String> {
        let filename = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        let file = std::fs::File::open(file_path).map_err(|e| format!("Failed to open package: {}", e))?;

        let mut metadata = PackageMetadata {
            name: "unknown".to_string(),
            version: "0.0.0".to_string(),
            description: "".to_string(),
            filename: filename.clone(),
            size: std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0),
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
                let path = entry.path().unwrap().to_string_lossy().to_string();
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
                let path = entry.path().unwrap().to_string_lossy().to_string();
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
                 }
             } else {
                 if let Ok(json) = serde_json::from_slice::<Value>(&content) {
                     if let Some(n) = json.get("name").and_then(|v| v.as_str()) { metadata.name = n.to_string(); }
                     if let Some(v) = json.get("version").and_then(|v| v.as_str()) { metadata.version = v.to_string(); }
                     if let Some(d) = json.get("description").and_then(|v| v.as_str()) { metadata.description = d.to_string(); }
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
         let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
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

    fn handle_list_staged(&self, ctx: &ox_pipeline_plugin::PipelineContext) -> HandlerResult {
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

        let response_json = serde_json::json!({
            "result": "success",
            "packages": packages
        });

        let _ = ctx.set("response.status", serde_json::json!(200));
        let _ = ctx.set("response.type", serde_json::Value::String("application/json".to_string()));
        let _ = ctx.set("response.body", serde_json::Value::String(response_json.to_string()));

        HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
        }
    }

    fn handle_upload(&self, ctx: &ox_pipeline_plugin::PipelineContext, pipeline_state: &mut PipelineState) -> HandlerResult {
        self.log(LogLevel::Info, "Processing upload request...".to_string());

        // Check Content-Type
        let content_type = ctx.get("request.header.Content-Type")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        
        self.log(LogLevel::Info, format!("Content-Type: {}", content_type));

        if !content_type.contains("multipart/form-data") {
        let _ = ctx.set("response.status", serde_json::json!(400));
        let _ = ctx.set("response.type", serde_json::Value::String("application/json".to_string()));
        let _ = ctx.set("response.body", serde_json::Value::String("Invalid Content-Type".to_string()));

        return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
    }

        let boundary_param = "boundary=";
        let boundary_idx = match content_type.find(boundary_param) {
        Some(i) => i + boundary_param.len(),
        None => {
             let _ = ctx.set("response.status", serde_json::json!(400));
              return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
         }
        };
        let boundary_full = &content_type[boundary_idx..];
        let boundary = boundary_full.split(';').next().unwrap_or(boundary_full).trim().trim_matches('"');
 
        let body_reader: Box<dyn std::io::Read> = if let Some(path_val) = ctx.get("request.body_path") {
             if let Some(path) = path_val.as_str() {
                  match std::fs::File::open(path) {
                       Ok(f) => Box::new(f),
                       Err(e) => {
                            let _ = ctx.set("response.status", serde_json::json!(500));
                            let _ = ctx.set("response.body", serde_json::Value::String(format!("Failed to open request body file: {}", e)));
                            return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
                       }
                  }
             } else {
                  Box::new(Cursor::new(&pipeline_state.request_body))
             }
        } else {
             Box::new(Cursor::new(&pipeline_state.request_body))
        };

        let mut multipart = Multipart::with_body(body_reader, boundary);
        
        let mut upload_error: Option<String> = None;
        let mut processed_files = Vec::new();

        loop {
            match multipart.read_entry() {
                Ok(Some(mut entry)) => {
                    let name = entry.headers.name.as_ref().to_string();
                    if name == "package" {
                        let filename = entry.headers.filename.clone().unwrap_or("unknown.blob".to_string());
                        let target_path = PathBuf::from(&self.config.staging_directory).join(&filename);
                        
                        // Validate extension early (Quick fail)? No, do it in on_file_upload_completion to be consistent with prompt requirement.
                        
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
         // Use 200 to prevent ox_webservice_errorhandler from replacing JSON with HTML
         let _ = ctx.set("response.status", serde_json::json!(200));
         let _ = ctx.set("response.type", serde_json::Value::String("application/json".to_string()));
         let _ = ctx.set("response.body", serde_json::Value::String(response_json.to_string()));

    } else {
         // Success
         let response_json = serde_json::json!({
             "result": "success",
             "files": processed_files
         });
         let _ = ctx.set("response.status", serde_json::json!(200));
         let _ = ctx.set("response.type", serde_json::Value::String("application/json".to_string()));
         let _ = ctx.set("response.body", serde_json::Value::String(response_json.to_string()));
    }

        HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue, 
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
        }
    }
}

// Boilerplate C-exports
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
    
    // Deserialize config directly from params
    let mut config: Config = serde_json::from_str(module_params_json).unwrap_or_else(|_| Config::default());

    // Check if we need to load from external file (if staging_directory is still default or explicit config_file present)
    let params_value: Value = serde_json::from_str(module_params_json).unwrap_or(Value::Null);
    
    if let Some(config_path) = params_value.get("config_file").and_then(|v| v.as_str()) {
         if let Ok(file_content) = std::fs::read_to_string(config_path) {
             // Try YAML first (common), then JSON
             if let Ok(loaded_config) = serde_yaml::from_str::<Config>(&file_content) {
                 config = loaded_config;
             } else if let Ok(loaded_config) = serde_json::from_str::<Config>(&file_content) {
                 config = loaded_config;
             }
         }
    }

    let module_id = if !module_id_ptr.is_null() {
        CStr::from_ptr(module_id_ptr).to_string_lossy().to_string()
    } else {
        MODULE_NAME.to_string()
    };

    let module = OxModule::new(api, config, module_id);
    let instance_ptr = Box::into_raw(Box::new(module)) as *mut c_void;

    Box::into_raw(Box::new(ModuleInterface {
        instance_ptr,
        handler_fn: process_request_c,
        log_callback: api.log_callback,
        get_config: get_config_c,
    }))
}

unsafe extern "C" fn process_request_c(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena: *const c_void, 
) -> HandlerResult {
    if instance_ptr.is_null() {
        return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let handler = unsafe { &*(instance_ptr as *mut OxModule) };
        handler.process_request(pipeline_state_ptr)
    }));

    match result {
        Ok(handler_result) => handler_result,
        Err(e) => {
             // Try to log the panic if possible
             let msg = if let Some(s) = e.downcast_ref::<&str>() {
                 format!("Panic in ox_package_manager: {}", s)
             } else if let Some(s) = e.downcast_ref::<String>() {
                 format!("Panic in ox_package_manager: {}", s)
             } else {
                 "Panic in ox_package_manager: unknown error".to_string()
             };

             if let Ok(c_msg) = CString::new(msg) {
                 if let Ok(mod_name) = CString::new(MODULE_NAME) {
                      unsafe { (log_callback)(LogLevel::Error, mod_name.as_ptr(), c_msg.as_ptr()); }
                 }
             }

             HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
            }
        }
    }
}

unsafe extern "C" fn get_config_c(
    _instance_ptr: *mut c_void,
    _arena: *const c_void,
    _alloc_fn: AllocStrFn,
) -> *mut c_char {
    std::ptr::null_mut() // TODO: Implement
}
