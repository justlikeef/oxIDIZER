# ox_data_error

Shared error types for the ox_data layer. All data layer crates use `OxDataError` to
propagate failures uniformly.

---

## OxDataError

```rust
pub enum OxDataError {
    NotFound(String),
    DriverError(String),
    SerializationError(String),
    DeserializationError(String),
    TypeConversionError(String),
    ValidationError(String),
    LockError(String),
    TransactionError(String),
    IoError(String),
    ConfigError(String),
    NotSupported(String),
    Internal(String),
}
```

| Variant | Typical cause |
|---|---|
| `NotFound` | Record not found in backing store |
| `DriverError` | Driver returned a non-zero exit code or database error |
| `SerializationError` | Failed to serialize GDO to wire format |
| `DeserializationError` | Failed to parse driver response |
| `TypeConversionError` | `ConversionRegistry` could not convert a value |
| `ValidationError` | Validation rules failed before a save |
| `LockError` | Lock acquisition failed (record held by another holder) |
| `TransactionError` | Transaction rollback or commit failure |
| `IoError` | File system error (file drivers) |
| `ConfigError` | Driver initialization config missing or invalid |
| `NotSupported` | `call_action` received an unsupported action name |
| `Internal` | Unexpected internal error |

---

## Usage

All data layer crates declare:
```toml
[dependencies]
ox_data_error = { path = "../../ox_data_error" }
```

Then use `OxDataError` as the `Err` variant in all data layer `Result` types.

---

## Implementation Notes

- All variants wrap a `String` message for human-readable context.
- `OxDataError` implements `std::error::Error`, `Display`, and `From<std::io::Error>`.
- Driver FFI code maps C-level error codes to the appropriate `OxDataError` variant
  before returning to Rust callers.
