use regex::Regex;
use libc::{c_void, c_char};
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_STREAM_FILE,
    OX_LOG_INFO, OX_LOG_ERROR, OX_LOG_WARN,
};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{CStr, CString};
use std::fs;
use std::path::PathBuf;
use anyhow::Result;

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

pub struct ModuleContext {
    content_root: PathBuf,
    mimetypes: Vec<MimeTypeMapping>,
    default_documents: Vec<DocumentConfig>,
    content_config: ContentConfig,
    api: CoreHostApi,
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let res_ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if res_ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(res_ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    let c_key = CString::new(key).unwrap();
    let c_val = CString::new(value).unwrap();
    (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

impl ModuleContext {
    fn resolve_and_find_file(&self, request_path: &str) -> Option<PathBuf> {
        let mut file_path = self.content_root.clone();
        file_path.push(request_path.trim_start_matches('/'));

        if !file_path.exists() { return None; }

        if let Ok(canonical) = file_path.canonicalize() {
            if !canonical.starts_with(&self.content_root) { return None; }
            file_path = canonical;
        } else { return None; }

        if file_path.is_dir() {
            for doc in &self.default_documents {
                let mut candidate = file_path.clone();
                candidate.push(&doc.document);
                if candidate.exists() { return Some(candidate); }
            }
            None
        } else {
            Some(file_path)
        }
    }
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

    let params: Value = serde_json::from_str(&params_str).unwrap_or(Value::Null);

    let config: ContentConfig = match params.get("config_file").and_then(|v| v.as_str()) {
        Some(cfg_file) => match ox_fileproc::process_file(&PathBuf::from(cfg_file), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to parse config: {}", e)); return std::ptr::null_mut(); }
            },
            Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to read config: {}", e)); return std::ptr::null_mut(); }
        },
        None => {
            let content_root = params.get("content_root").and_then(|v| v.as_str()).unwrap_or("./www").to_string();
            let mimetypes_file = params.get("mimetypes_file").and_then(|v| v.as_str()).unwrap_or("conf/mimetypes.yaml").to_string();
            ContentConfig { content_root, mimetypes_file, default_documents: vec![DocumentConfig { document: "index.html".to_string() }], on_content_conflict: None }
        }
    };

    let mimo_path = PathBuf::from(&config.mimetypes_file);
    let mut mimo_cfg: MimeTypeConfig = match ox_fileproc::process_file(&mimo_path, 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to parse mimetypes: {}", e)); return std::ptr::null_mut(); }
        },
        Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to read mimetypes: {}", e)); return std::ptr::null_mut(); }
    };

    for m in &mut mimo_cfg.mimetypes {
        if let Ok(re) = Regex::new(&m.url) { m.compiled_regex = Some(re); }
    }

    let content_root = match fs::canonicalize(&config.content_root) {
        Ok(p) => p,
        Err(_) => PathBuf::from(&config.content_root),
    };

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, &format!("{} initialized with {} mimetypes", MODULE_NAME, mimo_cfg.mimetypes.len()));

    let ctx = Box::new(ModuleContext {
        content_root,
        mimetypes: mimo_cfg.mimetypes,
        default_documents: config.default_documents.clone(),
        content_config: config,
        api,
    });
    Box::into_raw(ctx) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_process(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    if plugin_config_ctx.is_null() {
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }
    let context = unsafe { &*(plugin_config_ctx as *mut ModuleContext) };
    let api = &context.api;

    let existing_body = get_field(api, task_ctx, "response.body");
    if !existing_body.is_empty() {
        match context.content_config.on_content_conflict {
            Some(ContentConflictAction::skip) => return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() },
            Some(ContentConflictAction::error) => {
                set_field(api, task_ctx, "response.status", "500");
                return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
            }
            _ => {}
        }
    }

    let request_path = {
        let capture = get_field(api, task_ctx, "request.capture");
        if !capture.is_empty() { capture } else {
            let p = get_field(api, task_ctx, "request.path");
            if p.is_empty() { "/".to_string() } else { p }
        }
    };

    if let Some(file_path) = context.resolve_and_find_file(&request_path) {
        let file_name_str = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("");

        let mimetype = context.mimetypes.iter()
            .find(|m| m.compiled_regex.as_ref().map(|re| re.is_match(file_name_str)).unwrap_or(false))
            .map(|m| m.mimetype.clone())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        match fs::metadata(&file_path) {
            Ok(metadata) if metadata.is_file() => {
                log(api, task_ctx, OX_LOG_INFO, &format!("Streaming file: {:?}", file_path));
                set_field(api, task_ctx, "response.header.Content-Type", &mimetype);
                set_field(api, task_ctx, "response.status", "200");

                let c_path = CString::new(file_path.to_string_lossy().into_owned()).unwrap();
                let payload_ptr = c_path.into_raw() as *const libc::c_char;
                return FlowControl { code: FLOW_CONTROL_STREAM_FILE, payload: payload_ptr };
            }
            Ok(_) => {
                set_field(api, task_ctx, "response.status", "404");
                set_field(api, task_ctx, "response.body", "404 Not Found (not a file)");
            }
            Err(e) => {
                log(api, task_ctx, OX_LOG_ERROR, &format!("Metadata error for {:?}: {}", file_path, e));
                set_field(api, task_ctx, "response.status", "500");
            }
        }
    } else {
        log(api, task_ctx, OX_LOG_WARN, &format!("File not found: {}", request_path));
        set_field(api, task_ctx, "response.status", "404");
        set_field(api, task_ctx, "response.body", "404 Not Found");
    }

    FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        let _ = Box::from_raw(plugin_config_ctx as *mut ModuleContext);
    }
}
