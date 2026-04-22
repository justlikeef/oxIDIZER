use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// Re-export new ox_workflow ABI types
pub use ox_workflow_abi::{
    CoreHostApi, FlowControl, OxPluginInitFn, OxPluginProcessFn,
    OxPluginErrorFn, OxPluginDestroyFn, OxPluginNegotiateFn, PluginCapabilities,
    FLOW_CONTROL_CONTINUE, FLOW_CONTROL_END, FLOW_CONTROL_ERROR,
    FLOW_CONTROL_JUMP, FLOW_CONTROL_SKIP, FLOW_CONTROL_SUSPEND, FLOW_CONTROL_YIELD, FLOW_CONTROL_STREAM_FILE,
    FLAG_SCOPE_STAGE, FLAG_SCOPE_TASK,
    OX_LOG_ERROR, OX_LOG_WARN, OX_LOG_INFO, OX_LOG_DEBUG, OX_LOG_TRACE,
    OX_WORKFLOW_ABI_VERSION, OX_WORKFLOW_ABI_MIN_VERSION,
    FEATURE_NONE, FEATURE_BINARY_DATA, FEATURE_METADATA, FEATURE_FLAGS,
    FEATURE_FLOW_INSERT, FEATURE_TASK_PAUSE, FEATURE_ASYNC, FEATURE_WASM,
};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UriMatcher {
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(alias = "url")]
    pub path: String,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub query: Option<HashMap<String, String>>,
    #[serde(default)]
    pub priority: u16,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub status_code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModuleConfig {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default, alias = "uris")]
    pub routes: Option<Vec<UriMatcher>>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub query: Option<HashMap<String, String>>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(flatten)]
    pub extra_params: HashMap<String, Value>,
}

impl Default for ModuleConfig {
    fn default() -> Self {
        ModuleConfig {
            id: None,
            name: String::new(),
            routes: None,
            headers: None,
            query: None,
            path: None,
            phase: None,
            params: None,
            extra_params: HashMap::new(),
        }
    }
}
