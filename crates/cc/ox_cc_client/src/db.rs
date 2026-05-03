/// SQLite state database for the client.
///
/// Tracks applied manifests and pending "applied" notifications.
/// WAL mode. File ACL must be 600 (owner: ox_cc service account).
use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};

use ox_cc_common::Manifest;

use crate::config::ClientConfig;
use crate::fetcher::Notifier;

pub struct ClientDb {
    conn: Connection,
}

impl ClientDb {
    pub fn open(path: &str, encryption_key: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| anyhow::anyhow!("open client db {}: {}", path, e))?;

        // PRAGMA key must be the very first statement on a SQLCipher database.
        conn.execute_batch(&format!("PRAGMA key = '{}';", encryption_key.replace('\'', "''")))?;
        conn.execute_batch("PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL;")?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS manifests (
                manifest_id             TEXT PRIMARY KEY,
                consumer                TEXT NOT NULL,
                name                    TEXT NOT NULL,
                description             TEXT NOT NULL,
                applied_at              TEXT NOT NULL,
                expires_at              TEXT NOT NULL,
                applied_notified_at     TEXT,     -- NULL until POST "applied" succeeds
                notify_retry_count      INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )
        .map_err(|e| anyhow::anyhow!("client db schema: {}", e))?;

        Ok(Self { conn })
    }

    /// Returns true if the manifest has already been applied.
    pub fn is_applied(&self, manifest_id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM manifests WHERE manifest_id = ?1",
            params![manifest_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Record a successfully applied manifest. applied_notified_at is NULL
    /// until the POST notification succeeds.
    pub fn record_applied(&self, manifest: &Manifest) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO manifests
             (manifest_id, consumer, name, description, applied_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                manifest.manifest_id,
                manifest.consumer,
                manifest.name,
                manifest.description,
                now,
                manifest.expires_at
            ],
        )?;
        Ok(())
    }

    /// Mark a manifest as having been successfully notified.
    pub fn mark_notified(&self, manifest_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE manifests SET applied_notified_at = ?1 WHERE manifest_id = ?2",
            params![now, manifest_id],
        )?;
        Ok(())
    }

    /// Increment retry count for a failed notification.
    pub fn increment_retry(&self, manifest_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE manifests SET notify_retry_count = notify_retry_count + 1
             WHERE manifest_id = ?1",
            params![manifest_id],
        )?;
        Ok(())
    }

    /// Query all manifests with pending notifications and retry them.
    /// Called at the start of each poll cycle before fetching new manifests.
    pub async fn retry_pending_notifications<N: Notifier>(
        &self,
        fetcher: &N,
        cfg: &ClientConfig,
    ) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT manifest_id, notify_retry_count FROM manifests
             WHERE applied_notified_at IS NULL
             ORDER BY applied_at ASC",
        )?;

        let pending: Vec<(String, u32)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        for (manifest_id, retry_count) in pending {
            // Exponential backoff: skip if retry_count is high and we're not
            // on a retry-eligible cycle. Simple approach: always attempt
            // (the poll interval itself provides natural spacing).
            match fetcher.post_applied(cfg, &manifest_id, None).await {
                Ok(_) => {
                    self.mark_notified(&manifest_id)?;
                    tracing::info!(manifest_id = %manifest_id, "applied notification sent");
                }
                Err(e) => {
                    self.increment_retry(&manifest_id)?;
                    tracing::warn!(
                        manifest_id = %manifest_id,
                        retry = retry_count + 1,
                        error = %e,
                        "applied notification failed"
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use chrono::{Duration, Utc};
    use serde_json::json;
    use tempfile::NamedTempFile;

    use crate::config::{ClientConfig, ClientTlsConfig};

    fn make_manifest(id: &str) -> Manifest {
        let now = Utc::now();
        Manifest {
            version: "1".to_string(),
            manifest_id: id.to_string(),
            client_id: "test-client".to_string(),
            consumer: "test_consumer".to_string(),
            name: "Test".to_string(),
            description: "desc".to_string(),
            issued_at: now.to_rfc3339(),
            expires_at: (now + Duration::hours(24)).to_rfc3339(),
            payload: json!({}),
        }
    }

    fn open_test_db() -> (ClientDb, NamedTempFile) {
        let tmp = NamedTempFile::new().expect("tempfile");
        let db = ClientDb::open(tmp.path().to_str().unwrap(), "testkey")
            .expect("open client db");
        (db, tmp)
    }

    fn stub_config() -> ClientConfig {
        ClientConfig {
            client_id: "test-client".to_string(),
            manifest_url: Some("https://manifest.example.com".to_string()),
            bootstrap_url: None,
            report_url: Some("https://manifest.example.com/cc/report/test-client".to_string()),
            db_path: ":memory:".to_string(),
            db_encryption_key: "testkey".to_string(),
            poll_interval_secs: 60,
            max_manifest_window_secs: 90 * 24 * 3600,
            broker_signing_pubkeys_dir: Some("/tmp".to_string()),
            client_enc_privkey_b64: Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string()),
            consumer_dirs: HashMap::new(),
            plugin_dir: None,
            tls: Some(ClientTlsConfig {
                client_cert: "/dev/null".to_string(),
                client_key: "/dev/null".to_string(),
                ca_cert: "/dev/null".to_string(),
            }),
        }
    }

    // ── Notifier stubs ────────────────────────────────────────────────────────

    struct AlwaysOkNotifier;
    impl Notifier for AlwaysOkNotifier {
        async fn post_applied(&self, _cfg: &ClientConfig, _manifest_id: &str, _detail: Option<&str>) -> Result<()> {
            Ok(())
        }
    }

    struct AlwaysFailNotifier;
    impl Notifier for AlwaysFailNotifier {
        async fn post_applied(&self, _cfg: &ClientConfig, _manifest_id: &str, _detail: Option<&str>) -> Result<()> {
            Err(anyhow::anyhow!("simulated network failure"))
        }
    }

    /// Records which manifest_ids were successfully notified.
    struct RecordingNotifier {
        notified: Arc<Mutex<Vec<String>>>,
    }
    impl Notifier for RecordingNotifier {
        async fn post_applied(&self, _cfg: &ClientConfig, manifest_id: &str, _detail: Option<&str>) -> Result<()> {
            self.notified.lock().unwrap().push(manifest_id.to_string());
            Ok(())
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_is_applied_false_initially() {
        let (db, _tmp) = open_test_db();
        assert!(!db.is_applied("m1").unwrap());
    }

    #[test]
    fn test_record_applied_and_is_applied() {
        let (db, _tmp) = open_test_db();
        let m = make_manifest("m1");
        db.record_applied(&m).unwrap();
        assert!(db.is_applied("m1").unwrap());
        assert!(!db.is_applied("m2").unwrap());
    }

    #[test]
    fn test_record_applied_idempotent() {
        let (db, _tmp) = open_test_db();
        let m = make_manifest("m1");
        db.record_applied(&m).unwrap();
        db.record_applied(&m).unwrap(); // INSERT OR IGNORE — should not error
        assert!(db.is_applied("m1").unwrap());
    }

    #[test]
    fn test_mark_notified_sets_timestamp() {
        let (db, _tmp) = open_test_db();
        let m = make_manifest("m1");
        db.record_applied(&m).unwrap();
        db.mark_notified("m1").unwrap();

        let notified_at: Option<String> = db.conn.query_row(
            "SELECT applied_notified_at FROM manifests WHERE manifest_id = 'm1'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(notified_at.is_some(), "applied_notified_at should be set");
    }

    #[test]
    fn test_increment_retry() {
        let (db, _tmp) = open_test_db();
        let m = make_manifest("m1");
        db.record_applied(&m).unwrap();
        db.increment_retry("m1").unwrap();
        db.increment_retry("m1").unwrap();

        let count: i64 = db.conn.query_row(
            "SELECT notify_retry_count FROM manifests WHERE manifest_id = 'm1'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_retry_pending_ok_marks_notified() {
        let (db, _tmp) = open_test_db();
        db.record_applied(&make_manifest("m1")).unwrap();
        db.record_applied(&make_manifest("m2")).unwrap();

        let cfg = stub_config();
        let notifier = RecordingNotifier { notified: Arc::new(Mutex::new(vec![])) };
        db.retry_pending_notifications(&notifier, &cfg).await.unwrap();

        let notified = notifier.notified.lock().unwrap().clone();
        assert_eq!(notified.len(), 2);
        assert!(notified.contains(&"m1".to_string()));
        assert!(notified.contains(&"m2".to_string()));

        // Both should now be marked notified in DB (no longer pending)
        assert!(db.is_applied("m1").unwrap());
        let not_at: Option<String> = db.conn.query_row(
            "SELECT applied_notified_at FROM manifests WHERE manifest_id = 'm1'",
            [], |row| row.get(0)).unwrap();
        assert!(not_at.is_some());
    }

    #[tokio::test]
    async fn test_retry_pending_failure_increments_retry_count() {
        let (db, _tmp) = open_test_db();
        db.record_applied(&make_manifest("m1")).unwrap();

        let cfg = stub_config();
        db.retry_pending_notifications(&AlwaysFailNotifier, &cfg).await.unwrap();

        let count: i64 = db.conn.query_row(
            "SELECT notify_retry_count FROM manifests WHERE manifest_id = 'm1'",
            [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);

        // applied_notified_at remains NULL
        let not_at: Option<String> = db.conn.query_row(
            "SELECT applied_notified_at FROM manifests WHERE manifest_id = 'm1'",
            [], |row| row.get(0)).unwrap();
        assert!(not_at.is_none());
    }

    #[tokio::test]
    async fn test_retry_pending_already_notified_not_retried() {
        let (db, _tmp) = open_test_db();
        db.record_applied(&make_manifest("m1")).unwrap();
        db.mark_notified("m1").unwrap(); // already done

        let cfg = stub_config();
        let notifier = RecordingNotifier { notified: Arc::new(Mutex::new(vec![])) };
        db.retry_pending_notifications(&notifier, &cfg).await.unwrap();

        // Should not be called again since applied_notified_at is set
        assert_eq!(notifier.notified.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_retry_pending_ok_clears_from_pending() {
        let (db, _tmp) = open_test_db();
        db.record_applied(&make_manifest("m1")).unwrap();

        let cfg = stub_config();
        db.retry_pending_notifications(&AlwaysOkNotifier, &cfg).await.unwrap();

        // Second call: m1 is now notified, should not be retried
        let notifier2 = RecordingNotifier { notified: Arc::new(Mutex::new(vec![])) };
        db.retry_pending_notifications(&notifier2, &cfg).await.unwrap();
        assert_eq!(notifier2.notified.lock().unwrap().len(), 0);
    }
}
