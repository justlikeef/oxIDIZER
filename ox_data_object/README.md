# Generic Data Object

A flexible Rust library for storing and managing generic data objects with type conversion, callback systems, and extensible attribute storage.

## Features

- **Flexible Attribute Storage**: Store any valid Rust type with automatic type inference
- **Native Type Preservation**: Values are stored in their original type, not converted to strings
- **Type Conversion**: Automatic conversion between different data types when needed
- **Callback System**: Register callbacks for various events (BeforeGet, AfterSet, etc.)
- **Type Parameters**: Support for conversion parameters (e.g., decimal places for floats)
- **Event-Driven Architecture**: Built-in event system for extensibility
- **Type Safety**: Full type safety with compile-time checks

## Project Structure

This project follows the standard Rust Cargo layout as specified in the [Rust documentation](https://doc.rust-lang.org/cargo/guide/project-layout.html):

```
.
├── Cargo.toml
├── Cargo.lock
├── src/
│   ├── lib.rs
│   ├── main.rs
│   └── generic_data_object.rs
├── examples/
│   └── basic_usage.rs
└── tests/
```

## Quick Start

### Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
ox_data_object = "0.1.0"
```

### Basic Usage

```rust
use ox_data_object::GenericDataObject;

fn main() {
    // Create a new data object
    let mut data_object = GenericDataObject::new();
    
    // Set values
    data_object.set("name", "John Doe").unwrap();
    data_object.set("age", 30).unwrap();
    data_object.set("height", 175.5).unwrap();
    
    // Get values with type conversion
    let name: String = data_object.get("name").unwrap();
    let age: i32 = data_object.get("age").unwrap();
    let height: f64 = data_object.get("height").unwrap();
    
    println!("Name: {}, Age: {}, Height: {}", name, age, height);
}
```

## Core Components

### GenericDataObject

The main structure that holds all attributes and manages the callback system.

```rust
pub struct GenericDataObject {
    attributes: HashMap<String, AttributeValue>,
    callbacks: HashMap<EventType, Vec<CallbackFn>>,
}
```

### AttributeValue

Represents a single attribute with its value, type, and conversion parameters.

```rust
pub struct AttributeValue {
    pub value: Box<dyn Any + Send + Sync>,  // Stores any valid Rust type
    pub value_type: ValueType,
    pub value_type_parameters: HashMap<String, String>,
}

// Methods available:
// - get_value<T>() -> Option<T>  // Get value as specific type
// - is<T>() -> bool              // Check if value is of specific type
// - type_id() -> TypeId          // Get the type ID
// - to_string() -> String        // Convert to string representation
```

### ValueType

String-based value type identifier:

```rust
pub struct ValueType(pub String);

// Create value types
let string_type = ValueType::new("string");
let int_type = ValueType::new("integer");
let float_type = ValueType::new("float");
let bool_type = ValueType::new("boolean");
let custom_type = ValueType::new("custom_type");
```

### EventType

String-based event type identifier:

```rust
pub struct EventType(pub String);

// Create event types
let before_get = EventType::new("BeforeGet");
let after_set = EventType::new("AfterSet");
let custom_event = EventType::new("validation");
```

## API Reference

### Creating and Managing Objects

```rust
// Create a new empty object
let mut data_object = GenericDataObject::new();

// Check if object is empty
let is_empty = data_object.is_empty();

// Get the number of attributes
let count = data_object.len();
```

### Setting Values

```rust
// Basic set with automatic type inference
data_object.set("name", "John Doe").unwrap();
data_object.set("age", 30).unwrap();
data_object.set("price", 19.99).unwrap();

// Set with explicit type and parameters
let mut parameters = HashMap::new();
parameters.insert("decimal_places".to_string(), "2".to_string());
data_object.set_with_type("price", 19.99, ValueType::new("float"), Some(parameters)).unwrap();

// Store any valid Rust type
data_object.set("custom_struct", MyCustomStruct { field: "value" }).unwrap();
data_object.set("vector", vec![1, 2, 3, 4, 5]).unwrap();
data_object.set("option", Some("optional_value")).unwrap();
```

### Getting Values

```rust
// Get values with type conversion
let name: String = data_object.get("name").unwrap();
let age: i32 = data_object.get("age").unwrap();
let price: f64 = data_object.get("price").unwrap();

// Get raw values in their original type (more efficient)
let raw_age: Option<i32> = data_object.get_raw_value("age");
let raw_custom: Option<MyCustomStruct> = data_object.get_raw_value("custom_struct");

// Check if attribute exists
if data_object.has_attribute("name") {
    // Attribute exists
}
```

### Callback System

```rust
// Register a callback for BeforeGet events
data_object.register_callback(EventType::new("BeforeGet"), |obj, params| {
    println!("BeforeGet callback triggered!");
    // Access parameters if needed
});

// Register a callback for AfterSet events
data_object.register_callback(EventType::new("AfterSet"), |obj, params| {
    println!("AfterSet callback triggered!");
});

// Register a custom event callback
data_object.register_callback(EventType::new("validation"), |obj, params| {
    println!("Validation callback triggered!");
});
```

### Attribute Management

```rust
// Get all attribute names
let names = data_object.get_attribute_names();

// Get raw attribute data
if let Some(attr) = data_object.get_attribute("price") {
    println!("Value: {}", attr.to_string());
    println!("Type: {:?}", attr.value_type);
    println!("Parameters: {:?}", attr.value_type_parameters);
    println!("Is float: {}", attr.is::<f64>());
    println!("Type ID: {:?}", attr.type_id());
}

// Get raw values in original type
let raw_value: Option<i32> = data_object.get_raw_value("age");

// Remove an attribute
let removed = data_object.remove_attribute("name");
```

## Examples

### Basic Usage Example

Run the included example:

```bash
cargo run --example basic_usage
```

This demonstrates:
- Setting and getting values of different types
- Using the callback system
- Managing attributes
- Working with type parameters

### Advanced Usage

```rust
use ox_data_object::{GenericDataObject, EventType, ValueType};
use std::collections::HashMap;

fn main() {
    let mut data_object = GenericDataObject::new();
    
    // Register validation callback
    data_object.register_callback(EventType::new("BeforeSet"), |obj, params| {
        if let Some(identifier) = params.get(0).and_then(|p| p.downcast_ref::<&str>()) {
            if identifier == &"age" {
                // Validate age is positive
                if let Some(value) = params.get(1).and_then(|p| p.downcast_ref::<i32>()) {
                    if *value < 0 {
                        panic!("Age cannot be negative!");
                    }
                }
            }
        }
    });
    
    // This will trigger the validation callback
    data_object.set("age", 25).unwrap();
    
    // This would panic due to validation
    // data_object.set("age", -5).unwrap();
}
```

## Testing

Run the test suite:

```bash
cargo test
```

The tests cover:
- Basic object creation and management
- Setting and getting values of different types
- Callback system functionality
- Error handling for non-existent attributes

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests for new functionality
5. Run the test suite
6. Submit a pull request

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## References

- [Rust Cargo Project Layout](https://doc.rust-lang.org/cargo/guide/project-layout.html)
- [Rust Documentation](https://doc.rust-lang.org/) 