use libc::{c_char, c_void};

/// Current ABI version for the workflow engine and plugins
pub const OX_WORKFLOW_ABI_VERSION: u32 = 3;

/// Flow control code: Continue to the next plugin or stage.
pub const FLOW_CONTROL_CONTINUE: u8 = 0;
/// Flow control code: End the flow successfully.
pub const FLOW_CONTROL_END: u8 = 1;
/// Flow control code: Trigger error lifecycle.
pub const FLOW_CONTROL_ERROR: u8 = 2;
/// Flow control code: Branch to a specific stage named in `payload`.
pub const FLOW_CONTROL_JUMP: u8 = 3;
/// Flow control code: Skip to a plugin named in `payload` within the current stage.
pub const FLOW_CONTROL_SKIP: u8 = 4;
/// Flow control code: Pause task. `payload` can specify a timer or signal key.
pub const FLOW_CONTROL_SUSPEND: u8 = 5;
pub const FLOW_CONTROL_YIELD: u8 = 6;
/// Flow control code: Stream a file from the path in `payload` (a c_char path string).
pub const FLOW_CONTROL_STREAM_FILE: u8 = 7;

/// Flag scope: Cleared at each stage boundary.
pub const FLAG_SCOPE_STAGE: u8 = 0;
/// Flag scope: Persists with task state across stages.
pub const FLAG_SCOPE_TASK: u8 = 1;

/// Log level: error — unrecoverable condition.
pub const OX_LOG_ERROR: u8 = 1;
/// Log level: warning — recoverable but noteworthy.
pub const OX_LOG_WARN: u8 = 2;
/// Log level: informational.
pub const OX_LOG_INFO: u8 = 3;
/// Log level: debug — verbose diagnostic.
pub const OX_LOG_DEBUG: u8 = 4;
/// Log level: trace — very verbose, inner-loop detail.
pub const OX_LOG_TRACE: u8 = 5;

/// Structure returned by plugins to dictate the engine's next action.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FlowControl {
    pub code: u8,
    pub payload: *const c_char,
}

/// Host-provided API function table. 
/// Plugins use these functions to read/write state and perform task operations.
/// All strings are null-terminated C strings.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CoreHostApi {
    pub get_field: extern "C" fn(task_ctx: *mut c_void, key: *const c_char) -> *const c_char,
    pub set_field: extern "C" fn(task_ctx: *mut c_void, key: *const c_char, value: *const c_char),
    /// Read a binary (Bytes) field. Returns null pointer and sets len_out=0 if not found.
    /// The returned pointer is valid until the next API call on this task.
    pub get_field_bytes: extern "C" fn(task_ctx: *mut c_void, key: *const c_char, len_out: *mut usize) -> *const u8,
    /// Write a binary (Bytes) field. Copies `len` bytes from `value`.
    pub set_field_bytes: extern "C" fn(task_ctx: *mut c_void, key: *const c_char, value: *const u8, len: usize),
    pub get_metadata: extern "C" fn(task_ctx: *mut c_void, key: *const c_char) -> *const c_char,
    pub insert_into_flow: extern "C" fn(task_ctx: *mut c_void, flow_name: *const c_char) -> bool,
    pub pause_task: extern "C" fn(task_ctx: *mut c_void, signal_key: *const c_char),
    /// Log a message. `task_ctx` may be null when called outside of a task context.
    /// Use OX_LOG_* constants for `level`. The host enriches the record with task/stage context.
    pub log: extern "C" fn(task_ctx: *mut c_void, level: u8, message: *const c_char),
    
    // Flag management
    pub set_flag: extern "C" fn(task_ctx: *mut c_void, flag: *const c_char, scope: u8),
    pub set_flags: extern "C" fn(task_ctx: *mut c_void, flags: *const *const c_char, scope: u8),
    pub has_flag: extern "C" fn(task_ctx: *mut c_void, flag: *const c_char, scope: u8) -> bool,
    pub clear_flag: extern "C" fn(task_ctx: *mut c_void, flag: *const c_char, scope: u8),
}

/// Type representing the plugin initialization function
pub type OxPluginInitFn = extern "C" fn(
    plugin_config_ctx: *const c_char,
    api: *const CoreHostApi,
    abi_version: u32,
) -> *mut c_void;

/// Type representing the plugin process function
pub type OxPluginProcessFn = extern "C" fn(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl;

/// Type representing the plugin error callback
pub type OxPluginErrorFn = extern "C" fn(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
);

/// Type representing the plugin teardown/destroy function
pub type OxPluginDestroyFn = extern "C" fn(plugin_config_ctx: *mut c_void);

#[no_mangle]
pub extern "C" fn _ox_workflow_dummy_export(
    _fc: FlowControl,
    _api: CoreHostApi,
    _init: OxPluginInitFn,
    _proc: OxPluginProcessFn,
    _err: OxPluginErrorFn,
    _destroy: OxPluginDestroyFn,
) {}
