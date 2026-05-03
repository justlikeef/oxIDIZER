# Query Engine

**Module:** `ox_data_object_manager::query`
**Type:** module within `ox_data_object_manager` library

Executes cross-datasource queries expressed as a tree of `QueryNode`s. Each leaf node
fetches from a single datasource container; interior nodes join two sub-results. The
engine operates entirely through the `PersistenceDriver` trait — no SQL is generated
directly.

---

## QueryOptions

Standardized parameters for pagination and sorting, passed to drivers.

```rust
pub struct QueryOptions {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub sort_field: Option<String>,
    pub sort_direction: SortDirection,
}

pub enum SortDirection {
    Ascending,
    Descending,
}
```

---

## QueryPlan

```rust
pub struct QueryPlan {
    pub root: QueryNode,
    pub options: QueryOptions,
}
```

The plan is a tree with one root. Leaf nodes are `Fetch`; interior nodes are `Join`.

---

## QueryNode

```rust
pub enum QueryNode {
    Fetch {
        container_id: String,
        datasource_id: String,  // driver name in PERSISTENCE_DRIVER_REGISTRY
        location: String,       // physical location passed to the driver (table, path, …)
        filters: HashMap<String, String>,  // attribute_name → value, equality conjunctions
    },
    Join {
        left: Box<QueryNode>,
        right: Box<QueryNode>,
        join_type: JoinType,
        conditions: Vec<JoinCondition>,
    },
}
```

---

## QueryEngine

Stateless. One instance per execution; created inline by `DataObjectManager`.

```rust
pub struct QueryEngine;

impl QueryEngine {
    pub fn new() -> Self;

    pub fn execute_plan(
        &self,
        plan: &QueryPlan,
    ) -> Result<Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>, OxDataError>;
}
```

Each result row is a flat map of `field_name → (string_value, value_type, parameters)`.

### Fetch execution

1. Look up driver in `PERSISTENCE_DRIVER_REGISTRY` by `datasource_id`.
2. Convert the `filters: HashMap<String, String>` to a typed filter map:
   each entry becomes `(value_str, ValueType::String, {})`. Drivers perform equality
   matching.
3. Pass `plan.options` to the driver.
4. Call `driver.fetch(&typed_filter, location, &plan.options)` → `Vec<String>` (IDs).
5. For each ID: call `driver.restore(location, id)` → typed row map.
6. Return all rows.

Empty `filters` returns all records from the container (subject to `limit`/`offset`).

### Join execution

| JoinType | Behaviour |
|----------|-----------|
| `Inner` | Include only rows where conditions match on both sides |
| `Left` | Include all left rows; right fields are absent if no match |
| `Right` | Include all right rows; left fields are absent if no match |
| `Outer` | Include all rows from both sides; unmatched fields are absent |

`Right` and `Outer` are implemented as symmetric inverses of `Left`.

**Condition matching:** all `JoinCondition` entries must match (AND semantics).
`operator` is always `"="` (string equality on the coerced value).

**Merge:** when two rows are joined, right-side fields overwrite left-side fields on key
collision. Use distinct field names to avoid collisions.

---

## Filter Semantics

The `filters` field in `QueryNode::Fetch` represents equality filters:
- `{"status": "active", "tenant_id": "abc"}` → `WHERE status = 'active' AND tenant_id = 'abc'`
- `{}` → no filter (all records)

Filters use string values. The driver converts them to the appropriate storage type using
the field's `ValueType` from `describe_dataset`.

For range queries, complex logic, or driver-specific extensions, use `driver.call_action("query", params)`.

---

## Building Plans from DataObjectDefinition

`DataObjectManager::load_data_object` builds plans automatically:

1. **Single container:** if all `Direct` attributes map to the same container, create a
   single `Fetch` node with `filters = {"id": requested_id}`.

2. **Multiple containers:** create one `Fetch` per distinct container, then wrap them in
   `Join` nodes following the object's `RelationshipDefinition` list. The first container
   (containing the `"id"` attribute) is always the left node of the outermost join.

3. **Calculated attributes:** not included in the plan. Evaluated after plan execution.

---

## Result Structure

All results are `Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>`.
Each map entry represents one field in one row. `DataObjectManager` maps these back to
`GenericDataObject` attributes using the `DataObjectDefinition`'s attribute list.

Field names in the result are the physical field names from the container. The
`DataObjectManager` applies the `name → field_name` mapping from `DataObjectAttribute`
to produce the logical attribute names on the GDO.
