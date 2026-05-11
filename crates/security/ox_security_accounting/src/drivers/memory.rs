use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use ox_security_core::accounting::AccountingEvent;
use ox_security_core::drivers::AccountingDriver;
use crate::event_serializer::serialize_event;

/// In-memory accounting driver intended for testing and development use only.
/// Serialises each event to a JSON string and stores in a shared Vec.
#[derive(Clone)]
pub struct MemoryAccountingDriver {
    store: Arc<Mutex<Vec<String>>>,
}

impl MemoryAccountingDriver {
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns a snapshot of all recorded events as JSON strings.
    pub fn events(&self) -> Vec<String> {
        self.store.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Default for MemoryAccountingDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AccountingDriver for MemoryAccountingDriver {
    async fn record(&self, event: &AccountingEvent) {
        let map = serialize_event(event);
        let json = serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string());
        self.store.lock().unwrap_or_else(|p| p.into_inner()).push(json);
    }
}
