
use std::fs;
use std::path::PathBuf;

use base64::engine::Engine as _;
use base64::engine::general_purpose::STANDARD;



pub fn stream_handler(file_path: PathBuf, mimetype: &str) -> Result<String, String> {
    match fs::read(&file_path) {
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
            Ok(response.to_string())
        }
        Err(_) => {
            Err(format!("File not found: {}", file_path.display()))
        }
    }
}
