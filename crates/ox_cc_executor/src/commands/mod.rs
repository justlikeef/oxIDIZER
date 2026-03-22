use std::collections::HashMap;
use std::path::Path;
use serde_json::{Map, Value};
use async_trait::async_trait;

pub mod download;
pub mod install;
pub mod log;
pub mod os_info;
pub mod process;

pub use download::DownloadCommand;
pub use install::InstallCommand;
pub use log::LogCommand;
pub use os_info::OsInfoCommand;
pub use process::ProcessCommand;

/// Cumulative key-value store built up across command outputs.
pub type StateMap = HashMap<String, Value>;

#[async_trait]
pub trait CommandPlugin: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(
        &self,
        params: &Map<String, Value>,
        state: &StateMap,
    ) -> anyhow::Result<Map<String, Value>>;
}

/// Returns a built-in plugin for the given command name, or a `ProcessCommand`
/// if an executable with that name exists under `plugin_dir`.
pub fn resolve(command: &str, plugin_dir: Option<&str>) -> Option<Box<dyn CommandPlugin>> {
    match command {
        "log_info" => Some(Box::new(LogCommand)),
        "os_info"  => Some(Box::new(OsInfoCommand)),
        "download" => Some(Box::new(DownloadCommand)),
        "install"  => Some(Box::new(InstallCommand)),
        _ => {
            let dir = plugin_dir?;
            let path = Path::new(dir).join(command);
            if path.exists() {
                Some(Box::new(ProcessCommand { binary: path, name: command.to_string() }))
            } else {
                None
            }
        }
    }
}
