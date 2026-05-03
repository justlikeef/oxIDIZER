use crate::model::*;
use crate::CertError;
use ox_data_object::GenericDataObject;
use ox_data_object_manager::DataObjectManager;
use std::sync::Arc;


// ---------------------------------------------------------------------------
// CertStore trait
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// OxPersistenceCertStore — GDO-backed implementation
// ---------------------------------------------------------------------------

/// Persists every record as a JSON blob in the `data` attribute of a GDO.
/// Keys and tenant filtering are stored as separate indexed attributes so that
/// the backing driver can execute efficient lookups.
///
/// Format in each GDO:
///   `data`      — full `serde_json::to_string(record)` value
///   `tenant_id` — tenant partition key
///   `<pk>`      — primary key attribute (e.g. `serial`, `id`) for lookup
pub struct OxPersistenceCertStore {
    dom: Arc<DataObjectManager>,
}

impl OxPersistenceCertStore {
    /// Construct from a pre-configured DataObjectManager.
    pub fn new(dom: Arc<DataObjectManager>) -> Self {
        Self { dom }
    }

    /// Create a fresh store with all cert schemas already registered.
    /// This is the recommended constructor — call migrate() after open() to
    /// ensure schemas are current (it is idempotent).
    pub fn open() -> Result<Self, CertError> {
        let mut dom = DataObjectManager::new();
        crate::register_schemas(&mut dom.dictionary)
            .map_err(|e| CertError::Storage(format!("schema registration failed: {}", e)))?;
        Ok(Self { dom: Arc::new(dom) })
    }

    // ── internal helpers ────────────────────────────────────────────────────

    fn to_gdo<T: serde::Serialize>(
        identifier_name: &str,
        pk_field: &str,
        pk_value: &str,
        tenant_id: &str,
        record: &T,
    ) -> Result<GenericDataObject, CertError> {
        let json =
            serde_json::to_string(record).map_err(|e| CertError::Internal(e.to_string()))?;
        let mut gdo = GenericDataObject::new(identifier_name, None);
        gdo.set(pk_field, pk_value.to_string())
            .map_err(|e| CertError::Internal(e.to_string()))?;
        gdo.set("tenant_id", tenant_id.to_string())
            .map_err(|e| CertError::Internal(e.to_string()))?;
        gdo.set("data", json)
            .map_err(|e| CertError::Internal(e.to_string()))?;
        Ok(gdo)
    }

    fn from_gdo<T: serde::de::DeserializeOwned>(
        gdo: &GenericDataObject,
        tenant_id: &str,
    ) -> Result<Option<T>, CertError> {
        let map = gdo.to_serializable_map();
        let stored_tenant = map.get("tenant_id").map(|(v, _, _)| v.as_str()).unwrap_or("");
        if stored_tenant != tenant_id {
            return Ok(None);
        }
        let data_str = map
            .get("data")
            .map(|(v, _, _)| v.as_str())
            .ok_or_else(|| CertError::Internal("GDO missing 'data' field".to_string()))?;
        let record: T =
            serde_json::from_str(data_str).map_err(|e| CertError::Internal(e.to_string()))?;
        Ok(Some(record))
    }

    fn save<T: serde::Serialize>(
        &self,
        identifier_name: &str,
        pk_field: &str,
        pk_value: &str,
        tenant_id: &str,
        record: &T,
    ) -> Result<(), CertError> {
        let gdo = Self::to_gdo(identifier_name, pk_field, pk_value, tenant_id, record)?;
        self.dom
            .save_data_object(&gdo)
            .map_err(|e| CertError::Storage(e.to_string()))
    }

    fn load<T: serde::de::DeserializeOwned>(
        &self,
        identifier_name: &str,
        pk_value: &str,
        tenant_id: &str,
    ) -> Result<Option<T>, CertError> {
        match self.dom.load_data_object(identifier_name, pk_value) {
            Ok(gdo) => Self::from_gdo(&gdo, tenant_id),
            Err(_) => Ok(None),
        }
    }

    fn empty_page<T>(&self, page: &Pagination) -> PagedResult<T> {
        PagedResult { items: vec![], total: 0, offset: page.offset, limit: page.limit }
    }
}

// ---------------------------------------------------------------------------
// CertStore impl
// ---------------------------------------------------------------------------

impl CertStore for OxPersistenceCertStore {
    fn migrate(&self) -> Result<(), CertError> {
        // Schemas are registered at open() time. Verify the core schema is present.
        if self.dom.dictionary.objects.contains_key("certificate") {
            Ok(())
        } else {
            Err(CertError::Storage(
                "cert schemas not registered; construct the store via OxPersistenceCertStore::open()".to_string(),
            ))
        }
    }

    // --- X.509 Certificates ---

    fn store_cert(&self, tenant_id: &str, record: &CertificateRecord) -> Result<(), CertError> {
        self.save("certificate", "serial", &record.serial, tenant_id, record)
    }

    fn get_cert_by_serial(
        &self,
        tenant_id: &str,
        serial: &str,
    ) -> Result<Option<CertificateRecord>, CertError> {
        self.load("certificate", serial, tenant_id)
    }

    fn get_certs_by_subject(
        &self,
        _tenant_id: &str,
        _subject_cn: &str,
    ) -> Result<Vec<CertificateRecord>, CertError> {
        // Full-scan filtering requires driver-level fetch support.
        // Stub: returns empty until query engine filter propagation is implemented.
        Ok(vec![])
    }

    fn list_certs(
        &self,
        _tenant_id: &str,
        _filter: &CertFilter,
        page: &Pagination,
    ) -> Result<PagedResult<CertificateRecord>, CertError> {
        Ok(self.empty_page(page))
    }

    fn mark_revoked(
        &self,
        tenant_id: &str,
        serial: &str,
        reason: RevocationReason,
        timestamp: time::OffsetDateTime,
    ) -> Result<(), CertError> {
        let mut record = self
            .get_cert_by_serial(tenant_id, serial)?
            .ok_or_else(|| CertError::NotFound(serial.to_string()))?;
        record.status = CertStatus::Revoked;
        record.revoked_at = Some(timestamp);
        record.revocation_reason = Some(reason);
        self.store_cert(tenant_id, &record)
    }

    fn list_revoked(&self, _tenant_id: &str) -> Result<Vec<CertificateRecord>, CertError> {
        Ok(vec![])
    }

    fn list_revoked_since(
        &self,
        _tenant_id: &str,
        _since: time::OffsetDateTime,
    ) -> Result<Vec<CertificateRecord>, CertError> {
        Ok(vec![])
    }

    fn list_expiring(
        &self,
        _tenant_id: &str,
        _within_days: u32,
    ) -> Result<Vec<CertificateRecord>, CertError> {
        Ok(vec![])
    }

    fn update_status_expired(&self, _tenant_id: &str) -> Result<u64, CertError> {
        Ok(0)
    }

    // --- SSH Certificates ---

    fn store_ssh_cert(
        &self,
        tenant_id: &str,
        record: &SshCertRecord,
    ) -> Result<(), CertError> {
        self.save("ssh_certificate", "serial", &record.serial.to_string(), tenant_id, record)
    }

    fn get_ssh_cert_by_serial(
        &self,
        tenant_id: &str,
        serial: u64,
    ) -> Result<Option<SshCertRecord>, CertError> {
        self.load("ssh_certificate", &serial.to_string(), tenant_id)
    }

    fn list_ssh_certs(
        &self,
        _tenant_id: &str,
        _filter: &SshCertFilter,
        page: &Pagination,
    ) -> Result<PagedResult<SshCertRecord>, CertError> {
        Ok(self.empty_page(page))
    }

    fn get_next_ssh_serial(&self, _tenant_id: &str) -> Result<u64, CertError> {
        // Stub: returns a random u64. Production would use an atomic counter in the store.
        Ok(rand_u64())
    }

    // --- CA Keys ---

    fn store_ca_key(&self, tenant_id: &str, key: &CaKeyRecord) -> Result<(), CertError> {
        self.save("ca_key", "id", &key.id, tenant_id, key)
    }

    fn get_active_ca_key(
        &self,
        _tenant_id: &str,
    ) -> Result<Option<CaKeyRecord>, CertError> {
        // Stub: full-scan by status not yet supported without filter propagation.
        Ok(None)
    }

    fn get_ca_key_by_id(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<CaKeyRecord>, CertError> {
        self.load("ca_key", id, tenant_id)
    }

    fn update_ca_key_status(
        &self,
        tenant_id: &str,
        id: &str,
        status: CaKeyStatus,
    ) -> Result<(), CertError> {
        let mut key = self
            .get_ca_key_by_id(tenant_id, id)?
            .ok_or_else(|| CertError::NotFound(id.to_string()))?;
        key.status = status;
        self.store_ca_key(tenant_id, &key)
    }

    // --- ACME ---

    fn store_acme_account(
        &self,
        tenant_id: &str,
        account: &AcmeAccount,
    ) -> Result<(), CertError> {
        self.save("acme_account", "id", &account.id, tenant_id, account)
    }

    fn get_acme_account(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<AcmeAccount>, CertError> {
        self.load("acme_account", id, tenant_id)
    }

    fn store_acme_order(&self, tenant_id: &str, order: &AcmeOrder) -> Result<(), CertError> {
        self.save("acme_order", "id", &order.id, tenant_id, order)
    }

    fn get_acme_order(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<AcmeOrder>, CertError> {
        self.load("acme_order", id, tenant_id)
    }

    fn update_acme_order_status(
        &self,
        tenant_id: &str,
        id: &str,
        status: AcmeOrderStatus,
    ) -> Result<(), CertError> {
        let mut order = self
            .get_acme_order(tenant_id, id)?
            .ok_or_else(|| CertError::NotFound(id.to_string()))?;
        order.status = status;
        self.store_acme_order(tenant_id, &order)
    }

    fn store_acme_authorization(
        &self,
        tenant_id: &str,
        authz: &AcmeAuthorization,
    ) -> Result<(), CertError> {
        self.save("acme_authorization", "id", &authz.id, tenant_id, authz)
    }

    fn get_acme_authorization(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<AcmeAuthorization>, CertError> {
        self.load("acme_authorization", id, tenant_id)
    }

    fn update_acme_authorization(
        &self,
        tenant_id: &str,
        authz: &AcmeAuthorization,
    ) -> Result<(), CertError> {
        self.store_acme_authorization(tenant_id, authz)
    }

    // --- RA ---

    fn store_ra_request(
        &self,
        tenant_id: &str,
        request: &ApprovalRequest,
    ) -> Result<(), CertError> {
        self.save("ra_request", "id", &request.id, tenant_id, request)
    }

    fn get_ra_request(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<ApprovalRequest>, CertError> {
        self.load("ra_request", id, tenant_id)
    }

    fn list_ra_pending(
        &self,
        _tenant_id: &str,
        page: &Pagination,
    ) -> Result<PagedResult<ApprovalRequest>, CertError> {
        Ok(self.empty_page(page))
    }

    fn update_ra_request(
        &self,
        tenant_id: &str,
        id: &str,
        status: ApprovalStatus,
        reviewer: &str,
        notes: &str,
    ) -> Result<(), CertError> {
        let mut req = self
            .get_ra_request(tenant_id, id)?
            .ok_or_else(|| CertError::NotFound(id.to_string()))?;
        req.status = status;
        req.reviewer = Some(reviewer.to_string());
        req.review_notes = Some(notes.to_string());
        req.reviewed_at = Some(time::OffsetDateTime::now_utc());
        self.store_ra_request(tenant_id, &req)
    }

    // --- SCEP ---

    fn store_scep_challenge(
        &self,
        tenant_id: &str,
        challenge: &ScepChallenge,
    ) -> Result<(), CertError> {
        self.save("scep_challenge", "id", &challenge.id, tenant_id, challenge)
    }

    fn consume_scep_challenge(
        &self,
        _tenant_id: &str,
        _password_hash: &str,
    ) -> Result<bool, CertError> {
        // Stub: full-scan by hash not yet implemented.
        Ok(false)
    }

    // --- Notifications ---

    fn store_notification(
        &self,
        tenant_id: &str,
        notification: &NotificationRecord,
    ) -> Result<(), CertError> {
        self.save(
            "notification",
            "id",
            &notification.id.to_string(),
            tenant_id,
            notification,
        )
    }

    fn was_notification_sent(
        &self,
        _tenant_id: &str,
        _serial: &str,
        _threshold_days: u32,
    ) -> Result<bool, CertError> {
        Ok(false)
    }

    // --- Audit ---

    fn store_audit_event(&self, tenant_id: &str, event: &AuditEvent) -> Result<(), CertError> {
        self.save("audit_log", "id", &event.id.to_string(), tenant_id, event)
    }

    fn get_audit_log(
        &self,
        _tenant_id: &str,
        _filter: &AuditFilter,
        page: &Pagination,
    ) -> Result<PagedResult<AuditEvent>, CertError> {
        Ok(self.empty_page(page))
    }

    // --- CRL coordination ---

    fn acquire_crl_lock(
        &self,
        _tenant_id: &str,
        _lock_key: &str,
        _holder_id: &str,
        _ttl_secs: u64,
    ) -> Result<Option<u64>, CertError> {
        // Stub: advisory lock via advisory lock table not yet implemented.
        Ok(Some(1))
    }

    fn release_crl_lock(
        &self,
        _tenant_id: &str,
        _lock_key: &str,
        _holder_id: &str,
    ) -> Result<(), CertError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internal utility
// ---------------------------------------------------------------------------

fn rand_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(1);
    nanos as u64
}
