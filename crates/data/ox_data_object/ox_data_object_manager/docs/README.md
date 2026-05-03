# ox_data_object_manager

Data dictionary and object manager. Adds a metadata layer on top of `GenericDataObject`:
logical object definitions, physical storage mappings, cross-datasource relationships,
and a query engine that resolves them.

---

## Data Dictionary

### DataDictionary

Root container for metadata. Serializes to/from JSON.

```rust
pub struct DataDictionary {
    pub containers: HashMap<String, DataStoreContainer>,
    pub objects: HashMap<String, DataObjectDefinition>,
}
```

Methods: `add_container`, `add_object`, `merge_container`, `save_to_file`, `load_from_file`.

### DataStoreContainer

Describes one physical storage unit:

```rust
pub struct DataStoreContainer {
    pub id: String,
    pub datasource_id: String,      // driver name in PERSISTENCE_DRIVER_REGISTRY
    pub name: String,               // physical name: table name, file path, etc.
    pub container_type: String,     // "table" | "view" | "file" | "key"
    pub fields: Vec<DataStoreField>,
    pub metadata: HashMap<String, String>,
}
```

### DataObjectDefinition

Logical object type. Maps attribute names to physical fields.

```rust
pub struct DataObjectDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub attributes: Vec<DataObjectAttribute>,
    pub relationships: Vec<RelationshipDefinition>,
}
```

### AttributeMapping

```rust
pub enum AttributeMapping {
    Direct    { container_id: String, field_name: String },
    Calculated { expression: String },   // template: "{first_name} {last_name}"
}
```

`Calculated` attributes are read-only (skipped on save). Evaluated after all `Direct`
attributes are loaded.

---

## DataObjectManager

```rust
pub struct DataObjectManager {
    pub dictionary: DataDictionary,
}
```

| Method | Description |
|---|---|
| `create_data_object(identifier_name)` | New GDO, state `New`, fresh UUID |
| `load_data_object(object_id, id)` | Load from backing store(s) using dictionary |
| `save_data_object(object_id, gdo)` | Write to all containers touched by `Direct` mappings |
| `delete_data_object(object_id, gdo)` | Mark deleted and remove from backing store |

### save_data_object and Transactions

- **With active transaction:** if any write fails, completed writes are reversed and
  `rollback_transaction()` is called. Returns `Err`.
- **Without transaction:** failures are logged; execution continues. Returns success
  with warnings.

### Callback Events

| Event | Fired by |
|---|---|
| `before_load` / `after_load` / `on_error_load` | `load_data_object` |
| `before_save` / `after_save` / `on_error_save` | `save_data_object` |
| `before_delete_object` / `after_delete_object` / `on_error_delete_object` | `delete_data_object` |

The `before_save` event is the hook point for validation — register a callback that calls
`gdo.validate(object_id)` and returns `Err` to abort the save.

---

## Relationships

`RelationshipDefinition` describes a cross-container join:
- `OneToOne`, `OneToMany`, `ManyToMany { junction_container_id }`
- `JoinType`: `Inner`, `Left`, `Right`, `Outer`
- `conditions: Vec<JoinCondition>` — field equality pairs

`DataObjectManager` uses relationships to build multi-container `QueryPlan`s
automatically during `load_data_object`.

---

## Query Engine

`QueryEngine` executes `QueryPlan` trees across multiple datasources:

```
QueryNode::Fetch { container_id, location, filters }
QueryNode::Join  { left, right, join_type, conditions }
```

Plans are built automatically by `DataObjectManager.load_data_object`. For manual query
construction:
- One `Fetch` per container (filters are equality conjunctions)
- Multiple containers wrapped in `Join` nodes
- Empty `filters` returns all records (subject to `options.limit`)

Results are `Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>` — flat
row maps that `DataObjectManager` maps back to GDO attributes.
