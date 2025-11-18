use libc::{c_char, c_void};
use ox_webservice_api::{
    ModuleInterface, LogCallback, LogLevel, RequestContext, HandlerResult,
    GetRequestMethodFn, GetRequestPathFn, GetRequestQueryFn, GetRequestHeaderFn, GetRequestHeadersFn,
    GetRequestBodyFn, GetSourceIpFn, SetRequestPathFn, SetRequestHeaderFn, SetSourceIpFn, GetResponseStatusFn,
    GetResponseHeaderFn, SetResponseStatusFn, SetResponseHeaderFn, SetResponseBodyFn,
};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{CStr, CString};
use std::fs;
use std::path::{PathBuf};
use std::panic;

// --- Static FFI Function Pointers ---
static mut LOGGER_CALLBACK: Option<LogCallback> = None;
static mut GET_REQUEST_METHOD_FN: Option<GetRequestMethodFn> = None;
static mut GET_REQUEST_PATH_FN: Option<GetRequestPathFn> = None;
static mut GET_REQUEST_QUERY_FN: Option<GetRequestQueryFn> = None;
static mut GET_REQUEST_HEADER_FN: Option<GetRequestHeaderFn> = None;
static mut GET_REQUEST_HEADERS_FN: Option<GetRequestHeadersFn> = None;
static mut GET_REQUEST_BODY_FN: Option<GetRequestBodyFn> = None;
static mut GET_SOURCE_IP_FN: Option<GetSourceIpFn> = None;
static mut SET_REQUEST_PATH_FN: Option<SetRequestPathFn> = None;
static mut SET_REQUEST_HEADER_FN: Option<SetRequestHeaderFn> = None;
static mut SET_SOURCE_IP_FN: Option<SetSourceIpFn> = None;
static mut GET_RESPONSE_STATUS_FN: Option<GetResponseStatusFn> = None;
static mut GET_RESPONSE_HEADER_FN: Option<GetResponseHeaderFn> = None;
static mut SET_RESPONSE_STATUS_FN: Option<SetResponseStatusFn> = None;
static mut SET_RESPONSE_HEADER_FN: Option<SetResponseHeaderFn> = None;
static mut SET_RESPONSE_BODY_FN: Option<SetResponseBodyFn> = None;

macro_rules! module_log {
    ($level:expr, $($arg:tt)*) => ({
        if let Some(log_cb) = unsafe { LOGGER_CALLBACK } {
            let message = format!($($arg)*);
            if let Ok(c_message) = CString::new(message) {
                unsafe {
                    log_cb($level, c_message.as_ptr());
                }
            }
        }
    });
}

mod handlers;

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
struct UrlConfig {
    url: String,
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

pub struct ContentModule {
    pub content_root: PathBuf,
    pub mimetypes: Vec<MimeTypeMapping>,
    pub default_documents: Vec<DocumentConfig>,
    pub render_template_fn: unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char,
    pub content_config: ContentConfig,
}

impl ContentModule {
    pub fn new(config: ContentConfig, render_template_fn: unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char) -> anyhow::Result<Self> {
        module_log!(LogLevel::Debug, "ox_content: new: Initializing with content_root: {:?}", config.content_root);

        let mimetype_file_name = &config.mimetypes_file;
        let mimetype_config: MimeTypeConfig = match fs::read_to_string(mimetype_file_name) {
            Ok(content) => match serde_yaml::from_str::<MimeTypeConfig>(&content) {
                Ok(cfg) => cfg,
                Err(e) => {
                    module_log!(LogLevel::Error, "ox_content: Failed to parse mimetype config file {}: {}", mimetype_file_name, e);
                    anyhow::bail!("Failed to parse mimetype config: {}", e);
                }
            },
            Err(e) => {
                module_log!(LogLevel::Error, "ox_content: Failed to read mimetype config file {}: {}", mimetype_file_name, e);
                anyhow::bail!("Failed to read mimetype config: {}", e);
            }
        };

        Ok(Self {
            content_root: PathBuf::from(config.content_root.clone()),
            mimetypes: mimetype_config.mimetypes,
            default_documents: config.default_documents.clone(),
            render_template_fn,
            content_config: config,
        })
    }

    pub fn process_request(&self, context: &mut RequestContext) -> HandlerResult {
        module_log!(LogLevel::Debug, "ox_content: process_request called.");

        let request_path_ptr = unsafe { GET_REQUEST_PATH_FN.unwrap()(context) };
        let request_path = unsafe { CStr::from_ptr(request_path_ptr).to_str().unwrap_or("/") };

        if request_path == "/error_test" {
            unsafe { SET_RESPONSE_STATUS_FN.unwrap()(context, 500); }
            return HandlerResult::ModifiedJumpToError;
        }

        if let Some(file_path) = self.resolve_and_find_file(request_path) {
            let extension = file_path.extension().and_then(|s| s.to_str()).unwrap_or("");
            let mimetype_mapping = self.mimetypes.iter().find(|m| m.extension == extension);

            let handler_result = if let Some(mapping) = mimetype_mapping {
                match mapping.handler.as_str() {
                    "stream" => handlers::stream_handler::stream_handler(file_path.clone(), &mapping.mimetype),
                    "template" => handlers::template_handler::template_handler(file_path.clone(), &mapping.mimetype, self.render_template_fn, &self.content_root),
                    _ => Err(format!("Unsupported handler for file: {}", mapping.handler)),
                }
            } else {
                handlers::stream_handler::stream_handler(file_path.clone(), "application/octet-stream")
            };

            match handler_result {
                Ok((response_body, mimetype)) => {
                    module_log!(LogLevel::Debug, "ox_content: Successfully handled request for path: {}", request_path);
                    unsafe {
                        if let (Some(set_body_fn), Some(set_header_fn)) = (SET_RESPONSE_BODY_FN, SET_RESPONSE_HEADER_FN) {
                            let c_content_type_key = CString::new("Content-Type").unwrap();
                            let c_content_type_value = CString::new(mimetype).unwrap();
                            set_header_fn(context, c_content_type_key.as_ptr(), c_content_type_value.as_ptr());
                            set_body_fn(context, response_body.as_ptr(), response_body.len());
                        }
                    }
                    HandlerResult::ModifiedContinue
                },
                Err(error_message) => {
                    module_log!(LogLevel::Error, "ox_content: Error handling request for path {}: {}", request_path, error_message);
                    unsafe { SET_RESPONSE_STATUS_FN.unwrap()(context, 500); }
                    HandlerResult::ModifiedJumpToError
                }
            }
        } else {
            module_log!(LogLevel::Debug, "ox_content: File not found for path: {}", request_path);
            unsafe { SET_RESPONSE_STATUS_FN.unwrap()(context, 404); }
            HandlerResult::UnmodifiedContinue // Let the pipeline handle the 404
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
    render_template_ffi: unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char,
    log_callback: LogCallback,
    // Request Getters
    get_request_method_fn: GetRequestMethodFn,
    get_request_path_fn: GetRequestPathFn,
    get_request_query_fn: GetRequestQueryFn,
    get_request_header_fn: GetRequestHeaderFn,
    get_request_headers_fn: GetRequestHeadersFn,
    get_request_body_fn: GetRequestBodyFn,
    get_source_ip_fn: GetSourceIpFn,
    // Request Setters
    set_request_path_fn: SetRequestPathFn,
    set_request_header_fn: SetRequestHeaderFn,
    set_source_ip_fn: SetSourceIpFn,
    // Response Getters
    get_response_status_fn: GetResponseStatusFn,
    get_response_header_fn: GetResponseHeaderFn,
    // Response Setters
    set_response_status_fn: SetResponseStatusFn,
    set_response_header_fn: SetResponseHeaderFn,
    set_response_body_fn: SetResponseBodyFn,
) -> *mut ModuleInterface {
    unsafe {
        LOGGER_CALLBACK = Some(log_callback);
        // Request Getters
        GET_REQUEST_METHOD_FN = Some(get_request_method_fn);
        GET_REQUEST_PATH_FN = Some(get_request_path_fn);
        GET_REQUEST_QUERY_FN = Some(get_request_query_fn);
        GET_REQUEST_HEADER_FN = Some(get_request_header_fn);
        GET_REQUEST_HEADERS_FN = Some(get_request_headers_fn);
        GET_REQUEST_BODY_FN = Some(get_request_body_fn);
        GET_SOURCE_IP_FN = Some(get_source_ip_fn);
        // Request Setters
        SET_REQUEST_PATH_FN = Some(set_request_path_fn);
        SET_REQUEST_HEADER_FN = Some(set_request_header_fn);
        SET_SOURCE_IP_FN = Some(set_source_ip_fn);
        // Response Getters
        GET_RESPONSE_STATUS_FN = Some(get_response_status_fn);
        GET_RESPONSE_HEADER_FN = Some(get_response_header_fn);
        // Response Setters
        SET_RESPONSE_STATUS_FN = Some(set_response_status_fn);
        SET_RESPONSE_HEADER_FN = Some(set_response_header_fn);
        SET_RESPONSE_BODY_FN = Some(set_response_body_fn);
    }

    let result = panic::catch_unwind(|| {
        let module_params_json = unsafe { CStr::from_ptr(module_params_json_ptr).to_str().unwrap() };
        let params: Value = serde_json::from_str(module_params_json)
            .expect("Failed to parse module params JSON");

        let config_file_name = match params.get("config_file").and_then(|v| v.as_str()) {
            Some(name) => name,
            None => {
                module_log!(LogLevel::Error, "'config_file' parameter is missing or not a string.");
                return std::ptr::null_mut();
            }
        };

        let contents = match fs::read_to_string(config_file_name) {
            Ok(c) => c,
            Err(e) => {
                module_log!(LogLevel::Error, "Failed to read config file '{}': {}", config_file_name, e);
                return std::ptr::null_mut();
            }
        };

        let config: ContentConfig = match serde_yaml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                module_log!(LogLevel::Error, "Failed to deserialize ContentConfig: {}", e);
                return std::ptr::null_mut();
            }
        };

        let handler = match ContentModule::new(config, render_template_ffi) {
            Ok(h) => h,
            Err(e) => {
                module_log!(LogLevel::Error, "Failed to create ContentModule: {}", e);
                return std::ptr::null_mut();
            }
        };

        let instance_ptr = Box::into_raw(Box::new(handler)) as *mut c_void;

        let module_interface = Box::new(ModuleInterface {
            instance_ptr,
            handler_fn: process_request_c,
        });

        Box::into_raw(module_interface)
    });

    match result {
        Ok(ptr) => ptr,
        Err(e) => {
            module_log!(LogLevel::Error, "Panic occurred during module initialization: {:?}.", e);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn process_request_c(instance_ptr: *mut c_void, context_ptr: *mut RequestContext) -> HandlerResult {
    let result = panic::catch_unwind(|| {
        let handler = unsafe { &*(instance_ptr as *mut ContentModule) };
        let context = unsafe { &mut *context_ptr };
        handler.process_request(context)
    });

    match result {
        Ok(handler_result) => handler_result,
        Err(e) => {
            module_log!(LogLevel::Error, "Panic occurred in process_request_c: {:?}.", e);
            HandlerResult::ModifiedJumpToError
        }
    }
}