pub mod plugin;

/// ox_cc_broker_plugin — core broker business logic.
#[cfg(test)]
mod tests;
///
/// This crate is a plain Rust library. The HTTP layer (plugin or server)
/// is added separately once the plugin interface is finalised.
///
/// Public surface: config, db, policy, queue, signing, encrypt, handlers.
/// Callers pass a `BrokerDb` and `BrokerPluginConfig` into handler functions
/// and receive a `HandlerResponse` back.

pub mod config;
pub mod db;
pub mod encrypt;
pub mod handlers;
pub mod policy;
pub mod queue;
pub mod signing;

/// Uniform return value from every handler function.
/// The HTTP layer converts this to its native response type.
#[derive(Debug)]
pub struct HandlerResponse {
    pub status: u16,
    pub body: String,
}
