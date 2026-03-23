/// Opens the shared manifest_instance.db.
/// Schema is created by ox_cc_manifest_plugin; this plugin relies on that schema.
/// Both plugins must open the same db_path with WAL mode.
use rusqlite::Connection;

pub struct ReportDb {
    conn: Connection,
}

impl ReportDb {
    pub fn open(path: &str, encryption_key: &str) -> Result<Self, anyhow::Error> {
        let conn = Connection::open(path)
            .map_err(|e| anyhow::anyhow!("open report db {}: {}", path, e))?;
        // PRAGMA key must be the very first statement on a SQLCipher database.
        conn.execute_batch(&format!("PRAGMA key = '{}';", encryption_key.replace('\'', "''")))?;
        conn.execute_batch("PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        // Schema is owned by ox_cc_manifest_plugin; we rely on it being present.
        Ok(Self { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
