
use std::fs;
use std::path::PathBuf;

pub fn stream_handler(file_path: PathBuf, mimetype: &str) -> Result<(Vec<u8>, String), String> {
    match fs::read(&file_path) {
        Ok(content) => {
            Ok((content, mimetype.to_string()))
        }
        Err(e) => {
            Err(format!("File not found: {}: {}", file_path.display(), e))
        }
    }
}
