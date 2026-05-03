# ox_data_object_manager — Data Dictionary & Object Manager

**Crate:** `ox_data_object_manager` (sub-crate of `ox_data_object`)
**Type:** library

Adds a metadata layer on top of `GenericDataObject`. The data dictionary describes the
logical structure of objects and their physical mapping to datasource containers. The
`DataObjectManager` uses this metadata to load and save objects across one or more
datasources with automatic field remapping. Validation is a separate addon
(see [spec/validation.md](validation.md)).

---

## Data Dictionary

### DataDictionary

The root metadata container. Serializes to/from JSON for file-based persistence.

```rust
pub struct DataDictionary {
    pub containers: HashMap<String, DataStoreContainer>,  // keyed by container id
    pub objects: HashMap<String, DataObjectDefinition>,   // keyed by object id
}
```

| Method | Description |
|--------|-------------|
| `new()` | Empty dictionary |
| `add_container(c)` | Insert container by id |
| `add_object(o)` | Insert object definition by id |
| `merge_container(c)` | Update fields of existing container; insert if new |
| `save_to_file(path)` | Serialize to JSON |
| `load_from_file(path)` | Deserialize from JSON |

### DataStoreContainer

Describes a physical storage unit — a table, file, API endpoint, key, etc.

```rust
pub struct DataStoreContainer {
    pub id: String,
    pub datasource_id: String,      // driver name in PERSISTENCE_DRIVER_REGISTRY
    pub name: String,               // physical name: table, file path, etc.
    pub container_type: String,     // "table" | "view" | "file" | "key"
    pub fields: Vec<DataStoreField>,
    pub metadata: HashMap<String, String>,
}
```

### DataStoreField

```rust
pub struct DataStoreField {
    pub name: String,
    pub data_type: ValueType,
    pub parameters: HashMap<String, String>,
    pub description: Option<String>,
}
```

---

## Object Definition

### DataObjectDefinition

```rust
pub struct DataObjectDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub attributes: Vec<DataObjectAttribute>,
    pub relationships: Vec<RelationshipDefinition>,
}
```

The `id` attribute must exist and have a `Direct` mapping to the primary container's
primary key field. `DataObjectManager` uses it to locate records.

### DataObjectAttribute

```rust
pub struct DataObjectAttribute {
    pub name: String,
    pub data_type: ValueType,
    pub mapping: AttributeMapping,
    pub description: Option<String>,
}
```

Validation rules are no longer defined on `DataObjectAttribute`. They are defined
separately in the validation addon and attached to a `DataObjectDefinition` by name.
See [spec/validation.md](validation.md).

### AttributeMapping

```rust
pub enum AttributeMapping {
    Direct {
        container_id: String,
        field_name: String,
    },
    Calculated {
        expression: String,
    },
}
```

**Calculated expressions** are string templates using `{attribute_name}` substitution,
evaluated after all `Direct` attributes are loaded. Example: `"{first_name} {last_name}"`.
Calculated attributes are read-only and not written back on save.

---

## Relationships

### RelationshipDefinition

```rust
pub struct RelationshipDefinition {
    pub id: String,
    pub name: String,                         // human name, e.g. "orders"
    pub from_container_id: String,
    pub to_container_id: String,
    pub cardinality: Cardinality,
    pub join_type: JoinType,
    pub conditions: Vec<JoinCondition>,
}
```

### Cardinality

```rust
pub enum Cardinality {
    OneToOne,
    OneToMany,
    ManyToMany { junction_container_id: String },
}
```

`OneToOne` — a single matching row in the target container.
`OneToMany` — multiple rows in the target container; the GDO's attribute for this
relationship holds a `Vec` of child GDOs (loaded separately, not inlined in the
serializable map).
`ManyToMany` — a junction container maps source IDs to target IDs; both sides are loaded
as `Vec` of GDOs.

### JoinType

```rust
pub enum JoinType { Inner, Left, Right, Outer }
```

### JoinCondition

```rust
pub struct JoinCondition {
    pub from_field: String,
    pub to_field: String,
    pub operator: String,   // "=" | "<" | ">" | "<=" | ">=" | "!="
}
```

---

## DataObjectManager

```rust
pub struct DataObjectManager {
    pub dictionary: DataDictionary,
}

impl DataObjectManager {
    pub fn new() -> Self;
    pub fn with_dictionary(dict: DataDictionary) -> Self;
}
```

### Manager Callback Events

All manager operations fire events through the GDO's two-level callback dispatch.

| Event | Fired by |
|-------|----------|
| `before_load` / `after_load` / `on_error_load` | `load_data_object` |
| `before_save` / `after_save` / `on_error_save` | `save_data_object` |
| `before_delete_object` / `after_delete_object` / `on_error_delete_object` | `delete_data_object` |

### create_data_object

```rust
pub fn create_data_object(&self, identifier_name: &str) -> GenericDataObject
```

Returns a new GDO with a fresh UUID and state `DataObjectState::New`. No callbacks fired.

### load_data_object

```rust
pub fn load_data_object(&self, object_id: &str, id: &str) -> Result<GenericDataObject, OxDataError>
```

1. Fire `before_load` on the (not-yet-populated) GDO shell.
2. Look up `DataObjectDefinition`. Build `QueryPlan` from attribute mappings and relationships.
3. Execute plan via `QueryEngine`. Map result row to GDO attributes:
   - `Direct`: convert via `ConversionRegistry`.
   - `Calculated`: evaluate expression after `Direct` attributes are loaded.
4. Set `PersistenceInfo` (driver + location from primary container). Set state `Hydrated`.
5. Fire `after_load`. On failure: fire `on_error_load`.

### save_data_object

```rust
pub fn save_data_object(&self, object_id: &str, gdo: &mut GenericDataObject) -> Result<(), OxDataError>
```

1. Fire `before_save`. This allows the validation addon to run rules and return an `Err`
   to block the save if the data is dirty/invalid.
2. Look up `DataObjectDefinition`. Check transaction state on GDO.
3. For each container touched by `Direct` mappings: build sub-map, call `driver.persist`.
   - **With active transaction:** if any write fails, undo completed writes, call
     `rollback_transaction()`, fire `on_error_save`, return `Err`.
   - **Without transaction:** log failures, continue, fire `after_save` with warning.
4. On full success: update state `Consistent`. Fire `after_save`.

Calculated attributes are skipped.

### delete_data_object

```rust
pub fn delete_data_object(&self, object_id: &str, gdo: &mut GenericDataObject) -> Result<(), OxDataError>
```

1. Fire `before_delete_object`.
2. Call `gdo.delete()` (marks state `Deleted`).
3. Call `gdo.persist()` (calls `driver.delete()` on the backing store).
4. Fire `after_delete_object`. On failure: fire `on_error_delete_object`.

---

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `ox_data_object` | `GenericDataObject`, `AttributeValue` |
| `ox_persistence` | `PERSISTENCE_DRIVER_REGISTRY`, `DataObjectState` |
| `ox_type_converter` | `ValueType`, `CONVERSION_REGISTRY` |
| `ox_callback_manager` | `CALLBACK_MANAGER`, `EventType` |
| `serde` / `serde_json` | Dictionary serialization |
| `anyhow` | Error handling in query engine |
