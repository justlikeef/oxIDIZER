/// ox_cc_manifest_plugin — core manifest instance business logic.
pub mod plugin;
pub mod config;
pub mod db;
pub mod handlers;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub struct HandlerResponse {
    pub status: u16,
    pub body: String,
}
