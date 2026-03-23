use serde_json::{json, Value};
use tempfile::NamedTempFile;

use crate::db::AdminDb;
use crate::handlers;
use crate::http_client::HttpClient;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn open_test_db() -> (AdminDb, NamedTempFile) {
    let tmp = NamedTempFile::new().expect("tempfile");
    let db = AdminDb::open(tmp.path().to_str().unwrap(), "testkey")
        .expect("open admin db");
    (db, tmp)
}

fn make_config() -> crate::config::AdminPluginConfig {
    crate::config::AdminPluginConfig {
        db_path: ":memory:".to_string(),
        db_encryption_key: "key".to_string(),
        broker_url: "https://broker.internal".to_string(),
        manifest_instance_url: "https://manifest.example.com".to_string(),
        tls: crate::config::AdminTlsConfig {
            client_cert: "/dev/null".to_string(),
            client_key: "/dev/null".to_string(),
            ca_cert: "/dev/null".to_string(),
        },
    }
}

fn create_body(name: &str, client_ids: &[&str]) -> String {
    json!({
        "consumer": "test_consumer",
        "name": name,
        "description": "A test template",
        "expires_in_secs": 86400,
        "payload": { "pkg": "nginx" },
        "client_ids": client_ids,
        "created_by": "admin-user"
    })
    .to_string()
}

// ── Stub HttpClient implementations ──────────────────────────────────────────

struct OkClient {
    body: Value,
}
impl HttpClient for OkClient {
    fn get(&self, _url: &str) -> Result<Value, String> { Ok(self.body.clone()) }
    fn post(&self, _url: &str, _p: &Value) -> Result<Value, String> { Ok(self.body.clone()) }
    fn patch(&self, _url: &str, _p: &Value) -> Result<Value, String> { Ok(self.body.clone()) }
}

struct ErrClient;
impl HttpClient for ErrClient {
    fn get(&self, url: &str) -> Result<Value, String> { Err(format!("stubbed error for {}", url)) }
    fn post(&self, url: &str, _p: &Value) -> Result<Value, String> { Err(format!("stubbed error for {}", url)) }
    fn patch(&self, url: &str, _p: &Value) -> Result<Value, String> { Err(format!("stubbed error for {}", url)) }
}

/// Records POST calls made and returns configurable responses per-call.
struct RecordingClient {
    response: Value,
    pub calls: std::cell::RefCell<Vec<(String, Value)>>,
}
impl RecordingClient {
    fn new(response: Value) -> Self {
        Self { response, calls: std::cell::RefCell::new(vec![]) }
    }
    fn call_count(&self) -> usize { self.calls.borrow().len() }
}
impl HttpClient for RecordingClient {
    fn get(&self, url: &str) -> Result<Value, String> {
        self.calls.borrow_mut().push((url.to_string(), json!(null)));
        Ok(self.response.clone())
    }
    fn post(&self, url: &str, payload: &Value) -> Result<Value, String> {
        self.calls.borrow_mut().push((url.to_string(), payload.clone()));
        Ok(self.response.clone())
    }
    fn patch(&self, url: &str, payload: &Value) -> Result<Value, String> {
        self.calls.borrow_mut().push((url.to_string(), payload.clone()));
        Ok(self.response.clone())
    }
}

// ── DB-only handler tests ─────────────────────────────────────────────────────

#[test]
fn test_list_templates_empty() {
    let (db, _tmp) = open_test_db();
    let resp = handlers::list_templates(&db);
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["templates"].as_array().unwrap().len(), 0);
}

#[test]
fn test_get_template_not_found() {
    let (db, _tmp) = open_test_db();
    let resp = handlers::get_template(&db, "no-such-template");
    assert_eq!(resp.status, 404);
}

#[test]
fn test_create_template_bad_body() {
    let (db, _tmp) = open_test_db();
    let cfg = make_config();
    let resp = handlers::create_template(&db, &OkClient { body: json!({}) }, &cfg, "not-json");
    assert_eq!(resp.status, 400);
}

#[test]
fn test_create_template_broker_success() {
    let (db, _tmp) = open_test_db();
    let cfg = make_config();
    let client = OkClient { body: json!({ "ok": true }) };

    let resp = handlers::create_template(&db, &client, &cfg, &create_body("Template A", &["c1", "c2"]));
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["status"], "pending");
    assert!(v["template_id"].as_str().is_some());

    // Should appear in list_templates
    let list = handlers::list_templates(&db);
    let lv: Value = serde_json::from_str(&list.body).unwrap();
    assert_eq!(lv["templates"].as_array().unwrap().len(), 1);
    assert_eq!(lv["templates"][0]["status"], "pending");
}

#[test]
fn test_create_template_broker_failure_sets_draft() {
    let (db, _tmp) = open_test_db();
    let cfg = make_config();

    let resp = handlers::create_template(&db, &ErrClient, &cfg, &create_body("Template B", &["c1"]));
    assert_eq!(resp.status, 502);

    // Template should exist in DB with status = 'draft'
    let list = handlers::list_templates(&db);
    let lv: Value = serde_json::from_str(&list.body).unwrap();
    assert_eq!(lv["templates"][0]["status"], "draft");
}

#[test]
fn test_get_template_detail() {
    let (db, _tmp) = open_test_db();
    let cfg = make_config();
    let client = OkClient { body: json!({}) };
    handlers::create_template(&db, &client, &cfg, &create_body("My Template", &["client-a"]));

    // Get template_id from list
    let list = handlers::list_templates(&db);
    let lv: Value = serde_json::from_str(&list.body).unwrap();
    let tid = lv["templates"][0]["template_id"].as_str().unwrap();

    let resp = handlers::get_template(&db, tid);
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["name"], "My Template");
    assert_eq!(v["client_ids"].as_array().unwrap()[0], "client-a");
}

// ── Proxy handler tests (using stub HTTP client) ──────────────────────────────

#[test]
fn test_list_clients_proxies_to_broker() {
    let cfg = make_config();
    let client = RecordingClient::new(json!({ "clients": [] }));
    let resp = handlers::list_clients(&client, &cfg);
    assert_eq!(resp.status, 200);
    assert_eq!(client.call_count(), 1);
    assert!(client.calls.borrow()[0].0.contains("/broker/clients"));
}

#[test]
fn test_list_clients_502_on_broker_error() {
    let cfg = make_config();
    let resp = handlers::list_clients(&ErrClient, &cfg);
    assert_eq!(resp.status, 502);
}

#[test]
fn test_list_pending_proxies_to_broker() {
    let cfg = make_config();
    let client = RecordingClient::new(json!({ "pending": [] }));
    let resp = handlers::list_pending(&client, &cfg);
    assert_eq!(resp.status, 200);
    assert!(client.calls.borrow()[0].0.contains("/broker/pending"));
}

#[test]
fn test_approve_posts_to_broker() {
    let cfg = make_config();
    let client = RecordingClient::new(json!({ "approved": true }));
    let resp = handlers::approve(&client, &cfg, "tmpl-1", r#"{"approved_by":"user"}"#);
    assert_eq!(resp.status, 200);
    assert!(client.calls.borrow()[0].0.contains("/broker/pending/tmpl-1/approve"));
}

#[test]
fn test_reject_posts_to_broker() {
    let cfg = make_config();
    let client = RecordingClient::new(json!({ "rejected": true }));
    let resp = handlers::reject(&client, &cfg, "tmpl-1", r#"{"reason":"not authorized"}"#);
    assert_eq!(resp.status, 200);
    assert!(client.calls.borrow()[0].0.contains("/broker/pending/tmpl-1/reject"));
}

#[test]
fn test_manifest_expire_patches_manifest_instance() {
    let cfg = make_config();
    let client = RecordingClient::new(json!({ "status": "expired" }));
    let resp = handlers::manifest_expire(&client, &cfg, "client-a");
    assert_eq!(resp.status, 200);
    assert!(client.calls.borrow()[0].0.contains("/cc/manifest/client-a/expire"));
}

#[test]
fn test_deploy_stores_deployment_record() {
    let (db, _tmp) = open_test_db();
    let cfg = make_config();

    // First create a template so FK constraint is satisfied
    let template_id = {
        let c = OkClient { body: json!({}) };
        handlers::create_template(&db, &c, &cfg, &create_body("T", &["client-a"]));
        let list = handlers::list_templates(&db);
        let lv: Value = serde_json::from_str(&list.body).unwrap();
        lv["templates"][0]["template_id"].as_str().unwrap().to_string()
    };

    // Broker returns one approved envelope
    let broker_resp = json!({
        "envelopes": [{
            "client_id": "client-a",
            "envelope": { "manifest_id": "m-xyz" }
        }]
    });
    let client = RecordingClient::new(broker_resp);
    let resp = handlers::deploy(&db, &client, &cfg, &template_id);
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["deployed"], 1);
    assert_eq!(v["failures"].as_array().unwrap().len(), 0);

    // Deployment record exists in DB
    let count: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM manifest_deployments WHERE manifest_id = 'm-xyz'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);

    // Template status updated to 'deployed'
    let status: String = db.conn().query_row(
        "SELECT status FROM templates WHERE template_id = ?1",
        rusqlite::params![template_id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(status, "deployed");
}

fn stub_config() -> crate::config::AdminPluginConfig { make_config() }

// ── Session tests ─────────────────────────────────────────────────────────

#[test]
fn test_open_session_proxies_to_broker() {
    let tmp = NamedTempFile::new().unwrap();
    let db = AdminDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();

    let broker_resp = json!({"session_id": "sess-1", "status": "pending"});
    let client = RecordingClient::new(broker_resp);
    let config = stub_config();

    let body = json!({
        "client_ids": ["client-a"],
        "allowed_commands": ["download"],
        "created_by": "alice"
    }).to_string();

    let resp = handlers::open_session(&db, &client, &config, &body);
    assert_eq!(resp.status, 200);

    let calls = client.calls.borrow();
    assert_eq!(calls.len(), 1);
    let (url, payload) = &calls[0];
    assert!(url.contains("/broker/sessions"));
    assert_eq!(payload["client_ids"].as_array().unwrap().len(), 1);
    assert_eq!(payload["submitted_by"].as_str().unwrap(), "alice");
}

#[test]
fn test_approve_session_stores_token() {
    let tmp = NamedTempFile::new().unwrap();
    let db = AdminDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();

    db.conn().execute(
        "INSERT INTO sessions (session_id, created_at, created_by, client_ids, allowed_commands, status)
         VALUES ('sess-1','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','pending')",
        [],
    ).unwrap();

    let broker_resp = json!({"session_id": "sess-1", "token": "abc123token"});
    let client = RecordingClient::new(broker_resp);
    let config = stub_config();

    let body = json!({"actioned_by": "bob"}).to_string();
    let resp = handlers::approve_session(&db, &client, &config, "sess-1", &body);
    assert_eq!(resp.status, 200);

    let token: Option<String> = db.conn().query_row(
        "SELECT token FROM sessions WHERE session_id = 'sess-1'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(token.as_deref(), Some("abc123token"));

    let status: String = db.conn().query_row(
        "SELECT status FROM sessions WHERE session_id = 'sess-1'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(status, "approved");
}

#[test]
fn test_approve_session_502_when_broker_response_missing_token() {
    let tmp = NamedTempFile::new().unwrap();
    let db = AdminDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();

    db.conn().execute(
        "INSERT INTO sessions (session_id, created_at, created_by, client_ids, allowed_commands, status)
         VALUES ('sess-1','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','pending')",
        [],
    ).unwrap();

    // Broker response without a "token" field
    let broker_resp = json!({"session_id": "sess-1", "status": "approved"});
    let client = RecordingClient::new(broker_resp);
    let config = stub_config();

    let body = json!({"actioned_by": "bob"}).to_string();
    let resp = handlers::approve_session(&db, &client, &config, "sess-1", &body);
    assert_eq!(resp.status, 502, "should return 502 when broker response lacks token");

    // Admin DB status must remain unchanged
    let status: String = db.conn().query_row(
        "SELECT status FROM sessions WHERE session_id = 'sess-1'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(status, "pending", "DB status must not change when token is absent");
}

#[test]
fn test_close_session_proxies_to_broker_and_updates_local() {
    let tmp = NamedTempFile::new().unwrap();
    let db = AdminDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();

    db.conn().execute(
        "INSERT INTO sessions (session_id, created_at, created_by, client_ids, allowed_commands, status, token)
         VALUES ('sess-1','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','approved','mytoken')",
        [],
    ).unwrap();

    let broker_resp = json!({"session_id": "sess-1", "status": "closed"});
    let client = RecordingClient::new(broker_resp);
    let config = stub_config();

    let resp = handlers::close_session(&db, &client, &config, "sess-1");
    assert_eq!(resp.status, 200);

    let status: String = db.conn().query_row(
        "SELECT status FROM sessions WHERE session_id = 'sess-1'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(status, "closed");
}

#[test]
fn test_list_pending_admin_sessions_returns_only_pending() {
    let tmp = NamedTempFile::new().unwrap();
    let db = AdminDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();

    db.conn().execute(
        "INSERT INTO sessions (session_id, created_at, created_by, client_ids, allowed_commands, status)
         VALUES ('sess-pending','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','pending')",
        [],
    ).unwrap();
    db.conn().execute(
        "INSERT INTO sessions (session_id, created_at, created_by, client_ids, allowed_commands, status, token)
         VALUES ('sess-approved','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','approved','tok')",
        [],
    ).unwrap();

    let resp = handlers::list_pending_admin_sessions(&db);
    assert_eq!(resp.status, 200);
    let val: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    let sessions = val["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["session_id"].as_str().unwrap(), "sess-pending");
}

#[test]
fn test_submit_manifest_under_session_includes_token() {
    let tmp = NamedTempFile::new().unwrap();
    let db = AdminDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();

    db.conn().execute(
        "INSERT INTO sessions (session_id, created_at, created_by, client_ids, allowed_commands, status, token)
         VALUES ('sess-1','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','approved','secret-token')",
        [],
    ).unwrap();

    let broker_resp = json!({"template_id": "tmpl-1", "status": "approved", "session_id": "sess-1"});
    let client = RecordingClient::new(broker_resp);
    let config = stub_config();

    let body = json!({
        "consumer": "test",
        "name": "Session Manifest",
        "description": "submitted under session",
        "expires_in_secs": 3600,
        "payload": {"commandset": []},
        "client_id": "c1",
        "submitted_by": "alice"
    }).to_string();

    let resp = handlers::submit_session_manifest(&db, &client, &config, "sess-1", &body);
    assert_eq!(resp.status, 200);

    let calls = client.calls.borrow();
    let broker_call = calls.iter().find(|(url, _)| url.contains("/broker/request")).unwrap();
    assert_eq!(broker_call.1["session_token"].as_str().unwrap(), "secret-token");
}

#[test]
fn test_list_sessions_returns_all_rows() {
    let tmp = NamedTempFile::new().unwrap();
    let db = AdminDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();

    db.conn().execute(
        "INSERT INTO sessions (session_id, created_at, created_by, client_ids, allowed_commands, status)
         VALUES ('s1','2026-01-01T00:00:00Z','alice','[]','[]','pending')",
        [],
    ).unwrap();
    db.conn().execute(
        "INSERT INTO sessions (session_id, created_at, created_by, client_ids, allowed_commands, status)
         VALUES ('s2','2026-01-02T00:00:00Z','bob','[]','[]','approved')",
        [],
    ).unwrap();

    let resp = handlers::list_sessions(&db);
    assert_eq!(resp.status, 200);

    let val: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(val["sessions"].as_array().unwrap().len(), 2);
}

#[test]
fn test_reject_session_proxies_to_broker_and_updates_local() {
    let tmp = NamedTempFile::new().unwrap();
    let db = AdminDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();

    db.conn().execute(
        "INSERT INTO sessions (session_id, created_at, created_by, client_ids, allowed_commands, status)
         VALUES ('sess-1','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','pending')",
        [],
    ).unwrap();

    let broker_resp = json!({"session_id": "sess-1", "status": "rejected"});
    let client = RecordingClient::new(broker_resp);
    let body = json!({"actioned_by": "bob", "reason": "out of scope"}).to_string();
    let resp = handlers::reject_session(&db, &client, &stub_config(), "sess-1", &body);
    assert_eq!(resp.status, 200);

    let status: String = db.conn().query_row(
        "SELECT status FROM sessions WHERE session_id = 'sess-1'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(status, "rejected");
}

#[test]
fn test_sessions_table_exists() {
    let (db, _tmp) = open_test_db();
    db.conn().execute(
        "INSERT INTO sessions
         (session_id, created_at, created_by, client_ids, allowed_commands, status)
         VALUES ('s1','2026-01-01T00:00:00Z','alice','[]','[]','pending')",
        [],
    ).unwrap();

    let count: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM sessions WHERE session_id = 's1'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
}
