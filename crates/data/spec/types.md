# ox_type_converter — Type System

**Crate:** `ox_type_converter`
**Type:** library

Provides a unified, extensible type system for the data module. All attribute values in
`GenericDataObject` carry a `ValueType` tag, and all cross-type conversions go through the
`ConversionRegistry`.

---

## ValueType

An opaque string wrapper that identifies a value's logical type.

```rust
pub struct ValueType(String);
```

Well-known type identifiers:

| Identifier | Rust type |
|------------|-----------|
| `"string"` | `String` |
| `"integer"` | `i64` |
| `"float"` | `f64` |
| `"boolean"` | `bool` |
| `"uuid"` | `String` (UUID formatted) |
| `"date"` | `String` (ISO 8601 date) |
| `"datetime"` | `String` (ISO 8601 datetime) |
| `"decimal"` | `String` (with `precision` and `scale` parameters) |
| `"bytes"` | `Vec<u8>` |

Custom types can be registered as any string — the registry determines what conversions
are available.

**Methods:**
- `ValueType::new(s: &str) -> Self`
- `ValueType::as_str(&self) -> &str`
- Implements `Clone`, `Debug`, `PartialEq`, `Eq`, `Serialize`, `Deserialize`

**Predefined constants:** `ValueType::String`, `ValueType::Integer`, `ValueType::Float`,
`ValueType::Boolean`, `ValueType::Uuid`, `ValueType::Date`, `ValueType::DateTime`,
`ValueType::Decimal`, `ValueType::Bytes`.

---

## TypeConverter

Utility functions for inferring and coercing types.

```rust
pub struct TypeConverter;
```

| Method | Description |
|--------|-------------|
| `infer_value_type<T>(value: &T) -> ValueType` | Returns the `ValueType` for a Rust value (uses `TypeId` matching) |
| `coerce_string(value: &str, target_type: &ValueType) -> String` | Converts a string to its canonical form for the given type (e.g., `"1"` → `"true"` for boolean, `"25.5"` → `"25"` for integer) |

`infer_value_type` supports: `String`, `&str`, `i32`, `i64`, `u32`, `u64`, `f32`, `f64`,
`bool`, `Uuid`. Unknown types default to `ValueType::String`.

---

## ConversionRegistry

A thread-safe, globally accessible registry of converter functions.

```rust
pub struct ConversionRegistry {
    converters: HashMap<(String, String), ConverterFn>,
}

pub type ConverterFn = fn(&str, &HashMap<String, String>) -> Result<Box<dyn Any + Send + Sync>, String>;

lazy_static! {
    pub static ref CONVERSION_REGISTRY: Mutex<ConversionRegistry> = ...;
}
```

**Methods:**

| Method | Description |
|--------|-------------|
| `register(from, to, fn)` | Register a converter from one type identifier to another |
| `convert(from_type, to_type, value, params) -> Result<Box<dyn Any>, String>` | Generic conversion (tries registry; falls back to identity if same type) |
| `convert_with_specific_converter(from, to, value, params)` | Same as convert; error if no converter found |

### Built-in Converters

Registered at startup via `converters/` modules:

| From → To | Notes |
|-----------|-------|
| `string → integer` | `parse::<i64>()` |
| `string → float` | `parse::<f64>()` |
| `string → boolean` | `"true"/"1"/"yes"` → true |
| `string → uuid` | passthrough (validates format) |
| `integer → string` | `to_string()` |
| `integer → float` | lossless widening |
| `integer → boolean` | `0` → false, non-zero → true |
| `float → integer` | truncation |
| `float → string` | `to_string()` |
| `boolean → string` | `"true"` / `"false"` |
| `boolean → integer` | `1` / `0` |
| `decimal → string` | preserves precision/scale from parameters |

### Parameters

Converters receive a `&HashMap<String, String>` of type-specific parameters. Standard
parameter keys:

| Key | Used by |
|-----|---------|
| `"precision"` | decimal |
| `"scale"` | decimal |

---

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `lazy_static` | `CONVERSION_REGISTRY` global |
| `serde` | `Serialize`/`Deserialize` for `ValueType` |
| `chrono` | Date/datetime parsing |
