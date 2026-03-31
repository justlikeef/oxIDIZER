#![no_main]
use libfuzzer_sys::fuzz_target;
use std::ffi::{CString, c_void};
use ox_workflow_core::Task;
use ox_workflow_executor::create_host_api;

fuzz_target!(|data: &[u8]| {
    if let Ok(c_val) = CString::new(data) {
        let api = create_host_api();
        let mut task = Task::new(1);
        let task_ptr = &mut task as *mut Task as *mut c_void;

        let key = CString::new("request.path").unwrap();
        (api.set_field)(task_ptr, key.as_ptr(), c_val.as_ptr());
    }
});
