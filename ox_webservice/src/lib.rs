pub use ox_webservice_api::*;

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::Read;
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

// Structs need to be public for main.rs to use them.
#[derive(Debug, Deserialize)]
pub struct UrlRoute {
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    pub url: String,
    pub module_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub urls: Vec<UrlRoute>,
    #[serde(default)]
    pub modules: Vec<ModuleConfig>,
    pub log4rs_config: String,
    pub servers: Vec<ServerDetails>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HostDetails {
    pub name: String,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
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

    let mut file = File::open(path)
        .map_err(ConfigError::ReadError)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(ConfigError::ReadError)?;

    debug!("Content read from config file: \n{}", contents);

    if cli_log_level == "trace" {
        trace!("Parsed config file content:\n{}", contents);
    } else if cli_log_level == "debug" {
        debug!("Parsed config file content:\n{}", contents);
    }

    match path.extension().and_then(|s| s.to_str()) {
        Some("yaml") | Some("yml") => {
            debug!("Parsing as YAML");
            serde_yaml::from_str(&contents).map_err(|e| ConfigError::Deserialization(e.to_string()))
        }
        Some("json") => {
            debug!("Parsing as JSON");
            serde_json::from_str(&contents).map_err(|e| ConfigError::Deserialization(e.to_string()))
        }
        Some("toml") => {
            debug!("Parsing as TOML");
            toml::from_str(&contents).map_err(|e| ConfigError::Deserialization(e.to_string()))
        }
        Some("xml") => {
            debug!("Parsing as XML");
            serde_xml_rs::from_str(&contents).map_err(|e| ConfigError::Deserialization(e.to_string()))
        }
        _ => {
            error!("Unsupported server config file format: {:?}. Exiting.", path.extension());
            Err(ConfigError::UnsupportedFileExtension)
        }
    }
}
