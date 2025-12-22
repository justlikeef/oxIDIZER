use regex::Regex;
use libc::{c_void, c_char};
use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    WebServiceApiV1, AllocFn, AllocStrFn, PipelineState,
    ModuleStatus, FlowControl, Phase, ReturnParameters,
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
    api: &'a WebServiceApiV1,
}

impl<'a> OxModule<'a> {
    fn log(&self, level: LogLevel, message: String) {
        if let Ok(c_message) = CString::new(message) {
            let module_name = CString::new(MODULE_NAME).unwrap();
            unsafe {
                (self.api.log_callback)(level, module_name.as_ptr(), c_message.as_ptr());
            }
        }
    }

    pub fn new(config: ContentConfig, api: &'a WebServiceApiV1) -> Result<Self> {
        let _ = ox_webservice_api::init_logging(api.log_callback, MODULE_NAME);

        if let Ok(c_message) = CString::new(format!(
            "ox_webservice_stream: new: Initializing with content_root: {:?}",
            config.content_root
        )) {
            let module_name = CString::new(MODULE_NAME).unwrap();
            unsafe { (api.log_callback)(LogLevel::Debug, module_name.as_ptr(), c_message.as_ptr()); }
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
                    // Continue, just won't match. Or could fail hard. Let's log error and leave None.
                }
            }
        }

        Ok(Self {
            content_root: PathBuf::from(config.content_root.clone()),
            mimetypes: mimetype_config.mimetypes,
            default_documents: config.default_documents.clone(),
            content_config: config,
            api,
        })
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        if pipeline_state_ptr.is_null() {
            self.log(LogLevel::Error, "ox_webservice_stream: proccess_request called with null pipeline state.".to_string());
            return HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::JumpTo,
                return_parameters: ReturnParameters {
                    return_data: (Phase::ErrorHandling as usize) as *mut c_void,
                },
            };
        }

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

        // --- Early Conflict Check for Skip ---
        let existing_body_ptr = unsafe { (self.api.get_response_body)(pipeline_state, arena_ptr, self.api.alloc_str) };
        let mut existing_body = String::new();
        if !existing_body_ptr.is_null() {
            existing_body = unsafe { CStr::from_ptr(existing_body_ptr).to_string_lossy().into_owned() };
        }
        
        let has_existing_content = !existing_body.is_empty();

        if has_existing_content {
             match self.content_config.on_content_conflict {
                 Some(ContentConflictAction::skip) => {
                     self.log(LogLevel::Debug, "ox_webservice_stream: Skipping due to existing content (early check).".to_string());
                     return HandlerResult {
                        status: ModuleStatus::Unmodified,
                        flow_control: FlowControl::Continue,
                        return_parameters: ReturnParameters {
                            return_data: std::ptr::null_mut(),
                        },
                     };
                 },
                 Some(ContentConflictAction::error) => {
                     self.log(LogLevel::Error, "ox_webservice_stream: Conflict error (early check).".to_string());
                     unsafe { (self.api.set_response_status)(pipeline_state, 500); }
                     return HandlerResult {
                        status: ModuleStatus::Modified,
                        flow_control: FlowControl::JumpTo,
                        return_parameters: ReturnParameters {
                            return_data: (Phase::ErrorHandling as usize) as *mut c_void,
                        },
                     };
                 },
                 _ => {}
             }
        }

        let request_path_ptr = unsafe { (self.api.get_request_path)(pipeline_state, arena_ptr, self.api.alloc_str) };
        if request_path_ptr.is_null() {
             self.log(LogLevel::Error, "ox_webservice_stream: get_request_path returned null.".to_string());
             unsafe { (self.api.set_response_status)(pipeline_state, 500); }
             return HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::JumpTo,
                return_parameters: ReturnParameters {
                    return_data: (Phase::ErrorHandling as usize) as *mut c_void,
                },
             };
        }
        let raw_request_path = unsafe { CStr::from_ptr(request_path_ptr).to_str().unwrap_or("/") };

        // Check for regex matches in module context
        let regex_matches_key = CString::new("regex_matches").unwrap();
        let regex_matches_ptr = unsafe {
            (self.api.get_module_context_value)(pipeline_state, regex_matches_key.as_ptr(), arena_ptr, self.api.alloc_str)
        };
        
        let mut request_path = raw_request_path;
        // Need to keep the CString alive if we allocate one, but here we just have a pointer to arena-allocated memory.
        
        let mut regex_path_buf_storage: Option<String> = None;

        if !regex_matches_ptr.is_null() {
            let regex_matches_json = unsafe { CStr::from_ptr(regex_matches_ptr).to_str().unwrap_or("[]") };
            self.log(LogLevel::Debug, format!("ox_webservice_stream: Found regex_matches: {}", regex_matches_json));
             if let Ok(Value::Array(matches)) = serde_json::from_str::<Value>(regex_matches_json) {
                 if let Some(Value::String(first_match)) = matches.get(0) {
                     // Use the first capture group as the path
                     regex_path_buf_storage = Some(first_match.clone());
                 }
             }
        }

        if let Some(ref p) = regex_path_buf_storage {
            request_path = p.as_str();
        }

        if let Some(file_path) = self.resolve_and_find_file(request_path) {
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
            if has_existing_content {
                    // Only Overwrite and Append reach here
                    match self.content_config.on_content_conflict.unwrap_or(ContentConflictAction::overwrite) {
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

            // Verify file existence and readability without loading into memory
            match fs::metadata(&file_path) {
                Ok(metadata) => {
                        if !metadata.is_file() {
                            // Not a file (e.g. directory)
                            unsafe { (self.api.set_response_status)(pipeline_state, 404); }
                            return HandlerResult {
                                status: ModuleStatus::Modified,
                                flow_control: FlowControl::Continue, // Or halt?
                                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
                            };
                        }
                        
                        // Check readability? fs::File::open is better.
                        if let Err(e) = fs::File::open(&file_path) {
                            self.log(LogLevel::Error, format!("ox_webservice_stream: File exists but cannot be opened {}: {}", request_path, e));
                            unsafe { (self.api.set_response_status)(pipeline_state, 500); }
                            return HandlerResult {
                                status: ModuleStatus::Modified,
                                flow_control: FlowControl::JumpTo,
                                return_parameters: ReturnParameters { return_data: (Phase::ErrorHandling as usize) as *mut c_void },
                            };
                        }

                        self.log(
                        LogLevel::Debug,
                        format!(
                            "ox_webservice_stream: Streaming file for path: {}",
                            request_path
                        ),
                    );

                    // Set Content-Type
                    unsafe {
                        let c_content_type_key = CString::new("Content-Type").unwrap();
                        let c_content_type_value = CString::new(mimetype.as_str()).unwrap();
                        (self.api.set_response_header)(
                            pipeline_state,
                            c_content_type_key.as_ptr(),
                            c_content_type_value.as_ptr(),
                        );
                        // Explicitly set 200 OK
                        (self.api.set_response_status)(pipeline_state, 200);
                    }

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
                            request_path, e
                        ),
                    );

                    unsafe { (self.api.set_response_status)(pipeline_state, 500); }
                    return HandlerResult {
                        status: ModuleStatus::Modified,
                        flow_control: FlowControl::JumpTo, // Go to error handling
                        return_parameters: ReturnParameters { return_data: (Phase::ErrorHandling as usize) as *mut c_void }, 
                    };
                }
            }
        } else {
             self.log(
                LogLevel::Debug,
                format!(
                    "ox_webservice_stream: File not found for path: {}",
                    request_path
                ),
            );
            unsafe { 
                (self.api.set_response_status)(pipeline_state, 404);
                let body = CString::new("404 Not Found").unwrap();
                (self.api.set_response_body)(pipeline_state, body.as_ptr() as *const u8, body.as_bytes().len());
            }
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
    _module_id: *const c_char,
    api_ptr: *const WebServiceApiV1,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        eprintln!("ox_webservice_stream: api_ptr is null in initialize_module");
        return std::ptr::null_mut();
    }
    let api_instance = unsafe { &*api_ptr };

    if module_params_json_ptr.is_null() {
         let log_msg = CString::new("ox_webservice_stream: module_params_json_ptr is null").unwrap();
         let module_name = CString::new(MODULE_NAME).unwrap();
         unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
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
                let module_name = CString::new(MODULE_NAME).unwrap();
                let _ = panic::catch_unwind(|| {
                    unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                });
                return std::ptr::null_mut();
            }
        };

        let config_path = PathBuf::from(config_file_name);
        
        // Use ox_fileproc for module config
        let config: ContentConfig = match ox_fileproc::process_file(&config_path, 5) {
            Ok(value) => match serde_json::from_value(value) {
                Ok(c) => c,
                Err(e) => {
                     let log_msg = CString::new(format!("Failed to deserialize ContentConfig: {}", e)).unwrap();
                     let module_name = CString::new(MODULE_NAME).unwrap();
                     unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                     return std::ptr::null_mut();
                }
            },
            Err(e) => {
                 let log_msg = CString::new(format!("Failed to process config file '{}': {}", config_file_name, e)).unwrap();
                 let module_name = CString::new(MODULE_NAME).unwrap();
                 let _ = panic::catch_unwind(|| {
                     unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                 });
                 return std::ptr::null_mut();
            }
        };

        let handler = match OxModule::new(config, api_instance) {
            Ok(h) => {
                let log_msg = CString::new("ox_webservice_stream initialized").unwrap();
                let module_name = CString::new(MODULE_NAME).unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Info, module_name.as_ptr(), log_msg.as_ptr()); }
                h
            },
            Err(e) => {
                let log_msg = CString::new(format!("Failed to create OxModule: {}", e)).unwrap();
                let module_name = CString::new(MODULE_NAME).unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
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
    _instance_ptr: *mut c_void, 
    pipeline_state_ptr: *mut PipelineState, 
    _log_callback: LogCallback,
    alloc_fn: AllocFn,
    arena: *const c_void,
) -> HandlerResult {
    ox_webservice_api::init_logging(_log_callback, "ox_webservice_stream").ok();
    
    // ... logic ...
    // Note: Can't replace logic here without viewing file first, but I'll replace the function signature and known return points.
    // Wait, I can't blindly replace return points without knowing context.
    // I should view the files or perform comprehensive replacements.
    // For now, I'll update signature and import statements in this tool call, then handle body.
    // Actually, blindly replacing is risky. I used multi_replace incorrectly in thought process.
    // I will modify imports first.

    // Safety check for handler instance
    if _instance_ptr.is_null() {
        let log_msg = CString::new("ox_webservice_stream: process_request_c called with null instance_ptr").unwrap();
        let module_name = CString::new(MODULE_NAME).unwrap();
        unsafe { (_log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
        return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::JumpTo,
            return_parameters: ReturnParameters {
                return_data: (Phase::ErrorHandling as usize) as *mut c_void,
            },
        };
    }

    let result = panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let handler = unsafe { &*(_instance_ptr as *mut OxModule) };
        handler.process_request(pipeline_state_ptr)
    }));

    match result {
        Ok(handler_result) => handler_result,
        Err(e) => {
            let log_msg =
                CString::new(format!("Panic occurred in process_request_c: {:?}.", e)).unwrap();
            let module_name = CString::new(MODULE_NAME).unwrap();
            unsafe { (_log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
            HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::JumpTo,
                return_parameters: ReturnParameters {
                    return_data: (Phase::ErrorHandling as usize) as *mut c_void,
                },
            }
        }
    }
}

unsafe extern "C" fn get_config_c(
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
    alloc_fn(arena, CString::new(json).unwrap().as_ptr())
}
