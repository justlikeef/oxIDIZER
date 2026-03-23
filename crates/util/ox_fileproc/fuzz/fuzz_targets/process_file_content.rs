#![no_main]
use libfuzzer_sys::fuzz_target;
use ox_fileproc::processor::parse_content;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() { return; }
    
    // Use first byte to select extension
    let selector = data[0] % 6;
    let extension = match selector {
        0 => "json",
        1 => "yaml",
        2 => "toml",
        3 => "xml",
        4 => "json5",
        5 => "kdl",
        _ => "json", // Unreachable
    };
    
    let content_bytes = &data[1..];
    if let Ok(content) = std::str::from_utf8(content_bytes) {
         let _ = parse_content(content, extension);
    }
});
