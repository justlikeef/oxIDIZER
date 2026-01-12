use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
pub use ox_webservice_api::{ModuleConfig, UriMatcher};

pub mod pipeline;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HostConfig {
    pub name: String,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerDetails {
    #[serde(default = "default_details_id")]
    pub id: String,
    pub protocol: String,
    pub port: u16,
    pub bind_address: String,
    #[serde(default)]
    pub hosts: Vec<HostConfig>,
}

fn default_details_id() -> String {
    "default".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PipelineConfig {
    pub phases: Option<Vec<HashMap<String, String>>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UrlRoute {
    pub protocol: Option<String>,
    pub hostname: Option<String>,
    #[serde(alias = "url")]
    pub url: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub query: Option<HashMap<String, String>>,
    #[serde(default)]
    pub priority: u16,
    pub phase: Option<String>,
    pub module_id: Option<String>,
    pub status_code: Option<String>,
}


// Config struct for direct deserialization - ox_fileproc handles merges now
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub log4rs_config: String,
    #[serde(default)]
    pub servers: Vec<ServerDetails>,
    #[serde(default)]
    pub modules: Vec<ModuleConfig>,
    pub pipeline: Option<PipelineConfig>,
    #[serde(default)]
    pub routes: Vec<UrlRoute>,
    #[serde(default)]
    pub merge: Option<Vec<String>>,
    #[serde(default)]
    pub merge_recursive: Option<Vec<String>>,
    pub enable_metrics: Option<bool>,
}

pub fn load_config_from_path(path: &Path, _log_level: &str) -> Result<(ServerConfig, String), String> {
    // Phase 1: Load Main Config using ox_fileproc (handles standard includes/substitutions/merges)
    let processed_value = ox_fileproc::process_file(path, 10).map_err(|e| format!("ox_fileproc failed to process config {:?}: {}", path, e))?;
    
    // ox_fileproc returns a merged Value. We just deserialize.
    // Note: The 'merge' and 'merge_recursive' keys are removed by ox_fileproc upon processing, 
    // so ServerConfig.merge/merge_recursive will be None (default), which is correct.
    
    let config: ServerConfig = serde_json::from_value(processed_value).map_err(|e| format!("Failed to deserialize ServerConfig from processed JSON: {}", e))?;
    
    // Re-serialize strictly for the "json" return required by main
    let config_json = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    
    Ok((config, config_json))
}
