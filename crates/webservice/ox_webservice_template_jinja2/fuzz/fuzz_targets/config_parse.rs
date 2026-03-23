#![no_main]
use libfuzzer_sys::fuzz_target;
use ox_webservice_template_jinja2::{ContentConfig, MimeTypeMapping};
use serde_json::{self, Value};
use serde::Deserialize;

#[derive(Deserialize)]
struct MimeTypeConfig {
    mimetypes: Vec<MimeTypeMapping>,
}

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Fuzz ContentConfig
        let _ = serde_json::from_str::<ContentConfig>(s);
        
        // Fuzz MimeTypeConfig
        let _ = serde_json::from_str::<MimeTypeConfig>(s);
    }
});
