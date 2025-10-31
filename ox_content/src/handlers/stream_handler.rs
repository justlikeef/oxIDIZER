use std::ffi::{CStr, CString};
use std::fs;
use std::path::PathBuf;
use libc::c_char;
use serde_json::json;

use crate::ModuleState;
use super::template_handler;

pub fn stream_handler(file_path: PathBuf, mimetype: &str) -> *mut c_char {
    match fs::read(file_path) {
        Ok(content) => {
            let response = serde_json::json!({
                "headers": {
                    "Content-Type": mimetype
                },
                "body": content
            });
            CString::new(response.to_string()).unwrap().into_raw()
        }
        Err(_) => not_found_handler(),
    }
}

pub fn not_found_handler() -> *mut c_char {
    let state = unsafe { crate::MODULE_STATE.as_ref().unwrap() };
    let mut error_template_path = state.content_root.clone();
    error_template_path.push("errors");
    error_template_path.push("404.jinja2");

    let rendered_response_ptr = template_handler::template_handler(error_template_path, "text/html");
    let rendered_response_str = unsafe { CStr::from_ptr(rendered_response_ptr).to_str().unwrap() };
    let rendered_response: serde_json::Value = serde_json::from_str(rendered_response_str).unwrap();
    let rendered_body = rendered_response["body"].as_str().unwrap_or("Error rendering 404 template");

    let response = serde_json::json!({
        "status": 404,
        "headers": {
            "Content-Type": "text/html"
        },
        "body": rendered_body
    });
    CString::new(response.to_string()).unwrap().into_raw()
}
