use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use libc::c_char;
use std::ffi::CStr;

use std::thread;

pub fn template_handler(file_path: PathBuf, mimetype: &str, render_fn: unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char) -> Result<String, String> {
    match fs::read_to_string(&file_path) {
        Ok(content) => {
            let template_name = {
                let state = unsafe { crate::MODULE_STATE.as_ref().ok_or_else(|| "Module state not initialized".to_string())? };
                let stripped_path = file_path.strip_prefix(&state.content_root)
                    .unwrap_or(&file_path);
                stripped_path.to_str()
                    .ok_or_else(|| format!("Invalid UTF-8 in path: {}", stripped_path.display()))?
                    .replace("\\", "/")
                    .to_string()
            };

            let data_json = serde_json::json!({ "content": content, "path": file_path.to_str().unwrap_or("") }).to_string();

            let handle = thread::spawn(move || {
                let template_name_cstring = CString::new(template_name)
                    .map_err(|e: std::ffi::NulError| format!("Failed to create CString for template name: {}", e))?;
                let data_cstring = CString::new(data_json)
                    .map_err(|e: std::ffi::NulError| format!("Failed to create CString for data JSON: {}", e))?;

                let rendered_html = unsafe {
                    let rendered_html_ptr = render_fn(template_name_cstring.into_raw(), data_cstring.into_raw());
                    let rendered_html = CStr::from_ptr(rendered_html_ptr).to_str()
                        .map_err(|e: std::str::Utf8Error| format!("Failed to convert rendered HTML to string: {}", e))?.to_string();
                    // Free the CString from the FFI call
                    let _ = CString::from_raw(rendered_html_ptr);
                    rendered_html
                };
                Ok(rendered_html)
            });

            let rendered_html = handle.join()
                .map_err(|e: Box<dyn std::any::Any + Send>| format!("Template rendering thread panicked: {:?}", e))?
                .map_err(|e: String| format!("Template rendering failed: {}", e))?;

            let response = serde_json::json!({
                "headers": {
                    "Content-Type": mimetype
                },
                "body": rendered_html
            });
            println!("DEBUG: template_handler returning: {}", response.to_string());
            Ok(response.to_string())
        }
        Err(e) => {
            Err(format!("Error reading template file: {}", e))
        }
    }
}
