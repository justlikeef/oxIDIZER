use libloading::Library;
use ox_workflow_abi::{
    CoreHostApi, FlowControl, OxPluginDestroyFn, OxPluginErrorFn, OxPluginInitFn,
    OxPluginProcessFn, OX_WORKFLOW_ABI_VERSION,
};
use std::ffi::CString;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PluginError {
    #[error("Failed to load library: {0}")]
    LoadError(#[from] libloading::Error),
    #[error("Initialization failed")]
    InitFailed,
}

/// Wraps the dynamically loaded library and its function pointers.
/// The `Drop` implementation of `libloading::Library` automatically calls `dlclose`.
pub struct LoadedPlugin {
    _lib: Library,
    init_fn: OxPluginInitFn,
    process_fn: OxPluginProcessFn,
    error_fn: OxPluginErrorFn,
    destroy_fn: OxPluginDestroyFn,
}

impl LoadedPlugin {
    pub unsafe fn new(path: &str) -> Result<Self, PluginError> {
        let lib = Library::new(path)?;
        
        let init_fn: libloading::Symbol<OxPluginInitFn> = lib.get(b"ox_plugin_init\0")?;
        let process_fn: libloading::Symbol<OxPluginProcessFn> = lib.get(b"ox_plugin_process\0")?;
        let error_fn: libloading::Symbol<OxPluginErrorFn> = lib.get(b"ox_plugin_error\0")?;
        let destroy_fn: libloading::Symbol<OxPluginDestroyFn> = lib.get(b"ox_plugin_destroy\0")?;

        Ok(Self {
            init_fn: *init_fn,
            process_fn: *process_fn,
            error_fn: *error_fn,
            destroy_fn: *destroy_fn,
            _lib: lib,
        })
    }

    pub fn init(&self, config_json: &str, api: &CoreHostApi) -> Result<*mut std::ffi::c_void, PluginError> {
        let config_cstr = CString::new(config_json).unwrap();
        let ctx = (self.init_fn)(config_cstr.as_ptr(), api, OX_WORKFLOW_ABI_VERSION);
        if ctx.is_null() {
            return Err(PluginError::InitFailed);
        }
        Ok(ctx)
    }

    pub fn process(&self, plugin_ctx: *mut std::ffi::c_void, task_ctx: *mut std::ffi::c_void) -> FlowControl {
        (self.process_fn)(plugin_ctx, task_ctx)
    }

    pub fn error(&self, plugin_ctx: *mut std::ffi::c_void, task_ctx: *mut std::ffi::c_void) {
        (self.error_fn)(plugin_ctx, task_ctx)
    }

    pub fn destroy(&self, plugin_ctx: *mut std::ffi::c_void) {
        (self.destroy_fn)(plugin_ctx)
    }
}

/// Represents the initialized state of a plugin for a specific stage.
pub struct PluginInstance {
    pub name: String,
    pub plugin: Arc<LoadedPlugin>,
    pub ctx: *mut std::ffi::c_void,
}

unsafe impl Send for PluginInstance {}
unsafe impl Sync for PluginInstance {}

impl Drop for PluginInstance {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            self.plugin.destroy(self.ctx);
        }
    }
}
