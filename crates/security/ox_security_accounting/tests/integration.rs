use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::sync::Arc;
use async_trait::async_trait;
use chrono::Utc;
use ox_security_accounting::drivers::MemoryAccountingDriver;
use ox_security_accounting::AccountingPipeline;
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
use ox_security_accounting::drivers::RecordFn;
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

// ── Task 4 tests: TacacsAccountingDriver ─────────────────────────────────────

use ox_security_accounting::TacacsAccountingDriver;
use ox_security_accounting::drivers::tacacs::TacacsTcpSendFn;

fn tacacs_driver_with_capture() -> (TacacsAccountingDriver, Arc<Mutex<Vec<Vec<u8>>>>) {
    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();

    let send_fn: TacacsTcpSendFn = Arc::new(move |pkt: Vec<u8>| {
        captured_clone.lock().unwrap().push(pkt);
        // Return a minimal ACCT_REPLY: header (12) + body [status=SUCCESS, 0,0,0,0]
        let status = 0x01u8;
        let body = vec![status, 0x00, 0x00, 0x00, 0x00];
        let mut reply = vec![
            0xC1, 0x03, 0x02, 0x04,  // version, TYPE_ACCT, seq=2, flags=UNENCRYPTED
            0x00, 0x00, 0x00, 0x00,  // session_id (ignored in fire-and-forget)
            0x00, 0x00, 0x00, 0x05,  // body_len = 5
        ];
        reply.extend_from_slice(&body);
        Box::pin(async move { Ok(reply) })
    });

    let config = ox_security_accounting::drivers::tacacs::TacacsAccountingConfig {
        server: "127.0.0.1:49".to_string(),
        secret: secrecy::SecretString::new("test".to_string()),
        timeout_secs: 5,
        encrypted: false,
    };

    (TacacsAccountingDriver::new(config, send_fn), captured)
}

#[tokio::test]
async fn tacacs_accounting_sends_event_on_success() {
    let (driver, captured) = tacacs_driver_with_capture();
    driver.record(&test_event()).await;
    let pkts = captured.lock().unwrap();
    assert_eq!(pkts.len(), 1, "one ACCT_REQUEST packet must be sent");
    // packet type byte (index 1) must be 0x03 (ACCT)
    assert_eq!(pkts[0][1], 0x03, "packet type must be ACCT");
}

#[tokio::test]
async fn tacacs_accounting_ignores_send_failure() {
    let send_fn: TacacsTcpSendFn = Arc::new(|_pkt: Vec<u8>| {
        Box::pin(async { Err("connection refused".to_string()) })
    });
    let config = ox_security_accounting::drivers::tacacs::TacacsAccountingConfig {
        server: "127.0.0.1:49".to_string(),
        secret: secrecy::SecretString::new("test".to_string()),
        timeout_secs: 5,
        encrypted: false,
    };
    let driver = TacacsAccountingDriver::new(config, send_fn);
    // Must not panic or propagate error
    driver.record(&test_event()).await;
}
