/// Signing request lifecycle state machine.
///
/// States:
///   pending           — awaiting approver action
///   approved          — all clients signed successfully
///   partially_approved — some clients signed; failed_client_ids is non-empty
///   rejected          — approver rejected
///   expired           — pending TTL elapsed without action
use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;


/// Create signing_request rows for each client in the template batch.
/// Called after policy validation passes for all clients.
pub fn create_signing_requests(
    conn: &Connection,
    template_id: &str,
    client_ids: &[String],
) -> Result<(), anyhow::Error> {
    for client_id in client_ids {
        let request_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO signing_requests (request_id, template_id, client_id, status)
             VALUES (?1, ?2, ?3, 'pending')",
            params![request_id, template_id, client_id],
        )?;
    }
    Ok(())
}

/// Mark pending templates whose submitted_at + pending_ttl_secs < now as expired.
/// Should be called periodically (e.g. on each incoming request).
pub fn expire_stale_templates(
    conn: &Connection,
    ttl_secs: u64,
) -> Result<usize, anyhow::Error> {
    let cutoff = Utc::now()
        .checked_sub_signed(chrono::Duration::seconds(ttl_secs as i64))
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default();

    let count = conn.execute(
        "UPDATE manifest_templates
         SET status = 'expired', actioned_at = ?1
         WHERE status = 'pending' AND submitted_at < ?2",
        params![Utc::now().to_rfc3339(), cutoff],
    )?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::BrokerDb;
    use chrono::Duration;
    use rusqlite::params;
    use tempfile::NamedTempFile;
    use uuid::Uuid;

    fn open_test_db() -> (BrokerDb, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let db = BrokerDb::open(tmp.path().to_str().unwrap(), "key").unwrap();
        (db, tmp)
    }

    fn insert_template(conn: &Connection, template_id: &str, submitted_at: &str, status: &str) {
        conn.execute(
            "INSERT INTO manifest_templates
             (template_id, submitted_at, submitted_by, consumer, name, description,
              payload_path, expires_in_secs, status)
             VALUES (?1, ?2, 'admin', 'cons', 'name', 'desc', 'p.enc', 86400, ?3)",
            params![template_id, submitted_at, status],
        ).unwrap();
    }

    #[test]
    fn test_create_signing_requests_inserts_rows() {
        let (db, _tmp) = open_test_db();
        let conn = db.conn();
        let tid = "tmpl-1";
        insert_template(conn, tid, &Utc::now().to_rfc3339(), "pending");

        create_signing_requests(conn, tid, &[
            "client-a".to_string(),
            "client-b".to_string(),
        ]).unwrap();

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM signing_requests WHERE template_id = ?1",
            params![tid],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_create_signing_requests_status_pending() {
        let (db, _tmp) = open_test_db();
        let conn = db.conn();
        let tid = Uuid::new_v4().to_string();
        insert_template(conn, &tid, &Utc::now().to_rfc3339(), "pending");
        create_signing_requests(conn, &tid, &["c1".to_string()]).unwrap();

        let status: String = conn.query_row(
            "SELECT status FROM signing_requests WHERE template_id = ?1",
            params![tid],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "pending");
    }

    #[test]
    fn test_expire_stale_templates_marks_old_pending() {
        let (db, _tmp) = open_test_db();
        let conn = db.conn();
        let tid = Uuid::new_v4().to_string();
        // submitted 2 hours ago
        let old_ts = (Utc::now() - Duration::hours(2)).to_rfc3339();
        insert_template(conn, &tid, &old_ts, "pending");

        // TTL = 1 hour → this template is stale
        let expired = expire_stale_templates(conn, 3600).unwrap();
        assert_eq!(expired, 1);

        let status: String = conn.query_row(
            "SELECT status FROM manifest_templates WHERE template_id = ?1",
            params![tid],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "expired");
    }

    #[test]
    fn test_expire_stale_templates_leaves_recent_alone() {
        let (db, _tmp) = open_test_db();
        let conn = db.conn();
        let tid = Uuid::new_v4().to_string();
        // submitted 30 minutes ago
        let recent = (Utc::now() - Duration::minutes(30)).to_rfc3339();
        insert_template(conn, &tid, &recent, "pending");

        let expired = expire_stale_templates(conn, 3600).unwrap(); // 1h TTL
        assert_eq!(expired, 0);

        let status: String = conn.query_row(
            "SELECT status FROM manifest_templates WHERE template_id = ?1",
            params![tid],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "pending");
    }

    #[test]
    fn test_expire_stale_templates_ignores_non_pending() {
        let (db, _tmp) = open_test_db();
        let conn = db.conn();
        let old_ts = (Utc::now() - Duration::hours(2)).to_rfc3339();

        for (id, status) in &[("t1", "approved"), ("t2", "rejected"), ("t3", "expired")] {
            insert_template(conn, id, &old_ts, status);
        }
        let expired = expire_stale_templates(conn, 3600).unwrap();
        assert_eq!(expired, 0, "non-pending templates should not be expired");
    }
}
