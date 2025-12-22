#![no_main]
use libfuzzer_sys::fuzz_target;
use ox_persistence_datastore_drivermanager::DriverManagerConfig;
use serde_json::{self, Value};

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = serde_json::from_str::<DriverManagerConfig>(s);
    }
});
