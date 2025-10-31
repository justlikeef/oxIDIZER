use ox_persistence::{PersistenceDriver, DataSet, ConnectionParameter};
use std::collections::HashMap;
use ox_type_converter::ValueType;

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
        _id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        Err("Not implemented".to_string())
    }

    fn fetch(
        &self,
        _filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        _location: &str,
    ) -> Result<Vec<String>, String> {
        Err("Not implemented".to_string())
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("ApiDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
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

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "api_endpoint".to_string(),
                description: "The API endpoint URL for persistence operations.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "api_key".to_string(),
                description: "API key for authentication (if required).".to_string(),
                data_type: "string".to_string(),
                is_required: false,
                default_value: None,
            },
        ]
    }
}
