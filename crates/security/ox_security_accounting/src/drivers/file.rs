use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use async_trait::async_trait;
use ox_security_core::accounting::AccountingEvent;
use ox_security_core::drivers::AccountingDriver;
use crate::event_serializer::serialize_event;

/// Append-only JSON-lines file accounting driver.
/// Each event is serialised to a single JSON line and appended to the file.
/// The file is created if it does not exist. Errors are swallowed so driver
/// failures do not propagate to the request path.
pub struct FileAccountingDriver {
    path: PathBuf,
}

impl FileAccountingDriver {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl AccountingDriver for FileAccountingDriver {
    async fn record(&self, event: &AccountingEvent) {
        let map = serialize_event(event);
        let mut line = serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string());
        line.push('\n');

        // Open in append mode, create if absent.
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
}
