/// HTTP request handlers for the broker plugin.
///
/// All handlers receive a shared DB reference and return a HandlerResponse
/// (status code + JSON body string). The dispatcher in lib.rs writes these
/// into the flow state.
///
/// mTLS role enforcement is noted at each endpoint but not yet implemented —
/// it requires ox_webservice mTLS support (see Open Questions in DESIGN.md).
use chrono::Utc;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::BrokerPluginConfig;
use crate::db::BrokerDb;
use crate::HandlerResponse;
use crate::policy;
use crate::queue;
use crate::signing;

fn ok(body: Value) -> HandlerResponse {
    HandlerResponse {
        status: 200,
        body: body.to_string(),
    }
}

fn err(status: u16, message: &str) -> HandlerResponse {
    HandlerResponse {
        status,
        body: json!({ "error": message }).to_string(),
    }
}

// ── GET /broker/healthz ─────────────────────────────────────────────────────

pub fn healthz() -> HandlerResponse {
    ok(json!({ "status": "ok" }))
}

// ── POST /broker/request ────────────────────────────────────────────────────
// Role: admin cert

#[derive(Debug, Deserialize)]
struct SubmitTemplateRequest {
    template_id: String,        // assigned by admin server before submission
    consumer: String,
    name: String,
    description: String,
    expires_in_secs: i64,
    payload: Value,
    client_ids: Vec<String>,
    submitted_by: String,       // TODO: replace with cert CN when mTLS is available
    session_token: Option<String>,   // present only for session-driven submissions
}

pub fn submit_template(
    db: &BrokerDb,
    config: &BrokerPluginConfig,
    body: &str,
) -> HandlerResponse {
    let req: SubmitTemplateRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid request body: {}", e)),
    };

    // Session token path: validate token, check scope, then sign immediately
    if let Some(token) = &req.session_token {
        return submit_template_with_session(db, config, &req, token);
    }

    // Policy validation — all clients must pass before any row is created
    if let Err(e) = policy::validate_name(&req.name) {
        return err(422, &e.to_string());
    }
    if let Err(e) = policy::validate_description(&req.description) {
        return err(422, &e.to_string());
    }
    if let Err(e) = policy::validate_clients(db.conn(), &req.client_ids) {
        return err(422, &e.to_string());
    }
    if let Err(e) = policy::validate_payload(&req.payload, &req.consumer, &config.policy) {
        return err(422, &e.to_string());
    }

    // Write payload to disk (not inline in DB)
    let payload_filename = format!("{}.json", req.template_id);
    let payload_path = std::path::Path::new(&config.payload_dir).join(&payload_filename);
    if let Err(e) = std::fs::write(&payload_path, req.payload.to_string()) {
        return err(500, &format!("failed to store payload: {}", e));
    }

    let conn = db.conn();
    let now = Utc::now().to_rfc3339();

    // Insert template row
    let res = conn.execute(
        "INSERT INTO manifest_templates
         (template_id, submitted_at, submitted_by, consumer, name, description,
          payload_path, expires_in_secs, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending')",
        params![
            req.template_id,
            now,
            req.submitted_by,
            req.consumer,
            req.name,
            req.description,
            payload_filename,
            req.expires_in_secs
        ],
    );
    if let Err(e) = res {
        return err(500, &format!("db error: {}", e));
    }

    // Create per-client signing_request rows
    if let Err(e) = queue::create_signing_requests(conn, &req.template_id, &req.client_ids) {
        return err(500, &format!("signing request creation failed: {}", e));
    }

    // Audit log
    let _ = conn.execute(
        "INSERT INTO audit_log (occurred_at, actor_cn, action, template_id, detail)
         VALUES (?1, ?2, 'submit_template', ?3, ?4)",
        params![
            now,
            req.submitted_by,
            req.template_id,
            json!({ "client_count": req.client_ids.len() }).to_string()
        ],
    );

    ok(json!({
        "template_id": req.template_id,
        "status": "pending",
        "client_count": req.client_ids.len()
    }))
}

fn submit_template_with_session(
    db: &BrokerDb,
    config: &BrokerPluginConfig,
    req: &SubmitTemplateRequest,
    token: &str,
) -> HandlerResponse {
    let conn = db.conn();

    // 1. Look up session by token
    let (session_id, status, client_ids_json, allowed_commands_json, expires_at): (String, String, String, String, Option<String>) =
        match conn.query_row(
            "SELECT session_id, status, client_ids, allowed_commands, expires_at
             FROM sessions WHERE token = ?1",
            params![token],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ) {
            Ok(r) => r,
            Err(rusqlite::Error::QueryReturnedNoRows) => return err(401, "invalid session token"),
            Err(e) => return err(500, &format!("db: {}", e)),
        };

    // 2. Check session is approved
    if status != "approved" {
        return err(403, &format!("session is '{}', not 'approved'", status));
    }

    // 3. Check expiry
    if let Some(exp) = &expires_at {
        match chrono::DateTime::parse_from_rfc3339(exp) {
            Ok(exp_time) => {
                if Utc::now() > exp_time.with_timezone(&Utc) {
                    let _ = conn.execute(
                        "UPDATE sessions SET status = 'expired' WHERE session_id = ?1",
                        params![session_id],
                    );
                    return err(403, "session has expired");
                }
            }
            Err(_) => {
                return err(500, "session has invalid expires_at timestamp");
            }
        }
    }

    // 4. Check all submitted client_ids are within session scope
    let session_clients: Vec<String> = serde_json::from_str(&client_ids_json).unwrap_or_default();
    for cid in &req.client_ids {
        if !session_clients.contains(cid) {
            return err(422, &format!("client_id '{}' is outside session scope", cid));
        }
    }

    // 5. Check all payload commandset commands are within allowed_commands
    let allowed: Vec<String> = serde_json::from_str(&allowed_commands_json).unwrap_or_default();
    if let Some(cs) = req.payload.get("commandset") {
        if let Some(commands) = cs.as_array() {
            for cmd in commands {
                if let Some(name) = cmd.get("command").and_then(|v| v.as_str()) {
                    if !allowed.contains(&name.to_string()) {
                        return err(422, &format!("command '{}' is outside session allowed_commands", name));
                    }
                }
            }
        }
    }

    // 6. Run existing policy engine checks
    if let Err(e) = policy::validate_name(&req.name) {
        return err(422, &e.to_string());
    }
    if let Err(e) = policy::validate_description(&req.description) {
        return err(422, &e.to_string());
    }
    if let Err(e) = policy::validate_clients(conn, &req.client_ids) {
        return err(422, &e.to_string());
    }
    if let Err(e) = policy::validate_payload(&req.payload, &req.consumer, &config.policy) {
        return err(422, &e.to_string());
    }

    // 7. Write payload to disk and create template row + signing requests
    let payload_filename = format!("{}.json", req.template_id);
    let payload_path = std::path::Path::new(&config.payload_dir).join(&payload_filename);
    if let Err(e) = std::fs::write(&payload_path, req.payload.to_string()) {
        return err(500, &format!("failed to store payload: {}", e));
    }

    let now = Utc::now().to_rfc3339();
    if let Err(e) = conn.execute(
        "INSERT INTO manifest_templates
         (template_id, submitted_at, submitted_by, consumer, name, description,
          payload_path, expires_in_secs, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending')",
        params![
            req.template_id, now, req.submitted_by, req.consumer,
            req.name, req.description, payload_filename, req.expires_in_secs
        ],
    ) {
        return err(500, &format!("db error: {}", e));
    }

    if let Err(e) = queue::create_signing_requests(conn, &req.template_id, &req.client_ids) {
        return err(500, &format!("signing request creation failed: {}", e));
    }

    // 8. Sign immediately (approve the template right away under session authority)
    let approve_body = json!({
        "actioned_by": format!("session:{}", session_id)
    }).to_string();
    let approve_resp = approve_template(db, config, &req.template_id, &approve_body);
    if approve_resp.status != 200 {
        return approve_resp;
    }

    let _ = conn.execute(
        "INSERT INTO audit_log (occurred_at, actor_cn, action, template_id, detail)
         VALUES (?1, ?2, 'session_submit', ?3, ?4)",
        params![
            now, req.submitted_by, req.template_id,
            json!({ "session_id": session_id, "client_count": req.client_ids.len() }).to_string()
        ],
    );

    let actual_status = serde_json::from_str::<Value>(&approve_resp.body)
        .ok()
        .and_then(|v| v["status"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "approved".to_string());

    ok(json!({
        "template_id": req.template_id,
        "status": actual_status,
        "session_id": session_id
    }))
}

// ── GET /broker/pending ─────────────────────────────────────────────────────
// Role: approver cert

pub fn list_pending(db: &BrokerDb) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT template_id, submitted_at, submitted_by, consumer, name, description
         FROM manifest_templates WHERE status = 'pending'
         ORDER BY submitted_at ASC",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "template_id": row.get::<_, String>(0)?,
                "submitted_at": row.get::<_, String>(1)?,
                "submitted_by": row.get::<_, String>(2)?,
                "consumer": row.get::<_, String>(3)?,
                "name": row.get::<_, String>(4)?,
                "description": row.get::<_, String>(5)?
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "pending": rows }))
}

// ── GET /broker/pending/{template_id} ───────────────────────────────────────
// Role: approver cert

pub fn get_pending(db: &BrokerDb, template_id: &str) -> HandlerResponse {
    let conn = db.conn();
    let row = conn.query_row(
        "SELECT template_id, submitted_at, submitted_by, consumer, name, description,
                expires_in_secs, payload_path
         FROM manifest_templates WHERE template_id = ?1 AND status = 'pending'",
        params![template_id],
        |row| {
            Ok(json!({
                "template_id": row.get::<_, String>(0)?,
                "submitted_at": row.get::<_, String>(1)?,
                "submitted_by": row.get::<_, String>(2)?,
                "consumer": row.get::<_, String>(3)?,
                "name": row.get::<_, String>(4)?,
                "description": row.get::<_, String>(5)?,
                "expires_in_secs": row.get::<_, i64>(6)?,
                "payload_path": row.get::<_, String>(7)?
            }))
        },
    );

    match row {
        Ok(v) => ok(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => err(404, "template not found or not pending"),
        Err(e) => err(500, &format!("db: {}", e)),
    }
}

// ── POST /broker/pending/{template_id}/approve ───────────────────────────────
// Role: approver cert

#[derive(Debug, Deserialize)]
struct ApproveRequest {
    actioned_by: String,    // TODO: replace with cert CN when mTLS is available
}

pub fn approve_template(
    db: &BrokerDb,
    config: &BrokerPluginConfig,
    template_id: &str,
    body: &str,
) -> HandlerResponse {
    let req: ApproveRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid request body: {}", e)),
    };

    let conn = db.conn();

    // Fetch template
    let (consumer, name, description, expires_in_secs, payload_filename): (String, String, String, i64, String) =
        match conn.query_row(
            "SELECT consumer, name, description, expires_in_secs, payload_path
             FROM manifest_templates WHERE template_id = ?1 AND status = 'pending'",
            params![template_id],
            |row| Ok((
                row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
            )),
        ) {
            Ok(r) => r,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return err(404, "template not found or not pending")
            }
            Err(e) => return err(500, &format!("db: {}", e)),
        };

    // Load payload from disk
    let payload_path = std::path::Path::new(&config.payload_dir).join(&payload_filename);
    let payload_json = match std::fs::read_to_string(&payload_path) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("failed to read payload: {}", e)),
    };

    // Sign all pending requests in the batch
    let (signed_count, failed_client_ids) = match signing::sign_batch(
        conn,
        config,
        template_id,
        &payload_json,
        &consumer,
        &name,
        &description,
        expires_in_secs,
    ) {
        Ok(r) => r,
        Err(e) => return err(500, &format!("signing batch failed: {}", e)),
    };

    let now = Utc::now().to_rfc3339();
    let new_status = if failed_client_ids.is_empty() {
        "approved"
    } else {
        "partially_approved"
    };

    let failed_json = serde_json::to_string(&failed_client_ids).unwrap_or_default();

    let _ = conn.execute(
        "UPDATE manifest_templates
         SET status = ?1, actioned_at = ?2, actioned_by = ?3, failed_client_ids = ?4
         WHERE template_id = ?5",
        params![new_status, now, req.actioned_by, failed_json, template_id],
    );

    // Audit log
    let _ = conn.execute(
        "INSERT INTO audit_log (occurred_at, actor_cn, action, template_id, detail)
         VALUES (?1, ?2, 'approve', ?3, ?4)",
        params![
            now,
            req.actioned_by,
            template_id,
            json!({
                "signed": signed_count,
                "failed": failed_client_ids
            })
            .to_string()
        ],
    );

    ok(json!({
        "template_id": template_id,
        "status": new_status,
        "signed_count": signed_count,
        "failed_client_ids": failed_client_ids
    }))
}

// ── POST /broker/pending/{template_id}/reject ────────────────────────────────
// Role: approver cert

#[derive(Debug, Deserialize)]
struct RejectRequest {
    actioned_by: String,
    reason: String,
}

pub fn reject_template(db: &BrokerDb, template_id: &str, body: &str) -> HandlerResponse {
    let req: RejectRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid request body: {}", e)),
    };

    let conn = db.conn();
    let now = Utc::now().to_rfc3339();

    let n = match conn.execute(
        "UPDATE manifest_templates
         SET status = 'rejected', actioned_at = ?1, actioned_by = ?2, rejected_reason = ?3
         WHERE template_id = ?4 AND status = 'pending'",
        params![now, req.actioned_by, req.reason, template_id],
    ) {
        Ok(n) => n,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    if n == 0 {
        return err(404, "template not found or not pending");
    }

    // Mark all signing_requests as failed
    let _ = conn.execute(
        "UPDATE signing_requests SET status = 'failed' WHERE template_id = ?1 AND status = 'pending'",
        params![template_id],
    );

    let _ = conn.execute(
        "INSERT INTO audit_log (occurred_at, actor_cn, action, template_id, detail)
         VALUES (?1, ?2, 'reject', ?3, ?4)",
        params![now, req.actioned_by, template_id, json!({ "reason": req.reason }).to_string()],
    );

    ok(json!({ "template_id": template_id, "status": "rejected" }))
}

// ── GET /broker/approved ─────────────────────────────────────────────────────
// Role: admin cert

pub fn list_approved(db: &BrokerDb) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT template_id, actioned_at, status, failed_client_ids
         FROM manifest_templates
         WHERE status IN ('approved', 'partially_approved')
         ORDER BY actioned_at DESC",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "template_id": row.get::<_, String>(0)?,
                "actioned_at": row.get::<_, String>(1)?,
                "status": row.get::<_, String>(2)?,
                "failed_client_ids": row.get::<_, Option<String>>(3)?
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                    .unwrap_or(Value::Array(vec![]))
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "approved": rows }))
}

// ── GET /broker/approved/{template_id} ───────────────────────────────────────
// Role: admin cert

pub fn get_approved(db: &BrokerDb, template_id: &str) -> HandlerResponse {
    let conn = db.conn();

    let template_row = conn.query_row(
        "SELECT template_id, actioned_at, status, failed_client_ids
         FROM manifest_templates
         WHERE template_id = ?1 AND status IN ('approved', 'partially_approved')",
        params![template_id],
        |row| {
            Ok(json!({
                "template_id": row.get::<_, String>(0)?,
                "actioned_at": row.get::<_, String>(1)?,
                "status": row.get::<_, String>(2)?,
                "failed_client_ids": row.get::<_, Option<String>>(3)?
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                    .unwrap_or(Value::Array(vec![]))
            }))
        },
    );

    let template = match template_row {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return err(404, "template not found or not approved")
        }
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    // Fetch approved envelopes
    let mut stmt = match conn.prepare(
        "SELECT client_id, envelope_json FROM signing_requests
         WHERE template_id = ?1 AND status = 'approved'",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    // envelope_json column stores the wire string (base64url.base64url), not JSON
    let envelopes: Vec<Value> = stmt
        .query_map(params![template_id], |row| {
            Ok(json!({
                "client_id": row.get::<_, String>(0)?,
                "envelope_wire": row.get::<_, Option<String>>(1)?
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({
        "template": template,
        "envelopes": envelopes
    }))
}

// ── POST /broker/approved/{template_id}/acknowledge ──────────────────────────
// Role: admin cert

pub fn acknowledge_approved(db: &BrokerDb, template_id: &str) -> HandlerResponse {
    let conn = db.conn();
    let now = Utc::now().to_rfc3339();

    let n = conn.execute(
        "UPDATE signing_requests SET delivered_at = ?1
         WHERE template_id = ?2 AND status = 'approved' AND delivered_at IS NULL",
        params![now, template_id],
    );

    match n {
        Ok(0) => err(404, "no unacknowledged approved requests for this template"),
        Ok(count) => ok(json!({ "acknowledged": count })),
        Err(e) => err(500, &format!("db: {}", e)),
    }
}

// ── POST /broker/clients ─────────────────────────────────────────────────────
// Role: admin cert
// Registers a new client or updates the X25519 pubkey for an existing client.

#[derive(Debug, Deserialize)]
struct RegisterClientRequest {
    client_id: String,
    enc_pubkey_b64: String,  // base64url-encoded X25519 public key (32 raw bytes)
    notes: Option<String>,
    enrolled_by: String,     // TODO: replace with cert CN / operator_id when JWT is wired
}

pub fn register_client(db: &BrokerDb, body: &str) -> HandlerResponse {
    let req: RegisterClientRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid request body: {}", e)),
    };

    // Validate the pubkey is base64url-decodable to exactly 32 bytes
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let pubkey_bytes = match URL_SAFE_NO_PAD.decode(&req.enc_pubkey_b64) {
        Ok(b) if b.len() == 32 => b,
        Ok(_) => return err(422, "enc_pubkey_b64 must decode to exactly 32 bytes"),
        Err(e) => return err(422, &format!("enc_pubkey_b64 is not valid base64url: {}", e)),
    };
    drop(pubkey_bytes); // validation only

    let conn = db.conn();
    let now = Utc::now().to_rfc3339();

    let is_update = conn
        .query_row(
            "SELECT COUNT(*) FROM clients WHERE client_id = ?1",
            params![req.client_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    let res = conn.execute(
        "INSERT INTO clients (client_id, enc_pubkey_b64, enrolled_at, enrolled_by, notes)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(client_id) DO UPDATE SET
             enc_pubkey_b64 = excluded.enc_pubkey_b64,
             enrolled_at    = excluded.enrolled_at,
             enrolled_by    = excluded.enrolled_by,
             notes          = excluded.notes",
        params![req.client_id, req.enc_pubkey_b64, now, req.enrolled_by, req.notes],
    );

    if let Err(e) = res {
        return err(500, &format!("db error: {}", e));
    }

    let _ = conn.execute(
        "INSERT INTO audit_log (occurred_at, actor_cn, action, client_id, detail)
         VALUES (?1, ?2, 'enroll_client', ?3, ?4)",
        params![
            now,
            req.enrolled_by,
            req.client_id,
            json!({ "update": is_update }).to_string()
        ],
    );

    let status = if is_update { "updated" } else { "enrolled" };
    HandlerResponse {
        status: if is_update { 200 } else { 201 },
        body: json!({ "client_id": req.client_id, "status": status }).to_string(),
    }
}

// ── GET /broker/clients ──────────────────────────────────────────────────────
// Role: admin cert

pub fn list_clients(db: &BrokerDb) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT client_id, enrolled_at, enrolled_by, notes FROM clients ORDER BY enrolled_at ASC",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "client_id": row.get::<_, String>(0)?,
                "enrolled_at": row.get::<_, String>(1)?,
                "enrolled_by": row.get::<_, String>(2)?,
                "notes": row.get::<_, Option<String>>(3)?
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "clients": rows }))
}

// ── POST /broker/sessions ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SubmitSessionRequest {
    session_id: String,
    submitted_by: String,
    client_ids: Vec<String>,
    allowed_commands: Vec<String>,
    expires_at: Option<String>,
}

pub fn submit_session(db: &BrokerDb, body: &str) -> HandlerResponse {
    let req: SubmitSessionRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid request body: {}", e)),
    };

    if req.client_ids.is_empty() {
        return err(422, "client_ids must not be empty");
    }
    if req.allowed_commands.is_empty() {
        return err(422, "allowed_commands must not be empty");
    }

    let conn = db.conn();
    let now = Utc::now().to_rfc3339();
    let client_ids_json = serde_json::to_string(&req.client_ids).expect("Vec<String> serialization is infallible");
    let commands_json = serde_json::to_string(&req.allowed_commands).expect("Vec<String> serialization is infallible");

    let res = conn.execute(
        "INSERT INTO sessions
         (session_id, submitted_at, submitted_by, client_ids, allowed_commands, expires_at, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending')",
        params![
            req.session_id, now, req.submitted_by,
            client_ids_json, commands_json, req.expires_at
        ],
    );

    match res {
        Ok(_) => ok(json!({ "session_id": req.session_id, "status": "pending" })),
        Err(e) => err(500, &format!("db error: {}", e)),
    }
}

// ── GET /broker/sessions/pending ────────────────────────────────────────────

pub fn list_pending_sessions(db: &BrokerDb) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT session_id, submitted_at, submitted_by, client_ids, allowed_commands, expires_at
         FROM sessions WHERE status = 'pending'
         ORDER BY submitted_at ASC",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "session_id":       row.get::<_, String>(0)?,
                "submitted_at":     row.get::<_, String>(1)?,
                "submitted_by":     row.get::<_, String>(2)?,
                "client_ids":       serde_json::from_str::<Value>(&row.get::<_, String>(3)?).unwrap_or(Value::Null),
                "allowed_commands": serde_json::from_str::<Value>(&row.get::<_, String>(4)?).unwrap_or(Value::Null),
                "expires_at":       row.get::<_, Option<String>>(5)?,
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "sessions": rows }))
}

// ── GET /broker/sessions/pending/{session_id} ────────────────────────────────

pub fn get_pending_session(db: &BrokerDb, session_id: &str) -> HandlerResponse {
    let conn = db.conn();
    match conn.query_row(
        "SELECT session_id, submitted_at, submitted_by, client_ids, allowed_commands, expires_at
         FROM sessions WHERE session_id = ?1 AND status = 'pending'",
        params![session_id],
        |row| Ok(json!({
            "session_id":       row.get::<_, String>(0)?,
            "submitted_at":     row.get::<_, String>(1)?,
            "submitted_by":     row.get::<_, String>(2)?,
            "client_ids":       serde_json::from_str::<Value>(&row.get::<_, String>(3)?).unwrap_or(Value::Null),
            "allowed_commands": serde_json::from_str::<Value>(&row.get::<_, String>(4)?).unwrap_or(Value::Null),
            "expires_at":       row.get::<_, Option<String>>(5)?,
        })),
    ) {
        Ok(v) => ok(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => err(404, "session not found or not pending"),
        Err(e) => err(500, &format!("db: {}", e)),
    }
}

// ── POST /broker/sessions/pending/{session_id}/approve ──────────────────────

#[derive(Debug, Deserialize)]
struct ApproveSessionRequest {
    actioned_by: String,
}

pub fn approve_session(db: &BrokerDb, session_id: &str, body: &str) -> HandlerResponse {
    let req: ApproveSessionRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid body: {}", e)),
    };

    let conn = db.conn();

    let (submitted_by, status): (String, String) = match conn.query_row(
        "SELECT submitted_by, status FROM sessions WHERE session_id = ?1",
        params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ) {
        Ok(r) => r,
        Err(_) => return err(404, "session not found"),
    };

    if status != "pending" {
        return err(409, &format!("session is '{}', not 'pending'", status));
    }

    if req.actioned_by == submitted_by {
        return err(403, "approver must differ from submitter (two-person rule)");
    }

    // Generate 32-byte random token
    let token = {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        URL_SAFE_NO_PAD.encode(bytes)
    };

    let now = Utc::now().to_rfc3339();
    match conn.execute(
        "UPDATE sessions SET status = 'approved', actioned_at = ?1, actioned_by = ?2, token = ?3
         WHERE session_id = ?4 AND status = 'pending'",
        params![now, req.actioned_by, token, session_id],
    ) {
        Ok(0) => return err(409, "session status changed concurrently; try again"),
        Ok(_) => {}
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    ok(json!({ "session_id": session_id, "token": token }))
}

// ── POST /broker/sessions/pending/{session_id}/reject ───────────────────────

#[derive(Debug, Deserialize)]
struct RejectSessionRequest {
    actioned_by: String,
    reason: String,
}

pub fn reject_session(db: &BrokerDb, session_id: &str, body: &str) -> HandlerResponse {
    let req: RejectSessionRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid body: {}", e)),
    };

    let conn = db.conn();
    let now = Utc::now().to_rfc3339();
    let n = match conn.execute(
        "UPDATE sessions SET status = 'rejected', actioned_at = ?1, actioned_by = ?2, rejected_reason = ?3
         WHERE session_id = ?4 AND status = 'pending'",
        params![now, req.actioned_by, req.reason, session_id],
    ) {
        Ok(n) => n,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    if n == 0 {
        return err(404, "session not found or not pending");
    }
    ok(json!({ "session_id": session_id, "status": "rejected" }))
}

// ── POST /broker/sessions/{session_id}/close ─────────────────────────────────

pub fn close_session(db: &BrokerDb, session_id: &str) -> HandlerResponse {
    let conn = db.conn();
    let now = Utc::now().to_rfc3339();
    let n = match conn.execute(
        "UPDATE sessions SET status = 'closed', actioned_at = ?1
         WHERE session_id = ?2 AND status = 'approved'",
        params![now, session_id],
    ) {
        Ok(n) => n,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    if n == 0 {
        return err(404, "session not found or not approved");
    }
    ok(json!({ "session_id": session_id, "status": "closed" }))
}

// ── GET /broker/audit ────────────────────────────────────────────────────────
// Role: admin cert

pub fn query_audit(db: &BrokerDb) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT occurred_at, actor_cn, action, template_id, client_id, detail
         FROM audit_log ORDER BY id DESC LIMIT 500",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "occurred_at": row.get::<_, String>(0)?,
                "actor_cn": row.get::<_, String>(1)?,
                "action": row.get::<_, String>(2)?,
                "template_id": row.get::<_, Option<String>>(3)?,
                "client_id": row.get::<_, Option<String>>(4)?,
                "detail": row.get::<_, Option<String>>(5)?
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "audit": rows }))
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use tempfile::NamedTempFile;
    use crate::db::BrokerDb;

    fn open_test_db() -> (BrokerDb, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let db = BrokerDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();
        (db, tmp)
    }

    #[test]
    fn test_submit_session_creates_pending_row() {
        let (db, _tmp) = open_test_db();
        let body = serde_json::json!({
            "session_id": "sess-1",
            "submitted_by": "alice",
            "client_ids": ["client-a"],
            "allowed_commands": ["download", "install"]
        }).to_string();

        let resp = submit_session(&db, &body);
        assert_eq!(resp.status, 200);

        let status: String = db.conn().query_row(
            "SELECT status FROM sessions WHERE session_id = 'sess-1'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "pending");
    }

    #[test]
    fn test_submit_session_bad_body() {
        let (db, _tmp) = open_test_db();
        let resp = submit_session(&db, "not-json");
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn test_list_sessions_pending() {
        let (db, _tmp) = open_test_db();
        db.conn().execute(
            "INSERT INTO sessions (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status)
             VALUES ('s1','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','pending')",
            [],
        ).unwrap();

        let resp = list_pending_sessions(&db);
        assert_eq!(resp.status, 200);
        let val: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(val["sessions"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_get_pending_session_returns_detail() {
        let (db, _tmp) = open_test_db();
        db.conn().execute(
            "INSERT INTO sessions (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status)
             VALUES ('s1','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','pending')",
            [],
        ).unwrap();

        let resp = get_pending_session(&db, "s1");
        assert_eq!(resp.status, 200);
        let val: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(val["session_id"].as_str().unwrap(), "s1");
        assert_eq!(val["submitted_by"].as_str().unwrap(), "alice");
    }

    #[test]
    fn test_get_pending_session_not_found() {
        let (db, _tmp) = open_test_db();
        let resp = get_pending_session(&db, "nonexistent");
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn test_approve_session_generates_token_and_enforces_two_person_rule() {
        let (db, _tmp) = open_test_db();
        db.conn().execute(
            "INSERT INTO sessions (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status)
             VALUES ('s1','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','pending')",
            [],
        ).unwrap();

        // Same person as submitter — must be rejected
        let body = serde_json::json!({"actioned_by": "alice"}).to_string();
        let resp = approve_session(&db, "s1", &body);
        assert_eq!(resp.status, 403);

        // Different person — should succeed
        let body = serde_json::json!({"actioned_by": "bob"}).to_string();
        let resp = approve_session(&db, "s1", &body);
        assert_eq!(resp.status, 200);

        let val: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        assert!(val["token"].as_str().is_some(), "token should be in response");

        let status: String = db.conn().query_row(
            "SELECT status FROM sessions WHERE session_id = 's1'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "approved");

        let token: Option<String> = db.conn().query_row(
            "SELECT token FROM sessions WHERE session_id = 's1'",
            [], |row| row.get(0),
        ).unwrap();
        assert!(token.is_some());
    }

    #[test]
    fn test_reject_session() {
        let (db, _tmp) = open_test_db();
        db.conn().execute(
            "INSERT INTO sessions (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status)
             VALUES ('s1','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','pending')",
            [],
        ).unwrap();

        let body = serde_json::json!({"actioned_by": "bob", "reason": "not approved"}).to_string();
        let resp = reject_session(&db, "s1", &body);
        assert_eq!(resp.status, 200);

        let (status, reason): (String, Option<String>) = db.conn().query_row(
            "SELECT status, rejected_reason FROM sessions WHERE session_id = 's1'",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(status, "rejected");
        assert_eq!(reason.as_deref(), Some("not approved"));
    }

    #[test]
    fn test_close_session() {
        let (db, _tmp) = open_test_db();
        db.conn().execute(
            "INSERT INTO sessions (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status, token)
             VALUES ('s1','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','approved','sometoken')",
            [],
        ).unwrap();

        let resp = close_session(&db, "s1");
        assert_eq!(resp.status, 200);

        let status: String = db.conn().query_row(
            "SELECT status FROM sessions WHERE session_id = 's1'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "closed");
    }

    #[test]
    fn test_submit_template_with_valid_session_token_signs_immediately() {
        let (db, _tmp) = open_test_db();
        let cfg = BrokerPluginConfig {
            db_path: ":memory:".to_string(),
            db_encryption_key: "testkey".to_string(),
            payload_dir: "/tmp".to_string(),
            signing_key_path: "/tmp/broker.key".to_string(),
            enc_key_path: "/tmp/broker_enc.key".to_string(),
            cipher: "aes256gcm".to_string(),
            pending_ttl_secs: 86_400,
            max_manifest_window_secs: 90 * 24 * 3600,
            policy: Default::default(),
        };

        // Enroll a client
        db.conn().execute(
            "INSERT INTO clients (client_id, enc_pubkey_b64, enrolled_at, enrolled_by)
             VALUES ('client-a', 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=', '2026-01-01T00:00:00Z', 'admin')",
            [],
        ).unwrap();

        // Create an approved session with a known token
        db.conn().execute(
            "INSERT INTO sessions
             (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status, token)
             VALUES ('sess-1','2026-01-01T00:00:00Z','alice','[\"client-a\"]','[\"download\"]','approved','valid-token-abc')",
            [],
        ).unwrap();

        let body = serde_json::json!({
            "template_id": "tmpl-1",
            "consumer": "test",
            "name": "Test Manifest",
            "description": "A test manifest for session authorization",
            "expires_in_secs": 3600,
            "payload": {"commandset": []},
            "client_ids": ["client-a"],
            "submitted_by": "alice",
            "session_token": "valid-token-abc"
        }).to_string();

        let resp = submit_template(&db, &cfg, &body);
        // Signing will fail (no real key), but should NOT get a 403 scope rejection
        assert_ne!(resp.status, 403, "should not get scope rejection for valid session");
    }

    #[test]
    fn test_submit_template_with_invalid_token_is_401() {
        let (db, _tmp) = open_test_db();
        let cfg = BrokerPluginConfig {
            db_path: ":memory:".to_string(),
            db_encryption_key: "testkey".to_string(),
            payload_dir: "/tmp".to_string(),
            signing_key_path: "/tmp/broker.key".to_string(),
            enc_key_path: "/tmp/broker_enc.key".to_string(),
            cipher: "aes256gcm".to_string(),
            pending_ttl_secs: 86_400,
            max_manifest_window_secs: 90 * 24 * 3600,
            policy: Default::default(),
        };

        let body = serde_json::json!({
            "template_id": "tmpl-1",
            "consumer": "test",
            "name": "Test Manifest",
            "description": "A test",
            "expires_in_secs": 3600,
            "payload": {},
            "client_ids": ["client-a"],
            "submitted_by": "alice",
            "session_token": "completely-invalid-token"
        }).to_string();

        let resp = submit_template(&db, &cfg, &body);
        assert_eq!(resp.status, 401);
    }

    #[test]
    fn test_submit_template_with_closed_session_is_403() {
        let (db, _tmp) = open_test_db();
        let cfg = BrokerPluginConfig {
            db_path: ":memory:".to_string(),
            db_encryption_key: "testkey".to_string(),
            payload_dir: "/tmp".to_string(),
            signing_key_path: "/tmp/broker.key".to_string(),
            enc_key_path: "/tmp/broker_enc.key".to_string(),
            cipher: "aes256gcm".to_string(),
            pending_ttl_secs: 86_400,
            max_manifest_window_secs: 90 * 24 * 3600,
            policy: Default::default(),
        };

        db.conn().execute(
            "INSERT INTO sessions
             (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status, token)
             VALUES ('sess-1','2026-01-01T00:00:00Z','alice','[\"client-a\"]','[\"download\"]','closed','closed-token')",
            [],
        ).unwrap();

        let body = serde_json::json!({
            "template_id": "tmpl-1",
            "consumer": "test",
            "name": "A",
            "description": "B",
            "expires_in_secs": 3600,
            "payload": {},
            "client_ids": ["client-a"],
            "submitted_by": "alice",
            "session_token": "closed-token"
        }).to_string();

        let resp = submit_template(&db, &cfg, &body);
        assert_eq!(resp.status, 403);
    }

    #[test]
    fn test_submit_template_session_client_id_outside_scope_is_422() {
        let (db, _tmp) = open_test_db();
        let cfg = BrokerPluginConfig {
            db_path: ":memory:".to_string(),
            db_encryption_key: "testkey".to_string(),
            payload_dir: "/tmp".to_string(),
            signing_key_path: "/tmp/broker.key".to_string(),
            enc_key_path: "/tmp/broker_enc.key".to_string(),
            cipher: "aes256gcm".to_string(),
            pending_ttl_secs: 86_400,
            max_manifest_window_secs: 90 * 24 * 3600,
            policy: Default::default(),
        };

        // Session allows only "client-a"
        db.conn().execute(
            "INSERT INTO sessions
             (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status, token)
             VALUES ('sess-1','2026-01-01T00:00:00Z','alice','[\"client-a\"]','[\"download\"]','approved','scope-token')",
            [],
        ).unwrap();
        // Enroll client-b (not in session)
        db.conn().execute(
            "INSERT INTO clients (client_id, enc_pubkey_b64, enrolled_at, enrolled_by)
             VALUES ('client-b', 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=', '2026-01-01T00:00:00Z', 'admin')",
            [],
        ).unwrap();

        let body = serde_json::json!({
            "template_id": "tmpl-1",
            "consumer": "test",
            "name": "A",
            "description": "B",
            "expires_in_secs": 3600,
            "payload": {},
            "client_ids": ["client-b"],
            "submitted_by": "alice",
            "session_token": "scope-token"
        }).to_string();

        let resp = submit_template(&db, &cfg, &body);
        assert_eq!(resp.status, 422);
        assert!(resp.body.contains("outside session scope"));
    }
}
