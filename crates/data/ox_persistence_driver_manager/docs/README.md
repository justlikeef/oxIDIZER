# ox_persistence_driver_manager

REST plugin for managing the lifecycle of persistence driver libraries. Load, unload, and
list drivers at runtime without restarting the server.

---

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/drivers` | List all registered drivers with metadata |
| `POST` | `/drivers/reload` | Re-read `conf/drivers.yaml` and reload enabled drivers |
| `POST` | `/drivers/{name}/unload` | Unregister a specific driver |

---

## conf/drivers.yaml

```yaml
drivers:
  - id: "pg-main"
    name: "ox_persistence_driver_db_postgres"
    library_path: "/opt/ox/drivers"   # optional; defaults to conf/drivers/
    state: "enabled"
  - id: "sqlite-local"
    name: "ox_persistence_driver_db_sqlite"
    state: "enabled"
  - id: "mysql-reports"
    name: "ox_persistence_driver_db_mysql"
    state: "disabled"
```

---

## Reload Processing

`POST /drivers/reload`:

1. Reads `conf/drivers.yaml` via `ox_fileproc::process_file` (`!include` supported).
2. For each entry where `state == "enabled"`:
   - Constructs platform-specific filename (`libNAME.so` / `.dylib` / `.dll`).
   - Loads via `libloading::Library`.
   - Calls `ox_driver_get_driver_metadata()` to get the driver's name.
   - Registers in `PERSISTENCE_DRIVER_REGISTRY`.
3. Returns `{"loaded": N, "errors": [...]}`.

Response codes: `200` (all succeeded) or `207` (partial success with errors listed).

---

## Implementation Notes

- Drivers already in the registry are replaced on reload if their name matches.
- Unloading a driver (`POST /drivers/{name}/unload`) removes it from
  `PERSISTENCE_DRIVER_REGISTRY`. Any GDOs with `PersistenceInfo` pointing to this driver
  will fail on subsequent persist/hydrate calls until the driver is reloaded.
- `ox_data_broker` maintains its own `DriverManager` separate from
  `PERSISTENCE_DRIVER_REGISTRY`. Reloading via this plugin does not affect the broker's
  manager; `POST /drivers/reload` on the broker endpoint must be called separately.
