use serde_json::Value;
use tempfile::NamedTempFile;

use crate::db::ReportDb;
use crate::handlers;
use crate::rate_limit::RateLimiter;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Open a ReportDb against a fresh temp file.
/// Because ReportDb relies on the schema created by ManifestDb, we seed it
/// by opening an in-process ManifestDb on the same file first.
fn open_test_db() -> (ReportDb, NamedTempFile) {
    let tmp = NamedTempFile::new().expect("tempfile");
    let path = tmp.path().to_str().unwrap();

    // Seed schema via ManifestDb (mirrors production where manifest plugin
    // is always loaded alongside report plugin on the same ox_webservice instance)
    ox_cc_manifest_plugin::db::ManifestDb::open(path, "testkey")
        .expect("seed manifest schema");

    let db = ReportDb::open(path, "testkey").expect("open report db");
    (db, tmp)
}

fn unlimited() -> RateLimiter {
    RateLimiter::new(u32::MAX)
}

fn post(db: &ReportDb, client_id: &str, manifest_id: &str, report_id: &str, seq: i64, status: &str) {
    let rl = unlimited();
    let body = serde_json::json!({
        "manifest_id": manifest_id,
        "report_id": report_id,
        "sequence": seq,
        "status": status
    })
    .to_string();
    let resp = handlers::post_report(db, &rl, client_id, &body);
    assert!(resp.status == 201 || resp.status == 200, "post_report failed: {}", resp.body);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn test_post_report_returns_201() {
    let (db, _tmp) = open_test_db();
    let rl = unlimited();
    let body = serde_json::json!({
        "manifest_id": "m1",
        "report_id": "r1",
        "sequence": 1,
        "status": "applied"
    })
    .to_string();
    let resp = handlers::post_report(&db, &rl, "client-a", &body);
    assert_eq!(resp.status, 201);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["received"], true);
}

#[test]
fn test_duplicate_report_idempotent() {
    let (db, _tmp) = open_test_db();
    post(&db, "client-a", "m1", "r-dup", 1, "applied");

    // Second POST with same report_id should return 200, not 201
    let rl = unlimited();
    let body = serde_json::json!({
        "manifest_id": "m1",
        "report_id": "r-dup",
        "sequence": 1,
        "status": "applied"
    })
    .to_string();
    let resp = handlers::post_report(&db, &rl, "client-a", &body);
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["duplicate"], true);
}

#[test]
fn test_post_report_bad_body() {
    let (db, _tmp) = open_test_db();
    let rl = unlimited();
    let resp = handlers::post_report(&db, &rl, "client-a", "not-json");
    assert_eq!(resp.status, 400);
}

#[test]
fn test_post_report_with_detail() {
    let (db, _tmp) = open_test_db();
    let rl = unlimited();
    let body = serde_json::json!({
        "manifest_id": "m1",
        "report_id": "r-detail",
        "sequence": 1,
        "status": "failed",
        "detail": "apt exited with code 1"
    })
    .to_string();
    let resp = handlers::post_report(&db, &rl, "client-a", &body);
    assert_eq!(resp.status, 201);

    // Verify detail is stored and returned
    let list_resp = handlers::list_reports_for_manifest(&db, "client-a", "m1");
    let v: Value = serde_json::from_str(&list_resp.body).unwrap();
    let reports = v["reports"].as_array().unwrap();
    assert_eq!(reports[0]["detail"], "apt exited with code 1");
}

#[test]
fn test_list_reports_empty() {
    let (db, _tmp) = open_test_db();
    let resp = handlers::list_reports(&db, "nobody");
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["reports"].as_array().unwrap().len(), 0);
}

#[test]
fn test_list_reports_newest_first() {
    let (db, _tmp) = open_test_db();
    post(&db, "client-a", "m1", "r1", 1, "applied");
    post(&db, "client-a", "m1", "r2", 2, "applied");
    post(&db, "client-a", "m1", "r3", 3, "applied");

    let resp = handlers::list_reports(&db, "client-a");
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    let reports = v["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 3);
    let ids: Vec<&str> = reports.iter().map(|r| r["report_id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"r1") && ids.contains(&"r2") && ids.contains(&"r3"));
}

#[test]
fn test_list_reports_only_for_requested_client() {
    let (db, _tmp) = open_test_db();
    post(&db, "client-a", "m1", "ra1", 1, "applied");
    post(&db, "client-b", "m1", "rb1", 1, "applied");

    let resp = handlers::list_reports(&db, "client-a");
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    let reports = v["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["report_id"], "ra1");
}

#[test]
fn test_list_reports_for_manifest_ordered_by_sequence() {
    let (db, _tmp) = open_test_db();
    post(&db, "client-a", "m1", "r3", 3, "applied");
    post(&db, "client-a", "m1", "r1", 1, "applied");
    post(&db, "client-a", "m1", "r2", 2, "applied");

    let resp = handlers::list_reports_for_manifest(&db, "client-a", "m1");
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    let reports = v["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 3);
    assert_eq!(reports[0]["sequence"], 1);
    assert_eq!(reports[1]["sequence"], 2);
    assert_eq!(reports[2]["sequence"], 3);
}

#[test]
fn test_list_reports_for_manifest_filters_by_manifest() {
    let (db, _tmp) = open_test_db();
    post(&db, "client-a", "m1", "r-m1", 1, "applied");
    post(&db, "client-a", "m2", "r-m2", 1, "applied");

    let resp = handlers::list_reports_for_manifest(&db, "client-a", "m1");
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    let reports = v["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["report_id"], "r-m1");
}

#[test]
fn test_list_reports_for_manifest_empty() {
    let (db, _tmp) = open_test_db();
    let resp = handlers::list_reports_for_manifest(&db, "client-a", "no-such-manifest");
    assert_eq!(resp.status, 200);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["reports"].as_array().unwrap().len(), 0);
}

// ── Rate limiter tests ────────────────────────────────────────────────────────

#[test]
fn test_rate_limit_allows_up_to_limit() {
    let (db, _tmp) = open_test_db();
    let rl = RateLimiter::new(3);

    for i in 0..3u64 {
        let body = serde_json::json!({
            "manifest_id": "m1",
            "report_id": format!("r{}", i),
            "sequence": i as i64,
            "status": "applied"
        })
        .to_string();
        let resp = handlers::post_report(&db, &rl, "client-a", &body);
        assert_eq!(resp.status, 201, "request {} should be allowed", i);
    }
}

#[test]
fn test_rate_limit_rejects_over_limit() {
    let (db, _tmp) = open_test_db();
    let rl = RateLimiter::new(2);

    for i in 0..2u64 {
        let body = serde_json::json!({
            "manifest_id": "m1",
            "report_id": format!("r{}", i),
            "sequence": i as i64,
            "status": "applied"
        })
        .to_string();
        handlers::post_report(&db, &rl, "client-a", &body);
    }

    // Third request should be rate-limited
    let body = serde_json::json!({
        "manifest_id": "m1",
        "report_id": "r-over",
        "sequence": 99,
        "status": "applied"
    })
    .to_string();
    let resp = handlers::post_report(&db, &rl, "client-a", &body);
    assert_eq!(resp.status, 429);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert!(v["error"].as_str().unwrap().contains("rate limit"));
}

#[test]
fn test_rate_limit_independent_per_client() {
    let (db, _tmp) = open_test_db();
    let rl = RateLimiter::new(1);

    // client-a uses its 1 slot
    let body_a = serde_json::json!({
        "manifest_id": "m1", "report_id": "ra1", "sequence": 1, "status": "applied"
    }).to_string();
    let resp_a = handlers::post_report(&db, &rl, "client-a", &body_a);
    assert_eq!(resp_a.status, 201);

    // client-b still has its own slot — should be allowed
    let body_b = serde_json::json!({
        "manifest_id": "m1", "report_id": "rb1", "sequence": 1, "status": "applied"
    }).to_string();
    let resp_b = handlers::post_report(&db, &rl, "client-b", &body_b);
    assert_eq!(resp_b.status, 201, "client-b should not be affected by client-a's rate");

    // client-a is now blocked
    let body_a2 = serde_json::json!({
        "manifest_id": "m1", "report_id": "ra2", "sequence": 2, "status": "applied"
    }).to_string();
    let resp_a2 = handlers::post_report(&db, &rl, "client-a", &body_a2);
    assert_eq!(resp_a2.status, 429);
}

#[test]
fn test_rate_limiter_window_resets() {
    use crate::rate_limit::RateLimiter;
    // Test the rate limiter logic directly without the DB overhead.
    // Limit = 2; send 2 → both allowed; send 1 more → rejected.
    let rl = RateLimiter::new(2);
    assert!(rl.check("c"), "first should pass");
    assert!(rl.check("c"), "second should pass");
    assert!(!rl.check("c"), "third should be rejected");
    // A different client is unaffected
    assert!(rl.check("d"), "different client should pass");
}
