use serde_json::{Map, Value};
use async_trait::async_trait;
use super::{CommandPlugin, StateMap};

pub struct InstallCommand;

#[async_trait]
impl CommandPlugin for InstallCommand {
    fn name(&self) -> &str { "install" }

    async fn execute(&self, params: &Map<String, Value>, _state: &StateMap) -> anyhow::Result<Map<String, Value>> {
        let source = params.get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("install: missing required param 'source'"))?;
        let dest = params.get("dest")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("install: missing required param 'dest'"))?;
        let mode = params.get("mode")
            .and_then(|v| v.as_u64())
            .unwrap_or(0o755);

        tokio::fs::copy(source, dest).await
            .map_err(|e| anyhow::anyhow!("install: copy {} → {}: {}", source, dest, e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(mode as u32);
            std::fs::set_permissions(dest, perms)
                .map_err(|e| anyhow::anyhow!("install: chmod {}: {}", dest, e))?;
        }

        tracing::info!(command = "install", source = %source, dest = %dest, "installed");

        let mut out = Map::new();
        out.insert("dest".to_string(), Value::String(dest.to_string()));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::fs;

    #[tokio::test]
    async fn test_install_copies_file() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("source.txt");
        let dst = tmp.path().join("dest.txt");
        fs::write(&src, b"hello").await.unwrap();

        let params: Map<String, Value> = serde_json::from_value(serde_json::json!({
            "source": src.to_str().unwrap(),
            "dest": dst.to_str().unwrap()
        })).unwrap();

        let cmd = InstallCommand;
        let result = cmd.execute(&params, &StateMap::new()).await.unwrap();
        assert_eq!(result["dest"].as_str().unwrap(), dst.to_str().unwrap());
        assert_eq!(fs::read(&dst).await.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn test_install_missing_source_is_error() {
        let tmp = TempDir::new().unwrap();
        let params: Map<String, Value> = serde_json::from_value(serde_json::json!({
            "source": "/nonexistent/file",
            "dest": tmp.path().join("out").to_str().unwrap()
        })).unwrap();
        assert!(InstallCommand.execute(&params, &StateMap::new()).await.is_err());
    }

    #[tokio::test]
    async fn test_install_missing_params_is_error() {
        let params = Map::new();
        assert!(InstallCommand.execute(&params, &StateMap::new()).await.is_err());
    }
}
