use std::collections::HashMap;
use std::sync::{Arc};
use ox_data_object::generic_data_object::GenericDataObject;
use ox_locking::{Lockable, LockableGenericDataObject, LockStatus};
use ox_persistence::{PERSISTENCE_DRIVER_REGISTRY, Persistent, PersistenceDriver};
use ox_type_converter::ValueType;

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
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        // Not implemented for this example
        unimplemented!()
    }

    fn notify_lock_status_change(&self, lock_status: LockStatus, gdo_id: usize) {
        println!("MySQL driver notified of lock status change to {:?} for object {}", gdo_id, lock_status);
    }
}

fn main() {
    // Register the MySQL driver
    {
        let mut registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        registry.register_driver("mysql", Arc::new(MySqlDriver));
    }

    // Create a new LockableGenericDataObject
    let gdo = GenericDataObject::new();
    let mut locked_gdo = LockableGenericDataObject::new(gdo);

    println!("Setting initial values...");

    // Set some values. The first set will create a lock.
    let lock = locked_gdo.set("name", "John Doe".to_string(), None).unwrap().unwrap();
    locked_gdo.set("age", 30, Some(lock.clone())).unwrap();
    locked_gdo.set("city", "New York".to_string(), Some(lock.clone())).unwrap();

    println!("Object lock status: {:?}", locked_gdo.get_lock_status());

    // Persist the changes to the MySQL database
    let mysql_connection_string = "mysql://user:password@localhost/my_database";
    match locked_gdo.persist("mysql", mysql_connection_string) {
        Ok(_) => println!("Successfully persisted data to MySQL."),
        Err(e) => println!("Error persisting data: {}", e),
    }

    println!("Object lock status after persist: {:?}", locked_gdo.get_lock_status());
}
