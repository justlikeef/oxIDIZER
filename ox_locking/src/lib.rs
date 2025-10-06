use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;
use std::any::Any;

use ox_data_object::generic_data_object::GenericDataObject;


/// Represents the locking status of a GenericDataObject.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LockStatus {
    None,
    Read,
    Update,
    Delete,
}

/// Represents a lock on a GenericDataObject.
#[derive(Debug, Clone)]
pub struct Lock {
    pub id: Uuid,
    pub owner: String,
    pub created_at: Instant,
    pub expires_at: Instant,
    pub lock_type: LockStatus, // Use LockStatus from ox_locking
}

impl Lock {
    pub fn new(owner: String, duration: Duration, lock_type: LockStatus) -> Self {
        let now = Instant::now();
        Self {
            id: Uuid::new_v4(),
            owner,
            created_at: now,
            expires_at: now + duration,
            lock_type,
        }
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() > self.expires_at
    }

    pub fn commit(&mut self) {
        // For now, committing just extends the lock indefinitely or marks it as resolved.
        // A more complex system might involve notifying a central lock manager.
        self.expires_at = Instant::now() + Duration::from_secs(3600 * 24 * 365 * 100); // Effectively never expires
    }
}

/// Trait for objects that can be locked.
pub trait Lockable {
    fn acquire_lock(&mut self, owner: String, duration: Duration, lock_type: LockStatus) -> Result<Arc<Mutex<Lock>>, String>;
    fn release_lock(&mut self, lock_id: &Uuid) -> Result<(), String>;
    fn get_lock_status(&self) -> LockStatus;
    fn get_current_lock(&self) -> Option<Arc<Mutex<Lock>>>;
    fn set_lock_status(&mut self, status: LockStatus);
    fn set_current_lock(&mut self, lock: Option<Arc<Mutex<Lock>>>);
}

pub struct LockableGenericDataObject {
    pub gdo: GenericDataObject,
    lock_status: LockStatus,
    current_lock: Option<Arc<Mutex<Lock>>>,
}

impl LockableGenericDataObject {
    pub fn new(gdo: GenericDataObject) -> Self {
        Self {
            gdo,
            lock_status: LockStatus::None,
            current_lock: None,
        }
    }

    // Helper to get the GDO's ID
    fn get_gdo_id(&self) -> Result<String, String> {
        self.gdo.get_raw_value::<String>(&self.gdo.identifier_name)
            .ok_or_else(|| "GenericDataObject has no ID.".to_string())
    }

    // Custom get method for LockableGenericDataObject
    pub fn get<T: Clone + 'static + Default>(&mut self, identifier: &str) -> Option<T>
    where
        T: Any + Clone,
    {
        self.gdo.get(identifier)
    }

    // Custom set method for LockableGenericDataObject
    pub fn set<T: Any + Send + Sync + Clone + 'static>(&mut self, identifier: &str, value: T, _lock: Option<Arc<Mutex<Lock>>>) {
        // Update state if the object is not new
        // This logic will be moved to a persistence wrapper
        self.gdo.set(identifier, value);
    }
}

impl Lockable for LockableGenericDataObject {
    fn acquire_lock(&mut self, owner: String, duration: Duration, lock_type: LockStatus) -> Result<Arc<Mutex<Lock>>, String> {
        if self.lock_status != LockStatus::None {
            return Err(format!("Object already locked with status: {:?}", self.lock_status));
        }

        let new_lock = Arc::new(Mutex::new(Lock::new(owner, duration, lock_type.clone())));
        self.lock_status = lock_type;
        self.current_lock = Some(new_lock.clone());

        // Notify persistence driver about lock status change
        // This logic will be moved to a persistence wrapper
        // if let Some(info) = &self.persistence_info {
        //     let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        //     if let Some((driver, _)) = registry.get_driver(&info.driver_name) {
        //         driver.notify_lock_status_change(&self.lock_status.to_string(), &self.get_gdo_id()?);
        //     }
        // }

        Ok(new_lock)
    }

    fn release_lock(&mut self, lock_id: &Uuid) -> Result<(), String> {
        if let Some(current_lock_arc) = self.current_lock.take() {
            let current_lock = current_lock_arc.lock().unwrap();
            if &current_lock.id == lock_id {
                self.lock_status = LockStatus::None;
                // Notify persistence driver about lock status change
                // This logic will be moved to a persistence wrapper
                // if let Some(info) = &self.persistence_info {
                //     let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
                //     if let Some((driver, _)) = registry.get_driver(&info.driver_name) {
                //         driver.notify_lock_status_change(&self.lock_status.to_string(), &self.get_gdo_id()?);
                //     }
                // }

                Ok(())
            } else {
                self.current_lock = Some(current_lock_arc.clone()); // Put it back if it didn't match
                Err("Lock ID does not match current lock.".to_string())
            }
        } else {
            Err("No lock currently held.".to_string())
        }
    }

    fn get_lock_status(&self) -> LockStatus {
        self.lock_status.clone()
    }

    fn get_current_lock(&self) -> Option<Arc<Mutex<Lock>>> {
        self.current_lock.clone()
    }

    fn set_lock_status(&mut self, status: LockStatus) {
        self.lock_status = status;
    }

    fn set_current_lock(&mut self, lock: Option<Arc<Mutex<Lock>>>) {
        self.current_lock = lock;
    }
}