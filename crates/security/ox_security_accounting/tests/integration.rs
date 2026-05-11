use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::sync::Arc;
use async_trait::async_trait;
use chrono::Utc;
use ox_security_accounting::drivers::MemoryAccountingDriver;
use ox_security_accounting::pipeline::AccountingPipeline;
use ox_security_core::accounting::AccountingEvent;
use ox_security_core::drivers::AccountingDriver;
use ox_security_core::types::TenantId;

fn test_event() -> AccountingEvent {
    use ox_security_core::accounting::AuthOutcome;
    AccountingEvent {
        principal_id: None,
        auth_outcome: AuthOutcome::Authenticated,
        authz_outcome: None,
        call_context: "com.test.app".to_string(),
        object_fragment: None,
        operation_name: Some("read".to_string()),
        timestamp: Utc::now(),
        source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        session_id: None,
        tenant_id: TenantId::from_str("test").unwrap(),
    }
}

// ── Task 1 tests ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn memory_driver_records_events() {
    let driver = Arc::new(MemoryAccountingDriver::new());
    let pipeline = AccountingPipeline::new(vec![driver.clone()]);
    pipeline.record(&test_event()).await;
    let events = driver.events();
    assert_eq!(events.len(), 1);
    // stored as a JSON string — must be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&events[0]).unwrap();
    assert!(parsed.is_object());
}

#[tokio::test]
async fn pipeline_records_to_all_drivers() {
    let d1 = Arc::new(MemoryAccountingDriver::new());
    let d2 = Arc::new(MemoryAccountingDriver::new());
    let pipeline = AccountingPipeline::new(vec![d1.clone(), d2.clone()]);
    pipeline.record(&test_event()).await;
    assert_eq!(d1.events().len(), 1);
    assert_eq!(d2.events().len(), 1);
}

/// A driver that does nothing — used to confirm the pipeline keeps calling
/// subsequent drivers even when a preceding driver is a no-op or silent failure.
struct NoOpDriver;

#[async_trait]
impl AccountingDriver for NoOpDriver {
    async fn record(&self, _event: &AccountingEvent) {
        // intentionally does nothing
    }
}

#[tokio::test]
async fn pipeline_calls_all_drivers_even_when_one_is_noop() {
    let noop = Arc::new(NoOpDriver);
    let recording = Arc::new(MemoryAccountingDriver::new());
    let pipeline = AccountingPipeline::new(vec![noop, recording.clone()]);
    pipeline.record(&test_event()).await;
    assert_eq!(recording.events().len(), 1);
}

// ── Task 2 tests ──────────────────────────────────────────────────────────────

use ox_security_accounting::drivers::FileAccountingDriver;
use tempfile::TempDir;
use std::io::{BufRead, BufReader};
use std::fs;

#[tokio::test]
async fn file_driver_appends_json_lines() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("audit.log");
    let driver = FileAccountingDriver::new(path.clone());

    driver.record(&test_event()).await;
    driver.record(&test_event()).await;

    let file = fs::File::open(&path).unwrap();
    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .map(|l| l.unwrap())
        .collect();

    assert_eq!(lines.len(), 2, "expected 2 lines, got {}", lines.len());

    for line in &lines {
        let parsed: serde_json::Value = serde_json::from_str(line)
            .expect("each line must be valid JSON");
        assert!(parsed.is_object(), "each line must be a JSON object");
    }
}

#[tokio::test]
async fn file_driver_creates_file_if_missing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("subdir_that_exists").join("new_audit.log");
    // create the parent dir but not the file
    fs::create_dir_all(path.parent().unwrap()).unwrap();

    let driver = FileAccountingDriver::new(path.clone());
    driver.record(&test_event()).await;

    assert!(path.exists(), "driver must create the log file if it does not exist");
    let content = fs::read_to_string(&path).unwrap();
    assert!(!content.is_empty());
}

// ── Task 3 tests ──────────────────────────────────────────────────────────────

use ox_security_accounting::drivers::{DbAccountingDriver, SyslogAccountingDriver};
use ox_security_accounting::drivers::db::RecordFn;
use std::sync::Mutex;

#[tokio::test]
async fn syslog_driver_records_without_error() {
    let driver = SyslogAccountingDriver::new("localhost:514");
    // must not panic
    driver.record(&test_event()).await;
}

#[tokio::test]
async fn db_driver_calls_injected_fn() {
    let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let received_clone = received.clone();

    let record_fn: RecordFn = Arc::new(move |json: String| {
        received_clone.lock().unwrap().push(json);
    });

    let driver = DbAccountingDriver::new(record_fn);
    driver.record(&test_event()).await;

    let calls = received.lock().unwrap();
    assert_eq!(calls.len(), 1, "injected fn must be called exactly once");
    assert!(!calls[0].is_empty(), "injected fn must receive a non-empty string");

    // The string must be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&calls[0])
        .expect("injected fn must receive a valid JSON string");
    assert!(parsed.is_object());
}
