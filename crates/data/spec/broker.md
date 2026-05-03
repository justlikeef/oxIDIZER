# ox_data_broker — Data Broker REST API

**Crate:** `ox_data_broker`
**Type:** `cdylib` plugin (`ox_plugin_init` / `ox_plugin_process` / `ox_plugin_destroy`)

Provides a remote HTTP API for performing GDO persist/restore/fetch operations through
any registered persistence driver. Acts as a gateway — callers do not need to link against
`ox_persistence` directly.

---

## Routes

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/drivers` | List loaded drivers and their metadata |
| `POST` | `/drivers/reload` | Load/reload drivers from `conf/drivers.yaml` |
| `POST` | `/data/{driver_name}/persist` | Persist a GDO via the named driver |
| `GET` | `/data/{driver_name}/restore` | Restore a GDO by ID |
| `POST` | `/data/{driver_name}/fetch` | Fetch GDOs matching a filter, with pagination |
| `DELETE` | `/data/{driver_name}/record/{id}` | Mark GDO as deleted and remove from store |
| `GET` | `/data/listen` | WebSocket upgrade — subscribe to change events for one or more object IDs |

---

## Route Details

### `GET /drivers`

Returns the metadata of all drivers currently loaded in the broker's local `DriverManager`.

Response `200`:
```json
[
  {
    "name": "ox_persistence_pg",
    "friendly_name": "PostgreSQL Driver",
    "version": "1.0.0",
    "description": "...",
    "compatible_modules": {}
  }
]
```

### `POST /drivers/reload`

Re-reads `conf/drivers.yaml` and loads all enabled drivers. Drivers already loaded by
name are replaced. Uses `ox_fileproc::process_file` for `!include` support.

Request: no body.

Response `200` (no errors):
```json
{"loaded": 2, "errors": []}
```

Response `207` (partial success):
```json
{"loaded": 1, "errors": ["Failed 'ox_persistence_mysql': library not found"]}
```

### `POST /data/{driver_name}/persist`

Persists a GDO through the named driver.

Request body: JSON map in `to_serializable_map()` format:
```json
{
  "id":   ["550e8400-…", "uuid",   {}],
  "name": ["Alice",      "string", {}],
  "__extensions__": ["{\"ox.persistence\":{…}}", "string", {}]
}
```

The `location` is resolved from the datasource configuration for the driver (see
Location Resolution below).

Response `200`: `{"ok": true}`
Response `500`: `{"error": "persist failed"}`

### `GET /data/{driver_name}/restore`

Restores a GDO by ID.

Request body (or query param `id`): the UUID string of the object to restore.

Response `200`: JSON map in `to_serializable_map()` format (same as persist body).
Response `404`: `{"error": "Not found"}`

### `POST /data/{driver_name}/fetch`

Fetches GDOs matching a filter, with optional pagination.

Query parameters:
- `limit` — maximum records to return (default: 100, max: 1000)
- `offset` — number of records to skip (default: 0)

Request body: JSON map of equality filters (same format as the persist body, but only
the fields to filter on need be present):
```json
{
  "status": ["active", "string", {}]
}
```

Response `200`:
```json
{
  "data": [ { ... }, { ... } ],
  "total": 42,
  "limit": 100,
  "offset": 0
}
```

`total` is the total number of matching records before pagination. Drivers that do not
support counting return `null` for `total`.

### `DELETE /data/{driver_name}/record/{id}`

Removes a record from the backing store immediately. This is a direct deletion — it does
not go through the `Deleted` state machine (which is an in-process concept). The broker
is stateless; it calls `driver.delete(location, id)` directly.

Response `200`: `{"ok": true}`
Response `404`: `{"error": "Not found"}`
Response `500`: `{"error": "delete failed"}`

### `GET /data/listen` (WebSocket)

Upgrades the connection to a WebSocket. After the upgrade, the client sends a
subscription message to declare which object IDs it wants to watch:

```json
{ "subscribe": ["550e8400-…", "6ba7b810-…"] }
```

The broker registers the connection in its `LISTENER_REGISTRY` against each requested
ID. From that point on, any `after_set` (or `after_commit`) event on a watched GDO is
pushed to all subscribed connections for that object.

To unsubscribe from specific IDs without closing the connection:

```json
{ "unsubscribe": ["550e8400-…"] }
```

**Change event message** (broker → client):

```json
{
  "object_id": "550e8400-…",
  "attribute": "email",
  "value": "new@example.com",
  "event": "after_set"
}
```

If the change was part of a committed transaction, `"event"` is `"after_commit"` and
`"attribute"` / `"value"` are omitted (the client should re-fetch the full object).

---

## Change Listener System

### LISTENER_REGISTRY

A process-global registry mapping object ID to active WebSocket connections.

```rust
lazy_static! {
    static ref LISTENER_REGISTRY: Mutex<HashMap<Uuid, Vec<WebSocketSender>>> = ...;
}
```

`WebSocketSender` is a cloneable handle capable of sending text frames to a connected
client. Stale handles (disconnected clients) are pruned on send failure.

### Global after_set Callback

During `ox_plugin_init`, the broker registers a global callback in `CALLBACK_MANAGER`:

```rust
CALLBACK_MANAGER.lock().unwrap().register(
    EventType::new("after_set"),
    Arc::new(|gdo, params| {
        let id = gdo.get::<Uuid>(&gdo.identifier_name)?;
        // Skip broadcast if a transaction is active on this object
        if gdo.transaction_active() { return Ok(()); }
        broadcast_change(id, params);
        Ok(())
    }),
);
```

`broadcast_change` locks `LISTENER_REGISTRY`, looks up the object ID, and sends a
change event frame to each registered sender. Failed sends (disconnected clients) are
removed from the registry.

### Transaction Awareness

When a transaction is active on the GDO, `after_set` callbacks fire but the broker
suppresses broadcast. Instead, the broker also registers a global `after_commit` callback
that broadcasts a single notification without attribute details — signalling that the
object has changed and clients should re-fetch:

```rust
CALLBACK_MANAGER.lock().unwrap().register(
    EventType::new("after_commit"),
    Arc::new(|gdo, _params| {
        let id = gdo.get::<Uuid>(&gdo.identifier_name)?;
        broadcast_commit(id);
        Ok(())
    }),
);
```

On `after_rollback`, no broadcast is sent — the object did not change from the
listeners' perspective.

### Multiple Listeners

Any number of clients may subscribe to the same object ID. All receive the same event
frame. There is no coordination between listeners — each receives an independent copy
of the message. The server does not deduplicate subscriptions; subscribing to the same ID
twice from the same connection results in duplicate messages.

---

## Location Resolution

The `location` passed to driver operations is resolved from the configured datasource for
the driver, not hardcoded. Resolution order:

1. Request field `request.location` (allows caller to override per-request).
2. Datasource configuration: look up the datasource whose `driver_id` matches
   `driver_name`; use `config.location` or `config.table` as the location string.
3. Error `400` if no location can be resolved.

This replaces the stub behaviour of hardcoding `"data.csv"`.

---

## Driver Loading

The broker maintains its own `DriverManager` (separate from `PERSISTENCE_DRIVER_REGISTRY`)
as a `lazy_static Mutex<DriverManager>`. This allows the broker to run as an isolated
plugin without coupling to the in-process registry.

### LoadedDriver

Wraps a dynamically loaded library and caches the resolved function pointers:

```rust
struct LoadedDriver {
    library: Library,         // kept alive to prevent unloading
    context: *mut c_void,     // opaque driver context
    destroy_fn: DestroyFn,
    persist_fn: PersistFn,
    restore_fn: RestoreFn,
    fetch_fn: FetchFn,
    free_buffer_fn: FreeBufferFn,
    metadata: DriverMetadata,
}
```

On `Drop`, calls `destroy_fn(context)` to free driver resources.

### Load Sequence

1. Resolve platform-specific filename (`lib{name}.so` / `.dylib` / `.dll`).
2. `Library::new(path)` — loads the shared library.
3. Resolve all required symbols.
4. Call `ox_driver_init("{}")` to get context (config supplied separately for configured drivers).
5. Call `ox_driver_get_driver_metadata()` to get `DriverMetadata`.
6. Store in `DriverManager.drivers` keyed by `metadata.name`.

---

## Plugin Lifecycle

### `ox_plugin_init(config_json, api_ptr, abi_version) -> *mut c_void`

Stores `CoreHostApi` in a `ModuleContext` heap allocation. Returns pointer as plugin
context. Logs `"ox_data_broker initialized"`.

### `ox_plugin_process(plugin_ctx, task_ctx) -> FlowControl`

Routes based on `request.path` and `request.method` task fields. Always returns
`FLOW_CONTROL_CONTINUE` — the broker does not interrupt the pipeline.

### `ox_plugin_destroy(plugin_ctx)`

Drops `ModuleContext`. All `LoadedDriver` instances in `DRIVER_MANAGER` are also dropped
(their `Drop` impl calls `destroy_fn`).

---

## Error Cases

| Condition | HTTP | Body |
|-----------|------|------|
| Path not matched | 404 | `{"error":"Not found"}` |
| `driver_name` not loaded | 404 | `{"error":"Driver X not found"}` |
| Path segments < 4 for `/data/` routes | 400 | `{"error":"Invalid path"}` |
| Driver persist returns non-zero | 500 | `{"error":"persist failed"}` |
| `drivers/reload` partial failure | 207 | `{"loaded": N, "errors": [...]}` |
| Location not resolvable | 400 | `{"error":"Location not found for driver X"}` |
| WebSocket upgrade fails | 400 | `{"error":"WebSocket upgrade required"}` |

---

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `ox_workflow_abi` | Plugin ABI |
| `ox_persistence` | `OxBuffer`, `DriverMetadata`, `DriversList` |
| `ox_transaction` | `transaction_active()` check in callback |
| `ox_fileproc` | `process_file` for drivers.yaml loading |
| `libloading` | Dynamic library loading |
| `serde_json` | Request/response serialization |
| `lazy_static` | `DRIVER_MANAGER`, `LISTENER_REGISTRY` globals |
| `tungstenite` (or `tokio-tungstenite`) | WebSocket framing and upgrade |
