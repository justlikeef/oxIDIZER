use libc::{c_char, c_void};
use ox_webservice_api::{ModuleConfig, SendableWebServiceHandler, InitializationData};
use serde::Deserialize;
use std::ffi::{CStr, CString};
use std::fs;
use std::path::{PathBuf};
use log::{debug, error};

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
    urls: Option<Vec<UrlConfig>>,
}

pub struct ModuleState {
    pub content_root: PathBuf,
    pub mimetypes: Vec<MimeTypeMapping>,
    pub default_documents: Vec<DocumentConfig>,
    pub render_template_fn: unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char,
    pub module_config: ModuleConfig,
    pub content_config: ContentConfig,
}

static mut MODULE_STATE: Option<ModuleState> = None;

// ... (rest of the file)

#[no_mangle]
pub extern "C" fn initialize_module(module_config_ptr: *mut c_void, render_template_fn_ptr: unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char) -> SendableWebServiceHandler {
    debug!("ox_content: Entering initialize_module function.");
    let module_config = unsafe { &*(module_config_ptr as *mut ModuleConfig) };
    let params = module_config.params.as_ref().unwrap();

    debug!("ox_content: Initializing module...");
    debug!("ox_content: Config file: {}", params.get("config_file").and_then(|v| v.as_str()).unwrap_or("ox_content.yaml"));

    let config_file_name = params.get("config_file").and_then(|v| v.as_str()).unwrap_or("ox_content.yaml");

    debug!("ox_content: Attempting to read content config file: {}", config_file_name);
    let content_config = match fs::read_to_string(config_file_name) {
        Ok(content) => match serde_yaml::from_str::<ContentConfig>(&content) {
            Ok(config) => config,
            Err(e) => {
                error!("ox_content: Failed to parse content config file {}: {}", config_file_name, e);
                panic!("Failed to parse content config file: {}", e);
            }
        },
        Err(e) => {
            error!("ox_content: Failed to read content config file {}: {}", config_file_name, e);
            panic!("Failed to read content config file: {}", e);
        }
    };
    let mimetype_file_name = &content_config.mimetypes_file;


    let mimetype_config = match fs::read_to_string(mimetype_file_name) {
        Ok(content) => match serde_yaml::from_str::<MimeTypeConfig>(&content) {
            Ok(config) => config,
            Err(e) => {
                error!("ox_content: Failed to parse mimetype config file {}: {}", mimetype_file_name, e);
                panic!("Failed to parse mimetype config file: {}", e);
            }
        },
        Err(e) => {
            error!("ox_content: Failed to read mimetype config file {}: {}", mimetype_file_name, e);
            panic!("Failed to read mimetype config file: {}", e);
        }
    };

    debug!("ox_content: Content root: {:?}", content_config.content_root);
    debug!("ox_content: Mimetypes: {:?}", mimetype_config.mimetypes);
    debug!("ox_content: Default documents: {:?}", content_config.default_documents);

    unsafe {
        MODULE_STATE = Some(ModuleState {
            content_root: PathBuf::from(content_config.content_root.clone()),
            mimetypes: mimetype_config.mimetypes,
            default_documents: content_config.default_documents.clone(),
            render_template_fn: render_template_fn_ptr,
            module_config: module_config.clone(),
            content_config: content_config.clone(),
        });
    }

    SendableWebServiceHandler(content_handler)
}


fn resolve_and_find_file(state: &ModuleState, request_path: &str) -> Option<PathBuf> {
    let mut file_path = state.content_root.clone();
    file_path.push(request_path.trim_start_matches('/'));

    if !file_path.exists() {
        return None;
    }

    // Prevent directory traversal attacks
    if let Ok(canonical_path) = file_path.canonicalize() {
        if !canonical_path.starts_with(&state.content_root) {
            return None; // Path is outside the content root
        }
        file_path = canonical_path;
    } else {
        return None; // Path does not exist or other error
    }

    if file_path.is_dir() {
        for doc_config in &state.default_documents {
            let mut default_doc_candidate = file_path.clone();
            default_doc_candidate.push(&doc_config.document);
            if default_doc_candidate.exists() {
                return Some(default_doc_candidate);
            }
        }
        None // No default document found
    } else {
        Some(file_path)
    }
}

extern "C" fn content_handler(request_ptr: *mut c_char) -> *mut c_char {
    debug!("content_handler called");
    let request_str = unsafe { CStr::from_ptr(request_ptr).to_str().unwrap() };
    let request: serde_json::Value = serde_json::from_str(request_str).unwrap();
    let path = request.get("path").and_then(|v| v.as_str()).unwrap_or("");

    let state = unsafe { MODULE_STATE.as_ref().unwrap() };

    if path == "/error_test" {
        let mut module_config = state.module_config.clone();
        module_config.error_path = Some(path.to_string());
        let parsed_config_value = serde_json::to_value(&state.content_config).unwrap_or(serde_json::Value::Null);
        if let Some(params) = module_config.params.as_mut() {
            if let Some(map) = params.as_object_mut() {
                map.insert("parsed_config".to_string(), parsed_config_value);
            }
        }
        let module_context_json = serde_json::to_string(&module_config).unwrap_or_else(|_| "null".to_string());
        let error_response = serde_json::json!({
            "status": 500,
            "message": "Simulated error from ox_content",
            "context": path,
            "module_context": module_context_json,
            "headers": {
                "Content-Type": "text/html"
            }
        });
        debug!("ox_content: Returning simulated error response for /error_test: {}", error_response.to_string());
        return CString::new(error_response.to_string()).unwrap().into_raw();
    }

    if let Some(file_path) = resolve_and_find_file(state, path) {
        let extension = file_path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let mimetype_mapping = state.mimetypes.iter().find(|m| m.extension == extension);

        let handler_result = if let Some(mapping) = mimetype_mapping {
            match mapping.handler.as_str() {
                "stream" => handlers::stream_handler::stream_handler(file_path.clone(), &mapping.mimetype),
                "template" => handlers::template_handler::template_handler(file_path.clone(), &mapping.mimetype, state.render_template_fn),
                _ => Err(format!("Unsupported handler for file: {}", mapping.handler)),
            }
        } else {
            // If no mimetype mapping is found, default to the stream handler with a generic mimetype.
            handlers::stream_handler::stream_handler(file_path.clone(), "application/octet-stream")
        };

        match handler_result {
            Ok(response_string) => {
                debug!("content_handler: Returning successful response: {}", response_string);
                CString::new(response_string).unwrap().into_raw()
            },
            Err(error_message) => {
                let mut module_config = state.module_config.clone();
                module_config.error_path = Some(file_path.to_str().unwrap_or("unknown").to_string());
                let parsed_config_value = serde_json::to_value(&state.content_config).unwrap_or(serde_json::Value::Null);
                if let Some(params) = module_config.params.as_mut() {
                    if let Some(map) = params.as_object_mut() {
                        map.insert("parsed_config".to_string(), parsed_config_value);
                    }
                }
                let module_context_json = serde_json::to_string(&module_config).unwrap_or_else(|_| "null".to_string());
                let error_response = serde_json::json!({
                    "status": 500,
                    "message": error_message,
                    "context": file_path.to_str().unwrap_or("unknown"),
                    "module_context": module_context_json,
                    "headers": {
                        "Content-Type": "text/html"
                    }
                });
                debug!("content_handler: Returning error response: {}", error_response.to_string());
                CString::new(error_response.to_string()).unwrap().into_raw()
            }
        }
    } else {
        let mut module_config = state.module_config.clone();
        module_config.error_path = Some(path.to_string());
        let parsed_config_value = serde_json::to_value(&state.content_config).unwrap_or(serde_json::Value::Null);
        if let Some(params) = module_config.params.as_mut() {
            if let Some(map) = params.as_object_mut() {
                map.insert("parsed_config".to_string(), parsed_config_value);
            }
        }
        let module_context_json = serde_json::to_string(&module_config).unwrap_or_else(|_| "null".to_string());
        let error_response = serde_json::json!({
            "status": 404,
            "message": "Not Found",
            "context": path,
            "module_name": "ox_content",
            "module_context": module_context_json,
            "headers": {
                "Content-Type": "text/html"
            }
        });
        debug!("content_handler: Returning 404 response: {}", error_response.to_string());
        CString::new(error_response.to_string()).unwrap().into_raw()
    }
}