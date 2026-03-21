use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Per-function call limits for `CoreHostApi` rate limiting.
/// Key is function name (e.g. "insert_into_flow"), value is max calls per task execution (0 = unlimited).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HostApiLimits {
    pub limits: HashMap<String, u32>,
}

impl Default for HostApiLimits {
    fn default() -> Self {
        let mut limits = HashMap::new();
        limits.insert("insert_into_flow".to_string(), 100);
        limits.insert("pause_task".to_string(), 1);
        Self { limits }
    }
}

/// The primary engine configuration found in `engine.yaml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EngineConfig {
    pub max_concurrent_flows: usize,
    pub max_concurrent_tasks: usize,
    pub tick_interval_ms: u64,
    pub default_data_dir: PathBuf,
    pub default_flow_dir: PathBuf,
    pub default_stage_dir: PathBuf,
    pub log_config_path: Option<PathBuf>,
    pub api_limits: HostApiLimits,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_concurrent_flows: 10_000,
            max_concurrent_tasks: 100_000,
            tick_interval_ms: 1000,
            default_data_dir: PathBuf::from("data"),
            default_flow_dir: PathBuf::from("flows"),
            default_stage_dir: PathBuf::from("stages"),
            log_config_path: Some(PathBuf::from("config/log4rs.yaml")),
            api_limits: HostApiLimits::default(),
        }
    }
}

/// The discipline (behavior protocol) for a queue.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueueDiscipline {
    Fifo,
    Priority,
    Hybrid,
    EventOnly,
    DeadLetter,
}

/// Configuration for a specific queue defined in `queues.yaml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QueueConfig {
    pub name: String,
    pub discipline: QueueDiscipline,
    pub run_every: i32,
    pub priority_levels: u8,
    pub max_messages: usize,
    pub max_throughput_per_sec: Option<u32>,
}

/// The structure of the `queues.yaml` file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QueuesManifest {
    pub queues: Vec<QueueConfig>,
}

/// Loads a YAML configuration from a given file path using `ox_fileproc` (supporting `!include`).
pub fn load_config_from_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let _value = ox_fileproc::process_file(path, 10).map_err(|e| e.to_string())?;
    // ox_fileproc returns a serde_json::Value. We can deserialize T from it.
    serde_json::from_value(_value).map_err(|e| format!("Failed to parse config: {}", e))
}

/// Instantiates log4rs based on the path provided.
pub fn init_logging(path: Option<&Path>) -> Result<(), String> {
    if let Some(p) = path {
        log4rs::init_file(p, Default::default()).map_err(|e| format!("log4rs error: {}", e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_config_default() {
        let config = EngineConfig::default();
        assert_eq!(config.max_concurrent_flows, 10_000);
        assert_eq!(config.tick_interval_ms, 1000);
    }
}
