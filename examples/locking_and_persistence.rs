use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ox_data_object::{GenericDataObject};
use ox_type_converter::ValueType;
use ox_locking::{Lockable, LockableGenericDataObject, LockStatus};
use ox_persistence::{PERSISTENCE_DRIVER_REGISTRY, Persistent, PersistenceDriver, ConnectionParameter, DataSet}; // Added required imports
use uuid::Uuid;

// Dummy MySQL driver for demonstration purposes
struct MySqlDriver;

impl PersistenceDriver for MySqlDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        println!("\n--- Persisting to MySQL ---");
        println!("Connection string: {}", location);
        println!("Data:");
        for (key, (value, value_type, _)) in serializable_map {
            println!("  - {}: {} ({})", key, value, value_type.as_str());
        }
        println!("--- Persistence complete ---\n");
        Ok(())
    }

    fn restore(
        &self,
        _location: &str,
        _id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        // Not implemented for this example
        unimplemented!()
    }

    fn fetch(&self, _filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, _location: &str) -> Result<Vec<String>, String> {
        unimplemented!()
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("MySQL driver notified of lock status change to {} for object {}", lock_status, gdo_id);
    }

    fn prepare_datastore(&self, _connection_info: &HashMap<String, String>) -> Result<(), String> {
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    
    fn describe_dataset(&self, _connection_info: &HashMap<String, String>, _dataset_name: &str) -> Result<DataSet, String> {
        Err("Not implemented".to_string())
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![]
    }
}

fn main() {
    // Register the MySQL driver
    {
        // Fixing duplicate re-export issue if any, assuming ox_persistence doesn't re-export GenericDataObject names overlapping
        // We need DrivrerMetadata
        use ox_persistence::DriverMetadata;
        let metadata = DriverMetadata {
            name: "mysql".to_string(),
            version: "0.1".to_string(),
            description: "Mock MySQL Driver".to_string(),
            compatible_modules: HashMap::new(),
        };
        ox_persistence::register_persistence_driver(Arc::new(MySqlDriver), metadata);
    }

    // Create a new LockableGenericDataObject
    let gdo = GenericDataObject::new("id", None);
    let mut locked_gdo = LockableGenericDataObject::new(gdo);

    println!("Setting initial values...");

    // Set some values.
    // Lockable logic has changed; using explicit lock status management validation + inner GDO set
    locked_gdo.get_gdo_mut().set("name", "John Doe".to_string());
    
    // Simulate locking
    locked_gdo.set_lock_status(LockStatus::Locked(Uuid::new_v4()));
    println!("Object lock status: {:?}", locked_gdo.get_lock_status());

    locked_gdo.get_gdo_mut().set("age", 30);
    locked_gdo.get_gdo_mut().set("city", "New York".to_string());


    // Persist the changes to the MySQL database
    let mysql_connection_string = "mysql://user:password@localhost/my_database";
    // Using inner GDO to persist as Lockable wrapper doesn't implement Persistent
    match locked_gdo.get_gdo_mut().persist("mysql", mysql_connection_string) {
        Ok(_) => println!("Successfully persisted data to MySQL."),
        Err(e) => println!("Error persisting data: {}", e),
    }

    println!("Object lock status after persist: {:?}", locked_gdo.get_lock_status());
}
