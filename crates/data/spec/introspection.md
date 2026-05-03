# ox_introspection — Object Schema Introspection

**Crate:** `ox_introspection`
**Type:** library

Assembles a complete, structured description of a `GenericDataObject`'s schema — field
definitions, value types, validation rules, and relationship definitions — from the three
sources of truth: the GDO's current attribute state, the `DataDictionary`, and the
`VALIDATION_REGISTRY`. The forms module consumes this to render forms without any
manual field specification.

---

## Design

Introspection is read-only and fires no callbacks. It does not modify the GDO, the
dictionary, or the validation registry. All assembly happens at call time from the
current state of each source — there is no caching.

A `describe_object` call requires a live GDO instance and populates `current_value` for
each field. A `describe_schema` call requires only a `DataObjectDefinition` and produces
the same structure without current values — useful for rendering empty "create" forms.

---

## Types

### MappingKind

```rust
pub enum MappingKind {
    Direct,
    Calculated,
}
```

Derived from `AttributeMapping` on the `DataObjectAttribute`. Calculated fields are
rendered read-only in forms.

---

### RuleDescriptor

A portable description of a single validation rule for a field.

```rust
pub struct RuleDescriptor {
    pub rule_type: String,            // "required", "min_length", "regex", etc.
    pub description: String,          // from ValidationRule::description()
    pub constraint: serde_json::Value, // rule-specific parameters (see table below)
}
```

**`rule_type` values and their `constraint` shape:**

| `rule_type` | `constraint` |
|-------------|-------------|
| `required` | `null` |
| `min_length` | `{"min": 8}` |
| `max_length` | `{"max": 255}` |
| `min` | `{"min": 0.0}` |
| `max` | `{"max": 100.0}` |
| `range` | `{"min": 0.0, "max": 100.0}` |
| `regex` | `{"pattern": "^[^@]+@[^@]+\\.[^@]+$"}` |
| `one_of` | `{"values": ["active", "inactive"]}` |
| `not_one_of` | `{"values": ["banned"]}` |
| `matches` | `{"other_attribute": "password"}` |
| `custom` | `null` |

---

### FieldDescriptor

A complete per-field description.

```rust
pub struct FieldDescriptor {
    pub name: String,
    pub data_type: ValueType,
    pub parameters: HashMap<String, String>,   // value_type_parameters from AttributeValue
    pub description: Option<String>,           // from DataObjectAttribute
    pub mapping_kind: MappingKind,
    pub is_required: bool,                     // true if a Required rule exists for this field
    pub is_readonly: bool,                     // true when mapping_kind == Calculated
    pub current_value: Option<String>,         // coerced string value from GDO; None for schema-only calls
    pub validation_rules: Vec<RuleDescriptor>,
}
```

`is_required` is derived from the presence of a `Required` rule in `validation_rules` —
it is a convenience shortcut so form renderers do not need to search the rule list.

`is_readonly` mirrors `MappingKind::Calculated` — it is `true` when the field has no
backing store write path.

---

### RelationshipDescriptor

```rust
pub struct RelationshipDescriptor {
    pub id: String,
    pub name: String,                 // from RelationshipDefinition::name
    pub from_container_id: String,
    pub to_container_id: String,
    pub cardinality: Cardinality,     // re-exported from ox_data_object_manager
    pub join_type: JoinType,          // re-exported from ox_data_object_manager
}
```

---

### ObjectSchema

The top-level introspection result.

```rust
pub struct ObjectSchema {
    pub object_id: String,
    pub name: String,
    pub description: Option<String>,
    pub fields: Vec<FieldDescriptor>,
    pub relationships: Vec<RelationshipDescriptor>,
}
```

`fields` preserves the attribute order from `DataObjectDefinition::attributes`. Fields
present in the GDO but absent from the definition appear at the end, with
`mapping_kind: MappingKind::Direct`, no description, and empty `validation_rules`.

---

## Functions

### `describe_object`

Assembles `ObjectSchema` from a live GDO instance. Populates `current_value` for each
field.

```rust
pub fn describe_object(
    object_id: &str,
    gdo: &GenericDataObject,
    dictionary: &DataDictionary,
) -> Result<ObjectSchema, IntrospectionError>
```

**Assembly steps:**

1. Look up `DataObjectDefinition` in `dictionary.objects` by `object_id`. Return
   `IntrospectionError::DefinitionNotFound` if absent.
2. For each `DataObjectAttribute` in the definition:
   a. Read `current_value` from `gdo.attribute_value_string(attr.name)`.
   b. Read `parameters` from `gdo.attribute_parameters(attr.name)`.
   c. Look up `ValidationSet` in `VALIDATION_REGISTRY` for `object_id`.
      Collect all `ValidationRule`s whose `attribute()` matches `attr.name`.
      Convert each to a `RuleDescriptor` (see Rule Conversion below).
   d. Derive `is_required`, `is_readonly`, `mapping_kind`.
   e. Build `FieldDescriptor`.
3. For GDO attributes not in the definition: append a `FieldDescriptor` with
   `mapping_kind: Direct`, `description: None`, `validation_rules: []`.
4. For each `RelationshipDefinition` in the definition, build a `RelationshipDescriptor`.
5. Return `ObjectSchema`.

No callbacks are fired. The function is `&self`-safe on all inputs.

---

### `describe_schema`

Assembles `ObjectSchema` from the definition and validation registry alone — no GDO
instance required. All `FieldDescriptor::current_value` fields are `None`.

```rust
pub fn describe_schema(
    object_id: &str,
    dictionary: &DataDictionary,
) -> Result<ObjectSchema, IntrospectionError>
```

Same assembly as `describe_object` steps 1, 2c–2e, 4–5. Skips step 2a, 2b, and step 3
(no GDO attribute scan).

---

## Rule Conversion

When building `RuleDescriptor` from a `ValidationRule`:

```rust
fn rule_to_descriptor(rule: &dyn ValidationRule) -> RuleDescriptor {
    let rule_type = /* derived from rule type name */;
    let description = rule.description().to_string();
    let constraint = match rule_type.as_str() {
        "required"    => Value::Null,
        "min_length"  => json!({"min": rule.min}),
        "max_length"  => json!({"max": rule.max}),
        "min"         => json!({"min": rule.min}),
        "max"         => json!({"max": rule.max}),
        "range"       => json!({"min": rule.min, "max": rule.max}),
        "regex"       => json!({"pattern": rule.pattern}),
        "one_of"      => json!({"values": rule.values}),
        "not_one_of"  => json!({"values": rule.values}),
        "matches"     => json!({"other_attribute": rule.other_attribute}),
        _             => Value::Null,
    };
    RuleDescriptor { rule_type, description, constraint }
}
```

`rule_type` is derived by downcasting the `&dyn ValidationRule` to concrete types using
`Any`. Each built-in rule type registers itself in `ox_validation` with a stable string
identifier returned by a provided `rule_type_name() -> &'static str` method added to the
`ValidationRule` trait (see Trait Extension below).

---

## Trait Extension

`ox_introspection` requires one addition to the `ValidationRule` trait in `ox_validation`:

```rust
pub trait ValidationRule: Send + Sync {
    fn attribute(&self) -> &str;
    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError>;
    fn description(&self) -> &str;
    fn rule_type_name(&self) -> &'static str;    // new — stable identifier
    fn constraint_json(&self) -> serde_json::Value;  // new — structured parameters
}
```

Each built-in rule implements these:

| Rule | `rule_type_name()` | `constraint_json()` |
|------|--------------------|---------------------|
| `Required` | `"required"` | `Value::Null` |
| `MinLength` | `"min_length"` | `{"min": self.min}` |
| `MaxLength` | `"max_length"` | `{"max": self.max}` |
| `Min` | `"min"` | `{"min": self.min}` |
| `Max` | `"max"` | `{"max": self.max}` |
| `Range` | `"range"` | `{"min": self.min, "max": self.max}` |
| `Regex` | `"regex"` | `{"pattern": self.pattern.clone()}` |
| `OneOf` | `"one_of"` | `{"values": self.values.clone()}` |
| `NotOneOf` | `"not_one_of"` | `{"values": self.values.clone()}` |
| `Matches` | `"matches"` | `{"other_attribute": self.other_attribute.clone()}` |
| `Custom` | `"custom"` | `Value::Null` |

Rule conversion in `describe_object` / `describe_schema` calls `rule_type_name()` and
`constraint_json()` directly — no downcasting required.

---

## Error Type

```rust
pub enum IntrospectionError {
    DefinitionNotFound(String),    // object_id not in dictionary
}

impl fmt::Display for IntrospectionError { ... }
```

---

## Usage Example

```rust
use ox_introspection::{describe_object, describe_schema};

// Render a form for an existing object
let schema = describe_object("user", &gdo, &manager.dictionary)?;
for field in &schema.fields {
    if field.is_readonly { continue; }
    render_field(
        &field.name,
        &field.data_type,
        field.current_value.as_deref(),
        field.is_required,
        &field.validation_rules,
    );
}

// Render an empty "create" form — no GDO needed
let schema = describe_schema("user", &manager.dictionary)?;
```

---

## Integration with the Forms Module

The forms module (`ox_forms_api`) calls `describe_schema` to render creation forms and
`describe_object` to render edit forms. It maps `FieldDescriptor` properties to HTML
input attributes:

| `FieldDescriptor` field | HTML behavior |
|------------------------|---------------|
| `data_type` | `<input type="...">` selection |
| `is_required` | `required` attribute |
| `is_readonly` | `readonly` / disabled |
| `current_value` | `value="..."` pre-fill |
| `rule_type: "min_length"` | `minlength="N"` |
| `rule_type: "max_length"` | `maxlength="N"` |
| `rule_type: "min"` / `"max"` | `min="N"` / `max="N"` |
| `rule_type: "regex"` | `pattern="..."` |
| `rule_type: "one_of"` | `<select>` with option list |
| `description` | `<label>` / tooltip |

Relationships in `ObjectSchema::relationships` drive the rendering of linked-object
pickers (dropdown or search-as-you-type) where `cardinality` determines whether the
picker is single-select or multi-select.

### Real-Time Field Updates

For edit forms on objects that may be modified by other users, `ox_forms_api` connects to
the broker's WebSocket endpoint and subscribes to the object being edited:

```
GET /data/listen  (WebSocket upgrade)
→ send: { "subscribe": ["<object-id>"] }
← recv: { "object_id": "…", "attribute": "email", "value": "new@…", "event": "after_set" }
```

On receiving an `after_set` event, the form module updates the field value in the UI if
the field is not currently being edited by the local user. On receiving an `after_commit`
event (transaction closed), the form re-calls `describe_object` to refresh all field
values.

`ox_forms_api` dependencies added by this integration:

| Crate / library | Purpose |
|-----------------|---------|
| `ox_introspection` | `describe_schema`, `describe_object`, `ObjectSchema`, `FieldDescriptor` |
| WebSocket client (e.g. `tungstenite`) | Connection to `ox_data_broker` `/data/listen` |

---

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `ox_data_object` | `GenericDataObject`, `Introspectable` trait |
| `ox_data_object_manager` | `DataDictionary`, `DataObjectDefinition`, `DataObjectAttribute`, `AttributeMapping`, `RelationshipDefinition`, `Cardinality`, `JoinType` |
| `ox_validation` | `VALIDATION_REGISTRY`, `ValidationRule`, `ValidationSet` |
| `serde_json` | `RuleDescriptor::constraint` |
