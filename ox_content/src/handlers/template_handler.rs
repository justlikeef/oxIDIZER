use std::fs;
use std::path::PathBuf;
use tera::{Context, Tera};

pub fn template_handler(file_path: PathBuf, mimetype: &str) -> Result<(Vec<u8>, String), String> {
    match fs::read_to_string(&file_path) {
        Ok(template_content) => {
            let context = Context::new();
            // You can add more variables to the context here if needed
            // context.insert("title", "My Page");

            match Tera::one_off(&template_content, &context, false) {
                Ok(rendered) => Ok((rendered.into_bytes(), mimetype.to_string())),
                Err(e) => Err(format!("Failed to render template: {}", e)),
            }
        }
        Err(e) => {
            Err(format!("Error reading template file: {}", e))
        }
    }
}
