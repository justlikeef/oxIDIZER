use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EventMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    pub headers: HashMap<String, String>,
    pub correlation_id: Option<String>,
    pub reply_to: Option<String>,
}

#[derive(Debug)]
pub enum BusError {
    ConnectionError(String),
    PublishError(String),
    RequestTimeout,
    SubscriptionError(String),
    SerializationError(String),
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for BusError {}

#[async_trait]
pub trait EventBus: Send + Sync {
    async fn publish(&self, topic: &str, payload: &[u8]) -> Result<(), BusError>;
    async fn request(&self, topic: &str, payload: &[u8], timeout: Duration) -> Result<EventMessage, BusError>;
    async fn subscribe(&self, topic: &str) -> Result<std::pin::Pin<Box<dyn Stream<Item = EventMessage> + Send>>, BusError>;
    async fn reply(&self, original: &EventMessage, payload: &[u8]) -> Result<(), BusError>;
}
