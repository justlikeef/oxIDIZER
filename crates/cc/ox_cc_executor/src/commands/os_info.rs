use serde_json::{Map, Value};
use async_trait::async_trait;
use super::{CommandPlugin, StateMap};

pub struct OsInfoCommand;

#[async_trait]
impl CommandPlugin for OsInfoCommand {
    fn name(&self) -> &str { "os_info" }

    async fn execute(&self, _params: &Map<String, Value>, _state: &StateMap) -> anyhow::Result<Map<String, Value>> {
        let mut out = Map::new();

        // hostname
        if let Ok(h) = hostname::get() {
            out.insert("hostname".to_string(), Value::String(h.to_string_lossy().into_owned()));
        }

        // OS release info from /etc/os-release (Linux)
        #[cfg(target_os = "linux")]
        {
            if let Ok(content) = tokio::fs::read_to_string("/etc/os-release").await {
                for line in content.lines() {
                    if let Some((key, val)) = line.split_once('=') {
                        let val = val.trim_matches('"');
                        out.insert(format!("os_{}", key.to_lowercase()), Value::String(val.to_string()));
                    }
                }
            }
        }

        tracing::debug!(command = "os_info", keys = %out.len(), "gathered OS info");
        Ok(out)
    }
}
