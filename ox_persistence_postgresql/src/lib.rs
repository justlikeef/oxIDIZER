use ox_data_object::{
    generic_data_object::GenericDataObject,
    AttributeValue,
};
use ox_persistence::{PersistenceDriver, register_persistence_driver, DriverMetadata, DataSet, ConnectionParameter};
use ox_locking::LockStatus;
use ox_type_converter::ValueType;
use std::collections::HashMap;
use std::sync::Arc;
use ox_persistence_sql::{SqlPersistenceDriver, GenericSqlDriver};

pub struct PostgresqlDriver;

impl PersistenceDriver for PostgresqlDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        // Delegate to GenericSqlDriver
        GenericSqlDriver.persist(serializable_map, location)
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        // Delegate to GenericSqlDriver
        GenericSqlDriver.restore(location, id)
    }

    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
        // Delegate to GenericSqlDriver
        GenericSqlDriver.fetch(filter, location)
    }

    fn restore_one(&self, location: &str, id: &str) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        // Delegate to GenericSqlDriver
        GenericSqlDriver.restore_one(location, id)
    }

    fn notify_lock_status_change(&self, lock_status: LockStatus, gdo_id: usize) {
        println!("PostgresqlDriver: GDO {} lock status changed to {:?}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("\n--- Preparing PostgreSQL Datastore ---");
        println!("Connection Info: {:?}", connection_info);
        println!("--- PostgreSQL Datastore Prepared ---\n");
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        // TODO: Implement by querying pg_catalog.pg_tables
        Err("Not implemented for PostgreSQL driver yet.".to_string())
    }

    fn describe_dataset(&self, _connection_info: &HashMap<String, String>, _dataset_name: &str) -> Result<DataSet, String> {
        // TODO: Implement by querying information_schema.columns
        Err("Not implemented for PostgreSQL driver yet.".to_string())
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "host".to_string(),
                description: "The PostgreSQL server host address.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: Some("localhost".to_string()),
            },
            ConnectionParameter {
                name: "port".to_string(),
                description: "The PostgreSQL server port.".to_string(),
                data_type: "integer".to_string(),
                is_required: false,
                default_value: Some("5432".to_string()),
            },
            ConnectionParameter {
                name: "database".to_string(),
                description: "The name of the PostgreSQL database.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "username".to_string(),
                description: "The username for PostgreSQL database access.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "password".to_string(),
                description: "The password for PostgreSQL database access.".to_string(),
                data_type: "string".to_string(),
                is_required: false,
                default_value: None,
            },
            ConnectionParameter {
                name: "service_name".to_string(),
                description: "The PostgreSQL service name (alternative to host/port/db). If provided, other connection details might be ignored.".to_string(),
                data_type: "string".to_string(),
                is_required: false,
                default_value: None,
            },
        ]
    }
}

impl SqlPersistenceDriver for PostgresqlDriver {
    fn execute_query(&self, query: &str, params: &HashMap<String, String>) -> Result<Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>, String> {
        // This would be the actual PostgreSQL specific query execution
        println!("Executing PostgreSQL query: {}", query);
        println!("With params: {:?}", params);
        // Dummy implementation
        Ok(vec![])
    }

    fn build_where_clause(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>) -> (String, HashMap<String, String>) {
        GenericSqlDriver.build_where_clause(filter)
    }
}

pub fn init() {
    let metadata = DriverMetadata {
        name: "postgresql".to_string(),
        description: "A driver for PostgreSQL databases.".to_string(),
        version: "0.1.0".to_string(),
    };
    register_persistence_driver(Arc::new(PostgresqlDriver), metadata);
}
