/// ox_cc_admin_plugin — core admin business logic.
pub mod plugin;
pub mod config;
pub mod db;
pub mod handlers;
pub mod http_client;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub struct HandlerResponse {
    pub status: u16,
    pub body: String,
}
