use ox_data_object::{
    GenericDataObject,
    AttributeValue,
};
use ox_persistence::{PersistenceDriver, DataSet};
use ox_locking::LockStatus;
use ox_type_converter::ValueType;
use std::collections::HashMap;

pub struct OxPersistenceApiDriver;

impl PersistenceDriver for OxPersistenceApiDriver {
    fn persist(
        &self,
        _serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        _location: &str,
    ) -> Result<(), String> {
        Err("Not implemented".to_string())
    }

    fn restore(
        &self,
        _location: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        Err("Not implemented".to_string())
    }

    fn fetch(
        &self,
        _filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        _location: &str,
    ) -> Result<Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>, String> {
        Err("Not implemented".to_string())
    }

    fn notify_lock_status_change(&self, lock_status: LockStatus, gdo_id: usize) {
        println!("OxPersistenceApiDriver: GDO {} lock status changed to {:?}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, _connection_info: &HashMap<String, String>) -> Result<(), String> {
        Err("Not implemented".to_string())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        Err("Not implemented".to_string())
    }

    fn describe_dataset(&self, _connection_info: &HashMap<String, String>, _dataset_name: &str) -> Result<DataSet, String> {
        Err("Not implemented".to_string())
    }
}
