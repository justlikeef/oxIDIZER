use std::fs::File;
use std::io::Write;
use tempfile::tempdir;
use ox_webservice::{load_config_from_path, ConfigError};

#[test]
fn test_load_config_from_yaml() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("config.yaml");
    let mut file = File::create(&file_path).unwrap();
    writeln!(
        file,
        r#"
log4rs_config: "log.yaml"
servers:
  - protocol: "http"
    port: 8080
    bind_address: "127.0.0.1"
    hosts:
      - name: "localhost"
"#
    )
    .unwrap();

    let (config, _) = load_config_from_path(&file_path, "info").unwrap();
    assert_eq!(config.servers.len(), 1);
    assert_eq!(config.servers[0].port, 8080);
}

#[test]
fn test_load_config_from_json() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("config.json");
    let mut file = File::create(&file_path).unwrap();
    writeln!(
        file,
        r#"
{{
    "log4rs_config": "log.json",
    "servers": [
        {{
            "protocol": "http",
            "port": 8081,
            "bind_address": "0.0.0.0",
            "hosts": [
                {{"name": "example.com"}}
            ]
        }}
    ]
}}
"#
    )
    .unwrap();

    let (config, _) = load_config_from_path(&file_path, "info").unwrap();
    assert_eq!(config.servers.len(), 1);
    assert_eq!(config.servers[0].port, 8081);
    assert_eq!(config.servers[0].bind_address, "0.0.0.0");
}

#[test]
fn test_load_config_from_toml() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("config.toml");
    let mut file = File::create(&file_path).unwrap();
    writeln!(
        file,
        r#"
log4rs_config = "log.toml"

[[servers]]
protocol = "https"
port = 443
bind_address = "::1"

[[servers.hosts]]
name = "localhost"
"#
    )
    .unwrap();

    let (config, _) = load_config_from_path(&file_path, "info").unwrap();
    assert_eq!(config.servers.len(), 1);
    assert_eq!(config.servers[0].port, 443);
    assert_eq!(config.servers[0].protocol, "https");
}

#[test]
fn test_load_config_not_found() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("non_existent_config.yaml");
    let result = load_config_from_path(&file_path, "info");
    assert!(matches!(result, Err(ConfigError::NotFound)));
}

#[test]
fn test_load_config_invalid_extension() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("config.txt");
    let mut file = File::create(&file_path).unwrap();
    writeln!(file, "this is not a valid config file").unwrap();

    let result = load_config_from_path(&file_path, "info");
    assert!(matches!(result, Err(ConfigError::UnsupportedFileExtension)));
}

#[test]
fn test_load_config_invalid_content() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("config.yaml");
    let mut file = File::create(&file_path).unwrap();
    writeln!(file, "this is not a valid yaml file: [").unwrap();

    let result = load_config_from_path(&file_path, "info");
    assert!(matches!(result, Err(ConfigError::Deserialization(_))));
}
