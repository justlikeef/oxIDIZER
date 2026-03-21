use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_ERROR, FLOW_CONTROL_JUMP, FLOW_CONTROL_SUSPEND, OX_WORKFLOW_ABI_VERSION,
};
use std::ffi::{c_char, c_void, CStr, CString};

#[no_mangle]
pub extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    _api: *const CoreHostApi,
    abi_version: u32,
) -> *mut c_void {
    if abi_version != OX_WORKFLOW_ABI_VERSION {
        return std::ptr::null_mut();
    }
    
    let name = if plugin_config_ctx.is_null() {
        "default".to_string()
    } else {
        unsafe { CStr::from_ptr(plugin_config_ctx) }.to_string_lossy().to_string()
    };

    let behavior = match name.as_str() {
        "test_jump" => 1,
        "test_panic" => 2,
        "test_suspend" => 3,
        "test_malformed" => 4,
        _ => 0,
    };

    Box::into_raw(Box::new(behavior)) as *mut c_void
}

#[no_mangle]
pub extern "C" fn ox_plugin_process(
    plugin_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) -> FlowControl {
    if plugin_ctx.is_null() {
        return FlowControl {
            code: FLOW_CONTROL_CONTINUE,
            payload: std::ptr::null(),
        };
    }

    let behavior = unsafe { *(plugin_ctx as *mut i32) };
    match behavior {
        1 => {
            // Jump behavior
            FlowControl {
                code: FLOW_CONTROL_JUMP,
                payload: CString::new("target_stage_name").unwrap().into_raw(),
            }
        }
        2 => {
            // Panic behavior
            panic!("Test panic triggered intentionally.");
        }
        3 => {
            // Suspend behavior
            FlowControl {
                code: FLOW_CONTROL_SUSPEND,
                payload: std::ptr::null(),
            }
        }
        4 => {
            // Malformed JUMP (null payload)
            FlowControl {
                code: FLOW_CONTROL_JUMP,
                payload: std::ptr::null(),
            }
        }
        _ => {
            FlowControl {
                code: FLOW_CONTROL_CONTINUE,
                payload: std::ptr::null(),
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn ox_plugin_error(_plugin_ctx: *mut c_void, _task_ctx: *mut c_void) {}

#[no_mangle]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        unsafe {
            let _ = Box::from_raw(plugin_ctx as *mut i32);
        }
    }
}
