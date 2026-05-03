# ox_data — Data Layer

The data layer provides a complete, driver-agnostic persistence system for the
oxIDIZER platform. It consists of a layered crate hierarchy where each layer adds
capabilities on top of the one below, and management plugins expose those capabilities
as REST APIs.

---

## Crate Hierarchy

```
ox_type_converter          # ValueType tag + ConversionRegistry
       ↓
ox_data_object             # GenericDataObject + callback system
       ↓
ox_persistence             # Persistence addon (DataObjectState, PersistenceDriver trait)
       ↓
ox_data_object_manager     # Data dictionary (DataObjectDefinition), DataObjectManager, query engine
       ↓
ox_validation              # Validation addon (ValidationSet, ValidationRule)
ox_transaction             # Transaction/locking addon (Transactable, LockStatus)
       ↓
ox_data_broker             # REST plugin — exposes persist/restore/fetch over HTTP
       ↓
(management plugins)       # driver manager, datasource manager, driver installer
```

Each layer depends only on the layers below it. The GDO core knows nothing about
persistence, transactions, or validation — those are addons that hook in via the
callback system and extension slot.

---

## Layer Responsibilities

### ox_type_converter

Defines `ValueType` (an opaque string tag) and `ConversionRegistry` (a global map of
`(from_type, to_type) → converter_fn`). All attribute values in the system carry a
`ValueType` tag. Type conversion is explicit and always goes through the registry.

### ox_data_object

`GenericDataObject` (GDO) is the core data container: a UUID-based attribute map with a
two-level callback system. Every data manipulation fires `before_*` / `after_*` /
`on_error_*` events through both the per-object callback manager and the global
`CALLBACK_MANAGER`. Addon systems (persistence, transaction, validation) hook in here.

The extension slot (`extensions: HashMap<String, serde_json::Value>`) lets addons store
their own state on a GDO without requiring a wrapper struct.

### ox_persistence

Defines the `DataObjectState` state machine (`New → Consistent`, `Modified → Consistent`,
`Deleted → removed`), the `PersistenceInfo` struct (driver name + location), and the
`Persistent` trait implemented on `GenericDataObject`. Introduces the
`PersistenceDriver` trait: the C-compatible FFI ABI that all driver crates implement.

The `PERSISTENCE_DRIVER_REGISTRY` is a global `Mutex<HashMap>` of loaded drivers.

### ox_data_object_manager

Adds a metadata layer. The `DataDictionary` holds `DataStoreContainer` definitions
(physical schema) and `DataObjectDefinition` objects (logical schema with attribute
mappings). `DataObjectManager` uses dictionary metadata to load and save objects across
multiple datasource containers, including cross-datasource joins via the `QueryEngine`.

### ox_validation

Standalone addon. `ValidationSet` holds `ValidationRule` implementations for a named
object type. Rules are run before `save_data_object` by caller code (not automatically).
Built-in rules: `Required`, `MinLength`, `MaxLength`, `Min`, `Max`, `Range`, `Regex`,
`OneOf`, `NotOneOf`, `Matches`, `Custom`.

### ox_transaction

Pessimistic locking and transaction semantics. `LockStatus` is stored in the GDO's
`"ox.transaction"` extension slot. `begin_transaction` / `commit_transaction` /
`rollback_transaction` wrap the lock lifecycle. Integrates with `DataObjectManager.save_data_object`
for atomic multi-container writes.

### ox_data_broker

A `cdylib` REST plugin that exposes persist/restore/fetch/delete operations over HTTP.
Also provides a WebSocket endpoint for real-time change notifications via the global
`after_set` callback. Loads drivers dynamically from `conf/drivers.yaml`.

### Management Plugins

- **`ox_persistence_driver_manager`** — load/unload driver libraries at runtime
- **`ox_persistence_datasource_manager`** — CRUD for datasource definitions; dataset
  discovery and auto-import into the dictionary
- **`ox_persistence_driver_installer`** — install driver packages via `ox_package_manager`

---

## Drivers

Persistence drivers are `cdylib` shared libraries that implement the driver FFI ABI.
They are loaded dynamically; no driver is compiled into the host process.

Available drivers (under `ox_persistence/drivers/`):

| Driver | Description |
|---|---|
| `ox_persistence_driver_db_postgres` | PostgreSQL |
| `ox_persistence_driver_db_mysql` | MySQL / MariaDB |
| `ox_persistence_driver_db_mssql` | Microsoft SQL Server |
| `ox_persistence_driver_db_sqlite` | SQLite (single-node/development) |
| `ox_persistence_driver_db_sql` | SQL base library shared by DB drivers |
| `ox_persistence_driver_file_json` | JSON files |
| `ox_persistence_driver_file_yaml` | YAML files |
| `ox_persistence_driver_file_xml` | XML files |
| `ox_persistence_driver_file_delimited` | CSV/TSV and other delimited formats |
| `ox_persistence_gdo_relational` | Cross-GDO relationship storage (meta-driver) |

---

## Addons at a Glance

| Crate | Extension slot key | Trait implemented on GDO |
|---|---|---|
| `ox_persistence` | `"ox.persistence"` | `Persistent` |
| `ox_transaction` | `"ox.transaction"` | `Transactable` |
| `ox_validation` | — (registry, not per-object) | `Validatable` |

---

## Module Reference Index

| Module | Type | Purpose |
|---|---|---|
| `ox_type_converter` | library | ValueType, ConversionRegistry |
| `ox_data_object` | library | GenericDataObject, callbacks |
| `ox_persistence` | library | Persistence state machine, PersistenceDriver trait |
| `ox_data_object_manager` | library | Dictionary, DataObjectManager, query engine |
| `ox_data_object/ox_data_object_manager` | library | (sub-crate of ox_data_object) |
| `ox_data_object/ox_data_object_dictionary_manager` | plugin | Dictionary manager REST API |
| `ox_persistence/ox_persistence_api` | library | Public persistence API surface |
| `ox_persistence/ox_persistence_dictionary_manager` | plugin | Persistence dictionary REST API |
| `ox_data_broker` | plugin | REST broker for GDO operations |
| `ox_persistence_datasource_manager` | plugin | Datasource CRUD and discovery |
| `ox_persistence_driver_manager` | plugin | Driver load/unload management |
| `ox_persistence_driver_installer` | plugin | Package-based driver installation |
| `ox_persistence_gdo_relational` | driver | Cross-GDO relationship storage |
| `ox_transaction` | library | Record locking and transactions |
| `ox_validation` | library | Attribute validation rules |
| `ox_data_error` | library | Shared error types |
