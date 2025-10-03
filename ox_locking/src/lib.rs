use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;
use std::any::Any;

use ox_data_object::generic_data_object::{AttributeValue, GenericDataObject};
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

    fn restore(&mut self, driver_name: &str, location: &str) -> Result<(), String> {
        let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        let (driver, _) = registry.get_driver(driver_name).ok_or_else(|| "Persistence driver not found.".to_string())?;

        let serializable_map = driver.restore(location)?;
        self.gdo = GenericDataObject::from_serializable_map(serializable_map);

        Ok(())
    }

    fn fetch(&self, driver_name: &str, location: &str) -> Result<Vec<GenericDataObject>, String> {
        let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        let (driver, _) = registry.get_driver(driver_name).ok_or_else(|| "Persistence driver not found.".to_string())?;

        let filter = self.gdo.to_serializable_map();
        let fetched_maps = driver.fetch(&filter, location)?;

        let mut fetched_objects = Vec::new();
        for map in fetched_maps {
            fetched_objects.push(GenericDataObject::from_serializable_map(map));
        }

        Ok(fetched_objects)
    }
}