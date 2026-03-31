#![no_main]
use libfuzzer_sys::fuzz_target;
use std::ffi::{CStr, CString, c_void};
use ox_workflow_core::{Task, state::FieldValue};
use ox_workflow_executor::create_host_api;

fuzz_target!(|data: &[u8]| {
    let api = create_host_api();
    let mut task = Task::new(1);

    // Pre-populate a field so get_field has something to allocate
    {
        let mut state = task.state.write();
        state.fields.insert("test.key".to_string(), FieldValue::String("test_value".to_string()));
    }

    let task_ptr = &mut task as *mut Task as *mut c_void;

    if let Ok(c_key) = CString::new(data) {
        let result_ptr = (api.get_field)(task_ptr, c_key.as_ptr());
        if !result_ptr.is_null() {
            unsafe { let _ = CStr::from_ptr(result_ptr); }
        }
    }
});
