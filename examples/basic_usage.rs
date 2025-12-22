use ox_data_object::{GenericDataObject};
use ox_type_converter::ValueType;
use std::collections::HashMap;

fn main() {
    println!("=== Generic Data Object Basic Usage Example ===\n");

    // Create a new generic data object
    let mut data_object = GenericDataObject::new("id", None);

    // Callbacks are no longer directly supported on GenericDataObject in this version
    // Skipping callback registration...

    // Set various types of values
    println!("Setting attributes...");
    // set returns Option<AttributeValue> (the old value), not Result
    data_object.set("name", "John Doe".to_string()); 
    data_object.set("age", 30);
    // data_object.set("height", 175.5); // auto-inference for float might be f64
    data_object.set("height", 175.5_f64);
    data_object.set("is_active", true);

    // Set a value with explicit type and parameters
    let mut parameters = HashMap::new();
    parameters.insert("decimal_places".to_string(), "2".to_string());
    data_object.set_with_type("price", 19.99, ValueType::new("float"), Some(parameters));

    println!("\nGetting attributes...");
    
    // Get values back with type conversion
    // get returns Option<T> directly now? No, checking lib.rs:
    // pub fn get<T: Clone + 'static + Default>(&self, identifier: &str) -> Option<T>
    // So it returns Option<T>, not Result<Option<T>>.
    
    let name: String = data_object.get("name").unwrap_or_default();
    let age: i32 = data_object.get("age").unwrap_or_default();
    let height: f64 = data_object.get("height").unwrap_or_default();
    let is_active: bool = data_object.get("is_active").unwrap_or_default();
    let price: f64 = data_object.get("price").unwrap_or_default();

    println!("Name: {}", name);
    println!("Age: {}", age);
    println!("Height: {:.1}", height);
    println!("Is Active: {}", is_active);
    println!("Price: {:.2}", price);

    // Demonstrate attribute management
    println!("\n=== Attribute Management ===");
    println!("Number of attributes: {}", data_object.len());
    println!("Attribute names: {:?}", data_object.get_attribute_names());
    println!("Has 'name' attribute: {}", data_object.has_attribute("name"));
    println!("Has 'nonexistent' attribute: {}", data_object.has_attribute("nonexistent"));

    // Show raw attribute data
    if let Some(attr) = data_object.get_attribute("price") {
        println!("\nRaw attribute data for 'price':");
        println!("  Value: {}", attr.to_string());
        println!("  Type: {:?}", attr.value_type);
        println!("  Parameters: {:?}", attr.value_type_parameters);
        println!("  Is float: {}", attr.is::<f64>());
        println!("  Type ID: {:?}", attr.type_id());
    }

    println!("\n=== Custom Type Conversion Example ===");

    // Define a custom type
    #[derive(Debug, Clone, Default)]
    struct MyCustomType {
        value: String,
    }

    // Register a custom converter with the global registry
    ox_type_converter::CONVERSION_REGISTRY.lock().unwrap().register_conversion("string", "mycustomtype", |v, _p| {
        Ok(Box::new(MyCustomType { value: v.to_string() }) as Box<dyn std::any::Any + Send + Sync>)
    });

    // Create a new data object
    let mut custom_data_object = GenericDataObject::new("id", None);

    // Set a string value
    custom_data_object.set("custom", "hello".to_string());

    // Get the value as MyCustomType
    // Note: get() uses default() to infer target type, so we might need type annotation
    let custom_value: Option<MyCustomType> = custom_data_object.get("custom");
    if let Some(val) = custom_value {
        println!("Custom value: {:?}", val);
    } else {
        println!("Failed to convert custom value");
    }

    // Rollback example removed as it depends on callbacks
}