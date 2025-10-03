use ox_data_object::{GenericDataObject, PersistentGenericDataObject};
use json_persistent_generic_data_object::JsonPersistentGenericDataObject;

fn main() {
    println!("=== Persistence Example ===");

    // Create a new generic data object
    let mut data_object = GenericDataObject::new();
    data_object.set("name", "John Doe").unwrap();
    data_object.set("age", 30i64).unwrap();
    data_object.set("height", 175.5f64).unwrap();
    data_object.set("is_active", true).unwrap();

    // Create a JSON persistence driver
    let persister = JsonPersistentGenericDataObject;

    // Persist the data object to a JSON file
    let file_path = "data_object.json";
    println!("Persisting data object to {}", file_path);
    persister.persist(&data_object, file_path).unwrap();

    // Restore the data object from the JSON file
    println!("Restoring data object from {}", file_path);
    let restored_data_object = persister.restore(file_path).unwrap();

    // Verify that the restored object is the same as the original
    println!("Verification complete!");
}
