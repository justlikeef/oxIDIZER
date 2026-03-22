use serde_json::{Map, Value};
use async_trait::async_trait;
use super::{CommandPlugin, StateMap};

pub struct LogCommand;

#[async_trait]
impl CommandPlugin for LogCommand {
    fn name(&self) -> &str { "log_info" }

    async fn execute(&self, params: &Map<String, Value>, _state: &StateMap) -> anyhow::Result<Map<String, Value>> {
        let message = params.get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("(no message)");
        tracing::info!(command = "log_info", message = %message);
        Ok(Map::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_log_empty_params() {
        let cmd = LogCommand;
        let params = Map::new();
        let result = cmd.execute(&params, &StateMap::new()).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_log_with_message() {
        let cmd = LogCommand;
        let params: Map<String, Value> = serde_json::from_value(json!({"message": "hello"})).unwrap();
        let result = cmd.execute(&params, &StateMap::new()).await.unwrap();
        assert!(result.is_empty()); // log_info produces no output keys
    }
}
