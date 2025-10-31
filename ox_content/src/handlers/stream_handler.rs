use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use libc::c_char;
use base64::engine::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::template_handler;

pub fn stream_handler(file_path: PathBuf, mimetype: &str) -> *mut c_char {
    match fs::read(file_path) {
        Ok(content) => {
            let body_content = if mimetype.starts_with("text/") || mimetype.contains("javascript") || mimetype.contains("json") {
                // Assume UTF-8 for text-based content
                String::from_utf8_lossy(&content).into_owned()
            } else {
                // Base64 encode binary content
                STANDARD.encode(&content)
            };

            let response = serde_json::json!({
                "headers": {
                    "Content-Type": mimetype
                },
                "body": body_content
            });
            println!("DEBUG: stream_handler response: {}", response.to_string());
            CString::new(response.to_string()).unwrap().into_raw()
        }
        Err(_) => not_found_handler(),
    }
}

pub fn not_found_handler() -> *mut c_char {
    println!("DEBUG: not_found_handler called");
    let state = unsafe { crate::MODULE_STATE.as_ref().unwrap() };
    let mut error_template_path = state.error_path.clone();
    error_template_path.push("404.jinja2");

    // Directly return the JSON string from template_handler
    template_handler::template_handler(error_template_path, "text/html")
}
