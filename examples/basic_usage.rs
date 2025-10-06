use ox_data_object::{GenericDataObject, EventType, ValueType, CallbackResult, CallbackError, CallbackAction};
use std::any::Any;
use std::collections::HashMap;

fn main() {
    println!("=== Generic Data Object Basic Usage Example ===\n");

    // Create a new generic data object
    let mut data_object = GenericDataObject::new("id", None);

    // Register a callback for BeforeGet events
    data_object.register_callback(EventType::new("BeforeGet"), Box::new(|obj: &mut dyn Any, params: &[&dyn Any]| {
        if let Some(_gdo) = obj.downcast_mut::<GenericDataObject>() {
            println!("BeforeGet callback triggered!");
            if let Some(identifier) = params.get(0).and_then(|p| p.downcast_ref::<String>()) {
                println!("  Getting attribute: {}", identifier);
            }
        }
        Ok(None) // Indicate success with no message
    }));

    // Register a callback for AfterSet events
    data_object.register_callback(EventType::new("AfterSet"), Box::new(|obj: &mut dyn Any, params: &[&dyn Any]| {
        if let Some(_gdo) = obj.downcast_mut::<GenericDataObject>() {
            println!("AfterSet callback triggered!");
            if let Some(identifier) = params.get(0).and_then(|p| p.downcast_ref::<String>()) {
                println!("  Set attribute: {}", identifier);
            }
        }
        Ok(None)
    }));

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
    // .unwrap() is for the Result, the second .unwrap() is for the Option
    let name: String = data_object.get("name").unwrap().unwrap();
    let age: i32 = data_object.get("age").unwrap().unwrap();
    let height: f64 = data_object.get("height").unwrap().unwrap();
    let is_active: bool = data_object.get("is_active").unwrap().unwrap();
    let price: f64 = data_object.get("price").unwrap().unwrap();

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
    data_object.register_callback(EventType::new("BeforeSet"), Box::new(|_obj, params| {
        if let Some(identifier) = params.get(0).and_then(|p| p.downcast_ref::<String>()) {
            if identifier == "validated_field" {
                println!("Validation callback triggered for 'validated_field'!");
                return Ok(Some("Validated!".to_string()));
            }
        }
        Ok(None)
    }));

    // Trigger custom event by setting the specific field
    let messages = data_object.set("validated_field", "validated_value").unwrap();
    println!("Callback messages: {:?}", messages);

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
    let custom_value: MyCustomType = custom_data_object.get("custom").unwrap().unwrap();
    println!("Custom value: {:?}", custom_value);

    println!("\n=== Rollback Example ===");
    let mut rollback_gdo = GenericDataObject::new("id", None);
    rollback_gdo.set("status", "initial").unwrap();
    println!("Initial status: {}", rollback_gdo.get("status").unwrap().unwrap() as String);

    // Register a callback that will trigger a rollback
    rollback_gdo.register_callback(EventType::new("AfterSet"), Box::new(|_obj, params| {
        if let Some(identifier) = params.get(0).and_then(|p| p.downcast_ref::<String>()) {
            if identifier == "status" {
                if let Some(value) = params.get(1).and_then(|p| p.downcast_ref::<&str>()) {
                    if *value == "invalid" {
                        println!("Callback: Detected invalid status, requesting rollback.");
                        return Err(CallbackError {
                            message: "Status cannot be 'invalid'".to_string(),
                            action: CallbackAction::Rollback,
                        });
                    }
                }
            }
        }
        Ok(None)
    }));

    println!("\nAttempting to set status to 'invalid'...");
    let result = rollback_gdo.set("status", "invalid");
    assert!(result.is_err());
    println!("Set operation failed as expected: {}", result.err().unwrap().message);

    let final_status: String = rollback_gdo.get("status").unwrap().unwrap();
    println!("Final status: {}", final_status);
    assert_eq!(final_status, "initial");
    println!("Status was successfully rolled back to 'initial'.");
}