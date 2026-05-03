use ox_data_error::OxDataError;
use ox_persistence::{PersistenceDriver, DataSet, ConnectionParameter};
use ox_type_converter::HashMap;

mod sql_builder;
pub use sql_builder::*;
use ox_type_converter::ValueType;

pub struct SqlPersistenceDriver;

impl PersistenceDriver for SqlPersistenceDriver {
    fn persist(
        &self,
        _serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        _location: &str,
    ) -> Result<(), OxDataError> {
        Err(OxDataError::InternalError("Not implemented".to_string()))
    }

    fn restore(
        &self,
        _location: &str,
        _id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, OxDataError> {
        Err(OxDataError::InternalError("Not implemented".to_string()))
    }

    fn fetch(&self, _filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, _location: &str) -> Result<Vec<String>, OxDataError> {
        Err(OxDataError::InternalError("Not implemented".to_string()))
    }

    fn notify_lock_status_change(&self, _lock_status: &str, _gdo_id: &str) {
        // Not implemented
    }

    fn prepare_datastore(&self, _connection_info: &HashMap<String, String>) -> Result<(), OxDataError> {
        Err(OxDataError::InternalError("Not implemented".to_string()))
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, OxDataError> {
        Err(OxDataError::InternalError("Not implemented".to_string()))
    }
    fn describe_dataset(&self, _connection_info: &HashMap<String, String>, _dataset_name: &str) -> Result<DataSet, OxDataError> {
        Err(OxDataError::InternalError("Not implemented".to_string()))
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![]
    }
}

#[no_mangle]
pub extern "C" fn create_sql_driver() -> *mut SqlPersistenceDriver {
    let driver = Box::new(SqlPersistenceDriver);
    Box::into_raw(driver)
}

#[no_mangle]
pub extern "C" fn destroy_sql_driver(ptr: *mut SqlPersistenceDriver) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = Box::from_raw(ptr);
    }
}
