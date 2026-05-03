# ox_persistence_driver_db_mssql

Microsoft SQL Server persistence driver. Implements the driver FFI ABI for storing and
querying `GenericDataObject` data in SQL Server.

---

## Driver Name

`ox_persistence_driver_db_mssql`

---

## Connection Config Parameters

| Parameter | Required | Description |
|---|---|---|
| `host` | yes | SQL Server hostname or instance |
| `port` | no (default: 1433) | Port number |
| `database` | yes | Database name |
| `username` | yes | Login user |
| `password` | no | Login password |
| `pool_size` | no (default: 5) | Connection pool size |
| `connect_timeout_secs` | no (default: 30) | Connection timeout |
| `trust_server_certificate` | no (default: false) | Skip TLS certificate verification |
| `encrypt` | no (default: true) | Require encrypted connection |

---

## Storage Behavior

- Uses `MERGE INTO ... USING (VALUES ...) AS src ON ... WHEN MATCHED THEN UPDATE ...
  WHEN NOT MATCHED THEN INSERT ...` for idempotent upsert.
- `location` maps to a table name (optionally `[schema].[table]` format).
- Placeholder style: `@p1`, `@p2`, ... (SQL Server named parameters).
- Dynamic column addition via `IF NOT EXISTS (SELECT ...) ALTER TABLE ... ADD ...`.

---

## `list_datasets` and `describe_dataset`

- `list_datasets`: queries `INFORMATION_SCHEMA.TABLES` for the current database.
- `describe_dataset(name)`: queries `INFORMATION_SCHEMA.COLUMNS` for the named table.

---

## `notify_lock_status_change`

Uses SQL Server application locks via `sp_getapplock` and `sp_releaseapplock` for
advisory locking on GDO IDs.

---

## Dependencies

| Crate | Purpose |
|---|---|
| `ox_persistence_driver_db_sql` | Shared SQL generation |
| `tiberius` | Async SQL Server client |
| `tokio` | Async runtime for tiberius |
| `ox_persistence` | `OxBuffer`, FFI ABI |
