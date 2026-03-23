use std::collections::HashMap;
use ox_cc_common::{CommandEntry, OnFailure};

use crate::commands::{StateMap, resolve};
use crate::substitute::{validate_syntax, substitute_params};
use crate::types::{CommandResult, CommandsetResult, CommandsetStatus, CommandStatus};

pub async fn run(commands: &[CommandEntry], plugin_dir: Option<&str>) -> CommandsetResult {
    // 1. Syntax-check all $variable references before executing anything
    for entry in commands {
        if let Err(e) = validate_syntax(&entry.params) {
            return CommandsetResult {
                status: CommandsetStatus::Failed,
                commands: vec![CommandResult {
                    command: entry.command.clone(),
                    status: CommandStatus::Failed,
                    output: None,
                    error: Some(format!("param syntax error: {}", e)),
                }],
            };
        }
    }

    let mut state: StateMap = HashMap::new();
    let mut results: Vec<CommandResult> = Vec::new();
    let mut failed = false;

    for entry in commands {
        if failed {
            results.push(CommandResult {
                command: entry.command.clone(),
                status: CommandStatus::Skipped,
                output: None,
                error: None,
            });
            continue;
        }

        // Resolve plugin
        let plugin = match resolve(&entry.command, plugin_dir) {
            Some(p) => p,
            None => {
                let result = CommandResult {
                    command: entry.command.clone(),
                    status: CommandStatus::Failed,
                    output: None,
                    error: Some(format!("unknown command '{}': no built-in or plugin binary found", entry.command)),
                };
                if entry.on_failure == OnFailure::Fail {
                    results.push(result);
                    failed = true;
                } else {
                    results.push(result);
                }
                continue;
            }
        };

        // Substitute $variable references at dispatch time
        let resolved_params = match substitute_params(&entry.params, &state) {
            Ok(p) => p,
            Err(e) => {
                let result = CommandResult {
                    command: entry.command.clone(),
                    status: CommandStatus::Failed,
                    output: None,
                    error: Some(format!("variable substitution failed: {}", e)),
                };
                if entry.on_failure == OnFailure::Fail {
                    results.push(result);
                    failed = true;
                } else {
                    results.push(result);
                }
                continue;
            }
        };

        // Execute
        match plugin.execute(&resolved_params, &state).await {
            Ok(output) => {
                // Merge output into state map
                for (k, v) in &output {
                    state.insert(k.clone(), v.clone());
                }
                results.push(CommandResult {
                    command: entry.command.clone(),
                    status: CommandStatus::Ok,
                    output: if output.is_empty() { None } else { Some(output) },
                    error: None,
                });
            }
            Err(e) => {
                let result = CommandResult {
                    command: entry.command.clone(),
                    status: CommandStatus::Failed,
                    output: None,
                    error: Some(e.to_string()),
                };
                if entry.on_failure == OnFailure::Fail {
                    results.push(result);
                    failed = true;
                } else {
                    results.push(result);
                }
            }
        }
    }

    CommandsetResult {
        status: if failed { CommandsetStatus::Failed } else { CommandsetStatus::Complete },
        commands: results,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ox_cc_common::{CommandEntry, OnFailure};
    use serde_json::{json, Map, Value};
    use crate::commands::CommandPlugin;
    use crate::types::CommandsetStatus;

    use async_trait::async_trait;

    // A command that always succeeds and writes a fixed key to the state map
    #[allow(dead_code)]
    struct OkCommand { key: String, value: Value }
    #[async_trait]
    impl CommandPlugin for OkCommand {
        fn name(&self) -> &str { "ok" }
        async fn execute(&self, _p: &Map<String, Value>, _s: &StateMap) -> anyhow::Result<Map<String, Value>> {
            let mut m = Map::new();
            m.insert(self.key.clone(), self.value.clone());
            Ok(m)
        }
    }

    // A command that always fails
    #[allow(dead_code)]
    struct FailCommand;
    #[async_trait]
    impl CommandPlugin for FailCommand {
        fn name(&self) -> &str { "fail" }
        async fn execute(&self, _p: &Map<String, Value>, _s: &StateMap) -> anyhow::Result<Map<String, Value>> {
            Err(anyhow::anyhow!("simulated failure"))
        }
    }

    #[allow(dead_code)]
    // A command that reads a $variable from params (requiring state from prior command)
    struct EchoCommand;
    #[async_trait]
    impl CommandPlugin for EchoCommand {
        fn name(&self) -> &str { "echo" }
        async fn execute(&self, p: &Map<String, Value>, _s: &StateMap) -> anyhow::Result<Map<String, Value>> {
            // Just echo all params back as output (post-substitution values)
            Ok(p.clone())
        }
    }

    fn entry(command: &str, on_failure: OnFailure) -> CommandEntry {
        CommandEntry { command: command.to_string(), on_failure, params: Map::new() }
    }

    // --- Tests ---

    #[tokio::test]
    async fn test_empty_commandset_is_complete() {
        let r = run(&[], None).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        assert!(r.commands.is_empty());
    }

    #[tokio::test]
    async fn test_unknown_command_with_fail_stops_execution() {
        let cmds = vec![
            entry("nonexistent_command", OnFailure::Fail),
            entry("log_info", OnFailure::Fail),
        ];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Failed);
        assert_eq!(r.commands[0].status, CommandStatus::Failed);
        assert_eq!(r.commands[1].status, CommandStatus::Skipped);
    }

    #[tokio::test]
    async fn test_unknown_command_with_continue_proceeds() {
        let cmds = vec![
            entry("nonexistent_command", OnFailure::Continue),
            entry("log_info", OnFailure::Fail),
        ];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        assert_eq!(r.commands[0].status, CommandStatus::Failed);
        assert_eq!(r.commands[1].status, CommandStatus::Ok);
    }

    #[tokio::test]
    async fn test_two_sequential_commands_both_succeed() {
        let cmds = vec![
            entry("log_info", OnFailure::Fail),
            entry("log_info", OnFailure::Fail),
        ];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        assert_eq!(r.commands.len(), 2);
        for cmd in &r.commands {
            assert_eq!(cmd.status, CommandStatus::Ok);
        }
        // log_info produces empty output
        for cmd in &r.commands {
            assert!(cmd.output.is_none());
        }
    }

    #[tokio::test]
    async fn test_output_from_command_is_captured_in_result() {
        // External process writes JSON to stdout; result.commands[0].output should contain it
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let script_path = tmp.path().join("producer");
        tokio::fs::write(&script_path, b"#!/bin/sh\necho '{\"file_path\":\"/tmp/foo.deb\"}'").await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let cmds = vec![entry("producer", OnFailure::Fail)];
        let r = run(&cmds, Some(tmp.path().to_str().unwrap())).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        let output = r.commands[0].output.as_ref().unwrap();
        assert_eq!(output["file_path"].as_str().unwrap(), "/tmp/foo.deb");
    }

    #[tokio::test]
    async fn test_bad_variable_syntax_fails_before_execution() {
        let mut params = Map::new();
        params.insert("bad".to_string(), json!("$")); // empty var name
        let cmds = vec![CommandEntry {
            command: "log_info".to_string(),
            on_failure: OnFailure::Fail,
            params,
        }];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Failed);
        assert!(r.commands[0].error.as_ref().unwrap().contains("syntax error"));
    }

    #[tokio::test]
    async fn test_missing_variable_at_dispatch_time() {
        let mut params = Map::new();
        params.insert("msg".to_string(), json!("$nonexistent_key")); // will fail at runtime
        let cmds = vec![CommandEntry {
            command: "log_info".to_string(),
            on_failure: OnFailure::Fail,
            params,
        }];
        let r = run(&cmds, None).await;
        // substitute_params fails at dispatch, resulting in failure
        // (log_info with a $ref that doesn't exist yet in state)
        assert_eq!(r.status, CommandsetStatus::Failed);
    }

    #[tokio::test]
    async fn test_commands_after_fail_are_skipped() {
        let cmds = vec![
            entry("log_info", OnFailure::Fail),
            entry("nonexistent_will_fail", OnFailure::Fail),
            entry("log_info", OnFailure::Fail),
        ];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Failed);
        assert_eq!(r.commands[0].status, CommandStatus::Ok);
        assert_eq!(r.commands[1].status, CommandStatus::Failed);
        assert_eq!(r.commands[2].status, CommandStatus::Skipped);
    }

    #[tokio::test]
    async fn test_continue_command_does_not_fail_commandset() {
        let cmds = vec![
            entry("nonexistent_will_fail", OnFailure::Continue),
            entry("log_info", OnFailure::Fail),
        ];
        let r = run(&cmds, None).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        assert_eq!(r.commands[1].status, CommandStatus::Ok);
    }

    #[tokio::test]
    async fn test_external_plugin_binary_via_plugin_dir() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();

        // Write a tiny shell script that echoes JSON to stdout
        let script_path = tmp.path().join("my_plugin");
        tokio::fs::write(&script_path, b"#!/bin/sh\necho '{\"result\":\"ok\"}'").await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let cmds = vec![entry("my_plugin", OnFailure::Fail)];
        let r = run(&cmds, Some(tmp.path().to_str().unwrap())).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        assert_eq!(r.commands[0].status, CommandStatus::Ok);
    }

    #[tokio::test]
    async fn test_prior_command_output_flows_to_next_via_variable() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();

        // Script 1: produces {"dest": "/tmp/test.deb"}
        let producer = tmp.path().join("producer");
        tokio::fs::write(&producer, b"#!/bin/sh\necho '{\"dest\":\"/tmp/test.deb\"}'").await.unwrap();

        // Script 2: reads stdin (the substituted params) and echoes it back as output.
        // We pass {"path": "$dest"} as params; after substitution the consumer receives
        // {"path": "/tmp/test.deb"} on stdin and echoes it back as output.
        let consumer = tmp.path().join("consumer");
        tokio::fs::write(&consumer, b"#!/bin/sh\ncat").await.unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&producer, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::set_permissions(&consumer, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let plugin_dir = tmp.path().to_str().unwrap();
        let cmds = vec![
            CommandEntry {
                command: "producer".to_string(),
                on_failure: OnFailure::Fail,
                params: Map::new(),
            },
            CommandEntry {
                command: "consumer".to_string(),
                on_failure: OnFailure::Fail,
                params: serde_json::from_value(json!({"path": "$dest"})).unwrap(),
            },
        ];

        let r = run(&cmds, Some(plugin_dir)).await;
        assert_eq!(r.status, CommandsetStatus::Complete);
        assert_eq!(r.commands[0].status, CommandStatus::Ok);
        assert_eq!(r.commands[1].status, CommandStatus::Ok);

        // The consumer echoed back its substituted params; "$dest" should have been replaced
        // with "/tmp/test.deb" from producer's output.
        let consumer_output = r.commands[1].output.as_ref().expect("consumer should have output");
        assert_eq!(consumer_output["path"].as_str().unwrap(), "/tmp/test.deb");
    }
}
