use ox_data_object::{
    generic_data_object::GenericDataObject,
    AttributeValue,
};
use ox_persistence::{PersistenceDriver, register_persistence_driver, DriverMetadata};
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
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        // Delegate to GenericSqlDriver
        GenericSqlDriver.restore(location)
    }

    fn fetch(
        &self,
        filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>, String> {
        // Delegate to GenericSqlDriver
        GenericSqlDriver.fetch(filter, location)
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
