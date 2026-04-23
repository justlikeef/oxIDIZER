use libloading::Library;
use ox_workflow_abi::{
    CoreHostApi, FlowControl, OxPluginDestroyFn, OxPluginErrorFn, OxPluginInitFn,
    OxPluginNegotiateFn, OxPluginProcessFn, PluginCapabilities,
    OX_WORKFLOW_ABI_VERSION, OX_WORKFLOW_ABI_MIN_VERSION,
};
use std::collections::HashSet;
use std::ffi::CString;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PluginError {
    #[error("Failed to load library: {0}")]
    LoadError(#[from] libloading::Error),
    #[error("Initialization failed")]
    InitFailed,
    #[error("ABI version mismatch: plugin requires {0}, host supports {1}")]
    VersionMismatch(u32, u32),
    #[error("Missing dependency: {0}")]
    MissingDependency(String),
    #[error("Capability not supported: {0}")]
    UnsupportedCapability(String),
}

#[derive(Debug, Clone, Default)]
pub struct PluginConfig {
    pub path: String,
    pub depends_on: Vec<String>,
    pub optional_deps: Vec<String>,
}

impl PluginConfig {
    pub fn from_json(json: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        Some(Self {
            path: v.get("path")?.as_str()?.to_string(),
            depends_on: v.get("depends_on")?
                .as_array()?
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect(),
            optional_deps: v.get("optional_deps")?
                .as_array()?
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect(),
        })
    }
}

pub struct LoadedPlugin {
    _lib: Library,
    init_fn: OxPluginInitFn,
    #[allow(dead_code)]
    negotiate_fn: Option<OxPluginNegotiateFn>,
    process_fn: OxPluginProcessFn,
    error_fn: OxPluginErrorFn,
    destroy_fn: OxPluginDestroyFn,
    capabilities: Option<PluginCapabilities>,
}

impl LoadedPlugin {
    pub unsafe fn new(path: &str) -> Result<Self, PluginError> {
        let lib = Library::new(path)?;

        let init_fn: libloading::Symbol<OxPluginInitFn> = lib.get(b"ox_plugin_init\0")?;
        let init_fn_val = *init_fn;

        let process_fn: libloading::Symbol<OxPluginProcessFn> = lib.get(b"ox_plugin_process\0")?;
        let process_fn_val = *process_fn;

        let error_fn: libloading::Symbol<OxPluginErrorFn> = lib.get(b"ox_plugin_error\0")?;
        let error_fn_val = *error_fn;

        let destroy_fn: libloading::Symbol<OxPluginDestroyFn> = lib.get(b"ox_plugin_destroy\0")?;
        let destroy_fn_val = *destroy_fn;

        let negotiate_fn: Option<OxPluginNegotiateFn> = lib.get(b"ox_plugin_negotiate\0").ok().map(|x| *x);
        let capabilities: Option<PluginCapabilities> = if let Some(ref neg) = negotiate_fn {
            let ptr = (neg)(OX_WORKFLOW_ABI_VERSION);
            if ptr.is_null() { None } else { Some(*ptr) }
        } else {
            None
        };

        Ok(Self {
            _lib: lib,
            init_fn: init_fn_val,
            negotiate_fn,
            process_fn: process_fn_val,
            error_fn: error_fn_val,
            destroy_fn: destroy_fn_val,
            capabilities,
        })
    }

    pub fn check_dependencies(&self, loaded_plugins: &HashSet<String>) -> Result<(), PluginError> {
        if let Some(ref caps) = self.capabilities {
            let mut deps: Vec<String> = Vec::new();
            let mut n = Vec::new();
            let mut i = 0;
            while i < 64 && caps.name[i] != 0 {
                n.push(caps.name[i] as u8);
                i += 1;
            }
            let caps_name = String::from_utf8_lossy(&n).into_owned();
            if !caps_name.is_empty() {
                if !loaded_plugins.contains(&caps_name) {
                    deps.push(caps_name);
                }
            }
            if !deps.is_empty() {
                return Err(PluginError::MissingDependency(deps.join(", ")));
            }
        }
        Ok(())
    }

    pub fn init(&self, config_json: &str, api: &CoreHostApi) -> Result<*mut std::ffi::c_void, PluginError> {
        if let Some(ref caps) = self.capabilities {
            if caps.min_abi_version > OX_WORKFLOW_ABI_VERSION || caps.max_abi_version < OX_WORKFLOW_ABI_MIN_VERSION {
                return Err(PluginError::VersionMismatch(caps.min_abi_version, OX_WORKFLOW_ABI_VERSION));
            }
        }
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
