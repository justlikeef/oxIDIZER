use libc::{c_void, c_char};
use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    WebServiceApiV1, AllocFn, PipelineState,
};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{CStr, CString};
use std::fs;
use std::panic;
use std::path::PathBuf;
use anyhow::Result;
use bumpalo::Bump;

mod handlers;

const MODULE_NAME: &str = "ox_content";

#[derive(Debug, Deserialize, Clone)]
pub struct MimeTypeMapping {
    extension: String,
    mimetype: String,
    handler: String,
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
        if let Ok(c_message) = CString::new(format!(
            "ox_content: new: Initializing with content_root: {:?}",
            config.content_root
        )) {
            let module_name = CString::new(MODULE_NAME).unwrap();
            unsafe { (api.log_callback)(LogLevel::Debug, module_name.as_ptr(), c_message.as_ptr()); }
        }

        let mimetype_file_name = &config.mimetypes_file;
        let mimetype_config: MimeTypeConfig = match fs::read_to_string(mimetype_file_name) {
            Ok(content) => match serde_yaml::from_str::<MimeTypeConfig>(&content) {
                Ok(cfg) => cfg,
                Err(e) => {
                    if let Ok(c_message) = CString::new(format!(
                        "ox_content: Failed to parse mimetype config file {}: {}",
                        mimetype_file_name, e
                    )) {
                        let module_name = CString::new(MODULE_NAME).unwrap();
                        unsafe { (api.log_callback)(LogLevel::Error, module_name.as_ptr(), c_message.as_ptr()); }
                    }
                    anyhow::bail!("Failed to parse mimetype config: {}", e);
                }
            },
            Err(e) => {
                if let Ok(c_message) = CString::new(format!(
                    "ox_content: Failed to read mimetype config file {}: {}",
                    mimetype_file_name, e
                )) {
                    let module_name = CString::new(MODULE_NAME).unwrap();
                    unsafe { (api.log_callback)(LogLevel::Error, module_name.as_ptr(), c_message.as_ptr()); }
                }
                anyhow::bail!("Failed to read mimetype config: {}", e);
            }
        };

        Ok(Self {
            content_root: PathBuf::from(config.content_root.clone()),
            mimetypes: mimetype_config.mimetypes,
            default_documents: config.default_documents.clone(),
            content_config: config,
            api,
        })
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        self.log(LogLevel::Debug, "ox_content: process_request called.".to_string());

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

        let request_path_ptr = unsafe { (self.api.get_request_path)(pipeline_state, arena_ptr, self.api.alloc_str) };
        let request_path = unsafe { CStr::from_ptr(request_path_ptr).to_str().unwrap_or("/") };

        if request_path == "/error_test" {
            unsafe { (self.api.set_response_status)(pipeline_state, 500); }
            return HandlerResult::ModifiedJumpToError;
        }

        if let Some(file_path) = self.resolve_and_find_file(request_path) {
            let extension = file_path.extension().and_then(|s| s.to_str()).unwrap_or("");
            let mimetype_mapping = self.mimetypes.iter().find(|m| m.extension == extension);

            let handler_result = if let Some(mapping) = mimetype_mapping {
                match mapping.handler.as_str() {
                    "stream" => {
                        handlers::stream_handler::stream_handler(file_path.clone(), &mapping.mimetype)
                    }
                    "template" => handlers::template_handler::template_handler(
                        file_path.clone(),
                        &mapping.mimetype,
                    ),
                    _ => Err(format!("Unsupported handler for file: {}", mapping.handler)),
                }
            } else {
                handlers::stream_handler::stream_handler(
                    file_path.clone(),
                    "application/octet-stream",
                )
            };

            match handler_result {
                Ok((response_body, mimetype)) => {
                    self.log(
                        LogLevel::Debug,
                        format!(
                            "ox_content: Successfully handled request for path: {}",
                            request_path
                        ),
                    );
                    unsafe {
                        let c_content_type_key = CString::new("Content-Type").unwrap();
                        let c_content_type_value = CString::new(mimetype).unwrap();
                        (self.api.set_response_header)(
                            pipeline_state,
                            c_content_type_key.as_ptr(),
                            c_content_type_value.as_ptr(),
                        );
                        (self.api.set_response_body)(
                            pipeline_state,
                            response_body.as_ptr(),
                            response_body.len(),
                        );
                    }
                    HandlerResult::ModifiedContinue
                }
                Err(error_message) => {
                    self.log(
                        LogLevel::Error,
                        format!(
                            "ox_content: Error handling request for path {}: {}",
                            request_path, error_message
                        ),
                    );
                    unsafe { (self.api.set_response_status)(pipeline_state, 500); }
                    HandlerResult::ModifiedJumpToError
                }
            }
        } else {
            self.log(
                LogLevel::Debug,
                format!("ox_content: File not found for path: {}", request_path),
            );
            unsafe {
                (self.api.set_response_status)(pipeline_state, 404);
                let c_body = CString::new("404 Not Found").unwrap();
                let c_content_type_key = CString::new("Content-Type").unwrap();
                let c_content_type_value = CString::new("text/plain").unwrap();
                (self.api.set_response_header)(pipeline_state, c_content_type_key.as_ptr(), c_content_type_value.as_ptr());
                let c_content_length_key = CString::new("Content-Length").unwrap();
                let c_content_length_value = CString::new(c_body.as_bytes().len().to_string()).unwrap();
                (self.api.set_response_header)(pipeline_state, c_content_length_key.as_ptr(), c_content_length_value.as_ptr());
                (self.api.set_response_body)(pipeline_state, c_body.as_ptr().cast(), c_body.as_bytes().len());
            }
            HandlerResult::ModifiedContinue
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
    api_ptr: *const WebServiceApiV1,
) -> *mut ModuleInterface {
    let result = panic::catch_unwind(|| {
        let api_instance = unsafe { &*api_ptr };
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

        let contents = match fs::read_to_string(config_file_name) {
            Ok(c) => c,
            Err(e) => {
                let log_msg = CString::new(format!("Failed to read config file '{}': {}", config_file_name, e)).unwrap();
                let module_name = CString::new(MODULE_NAME).unwrap();
                let _ = panic::catch_unwind(|| {
                    unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                });
                return std::ptr::null_mut();
            }
        };

        let config: ContentConfig = match serde_yaml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                let log_msg = CString::new(format!("Failed to deserialize ContentConfig: {}", e)).unwrap();
                let module_name = CString::new(MODULE_NAME).unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                return std::ptr::null_mut();
            }
        };

        let handler = match OxModule::new(config, api_instance) {
            Ok(h) => {
                let log_msg = CString::new("LOGGING TEST: ox_content initialized").unwrap();
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
        });

        Box::into_raw(module_interface)
    });

    match result {
        Ok(ptr) => ptr,
        Err(e) => {
            // Cannot safely log here as we might not have the api pointer.
            eprintln!("Panic during module initialization: {:?}", e);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn process_request_c(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena: *const c_void, 
) -> HandlerResult {
    let result = panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let handler = unsafe { &*(instance_ptr as *mut OxModule) };
        handler.process_request(pipeline_state_ptr)
    }));

    match result {
        Ok(handler_result) => handler_result,
        Err(e) => {
            let log_msg =
                CString::new(format!("Panic occurred in process_request_c: {:?}.", e)).unwrap();
            let module_name = CString::new(MODULE_NAME).unwrap();
            unsafe { (log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
            HandlerResult::ModifiedJumpToError
        }
    }
}
