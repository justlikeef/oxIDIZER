use libc::{c_char, c_void};
use ox_webservice_api::{InitializationData, ModuleEndpoint, ModuleEndpoints, WebServiceContext, SendableWebServiceHandler};
use serde::Deserialize;
use std::ffi::{CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};
use log::{debug, error};
use libloading::{Library, Symbol};
use std::sync::Arc;

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

fn default_error_path() -> String {
    "www/error".to_string()
}

#[derive(Debug, Deserialize)]
struct UrlConfig {
    url: String,
}

#[derive(Debug, Deserialize)]
struct DocumentConfig {
    document: String,
}

#[derive(Debug, Deserialize)]
struct ContentConfig {
    content_root: String,
    #[serde(default = "default_error_path")]
    error_path: String,
    #[serde(default)]
    default_documents: Vec<DocumentConfig>,
    urls: Option<Vec<UrlConfig>>,
}

pub struct ModuleState {
    pub content_root: PathBuf,
    pub mimetypes: Vec<MimeTypeMapping>,
    pub webservice_context: WebServiceContext,
    pub error_path: PathBuf,
    pub default_documents: Vec<DocumentConfig>,
    pub render_template_fn: unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char,
}

static mut MODULE_STATE: Option<ModuleState> = None;

#[no_mangle]
pub extern "C" fn initialize_module(init_data_ptr: *mut c_char, render_template_fn_ptr: unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char) -> *mut c_void {
    let init_data_str = unsafe { CStr::from_ptr(init_data_ptr).to_str().unwrap() };
    let init_data: InitializationData = serde_json::from_str(init_data_str).unwrap();

    debug!("ox_content: Initializing module...");
    debug!("ox_content: Config file: {}", init_data.params.get("config_file").and_then(|v| v.as_str()).unwrap_or("ox_content.yaml"));
    debug!("ox_content: Mimetypes file: {}", init_data.params.get("mimetypes_file").and_then(|v| v.as_str()).unwrap_or("mimetypes.yaml"));

    let config_file = init_data.params.get("config_file").and_then(|v| v.as_str()).unwrap_or("ox_content.yaml");
    let mimetype_file = init_data.params.get("mimetypes_file").and_then(|v| v.as_str()).unwrap_or("mimetypes.yaml");

    let content_config: ContentConfig = serde_yaml::from_str(&fs::read_to_string(config_file).unwrap()).unwrap();
    let mimetype_config: MimeTypeConfig = serde_yaml::from_str(&fs::read_to_string(mimetype_file).unwrap()).unwrap();

    debug!("ox_content: Content root: {:?}", content_config.content_root);
    debug!("ox_content: Mimetypes: {:?}", mimetype_config.mimetypes);
    debug!("ox_content: Default documents: {:?}", content_config.default_documents);

    let error_path_str = init_data.params.get("error_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| default_error_path());

    debug!("ox_content: Error path: {}", error_path_str);

    unsafe {
        MODULE_STATE = Some(ModuleState {
            content_root: PathBuf::from(content_config.content_root),
            mimetypes: mimetype_config.mimetypes,
            webservice_context: init_data.context.clone(),
            error_path: PathBuf::from(error_path_str),
            default_documents: content_config.default_documents,
            render_template_fn: render_template_fn_ptr,
        });
    }

    let mut endpoints = Vec::new();

    if let Some(urls) = content_config.urls {
        for url_config in urls {
            endpoints.push(ModuleEndpoint {
                path: url_config.url,
                handler: SendableWebServiceHandler(content_handler),
                priority: 1000, // Default priority, can be made configurable later
            });
        }
    } else {
        // Default to wildcard if no URLs are specified
        endpoints.push(ModuleEndpoint {
            path: "*".to_string(),
            handler: SendableWebServiceHandler(content_handler),
            priority: 1000, // Default priority
        });
    }

    let boxed_endpoints = Box::new(ModuleEndpoints { endpoints });
    Box::into_raw(boxed_endpoints) as *mut c_void
}

extern "C" fn content_handler(request_ptr: *mut c_char) -> *mut c_char {
    debug!("content_handler called");
    let request_str = unsafe { CStr::from_ptr(request_ptr).to_str().unwrap() };
    let request: serde_json::Value = serde_json::from_str(request_str).unwrap();
    let path = request.get("path").and_then(|v| v.as_str()).unwrap_or("");

    let state = unsafe { MODULE_STATE.as_ref().unwrap() };

    let mut file_path = state.content_root.clone();
    file_path.push(path.trim_start_matches('/'));

    let handler_result = if file_path.is_dir() {
        let mut result = Err("Directory not found or no default document.".to_string());
        for doc_config in &state.default_documents {
            let mut default_doc_candidate = file_path.clone();
            default_doc_candidate.push(&doc_config.document);
            if default_doc_candidate.exists() {
                let extension = default_doc_candidate.extension().and_then(|s| s.to_str()).unwrap_or("");
                let mimetype_mapping = state.mimetypes.iter().find(|m| m.extension == extension);

                if let Some(mapping) = mimetype_mapping {
                    result = match mapping.handler.as_str() {
                        "stream" => handlers::stream_handler::stream_handler(default_doc_candidate, &mapping.mimetype),
                        "template" => handlers::template_handler::template_handler(default_doc_candidate, &mapping.mimetype, state.render_template_fn),
                        _ => Err(format!("Unsupported handler for default document: {}", mapping.handler)),
                    };
                    if result.is_ok() { break; }
                } else {
                    result = Err(format!("No mimetype mapping found for default document: {}", doc_config.document));
                }
            }
        }
        result
    } else {
        let extension = file_path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let mimetype_mapping = state.mimetypes.iter().find(|m| m.extension == extension);

        if let Some(mapping) = mimetype_mapping {
            match mapping.handler.as_str() {
                "stream" => handlers::stream_handler::stream_handler(file_path.clone(), &mapping.mimetype),
                "template" => handlers::template_handler::template_handler(file_path.clone(), &mapping.mimetype, state.render_template_fn),
                _ => Err(format!("Unsupported handler for file: {}", mapping.handler)),
            }
        } else {
            handlers::stream_handler::not_found_handler()
        }
    };

    match handler_result {
        Ok(response_string) => {
            debug!("content_handler: Returning successful response: {}", response_string);
            CString::new(response_string).unwrap().into_raw()
        },
        Err(error_message) => {
            let error_response = serde_json::json!({
                "status": 404,
                "message": error_message,
                "context": file_path.to_str().unwrap_or("unknown"),
                "headers": {
                    "Content-Type": "text/html"
                }
            });
            debug!("content_handler: Returning error response: {}", error_response.to_string());
            CString::new(error_response.to_string()).unwrap().into_raw()
        }
    }
}