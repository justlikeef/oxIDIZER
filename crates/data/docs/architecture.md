# ox_data Architecture

This document explains how the data layer is designed for plugin developers and
contributors.

---

## GenericDataObject (GDO)

The fundamental unit. A GDO is:
- A UUID-based identity (`identifier_name` + UUID stored as an attribute)
- A typed attribute map (`HashMap<String, AttributeValue>`)
- An opaque extension slot (`HashMap<String, serde_json::Value>`) for addons
- A per-object `CallbackManager`

`AttributeValue` holds a heap-allocated `Box<dyn Any + Send + Sync>`, a `ValueType`
tag, and optional conversion parameters. Values are stored in their native Rust type;
conversion happens on `get<T>()` via the `ConversionRegistry`.

```
AttributeValue {
    value:                Box<dyn Any + Send + Sync>   // native type
    value_type:           ValueType                    // e.g. "string", "integer"
    value_type_parameters: HashMap<String, String>    // e.g. {"precision": "2"}
}
```

---

## Callback Two-Level Dispatch

Every data manipulation fires callbacks at two levels:

1. **Per-object:** `gdo.callbacks` (registered with `gdo.register_callback(...)`)
2. **Global:** `CALLBACK_MANAGER` (registered with `CALLBACK_MANAGER.lock().unwrap().register(...)`)

Per-object callbacks fire first, then global callbacks. Both receive the same mutable
GDO reference. If any callback returns `Err`, dispatch stops.

The per-object level handles object-specific logic (e.g., compute a derived field when
a base field changes). The global level handles system-wide cross-cutting concerns
(e.g., audit logging, real-time change broadcast in `ox_data_broker`).

### Addon Hooks

Addons register callbacks in the global `CALLBACK_MANAGER` or on individual GDOs:

| Addon | Event | Registered on | Purpose |
|---|---|---|---|
| `ox_persistence` | `after_set` | per-object | Transition `Hydrated/Consistent → Modified` |
| `ox_data_broker` | `after_set` | global | Broadcast change to WebSocket subscribers |
| `ox_data_broker` | `after_commit` | global | Broadcast commit notification |

---

## DataObjectState Machine

Managed by `ox_persistence`, stored in GDO extension slot `"ox.persistence"`:

```
New
 └─persist()──► Consistent

NotHydrated
 └─hydrate()──► Hydrated

Hydrated ──set()──► Modified   (via after_set callback)
         ──persist()──► Consistent
         ──delete()──► Deleted

Modified ──persist()──► Consistent
         ──discard()──► Hydrated  (re-loads from store)

Consistent ──set()──► Modified
           ──delete()──► Deleted

Deleted ──persist()──► (record removed from store)
```

The state is advisory in-memory: the authoritative record is always the backing store.
On `hydrate()` / `discard()`, the state is refreshed from the driver.

---

## PersistenceDriver FFI ABI

Drivers are `cdylib` shared libraries. The host loads them via `libloading` and resolves
these C-compatible symbols:

| Symbol | Signature |
|---|---|
| `ox_driver_init` | `fn(*const c_char) -> *mut c_void` |
| `ox_driver_destroy` | `fn(*mut c_void)` |
| `ox_driver_persist` | `fn(*mut c_void, location: *const c_char, data: *const c_char) -> c_int` |
| `ox_driver_restore` | `fn(*mut c_void, location: *const c_char, id: *const c_char) -> OxBuffer` |
| `ox_driver_fetch` | `fn(*mut c_void, location: *const c_char, filter: *const c_char) -> OxBuffer` |
| `ox_driver_delete` | `fn(*mut c_void, location: *const c_char, id: *const c_char) -> c_int` |
| `ox_driver_free_buffer` | `fn(OxBuffer)` |
| `ox_driver_get_driver_metadata` | `fn() -> *mut c_char` |
| `ox_driver_get_config_schema` | `fn() -> *mut c_char` |
| `ox_driver_call_action` | `fn(*mut c_void, action: *const c_char, params: *const c_char) -> OxBuffer` (optional) |
| `ox_driver_list_datasets` | `fn(*mut c_void, config: *const c_char) -> OxBuffer` (optional) |
| `ox_driver_describe_dataset` | `fn(*mut c_void, name: *const c_char) -> OxBuffer` (optional) |

**Wire format** for persist/restore/fetch is the `to_serializable_map()` JSON format:
```json
{
  "id":    ["550e8400-...", "uuid",    {}],
  "name":  ["Alice",       "string",  {}],
  "score": ["99.5",        "float",   {"precision": "2"}],
  "__extensions__": ["{\"ox.persistence\":{...}}", "string", {}]
}
```

Drivers store and return `"__extensions__"` opaquely.

`OxBuffer` is a C-compatible heap buffer:
```rust
#[repr(C)]
pub struct OxBuffer { pub ptr: *mut u8, pub len: usize, pub cap: usize }
```
The driver allocates; the caller frees via `ox_driver_free_buffer`.

---

## Broker REST API

`ox_data_broker` is a stateless REST gateway over the driver registry:

| Endpoint | Action |
|---|---|
| `POST /data/{driver}/persist` | Call `driver.persist(location, map)` |
| `GET /data/{driver}/restore` | Call `driver.restore(location, id)` |
| `POST /data/{driver}/fetch` | Call `driver.fetch(filter, location)` with pagination |
| `DELETE /data/{driver}/record/{id}` | Call `driver.delete(location, id)` |
| `GET /data/listen` | WebSocket; subscribe to object change events |
| `GET /drivers` | List loaded drivers |
| `POST /drivers/reload` | Reload `conf/drivers.yaml` |

Location resolution order: `request.location` field → datasource config → 400 error.

The broker maintains its own `DriverManager` (separate from `PERSISTENCE_DRIVER_REGISTRY`)
as a `lazy_static`. This allows it to run as an isolated plugin without coupling to the
in-process registry.

---

## Transaction System

Locking state is stored in `"ox.transaction"` extension slot. `LockStatus` variants:
`Unlocked`, `Locked { holder, expires_at }`, `PendingLock(holder)`, `PendingUnlock(holder)`.

Lock acquisition: the driver's `notify_lock_status_change()` is called on every
lock/unlock so the backing store can enforce the lock (e.g., database row lock).

Transaction semantics in `DataObjectManager.save_data_object`:
- **With active transaction:** if any container write fails, previously completed writes
  are reversed and `rollback_transaction()` is called.
- **Without transaction:** failures are logged; execution continues to remaining
  containers.

`rollback_transaction()` calls `discard()` on the GDO (re-loads from store) then
`unlock()`. This ensures the in-memory state reverts to what the database holds.

---

## Dictionary and Introspection System

The data dictionary has two levels:

1. **Physical (`DataStoreContainer`):** describes one storage unit (table, file path,
   etc.) with its fields and `ValueType`s.
2. **Logical (`DataObjectDefinition`):** describes a named object type. Attributes map
   to container fields via `AttributeMapping::Direct` or are computed via
   `AttributeMapping::Calculated` (template expressions like `"{first_name} {last_name}"`).

`DataObjectManager.load_data_object` builds a `QueryPlan` from the definition and
executes it via `QueryEngine`. The query engine handles cross-datasource joins at the
Rust level (not in SQL) using `QueryNode::Fetch` leaves joined by `QueryNode::Join`
interior nodes.

**Calculated attributes** are not in the query plan — they are evaluated after all
`Direct` attributes are loaded, and they are skipped on save.

The `Introspectable` trait (on `GenericDataObject`) exposes attribute names, types,
parameters, and coerced string values without firing callbacks. This is the interface
consumed by `ox_forms_api` and external introspection tools.

---

## Validation System

Validation is entirely separate from the dictionary and from `DataObjectManager`. Callers
register a `ValidationSet` for an object type in `VALIDATION_REGISTRY`, then call
`gdo.validate(object_id)` before saving. `DataObjectManager.save_data_object` does not
run validation internally.

The `before_validate` / `after_validate` / `on_error_validate` events fire through the
GDO's callback system, so validation runs can be observed and hooked.

`constraint_json()` on each `ValidationRule` returns structured rule parameters (e.g.,
`{"min": 8}`) for form rendering and API introspection.

---

## Extension Slot Namespaces

| Key | Owner | Contents |
|---|---|---|
| `"ox.persistence"` | `ox_persistence` | `driver`, `location`, `state` |
| `"ox.transaction"` | `ox_transaction` | `status`, `holder`, `expires_at`, `transaction_active` |

No two addons share an extension slot key. New addons should choose unique,
prefixed keys (e.g., `"my_plugin.state"`).
