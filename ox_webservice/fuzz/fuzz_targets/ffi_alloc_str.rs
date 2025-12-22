#![no_main]
use libfuzzer_sys::fuzz_target;
use std::ffi::{CStr, CString, c_void};
use ox_webservice::pipeline::alloc_str_c;
use bumpalo::Bump;

fuzz_target!(|data: &[u8]| {
    let arena = Bump::new();
    let arena_ptr = &arena as *const Bump as *const c_void;

    if let Ok(c_str) = CString::new(data) {
        unsafe {
            let result_ptr = alloc_str_c(arena_ptr, c_str.as_ptr());
            if !result_ptr.is_null() {
                 let _ = CStr::from_ptr(result_ptr);
            }
        }
    }
});
