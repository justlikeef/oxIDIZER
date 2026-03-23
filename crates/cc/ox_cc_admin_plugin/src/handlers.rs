/// HTTP handlers for the admin plugin.
///
/// All routes under /admin/api/*. This plugin calls the broker and manifest
/// instance via mTLS using the injected `HttpClient`.
///
/// All handler functions that make outbound calls accept `client: &dyn HttpClient`
/// so tests can inject stubs without a live server.
use chrono::Utc;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::AdminPluginConfig;
use crate::db::AdminDb;
use crate::http_client::HttpClient;
use crate::HandlerResponse;

fn ok(body: Value) -> HandlerResponse {
    HandlerResponse { status: 200, body: body.to_string() }
}

fn err(status: u16, msg: &str) -> HandlerResponse {
    HandlerResponse { status, body: json!({ "error": msg }).to_string() }
}

// ── GET /admin/api/clients ───────────────────────────────────────────────────
// Proxies to broker: GET /broker/clients

pub fn list_clients(client: &dyn HttpClient, config: &AdminPluginConfig) -> HandlerResponse {
    let url = format!("{}/broker/clients", config.broker_url);
    match client.get(&url) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

// ── POST /admin/api/templates ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateTemplateRequest {
    consumer: String,
    name: String,
    description: String,
    expires_in_secs: i64,
    payload: Value,
    client_ids: Vec<String>,
    created_by: String,
}

pub fn create_template(
    db: &AdminDb,
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    body: &str,
) -> HandlerResponse {
    let req: CreateTemplateRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid body: {}", e)),
    };

    let template_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let client_ids_json = serde_json::to_string(&req.client_ids).unwrap_or_default();

    let conn = db.conn();
    if let Err(e) = conn.execute(
        "INSERT INTO templates
         (template_id, created_at, created_by, consumer, name, description,
          client_ids_json, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'submitted')",
        params![
            template_id, now, req.created_by, req.consumer, req.name,
            req.description, client_ids_json
        ],
    ) {
        return err(500, &format!("db: {}", e));
    }

    let broker_body = json!({
        "template_id": template_id,
        "consumer": req.consumer,
        "name": req.name,
        "description": req.description,
        "expires_in_secs": req.expires_in_secs,
        "payload": req.payload,
        "client_ids": req.client_ids,
        "submitted_by": req.created_by
    });

    let url = format!("{}/broker/request", config.broker_url);
    match client.post(&url, &broker_body) {
        Ok(_) => {
            let _ = conn.execute(
                "UPDATE templates SET status = 'pending' WHERE template_id = ?1",
                params![template_id],
            );
            ok(json!({ "template_id": template_id, "status": "pending" }))
        }
        Err(e) => {
            let _ = conn.execute(
                "UPDATE templates SET status = 'draft' WHERE template_id = ?1",
                params![template_id],
            );
            err(502, &format!("broker submission failed: {}", e))
        }
    }
}

// ── GET /admin/api/templates ─────────────────────────────────────────────────

pub fn list_templates(db: &AdminDb) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT template_id, created_at, created_by, consumer, name, status
         FROM templates ORDER BY created_at DESC",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "template_id": row.get::<_, String>(0)?,
                "created_at": row.get::<_, String>(1)?,
                "created_by": row.get::<_, String>(2)?,
                "consumer": row.get::<_, String>(3)?,
                "name": row.get::<_, String>(4)?,
                "status": row.get::<_, String>(5)?
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "templates": rows }))
}

// ── GET /admin/api/templates/{template_id} ───────────────────────────────────

pub fn get_template(db: &AdminDb, template_id: &str) -> HandlerResponse {
    let conn = db.conn();
    let row = conn.query_row(
        "SELECT template_id, created_at, created_by, consumer, name, description,
                client_ids_json, status, broker_status, rejected_reason, failed_client_ids
         FROM templates WHERE template_id = ?1",
        params![template_id],
        |row| Ok(json!({
            "template_id": row.get::<_, String>(0)?,
            "created_at": row.get::<_, String>(1)?,
            "created_by": row.get::<_, String>(2)?,
            "consumer": row.get::<_, String>(3)?,
            "name": row.get::<_, String>(4)?,
            "description": row.get::<_, String>(5)?,
            "client_ids": row.get::<_, String>(6)?
                .parse::<Value>().unwrap_or(Value::Array(vec![])),
            "status": row.get::<_, String>(7)?,
            "broker_status": row.get::<_, Option<String>>(8)?,
            "rejected_reason": row.get::<_, Option<String>>(9)?,
            "failed_client_ids": row.get::<_, Option<String>>(10)?
                .and_then(|s| s.parse::<Value>().ok())
                .unwrap_or(Value::Array(vec![]))
        })),
    );

    match row {
        Ok(v) => ok(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => err(404, "template not found"),
        Err(e) => err(500, &format!("db: {}", e)),
    }
}

// ── GET /admin/api/pending ───────────────────────────────────────────────────

pub fn list_pending(client: &dyn HttpClient, config: &AdminPluginConfig) -> HandlerResponse {
    let url = format!("{}/broker/pending", config.broker_url);
    match client.get(&url) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

// ── GET /admin/api/pending/{template_id} ────────────────────────────────────

pub fn get_pending(
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    template_id: &str,
) -> HandlerResponse {
    let url = format!("{}/broker/pending/{}", config.broker_url, template_id);
    match client.get(&url) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

// ── POST /admin/api/pending/{template_id}/approve ───────────────────────────

pub fn approve(
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    template_id: &str,
    body: &str,
) -> HandlerResponse {
    let body_val: Value = serde_json::from_str(body).unwrap_or(json!({}));
    let url = format!("{}/broker/pending/{}/approve", config.broker_url, template_id);
    match client.post(&url, &body_val) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

// ── POST /admin/api/pending/{template_id}/reject ─────────────────────────────

pub fn reject(
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    template_id: &str,
    body: &str,
) -> HandlerResponse {
    let body_val: Value = serde_json::from_str(body).unwrap_or(json!({}));
    let url = format!("{}/broker/pending/{}/reject", config.broker_url, template_id);
    match client.post(&url, &body_val) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

// ── GET /admin/api/approved ──────────────────────────────────────────────────

pub fn list_approved(client: &dyn HttpClient, config: &AdminPluginConfig) -> HandlerResponse {
    let url = format!("{}/broker/approved", config.broker_url);
    match client.get(&url) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

// ── POST /admin/api/approved/{template_id}/deploy ────────────────────────────

pub fn deploy(
    db: &AdminDb,
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    template_id: &str,
) -> HandlerResponse {
    // 1. Fetch approved envelopes from broker
    let url = format!("{}/broker/approved/{}", config.broker_url, template_id);
    let broker_resp = match client.get(&url) {
        Ok(v) => v,
        Err(e) => return err(502, &format!("broker fetch: {}", e)),
    };

    let envelopes = match broker_resp.get("envelopes").and_then(|v| v.as_array()) {
        Some(arr) => arr.clone(),
        None => return err(502, "broker response missing envelopes array"),
    };

    let conn = db.conn();
    let now = Utc::now().to_rfc3339();
    let mut deployed = 0usize;
    let mut failures: Vec<Value> = Vec::new();

    for entry in &envelopes {
        let client_id = match entry.get("client_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => continue,
        };
        let envelope = entry.get("envelope").cloned().unwrap_or(Value::Null);
        let manifest_id = envelope
            .get("manifest_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let deploy_body = json!({
            "envelope": envelope,
            "manifest_id": manifest_id,
            "stored_by": "admin"
        });
        let manifest_url = format!("{}/cc/manifest/{}", config.manifest_instance_url, client_id);
        match client.post(&manifest_url, &deploy_body) {
            Ok(_) => {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO manifest_deployments
                     (manifest_id, template_id, client_id, deployed_at, envelope_json)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        manifest_id,
                        template_id,
                        client_id,
                        now,
                        envelope.to_string()
                    ],
                );
                deployed += 1;
            }
            Err(e) => {
                failures.push(json!({ "client_id": client_id, "error": e }));
            }
        }
    }

    // Acknowledge to broker
    let ack_url = format!("{}/broker/approved/{}/acknowledge", config.broker_url, template_id);
    let _ = client.post(&ack_url, &json!({}));

    let new_status = if failures.is_empty() { "deployed" } else { "partially_deployed" };
    let _ = conn.execute(
        "UPDATE templates SET status = ?1 WHERE template_id = ?2",
        params![new_status, template_id],
    );

    ok(json!({
        "template_id": template_id,
        "deployed": deployed,
        "failures": failures
    }))
}

// ── GET /admin/api/audit ─────────────────────────────────────────────────────

pub fn query_audit(client: &dyn HttpClient, config: &AdminPluginConfig) -> HandlerResponse {
    let url = format!("{}/broker/audit", config.broker_url);
    match client.get(&url) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

// ── Manifest instance proxies ─────────────────────────────────────────────────

pub fn manifest_clients(client: &dyn HttpClient, config: &AdminPluginConfig) -> HandlerResponse {
    let url = format!("{}/cc/clients", config.manifest_instance_url);
    match client.get(&url) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

pub fn manifest_client_status(
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    client_id: &str,
) -> HandlerResponse {
    let url = format!("{}/cc/clients/{}/status", config.manifest_instance_url, client_id);
    match client.get(&url) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

pub fn manifest_client_history(
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    client_id: &str,
) -> HandlerResponse {
    let url = format!("{}/cc/manifest/{}/history", config.manifest_instance_url, client_id);
    match client.get(&url) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

pub fn manifest_reports(
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    client_id: &str,
) -> HandlerResponse {
    let url = format!("{}/cc/report/{}", config.manifest_instance_url, client_id);
    match client.get(&url) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

pub fn manifest_expire(
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    client_id: &str,
) -> HandlerResponse {
    let url = format!("{}/cc/manifest/{}/expire", config.manifest_instance_url, client_id);
    match client.patch(&url, &json!({})) {
        Ok(v) => ok(v),
        Err(e) => err(502, &e),
    }
}

// ── POST /admin/api/sessions ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OpenSessionRequest {
    client_ids: Vec<String>,
    allowed_commands: Vec<String>,
    expires_at: Option<String>,
    created_by: String,
}

pub fn open_session(
    db: &AdminDb,
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    body: &str,
) -> HandlerResponse {
    let req: OpenSessionRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid body: {}", e)),
    };

    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let client_ids_json = serde_json::to_string(&req.client_ids).expect("Vec<String> serialization is infallible");
    let commands_json = serde_json::to_string(&req.allowed_commands).expect("Vec<String> serialization is infallible");

    // Proxy to broker — map created_by → submitted_by
    let broker_payload = json!({
        "session_id": session_id,
        "submitted_by": req.created_by,
        "client_ids": req.client_ids,
        "allowed_commands": req.allowed_commands,
        "expires_at": req.expires_at,
    });

    let broker_url = format!("{}/broker/sessions", config.broker_url);
    if let Err(e) = client.post(&broker_url, &broker_payload) {
        return err(502, &format!("broker error: {}", e));
    }

    // Store locally
    let conn = db.conn();
    if let Err(e) = conn.execute(
        "INSERT INTO sessions (session_id, created_at, created_by, client_ids, allowed_commands, expires_at, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending')",
        params![session_id, now, req.created_by, client_ids_json, commands_json, req.expires_at],
    ) {
        return err(500, &format!("db: {}", e));
    }

    ok(json!({ "session_id": session_id, "status": "pending" }))
}

// ── GET /admin/api/sessions ───────────────────────────────────────────────────

pub fn list_sessions(db: &AdminDb) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT session_id, created_at, created_by, client_ids, allowed_commands, expires_at, status
         FROM sessions ORDER BY created_at DESC",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "session_id":       row.get::<_, String>(0)?,
                "created_at":       row.get::<_, String>(1)?,
                "created_by":       row.get::<_, String>(2)?,
                "client_ids":       serde_json::from_str::<Value>(&row.get::<_, String>(3)?).unwrap_or(Value::Null),
                "allowed_commands": serde_json::from_str::<Value>(&row.get::<_, String>(4)?).unwrap_or(Value::Null),
                "expires_at":       row.get::<_, Option<String>>(5)?,
                "status":           row.get::<_, String>(6)?,
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "sessions": rows }))
}

// ── GET /admin/api/sessions/pending ──────────────────────────────────────────

pub fn list_pending_admin_sessions(db: &AdminDb) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT session_id, created_at, created_by, client_ids, allowed_commands, expires_at
         FROM sessions WHERE status = 'pending'
         ORDER BY created_at ASC",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "session_id":       row.get::<_, String>(0)?,
                "created_at":       row.get::<_, String>(1)?,
                "created_by":       row.get::<_, String>(2)?,
                "client_ids":       serde_json::from_str::<Value>(&row.get::<_, String>(3)?).unwrap_or(Value::Null),
                "allowed_commands": serde_json::from_str::<Value>(&row.get::<_, String>(4)?).unwrap_or(Value::Null),
                "expires_at":       row.get::<_, Option<String>>(5)?,
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "sessions": rows }))
}

// ── POST /admin/api/sessions/{session_id}/approve ────────────────────────────

pub fn approve_session(
    db: &AdminDb,
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    session_id: &str,
    body: &str,
) -> HandlerResponse {
    let url = format!("{}/broker/sessions/pending/{}/approve", config.broker_url, session_id);
    let payload: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return err(400, &format!("invalid body: {}", e)),
    };

    let broker_resp = match client.post(&url, &payload) {
        Ok(v) => v,
        Err(e) => return err(502, &format!("broker error: {}", e)),
    };

    // Extract token from broker response — fail hard if absent so admin DB never de-syncs
    let token = match broker_resp.get("token").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return err(502, "broker approval response missing token field"),
    };

    match db.conn().execute(
        "UPDATE sessions SET status = 'approved', token = ?1 WHERE session_id = ?2",
        params![token, session_id],
    ) {
        Ok(_) => {}
        Err(e) => return err(500, &format!("db: {}", e)),
    }

    ok(broker_resp)
}

// ── POST /admin/api/sessions/{session_id}/reject ─────────────────────────────

pub fn reject_session(
    db: &AdminDb,
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    session_id: &str,
    body: &str,
) -> HandlerResponse {
    let url = format!("{}/broker/sessions/pending/{}/reject", config.broker_url, session_id);
    let payload: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return err(400, &format!("invalid body: {}", e)),
    };

    match client.post(&url, &payload) {
        Ok(v) => {
            if let Err(e) = db.conn().execute(
                "UPDATE sessions SET status = 'rejected' WHERE session_id = ?1",
                params![session_id],
            ) {
                return err(500, &format!("db: {}", e));
            }
            ok(v)
        }
        Err(e) => err(502, &format!("broker error: {}", e)),
    }
}

// ── POST /admin/api/sessions/{session_id}/close ───────────────────────────────

pub fn close_session(
    db: &AdminDb,
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    session_id: &str,
) -> HandlerResponse {
    let url = format!("{}/broker/sessions/{}/close", config.broker_url, session_id);
    match client.post(&url, &json!({})) {
        Ok(v) => {
            if let Err(e) = db.conn().execute(
                "UPDATE sessions SET status = 'closed' WHERE session_id = ?1",
                params![session_id],
            ) {
                return err(500, &format!("db: {}", e));
            }
            ok(v)
        }
        Err(e) => err(502, &format!("broker error: {}", e)),
    }
}

// ── POST /admin/api/sessions/{session_id}/manifests ──────────────────────────

#[derive(Debug, Deserialize)]
struct SessionManifestRequest {
    consumer: String,
    name: String,
    description: String,
    expires_in_secs: i64,
    payload: Value,
    client_id: String,
    submitted_by: String,
}

pub fn submit_session_manifest(
    db: &AdminDb,
    client: &dyn HttpClient,
    config: &AdminPluginConfig,
    session_id: &str,
    body: &str,
) -> HandlerResponse {
    let req: SessionManifestRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid body: {}", e)),
    };

    // Retrieve session token from admin DB
    let token: String = match db.conn().query_row(
        "SELECT token FROM sessions WHERE session_id = ?1 AND status = 'approved'",
        params![session_id],
        |row| row.get::<_, Option<String>>(0),
    ) {
        Ok(Some(t)) => t,
        Ok(None) => return err(403, "session has no token (not yet approved)"),
        Err(rusqlite::Error::QueryReturnedNoRows) => return err(404, "session not found or not approved"),
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let template_id = Uuid::new_v4().to_string();

    // Submit to broker with session token
    let broker_payload = json!({
        "template_id": template_id,
        "consumer": req.consumer,
        "name": req.name,
        "description": req.description,
        "expires_in_secs": req.expires_in_secs,
        "payload": req.payload,
        "client_ids": [req.client_id],
        "submitted_by": req.submitted_by,
        "session_token": token,
    });

    let broker_url = format!("{}/broker/request", config.broker_url);
    let broker_resp = match client.post(&broker_url, &broker_payload) {
        Ok(v) => v,
        Err(e) => return err(502, &format!("broker error: {}", e)),
    };

    ok(broker_resp)
}
