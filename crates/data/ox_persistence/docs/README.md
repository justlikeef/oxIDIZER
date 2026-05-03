# ox_persistence

Persistence addon for `GenericDataObject`. Defines the driver trait, state machine,
and `Persistent` trait. Does not know about SQL or files — all storage is delegated to
dynamically loaded `PersistenceDriver` implementations.

---

## DataObjectState

Stored in GDO extension slot `"ox.persistence"` as `state`:

| State | Meaning |
|---|---|
| `New` | Created in memory; no backing store record |
| `NotHydrated` | Shell object: has ID, full data not yet loaded |
| `Hydrated` | Fully loaded from datastore |
| `Modified` | In-memory state differs from datastore (set after any attribute change) |
| `Consistent` | In-memory matches datastore |
| `Deleted` | Marked for deletion; not yet removed from store |

The transition `Hydrated/Consistent → Modified` is triggered by an `after_set` callback
registered automatically when `PersistenceInfo` is first attached.

---

## PersistenceInfo

```rust
pub struct PersistenceInfo {
    pub driver_name: String,   // registered driver name, e.g. "postgres"
    pub location: String,      // driver-specific: table name, file path, etc.
}
```

Stored in the `"ox.persistence"` extension slot alongside `state`. Survives
serialization round-trips so a loaded object always knows its origin.

---

## Persistent Trait

Implemented on `GenericDataObject`. Self-aware methods use `PersistenceInfo` from the
extension slot. Explicit methods accept `driver_name` and `location` directly.

| Method | Description |
|---|---|
| `persist()` | Write to backing store; state → `Consistent` |
| `persist_to(driver, location)` | Set `PersistenceInfo` and persist |
| `hydrate()` | Load from backing store; state → `Hydrated` |
| `hydrate_from(driver, location)` | Set `PersistenceInfo` and hydrate |
| `fetch()` | Query matching objects; returns `Vec<GenericDataObject>` |
| `fetch_from(driver, location)` | Set `PersistenceInfo` and fetch |
| `delete()` | Mark state `Deleted` (deferred; call `persist()` to execute) |
| `discard()` | Re-load from store, discarding in-memory changes |

---

## PersistenceDriver Trait

```rust
pub trait PersistenceDriver: Send + Sync {
    fn persist(&self, map: &SerializableMap, location: &str) -> Result<(), OxDataError>;
    fn restore(&self, location: &str, id: &str) -> Result<SerializableMap, OxDataError>;
    fn fetch(&self, filter: &SerializableMap, location: &str) -> Result<Vec<String>, OxDataError>;
    fn delete(&self, location: &str, id: &str) -> Result<(), OxDataError>;
    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str);
    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), OxDataError>;
    fn list_datasets(&self, connection_info: &HashMap<String, String>) -> Result<Vec<String>, OxDataError>;
    fn describe_dataset(&self, connection_info: &HashMap<String, String>, name: &str) -> Result<DataSet, OxDataError>;
    fn get_connection_parameters(&self) -> Vec<ConnectionParameter>;
    fn call_action(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value, OxDataError>;
}
```

`fetch` filter semantics: equality conjunctions (`WHERE a = x AND b = y`). Empty filter
returns all IDs. Complex queries use `call_action("query", params)`.

---

## PERSISTENCE_DRIVER_REGISTRY

Global `lazy_static Mutex<PersistenceDriverRegistry>`. All in-process code that needs
to resolve drivers by name goes through this registry.

Key functions:
- `register_persistence_driver(driver, metadata)` — add driver
- `get_registered_drivers()` — list metadata
- `unregister_persistence_driver(name)` — remove driver

---

## Callback Events (fired on GDO)

| Event pair | Fired by |
|---|---|
| `before_persist` / `after_persist` / `on_error_persist` | `persist`, `persist_to` |
| `before_hydrate` / `after_hydrate` / `on_error_hydrate` | `hydrate`, `hydrate_from` |
| `before_fetch` / `after_fetch` / `on_error_fetch` | `fetch`, `fetch_from` |
| `before_delete` / `after_delete` / `on_error_delete` | `delete` |
| `before_discard` / `after_discard` / `on_error_discard` | `discard` |

---

## OxBuffer (FFI)

C-compatible heap buffer used by the driver ABI:
```rust
#[repr(C)]
pub struct OxBuffer { pub ptr: *mut u8, pub len: usize, pub cap: usize }
```
`OxBuffer::from_str(s)` takes ownership. `unsafe fn free_ox_buffer(buf: OxBuffer)` is
a `#[no_mangle]` export for driver callers.
