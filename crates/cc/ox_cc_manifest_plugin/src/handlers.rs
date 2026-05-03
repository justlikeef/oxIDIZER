/// HTTP handlers for the manifest plugin.
///
/// Routes:
///   POST   /cc/manifest/{client_id}           — admin deploys signed envelope
///   GET    /cc/manifest/{client_id}/latest     — client polls for current envelope
///   GET    /cc/manifest/{client_id}/history    — admin: list historical envelopes
///   PATCH  /cc/manifest/{client_id}/expire     — admin: expire current envelope
///   GET    /cc/clients                         — admin: list all enrolled clients
///   GET    /cc/clients/{client_id}/status      — admin: client status summary
use chrono::Utc;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};

use ox_cc_common::bootstrap::{BootstrapCheckinRequest, BootstrapCheckinResponse};
use crate::db::ManifestDb;
use crate::HandlerResponse;
use crate::config::ManifestPluginConfig;

fn ok(body: Value) -> HandlerResponse {
    HandlerResponse { status: 200, body: body.to_string() }
}

fn err(status: u16, msg: &str) -> HandlerResponse {
    HandlerResponse { status, body: json!({ "error": msg }).to_string() }
}

// ── POST /cc/manifest/{client_id} ───────────────────────────────────────────
// Role: admin cert

#[derive(Debug, Deserialize)]
struct DeployRequest {
    /// Wire string: base64url(envelope_json).base64url(signature)
    envelope_wire: String,
    manifest_id: String,
    stored_by: String,  // TODO: replace with cert CN when mTLS is available
}

pub fn deploy_envelope(db: &ManifestDb, client_id: &str, body: &str) -> HandlerResponse {
    let req: DeployRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid body: {}", e)),
    };

    let conn = db.conn();
    let now = Utc::now().to_rfc3339();

    // Verify client exists
    let client_exists: i64 = match conn.query_row(
        "SELECT COUNT(*) FROM clients WHERE client_id = ?1",
        params![client_id],
        |row| row.get(0),
    ) {
        Ok(n) => n,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    if client_exists == 0 {
        return err(404, "client not enrolled; perform bootstrap first");
    }

    // Clear is_latest on any previous envelope for this client
    let _ = conn.execute(
        "UPDATE envelopes SET is_latest = 0 WHERE client_id = ?1 AND is_latest = 1",
        params![client_id],
    );

    let res = conn.execute(
        "INSERT INTO envelopes (client_id, manifest_id, stored_at, stored_by, envelope_json, is_latest)
         VALUES (?1, ?2, ?3, ?4, ?5, 1)",
        params![client_id, req.manifest_id, now, req.stored_by, req.envelope_wire],
    );

    match res {
        Ok(_) => ok(json!({ "client_id": client_id, "manifest_id": req.manifest_id, "stored_at": now })),
        Err(e) => err(500, &format!("db: {}", e)),
    }
}

// ── GET /cc/manifest/{client_id}/latest ─────────────────────────────────────
// Role: client TLS cert (CN must match client_id — enforced by ox_webservice mTLS, pending)

pub fn get_latest(db: &ManifestDb, client_id: &str) -> HandlerResponse {
    let conn = db.conn();
    let now = Utc::now().to_rfc3339();

    // Check trust status
    let status: String = match conn.query_row(
        "SELECT status FROM clients WHERE client_id = ?1",
        params![client_id],
        |row| row.get(0),
    ) {
        Ok(s) => s,
        Err(rusqlite::Error::QueryReturnedNoRows) => return err(404, "client not found"),
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    if status != "trusted" {
        return err(403, &format!("client status is '{}'; awaiting administrator approval", status));
    }

    let row = conn.query_row(
        "SELECT id, manifest_id, envelope_json, stored_at, last_polled_at
         FROM envelopes WHERE client_id = ?1 AND is_latest = 1",
        params![client_id],
        |row| Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
        )),
    );

    match row {
        Err(rusqlite::Error::QueryReturnedNoRows) => err(404, "no manifest for client"),
        Err(e) => err(500, &format!("db: {}", e)),
        Ok((id, manifest_id, envelope_wire, stored_at, _prev_polled)) => {
            // Update last_polled_at on every client GET (even if content is unchanged)
            let _ = conn.execute(
                "UPDATE envelopes SET last_polled_at = ?1 WHERE id = ?2",
                params![now, id],
            );

            // Return the wire string as-is; client verifies signature before parsing
            ok(json!({
                "client_id": client_id,
                "manifest_id": manifest_id,
                "stored_at": stored_at,
                "envelope_wire": envelope_wire
            }))
        }
    }
}

// ── GET /cc/manifest/{client_id}/history ─────────────────────────────────────
// Role: admin cert

pub fn get_history(db: &ManifestDb, client_id: &str) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT manifest_id, stored_at, stored_by, is_latest
         FROM envelopes WHERE client_id = ?1 ORDER BY stored_at DESC",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map(params![client_id], |row| {
            Ok(json!({
                "manifest_id": row.get::<_, String>(0)?,
                "stored_at": row.get::<_, String>(1)?,
                "stored_by": row.get::<_, String>(2)?,
                "is_latest": row.get::<_, i64>(3)? == 1
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "client_id": client_id, "history": rows }))
}

// ── PATCH /cc/manifest/{client_id}/expire ────────────────────────────────────
// Role: admin cert — sets expires_at to a past timestamp (effective revocation)
// Does not delete history.

pub fn expire_manifest(db: &ManifestDb, client_id: &str) -> HandlerResponse {
    let conn = db.conn();

    // The expires_at field lives inside the envelope_json. We update a column
    // to flag effective expiry without modifying the signed envelope (which
    // would invalidate the Ed25519 signature). Instead, the client re-checks
    // expiry from the outer envelope; the manifest plugin marks is_latest = 0
    // to signal to the client that this envelope is revoked.
    let n = match conn.execute(
        "UPDATE envelopes SET is_latest = 0
         WHERE client_id = ?1 AND is_latest = 1",
        params![client_id],
    ) {
        Ok(n) => n,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    if n == 0 {
        err(404, "no active manifest to expire for this client")
    } else {
        ok(json!({ "client_id": client_id, "status": "expired" }))
    }
}

// ── GET /cc/clients ───────────────────────────────────────────────────────────
// Role: admin cert

pub fn list_clients(db: &ManifestDb) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT c.client_id, c.status, c.created_at, e.manifest_id, e.last_polled_at
         FROM clients c
         LEFT JOIN envelopes e ON c.client_id = e.client_id AND e.is_latest = 1
         ORDER BY c.client_id ASC",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "client_id": row.get::<_, String>(0)?,
                "status": row.get::<_, String>(1)?,
                "created_at": row.get::<_, String>(2)?,
                "latest_manifest_id": row.get::<_, Option<String>>(3)?,
                "last_polled_at": row.get::<_, Option<String>>(4)?
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "clients": rows }))
}

// ── GET /cc/clients/{client_id}/status ───────────────────────────────────────
// Role: admin cert

pub fn get_client_status(db: &ManifestDb, client_id: &str) -> HandlerResponse {
    let conn = db.conn();

    let env_row = conn.query_row(
        "SELECT manifest_id, stored_at, last_polled_at FROM envelopes
         WHERE client_id = ?1 AND is_latest = 1",
        params![client_id],
        |row| Ok(json!({
            "manifest_id": row.get::<_, String>(0)?,
            "stored_at": row.get::<_, String>(1)?,
            "last_polled_at": row.get::<_, Option<String>>(2)?
        })),
    );

    let last_report = conn.query_row(
        "SELECT status, received_at FROM reports
         WHERE client_id = ?1 ORDER BY received_at DESC LIMIT 1",
        params![client_id],
        |row| Ok(json!({
            "status": row.get::<_, String>(0)?,
            "received_at": row.get::<_, String>(1)?
        })),
    ).ok();

    match env_row {
        Err(rusqlite::Error::QueryReturnedNoRows) => err(404, "client not found"),
        Err(e) => err(500, &format!("db: {}", e)),
        Ok(env) => ok(json!({
            "client_id": client_id,
            "current_manifest": env,
            "last_report": last_report
        })),
    }
}
// ── Bootstrap Handlers ──────────────────────────────────────────────────────

pub fn bootstrap_checkin(db: &ManifestDb, config: &ManifestPluginConfig, body: &str) -> HandlerResponse {
    let req: BootstrapCheckinRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return err(400, &format!("invalid body: {}", e)),
    };

    let conn = db.conn();
    let now = Utc::now().to_rfc3339();

    // Insert or update client record. Initial status is 'pending'.
    let res = conn.execute(
        "INSERT INTO clients (client_id, enc_pubkey_b64, sig_pubkey_b64, status, created_at, last_seen_at)
         VALUES (?1, ?2, ?3, 'pending', ?4, ?4)
         ON CONFLICT(client_id) DO UPDATE SET
            enc_pubkey_b64 = excluded.enc_pubkey_b64,
            sig_pubkey_b64 = excluded.sig_pubkey_b64,
            last_seen_at = excluded.last_seen_at",
        params![req.client_id, req.enc_pubkey_b64, req.sig_pubkey_b64, now],
    );

    match res {
        Ok(_) => ok(json!(BootstrapCheckinResponse {
            broker_pubkeys: config.broker_pubkeys.clone(),
            manifest_url: config.manifest_url.clone(),
            report_url: config.report_url.clone(),
            config_overrides: None,
        })),
        Err(e) => err(500, &format!("db: {}", e)),
    }
}

pub fn trust_client(db: &ManifestDb, client_id: &str) -> HandlerResponse {
    let conn = db.conn();
    let res = conn.execute(
        "UPDATE clients SET status = 'trusted' WHERE client_id = ?1",
        params![client_id],
    );

    match res {
        Ok(0) => err(404, "client not found"),
        Ok(_) => ok(json!({ "client_id": client_id, "status": "trusted" })),
        Err(e) => err(500, &format!("db: {}", e)),
    }
}

pub fn list_pending_clients(db: &ManifestDb) -> HandlerResponse {
    let conn = db.conn();
    let mut stmt = match conn.prepare(
        "SELECT client_id, created_at FROM clients WHERE status = 'pending' ORDER BY created_at ASC",
    ) {
        Ok(s) => s,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "client_id": row.get::<_, String>(0)?,
                "created_at": row.get::<_, String>(1)?
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    ok(json!({ "pending_clients": rows }))
}
