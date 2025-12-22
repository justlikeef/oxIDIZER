use std::sync::Arc;
use ox_data_object::GenericDataObject;
use ox_locking::LockableGenericDataObject;
use ox_persistence::{Persistent};
use ox_persistence_driver_db_mysql;

fn main() {
    // Register the MySQL driver
    ox_persistence_driver_db_mysql::MysqlPersistenceDriver::register();

    // Create a filter object
    let mut filter_gdo = GenericDataObject::new("id", None);
    filter_gdo.set("city", "New York".to_string()).unwrap();
    
    // We can use the GDO directly for fetching since it implements Persistent
    println!("Fetching objects with city: New York...");

    // Fetch the data
    let mysql_connection_string = "mysql://user:password@localhost/my_database";
    match filter_gdo.fetch("mysql", mysql_connection_string) {
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
