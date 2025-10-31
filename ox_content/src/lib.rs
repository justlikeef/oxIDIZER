use libc::{c_char, c_void};
use ox_webservice_api::{InitializationData, ModuleEndpoint, ModuleEndpoints, WebServiceContext};
use serde::Deserialize;
use std::ffi::CStr;
use std::fs;
use std::path::PathBuf;
use log::debug;

mod handlers;

#[derive(Debug, Deserialize)]
struct ContentConfig {
    content_root: String,
    #[serde(default = "default_error_path")]
    error_path: String,
}

fn default_error_path() -> String {
    "www/error".to_string()
}

#[derive(Debug, Deserialize)]
struct MimeTypeConfig {
    mimetypes: Vec<MimeTypeMapping>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MimeTypeMapping {
    extension: String,
    mimetype: String,
    handler: String,
}

pub struct ModuleState {
    pub content_root: PathBuf,
    pub mimetypes: Vec<MimeTypeMapping>,
    pub webservice_context: WebServiceContext,
    pub render_template_fn: Option<unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char>,
    pub error_path: PathBuf,
}

static mut MODULE_STATE: Option<ModuleState> = None;

#[no_mangle]
pub extern "C" fn initialize_module(init_data_ptr: *mut c_char) -> *mut c_void {
    let init_data_str = unsafe { CStr::from_ptr(init_data_ptr).to_str().unwrap() };
    let init_data: InitializationData = serde_json::from_str(init_data_str).unwrap();

    let config_file = init_data.params.get("config_file").and_then(|v| v.as_str()).unwrap_or("ox_content.yaml");
    let mimetypes_file = init_data.params.get("mimetypes_file").and_then(|v| v.as_str()).unwrap_or("mimetypes.yaml");

    debug!("ox_content: Initializing module...");
    debug!("ox_content: Config file: {}", config_file);
    debug!("ox_content: Mimetypes file: {}", mimetypes_file);

    let content_config: ContentConfig = serde_yaml::from_str(&fs::read_to_string(config_file).unwrap()).unwrap();
    let mimetype_config: MimeTypeConfig = serde_yaml::from_str(&fs::read_to_string(mimetypes_file).unwrap()).unwrap();

    debug!("ox_content: Content root: {:?}", content_config.content_root);
    debug!("ox_content: Mimetypes: {:?}", mimetype_config.mimetypes);

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
            render_template_fn: init_data.context.render_template_fn,
            error_path: PathBuf::from(error_path_str),
        });
    }

    let endpoints = vec![ModuleEndpoint {
        path: "*".to_string(),
        handler: content_handler,
        priority: 1000, // A high priority to act as a fallback
    }];

    let boxed_endpoints = Box::new(ModuleEndpoints { endpoints });
    Box::into_raw(boxed_endpoints) as *mut c_void
}

extern "C" fn content_handler(request_ptr: *mut c_char) -> *mut c_char {
    println!("DEBUG: content_handler called");
    let request_str = unsafe { CStr::from_ptr(request_ptr).to_str().unwrap() };
    let request: serde_json::Value = serde_json::from_str(request_str).unwrap();
    let path = request.get("path").and_then(|v| v.as_str()).unwrap_or("");

    let state = unsafe { MODULE_STATE.as_ref().unwrap() };

    let mut file_path = state.content_root.clone();
    file_path.push(path.trim_start_matches('/'));

    if file_path.is_dir() {
        let index_found = false;
        for mapping in &state.mimetypes {
            let mut index_file_candidate = file_path.clone();
            index_file_candidate.push(format!("index.{}", mapping.extension));
            if index_file_candidate.exists() {
                return match mapping.handler.as_str() {
                    "stream" => handlers::stream_handler::stream_handler(index_file_candidate, &mapping.mimetype),
                    "template" => handlers::template_handler::template_handler(index_file_candidate, &mapping.mimetype),
                    _ => handlers::stream_handler::not_found_handler(),
                };
            }
        }
        // The `index_found` variable is not used after this point, so no need to set it.
        // If no index file is found, we proceed to the next block or return not_found_handler.
        return handlers::stream_handler::not_found_handler();
    }

    let extension = file_path.extension().and_then(|s| s.to_str()).unwrap_or("");

    let mimetype_mapping = state.mimetypes.iter().find(|m| m.extension == extension);

    if let Some(mapping) = mimetype_mapping {
        match mapping.handler.as_str() {
            "stream" => handlers::stream_handler::stream_handler(file_path, &mapping.mimetype),
            "template" => handlers::template_handler::template_handler(file_path, &mapping.mimetype),
            _ => handlers::stream_handler::not_found_handler(),
        }
    } else {
        handlers::stream_handler::not_found_handler()
    }
}