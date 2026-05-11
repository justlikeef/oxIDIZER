use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::accounting::AccountingEvent;
use ox_security_core::drivers::AccountingDriver;
use crate::event_serializer::serialize_event;

/// A function that receives a serialised JSON string for the accounting event.
/// In production this would write to a local audit table via the data crates.
pub type RecordFn = Arc<dyn Fn(String) + Send + Sync>;

/// Stub DB accounting driver. Serialises the event to a JSON string and calls
/// the injected `RecordFn`. Replace the function with a real data crate write
/// when the data layer is available.
pub struct DbAccountingDriver {
    record_fn: RecordFn,
}

impl DbAccountingDriver {
    pub fn new(record_fn: RecordFn) -> Self {
        Self { record_fn }
    }
}

#[async_trait]
impl AccountingDriver for DbAccountingDriver {
    async fn record(&self, event: &AccountingEvent) {
        let map = serialize_event(event);
        let json = serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string());
        (self.record_fn)(json);
    }
}
