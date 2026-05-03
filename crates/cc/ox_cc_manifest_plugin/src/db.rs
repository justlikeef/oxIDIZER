/// Shared SQLite database for the Manifest instance.
///
/// Both ox_cc_manifest_plugin and ox_cc_report_plugin use this same file
/// (manifest_instance.db). Both plugins must be configured with the same db_path.
///
/// WAL mode. File ACL must be 600 (owner: ox_cc_manifest service account).
use rusqlite::Connection;

pub struct ManifestDb {
    conn: Connection,
}

impl ManifestDb {
    pub fn open(path: &str, encryption_key: &str) -> Result<Self, anyhow::Error> {
        let conn = Connection::open(path)
            .map_err(|e| anyhow::anyhow!("open manifest db {}: {}", path, e))?;

        // PRAGMA key must be the very first statement on a SQLCipher database.
        conn.execute_batch(&format!("PRAGMA key = '{}';", encryption_key.replace('\'', "''")))?;
        conn.execute_batch("PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS clients (
                client_id       TEXT PRIMARY KEY,
                enc_pubkey_b64  TEXT NOT NULL,
                sig_pubkey_b64  TEXT,
                status          TEXT NOT NULL DEFAULT 'pending', -- 'pending', 'trusted', 'blocked'
                created_at      TEXT NOT NULL,
                last_seen_at    TEXT
            );

            CREATE TABLE IF NOT EXISTS envelopes (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                client_id      TEXT    NOT NULL,
                manifest_id    TEXT    NOT NULL UNIQUE,
                stored_at      TEXT    NOT NULL,
                stored_by      TEXT    NOT NULL,   -- CN of the admin that deployed
                envelope_json  TEXT    NOT NULL,
                is_latest      INTEGER NOT NULL DEFAULT 1,
                last_polled_at TEXT,               -- updated on every client GET (even 304)
                FOREIGN KEY(client_id) REFERENCES clients(client_id)
            );

            CREATE INDEX IF NOT EXISTS idx_env_client_id
                ON envelopes(client_id);
            CREATE INDEX IF NOT EXISTS idx_env_is_latest
                ON envelopes(client_id, is_latest);

            -- reports table is owned by this schema so both plugins share it
            CREATE TABLE IF NOT EXISTS reports (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                client_id   TEXT    NOT NULL,
                manifest_id TEXT    NOT NULL,
                report_id   TEXT    NOT NULL UNIQUE,
                sequence    INTEGER NOT NULL,
                received_at TEXT    NOT NULL,
                status      TEXT    NOT NULL,
                detail      TEXT,
                FOREIGN KEY(client_id) REFERENCES clients(client_id)
            );

            CREATE INDEX IF NOT EXISTS idx_rep_client_id
                ON reports(client_id);
            CREATE INDEX IF NOT EXISTS idx_rep_manifest_id
                ON reports(manifest_id);
            "#,
        )
        .map_err(|e| anyhow::anyhow!("manifest db schema: {}", e))?;

        Ok(Self { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
