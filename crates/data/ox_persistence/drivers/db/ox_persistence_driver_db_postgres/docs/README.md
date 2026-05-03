# ox_persistence_driver_db_postgres

PostgreSQL persistence driver. Implements the driver FFI ABI for storing and querying
`GenericDataObject` data in PostgreSQL.

---

## Driver Name

`ox_persistence_driver_db_postgres`

---

## Connection Config Parameters

| Parameter | Required | Description |
|---|---|---|
| `host` | yes | PostgreSQL hostname |
| `port` | no (default: 5432) | Port number |
| `database` | yes | Database name |
| `username` | yes | Login user |
| `password` | no | Login password |
| `pool_size` | no (default: 5) | Connection pool size |
| `connect_timeout_secs` | no (default: 30) | Connection timeout |
| `ssl_mode` | no | `disable` / `prefer` / `require` |

---

## Storage Behavior

- Uses upsert (`INSERT ... ON CONFLICT (id) DO UPDATE SET ...`) to make `persist` idempotent.
- Columns are created dynamically if the table does not have them (schema evolution via
  `ALTER TABLE ADD COLUMN IF NOT EXISTS`).
- `location` maps to a table name within the configured database.
- `call_action("query", {"sql": "...", "params": [...]})` — executes raw SQL and returns
  JSON row array.
- `call_action("raw_sql", {"sql": "..."})` — alias for the above; accepted for
  compatibility.

---

## `list_datasets` and `describe_dataset`

- `list_datasets`: queries `information_schema.tables` for all tables in the current
  database schema.
- `describe_dataset(name)`: queries `information_schema.columns` for the named table;
  maps SQL column types to `ValueType` identifiers.

---

## `notify_lock_status_change`

Uses PostgreSQL advisory locks via `pg_try_advisory_lock(hash(gdo_id))` and
`pg_advisory_unlock(hash(gdo_id))`. The lock is acquired/released on the connection
holding the current transaction context.

---

## Dependencies

| Crate | Purpose |
|---|---|
| `ox_persistence_driver_db_sql` | Shared SQL generation and result mapping |
| `sqlx` (postgres feature) | Async PostgreSQL client |
| `ox_persistence` | `OxBuffer`, FFI ABI |
