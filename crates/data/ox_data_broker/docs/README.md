# ox_data_broker

REST broker plugin (`cdylib`). Provides a remote HTTP API for GDO persist/restore/fetch
operations through any registered persistence driver. Stateless gateway — callers do not
need to link against `ox_persistence` directly.

---

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/drivers` | List loaded drivers and metadata |
| `POST` | `/drivers/reload` | Reload `conf/drivers.yaml` |
| `POST` | `/data/{driver_name}/persist` | Persist a GDO |
| `GET` | `/data/{driver_name}/restore` | Restore a GDO by ID |
| `POST` | `/data/{driver_name}/fetch` | Fetch GDOs matching a filter |
| `DELETE` | `/data/{driver_name}/record/{id}` | Delete a record |
| `GET` | `/data/listen` | WebSocket: subscribe to change events |

---

## Wire Format

All persist/restore/fetch operations use the `to_serializable_map()` JSON format:

```json
{
  "id":    ["550e8400-...", "uuid",   {}],
  "name":  ["Alice",       "string", {}],
  "__extensions__": ["{\"ox.persistence\":{...}}", "string", {}]
}
```

Each value is a 3-element array: `[string_value, type_string, parameters_map]`.

---

## Pagination (fetch)

```
GET /data/{driver}/fetch?limit=100&offset=0
```

Response:
```json
{ "data": [...], "total": 42, "limit": 100, "offset": 0 }
```

`total` is null for drivers that do not support counting.

---

## WebSocket Change Feed

```
GET /data/listen  (WebSocket upgrade)
```

After upgrade, send:
```json
{ "subscribe": ["object-uuid-1", "object-uuid-2"] }
```

Receive:
```json
{ "object_id": "...", "attribute": "email", "value": "new@example.com", "event": "after_set" }
```

For transaction commits: `"event": "after_commit"` (no attribute/value; re-fetch the
full object). For rollbacks: no event.

The broker registers a global `after_set` callback in `CALLBACK_MANAGER` during init.
When a transaction is active on the GDO, `after_set` events are suppressed; a single
`after_commit` is sent instead.

---

## Location Resolution

Location is resolved in this order:
1. `request.location` field in TaskState
2. Datasource config: `config.location` or `config.table` for the matching driver
3. 400 error if none found

---

## Driver Loading

The broker maintains its own `DriverManager` (`lazy_static Mutex`), separate from
`PERSISTENCE_DRIVER_REGISTRY`. On `POST /drivers/reload`:
1. Reads `conf/drivers.yaml` via `ox_fileproc::process_file` (`!include` supported).
2. Loads each enabled driver: resolves platform library filename, calls `libloading::Library::new()`.
3. Resolves ABI symbols; calls `ox_driver_init("{}")` for context; reads metadata.
4. Stores in manager, keyed by driver name.

Drivers in the broker's manager are dropped (and `ox_driver_destroy` called) on
`ox_plugin_destroy` or on reload of a driver that replaces a previously loaded one.

---

## Error Cases

| Condition | HTTP |
|---|---|
| Path not matched | 404 |
| `driver_name` not loaded | 404 |
| Driver persist returns non-zero | 500 |
| `drivers/reload` partial failure | 207 |
| Location not resolvable | 400 |
| WebSocket upgrade required | 400 |
