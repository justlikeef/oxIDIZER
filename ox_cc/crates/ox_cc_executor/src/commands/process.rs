use std::path::PathBuf;
use std::process::Stdio;
use serde_json::{Map, Value};
use async_trait::async_trait;
use super::{CommandPlugin, StateMap};
use tokio::io::AsyncWriteExt;

pub struct ProcessCommand {
    pub binary: PathBuf,
    pub name: String,
}

#[async_trait]
impl CommandPlugin for ProcessCommand {
    fn name(&self) -> &str { &self.name }

    async fn execute(&self, params: &Map<String, Value>, _state: &StateMap) -> anyhow::Result<Map<String, Value>> {
        let input = serde_json::to_vec(params)?;

        let mut child = tokio::process::Command::new(&self.binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn {:?}: {}", self.binary, e))?;

        // Write params JSON to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&input).await?;
        }

        let output = child.wait_with_output().await?;
        anyhow::ensure!(
            output.status.success(),
            "process {:?} exited with {}",
            self.binary,
            output.status
        );

        // Parse stdout as JSON object; empty stdout → empty output map
        if output.stdout.is_empty() {
            return Ok(Map::new());
        }
        let result: Map<String, Value> = serde_json::from_slice(&output.stdout)
            .map_err(|e| anyhow::anyhow!("process stdout is not a JSON object: {}", e))?;
        Ok(result)
    }
}
