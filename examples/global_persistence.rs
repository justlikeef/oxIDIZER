use ox_data_object::generic_data_object::GenericDataObject;
use ox_locking::{Lockable, LockableGenericDataObject};
use ox_persistence::{get_registered_drivers, Persistent};
use ox_persistence_mysql;
use ox_persistence_json;
use ox_persistence_sql;
use ox_persistence_flatfile;
use ox_persistence_mssql;
use ox_persistence_postgresql;
use ox_persistence_xml;
use ox_persistence_yaml;

fn main() {
    // Initialize all the drivers, which will register themselves
    ox_persistence_mysql::init();
    ox_persistence_json::init();
    ox_persistence_sql::init(); // Generic SQL driver
    ox_persistence_flatfile::init();
    ox_persistence_mssql::init();
    ox_persistence_postgresql::init();
    ox_persistence_xml::init();
    ox_persistence_yaml::init();

    // Get and print the list of registered drivers
    println!("Registered persistence drivers:");
    let drivers = get_registered_drivers();
    for driver in drivers {
        println!("  - Name: {}, Version: {}, Description: {}", driver.name, driver.version, driver.description);
    }

    // Now, use one of the drivers to persist an object
    let gdo = GenericDataObject::new("id", None);
    let mut locked_gdo = LockableGenericDataObject::new(gdo);
    locked_gdo.set("name", "John Doe".to_string()).unwrap();
    locked_gdo.set("age", 30).unwrap();

    // Set a value to get an update lock
    locked_gdo.set("city", "New York".to_string(), None).unwrap();

    println!("\nPersisting object to JSON...");
    match locked_gdo.persist("json", "my_data.json") {
        Ok(_) => println!("Successfully persisted data."),
        Err(e) => println!("Error: {}", e),
    }

    println!("\nPersisting object to SQL...");
    match locked_gdo.persist("sql", "my_sql_data") {
        Ok(_) => println!("Successfully persisted data."),
        Err(e) => println!("Error: {}", e),
    }
}
