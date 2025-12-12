pub use ox_webservice_api::*;
pub mod pipeline;

use std::error::Error;
use std::fmt;
use std::path::Path;
use serde::Deserialize;
use log::{debug, trace, error};

#[derive(Debug)]
pub enum ConfigError {
    NotFound,
    ReadError(std::io::Error),
    ParseError(String),
    UnsupportedFileExtension,
    Deserialization(String),
    UnsupportedFormat,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ConfigError::NotFound => write!(f, "Configuration file not found"),
            ConfigError::ReadError(e) => write!(f, "Error reading configuration file: {}", e),
            ConfigError::ParseError(e) => write!(f, "Error parsing configuration file: {}", e),
            ConfigError::UnsupportedFileExtension => write!(f, "Unsupported configuration file extension"),
            ConfigError::Deserialization(e) => write!(f, "Error deserializing configuration: {}", e),
            ConfigError::UnsupportedFormat => write!(f, "Unsupported configuration file format"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ConfigError::ReadError(e) => Some(e),
            _ => None,
        }
    }
}

use serde::Serialize;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UrlRoute {
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    pub url: String,
    pub module_id: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    #[serde(default)]
    pub urls: Vec<UrlRoute>,
    #[serde(default)]
    pub modules: Vec<ModuleConfig>,
    pub log4rs_config: String,
    pub enable_metrics: Option<bool>,
    pub servers: Vec<ServerDetails>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct HostDetails {
    pub name: String,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerDetails {
    pub protocol: String,
    pub port: u16,
    pub bind_address: String,
    pub hosts: Vec<HostDetails>,
}

pub fn load_config_from_path(path: &Path, cli_log_level: &str) -> Result<ServerConfig, ConfigError> {
    debug!("Loading config from: {:?}", path);
    trace!("File extension: {:?}", path.extension());

    if !path.exists() {
        error!("Configuration file not found at {:?}", path);
        return Err(ConfigError::NotFound);
    }
    
    // Use ox_fileproc to process the file (supports include, variables, multi-format)
    let value = ox_fileproc::process_file(path, 5)
        .map_err(|e| ConfigError::ReadError(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;

    let contents = serde_json::to_string_pretty(&value)
        .map_err(|e| ConfigError::Deserialization(e.to_string()))?;

    if cli_log_level == "trace" {
        trace!("Processed config content:\n{}", contents);
    } else if cli_log_level == "debug" {
        debug!("Processed config content:\n{}", contents);
    }

    // Deserialize the processed JSON value into ServerConfig
    serde_json::from_value(value).map_err(|e| ConfigError::Deserialization(e.to_string()))
}
