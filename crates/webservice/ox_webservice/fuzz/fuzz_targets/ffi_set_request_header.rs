#![no_main]
use libfuzzer_sys::fuzz_target;
use std::ffi::{CString, c_void};
use ox_workflow_core::Task;
use ox_workflow_executor::create_host_api;

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 { return; }
    let split_idx = data.len() / 2;
    let (key_bytes, val_bytes) = data.split_at(split_idx);

    let key_c = match CString::new(key_bytes) { Ok(s) => s, Err(_) => return };
    let val_c = match CString::new(val_bytes) { Ok(s) => s, Err(_) => return };

    let api = create_host_api();
    let mut task = Task::new(1);
    let task_ptr = &mut task as *mut Task as *mut c_void;

    (api.set_field)(task_ptr, key_c.as_ptr(), val_c.as_ptr());
});
