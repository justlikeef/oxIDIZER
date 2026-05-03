use ox_data_object::GenericDataObject;
use uuid::Uuid;
use ox_data_error::OxDataError;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

const EXT_KEY: &str = "ox.transaction";

/// Serialized transaction state stored in the GDO extension slot under "ox.transaction".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TransactionState {
    #[serde(default)]
    status: LockStatusData,
    #[serde(default)]
    transaction_active: bool,
    /// Holder UUID preserved so commit/rollback can unlock without an explicit holder arg.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_holder: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LockStatusData {
    #[default]
    Unlocked,
    Locked {
        holder: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<DateTime<Utc>>,
    },
    PendingLock {
        holder: Uuid,
    },
    PendingUnlock {
        holder: Uuid,
    },
}

/// Public lock-status type returned by the `Transactable` trait.
#[derive(Debug, Clone, PartialEq)]
pub enum LockStatus {
    Unlocked,
    Locked { holder: Uuid, expires_at: Option<DateTime<Utc>> },
    PendingLock(Uuid),
    PendingUnlock(Uuid),
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn load_state(gdo: &GenericDataObject) -> TransactionState {
    gdo.get_extension(EXT_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

fn save_state(gdo: &mut GenericDataObject, state: TransactionState) {
    if let Ok(v) = serde_json::to_value(&state) {
        gdo.set_extension(EXT_KEY, v);
    }
}

fn is_expired_state(status: &LockStatusData) -> bool {
    if let LockStatusData::Locked { expires_at: Some(exp), .. } = status {
        return *exp < Utc::now();
    }
    false
}

fn effective_status(state: &TransactionState) -> &LockStatusData {
    // Treat an expired lock as Unlocked when reading status.
    if is_expired_state(&state.status) { &LockStatusData::Unlocked } else { &state.status }
}

// ─── trait ───────────────────────────────────────────────────────────────────

pub trait Transactable {
    fn get_lock_status(&self) -> LockStatus;
    fn set_lock_status(&mut self, status: LockStatus);

    fn lock(&mut self, holder: Uuid, ttl: Option<Duration>) -> Result<(), OxDataError>;
    fn unlock(&mut self, holder: Uuid) -> Result<(), OxDataError>;
    /// Break any lock regardless of holder. `admin_token` is checked against
    /// the `OX_FORCE_UNLOCK_TOKEN` environment variable (or any token when not set).
    fn force_unlock(&mut self, admin_token: &str) -> Result<(), OxDataError>;
    fn request_lock(&mut self, holder: Uuid, ttl: Option<Duration>);
    fn request_unlock(&mut self, holder: Uuid);

    fn is_locked(&self) -> bool;
    fn is_locked_by(&self, holder: Uuid) -> bool;
    fn is_expired(&self) -> bool;

    fn transaction_active(&self) -> bool;
    fn begin_transaction(&mut self, holder: Uuid, ttl: Option<Duration>) -> Result<(), OxDataError>;
    /// Unlock and clear transaction state, firing `after_commit`.
    fn commit_transaction(&mut self) -> Result<(), OxDataError>;
    /// Unlock and clear transaction state without committing, firing `after_rollback`.
    fn rollback_transaction(&mut self) -> Result<(), OxDataError>;
}

// ─── impl on GenericDataObject ───────────────────────────────────────────────

impl Transactable for GenericDataObject {
    fn get_lock_status(&self) -> LockStatus {
        let state = load_state(self);
        match effective_status(&state) {
            LockStatusData::Unlocked => LockStatus::Unlocked,
            LockStatusData::Locked { holder, expires_at } => LockStatus::Locked { holder: *holder, expires_at: *expires_at },
            LockStatusData::PendingLock { holder } => LockStatus::PendingLock(*holder),
            LockStatusData::PendingUnlock { holder } => LockStatus::PendingUnlock(*holder),
        }
    }

    fn set_lock_status(&mut self, status: LockStatus) {
        let mut state = load_state(self);
        state.status = match status {
            LockStatus::Unlocked => LockStatusData::Unlocked,
            LockStatus::Locked { holder, expires_at } => LockStatusData::Locked { holder, expires_at },
            LockStatus::PendingLock(holder) => LockStatusData::PendingLock { holder },
            LockStatus::PendingUnlock(holder) => LockStatusData::PendingUnlock { holder },
        };
        save_state(self, state);
    }

    fn lock(&mut self, holder: Uuid, ttl: Option<Duration>) -> Result<(), OxDataError> {
        self.trigger_callbacks("before_lock", None, None, None)?;

        let mut state = load_state(self);

        // Expire stale lock before checking.
        let effective = is_expired_state(&state.status);
        if effective {
            state.status = LockStatusData::Unlocked;
        }

        match &state.status {
            LockStatusData::Unlocked | LockStatusData::PendingLock { .. } => {
                let expires_at = ttl.map(|d| Utc::now() + d);
                state.status = LockStatusData::Locked { holder, expires_at };
                save_state(self, state);
                self.trigger_callbacks("after_lock", None, Some(&holder.to_string()), None)?;
                Ok(())
            }
            LockStatusData::Locked { holder: existing, .. } if *existing == holder => {
                // Re-entrant lock by same holder — refresh expires_at only.
                let expires_at = ttl.map(|d| Utc::now() + d);
                state.status = LockStatusData::Locked { holder, expires_at };
                save_state(self, state);
                self.trigger_callbacks("after_lock", None, Some(&holder.to_string()), None)?;
                Ok(())
            }
            _ => {
                let msg = "Object is locked by another holder".to_string();
                let _ = self.trigger_callbacks("on_error_lock", None, None, Some(&msg));
                Err(OxDataError::TransactionError(msg))
            }
        }
    }

    fn unlock(&mut self, holder: Uuid) -> Result<(), OxDataError> {
        self.trigger_callbacks("before_unlock", None, None, None)?;

        let mut state = load_state(self);

        match &state.status {
            LockStatusData::Locked { holder: h, .. } if *h == holder => {
                state.status = LockStatusData::Unlocked;
                save_state(self, state);
                self.trigger_callbacks("after_unlock", None, Some(&holder.to_string()), None)?;
                Ok(())
            }
            LockStatusData::Unlocked => {
                // Already unlocked — idempotent success.
                self.trigger_callbacks("after_unlock", None, None, None)?;
                Ok(())
            }
            _ => {
                let msg = "Cannot unlock: lock is held by another holder".to_string();
                let _ = self.trigger_callbacks("on_error_unlock", None, None, Some(&msg));
                Err(OxDataError::TransactionError(msg))
            }
        }
    }

    fn force_unlock(&mut self, admin_token: &str) -> Result<(), OxDataError> {
        self.trigger_callbacks("before_force_unlock", None, None, None)?;

        let configured = std::env::var("OX_FORCE_UNLOCK_TOKEN").unwrap_or_default();
        if !configured.is_empty() && admin_token != configured {
            let msg = "Invalid admin token for force unlock".to_string();
            let _ = self.trigger_callbacks("on_error_force_unlock", None, None, Some(&msg));
            return Err(OxDataError::TransactionError(msg));
        }

        let mut state = load_state(self);
        state.status = LockStatusData::Unlocked;
        state.transaction_active = false;
        state.active_holder = None;
        save_state(self, state);

        self.trigger_callbacks("after_force_unlock", None, None, None)?;
        Ok(())
    }

    fn request_lock(&mut self, holder: Uuid, _ttl: Option<Duration>) {
        let _ = self.trigger_callbacks("before_request_lock", None, Some(&holder.to_string()), None);
        let mut state = load_state(self);
        if matches!(state.status, LockStatusData::Unlocked) {
            state.status = LockStatusData::PendingLock { holder };
            save_state(self, state);
        }
        let _ = self.trigger_callbacks("after_request_lock", None, Some(&holder.to_string()), None);
    }

    fn request_unlock(&mut self, holder: Uuid) {
        let _ = self.trigger_callbacks("before_request_unlock", None, Some(&holder.to_string()), None);
        let mut state = load_state(self);
        if matches!(&state.status, LockStatusData::Locked { holder: h, .. } if *h == holder) {
            state.status = LockStatusData::PendingUnlock { holder };
            save_state(self, state);
        }
        let _ = self.trigger_callbacks("after_request_unlock", None, Some(&holder.to_string()), None);
    }

    fn is_locked(&self) -> bool {
        let state = load_state(self);
        matches!(effective_status(&state), LockStatusData::Locked { .. })
    }

    fn is_locked_by(&self, holder: Uuid) -> bool {
        let state = load_state(self);
        matches!(effective_status(&state), LockStatusData::Locked { holder: h, .. } if *h == holder)
    }

    fn is_expired(&self) -> bool {
        let state = load_state(self);
        is_expired_state(&state.status)
    }

    fn transaction_active(&self) -> bool {
        load_state(self).transaction_active
    }

    fn begin_transaction(&mut self, holder: Uuid, ttl: Option<Duration>) -> Result<(), OxDataError> {
        self.lock(holder, ttl)?;
        let mut state = load_state(self);
        state.transaction_active = true;
        state.active_holder = Some(holder);
        save_state(self, state);
        Ok(())
    }

    fn commit_transaction(&mut self) -> Result<(), OxDataError> {
        let state = load_state(self);
        let holder = state.active_holder
            .ok_or_else(|| OxDataError::TransactionError("No active transaction".to_string()))?;
        self.unlock(holder)?;
        let mut state = load_state(self);
        state.transaction_active = false;
        state.active_holder = None;
        save_state(self, state);
        self.trigger_callbacks("after_commit", None, None, None)?;
        Ok(())
    }

    fn rollback_transaction(&mut self) -> Result<(), OxDataError> {
        let state = load_state(self);
        let holder = state.active_holder
            .ok_or_else(|| OxDataError::TransactionError("No active transaction".to_string()))?;
        self.unlock(holder)?;
        let mut state = load_state(self);
        state.transaction_active = false;
        state.active_holder = None;
        save_state(self, state);
        let _ = self.trigger_callbacks("after_rollback", None, None, None);
        Ok(())
    }
}

// ─── legacy wrapper (kept for backward compatibility) ─────────────────────────

/// Wraps a GDO to provide `Transactable` delegation. Prefer `impl Transactable for
/// GenericDataObject` in new code.
pub struct TransactableGenericDataObject {
    gdo: GenericDataObject,
}

impl TransactableGenericDataObject {
    pub fn new(gdo: GenericDataObject) -> Self {
        Self { gdo }
    }

    pub fn into_gdo(self) -> GenericDataObject {
        self.gdo
    }
}

impl Transactable for TransactableGenericDataObject {
    fn get_lock_status(&self) -> LockStatus { self.gdo.get_lock_status() }
    fn set_lock_status(&mut self, s: LockStatus) { self.gdo.set_lock_status(s) }
    fn lock(&mut self, h: Uuid, t: Option<Duration>) -> Result<(), OxDataError> { self.gdo.lock(h, t) }
    fn unlock(&mut self, h: Uuid) -> Result<(), OxDataError> { self.gdo.unlock(h) }
    fn force_unlock(&mut self, tok: &str) -> Result<(), OxDataError> { self.gdo.force_unlock(tok) }
    fn request_lock(&mut self, h: Uuid, t: Option<Duration>) { self.gdo.request_lock(h, t) }
    fn request_unlock(&mut self, h: Uuid) { self.gdo.request_unlock(h) }
    fn is_locked(&self) -> bool { self.gdo.is_locked() }
    fn is_locked_by(&self, h: Uuid) -> bool { self.gdo.is_locked_by(h) }
    fn is_expired(&self) -> bool { self.gdo.is_expired() }
    fn transaction_active(&self) -> bool { self.gdo.transaction_active() }
    fn begin_transaction(&mut self, h: Uuid, t: Option<Duration>) -> Result<(), OxDataError> { self.gdo.begin_transaction(h, t) }
    fn commit_transaction(&mut self) -> Result<(), OxDataError> { self.gdo.commit_transaction() }
    fn rollback_transaction(&mut self) -> Result<(), OxDataError> { self.gdo.rollback_transaction() }
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_gdo() -> GenericDataObject {
        GenericDataObject::new("test", None)
    }

    #[test]
    fn test_initial_state_unlocked() {
        let gdo = make_gdo();
        assert_eq!(gdo.get_lock_status(), LockStatus::Unlocked);
        assert!(!gdo.is_locked());
        assert!(!gdo.transaction_active());
    }

    #[test]
    fn test_lock_and_unlock() {
        let mut gdo = make_gdo();
        let holder = Uuid::new_v4();
        gdo.lock(holder, None).unwrap();
        assert!(gdo.is_locked());
        assert!(gdo.is_locked_by(holder));
        gdo.unlock(holder).unwrap();
        assert!(!gdo.is_locked());
        assert_eq!(gdo.get_lock_status(), LockStatus::Unlocked);
    }

    #[test]
    fn test_lock_rejects_different_holder() {
        let mut gdo = make_gdo();
        let holder_a = Uuid::new_v4();
        let holder_b = Uuid::new_v4();
        gdo.lock(holder_a, None).unwrap();
        assert!(gdo.lock(holder_b, None).is_err());
    }

    #[test]
    fn test_ttl_expiry() {
        let mut gdo = make_gdo();
        let holder = Uuid::new_v4();
        // Lock with a TTL in the past to simulate expiry.
        let expired_at = Utc::now() - Duration::seconds(1);
        let mut state = load_state(&gdo);
        state.status = LockStatusData::Locked { holder, expires_at: Some(expired_at) };
        save_state(&mut gdo, state);

        assert!(gdo.is_expired());
        assert!(!gdo.is_locked()); // expired lock is treated as unlocked

        // A second holder should now be able to acquire it.
        let holder_b = Uuid::new_v4();
        gdo.lock(holder_b, None).unwrap();
        assert!(gdo.is_locked_by(holder_b));
    }

    #[test]
    fn test_begin_commit_transaction() {
        let mut gdo = make_gdo();
        let holder = Uuid::new_v4();
        gdo.begin_transaction(holder, None).unwrap();
        assert!(gdo.transaction_active());
        assert!(gdo.is_locked_by(holder));
        gdo.commit_transaction().unwrap();
        assert!(!gdo.transaction_active());
        assert!(!gdo.is_locked());
    }

    #[test]
    fn test_begin_rollback_transaction() {
        let mut gdo = make_gdo();
        let holder = Uuid::new_v4();
        gdo.begin_transaction(holder, None).unwrap();
        gdo.rollback_transaction().unwrap();
        assert!(!gdo.transaction_active());
        assert!(!gdo.is_locked());
    }

    #[test]
    fn test_legacy_wrapper() {
        let gdo = make_gdo();
        let mut twrapper = TransactableGenericDataObject::new(gdo);
        let holder = Uuid::new_v4();
        twrapper.lock(holder, None).unwrap();
        assert!(twrapper.is_locked_by(holder));
        twrapper.unlock(holder).unwrap();
        assert!(!twrapper.is_locked());
    }
}
