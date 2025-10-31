use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use libc::c_char;
use reqwest;
use tokio;
use serde_json::json;

use crate::ModuleState;

pub fn template_handler(file_path: PathBuf, mimetype: &str) -> *mut c_char {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        match fs::read_to_string(&file_path) {
            Ok(content) => {
                let state = unsafe { crate::MODULE_STATE.as_ref().unwrap() };
                let context = &state.webservice_context;

                let render_url = format!("http://{}:{}/render_template", context.bound_ip, context.server_port);

                let template_name = file_path.file_name().unwrap().to_str().unwrap();

                let client = reqwest::Client::new();
                let res = client.post(&render_url)
                    .json(&json!({
                        "name": template_name,
                        "data": {"content": content} // Pass content as data for the template
                    }))
                    .send()
                    .await
                    .unwrap();

                let rendered_html = res.text().await.unwrap();

                let response = serde_json::json!({
                    "headers": {
                        "Content-Type": mimetype
                    },
                    "body": rendered_html
                });
                CString::new(response.to_string()).unwrap().into_raw()
            }
            Err(_) => super::stream_handler::not_found_handler(),
        }
    })
}
