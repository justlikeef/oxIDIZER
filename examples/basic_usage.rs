use ox_data_object::{GenericDataObject, EventType, ValueType, CALLBACK_MANAGER};
use std::any::Any;
use std::collections::HashMap;

fn main() {
    println!("=== Generic Data Object Basic Usage Example ===\n");

    // Create a new generic data object
    let mut data_object = GenericDataObject::new("id", None);

    // Register a callback for BeforeGet events
    CALLBACK_MANAGER.lock().unwrap().register_callback(EventType::new("BeforeGet"), |obj, params| {
        if let Some(gdo) = obj.downcast_ref::<GenericDataObject>() {
            println!("BeforeGet callback triggered!");
            if let Some(identifier) = params.get(0).and_then(|p| p.downcast_ref::<String>()) {
                println!("  Getting attribute: {}", identifier);
            }
        }
    });

    // Register a callback for AfterSet events
    CALLBACK_MANAGER.lock().unwrap().register_callback(EventType::new("AfterSet"), |obj, params| {
        if let Some(gdo) = obj.downcast_ref::<GenericDataObject>() {
            println!("AfterSet callback triggered!");
            if let Some(identifier) = params.get(0).and_then(|p| p.downcast_ref::<String>()) {
                println!("  Set attribute: {}", identifier);
            }
        }
    });

    // Set various types of values
    println!("Setting attributes...");
    data_object.set("name", "John Doe").unwrap();
    data_object.set("age", 30).unwrap();
    data_object.set("height", 175.5).unwrap();
    data_object.set("is_active", true).unwrap();

    // Set a value with explicit type and parameters
    let mut parameters = HashMap::new();
    parameters.insert("decimal_places".to_string(), "2".to_string());
    data_object.set_with_type("price", 19.99, ValueType::new("float"), Some(parameters)).unwrap();

    println!("\nGetting attributes...");
    
    // Get values back with type conversion
    let name: String = data_object.get("name").unwrap();
    let age: i32 = data_object.get("age").unwrap();
    let height: f64 = data_object.get("height").unwrap();
    let is_active: bool = data_object.get("is_active").unwrap();
    let price: f64 = data_object.get("price").unwrap();

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

    // Demonstrate custom event
    println!("\n=== Custom Event Example ===");
    CALLBACK_MANAGER.lock().unwrap().register_callback(EventType::new("validation"), |_obj, _params| {
        println!("Validation callback triggered!");
    });

    // Trigger custom event (this would be done internally in a real application)
    // For demonstration, we'll simulate it by calling set again
    data_object.set("validated_field", "validated_value").unwrap();

    println!("\n=== Custom Type Conversion Example ===");

    // Define a custom type
    #[derive(Debug, Clone, Default)]
    struct MyCustomType {
        value: String,
    }

    // Register a custom converter with the global registry
    ox_data_object::CONVERSION_REGISTRY.lock().unwrap().register_conversion("string", "mycustomtype", |v, _p| {
        Ok(Box::new(MyCustomType { value: v.to_string() }) as Box<dyn std::any::Any + Send + Sync>)
    });

    // Create a new data object
    let mut custom_data_object = GenericDataObject::new("id", None);

    // Set a string value
    custom_data_object.set("custom", "hello").unwrap();

    // Get the value as MyCustomType
    let custom_value: MyCustomType = custom_data_object.get("custom").unwrap();
    println!("Custom value: {:?}", custom_value);
} 