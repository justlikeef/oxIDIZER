use crate::{FlowControl, PluginCapabilities};
use libc::{c_char, c_void};

pub const PLUGIN_ABI_VERSION: u32 = 3;

pub type PluginInitFn = extern "C" fn(
    plugin_config: *const c_char,
    api: *const crate::CoreHostApi,
    abi_version: u32,
) -> *mut c_void;

pub type PluginProcessFn = extern "C" fn(
    plugin_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl;

pub type PluginErrorFn = extern "C" fn(
    plugin_ctx: *mut c_void,
    task_ctx: *mut c_void,
);

pub type PluginDestroyFn = extern "C" fn(plugin_ctx: *mut c_void);

pub type PluginNegotiateFn = extern "C" fn(host_version: u32) -> *mut PluginCapabilities;

pub type AllocFn = extern "C" fn(size: u32) -> u32;
pub type DeallocFn = extern "C" fn(ptr: u32);

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