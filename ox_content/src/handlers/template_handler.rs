use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use libc::c_char;
use std::ffi::CStr;

use std::thread;

pub fn template_handler(file_path: PathBuf, mimetype: &str) -> *mut c_char {
    match fs::read_to_string(&file_path) {
        Ok(content) => {
            let state = unsafe { crate::MODULE_STATE.as_ref().unwrap() };

            let template_name = file_path.file_name().unwrap().to_str().unwrap().to_string();

            if let Some(render_fn) = state.render_template_fn {
                let data_json = serde_json::json!({ "content": content, "path": file_path.to_str().unwrap_or("") }).to_string();

                let handle = thread::spawn(move || {
                    let template_name_cstring = CString::new(template_name).unwrap();
                    let data_cstring = CString::new(data_json).unwrap();

                    unsafe {
                        let rendered_html_ptr = render_fn(template_name_cstring.into_raw(), data_cstring.into_raw());
                        let rendered_html = CStr::from_ptr(rendered_html_ptr).to_str().unwrap().to_string();
                        // Free the CString from the FFI call
                        let _ = CString::from_raw(rendered_html_ptr);
                        rendered_html
                    }
                });

                let rendered_html = handle.join().unwrap();

                let response = serde_json::json!({
                    "headers": {
                        "Content-Type": mimetype
                    },
                    "body": rendered_html
                });
                CString::new(response.to_string()).unwrap().into_raw()
            } else {
                let error_response = serde_json::json!({
                    "status": 500,
                    "body": "Template rendering function not available."
                });
                CString::new(error_response.to_string()).unwrap().into_raw()
            }
        }
        Err(e) => {
            let error_response = serde_json::json!({
                "status": 500,
                "headers": {
                    "Content-Type": "application/json"
                },
                "body": format!("Error reading template file: {}", e)
            });
            CString::new(error_response.to_string()).unwrap().into_raw()
        }
    }
}
