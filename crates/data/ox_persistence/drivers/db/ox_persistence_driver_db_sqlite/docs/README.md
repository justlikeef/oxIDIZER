# ox_persistence_driver_db_sqlite

SQLite persistence driver. Implements the driver FFI ABI for storing and querying
`GenericDataObject` data in a SQLite database file. Suitable for single-node deployments
and development environments.

---

## Driver Name

`ox_persistence_driver_db_sqlite`

---

## Connection Config Parameters

| Parameter | Required | Description |
|---|---|---|
| `path` | yes | Path to SQLite database file (created if it does not exist) |
| `wal_mode` | no (default: true) | Enable WAL journal mode for better concurrency |
| `pool_size` | no (default: 1) | Connection pool size (SQLite supports limited concurrency) |

**Note:** SQLite does not support true multi-writer concurrent access. For production
multi-node deployments, use PostgreSQL or another client/server database.

---

## Storage Behavior

- Uses `INSERT OR REPLACE INTO ...` for idempotent upsert (SQLite's native upsert).
- `location` maps to a table name.
- Dynamic column addition via `ALTER TABLE ... ADD COLUMN ...` (SQLite's limited ALTER
  TABLE support; only adds nullable columns).
- Placeholder style: `?` (positional).

---

## `list_datasets` and `describe_dataset`

- `list_datasets`: queries `sqlite_master` for all user tables.
- `describe_dataset(name)`: executes `PRAGMA table_info(name)` to get column definitions.

---

## `notify_lock_status_change`

SQLite does not support advisory locks. This method is a no-op for the SQLite driver.
Record-level locking must be managed at the application level when using SQLite.

---

## `call_action` Support

- `call_action("query", {"sql": "...", "params": [...]})` — executes raw SQL.
- `call_action("discover_local", {})` — scans the filesystem for `.db` and `.sqlite`
  files in common locations and returns their paths (useful for initial setup).

---

## Dependencies

| Crate | Purpose |
|---|---|
| `ox_persistence_driver_db_sql` | Shared SQL generation |
| `sqlx` (sqlite feature) | Async SQLite client |
| `ox_persistence` | `OxBuffer`, FFI ABI |
