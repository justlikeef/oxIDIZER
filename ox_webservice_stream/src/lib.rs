use regex::Regex;
use libc::{c_void, c_char};
use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    CoreHostApi, WebServiceApiV1, AllocFn, AllocStrFn, PipelineState,
    ModuleStatus,    FlowControl, ReturnParameters,
};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{CStr, CString};
use std::fs;
use std::panic;
use std::path::PathBuf;
use anyhow::Result;
use bumpalo::Bump;

mod tests;

const MODULE_NAME: &str = "ox_webservice_stream";

#[derive(Debug, Deserialize, Clone, serde::Serialize)]
pub struct MimeTypeMapping {
    url: String,
    mimetype: String,
    #[serde(skip)]
    compiled_regex: Option<Regex>,
}

#[derive(Debug, Deserialize)]
struct MimeTypeConfig {
    mimetypes: Vec<MimeTypeMapping>,
}

#[derive(Debug, Deserialize, Clone, serde::Serialize)]
pub struct DocumentConfig {
    document: String,
}

#[derive(Debug, Deserialize, Clone, serde::Serialize)]
pub struct ContentConfig {
    content_root: String,
    mimetypes_file: String,
    #[serde(default)]
    default_documents: Vec<DocumentConfig>,
    #[serde(default)]
    on_content_conflict: Option<ContentConflictAction>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(non_camel_case_types)]
pub enum ContentConflictAction {
    overwrite,
    append,
    skip,
    error,
}

pub struct OxModule<'a> {
    pub content_root: PathBuf,
    pub mimetypes: Vec<MimeTypeMapping>,
    pub default_documents: Vec<DocumentConfig>,
    pub content_config: ContentConfig,
    pub module_id: String,
    api: &'a CoreHostApi,
}

impl<'a> OxModule<'a> {
    fn log(&self, level: LogLevel, message: String) {
        if let Ok(c_message) = CString::new(message) {
            let module_name = CString::new(self.module_id.clone()).unwrap_or(CString::new(MODULE_NAME).unwrap());
            unsafe {
                (self.api.log_callback)(level, module_name.as_ptr(), c_message.as_ptr());
            }
        }
    }

    // Constructor still takes WebServiceApiV1 to support legacy casting in init, 
    // BUT we will cast it to CoreHostApi immediately or change signature.
    // Changing signature in `new` is cleaner.
    pub fn new(config: ContentConfig, api: &'a CoreHostApi, module_id: String) -> Result<Self> {
        let _ = ox_webservice_api::init_logging(api.log_callback, &module_id);

        if let Ok(c_message) = CString::new(format!(
            "ox_webservice_stream: new: Initializing with content_root: {:?}",
            config.content_root
        )) {
            let module_name = CString::new(module_id.clone()).unwrap();
            unsafe { (api.log_callback)(LogLevel::Info, module_name.as_ptr(), c_message.as_ptr()); }
        }

        let mimetype_file_name = &config.mimetypes_file;
        let mimetype_path = PathBuf::from(mimetype_file_name);
        
        // Use ox_fileproc for mimetypes config
        let mut mimetype_config: MimeTypeConfig = match ox_fileproc::process_file(&mimetype_path, 5) {
            Ok(value) => {
                 if let Ok(json_str) = serde_json::to_string_pretty(&value) {
                     use log::debug;
                     debug!("Fully processed config for {:?}:\n{}", mimetype_path, json_str);
                 }
                 match serde_json::from_value(value) {
                Ok(cfg) => cfg,
                Err(e) => {
                     if let Ok(c_message) = CString::new(format!(
                        "ox_webservice_stream: Failed to deserialize mimetype config file {}: {}",
                        mimetype_file_name, e
                    )) {
                        let module_name = CString::new(MODULE_NAME).unwrap();
                        unsafe { (api.log_callback)(LogLevel::Error, module_name.as_ptr(), c_message.as_ptr()); }
                    }
                    anyhow::bail!("Failed to deserialize mimetype config: {}", e);
                }
                }
            },
            Err(e) => {
                if let Ok(c_message) = CString::new(format!(
                    "ox_webservice_stream: Failed to process mimetype config file {}: {}",
                    mimetype_file_name, e
                )) {
                     let module_name = CString::new(MODULE_NAME).unwrap();
                     unsafe { (api.log_callback)(LogLevel::Error, module_name.as_ptr(), c_message.as_ptr()); }
                }
                anyhow::bail!("Failed to process mimetype config: {}", e);
            }
        };

        // Compile Regexes
        for mapping in &mut mimetype_config.mimetypes {
            match Regex::new(&mapping.url) {
                Ok(re) => mapping.compiled_regex = Some(re),
                Err(e) => {
                     if let Ok(c_message) = CString::new(format!(
                        "ox_webservice_stream: Failed to compile regex '{}': {}",
                        mapping.url, e
                    )) {
                        let module_name = CString::new(MODULE_NAME).unwrap();
                        unsafe { (api.log_callback)(LogLevel::Error, module_name.as_ptr(), c_message.as_ptr()); }
                    }
                }
            }
        }

        Ok(Self {
            content_root: PathBuf::from(config.content_root.clone()),
            mimetypes: mimetype_config.mimetypes,
            default_documents: config.default_documents.clone(),
            content_config: config,
            module_id,
            api,
        })
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        if pipeline_state_ptr.is_null() {
            self.log(LogLevel::Error, "ox_webservice_stream: proccess_request called with null pipeline state.".to_string());
            return HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Halt,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
            };
        }

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;
        
        let ctx = unsafe { ox_pipeline_plugin::PipelineContext::new(
            self.api, 
            pipeline_state_ptr as *mut c_void, 
            arena_ptr
        ) };


        // --- Early Conflict Check for Skip ---
        // Check generic state latch "pipeline.modified"
        let is_modified = if let Some(val) = ctx.get("pipeline.modified") {
             val.as_str().unwrap_or("false") == "true"
        } else { false };

        self.log(LogLevel::Info, format!("ox_webservice_stream: Early conflict check. is_modified={}, action={:?}", is_modified, self.content_config.on_content_conflict));

        if is_modified {
             match self.content_config.on_content_conflict.unwrap_or(ContentConflictAction::skip) {
                 ContentConflictAction::skip => {
                     self.log(LogLevel::Info, "ox_webservice_stream: Skipping due to existing content (early check).".to_string());
                     return HandlerResult {
                        status: ModuleStatus::Unmodified,
                        flow_control: FlowControl::Continue,
                        return_parameters: ReturnParameters {
                            return_data: std::ptr::null_mut(),
                        },
                     };
                 },
                 ContentConflictAction::error => {
                     self.log(LogLevel::Error, "ox_webservice_stream: Conflict error (early check).".to_string());
                     let _ = ctx.set("http.response.status", serde_json::json!(500));
                      return HandlerResult {
                         status: ModuleStatus::Modified,
                         flow_control: FlowControl::Continue, // Return 500 but continue pipeline? Or halt on 500?
                         return_parameters: ReturnParameters {
                             return_data: std::ptr::null_mut(),
                         },
                      };
                 },
                 _ => {}
             }
        }

        // Get Path
        let request_path = match ctx.get("http.request.path") {
            Some(v) => v.as_str().unwrap_or("/").to_string(),
            None => {
                 self.log(LogLevel::Error, "ox_webservice_stream: get(http.request.path) returned None.".to_string());
                 let _ = ctx.set("http.response.status", serde_json::json!(500));
                  return HandlerResult {
                     status: ModuleStatus::Modified,
                     flow_control: FlowControl::Continue,
                     return_parameters: ReturnParameters {
                         return_data: std::ptr::null_mut(),
                     },
                  };
            }
        };

        // Check for regex matches in module context (Legacy?)
        // "regex_matches" key relies on Generic State
        let mut resolved_path = request_path;
        
        // Router sets "http.request.path_capture"
        if let Some(val) = ctx.get("http.request.path_capture") {
             resolved_path = val.as_str().unwrap_or("").to_string();
        } else if let Some(val) = ctx.get("regex_matches") {
              if let Some(matches) = val.as_array() {
                  if let Some(first) = matches.get(0).and_then(|v| v.as_str()) {
                      resolved_path = first.to_string();
                  }
              }
        }

        if let Some(file_path) = self.resolve_and_find_file(&resolved_path) {
            let file_name_str = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            
            // Find first matching regex
            let mimetype_mapping = self.mimetypes.iter().find(|m| {
                if let Some(re) = &m.compiled_regex {
                    re.is_match(file_name_str)
                } else {
                    false
                }
            });

            // Determine mimetype (explicit or default)
            let mimetype = if let Some(mapping) = mimetype_mapping {
                mapping.mimetype.clone()
            } else {
                "application/octet-stream".to_string()
            };

            // Late Conflict Check (Optimization: Skip/Error handled early)
            if is_modified {
                    match self.content_config.on_content_conflict.unwrap_or(ContentConflictAction::skip) {
                        ContentConflictAction::skip | ContentConflictAction::error => {
                            // Should have been caught early
                            return HandlerResult {
                            status: ModuleStatus::Unmodified,
                            flow_control: FlowControl::Continue,
                            return_parameters: ReturnParameters {
                                return_data: std::ptr::null_mut(),
                            },
                            }; 
                        },
                        _ => {} // Proceed for Overwrite/Append
                    }
            }

            // Verify file existence (again? resolve_and_find check existence)
            // But we must open it? Host streams it by path. We just need to ensure metadata ok?
            match fs::metadata(&file_path) {
                Ok(metadata) => {
                        if !metadata.is_file() {
                            let _ = ctx.set("http.response.status", serde_json::json!(404));
                            return HandlerResult {
                                status: ModuleStatus::Modified,
                                flow_control: FlowControl::Continue, // Or halt?
                                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
                            };
                        }
                        
                        if let Err(e) = fs::File::open(&file_path) {
                            self.log(LogLevel::Error, format!("ox_webservice_stream: File exists but cannot be opened {}: {}", resolved_path, e));
                            let _ = ctx.set("http.response.status", serde_json::json!(500));
                            return HandlerResult {
                                status: ModuleStatus::Modified,
                                flow_control: FlowControl::Continue,
                                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
                            };
                        }

                        self.log(
                        LogLevel::Info,
                        format!(
                            "ox_webservice_stream: Streaming file for path: {}",
                            resolved_path
                        ),
                    );

                    // Set Content-Type
                    let _ = ctx.set("http.response.header.Content-Type", serde_json::Value::String(mimetype));
                    let _ = ctx.set("http.response.status", serde_json::json!(200));

                    // Pass file path to host
                    // Host takes ownership of the CString pointer
                    let c_path = CString::new(file_path.to_string_lossy().into_owned()).unwrap();
                    let return_data = c_path.into_raw() as *mut c_void;

                    return HandlerResult {
                        status: ModuleStatus::Modified,
                        flow_control: FlowControl::StreamFile,
                        return_parameters: ReturnParameters {
                            return_data,
                        },
                    };
                }
                Err(e) => {
                        self.log(
                        LogLevel::Error,
                        format!(
                            "ox_webservice_stream: Error accessing file metadata for path {}: {}",
                            resolved_path, e
                        ),
                    );

                    let _ = ctx.set("http.response.status", serde_json::json!(500));
                    return HandlerResult {
                        status: ModuleStatus::Modified,
                        flow_control: FlowControl::Continue, 
                        return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }, 
                    };
                }
            }
        } else {
             self.log(
                LogLevel::Info,
                format!(
                    "ox_webservice_stream: File not found for path: {}",
                    resolved_path
                ),
            );
            
            let _ = ctx.set("http.response.status", serde_json::json!(404));
            let _ = ctx.set("http.response.body", serde_json::Value::String("404 Not Found".to_string()));

             HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
            }
        }
    }

    fn resolve_and_find_file(&self, request_path: &str) -> Option<PathBuf> {
        let mut file_path = self.content_root.clone();
        file_path.push(request_path.trim_start_matches('/'));

        if !file_path.exists() {
            return None;
        }

        if let Ok(canonical_path) = file_path.canonicalize() {
            if !canonical_path.starts_with(&self.content_root) {
                return None;
            }
            file_path = canonical_path;
        } else {
            return None;
        }

        if file_path.is_dir() {
            for doc_config in &self.default_documents {
                let mut default_doc_candidate = file_path.clone();
                default_doc_candidate.push(&doc_config.document);
                if default_doc_candidate.exists() {
                    return Some(default_doc_candidate);
                }
            }
            None
        } else {
            Some(file_path)
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    module_id_ptr: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        eprintln!("ox_webservice_stream: api_ptr is null in initialize_module");
        return std::ptr::null_mut();
    }
    let api_instance = unsafe { &*api_ptr };

    let module_id = if !module_id_ptr.is_null() {
        unsafe { CStr::from_ptr(module_id_ptr).to_string_lossy().to_string() }
    } else {
        MODULE_NAME.to_string()
    };
    let c_module_id = CString::new(module_id.clone()).unwrap();

    if module_params_json_ptr.is_null() {
         let log_msg = CString::new("ox_webservice_stream: module_params_json_ptr is null").unwrap();
         unsafe { (api_instance.log_callback)(LogLevel::Error, c_module_id.as_ptr(), log_msg.as_ptr()); }
         return std::ptr::null_mut();
    }

    let result = panic::catch_unwind(|| {
        let module_params_json = unsafe { CStr::from_ptr(module_params_json_ptr).to_str().unwrap() };
        let params: Value =
            serde_json::from_str(module_params_json).expect("Failed to parse module params JSON");

        let config_file_name = match params.get("config_file").and_then(|v| v.as_str()) {
            Some(name) => name,
            None => {
                let log_msg = CString::new("'config_file' parameter is missing or not a string.").unwrap();
                let _ = panic::catch_unwind(|| {
                    unsafe { (api_instance.log_callback)(LogLevel::Error, c_module_id.as_ptr(), log_msg.as_ptr()); }
                });
                return std::ptr::null_mut();
            }
        };

        let config_path = PathBuf::from(config_file_name);
        
        let config: ContentConfig = match ox_fileproc::process_file(&config_path, 5) {
            Ok(value) => match serde_json::from_value(value) {
                Ok(c) => c,
                Err(e) => {
                     let log_msg = CString::new(format!("Failed to deserialize ContentConfig: {}", e)).unwrap();
                     unsafe { (api_instance.log_callback)(LogLevel::Error, c_module_id.as_ptr(), log_msg.as_ptr()); }
                     return std::ptr::null_mut();
                }
            },
            Err(e) => {
                 let log_msg = CString::new(format!("Failed to process config file '{}': {}", config_file_name, e)).unwrap();
                 let _ = panic::catch_unwind(|| {
                     unsafe { (api_instance.log_callback)(LogLevel::Error, c_module_id.as_ptr(), log_msg.as_ptr()); }
                 });
                 return std::ptr::null_mut();
            }
        };

    // Note: c_module_id is reference to module_id string which is cloned into OxModule?
    // We pass module_id (String) to new
    let handler = match OxModule::new(config, api_instance, module_id.clone()) {
            Ok(h) => {
                let log_msg = CString::new("ox_webservice_stream initialized").unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Info, c_module_id.as_ptr(), log_msg.as_ptr()); }
                h
            },
            Err(e) => {
                let log_msg = CString::new(format!("Failed to create OxModule: {}", e)).unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Error, c_module_id.as_ptr(), log_msg.as_ptr()); }
                return std::ptr::null_mut();
            }
        };

        let instance_ptr = Box::into_raw(Box::new(handler)) as *mut c_void;

        let module_interface = Box::new(ModuleInterface {
            instance_ptr,
            handler_fn: process_request_c,
            log_callback: api_instance.log_callback,
            get_config: get_config_c,
        });

        Box::into_raw(module_interface)
    });

    match result {
        Ok(ptr) => ptr,
        Err(e) => {
            eprintln!("Panic during module initialization: {:?}", e);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn process_request_c(
    instance_ptr: *mut c_void, 
    pipeline_state_ptr: *mut PipelineState, 
    _log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena: *const c_void,
) -> HandlerResult {
    // Safety check for handler instance
    if instance_ptr.is_null() {
        return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Halt,
            return_parameters: ReturnParameters {
                return_data: std::ptr::null_mut(),
            },
        };
    }

    let result = panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let handler = unsafe { &*(instance_ptr as *mut OxModule) };
        handler.process_request(pipeline_state_ptr)
    }));

    match result {
        Ok(handler_result) => handler_result,
        Err(e) => {
            let log_msg =
                CString::new(format!("Panic occurred in process_request_c: {:?}.", e)).unwrap();
            
             // We don't have log callback handy if we don't store it or pass it.
             // But we have _log_callback arg.
            let module_name = CString::new(MODULE_NAME).unwrap();
             // TODO: We could upgrade Handler to store module_id and log from there?
             // But process_request_c is the handler_fn.
             // We can cast instance_ptr to OxModule and get module_id?
             let handler_unsafe = unsafe { &*(instance_ptr as *mut OxModule) };
             let module_name_actual = CString::new(handler_unsafe.module_id.clone()).unwrap_or(module_name);
             
            unsafe { (_log_callback)(LogLevel::Error, module_name_actual.as_ptr(), log_msg.as_ptr()); }
            
            HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Halt,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_config_c(
    instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    if instance_ptr.is_null() { return std::ptr::null_mut(); }
    let handler = unsafe { &*(instance_ptr as *mut OxModule) };
    
    let mut config_val = serde_json::to_value(&handler.content_config).unwrap_or(Value::Null);
    if let Value::Object(ref mut map) = config_val {
         let mimetypes_val = serde_json::to_value(&handler.mimetypes).unwrap_or(Value::Null);
         map.insert("loaded_mimetypes".to_string(), mimetypes_val);
    }
    
    let json = serde_json::to_string_pretty(&config_val).unwrap_or("{}".to_string());
    
    // Use generic PluginContext for allocation safety
    // We pass null for state_ptr since we are only allocating
    let ctx = unsafe { ox_pipeline_plugin::PipelineContext::new(handler.api, std::ptr::null_mut(), arena) };
    ctx.alloc_string(&json)
}
