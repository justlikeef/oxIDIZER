#![no_main]
use libfuzzer_sys::fuzz_target;
use ox_webservice_api::ModuleConfig;

fuzz_target!(|data: &[u8]| {
    // Fuzz the string input for JSON deserialization
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = serde_json::from_str::<ModuleConfig>(s);
    }
});
