# ox_persistence — Persistence Layer

**Crate:** `ox_persistence`
**Type:** library

Defines the persistence addon: state machine, driver registry, and the `Persistent` trait.
The persistence system works with `GenericDataObject` instances but is entirely separate
from the core data object — `GenericDataObject` has no knowledge of persistence. The addon
stores its state in the GDO's extension slot under `"ox.persistence"`.

---

## DataObjectState

Tracks the relationship between a `GenericDataObject` in memory and its backing store.
Stored in the `"ox.persistence"` extension slot on the object.

```rust
pub enum DataObjectState {
    New,          // created in memory; no backing store record
    NotHydrated,  // shell: has ID, full data not yet loaded
    Hydrated,     // fully loaded from datastore
    Modified,     // in-memory state differs from datastore
    Consistent,   // in-memory state matches datastore
    Deleted,      // marked for deletion; not yet removed from store
}
```

**State transitions:**

```
New ──persist()──► Consistent
New ──(nothing)──► New

NotHydrated ──hydrate()──► Hydrated

Hydrated ──set()──► Modified        (via after_set callback)
          ──persist()──► Consistent
          ──delete()──► Deleted

Modified ──persist()──► Consistent
          ──discard()──► Hydrated    (re-loads from store, discards in-memory changes)

Consistent ──set()──► Modified      (via after_set callback)
            ──delete()──► Deleted

Deleted ──persist()──► (driver.delete() called; record removed from store)
```

### Hooking set() → Modified

The GDO core does not know about `DataObjectState`. The transition `Hydrated/Consistent →
Modified` is driven by an `after_set` callback registered by the persistence addon when
`PersistenceInfo` is first attached to a GDO. This callback reads the current state from
the `"ox.persistence"` extension slot and updates it to `Modified` if the current state
is `Hydrated` or `Consistent`.

```rust
// Registered automatically by persist_to() / hydrate_from()
let state_callback = Arc::new(|gdo: &mut GenericDataObject, _params: &CallbackParams| {
    if let Some(state) = gdo.persistence_state() {
        if matches!(state, DataObjectState::Hydrated | DataObjectState::Consistent) {
            gdo.set_persistence_state(DataObjectState::Modified);
        }
    }
    Ok(())
});
gdo.register_callback(EventType::new("after_set"), state_callback);
```

---

## PersistenceInfo

Holds the location metadata so an object can self-hydrate and self-persist.
Stored alongside `DataObjectState` in the `"ox.persistence"` extension slot.

```rust
pub struct PersistenceInfo {
    pub driver_name: String,  // registered driver name, e.g. "postgres"
    pub location: String,     // driver-specific location: table name, file path, etc.
}
```

---

## Extension Slot: `"ox.persistence"`

All persistence state is stored in the GDO's extensions map under `"ox.persistence"`:

```json
{
  "driver": "postgres",
  "location": "users",
  "state": "Hydrated"
}
```

This value survives `to_serializable_map()` / `from_serializable_map()`, so a loaded
object always knows which driver and location it belongs to.

---

## Persistence Callback Events

Every persistence operation fires through the GDO's two-level callback dispatch
(per-object first, then global `CALLBACK_MANAGER`).

| Event | Fired by |
|-------|----------|
| `before_persist` / `after_persist` / `on_error_persist` | `persist`, `persist_to` |
| `before_hydrate` / `after_hydrate` / `on_error_hydrate` | `hydrate`, `hydrate_from` |
| `before_fetch` / `after_fetch` / `on_error_fetch` | `fetch`, `fetch_from` |
| `before_delete` / `after_delete` / `on_error_delete` | `delete` |
| `before_discard` / `after_discard` / `on_error_discard` | `discard` |

`after_*` fires only on success. `on_error_*` fires in its place on failure, with
`params.error` set to the error description.

---

## Persistent Trait

Implemented directly on `GenericDataObject`. Self-aware methods (no arguments) use
`PersistenceInfo` from the extension slot. Explicit methods set `PersistenceInfo` first.

```rust
pub trait Persistent {
    // Self-aware (use stored PersistenceInfo — error if not set)
    fn persist(&mut self) -> Result<(), OxDataError>;
    fn hydrate(&mut self) -> Result<(), OxDataError>;
    fn fetch(&self) -> Result<Vec<GenericDataObject>, OxDataError>;
    fn delete(&mut self) -> Result<(), OxDataError>;
    fn discard(&mut self) -> Result<(), OxDataError>;

    // Explicit (set PersistenceInfo and operate)
    fn persist_to(&mut self, driver_name: &str, location: &str) -> Result<(), OxDataError>;
    fn hydrate_from(&mut self, driver_name: &str, location: &str) -> Result<(), OxDataError>;
    fn fetch_from(&self, driver_name: &str, location: &str) -> Result<Vec<GenericDataObject>, OxDataError>;

    // State and info access
    fn persistence_state(&self) -> Option<DataObjectState>;
    fn persistence_info(&self) -> Option<PersistenceInfo>;
    fn set_persistence_info(&mut self, info: PersistenceInfo);
    fn set_persistence_state(&mut self, state: DataObjectState);
}
```

### persist / persist_to

1. Fire `before_persist`.
2. Resolve driver from `PERSISTENCE_DRIVER_REGISTRY`.
3. If state is `Deleted`: call `driver.delete(location, &id)` → update state to `Deleted`
   (record removed from store) → fire `after_persist` → return.
4. Otherwise: call `self.to_serializable_map()` → `driver.persist(&map, location)`.
5. On success: update state → `Consistent`. If `PersistenceInfo` not yet set, store it.
   Register the `after_set → Modified` callback. Fire `after_persist`.
6. On failure: fire `on_error_persist`.

### hydrate / hydrate_from

1. Fire `before_hydrate`.
2. Resolve driver. Read the ID attribute value.
3. Call `driver.restore(location, &id)`.
4. For each entry in the returned map (excluding `"__extensions__"`): call `set_with_type`.
5. Restore `extensions` from `"__extensions__"` in the returned map.
6. Update state → `Hydrated`. Store `PersistenceInfo`. Register `after_set → Modified` callback.
7. Fire `after_hydrate`. On failure: fire `on_error_hydrate`.

### fetch / fetch_from

1. Fire `before_fetch`.
2. Build filter map from non-empty attributes via `to_serializable_map()`.
3. Call `driver.fetch(&filter, location)` → Vec of IDs.
4. For each ID: `driver.restore(location, id)` → `GenericDataObject::from_serializable_map`.
5. Fire `after_fetch`. Return Vec. On failure: fire `on_error_fetch`.

### delete

1. Fire `before_delete`.
2. Update state → `Deleted` in extension slot.
3. Fire `after_delete`.

The record is **not** removed from the store until `persist()` is subsequently called.

### discard

1. Fire `before_discard`.
2. Call `hydrate()` to re-load all attributes from the store, overwriting in-memory changes.
3. Update state → `Hydrated`.
4. Fire `after_discard`. On failure: fire `on_error_discard`.

---

## PersistenceDriver Trait

```rust
pub trait PersistenceDriver: Send + Sync {
    fn persist(
        &self,
        map: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<(), OxDataError>;

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, OxDataError>;

    fn fetch(
        &self,
        filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<Vec<String>, OxDataError>;

    fn delete(&self, location: &str, id: &str) -> Result<(), OxDataError>;

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str);

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), OxDataError>;
    fn list_datasets(&self, connection_info: &HashMap<String, String>) -> Result<Vec<String>, OxDataError>;
    fn describe_dataset(&self, connection_info: &HashMap<String, String>, name: &str) -> Result<DataSet, OxDataError>;
    fn get_connection_parameters(&self) -> Vec<ConnectionParameter>;

    fn call_action(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value, OxDataError> {
        Err(OxDataError::DriverError(format!("Action '{}' not supported", action)))
    }
}
```

**fetch filter semantics:** equality conjunctions (`WHERE a = x AND b = y`). Empty filter
returns all IDs. Richer queries use `call_action("query", params)`.

---

## PersistenceDriverRegistry

```rust
lazy_static! {
    pub static ref PERSISTENCE_DRIVER_REGISTRY: Mutex<PersistenceDriverRegistry> = ...;
}

pub fn register_persistence_driver(driver: Arc<dyn PersistenceDriver>, metadata: DriverMetadata);
pub fn get_registered_drivers() -> Vec<DriverMetadata>;
pub fn unregister_persistence_driver(driver_name: &str);
```

---

## Supporting Types

### OxBuffer

C-compatible heap buffer for FFI. `OxBuffer::from_str(s)` takes ownership of a String.
`unsafe fn free_ox_buffer(buf: OxBuffer)` — no-mangle export for driver callers.

```rust
#[repr(C)]
pub struct OxBuffer { pub ptr: *mut u8, pub len: usize, pub cap: usize }
```

### DataSet / ColumnDefinition / ColumnMetadata

Schema description returned by `describe_dataset`.

### DriverMetadata / ConfiguredDriver / DriversList

See [spec/drivers.md](drivers.md).

---

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `ox_data_object` | `GenericDataObject`, `CallbackManager` |
| `ox_type_converter` | `ValueType` |
| `serde` / `serde_json` | Serialization |
| `lazy_static` | `PERSISTENCE_DRIVER_REGISTRY` |
| `libc` | `OxBuffer` FFI types |
