use libc::{c_char, c_void};

pub mod sdk;

pub use sdk::{
    PluginInitFn, PluginProcessFn, PluginErrorFn, PluginDestroyFn,
    PluginNegotiateFn, AllocFn, DeallocFn, PluginConfig,
    PLUGIN_ABI_VERSION,
};

/// Current ABI version for the workflow engine and plugins
pub const OX_WORKFLOW_ABI_VERSION: u32 = 3;
/// Minimum ABI version this crate supports
pub const OX_WORKFLOW_ABI_MIN_VERSION: u32 = 3;

/// Feature flags for plugin capabilities
pub const FEATURE_NONE: u64 = 0;
pub const FEATURE_BINARY_DATA: u64 = 1 << 0;
pub const FEATURE_METADATA: u64 = 1 << 1;
pub const FEATURE_FLAGS: u64 = 1 << 2;
pub const FEATURE_FLOW_INSERT: u64 = 1 << 3;
pub const FEATURE_TASK_PAUSE: u64 = 1 << 4;
pub const FEATURE_ASYNC: u64 = 1 << 5;
pub const FEATURE_WASM: u64 = 1 << 6;

/// Plugin capabilities structure returned during version negotiation.
/// This allows the host to understand what features a plugin supports.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PluginCapabilities {
    /// Minimum ABI version the plugin supports
    pub min_abi_version: u32,
    /// Maximum ABI version the plugin supports
    pub max_abi_version: u32,
    /// Bitfield of supported features (use FEATURE_* constants)
    pub features: u64,
    /// Plugin name (null-terminated C string, max 64 chars including null)
    pub name: [c_char; 64],
    /// Plugin version (null-terminated C string, max 32 chars including null)
    pub version: [c_char; 32],
}

/// Initializes a PluginCapabilities with default values (zeroed).
impl Default for PluginCapabilities {
    fn default() -> Self {
        Self {
            min_abi_version: OX_WORKFLOW_ABI_VERSION,
            max_abi_version: OX_WORKFLOW_ABI_VERSION,
            features: FEATURE_NONE,
            name: [0; 64],
            version: [0; 32],
        }
    }
}

impl PluginCapabilities {
    /// Creates a new PluginCapabilities with the given version range and features.
    pub fn new(min_version: u32, max_version: u32, features: u64) -> Self {
        Self {
            min_abi_version: min_version,
            max_abi_version: max_version,
            features,
            name: [0; 64],
            version: [0; 32],
        }
    }

    /// Sets the plugin name.
    pub fn with_name(mut self, name: &str) -> Self {
        let bytes = name.as_bytes();
        let len = bytes.len().min(63);
        for (i, byte) in bytes.iter().take(len).enumerate() {
            self.name[i] = *byte as c_char;
        }
        self.name[len] = 0;
        self
    }

    /// Sets the plugin version.
    pub fn with_version(mut self, version: &str) -> Self {
        let bytes = version.as_bytes();
        let len = bytes.len().min(31);
        for (i, byte) in bytes.iter().take(len).enumerate() {
            self.version[i] = *byte as c_char;
        }
        self.version[len] = 0;
        self
    }
}

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
    pub log: extern "C" fn(task_ctx: *mut c_void, level: u8, message: *const c_char),

    // Flag management
    pub set_flag: extern "C" fn(task_ctx: *mut c_void, flag: *const c_char, scope: u8),
    pub set_flags: extern "C" fn(task_ctx: *mut c_void, flags: *const *const c_char, scope: u8),
    pub has_flag: extern "C" fn(task_ctx: *mut c_void, flag: *const c_char, scope: u8) -> bool,
    pub clear_flag: extern "C" fn(task_ctx: *mut c_void, flag: *const c_char, scope: u8),

    // Typed context accessors
    /// Get all keys in task context. Returns comma-separated list, empty if none.
    pub get_keys: extern "C" fn(task_ctx: *mut c_void) -> *const c_char,
    /// Remove a key from task context. Returns 1 if removed, 0 if not found.
    pub unset_field: extern "C" fn(task_ctx: *mut c_void, key: *const c_char) -> bool,
    /// Check if key exists. Returns 1 if exists, 0 if not.
    pub has_field: extern "C" fn(task_ctx: *mut c_void, key: *const c_char) -> bool,
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

/// Optional version negotiation function.
/// If present, called during plugin load to determine compatible ABI and capabilities.
/// Returns pointer to PluginCapabilities (caller frees with free_plugin_caps).
/// If null, uses legacy init path with version check.
pub type OxPluginNegotiateFn = extern "C" fn(abi_version: u32) -> *mut PluginCapabilities;

/// Frees capabilities returned by negotiate function.
/// Should be called by host after obtaining capabilities.
#[no_mangle]
pub unsafe extern "C" fn free_plugin_caps(caps: *mut PluginCapabilities) {
    if !caps.is_null() {
        let _ = Box::from_raw(caps);
    }
}

#[no_mangle]
#[allow(improper_ctypes_definitions)]
pub extern "C" fn _ox_workflow_dummy_export(
    _fc: FlowControl,
    _api: CoreHostApi,
    _init: OxPluginInitFn,
    _proc: OxPluginProcessFn,
    _err: OxPluginErrorFn,
    _destroy: OxPluginDestroyFn,
    _negotiate: Option<OxPluginNegotiateFn>,
    _caps: Option<PluginCapabilities>,
) {}
