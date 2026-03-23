use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

/// Status of the overall commandset execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandsetStatus {
    Complete,
    Failed,
}

/// Status of a single command within a commandset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Ok,
    Failed,
    Skipped,
}

/// Result of a single command execution.
#[derive(Debug, Clone, Serialize)]
pub struct CommandResult {
    pub command: String,
    pub status: CommandStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of an entire commandset execution.
#[derive(Debug, Clone, Serialize)]
pub struct CommandsetResult {
    pub status: CommandsetStatus,
    pub commands: Vec<CommandResult>,
}

impl CommandsetResult {
    /// Serialize to a JSON string suitable for the `detail` field of a report POST.
    pub fn to_detail_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| r#"{"status":"failed","commands":[]}"#.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_detail_json_complete() {
        let r = CommandsetResult {
            status: CommandsetStatus::Complete,
            commands: vec![CommandResult {
                command: "log_info".to_string(),
                status: CommandStatus::Ok,
                output: None,
                error: None,
            }],
        };
        let json = r.to_detail_json();
        assert!(json.contains("\"status\":\"complete\""));
        assert!(json.contains("\"command\":\"log_info\""));
    }

    #[test]
    fn test_to_detail_json_failed() {
        let r = CommandsetResult {
            status: CommandsetStatus::Failed,
            commands: vec![
                CommandResult {
                    command: "download".to_string(),
                    status: CommandStatus::Ok,
                    output: Some({
                        let mut m = serde_json::Map::new();
                        m.insert("dest".to_string(), Value::String("/tmp/x".to_string()));
                        m
                    }),
                    error: None,
                },
                CommandResult {
                    command: "install".to_string(),
                    status: CommandStatus::Failed,
                    output: None,
                    error: Some("permission denied".to_string()),
                },
            ],
        };
        let json = r.to_detail_json();
        assert!(json.contains("\"status\":\"failed\""));
        assert!(json.contains("\"error\":\"permission denied\""));
    }
}
