# ox_persistence_driver_installer

REST plugin for installing persistence drivers as Rust crate packages. Integrates with
`ox_package_manager` to download, build, and register driver libraries.

---

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/drivers/available` | List installable driver packages |
| `POST` | `/drivers/install` | Install a driver package |
| `DELETE` | `/drivers/{name}/uninstall` | Uninstall and unregister a driver |

---

## Install Request

```json
{ "package": "ox_persistence_driver_db_postgres", "version": "1.0.0" }
```

---

## Install Flow

1. Call `ox_package_manager` to resolve and download the crate.
2. Build the `cdylib` target.
3. Copy the compiled library to the configured driver root (`conf/drivers/` by default).
4. Append an entry to `conf/drivers.yaml` with `state: "enabled"`.
5. Trigger a `POST /drivers/reload` to activate the driver immediately.

---

## Uninstall Flow

1. Call `POST /drivers/{name}/unload` via `ox_persistence_driver_manager` to unregister.
2. Remove the library file from the driver root.
3. Remove the entry from `conf/drivers.yaml`.

---

## Implementation Notes

- This plugin is optional. If you build drivers manually and manage `conf/drivers.yaml`
  yourself, you do not need this plugin in your pipeline.
- Build artifacts are placed in a temporary directory and moved to the driver root only
  after successful compilation.
- Install and uninstall operations are synchronous — the HTTP response is returned after
  the operation completes (or fails).
