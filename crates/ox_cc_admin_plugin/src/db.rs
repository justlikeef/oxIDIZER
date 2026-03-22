/// SQLite database for the admin plugin.
///
/// Tracks templates submitted by this admin node and deployment records.
/// WAL mode. File ACL must be 600 (owner: ox_cc_admin service account).
use rusqlite::Connection;

pub struct AdminDb {
    conn: Connection,
}

impl AdminDb {
    pub fn open(path: &str, encryption_key: &str) -> Result<Self, anyhow::Error> {
        let conn = Connection::open(path)
            .map_err(|e| anyhow::anyhow!("open admin db {}: {}", path, e))?;

        // PRAGMA key must be the very first statement on a SQLCipher database.
        conn.execute_batch(&format!("PRAGMA key = '{}';", encryption_key.replace('\'', "''")))?;
        conn.execute_batch("PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS templates (
                template_id       TEXT PRIMARY KEY,
                created_at        TEXT NOT NULL,
                created_by        TEXT NOT NULL,
                consumer          TEXT NOT NULL,
                name              TEXT NOT NULL,
                description       TEXT NOT NULL,
                client_ids_json   TEXT NOT NULL,   -- JSON array of target client_ids
                status            TEXT NOT NULL DEFAULT 'draft',
                                  -- draft|submitted|pending|approved|partially_approved|rejected|deployed
                broker_status     TEXT,            -- echoed from broker on poll
                rejected_reason   TEXT,
                failed_client_ids TEXT             -- JSON array; populated on partial failure
            );

            CREATE TABLE IF NOT EXISTS manifest_deployments (
                manifest_id   TEXT PRIMARY KEY,
                template_id   TEXT NOT NULL REFERENCES templates(template_id),
                client_id     TEXT NOT NULL,
                deployed_at   TEXT,
                envelope_json TEXT              -- retained locally after broker delivery
            );

            CREATE INDEX IF NOT EXISTS idx_md_template
                ON manifest_deployments(template_id);

            CREATE TABLE IF NOT EXISTS sessions (
                session_id       TEXT PRIMARY KEY,
                created_at       TEXT NOT NULL,
                created_by       TEXT NOT NULL,
                client_ids       TEXT NOT NULL,
                allowed_commands TEXT NOT NULL,
                expires_at       TEXT,
                status           TEXT NOT NULL DEFAULT 'pending',
                token            TEXT
            );
            "#,
        )
        .map_err(|e| anyhow::anyhow!("admin db schema: {}", e))?;

        Ok(Self { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
