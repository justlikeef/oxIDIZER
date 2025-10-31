use libc::{c_char, c_void};
use ox_webservice_api::{InitializationData, ModuleEndpoint, ModuleEndpoints, WebServiceHandler, WebServiceContext};
use serde::Deserialize;
use std::ffi::{CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};

mod handlers;

#[derive(Debug, Deserialize)]
struct ContentConfig {
    content_root: String,
}

#[derive(Debug, Deserialize)]
struct MimeTypeConfig {
    mimetypes: Vec<MimeTypeMapping>,
}

#[derive(Debug, Deserialize, Clone)]
struct MimeTypeMapping {
    extension: String,
    mimetype: String,
    handler: String,
}

pub struct ModuleState {
    pub content_root: PathBuf,
    pub mimetypes: Vec<MimeTypeMapping>,
    pub webservice_context: WebServiceContext,
}

static mut MODULE_STATE: Option<ModuleState> = None;

#[no_mangle]
pub extern "C" fn initialize_module(init_data_ptr: *mut c_char) -> *mut c_void {
    let init_data_str = unsafe { CStr::from_ptr(init_data_ptr).to_str().unwrap() };
    let init_data: InitializationData = serde_json::from_str(init_data_str).unwrap();

    let config_file = init_data.params.get("config_file").and_then(|v| v.as_str()).unwrap_or("ox_content.yaml");
    let mimetypes_file = init_data.params.get("mimetypes_file").and_then(|v| v.as_str()).unwrap_or("mimetypes.yaml");

    println!("ox_content: Initializing module...");
    println!("ox_content: Config file: {}", config_file);
    println!("ox_content: Mimetypes file: {}", mimetypes_file);

    let content_config: ContentConfig = serde_yaml::from_str(&fs::read_to_string(config_file).unwrap()).unwrap();
    let mimetype_config: MimeTypeConfig = serde_yaml::from_str(&fs::read_to_string(mimetypes_file).unwrap()).unwrap();

    println!("ox_content: Content root: {:?}", content_config.content_root);
    println!("ox_content: Mimetypes: {:?}", mimetype_config.mimetypes);

    unsafe {
        MODULE_STATE = Some(ModuleState {
            content_root: PathBuf::from(content_config.content_root),
            mimetypes: mimetype_config.mimetypes,
            webservice_context: init_data.context,
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
    let request_str = unsafe { CStr::from_ptr(request_ptr).to_str().unwrap() };
    let request: serde_json::Value = serde_json::from_str(request_str).unwrap();
    let path = request.get("path").and_then(|v| v.as_str()).unwrap_or("");

    let state = unsafe { MODULE_STATE.as_ref().unwrap() };

    let mut file_path = state.content_root.clone();
    file_path.push(path.trim_start_matches('/'));

    if file_path.is_dir() {
        let mut index_found = false;
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
        if !index_found {
            return handlers::stream_handler::not_found_handler();
        }
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