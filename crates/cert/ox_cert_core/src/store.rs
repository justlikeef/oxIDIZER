use rusqlite::{Connection, OptionalExtension, params};
use std::sync::Mutex;

use crate::model::*;
use crate::CertError;

// ─────────────────────────────────────────────────────────────────────────────
// CertStore trait
// ─────────────────────────────────────────────────────────────────────────────

pub trait CertStore: Send + Sync {
    /// Apply schema migrations. Safe to call on every startup (idempotent).
    fn migrate(&self) -> Result<(), CertError>;

    // --- X.509 Certificates ---
    fn store_cert(&self, tenant_id: &str, record: &CertificateRecord) -> Result<(), CertError>;
    fn get_cert_by_serial(&self, tenant_id: &str, serial: &str)
        -> Result<Option<CertificateRecord>, CertError>;
    fn get_certs_by_subject(&self, tenant_id: &str, subject_cn: &str)
        -> Result<Vec<CertificateRecord>, CertError>;
    fn list_certs(&self, tenant_id: &str, filter: &CertFilter, page: &Pagination)
        -> Result<PagedResult<CertificateRecord>, CertError>;
    fn mark_revoked(
        &self,
        tenant_id: &str,
        serial: &str,
        reason: RevocationReason,
        timestamp: time::OffsetDateTime,
    ) -> Result<(), CertError>;
    fn list_revoked(&self, tenant_id: &str) -> Result<Vec<CertificateRecord>, CertError>;
    fn list_revoked_since(&self, tenant_id: &str, since: time::OffsetDateTime)
        -> Result<Vec<CertificateRecord>, CertError>;
    fn list_expiring(&self, tenant_id: &str, within_days: u32)
        -> Result<Vec<CertificateRecord>, CertError>;
    fn update_status_expired(&self, tenant_id: &str) -> Result<u64, CertError>;

    // --- SSH Certificates ---
    fn store_ssh_cert(&self, tenant_id: &str, record: &SshCertRecord) -> Result<(), CertError>;
    fn get_ssh_cert_by_serial(&self, tenant_id: &str, serial: u64)
        -> Result<Option<SshCertRecord>, CertError>;
    fn list_ssh_certs(&self, tenant_id: &str, filter: &SshCertFilter, page: &Pagination)
        -> Result<PagedResult<SshCertRecord>, CertError>;
    fn get_next_ssh_serial(&self, tenant_id: &str) -> Result<u64, CertError>;

    // --- CA Keys ---
    fn store_ca_key(&self, tenant_id: &str, key: &CaKeyRecord) -> Result<(), CertError>;
    fn get_active_ca_key(&self, tenant_id: &str) -> Result<Option<CaKeyRecord>, CertError>;
    fn get_ca_key_by_id(&self, tenant_id: &str, id: &str)
        -> Result<Option<CaKeyRecord>, CertError>;
    fn update_ca_key_status(&self, tenant_id: &str, id: &str, status: CaKeyStatus)
        -> Result<(), CertError>;

    // --- ACME ---
    fn store_acme_account(&self, tenant_id: &str, account: &AcmeAccount) -> Result<(), CertError>;
    fn get_acme_account(&self, tenant_id: &str, id: &str)
        -> Result<Option<AcmeAccount>, CertError>;
    fn store_acme_order(&self, tenant_id: &str, order: &AcmeOrder) -> Result<(), CertError>;
    fn get_acme_order(&self, tenant_id: &str, id: &str) -> Result<Option<AcmeOrder>, CertError>;
    fn update_acme_order_status(&self, tenant_id: &str, id: &str, status: AcmeOrderStatus)
        -> Result<(), CertError>;
    fn store_acme_authorization(&self, tenant_id: &str, authz: &AcmeAuthorization)
        -> Result<(), CertError>;
    fn get_acme_authorization(&self, tenant_id: &str, id: &str)
        -> Result<Option<AcmeAuthorization>, CertError>;
    fn update_acme_authorization(&self, tenant_id: &str, authz: &AcmeAuthorization)
        -> Result<(), CertError>;

    // --- RA ---
    fn store_ra_request(&self, tenant_id: &str, request: &ApprovalRequest)
        -> Result<(), CertError>;
    fn get_ra_request(&self, tenant_id: &str, id: &str)
        -> Result<Option<ApprovalRequest>, CertError>;
    fn list_ra_pending(&self, tenant_id: &str, page: &Pagination)
        -> Result<PagedResult<ApprovalRequest>, CertError>;
    fn update_ra_request(
        &self,
        tenant_id: &str,
        id: &str,
        status: ApprovalStatus,
        reviewer: &str,
        notes: &str,
    ) -> Result<(), CertError>;

    // --- SCEP ---
    fn store_scep_challenge(&self, tenant_id: &str, challenge: &ScepChallenge)
        -> Result<(), CertError>;
    fn consume_scep_challenge(&self, tenant_id: &str, password_hash: &str)
        -> Result<bool, CertError>;

    // --- Notifications ---
    fn store_notification(&self, tenant_id: &str, notification: &NotificationRecord)
        -> Result<(), CertError>;
    fn was_notification_sent(&self, tenant_id: &str, serial: &str, threshold_days: u32)
        -> Result<bool, CertError>;

    // --- Audit ---
    fn store_audit_event(&self, tenant_id: &str, event: &AuditEvent) -> Result<(), CertError>;
    fn get_audit_log(
        &self,
        tenant_id: &str,
        filter: &AuditFilter,
        page: &Pagination,
    ) -> Result<PagedResult<AuditEvent>, CertError>;

    // --- CRL coordination (active/active HA) ---
    fn acquire_crl_lock(
        &self,
        tenant_id: &str,
        lock_key: &str,
        holder_id: &str,
        ttl_secs: u64,
    ) -> Result<Option<u64>, CertError>;
    fn release_crl_lock(&self, tenant_id: &str, lock_key: &str, holder_id: &str)
        -> Result<(), CertError>;
}

// ─────────────────────────────────────────────────────────────────────────────
// SQLite-backed implementation
// ─────────────────────────────────────────────────────────────────────────────

/// Certificate store backed by a local SQLite database.
///
/// Every record is stored as a JSON blob alongside a small number of indexed
/// scalar columns used for efficient filtering.  Pass `":memory:"` as the path
/// in unit tests.
pub struct OxPersistenceCertStore {
    conn: Mutex<Connection>,
}

impl OxPersistenceCertStore {
    /// Open (or create) the SQLite database at `db_path`.
    /// Runs migrations immediately so callers do not need to call `migrate()`
    /// separately.
    pub fn open(db_path: &str) -> Result<Self, CertError> {
        let conn = Connection::open(db_path)
            .map_err(|e| CertError::Storage(format!("sqlite open '{}': {}", db_path, e)))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| CertError::Storage(format!("PRAGMA: {}", e)))?;
        let store = Self { conn: Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }
}

// ─── internal helpers ────────────────────────────────────────────────────────

macro_rules! db {
    ($self:expr) => {
        $self.conn.lock().map_err(|e| CertError::Storage(format!("mutex: {}", e)))?
    };
}

fn ser<T: serde::Serialize>(v: &T) -> Result<String, CertError> {
    serde_json::to_string(v).map_err(|e| CertError::Internal(e.to_string()))
}

fn de<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, CertError> {
    serde_json::from_str(s).map_err(|e| CertError::Internal(e.to_string()))
}

fn to_ts(dt: time::OffsetDateTime) -> i64 {
    dt.unix_timestamp()
}

fn cert_status_str(s: &CertStatus) -> &'static str {
    match s {
        CertStatus::Active => "Active",
        CertStatus::Revoked => "Revoked",
        CertStatus::Expired => "Expired",
        CertStatus::PendingApproval => "PendingApproval",
    }
}

fn ca_key_status_str(s: &CaKeyStatus) -> &'static str {
    match s {
        CaKeyStatus::Active => "Active",
        CaKeyStatus::Retiring => "Retiring",
        CaKeyStatus::Retired => "Retired",
    }
}

fn approval_status_str(s: &ApprovalStatus) -> &'static str {
    match s {
        ApprovalStatus::Pending => "Pending",
        ApprovalStatus::Approved => "Approved",
        ApprovalStatus::Denied => "Denied",
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CertStore impl
// ─────────────────────────────────────────────────────────────────────────────

impl CertStore for OxPersistenceCertStore {
    fn migrate(&self) -> Result<(), CertError> {
        let conn = db!(self);
        conn.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS certificate (
                serial     TEXT    PRIMARY KEY,
                tenant_id  TEXT    NOT NULL,
                status     TEXT    NOT NULL,
                not_after  INTEGER NOT NULL,
                revoked_at INTEGER,
                data       TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cert_ts ON certificate(tenant_id, status);
            CREATE INDEX IF NOT EXISTS idx_cert_na ON certificate(not_after);

            CREATE TABLE IF NOT EXISTS ssh_certificate (
                serial    INTEGER PRIMARY KEY,
                tenant_id TEXT    NOT NULL,
                data      TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ssh_t ON ssh_certificate(tenant_id);

            CREATE TABLE IF NOT EXISTS ca_key (
                id        TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                status    TEXT NOT NULL,
                data      TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS ca_key_ptr (
                id        TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                key_id    TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS acme_account (
                id        TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                data      TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS acme_order (
                id         TEXT PRIMARY KEY,
                tenant_id  TEXT NOT NULL,
                account_id TEXT NOT NULL,
                data       TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ao_acct ON acme_order(account_id);

            CREATE TABLE IF NOT EXISTS acme_authorization (
                id        TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                order_id  TEXT NOT NULL,
                data      TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_aa_ord ON acme_authorization(order_id);

            CREATE TABLE IF NOT EXISTS ra_request (
                id        TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                status    TEXT NOT NULL,
                data      TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ra_ts ON ra_request(tenant_id, status);

            CREATE TABLE IF NOT EXISTS scep_challenge (
                id            TEXT    PRIMARY KEY,
                tenant_id     TEXT    NOT NULL,
                password_hash TEXT    NOT NULL,
                used          INTEGER NOT NULL DEFAULT 0,
                expires_at    INTEGER NOT NULL,
                data          TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sc_hash ON scep_challenge(password_hash);

            CREATE TABLE IF NOT EXISTS notification (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                tenant_id      TEXT    NOT NULL,
                serial         TEXT    NOT NULL,
                threshold_days INTEGER NOT NULL,
                data           TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_notif ON notification(tenant_id, serial);

            CREATE TABLE IF NOT EXISTS audit_log (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                tenant_id TEXT    NOT NULL,
                timestamp INTEGER NOT NULL,
                action    TEXT    NOT NULL,
                serial    TEXT,
                data      TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_audit_t  ON audit_log(tenant_id, timestamp);
            CREATE INDEX IF NOT EXISTS idx_audit_ser ON audit_log(serial);

            CREATE TABLE IF NOT EXISTS crl_lock (
                lock_key   TEXT    PRIMARY KEY,
                tenant_id  TEXT    NOT NULL,
                holder_id  TEXT    NOT NULL,
                expires_at INTEGER NOT NULL,
                generation INTEGER NOT NULL DEFAULT 1
            );
        "#).map_err(|e| CertError::Storage(format!("migrate: {}", e)))
    }

    // ─── X.509 Certificates ──────────────────────────────────────────────────

    fn store_cert(&self, tenant_id: &str, record: &CertificateRecord) -> Result<(), CertError> {
        let conn = db!(self);
        conn.execute(
            "INSERT OR REPLACE INTO certificate \
             (serial, tenant_id, status, not_after, revoked_at, data) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.serial,
                tenant_id,
                cert_status_str(&record.status),
                to_ts(record.not_after),
                record.revoked_at.map(to_ts),
                ser(record)?,
            ],
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        Ok(())
    }

    fn get_cert_by_serial(&self, tenant_id: &str, serial: &str)
        -> Result<Option<CertificateRecord>, CertError>
    {
        let conn = db!(self);
        let row: Option<String> = conn.query_row(
            "SELECT data FROM certificate WHERE serial = ?1 AND tenant_id = ?2",
            params![serial, tenant_id],
            |r| r.get(0),
        ).optional().map_err(|e| CertError::Storage(e.to_string()))?;
        row.as_deref().map(de).transpose()
    }

    fn get_certs_by_subject(&self, tenant_id: &str, subject_cn: &str)
        -> Result<Vec<CertificateRecord>, CertError>
    {
        let conn = db!(self);
        let mut stmt = conn.prepare(
            "SELECT data FROM certificate \
             WHERE tenant_id = ?1 AND json_extract(data, '$.subject_cn') = ?2"
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        let rows = stmt.query_map(params![tenant_id, subject_cn], |r| r.get::<_, String>(0))
            .map_err(|e| CertError::Storage(e.to_string()))?;
        let mut out = Vec::new();
        for row in rows {
            let data = row.map_err(|e| CertError::Storage(e.to_string()))?;
            out.push(de::<CertificateRecord>(&data)?);
        }
        Ok(out)
    }

    fn list_certs(&self, tenant_id: &str, filter: &CertFilter, page: &Pagination)
        -> Result<PagedResult<CertificateRecord>, CertError>
    {
        let conn = db!(self);
        let mut conds = vec!["tenant_id = ?1".to_string()];
        let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(tenant_id.to_string())];
        let mut n = 2usize;

        macro_rules! add_filter {
            ($cond:literal, $val:expr) => {{
                conds.push(format!($cond, n));
                bind.push(Box::new($val));
                n += 1;
            }};
        }

        if let Some(status) = &filter.status {
            add_filter!("status = ?{}", cert_status_str(status).to_string());
        }
        if let Some(cn) = &filter.subject_cn {
            add_filter!("json_extract(data,'$.subject_cn') LIKE ?{}", format!("%{}%", cn));
        }
        if let Some(san) = &filter.san {
            add_filter!("INSTR(data, ?{}) > 0", san.clone());
        }
        if let Some(profile) = &filter.profile {
            add_filter!("json_extract(data,'$.profile') = ?{}", profile.clone());
        }
        if let Some(before) = filter.not_after_before {
            add_filter!("not_after < ?{}", to_ts(before));
        }
        if let Some(after) = filter.not_after_after {
            add_filter!("not_after > ?{}", to_ts(after));
        }

        let where_clause = conds.join(" AND ");
        let count_sql = format!("SELECT COUNT(*) FROM certificate WHERE {}", where_clause);
        let data_sql = format!(
            "SELECT data FROM certificate WHERE {} ORDER BY not_after ASC LIMIT ?{} OFFSET ?{}",
            where_clause, n, n + 1
        );

        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
        let total: i64 = conn.query_row(&count_sql, refs.as_slice(), |r| r.get(0)).unwrap_or(0);

        bind.push(Box::new(page.limit as i64));
        bind.push(Box::new(page.offset as i64));
        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();

        let mut stmt = conn.prepare(&data_sql).map_err(|e| CertError::Storage(e.to_string()))?;
        let items: Vec<CertificateRecord> = stmt
            .query_map(refs.as_slice(), |r| r.get::<_, String>(0))
            .map_err(|e| CertError::Storage(e.to_string()))?
            .filter_map(|r| r.ok())
            .filter_map(|data| de::<CertificateRecord>(&data).ok())
            .collect();

        Ok(PagedResult { items, total: total as u64, offset: page.offset, limit: page.limit })
    }

    fn mark_revoked(
        &self, tenant_id: &str, serial: &str,
        reason: RevocationReason, timestamp: time::OffsetDateTime,
    ) -> Result<(), CertError> {
        let mut record = self.get_cert_by_serial(tenant_id, serial)?
            .ok_or_else(|| CertError::NotFound(serial.to_string()))?;
        record.status = CertStatus::Revoked;
        record.revoked_at = Some(timestamp);
        record.revocation_reason = Some(reason);
        self.store_cert(tenant_id, &record)
    }

    fn list_revoked(&self, tenant_id: &str) -> Result<Vec<CertificateRecord>, CertError> {
        let conn = db!(self);
        let mut stmt = conn.prepare(
            "SELECT data FROM certificate WHERE tenant_id = ?1 AND status = 'Revoked'"
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        collect_records(&mut stmt, params![tenant_id])
    }

    fn list_revoked_since(&self, tenant_id: &str, since: time::OffsetDateTime)
        -> Result<Vec<CertificateRecord>, CertError>
    {
        let conn = db!(self);
        let mut stmt = conn.prepare(
            "SELECT data FROM certificate \
             WHERE tenant_id = ?1 AND status = 'Revoked' AND revoked_at >= ?2"
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        collect_records(&mut stmt, params![tenant_id, to_ts(since)])
    }

    fn list_expiring(&self, tenant_id: &str, within_days: u32)
        -> Result<Vec<CertificateRecord>, CertError>
    {
        let conn = db!(self);
        let now = to_ts(time::OffsetDateTime::now_utc());
        let cutoff = now + within_days as i64 * 86400;
        let mut stmt = conn.prepare(
            "SELECT data FROM certificate \
             WHERE tenant_id = ?1 AND status = 'Active' AND not_after > ?2 AND not_after <= ?3 \
             ORDER BY not_after ASC"
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        collect_records(&mut stmt, params![tenant_id, now, cutoff])
    }

    fn update_status_expired(&self, tenant_id: &str) -> Result<u64, CertError> {
        let now = to_ts(time::OffsetDateTime::now_utc());
        // Collect serials while holding the lock, then release before mutating.
        // Use .and_then() to avoid ?-on-MappedRows inside a scoped block, which
        // would create a ControlFlow temporary borrowing from the local conn.
        let serials: Vec<String> = {
            let conn = db!(self);
            let mut stmt = conn.prepare(
                "SELECT serial FROM certificate \
                 WHERE tenant_id = ?1 AND status = 'Active' AND not_after < ?2"
            ).map_err(|e| CertError::Storage(e.to_string()))?;
            stmt.query_map(params![tenant_id, now], |r| r.get::<_, String>(0))
                .and_then(|rows| rows.collect::<Result<Vec<String>, _>>())
                .map_err(|e| CertError::Storage(e.to_string()))?
        };
        let count = serials.len() as u64;
        for serial in &serials {
            if let Ok(Some(mut rec)) = self.get_cert_by_serial(tenant_id, serial) {
                rec.status = CertStatus::Expired;
                let _ = self.store_cert(tenant_id, &rec);
            }
        }
        Ok(count)
    }

    // ─── SSH Certificates ─────────────────────────────────────────────────────

    fn store_ssh_cert(&self, tenant_id: &str, record: &SshCertRecord) -> Result<(), CertError> {
        let conn = db!(self);
        conn.execute(
            "INSERT OR REPLACE INTO ssh_certificate (serial, tenant_id, data) VALUES (?1, ?2, ?3)",
            params![record.serial as i64, tenant_id, ser(record)?],
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        Ok(())
    }

    fn get_ssh_cert_by_serial(&self, tenant_id: &str, serial: u64)
        -> Result<Option<SshCertRecord>, CertError>
    {
        let conn = db!(self);
        let row: Option<String> = conn.query_row(
            "SELECT data FROM ssh_certificate WHERE serial = ?1 AND tenant_id = ?2",
            params![serial as i64, tenant_id],
            |r| r.get(0),
        ).optional().map_err(|e| CertError::Storage(e.to_string()))?;
        row.as_deref().map(de).transpose()
    }

    fn list_ssh_certs(&self, tenant_id: &str, filter: &SshCertFilter, page: &Pagination)
        -> Result<PagedResult<SshCertRecord>, CertError>
    {
        let conn = db!(self);
        let mut conds = vec!["tenant_id = ?1".to_string()];
        let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(tenant_id.to_string())];
        let mut n = 2usize;

        if let Some(ct) = &filter.cert_type {
            let s = match ct { SshCertType::User => "User", SshCertType::Host => "Host" };
            conds.push(format!("json_extract(data,'$.cert_type') = ?{}", n));
            bind.push(Box::new(s.to_string()));
            n += 1;
        }
        if let Some(principal) = &filter.principal {
            conds.push(format!("INSTR(json_extract(data,'$.principals'), ?{}) > 0", n));
            bind.push(Box::new(principal.clone()));
            n += 1;
        }
        if let Some(before) = filter.valid_before_before {
            conds.push(format!("json_extract(data,'$.valid_before') < ?{}", n));
            bind.push(Box::new(to_ts(before)));
            n += 1;
        }

        let where_clause = conds.join(" AND ");
        let count_sql = format!("SELECT COUNT(*) FROM ssh_certificate WHERE {}", where_clause);
        let data_sql = format!(
            "SELECT data FROM ssh_certificate WHERE {} ORDER BY serial DESC LIMIT ?{} OFFSET ?{}",
            where_clause, n, n + 1
        );

        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
        let total: i64 = conn.query_row(&count_sql, refs.as_slice(), |r| r.get(0)).unwrap_or(0);

        bind.push(Box::new(page.limit as i64));
        bind.push(Box::new(page.offset as i64));
        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();

        let mut stmt = conn.prepare(&data_sql).map_err(|e| CertError::Storage(e.to_string()))?;
        let items: Vec<SshCertRecord> = stmt
            .query_map(refs.as_slice(), |r| r.get::<_, String>(0))
            .map_err(|e| CertError::Storage(e.to_string()))?
            .filter_map(|r| r.ok())
            .filter_map(|data| de::<SshCertRecord>(&data).ok())
            .collect();

        Ok(PagedResult { items, total: total as u64, offset: page.offset, limit: page.limit })
    }

    fn get_next_ssh_serial(&self, tenant_id: &str) -> Result<u64, CertError> {
        let conn = db!(self);
        let max: i64 = conn.query_row(
            "SELECT COALESCE(MAX(serial), 0) FROM ssh_certificate WHERE tenant_id = ?1",
            params![tenant_id],
            |r| r.get(0),
        ).unwrap_or(0);
        Ok((max + 1) as u64)
    }

    // ─── CA Keys ─────────────────────────────────────────────────────────────

    fn store_ca_key(&self, tenant_id: &str, key: &CaKeyRecord) -> Result<(), CertError> {
        let data = ser(key)?;
        let is_active = key.status == CaKeyStatus::Active;
        let key_id = key.id.clone();
        {
            let conn = db!(self);
            conn.execute(
                "INSERT OR REPLACE INTO ca_key (id, tenant_id, status, data) VALUES (?1, ?2, ?3, ?4)",
                params![key_id, tenant_id, ca_key_status_str(&key.status), data],
            ).map_err(|e| CertError::Storage(e.to_string()))?;
        }
        if is_active {
            let ptr_id = format!("{}:active", tenant_id);
            let conn = db!(self);
            conn.execute(
                "INSERT OR REPLACE INTO ca_key_ptr (id, tenant_id, key_id) VALUES (?1, ?2, ?3)",
                params![ptr_id, tenant_id, key_id],
            ).map_err(|e| CertError::Storage(e.to_string()))?;
        }
        Ok(())
    }

    fn get_active_ca_key(&self, tenant_id: &str) -> Result<Option<CaKeyRecord>, CertError> {
        let key_id: Option<String> = {
            let conn = db!(self);
            conn.query_row(
                "SELECT key_id FROM ca_key_ptr WHERE id = ?1",
                params![format!("{}:active", tenant_id)],
                |r| r.get(0),
            ).optional().map_err(|e| CertError::Storage(e.to_string()))?
        };
        match key_id {
            None => Ok(None),
            Some(kid) => self.get_ca_key_by_id(tenant_id, &kid),
        }
    }

    fn get_ca_key_by_id(&self, tenant_id: &str, id: &str)
        -> Result<Option<CaKeyRecord>, CertError>
    {
        let conn = db!(self);
        let row: Option<String> = conn.query_row(
            "SELECT data FROM ca_key WHERE id = ?1 AND tenant_id = ?2",
            params![id, tenant_id],
            |r| r.get(0),
        ).optional().map_err(|e| CertError::Storage(e.to_string()))?;
        row.as_deref().map(de).transpose()
    }

    fn update_ca_key_status(&self, tenant_id: &str, id: &str, status: CaKeyStatus)
        -> Result<(), CertError>
    {
        let mut key = self.get_ca_key_by_id(tenant_id, id)?
            .ok_or_else(|| CertError::NotFound(id.to_string()))?;
        key.status = status;
        self.store_ca_key(tenant_id, &key)
    }

    // ─── ACME ────────────────────────────────────────────────────────────────

    fn store_acme_account(&self, tenant_id: &str, account: &AcmeAccount) -> Result<(), CertError> {
        let conn = db!(self);
        conn.execute(
            "INSERT OR REPLACE INTO acme_account (id, tenant_id, data) VALUES (?1, ?2, ?3)",
            params![account.id, tenant_id, ser(account)?],
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        Ok(())
    }

    fn get_acme_account(&self, tenant_id: &str, id: &str)
        -> Result<Option<AcmeAccount>, CertError>
    {
        let conn = db!(self);
        let row: Option<String> = conn.query_row(
            "SELECT data FROM acme_account WHERE id = ?1 AND tenant_id = ?2",
            params![id, tenant_id],
            |r| r.get(0),
        ).optional().map_err(|e| CertError::Storage(e.to_string()))?;
        row.as_deref().map(de).transpose()
    }

    fn store_acme_order(&self, tenant_id: &str, order: &AcmeOrder) -> Result<(), CertError> {
        let conn = db!(self);
        conn.execute(
            "INSERT OR REPLACE INTO acme_order (id, tenant_id, account_id, data) \
             VALUES (?1, ?2, ?3, ?4)",
            params![order.id, tenant_id, order.account_id, ser(order)?],
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        Ok(())
    }

    fn get_acme_order(&self, tenant_id: &str, id: &str) -> Result<Option<AcmeOrder>, CertError> {
        let conn = db!(self);
        let row: Option<String> = conn.query_row(
            "SELECT data FROM acme_order WHERE id = ?1 AND tenant_id = ?2",
            params![id, tenant_id],
            |r| r.get(0),
        ).optional().map_err(|e| CertError::Storage(e.to_string()))?;
        row.as_deref().map(de).transpose()
    }

    fn update_acme_order_status(&self, tenant_id: &str, id: &str, status: AcmeOrderStatus)
        -> Result<(), CertError>
    {
        let mut order = self.get_acme_order(tenant_id, id)?
            .ok_or_else(|| CertError::NotFound(id.to_string()))?;
        order.status = status;
        self.store_acme_order(tenant_id, &order)
    }

    fn store_acme_authorization(&self, tenant_id: &str, authz: &AcmeAuthorization)
        -> Result<(), CertError>
    {
        let conn = db!(self);
        conn.execute(
            "INSERT OR REPLACE INTO acme_authorization \
             (id, tenant_id, order_id, data) VALUES (?1, ?2, ?3, ?4)",
            params![authz.id, tenant_id, authz.order_id, ser(authz)?],
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        Ok(())
    }

    fn get_acme_authorization(&self, tenant_id: &str, id: &str)
        -> Result<Option<AcmeAuthorization>, CertError>
    {
        let conn = db!(self);
        let row: Option<String> = conn.query_row(
            "SELECT data FROM acme_authorization WHERE id = ?1 AND tenant_id = ?2",
            params![id, tenant_id],
            |r| r.get(0),
        ).optional().map_err(|e| CertError::Storage(e.to_string()))?;
        row.as_deref().map(de).transpose()
    }

    fn update_acme_authorization(&self, tenant_id: &str, authz: &AcmeAuthorization)
        -> Result<(), CertError>
    {
        self.store_acme_authorization(tenant_id, authz)
    }

    // ─── RA ──────────────────────────────────────────────────────────────────

    fn store_ra_request(&self, tenant_id: &str, request: &ApprovalRequest)
        -> Result<(), CertError>
    {
        let conn = db!(self);
        conn.execute(
            "INSERT OR REPLACE INTO ra_request (id, tenant_id, status, data) VALUES (?1, ?2, ?3, ?4)",
            params![request.id, tenant_id, approval_status_str(&request.status), ser(request)?],
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        Ok(())
    }

    fn get_ra_request(&self, tenant_id: &str, id: &str)
        -> Result<Option<ApprovalRequest>, CertError>
    {
        let conn = db!(self);
        let row: Option<String> = conn.query_row(
            "SELECT data FROM ra_request WHERE id = ?1 AND tenant_id = ?2",
            params![id, tenant_id],
            |r| r.get(0),
        ).optional().map_err(|e| CertError::Storage(e.to_string()))?;
        row.as_deref().map(de).transpose()
    }

    fn list_ra_pending(&self, tenant_id: &str, page: &Pagination)
        -> Result<PagedResult<ApprovalRequest>, CertError>
    {
        let conn = db!(self);
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ra_request WHERE tenant_id = ?1 AND status = 'Pending'",
            params![tenant_id],
            |r| r.get(0),
        ).unwrap_or(0);
        let mut stmt = conn.prepare(
            "SELECT data FROM ra_request \
             WHERE tenant_id = ?1 AND status = 'Pending' \
             ORDER BY rowid ASC LIMIT ?2 OFFSET ?3"
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        let items: Vec<ApprovalRequest> = stmt
            .query_map(params![tenant_id, page.limit as i64, page.offset as i64],
                       |r| r.get::<_, String>(0))
            .map_err(|e| CertError::Storage(e.to_string()))?
            .filter_map(|r| r.ok())
            .filter_map(|data| de::<ApprovalRequest>(&data).ok())
            .collect();
        Ok(PagedResult { items, total: total as u64, offset: page.offset, limit: page.limit })
    }

    fn update_ra_request(
        &self, tenant_id: &str, id: &str,
        status: ApprovalStatus, reviewer: &str, notes: &str,
    ) -> Result<(), CertError> {
        let mut req = self.get_ra_request(tenant_id, id)?
            .ok_or_else(|| CertError::NotFound(id.to_string()))?;
        req.status = status;
        req.reviewer = Some(reviewer.to_string());
        req.review_notes = Some(notes.to_string());
        req.reviewed_at = Some(time::OffsetDateTime::now_utc());
        self.store_ra_request(tenant_id, &req)
    }

    // ─── SCEP ────────────────────────────────────────────────────────────────

    fn store_scep_challenge(&self, tenant_id: &str, challenge: &ScepChallenge)
        -> Result<(), CertError>
    {
        let conn = db!(self);
        conn.execute(
            "INSERT OR REPLACE INTO scep_challenge \
             (id, tenant_id, password_hash, used, expires_at, data) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                challenge.id, tenant_id, challenge.password_hash,
                challenge.used as i64, to_ts(challenge.expires_at),
                ser(challenge)?,
            ],
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        Ok(())
    }

    fn consume_scep_challenge(&self, tenant_id: &str, password_hash: &str)
        -> Result<bool, CertError>
    {
        let now = to_ts(time::OffsetDateTime::now_utc());
        let conn = db!(self);
        let id: Option<String> = conn.query_row(
            "SELECT id FROM scep_challenge \
             WHERE tenant_id = ?1 AND password_hash = ?2 AND used = 0 AND expires_at > ?3 \
             LIMIT 1",
            params![tenant_id, password_hash, now],
            |r| r.get(0),
        ).optional().map_err(|e| CertError::Storage(e.to_string()))?;
        if let Some(id) = id {
            conn.execute(
                "UPDATE scep_challenge SET used = 1 WHERE id = ?1",
                params![id],
            ).map_err(|e| CertError::Storage(e.to_string()))?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ─── Notifications ────────────────────────────────────────────────────────

    fn store_notification(&self, tenant_id: &str, notification: &NotificationRecord)
        -> Result<(), CertError>
    {
        let conn = db!(self);
        conn.execute(
            "INSERT INTO notification (tenant_id, serial, threshold_days, data) \
             VALUES (?1, ?2, ?3, ?4)",
            params![
                tenant_id, notification.serial,
                notification.threshold_days as i64, ser(notification)?,
            ],
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        Ok(())
    }

    fn was_notification_sent(&self, tenant_id: &str, serial: &str, threshold_days: u32)
        -> Result<bool, CertError>
    {
        let conn = db!(self);
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM notification \
             WHERE tenant_id = ?1 AND serial = ?2 AND threshold_days = ?3",
            params![tenant_id, serial, threshold_days as i64],
            |r| r.get(0),
        ).unwrap_or(0);
        Ok(count > 0)
    }

    // ─── Audit ────────────────────────────────────────────────────────────────

    fn store_audit_event(&self, tenant_id: &str, event: &AuditEvent) -> Result<(), CertError> {
        let conn = db!(self);
        conn.execute(
            "INSERT INTO audit_log (tenant_id, timestamp, action, serial, data) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                tenant_id, to_ts(event.timestamp),
                format!("{:?}", event.action),
                event.serial,
                ser(event)?,
            ],
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        Ok(())
    }

    fn get_audit_log(&self, tenant_id: &str, filter: &AuditFilter, page: &Pagination)
        -> Result<PagedResult<AuditEvent>, CertError>
    {
        let conn = db!(self);
        let mut conds = vec!["tenant_id = ?1".to_string()];
        let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(tenant_id.to_string())];
        let mut n = 2usize;

        if let Some(action) = &filter.action {
            conds.push(format!("action = ?{}", n));
            bind.push(Box::new(format!("{:?}", action)));
            n += 1;
        }
        if let Some(serial) = &filter.serial {
            conds.push(format!("serial = ?{}", n));
            bind.push(Box::new(serial.clone()));
            n += 1;
        }
        if let Some(actor) = &filter.actor {
            conds.push(format!("json_extract(data,'$.actor') = ?{}", n));
            bind.push(Box::new(actor.clone()));
            n += 1;
        }
        if let Some(from) = filter.from {
            conds.push(format!("timestamp >= ?{}", n));
            bind.push(Box::new(to_ts(from)));
            n += 1;
        }
        if let Some(to) = filter.to {
            conds.push(format!("timestamp <= ?{}", n));
            bind.push(Box::new(to_ts(to)));
            n += 1;
        }

        let where_clause = conds.join(" AND ");
        let count_sql = format!("SELECT COUNT(*) FROM audit_log WHERE {}", where_clause);
        let data_sql = format!(
            "SELECT data FROM audit_log WHERE {} ORDER BY timestamp DESC LIMIT ?{} OFFSET ?{}",
            where_clause, n, n + 1
        );

        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
        let total: i64 = conn.query_row(&count_sql, refs.as_slice(), |r| r.get(0)).unwrap_or(0);

        bind.push(Box::new(page.limit as i64));
        bind.push(Box::new(page.offset as i64));
        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();

        let mut stmt = conn.prepare(&data_sql).map_err(|e| CertError::Storage(e.to_string()))?;
        let items: Vec<AuditEvent> = stmt
            .query_map(refs.as_slice(), |r| r.get::<_, String>(0))
            .map_err(|e| CertError::Storage(e.to_string()))?
            .filter_map(|r| r.ok())
            .filter_map(|data| de::<AuditEvent>(&data).ok())
            .collect();

        Ok(PagedResult { items, total: total as u64, offset: page.offset, limit: page.limit })
    }

    // ─── CRL coordination ────────────────────────────────────────────────────

    fn acquire_crl_lock(
        &self, tenant_id: &str, lock_key: &str, holder_id: &str, ttl_secs: u64,
    ) -> Result<Option<u64>, CertError> {
        let now = to_ts(time::OffsetDateTime::now_utc());
        let expires_at = now + ttl_secs as i64;
        let conn = db!(self);

        // Evict expired lock so our INSERT can succeed.
        conn.execute(
            "DELETE FROM crl_lock WHERE lock_key = ?1 AND expires_at < ?2",
            params![lock_key, now],
        ).map_err(|e| CertError::Storage(e.to_string()))?;

        // Insert only if no lock held by another holder.
        conn.execute(
            "INSERT OR IGNORE INTO crl_lock \
             (lock_key, tenant_id, holder_id, expires_at, generation) \
             VALUES (?1, ?2, ?3, ?4, 1)",
            params![lock_key, tenant_id, holder_id, expires_at],
        ).map_err(|e| CertError::Storage(e.to_string()))?;

        // If we already own the lock, refresh TTL and bump generation.
        conn.execute(
            "UPDATE crl_lock SET expires_at = ?1, generation = generation + 1 \
             WHERE lock_key = ?2 AND holder_id = ?3",
            params![expires_at, lock_key, holder_id],
        ).map_err(|e| CertError::Storage(e.to_string()))?;

        let gen: Option<i64> = conn.query_row(
            "SELECT generation FROM crl_lock WHERE lock_key = ?1 AND holder_id = ?2",
            params![lock_key, holder_id],
            |r| r.get(0),
        ).optional().map_err(|e| CertError::Storage(e.to_string()))?;

        Ok(gen.map(|g| g as u64))
    }

    fn release_crl_lock(&self, tenant_id: &str, lock_key: &str, holder_id: &str)
        -> Result<(), CertError>
    {
        let conn = db!(self);
        conn.execute(
            "DELETE FROM crl_lock WHERE lock_key = ?1 AND tenant_id = ?2 AND holder_id = ?3",
            params![lock_key, tenant_id, holder_id],
        ).map_err(|e| CertError::Storage(e.to_string()))?;
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

fn collect_records<T: serde::de::DeserializeOwned>(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> Result<Vec<T>, CertError> {
    let rows = stmt.query_map(params, |r| r.get::<_, String>(0))
        .map_err(|e| CertError::Storage(e.to_string()))?;
    let mut out = Vec::new();
    for row in rows {
        let data = row.map_err(|e| CertError::Storage(e.to_string()))?;
        out.push(de::<T>(&data)?);
    }
    Ok(out)
}
