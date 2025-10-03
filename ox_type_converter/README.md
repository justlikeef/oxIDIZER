# ox_type_converter

A modular and extensible type conversion system for Rust.

## Features

- **Modular Design**: Individual conversion functions organized into separate files
- **Automatic Inclusion**: All conversion modules are automatically included
- **Easy Extension**: Add new conversion routines by simply adding new files
- **Type Safety**: Full type safety with compile-time checks
- **Conversion Registry**: Central registry for managing and finding conversion functions

## Project Structure

```
ox_type_converter/
├── Cargo.toml
├── README.md
└── src/
    ├── lib.rs              # Main library entry point
    ├── value_type.rs       # ValueType definition
    ├── converters/         # Conversion modules
    │   ├── mod.rs         # Automatically includes all conversion files
    │   ├── string_conversions.rs
    │   ├── numeric_conversions.rs
    │   ├── boolean_conversions.rs
    │   └── generic_conversions.rs
    └── registry.rs         # Conversion registry system
```

## Usage

### Basic Usage

```rust
use ox_type_converter::{ValueType, TypeConverter, ConversionRegistry};

// Use the main TypeConverter for basic operations
let value_type = TypeConverter::infer_value_type(&42);
assert_eq!(value_type.as_str(), "integer");

// Use the conversion registry for advanced operations
let registry = ConversionRegistry::new(true);
let result = registry.convert_with_specific_converter("string", "integer", "123", &HashMap::new());
```

### Adding New Conversion Functions

To add new conversion functions, simply create a new file in the `src/converters/` directory and add it to the `mod.rs` file:

1. Create a new file, e.g., `src/converters/custom_conversions.rs`:
```rust
use crate::HashMap;

pub fn custom_to_string(value: &str, _parameters: &HashMap<String, String>) -> Result<String, String> {
    Ok(format!("custom_{}", value))
}
```

2. Add it to `src/converters/mod.rs`:
```rust
pub mod custom_conversions;
pub use custom_conversions::*;
```

3. Register it in the registry (optional):
```rust
// In registry.rs, add to register_builtin_conversions()
self.register_conversion("custom", "string", |v, p| {
    custom_to_string(v, p).map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
});
```

## Available Conversions

### String Conversions
- `string_to_integer` - Convert string to i64
- `string_to_float` - Convert string to f64
- `string_to_boolean` - Convert string to bool
- `string_to_string` - Identity conversion
- `string_to_uinteger` - Convert string to u64
- `string_to_i32` - Convert string to i32
- `string_to_i64` - Convert string to i64
- `string_to_f32` - Convert string to f32
- `string_to_f64` - Convert string to f64

### Numeric Conversions
- `integer_to_string` - Convert i64 to string
- `float_to_string` - Convert f64 to string
- `float_to_integer` - Convert f64 to i64 (truncates)
- `integer_to_float` - Convert i64 to f64
- And many more specific numeric type conversions...

### Boolean Conversions
- `boolean_to_string` - Convert bool to string
- `boolean_to_integer` - Convert bool to i64
- `integer_to_boolean` - Convert i64 to bool
- And more boolean conversions...

### Generic Conversions
- `convert_value<T>` - Generic conversion function for any type implementing FromStr

## Testing

Run the test suite:

```bash
cargo test
```

## Contributing

1. Fork the repository
2. Create a feature branch
3. Add your conversion functions to the appropriate module
4. Add tests for your conversions
5. Submit a pull request

## License

This project is licensed under the MIT License.
