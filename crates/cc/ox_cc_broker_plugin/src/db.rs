/// SQLite database for the broker.
///
/// Tables:
///   manifest_templates  — one row per submitted template (N clients)
///   signing_requests    — one row per per-client envelope (N per template)
///   audit_log           — immutable append-only event log
///
/// WAL mode is enabled for all databases. File ACL must be 600 (owner: ox_cc_broker).
use rusqlite::Connection;

pub struct BrokerDb {
    conn: Connection,
}

impl BrokerDb {
    pub fn open(path: &str, encryption_key: &str) -> Result<Self, anyhow::Error> {
        let conn = Connection::open(path)
            .map_err(|e| anyhow::anyhow!("open broker db {}: {}", path, e))?;

        // PRAGMA key must be the very first statement on a SQLCipher database.
        conn.execute_batch(&format!("PRAGMA key = '{}';", encryption_key.replace('\'', "''")))?;
        conn.execute_batch("PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS manifest_templates (
                template_id       TEXT PRIMARY KEY,
                submitted_at      TEXT NOT NULL,
                submitted_by      TEXT NOT NULL,  -- cert CN of the submitting admin
                consumer          TEXT NOT NULL,
                name              TEXT NOT NULL,
                description       TEXT NOT NULL,
                payload_path      TEXT NOT NULL,  -- relative path under payload_dir; NEVER absolute
                expires_in_secs   INTEGER NOT NULL,
                status            TEXT NOT NULL DEFAULT 'pending',
                                  -- pending | approved | partially_approved | rejected | expired
                actioned_at       TEXT,
                actioned_by       TEXT,           -- cert CN of the approver/rejecter
                rejected_reason   TEXT,
                failed_client_ids TEXT            -- JSON array; populated on partial failure
            );

            CREATE TABLE IF NOT EXISTS signing_requests (
                request_id   TEXT PRIMARY KEY,
                template_id  TEXT NOT NULL REFERENCES manifest_templates(template_id),
                client_id    TEXT NOT NULL,
                status       TEXT NOT NULL DEFAULT 'pending',
                              -- pending | approved | failed | delivered
                envelope_json TEXT,              -- populated after successful signing
                delivered_at  TEXT               -- set when admin acknowledges
            );

            CREATE INDEX IF NOT EXISTS idx_sr_template
                ON signing_requests(template_id);
            CREATE INDEX IF NOT EXISTS idx_sr_status
                ON signing_requests(status);

            CREATE TABLE IF NOT EXISTS clients (
                client_id      TEXT PRIMARY KEY,
                enc_pubkey_b64 TEXT NOT NULL,   -- base64url X25519 public key (32 bytes)
                enrolled_at    TEXT NOT NULL,
                enrolled_by    TEXT NOT NULL,   -- operator_id of the enrolling admin
                notes          TEXT             -- optional free-form notes
            );

            CREATE TABLE IF NOT EXISTS audit_log (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                occurred_at TEXT    NOT NULL,
                actor_cn    TEXT    NOT NULL,    -- cert CN of the actor
                action      TEXT    NOT NULL,    -- e.g. "submit_template", "approve", "reject"
                template_id TEXT,
                client_id   TEXT,
                detail      TEXT                -- free-form JSON
            );

            CREATE TABLE IF NOT EXISTS sessions (
                session_id       TEXT PRIMARY KEY,
                submitted_at     TEXT NOT NULL,
                submitted_by     TEXT NOT NULL,
                client_ids       TEXT NOT NULL,
                allowed_commands TEXT NOT NULL,
                expires_at       TEXT,
                status           TEXT NOT NULL DEFAULT 'pending',
                actioned_at      TEXT,
                actioned_by      TEXT,
                rejected_reason  TEXT,
                token            TEXT UNIQUE
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);
            CREATE INDEX IF NOT EXISTS idx_sessions_token  ON sessions(token);
            "#,
        )
        .map_err(|e| anyhow::anyhow!("broker db schema: {}", e))?;

        Ok(Self { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
