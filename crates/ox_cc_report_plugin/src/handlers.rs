/// HTTP handlers for the report plugin.
///
/// Routes:
///   POST /cc/report/{client_id}                    — client (or agent) posts progress
///   GET  /cc/report/{client_id}                    — admin: list all reports (paginated)
///   GET  /cc/report/{client_id}/{manifest_id}      — admin: reports for a manifest
use chrono::Utc;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::ReportDb;
use crate::rate_limit::RateLimiter;
use crate::HandlerResponse;

fn ok(body: Value) -> HandlerResponse {
    HandlerResponse { status: 200, body: body.to_string() }
}

fn err(status: u16, msg: &str) -> HandlerResponse {
    HandlerResponse { status, body: json!({ "error": msg }).to_string() }
}

// ── POST /cc/report/{client_id} ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ReportRequest {
    manifest_id: String,
    report_id: String,
    sequence: i64,
    status: String,
    detail: Option<String>,
}

pub fn post_report(db: &ReportDb, rate_limiter: &RateLimiter, client_id: &str, body: &str) -> HandlerResponse {
    if !rate_limiter.check(client_id) {
        return HandlerResponse {
            status: 429,
            body: json!({ "error": "rate limit exceeded" }).to_string(),
        };
    }

    let req: ReportRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid body: {}", e)),
    };

    let now = Utc::now().to_rfc3339();
    let conn = db.conn();

    let res = conn.execute(
        "INSERT INTO reports (client_id, manifest_id, report_id, sequence, received_at, status, detail)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            client_id,
            req.manifest_id,
            req.report_id,
            req.sequence,
            now,
            req.status,
            req.detail
        ],
    );

    match res {
        Ok(_) => HandlerResponse { status: 201, body: json!({ "received": true }).to_string() },
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.extended_code == 2067 => {
            // UNIQUE constraint (duplicate report_id) — idempotent, treat as success
            HandlerResponse { status: 200, body: json!({ "received": true, "duplicate": true }).to_string() }
        }
        Err(e) => err(500, &format!("db: {}", e)),
    }
}

// ── GET /cc/report/{client_id} ───────────────────────────────────────────────
// Role: admin cert; paginated, newest first

pub fn list_reports(db: &ReportDb, client_id: &str) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT manifest_id, report_id, sequence, received_at, status, detail
         FROM reports WHERE client_id = ?1 ORDER BY received_at DESC LIMIT 200",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map(params![client_id], |row| {
            Ok(json!({
                "manifest_id": row.get::<_, String>(0)?,
                "report_id": row.get::<_, String>(1)?,
                "sequence": row.get::<_, i64>(2)?,
                "received_at": row.get::<_, String>(3)?,
                "status": row.get::<_, String>(4)?,
                "detail": row.get::<_, Option<String>>(5)?
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "client_id": client_id, "reports": rows }))
}

// ── GET /cc/report/{client_id}/{manifest_id} ─────────────────────────────────
// Role: admin cert; ordered by sequence

pub fn list_reports_for_manifest(
    db: &ReportDb,
    client_id: &str,
    manifest_id: &str,
) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT report_id, sequence, received_at, status, detail
         FROM reports WHERE client_id = ?1 AND manifest_id = ?2 ORDER BY sequence ASC",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map(params![client_id, manifest_id], |row| {
            Ok(json!({
                "report_id": row.get::<_, String>(0)?,
                "sequence": row.get::<_, i64>(1)?,
                "received_at": row.get::<_, String>(2)?,
                "status": row.get::<_, String>(3)?,
                "detail": row.get::<_, Option<String>>(4)?
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "client_id": client_id, "manifest_id": manifest_id, "reports": rows }))
}
