# ox_persistence_api

Public API surface for the persistence layer. Re-exports the most commonly used types
and functions from `ox_persistence` under a stable, versioned interface.

---

## Purpose

Provides a clean import path for downstream crates that need to interact with the
persistence layer without depending on `ox_persistence` internals directly. Acts as a
facade that can evolve the public surface independently of the implementation.

---

## Key Re-Exports

- `PersistenceDriver` trait and `PersistenceDriverRegistry`
- `DataObjectState` enum
- `PersistenceInfo` struct
- `Persistent` trait
- `OxBuffer` FFI type
- `DataSet`, `ColumnDefinition`, `DriverMetadata`, `ConnectionParameter`
- `register_persistence_driver`, `get_registered_drivers`, `unregister_persistence_driver`

---

## Usage

```toml
[dependencies]
ox_persistence_api = { path = "../ox_persistence_api" }
```

```rust
use ox_persistence_api::{PersistenceDriver, register_persistence_driver, DriverMetadata};
```

Prefer this crate over depending on `ox_persistence` directly in plugin code.
