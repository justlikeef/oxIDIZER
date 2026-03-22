# Session Authorization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a pre-approved session mechanism to the broker and admin plugin so programmatic clients (e.g., arcnition) can submit manifests without per-manifest human approval, while preserving two-person integrity at session open time.

**Architecture:** Sessions are a first-class broker entity with their own submit→approve workflow. The broker owns session lifecycle and token generation; the admin plugin owns the operator-facing API for opening/closing sessions and exposes a `POST /admin/api/sessions/{id}/manifests` endpoint for arcnition. When `POST /broker/request` includes a valid `session_token`, the broker skips the pending queue and signs immediately.

**Tech Stack:** Rust, `rusqlite` (SQLCipher), `serde_json`, `rand` (token generation), `chrono`, existing `HttpClient` trait, existing handler patterns.

**Spec:** `docs/superpowers/specs/2026-03-20-commandset-executor-and-session-authorization-design.md` (Feature 2)

**Prerequisites:** No dependency on the commandset executor plan. Implement independently.

---

## File Map

### Modified files
| File | Change |
|------|--------|
| `crates/ox_cc_broker_plugin/src/db.rs` | Add `sessions` table to schema |
| `crates/ox_cc_broker_plugin/src/handlers.rs` | Add session endpoints; add `session_token` validation in `submit_template` |
| `crates/ox_cc_broker_plugin/src/lib.rs` | Route new session paths to handlers |
| `crates/ox_cc_admin_plugin/src/db.rs` | Add `sessions` table to schema |
| `crates/ox_cc_admin_plugin/src/handlers.rs` | Add session endpoints; add `POST /sessions/{id}/manifests` |
| `crates/ox_cc_admin_plugin/src/tests.rs` | Add session handler tests |

No new files required — all changes extend existing modules following established patterns.

---

## Task 1: Add `sessions` table to broker DB

**Files:**
- Modify: `crates/ox_cc_broker_plugin/src/db.rs`

- [ ] **Step 1: Write failing test**

Add to `crates/ox_cc_broker_plugin/src/db.rs` inline test module (find the `#[cfg(test)]` block or add one):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn open_test_db() -> (BrokerDb, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let db = BrokerDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();
        (db, tmp)
    }

    #[test]
    fn test_sessions_table_exists() {
        let (db, _tmp) = open_test_db();
        // Should be able to insert into sessions table without error
        db.conn().execute(
            "INSERT INTO sessions
             (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status)
             VALUES ('s1', '2026-01-01T00:00:00Z', 'alice', '[]', '[]', 'pending')",
            [],
        ).unwrap();

        let count: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM sessions WHERE session_id = 's1'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_sessions_token_is_unique() {
        let (db, _tmp) = open_test_db();
        db.conn().execute(
            "INSERT INTO sessions (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status, token)
             VALUES ('s1', '2026-01-01T00:00:00Z', 'alice', '[]', '[]', 'approved', 'tok1')",
            [],
        ).unwrap();

        // Second insert with same token should fail
        let result = db.conn().execute(
            "INSERT INTO sessions (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status, token)
             VALUES ('s2', '2026-01-01T00:00:00Z', 'bob', '[]', '[]', 'approved', 'tok1')",
            [],
        );
        assert!(result.is_err(), "duplicate token should violate UNIQUE constraint");
    }
}
```

Run: `cargo test -p ox_cc_broker_plugin test_sessions`
Expected: FAIL — `sessions` table does not exist.

- [ ] **Step 2: Add `sessions` table to schema**

In `BrokerDb::open`, append to the `execute_batch` schema string:

```sql
CREATE TABLE IF NOT EXISTS sessions (
    session_id       TEXT PRIMARY KEY,
    submitted_at     TEXT NOT NULL,
    submitted_by     TEXT NOT NULL,       -- cert CN of submitter (TODO: from mTLS)
    client_ids       TEXT NOT NULL,       -- JSON array of allowed client_ids
    allowed_commands TEXT NOT NULL,       -- JSON array of permitted command names
    expires_at       TEXT,                -- optional ISO 8601; NULL = no hard expiry
    status           TEXT NOT NULL DEFAULT 'pending',
                     -- pending|approved|rejected|closed|expired
    actioned_at      TEXT,
    actioned_by      TEXT,                -- cert CN of approver/rejecter; must != submitted_by
    rejected_reason  TEXT,
    token            TEXT UNIQUE          -- 32-byte random base64url; set on approval only
);
CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);
CREATE INDEX IF NOT EXISTS idx_sessions_token  ON sessions(token);
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ox_cc_broker_plugin`
Expected: all existing tests + 2 new session table tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ox_cc_broker_plugin/src/db.rs
git commit -m "feat(broker): add sessions table to broker DB schema"
```

---

## Task 2: Broker session endpoints

**Files:**
- Modify: `crates/ox_cc_broker_plugin/src/handlers.rs`
- Modify: `crates/ox_cc_broker_plugin/src/lib.rs`

- [ ] **Step 1: Write failing tests for session handlers**

Add to the inline test module in `crates/ox_cc_broker_plugin/src/handlers.rs` (find existing `#[cfg(test)]` block or add after the last `}` before EOF). First confirm there is a test module:

Check: `grep -n "cfg(test)" crates/ox_cc_broker_plugin/src/handlers.rs`

Then add these tests:

```rust
#[cfg(test)]
mod session_tests {
    use super::*;
    use tempfile::NamedTempFile;
    use crate::db::BrokerDb;
    use crate::config::BrokerPluginConfig;

    fn open_test_db() -> (BrokerDb, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let db = BrokerDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();
        (db, tmp)
    }

    fn stub_config() -> BrokerPluginConfig {
        BrokerPluginConfig {
            db_path: ":memory:".to_string(),
            db_encryption_key: "testkey".to_string(),
            payload_dir: "/tmp".to_string(),
            signing_key_path: "/tmp/broker.key".to_string(),
            enc_key_path: "/tmp/broker_enc.key".to_string(),
            cipher: "aes256gcm".to_string(),
            pending_ttl_secs: 86_400,
            max_manifest_window_secs: 90 * 24 * 3600,
            policy: Default::default(),
        }
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

        // Status must now be approved
        let status: String = db.conn().query_row(
            "SELECT status FROM sessions WHERE session_id = 's1'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "approved");

        // Token must be stored in DB
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
}
```

Run: `cargo test -p ox_cc_broker_plugin session_tests`
Expected: FAIL — functions not defined.

- [ ] **Step 2: Implement session handler functions**

Add to `crates/ox_cc_broker_plugin/src/handlers.rs`, after the existing handlers:

```rust
// ── POST /broker/sessions ───────────────────────────────────────────────────
// Role: admin cert

#[derive(Debug, Deserialize)]
struct SubmitSessionRequest {
    session_id: String,
    submitted_by: String,       // TODO: replace with cert CN when mTLS is available
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
    let client_ids_json = serde_json::to_string(&req.client_ids).unwrap();
    let commands_json = serde_json::to_string(&req.allowed_commands).unwrap();

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

    // Fetch session and check two-person rule
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
         WHERE session_id = ?4",
        params![now, req.actioned_by, token, session_id],
    ) {
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
    let n = match conn.execute(
        "UPDATE sessions SET status = 'closed'
         WHERE session_id = ?1 AND status = 'approved'",
        params![session_id],
    ) {
        Ok(n) => n,
        Err(e) => return err(500, &format!("db: {}", e)),
    };

    if n == 0 {
        return err(404, "session not found or not approved");
    }
    ok(json!({ "session_id": session_id, "status": "closed" }))
}
```

- [ ] **Step 3: Run session handler tests**

Run: `cargo test -p ox_cc_broker_plugin session_tests`
Expected: all 8 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ox_cc_broker_plugin/src/handlers.rs
git commit -m "feat(broker): add session submit/list/approve/reject/close handlers"
```

---

## Task 3: Session token validation in `submit_template`

**Files:**
- Modify: `crates/ox_cc_broker_plugin/src/handlers.rs`

- [ ] **Step 1: Write failing tests**

Add to `session_tests` module in `handlers.rs`:

```rust
    #[test]
    fn test_submit_template_with_valid_session_token_signs_immediately() {
        let (db, tmp) = open_test_db();
        let cfg = stub_config();

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

        // Write a fake signing key so the handler doesn't fail on missing key
        // (signing will fail — we just verify the session scope check passes/fails correctly)
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
        // Signing will fail (no real key), but we should NOT get a 403 scope rejection
        // We should get either 200 (partial/success) or 500 (signing error), not 403
        assert_ne!(resp.status, 403, "should not get scope rejection for valid session");
    }

    #[test]
    fn test_submit_template_with_invalid_token_is_401() {
        let (db, _tmp) = open_test_db();
        let cfg = stub_config();

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
        let cfg = stub_config();

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
        let cfg = stub_config();

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
            "client_ids": ["client-b"],   // outside session scope
            "submitted_by": "alice",
            "session_token": "scope-token"
        }).to_string();

        let resp = submit_template(&db, &cfg, &body);
        assert_eq!(resp.status, 422);
        assert!(resp.body.contains("outside session scope"));
    }
```

Run: `cargo test -p ox_cc_broker_plugin -- test_submit_template_with`
Expected: FAIL — `SubmitTemplateRequest` has no `session_token` field.

- [ ] **Step 2: Add `session_token` to `SubmitTemplateRequest`**

In `handlers.rs`, update the struct:

```rust
#[derive(Debug, Deserialize)]
struct SubmitTemplateRequest {
    template_id: String,
    consumer: String,
    name: String,
    description: String,
    expires_in_secs: i64,
    payload: Value,
    client_ids: Vec<String>,
    submitted_by: String,
    session_token: Option<String>,   // present only for session-driven submissions
}
```

- [ ] **Step 3: Add session token validation to `submit_template`**

At the top of the `submit_template` function body, after parsing the request but BEFORE policy validation, add:

```rust
    // Session token path: validate token, check scope, then sign immediately
    if let Some(token) = &req.session_token {
        return submit_template_with_session(db, config, &req, token);
    }
```

Then implement `submit_template_with_session` as a new private function:

```rust
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
            Err(_) => return err(401, "invalid session token"),
        };

    // 2. Check session is approved
    if status != "approved" {
        return err(403, &format!("session is '{}', not 'approved'", status));
    }

    // 3. Check expiry
    if let Some(exp) = &expires_at {
        if let Ok(exp_time) = chrono::DateTime::parse_from_rfc3339(exp) {
            if Utc::now() > exp_time.with_timezone(&Utc) {
                let _ = conn.execute(
                    "UPDATE sessions SET status = 'expired' WHERE session_id = ?1",
                    params![session_id],
                );
                return err(403, "session has expired");
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

    // 6. Run existing policy engine checks (name, description, payload structure)
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
    // Reuse existing approve_template logic
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

    // Forward the actual status from approve_template (may be "partially_approved")
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
```

- [ ] **Step 4: Run all broker tests**

Run: `cargo test -p ox_cc_broker_plugin`
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/ox_cc_broker_plugin/src/handlers.rs
git commit -m "feat(broker): add session token validation to submit_template with scope enforcement"
```

---

## Task 4: Verify broker crate builds cleanly

`crates/ox_cc_broker_plugin/src/lib.rs` is a module-declaration file only — there is no HTTP routing layer in this crate. Routing to handler functions is the responsibility of the HTTP server layer (a standalone axum binary, covered in a separate plan). This task just confirms the new session handlers compile without errors.

**Files:**
- No changes.

- [ ] **Step 1: Build to confirm no errors**

Run: `cargo build -p ox_cc_broker_plugin`
Expected: clean build with no warnings about unused public functions (session handlers are `pub`).

- [ ] **Step 2: Commit**

```bash
git commit --allow-empty -m "chore(broker): session handlers ready; routing deferred to HTTP server plan"
```

---

## Task 5: Add `sessions` table to admin DB

**Files:**
- Modify: `crates/ox_cc_admin_plugin/src/db.rs`

- [ ] **Step 1: Write failing test**

Add to inline test module in `crates/ox_cc_admin_plugin/src/db.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn open_test_db() -> (AdminDb, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let db = AdminDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();
        (db, tmp)
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
}
```

Run: `cargo test -p ox_cc_admin_plugin test_sessions_table_exists`
Expected: FAIL.

- [ ] **Step 2: Add `sessions` table to schema**

In `AdminDb::open`, append to the `execute_batch` schema string:

```sql
CREATE TABLE IF NOT EXISTS sessions (
    session_id       TEXT PRIMARY KEY,
    created_at       TEXT NOT NULL,     -- local timestamp; maps to submitted_at on broker
    created_by       TEXT NOT NULL,     -- local operator; maps to submitted_by on broker
    client_ids       TEXT NOT NULL,     -- JSON array
    allowed_commands TEXT NOT NULL,     -- JSON array
    expires_at       TEXT,
    status           TEXT NOT NULL DEFAULT 'pending',
                     -- mirrors broker: pending|approved|rejected|closed|expired
    token            TEXT               -- stored on approval; used for manifest submission
);
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ox_cc_admin_plugin`
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ox_cc_admin_plugin/src/db.rs
git commit -m "feat(admin): add sessions table to admin DB schema"
```

---

## Task 6: Admin session endpoints

**Files:**
- Modify: `crates/ox_cc_admin_plugin/src/handlers.rs`
- Modify: `crates/ox_cc_admin_plugin/src/tests.rs`

- [ ] **Step 1: Write failing tests**

Add to `crates/ox_cc_admin_plugin/src/tests.rs`. First add a `stub_config` alias at the top of the test file (or just below the existing `make_config` helper), so session tests can call `stub_config()` consistently with the broker plan:

```rust
fn stub_config() -> crate::config::AdminPluginConfig { make_config() }
```

Then add the session tests:

```rust
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

        // Verify broker was called
        let calls = client.calls.borrow();
        assert_eq!(calls.len(), 1);
        let (url, payload) = &calls[0];
        assert!(url.contains("/broker/sessions"));
        assert_eq!(payload["client_ids"].as_array().unwrap().len(), 1);
        // submitted_by should be mapped from created_by
        assert_eq!(payload["submitted_by"].as_str().unwrap(), "alice");
    }

    #[test]
    fn test_approve_session_stores_token() {
        let tmp = NamedTempFile::new().unwrap();
        let db = AdminDb::open(tmp.path().to_str().unwrap(), "testkey").unwrap();

        // Pre-insert session in admin DB
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

        // Token must be stored in admin DB
        let token: Option<String> = db.conn().query_row(
            "SELECT token FROM sessions WHERE session_id = 'sess-1'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(token.as_deref(), Some("abc123token"));

        // Status must be approved
        let status: String = db.conn().query_row(
            "SELECT status FROM sessions WHERE session_id = 'sess-1'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "approved");
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

        // Session with a stored token
        db.conn().execute(
            "INSERT INTO sessions (session_id, created_at, created_by, client_ids, allowed_commands, status, token)
             VALUES ('sess-1','2026-01-01T00:00:00Z','alice','[\"c1\"]','[\"download\"]','approved','secret-token')",
            [],
        ).unwrap();

        let broker_resp = json!({"template_id": "tmpl-1", "status": "approved", "session_id": "sess-1"});
        let client = RecordingClient::new(broker_resp.clone());
        // Also need manifest instance response for deployment
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

        // Verify the broker call included the session_token
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
```

Run: `cargo test -p ox_cc_admin_plugin -- test_open_session test_approve_session test_close_session test_submit_manifest test_list_pending_admin test_list_sessions test_reject_session`
Expected: FAIL — functions not defined.

- [ ] **Step 2: Implement admin session handler functions**

Add to `crates/ox_cc_admin_plugin/src/handlers.rs`:

```rust
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
    let client_ids_json = serde_json::to_string(&req.client_ids).unwrap();
    let commands_json = serde_json::to_string(&req.allowed_commands).unwrap();

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
            let _ = db.conn().execute(
                "UPDATE sessions SET status = 'rejected' WHERE session_id = ?1",
                params![session_id],
            );
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
            let _ = db.conn().execute(
                "UPDATE sessions SET status = 'closed' WHERE session_id = ?1",
                params![session_id],
            );
            ok(v)
        }
        Err(e) => err(502, &format!("broker error: {}", e)),
    }
}

// ── POST /admin/api/sessions/{session_id}/manifests ──────────────────────────
// Programmatic entry point for arcnition.

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
    // query_row returns Err(QueryReturnedNoRows) when no row matches; row.get returns
    // Option<String> so SQL NULL (token not yet set) is Ok(None), not Err.
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
```

- [ ] **Step 3: Run admin tests**

Run: `cargo test -p ox_cc_admin_plugin`
Expected: all tests pass including the 7 new session tests.

- [ ] **Step 4: Commit**

```bash
git add crates/ox_cc_admin_plugin/src/handlers.rs crates/ox_cc_admin_plugin/src/tests.rs
git commit -m "feat(admin): add session open/approve/reject/close/manifest endpoints"
```

---

## Task 7: Verify admin crate builds and all tests pass

`crates/ox_cc_admin_plugin/src/lib.rs` is a module-declaration file only — there is no HTTP routing layer in this crate. Routing to handler functions is the responsibility of the HTTP server layer (a standalone axum binary, covered in a separate plan). The route snippets below are reference material for when the HTTP server plan is implemented:

```
POST /admin/api/sessions                    → handlers::open_session
GET  /admin/api/sessions                    → handlers::list_sessions
GET  /admin/api/sessions/pending            → handlers::list_pending_admin_sessions
POST /admin/api/sessions/{id}/approve       → handlers::approve_session
POST /admin/api/sessions/{id}/reject        → handlers::reject_session
POST /admin/api/sessions/{id}/close         → handlers::close_session
POST /admin/api/sessions/{id}/manifests     → handlers::submit_session_manifest
```

**Files:**
- No changes.

- [ ] **Step 1: Build and run all tests**

Run: `cargo build -p ox_cc_admin_plugin && cargo test -p ox_cc_admin_plugin`
Expected: clean build, all tests pass.

- [ ] **Step 2: Commit**

```bash
git add crates/ox_cc_admin_plugin/src/handlers.rs crates/ox_cc_admin_plugin/src/tests.rs
git commit -m "feat(admin): add session endpoints; routing deferred to HTTP server plan"
```

---

## Final Verification

- [ ] Run `cargo test` — all tests pass across all crates
- [ ] Run `cargo build` — clean build, no warnings
- [ ] Verify session two-person rule: `cargo test -p ox_cc_broker_plugin -- test_approve_session_generates_token`
- [ ] Verify scope enforcement: `cargo test -p ox_cc_broker_plugin -- test_submit_template_session_client_id_outside_scope`

> **Note:** HTTP routing is deferred to the standalone axum server plan. The route reference table in Task 7 documents where each handler should be wired when that plan is executed.
