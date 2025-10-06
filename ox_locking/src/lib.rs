use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;
use std::any::Any;

use ox_data_object::generic_data_object::{AttributeValue, GenericDataObject, DataObjectState, PersistenceInfo};
use ox_persistence::{Persistent, PersistenceDriver, PERSISTENCE_DRIVER_REGISTRY};
use ox_type_converter::CONVERSION_REGISTRY;

// ... (rest of the file is the same until LockableGenericDataObject)

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
}

impl Lockable for LockableGenericDataObject {
    // ... (implementation of Lockable is the same)
}

impl Persistent for LockableGenericDataObject {
    fn persist(&mut self, driver_name: &str, location: &str) -> Result<(), String> {
        if self.get_lock_status() != LockStatus::Update {
            return Err("Object must be in an Update lock state to persist.".to_string());
        }

        let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        let (driver, _) = registry.get_driver(driver_name).ok_or_else(|| "Persistence driver not found.".to_string())?;

        let serializable_map = self.gdo.to_serializable_map();
        driver.persist(&serializable_map, location)?;

        if let Some(lock) = self.get_current_lock() {
            let mut lock_guard = lock.lock().unwrap();
            lock_guard.commit();
        }
        
        self.set_lock_status(LockStatus::None);
        self.set_current_lock(None);

        Ok(())
    }

    fn fetch(&self, driver_name: &str, location: &str) -> Result<Vec<GenericDataObject>, String> {
        let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        let (driver, _) = registry.get_driver(driver_name).ok_or_else(|| "Persistence driver not found.".to_string())?;

        let filter = self.gdo.to_serializable_map();
        let fetched_ids = driver.fetch(&filter, location)?;

        let mut fetched_objects = Vec::new();
        for id in fetched_ids {
            let mut gdo = GenericDataObject::new(&self.gdo.identifier_name, Some(Uuid::parse_str(&id).map_err(|e| e.to_string())?));
            gdo.state = DataObjectState::NotHydrated;
            gdo.persistence_info = Some(PersistenceInfo {
                driver_name: driver_name.to_string(),
                location: location.to_string(),
            });
            fetched_objects.push(gdo);
        }

        Ok(fetched_objects)
    }
}
