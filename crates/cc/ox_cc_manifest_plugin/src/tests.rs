use serde_json::Value;
use tempfile::NamedTempFile;

use crate::db::ManifestDb;
use crate::handlers;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn open_test_db() -> (ManifestDb, NamedTempFile) {
    let tmp = NamedTempFile::new().expect("tempfile");
    let db = ManifestDb::open(tmp.path().to_str().unwrap(), "testkey")
        .expect("open manifest db");
    (db, tmp)
}

fn make_wire(suffix: &str) -> String {
    // Fake wire string — handlers store it as-is, no validation at this layer
    format!("fakepayload{}.fakesig{}", suffix, suffix)
}

fn enroll(db: &ManifestDb, client_id: &str) {
    let body = serde_json::json!({
        "client_id": client_id,
        "enc_pubkey_b64": "fakeenc",
        "sig_pubkey_b64": "fakesig",
        "metadata": {}
    })
    .to_string();
    let cfg = crate::config::ManifestPluginConfig {
        db_path: "".to_string(),
        db_encryption_key: "".to_string(),
        max_manifest_window_secs: 100,
        broker_pubkeys: vec!["broker1".to_string()],
        manifest_url: "https://m.example.com".to_string(),
        report_url: "https://r.example.com".to_string(),
    };
    handlers::bootstrap_checkin(db, &cfg, &body);
    handlers::trust_client(db, client_id);
}

fn deploy(db: &ManifestDb, client_id: &str, manifest_id: &str) {
    // Ensure client is enrolled and trusted before deploying
    enroll(db, client_id);

    let body = serde_json::json!({
        "envelope_wire": make_wire(manifest_id),
        "manifest_id": manifest_id,
        "stored_by": "admin"
    })
    .to_string();
    let resp = handlers::deploy_envelope(db, client_id, &body);
    assert_eq!(resp.status, 200, "deploy failed: {}", resp.body);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn test_deploy_and_get_latest() {
    let (db, _tmp) = open_test_db();
    deploy(&db, "client-a", "manifest-1");

    let resp = handlers::get_latest(&db, "client-a");
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["client_id"], "client-a");
    assert_eq!(v["manifest_id"], "manifest-1");
    assert_eq!(v["envelope_wire"], make_wire("manifest-1").as_str());
}

#[test]
fn test_get_latest_not_found() {
    let (db, _tmp) = open_test_db();
    let resp = handlers::get_latest(&db, "nobody");
    assert_eq!(resp.status, 404);
}

#[test]
fn test_deploy_twice_only_latest_is_active() {
    let (db, _tmp) = open_test_db();
    deploy(&db, "client-a", "manifest-1");
    deploy(&db, "client-a", "manifest-2");

    // latest returns the second one
    let resp = handlers::get_latest(&db, "client-a");
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["manifest_id"], "manifest-2");
}

#[test]
fn test_get_latest_updates_last_polled_at() {
    let (db, _tmp) = open_test_db();
    deploy(&db, "client-a", "manifest-1");

    // First poll — last_polled_at is NULL
    let resp = handlers::get_latest(&db, "client-a");
    assert_eq!(resp.status, 200);

    // Second poll — last_polled_at should now be set
    let resp2 = handlers::get_latest(&db, "client-a");
    assert_eq!(resp2.status, 200);

    // Verify directly via DB that last_polled_at was set
    let conn = db.conn();
    let polled_at: Option<String> = conn
        .query_row(
            "SELECT last_polled_at FROM envelopes WHERE client_id = 'client-a' AND is_latest = 1",
            [],
            |row| row.get(0),
        )
        .expect("query");
    assert!(polled_at.is_some(), "last_polled_at should be set after polling");
}

#[test]
fn test_get_history_returns_all_envelopes() {
    let (db, _tmp) = open_test_db();
    deploy(&db, "client-a", "manifest-1");
    deploy(&db, "client-a", "manifest-2");
    deploy(&db, "client-a", "manifest-3");

    let resp = handlers::get_history(&db, "client-a");
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    let history = v["history"].as_array().unwrap();
    assert_eq!(history.len(), 3);

    // Only the latest should have is_latest = true
    let latest_count = history.iter().filter(|e| e["is_latest"] == true).count();
    assert_eq!(latest_count, 1);

    // Most recent first
    assert_eq!(history[0]["manifest_id"], "manifest-3");
}

#[test]
fn test_get_history_empty() {
    let (db, _tmp) = open_test_db();
    let resp = handlers::get_history(&db, "nobody");
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["history"].as_array().unwrap().len(), 0);
}

#[test]
fn test_expire_manifest() {
    let (db, _tmp) = open_test_db();
    deploy(&db, "client-a", "manifest-1");

    let resp = handlers::expire_manifest(&db, "client-a");
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["status"], "expired");

    // get_latest should now 404
    let resp2 = handlers::get_latest(&db, "client-a");
    assert_eq!(resp2.status, 404);
}

#[test]
fn test_expire_no_active_manifest() {
    let (db, _tmp) = open_test_db();
    let resp = handlers::expire_manifest(&db, "nobody");
    assert_eq!(resp.status, 404);
}

#[test]
fn test_expire_after_expire_is_idempotent_404() {
    let (db, _tmp) = open_test_db();
    deploy(&db, "client-a", "manifest-1");
    handlers::expire_manifest(&db, "client-a");
    // Second expire: no active manifest → 404
    let resp = handlers::expire_manifest(&db, "client-a");
    assert_eq!(resp.status, 404);
}

#[test]
fn test_list_clients() {
    let (db, _tmp) = open_test_db();
    deploy(&db, "client-a", "manifest-1");
    deploy(&db, "client-b", "manifest-2");

    let resp = handlers::list_clients(&db);
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    let clients = v["clients"].as_array().unwrap();
    assert_eq!(clients.len(), 2);
    let ids: Vec<&str> = clients.iter().map(|c| c["client_id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"client-a"));
    assert!(ids.contains(&"client-b"));
}

#[test]
fn test_list_clients_shows_all_enrolled() {
    let (db, _tmp) = open_test_db();
    deploy(&db, "client-a", "manifest-1");
    deploy(&db, "client-b", "manifest-2");
    handlers::expire_manifest(&db, "client-a");

    let resp = handlers::list_clients(&db);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    let clients = v["clients"].as_array().unwrap();
    assert_eq!(clients.len(), 2);
    
    let a = clients.iter().find(|c| c["client_id"] == "client-a").unwrap();
    assert!(a["latest_manifest_id"].is_null());

    let b = clients.iter().find(|c| c["client_id"] == "client-b").unwrap();
    assert_eq!(b["latest_manifest_id"], "manifest-2");
}

#[test]
fn test_get_client_status_no_reports() {
    let (db, _tmp) = open_test_db();
    deploy(&db, "client-a", "manifest-1");

    let resp = handlers::get_client_status(&db, "client-a");
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["client_id"], "client-a");
    assert_eq!(v["current_manifest"]["manifest_id"], "manifest-1");
    assert!(v["last_report"].is_null());
}

#[test]
fn test_get_client_status_not_found() {
    let (db, _tmp) = open_test_db();
    let resp = handlers::get_client_status(&db, "nobody");
    assert_eq!(resp.status, 404);
}

#[test]
fn test_deploy_bad_body() {
    let (db, _tmp) = open_test_db();
    let resp = handlers::deploy_envelope(&db, "client-a", "not-json");
    assert_eq!(resp.status, 400);
}
#[test]
fn test_bootstrap_flow() {
    let (db, _tmp) = open_test_db();
    let cfg = crate::config::ManifestPluginConfig {
        db_path: "".to_string(),
        db_encryption_key: "".to_string(),
        max_manifest_window_secs: 100,
        broker_pubkeys: vec!["broker1".to_string()],
        manifest_url: "https://m.example.com".to_string(),
        report_url: "https://r.example.com".to_string(),
    };

    // 1. Checkin
    let body = serde_json::json!({
        "client_id": "client-new",
        "enc_pubkey_b64": "pub1",
        "sig_pubkey_b64": "sig1",
        "metadata": {"hostname": "test"}
    }).to_string();
    let resp = handlers::bootstrap_checkin(&db, &cfg, &body);
    assert_eq!(resp.status, 200);

    // 2. Poll (should be Forbidden since status is pending)
    let resp = handlers::get_latest(&db, "client-new");
    assert_eq!(resp.status, 403);
    assert!(resp.body.contains("pending"));

    // 3. List pending
    let resp = handlers::list_pending_clients(&db);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["pending_clients"].as_array().unwrap().len(), 1);

    // 4. Trust
    let resp = handlers::trust_client(&db, "client-new");
    assert_eq!(resp.status, 200);

    // 5. Poll (now 404 since no manifest deployed yet, but no longer 403)
    let resp = handlers::get_latest(&db, "client-new");
    assert_eq!(resp.status, 404);
}
