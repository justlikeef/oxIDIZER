use async_trait::async_trait;
use ox_security_core::accounting::AccountingEvent;
use ox_security_core::drivers::AccountingDriver;
use crate::event_serializer::serialize_event;

/// Stub syslog driver. Formats the event as a single line and writes it to
/// stderr. A production implementation would open a UDP or TCP socket to the
/// configured syslog endpoint and send a CEF / RFC 5424 formatted message.
pub struct SyslogAccountingDriver {
    target: String,
}

impl SyslogAccountingDriver {
    pub fn new(target: impl Into<String>) -> Self {
        Self { target: target.into() }
    }
}

#[async_trait]
impl AccountingDriver for SyslogAccountingDriver {
    async fn record(&self, event: &AccountingEvent) {
        let map = serialize_event(event);
        let json = serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string());
        eprintln!("[syslog stub -> {}] {}", self.target, json);
    }
}
