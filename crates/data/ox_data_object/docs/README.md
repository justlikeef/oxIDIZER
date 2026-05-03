# ox_data_object

Core GDO library. `GenericDataObject` is a typed, callback-aware attribute container
with UUID-based identity. It knows nothing about persistence, locking, or validation —
those are addons that hook in via the callback system and extension slot.

---

## GenericDataObject

```rust
pub struct GenericDataObject {
    attributes: HashMap<String, AttributeValue>,
    extensions: HashMap<String, serde_json::Value>,
    callbacks: CallbackManager,
    pub identifier_name: String,
}
```

Construction: `GenericDataObject::new(identifier_name, id_option)` — stores
`id.unwrap_or_else(Uuid::new_v4)` under `identifier_name`. `Default` impl uses `"id"`.

---

## Callback System

Every data manipulation fires a two-level dispatch:
1. Per-object `callbacks` (private `CallbackManager`)
2. Global `CALLBACK_MANAGER` (`lazy_static Mutex<CallbackManager>`)

Per-object fires first. If any callback returns `Err`, dispatch stops.

```rust
pub type CallbackFn = Arc<dyn Fn(&mut GenericDataObject, &CallbackParams) -> Result<(), String> + Send + Sync>;
```

Event types follow the naming convention `before_*` / `after_*` / `on_error_*`.
`after_*` fires only on success. `on_error_*` fires on failure with `params.error` set.

### Core Events

| Event | Fired by |
|---|---|
| `before_get` / `after_get` / `on_error_get` | `get`, `get_raw_value`, `get_attribute` |
| `before_set` / `after_set` / `on_error_set` | `set`, `set_with_type`, `set_attribute_value`, `set_attributes` |
| `before_remove` / `after_remove` / `on_error_remove` | `remove_attribute` |
| `before_clear` / `after_clear` / `on_error_clear` | `clear_attributes` |

---

## AttributeValue

Holds a `Box<dyn Any + Send + Sync>` value, its `ValueType` tag, and optional conversion
parameters. Key methods: `get_value<T>()`, `is<T>()`, `to_string()` (coerces to string
for storage/comparison).

---

## Reading and Writing

| Method | Notes |
|---|---|
| `get<T>(name)` | Downcast first; falls back to `ConversionRegistry` if types differ |
| `get_raw_value<T>(name)` | Downcast only, no conversion |
| `set<T>(name, value)` | Infers `ValueType`; fires `before_set` / `after_set` |
| `set_with_type<T>(name, value, value_type, params)` | Explicit type + parameters |
| `remove_attribute(name)` | Fires `before_remove` / `after_remove` |
| `clear_attributes()` | Fires `before_clear` / `after_clear` |

---

## Introspection (no callbacks)

| Method | Notes |
|---|---|
| `has_attribute(name)` | Existence check |
| `get_attribute_names()` | All attribute keys |
| `attribute_type(name)` | Stored `ValueType` |
| `attribute_parameters(name)` | `value_type_parameters` |
| `attribute_value_string(name)` | Current value coerced to string |

The `Introspectable` trait exposes these methods. `ox_forms_api` programs against
`Introspectable`, not the concrete GDO type.

---

## Extension Slot

`extensions: HashMap<String, serde_json::Value>` — opaque data store for addons.
Access does not fire callbacks. Keys are namespaced by addon (e.g., `"ox.persistence"`,
`"ox.transaction"`). GDO does not read or interpret extension values.

---

## Serialization

`to_serializable_map()` → `HashMap<String, (String, ValueType, HashMap<String, String>)>`

Maps every attribute to `(coerced_string_value, value_type, parameters)`. Extensions are
included under `"__extensions__"` as a JSON-encoded string. Does not fire callbacks.

`from_serializable_map(map, identifier_name)` — restores attributes via
`ConversionRegistry`. Does not fire callbacks.

---

## Dependencies

| Crate | Purpose |
|---|---|
| `ox_type_converter` | `ValueType`, `ConversionRegistry` |
| `ox_callback_manager` | `CallbackManager`, `CALLBACK_MANAGER` |
| `uuid` | UUID generation |
| `serde_json` | Extension slot storage |
