/// ox_cc_report_plugin — core report endpoint business logic.
pub mod plugin;
pub mod config;
pub mod db;
pub mod handlers;
pub mod rate_limit;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub struct HandlerResponse {
    pub status: u16,
    pub body: String,
}
