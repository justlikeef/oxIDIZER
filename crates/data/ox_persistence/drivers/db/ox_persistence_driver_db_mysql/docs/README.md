# ox_persistence_driver_db_mysql

MySQL / MariaDB persistence driver. Implements the driver FFI ABI for storing and
querying `GenericDataObject` data in MySQL or MariaDB.

---

## Driver Name

`ox_persistence_driver_db_mysql`

---

## Connection Config Parameters

| Parameter | Required | Description |
|---|---|---|
| `host` | yes | MySQL hostname |
| `port` | no (default: 3306) | Port number |
| `database` | yes | Database/schema name |
| `username` | yes | Login user |
| `password` | no | Login password |
| `pool_size` | no (default: 5) | Connection pool size |
| `connect_timeout_secs` | no (default: 30) | Connection timeout |
| `ssl_mode` | no | `disabled` / `preferred` / `required` |

---

## Storage Behavior

- Uses `INSERT INTO ... ON DUPLICATE KEY UPDATE ...` for idempotent upsert.
- `location` maps to a table name within the configured database.
- Dynamic column addition via `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`.
- Placeholder style: `?` (positional, MySQL style).

---

## `list_datasets` and `describe_dataset`

- `list_datasets`: queries `INFORMATION_SCHEMA.TABLES` for the current database.
- `describe_dataset(name)`: queries `INFORMATION_SCHEMA.COLUMNS` for the named table.

---

## `notify_lock_status_change`

Uses MySQL `GET_LOCK(lock_name, timeout)` and `RELEASE_LOCK(lock_name)` for advisory
locking on GDO IDs.

---

## Dependencies

| Crate | Purpose |
|---|---|
| `ox_persistence_driver_db_sql` | Shared SQL generation |
| `sqlx` (mysql feature) | Async MySQL client |
| `ox_persistence` | `OxBuffer`, FFI ABI |
