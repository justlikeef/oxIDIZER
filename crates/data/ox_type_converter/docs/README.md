# ox_type_converter

Type system foundation for the data layer. Provides `ValueType` (the type tag on every
attribute), `TypeConverter` (utilities for inference and coercion), and
`ConversionRegistry` (the global converter map).

---

## ValueType

An opaque string wrapper identifying a value's logical type:

```rust
pub struct ValueType(String);
```

Well-known identifiers:

| Identifier | Rust type |
|---|---|
| `"string"` | `String` |
| `"integer"` | `i64` |
| `"float"` | `f64` |
| `"boolean"` | `bool` |
| `"uuid"` | `String` (UUID formatted) |
| `"date"` | `String` (ISO 8601 date) |
| `"datetime"` | `String` (ISO 8601 datetime) |
| `"decimal"` | `String` (with `precision` and `scale` parameters) |
| `"bytes"` | `Vec<u8>` |

Predefined constants: `ValueType::String`, `ValueType::Integer`, `ValueType::Float`,
`ValueType::Boolean`, `ValueType::Uuid`, `ValueType::Date`, `ValueType::DateTime`,
`ValueType::Decimal`, `ValueType::Bytes`.

Custom types are any string; the registry determines what conversions are available for them.

---

## TypeConverter

```rust
pub struct TypeConverter;
```

| Method | Description |
|---|---|
| `infer_value_type<T>(value: &T) -> ValueType` | Returns the `ValueType` for a Rust value using `TypeId` matching |
| `coerce_string(value: &str, target_type: &ValueType) -> String` | Converts a string to canonical form for the given type |

Supported inference types: `String`, `&str`, `i32`, `i64`, `u32`, `u64`, `f32`, `f64`,
`bool`, `Uuid`. Unknown types default to `ValueType::String`.

---

## ConversionRegistry

```rust
lazy_static! {
    pub static ref CONVERSION_REGISTRY: Mutex<ConversionRegistry> = ...;
}

pub type ConverterFn = fn(&str, &HashMap<String, String>) -> Result<Box<dyn Any + Send + Sync>, String>;
```

| Method | Description |
|---|---|
| `register(from, to, fn)` | Register a converter from one type to another |
| `convert(from, to, value, params)` | Convert a string value; identity if types match |
| `convert_with_specific_converter(from, to, value, params)` | Error if no converter found |

### Built-in Converters

| From → To | Behavior |
|---|---|
| `string → integer` | `parse::<i64>()` |
| `string → float` | `parse::<f64>()` |
| `string → boolean` | `"true"/"1"/"yes"` → true |
| `integer → string` | `to_string()` |
| `integer → float` | lossless widening |
| `integer → boolean` | `0` → false, non-zero → true |
| `float → integer` | truncation |
| `float → string` | `to_string()` |
| `boolean → string` | `"true"` / `"false"` |
| `boolean → integer` | `1` / `0` |
| `decimal → string` | preserves precision/scale from parameters |

### Adding a Custom Converter

```rust
CONVERSION_REGISTRY.lock().unwrap().register(
    "my_type",
    "string",
    |value, _params| Ok(Box::new(format!("prefix_{}", value))),
);
```

---

## Usage in GenericDataObject

`GenericDataObject::get<T>(name)` calls the `ConversionRegistry` when the stored type
and the requested type differ. `get_raw_value<T>(name)` skips conversion entirely.
