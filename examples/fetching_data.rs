use std::sync::Arc;
use ox_data_object::GenericDataObject;
use ox_locking::LockableGenericDataObject;
use ox_persistence::{PERSISTENCE_DRIVER_REGISTRY, Persistent};
use ox_persistence_mysql::MySqlDriver;

fn main() {
    // Register the MySQL driver
    {
        let mut registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        registry.register_driver("mysql", Arc::new(MySqlDriver));
    }

    // Create a filter object
    let mut filter_gdo = GenericDataObject::new();
    filter_gdo.set("city", "New York".to_string()).unwrap();
    let filter_object = LockableGenericDataObject::new(filter_gdo);

    println!("Fetching objects with city: New York...");

    // Fetch the data
    let mysql_connection_string = "mysql://user:password@localhost/my_database";
    match filter_object.fetch("mysql", mysql_connection_string) {
        Ok(fetched_objects) => {
            println!("Successfully fetched {} objects:", fetched_objects.len());
            for (i, mut obj) in fetched_objects.into_iter().enumerate() {
                println!("  - Object {}", i + 1);
                let name: String = obj.get("name").unwrap_or_default();
                let age: i32 = obj.get("age").unwrap_or_default();
                let city: String = obj.get("city").unwrap_or_default();
                println!("    - Name: {}, Age: {}, City: {}", name, age, city);
            }
        }
        Err(e) => println!("Error fetching data: {}", e),
    }
}
