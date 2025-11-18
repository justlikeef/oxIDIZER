use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use libc::c_char;
use std::ffi::CStr;

pub fn template_handler(file_path: PathBuf, mimetype: &str, render_fn: unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char, content_root: &PathBuf) -> Result<(Vec<u8>, String), String> {
    match fs::read_to_string(&file_path) {
        Ok(content) => {
            let template_name = {
                let stripped_path = file_path.strip_prefix(content_root)
                    .unwrap_or(&file_path);
                stripped_path.to_str()
                    .ok_or_else(|| format!("Invalid UTF-8 in path: {}", stripped_path.display()))?
                    .replace("\\", "/")
                    .to_string()
            };

            let data_json = serde_json::json!({ "content": content, "path": file_path.to_str().unwrap_or("") }).to_string();

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

            Ok((rendered_html.into_bytes(), mimetype.to_string()))
        }
        Err(e) => {
            Err(format!("Error reading template file: {}", e))
        }
    }
}
