# ox_persistence_driver_db_sql

SQL base library shared by all relational database drivers. Not a standalone driver —
it provides common SQL generation, result mapping, and connection pool management used
by the postgres, mysql, mssql, and sqlite drivers.

---

## What It Provides

### SQL Generation

Generates driver-agnostic SQL statements from the serializable map format:

- `INSERT OR REPLACE INTO {location} (field, ...) VALUES (?, ...)` (upsert pattern)
- `SELECT * FROM {location} WHERE id = ?`
- `SELECT id FROM {location} WHERE field1 = ? AND field2 = ?` (filter → fetch)
- `DELETE FROM {location} WHERE id = ?`

Placeholder style (`?` vs `$1`) is determined by a `PlaceholderStyle` parameter injected
by each concrete driver.

### Result Mapping

Maps SQL `Row` results back to the serializable map format. Each column is mapped to
`(string_value, ValueType, parameters)` using the column's SQL type for `ValueType`
inference.

### Connection Pool Management

Wraps a connection pool (via `sqlx` or `r2d2`) with:
- Pool size configuration
- Connection timeout handling
- Error wrapping into `OxDataError`

### `call_action("query", params)` Implementation

Provides a `raw_sql` action accepted by all DB drivers:

```json
{ "sql": "SELECT ...", "params": ["value1", "value2"] }
```

Returns a JSON array of row objects. Useful for complex queries beyond the equality-filter
semantics of `fetch`.

---

## Usage

Concrete drivers link against this library and delegate common operations:

```rust
// In ox_persistence_driver_db_postgres
use ox_persistence_driver_db_sql::{SqlDriver, PlaceholderStyle};

let base = SqlDriver::new(connection_pool, PlaceholderStyle::Dollar);
base.persist(&map, "users")
```

---

## Dependencies

| Crate | Purpose |
|---|---|
| `sqlx` or `r2d2` | Connection pooling (driver-specific feature) |
| `ox_persistence` | `OxBuffer`, `OxDataError`, `PersistenceDriver` trait |
| `ox_type_converter` | `ValueType` inference from SQL column types |
