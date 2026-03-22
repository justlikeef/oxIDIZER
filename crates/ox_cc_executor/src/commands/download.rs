use serde_json::{Map, Value};
use async_trait::async_trait;
use super::{CommandPlugin, StateMap};

pub struct DownloadCommand;

#[async_trait]
impl CommandPlugin for DownloadCommand {
    fn name(&self) -> &str { "download" }

    async fn execute(&self, params: &Map<String, Value>, _state: &StateMap) -> anyhow::Result<Map<String, Value>> {
        let url = params.get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("download: missing required param 'url'"))?;
        let dest = params.get("dest")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("download: missing required param 'dest'"))?;

        tracing::info!(command = "download", url = %url, dest = %dest, "downloading");

        let response = reqwest::get(url).await
            .map_err(|e| anyhow::anyhow!("download GET {}: {}", url, e))?;

        anyhow::ensure!(
            response.status().is_success(),
            "download: server returned {}",
            response.status()
        );

        let bytes = response.bytes().await?;
        tokio::fs::write(dest, &bytes).await
            .map_err(|e| anyhow::anyhow!("download: write to {}: {}", dest, e))?;

        tracing::info!(command = "download", dest = %dest, bytes = %bytes.len(), "download complete");

        let mut out = Map::new();
        out.insert("dest".to_string(), Value::String(dest.to_string()));
        out.insert("bytes".to_string(), Value::Number(bytes.len().into()));
        Ok(out)
    }
}
