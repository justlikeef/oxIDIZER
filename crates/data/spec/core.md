# ox_data_object — Core Data Object

**Crate:** `ox_data_object`
**Type:** library

`GenericDataObject` is a typed, callback-aware attribute container with a UUID-based
identity. Its only responsibilities are storing named values, converting them between
types on access, firing callbacks on every data manipulation, and serializing itself to
and from a portable map format. It knows nothing about where data comes from or who holds
a lock on it — those concerns belong to addon systems that build on top of it.

---

## Callback System

Every data manipulation call — in `GenericDataObject` and in every addon — fires a
`before_*` callback before the operation and an `after_*` or `on_error_*` callback after
it. There is no silent path for data access or mutation.

### CallbackFn

```rust
pub type CallbackFn = Arc<dyn Fn(&mut GenericDataObject, &CallbackParams) -> Result<(), String> + Send + Sync>;
```

Callbacks receive a mutable reference to the object — they may read or write attributes,
trigger persistence, call addon systems, or anything else. `CallbackParams` carries the
event context.

```rust
pub struct CallbackParams {
    pub event_type: EventType,
    pub attribute: Option<String>,   // attribute name, where applicable
    pub value: Option<String>,       // coerced string value, where applicable
    pub error: Option<String>,       // error message, for on_error_* events only
}
```

### EventType

A string wrapper representing a named event. Constructed from any string; the
`before_*` / `after_*` / `on_error_*` convention is enforced by usage, not by type.

```rust
pub struct EventType(String);

impl EventType {
    pub fn new(s: &str) -> Self;
    pub fn as_str(&self) -> &str;
}
```

### CallbackManager

Holds a list of `(EventType, CallbackFn)` pairs and dispatches them in registration order.

```rust
pub struct CallbackManager {
    callbacks: Vec<(EventType, CallbackFn)>,
}

impl CallbackManager {
    pub fn new() -> Self;
    pub fn register(&mut self, event_type: EventType, callback: CallbackFn);
    pub fn trigger(&mut self, gdo: &mut GenericDataObject, params: &CallbackParams) -> Result<(), String>;
}
```

`trigger` iterates all callbacks whose `EventType` matches `params.event_type` and calls
each in order. If any callback returns `Err`, dispatch stops and the error is returned.

### Two-Level Dispatch

Each `GenericDataObject` owns its own `CallbackManager` for object-specific hooks. A
global `CALLBACK_MANAGER` (`lazy_static Mutex<CallbackManager>`) handles system-wide
hooks such as audit logging or monitoring.

On every operation, the GDO fires its **per-object** callbacks first, then the **global**
callbacks. Both levels receive the same mutable reference to the object and the same
`CallbackParams`.

```rust
// Registration — per-object
gdo.register_callback(EventType::new("before_set"), Arc::new(|gdo, params| { ... }));

// Registration — global (fires for every GDO)
CALLBACK_MANAGER.lock().unwrap().register(EventType::new("after_persist"), Arc::new(|gdo, params| { ... }));
```

### Core Event Types

| Event | Fired by | `attribute` | `value` |
|-------|----------|-------------|---------|
| `before_get` | `get`, `get_raw_value`, `get_attribute` | attribute name | — |
| `after_get` | same | attribute name | returned value (coerced) |
| `on_error_get` | same, on failure | attribute name | — |
| `before_set` | `set`, `set_with_type`, `set_attribute_value`, `set_attributes` | attribute name | new value |
| `after_set` | same | attribute name | — |
| `on_error_set` | same, on failure | attribute name | new value |
| `before_remove` | `remove_attribute` | attribute name | — |
| `after_remove` | same | attribute name | — |
| `on_error_remove` | same, on failure | attribute name | — |
| `before_clear` | `clear_attributes` | — | — |
| `after_clear` | same | — | — |
| `on_error_clear` | same, on failure | — | — |

`after_*` fires only on success. `on_error_*` fires in its place on failure, with
`params.error` set to the error description.

Addon systems define additional event types and fire them through the same two-level
dispatch on the GDO they are operating on.

---

## AttributeValue

A single named attribute: a heap-allocated value, its `ValueType` tag, and optional
conversion parameters.

```rust
pub struct AttributeValue {
    pub value: Box<dyn Any + Send + Sync>,
    pub value_type: ValueType,
    pub value_type_parameters: HashMap<String, String>,
}
```

| Method | Description |
|--------|-------------|
| `new<T>(value, value_type)` | Construct with given type |
| `with_parameters(params)` | Builder: attach conversion parameters |
| `get_value<T>() -> Option<T>` | Downcast to concrete type |
| `is<T>() -> bool` | Type check |
| `type_id() -> TypeId` | Runtime type identity |
| `to_string() -> String` | Coerce to string (String, &str, i32, f64, bool, Debug fallback) |

---

## GenericDataObject

```rust
pub struct GenericDataObject {
    attributes: HashMap<String, AttributeValue>,
    extensions: HashMap<String, serde_json::Value>,
    callbacks: CallbackManager,           // per-object callbacks
    pub identifier_name: String,
}
```

`attributes` holds the object's typed data. `extensions` is an opaque data slot for
addon systems. `callbacks` handles object-specific event hooks; the global
`CALLBACK_MANAGER` handles system-wide hooks.

### Construction

```rust
GenericDataObject::new(identifier_name: &str, id: Option<Uuid>) -> Self
```

Stores `id.unwrap_or_else(Uuid::new_v4)` as a string under `identifier_name`. All other
attributes start empty. No callbacks are fired during construction.

```rust
impl Default for GenericDataObject {
    fn default() -> Self { Self::new("id", None) }
}
```

### Registering Callbacks

```rust
pub fn register_callback(&mut self, event_type: EventType, callback: CallbackFn)
```

Registers on the per-object `CallbackManager`. To register a system-wide callback use
`CALLBACK_MANAGER.lock().unwrap().register(...)`.

### Reading Attributes

All read operations fire `before_get` → read → `after_get` (or `on_error_get`).

| Method | Notes |
|--------|-------|
| `get<T>(identifier) -> Option<T>` | Direct downcast first; falls back to `ConversionRegistry` |
| `get_raw_value<T>(identifier) -> Option<T>` | Downcast only, no conversion |
| `get_attribute(identifier) -> Option<&AttributeValue>` | Raw attribute reference |

### Writing Attributes

All write operations fire `before_set` → write → `after_set` (or `on_error_set`).

| Method | Notes |
|--------|-------|
| `set<T>(identifier, value)` | Infers `ValueType`; returns displaced `AttributeValue` if any |
| `set_with_type<T>(identifier, value, value_type, params)` | Explicit type + parameters |
| `set_attribute_value(identifier, AttributeValue)` | Replace entire `AttributeValue` |
| `set_attributes(HashMap<String, AttributeValue>)` | Bulk replace; fires once for the whole operation |

### Removing Attributes

| Method | Notes |
|--------|-------|
| `remove_attribute(identifier) -> Option<AttributeValue>` | Fires `before_remove` / `after_remove` |
| `clear_attributes()` | Remove all; fires `before_clear` / `after_clear` |

### Introspection

No callbacks fired. These methods expose the GDO's current attribute state — names,
types, parameters, and coerced values — without triggering any `before_*` / `after_*`
hooks. They are the building blocks consumed by the `ox_introspection` crate.

| Method | Notes |
|--------|-------|
| `has_attribute(identifier) -> bool` | Existence check |
| `get_attribute_names() -> Vec<String>` | All attribute keys, in insertion order |
| `attribute_type(identifier) -> Option<ValueType>` | The stored `ValueType` for a field |
| `attribute_parameters(identifier) -> Option<HashMap<String, String>>` | `value_type_parameters` for a field |
| `attribute_value_string(identifier) -> Option<String>` | Current value coerced to string via `AttributeValue::to_string()` |
| `len() -> usize` | Attribute count |
| `is_empty() -> bool` | True if no attributes |

### Introspectable Trait

`GenericDataObject` implements `Introspectable`, defined in `ox_data_object`. This trait
is what `ox_introspection` and `ox_forms_api` program against — they do not depend on
the concrete GDO type.

```rust
pub trait Introspectable {
    fn attribute_names(&self) -> Vec<String>;
    fn attribute_type(&self, name: &str) -> Option<ValueType>;
    fn attribute_parameters(&self, name: &str) -> Option<HashMap<String, String>>;
    fn attribute_value_string(&self, name: &str) -> Option<String>;
}

impl Introspectable for GenericDataObject { ... }
```

No callbacks are fired by any `Introspectable` method.

---

## Extension Slot

`extensions: HashMap<String, serde_json::Value>` is a generic data slot that addon
systems use to store their own state on the object without requiring a wrapper struct.
`GenericDataObject` stores these values as opaque JSON — it does not read, interpret, or
act on them. Extension access does not fire callbacks.

Keys are namespaced strings chosen by each addon (`"ox.persistence"`, `"ox.transaction"`,
etc.). The contents and semantics of each key are defined entirely by the owning addon.

```rust
pub fn get_extension(&self, key: &str) -> Option<&serde_json::Value>
pub fn set_extension(&mut self, key: &str, value: serde_json::Value)
pub fn remove_extension(&mut self, key: &str) -> Option<serde_json::Value>
pub fn extension_keys(&self) -> Vec<&String>
```

---

## Serialization

### `to_serializable_map`

```rust
pub fn to_serializable_map(
    &self,
) -> HashMap<String, (String, ValueType, HashMap<String, String>)>
```

Produces `attribute_name → (coerced_string_value, value_type, parameters)` for every
entry in `attributes`. The `extensions` map is included under the reserved key
`"__extensions__"` as a JSON-encoded string with type `"string"` and no parameters.
Does not fire callbacks.

### `from_serializable_map`

```rust
pub fn from_serializable_map(
    map: HashMap<String, (String, ValueType, HashMap<String, String>)>,
    identifier_name: &str,
) -> Self
```

Restores attributes via `ConversionRegistry`. Restores `extensions` from the
`"__extensions__"` key if present. Does not fire callbacks.

---

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `ox_type_converter` | `ValueType`, `TypeConverter`, `CONVERSION_REGISTRY` |
| `ox_callback_manager` | `CallbackManager`, `CallbackFn`, `CallbackParams`, `EventType`, `CALLBACK_MANAGER` |
| `uuid` | UUID generation for identity |
| `serde_json` | Extension slot storage |
