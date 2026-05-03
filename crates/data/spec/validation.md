# ox_validation — Validation Addon

**Crate:** `ox_validation`
**Type:** library

A standalone addon that validates `GenericDataObject` attributes against a set of rules
before a save operation. Validation rules are defined as composable objects, registered
against a `DataObjectDefinition` name, and executed by calling `validate()` on the GDO.
Validation is entirely separate from the data dictionary — it is not stored on
`DataObjectAttribute`.

---

## ValidationRule Trait

The core interface. Every built-in and custom rule implements this trait.

```rust
pub trait ValidationRule: Send + Sync {
    fn attribute(&self) -> &str;
    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError>;
    fn description(&self) -> &str;
    fn rule_type_name(&self) -> &'static str;
    fn constraint_json(&self) -> serde_json::Value;
}
```

`attribute()` returns the name of the GDO attribute this rule applies to.
`validate()` receives the full GDO so rules can cross-reference other attributes.
`rule_type_name()` returns a stable lowercase string identifier (e.g. `"min_length"`).
`constraint_json()` returns structured rule parameters for introspection and form
rendering (e.g. `{"min": 8}`). Returns `Value::Null` for parameter-free rules.
See [spec/introspection.md](introspection.md) for the full rule-type table.

---

## ValidationError

```rust
pub struct ValidationError {
    pub attribute: String,
    pub rule: String,        // rule type name, e.g. "required", "min_length"
    pub message: String,     // human-readable description
}
```

---

## ValidationResult

```rust
pub struct ValidationResult {
    pub errors: Vec<ValidationError>,
}

impl ValidationResult {
    pub fn is_valid(&self) -> bool { self.errors.is_empty() }
}
```

---

## Built-in Rules

### Required

```rust
pub struct Required { pub attribute: String, pub message: Option<String> }
```

Fails if the attribute is absent, or its value is an empty string.

### MinLength / MaxLength

```rust
pub struct MinLength { pub attribute: String, pub min: usize, pub message: Option<String> }
pub struct MaxLength { pub attribute: String, pub max: usize, pub message: Option<String> }
```

Fails if the string representation of the attribute value has fewer than `min` or more
than `max` UTF-8 characters.

### Min / Max

```rust
pub struct Min { pub attribute: String, pub min: f64, pub message: Option<String> }
pub struct Max { pub attribute: String, pub max: f64, pub message: Option<String> }
```

Parses the attribute value as `f64`. Fails if it is less than `min` or greater than
`max`. Fails with a type error if the value cannot be parsed as a number.

### Range

```rust
pub struct Range { pub attribute: String, pub min: f64, pub max: f64, pub message: Option<String> }
```

Combined `Min` and `Max` check in a single rule.

### Regex

```rust
pub struct Regex { pub attribute: String, pub pattern: String, pub message: Option<String> }
```

Compiles the pattern at rule construction time. Fails if the attribute value does not
match the full pattern. Construction fails (returns `Err`) if the pattern is invalid.

### OneOf

```rust
pub struct OneOf { pub attribute: String, pub values: Vec<String>, pub message: Option<String> }
```

Fails if the attribute's string value is not in `values`.

### NotOneOf

```rust
pub struct NotOneOf { pub attribute: String, pub values: Vec<String>, pub message: Option<String> }
```

Fails if the attribute's string value is in `values`.

### Matches

```rust
pub struct Matches { pub attribute: String, pub other_attribute: String, pub message: Option<String> }
```

Fails if the attribute's value does not equal the value of `other_attribute`. Used for
password confirmation checks.

### Custom

```rust
pub struct Custom {
    pub attribute: String,
    pub description: String,
    pub rule_fn: Arc<dyn Fn(&GenericDataObject) -> Result<(), String> + Send + Sync>,
}
```

Arbitrary validation logic supplied as a closure. The closure returns `Ok(())` on success
or `Err(message)` on failure.

---

## ValidationSet

A named collection of rules for a single object type.

```rust
pub struct ValidationSet {
    pub object_id: String,
    pub rules: Vec<Box<dyn ValidationRule>>,
}

impl ValidationSet {
    pub fn new(object_id: &str) -> Self;
    pub fn add_rule(&mut self, rule: Box<dyn ValidationRule>) -> &mut Self;
    pub fn validate(&self, gdo: &GenericDataObject) -> ValidationResult;
}
```

`validate()` runs every rule and collects all failures — it does not stop at the first
error. Returns a `ValidationResult` containing all `ValidationError`s.

---

## ValidationRegistry

Global registry mapping `object_id` to a `ValidationSet`. Thread-safe.

```rust
lazy_static! {
    pub static ref VALIDATION_REGISTRY: Mutex<ValidationRegistry> = ...;
}

pub fn register_validation_set(set: ValidationSet);
pub fn validate(object_id: &str, gdo: &GenericDataObject) -> ValidationResult;
pub fn unregister_validation_set(object_id: &str);
```

---

## Validation Callback Events

Validation fires through the GDO's two-level callback dispatch.

| Event | Fired by | Notes |
|-------|----------|-------|
| `before_validate` | `validate()` | — |
| `after_validate` | `validate()`, when valid | — |
| `on_error_validate` | `validate()`, when invalid | `params.error` contains serialized errors |

---

## Validatable Trait

Implemented on `GenericDataObject`.

```rust
pub trait Validatable {
    fn validate(&self, object_id: &str) -> ValidationResult;
}
```

Looks up the `ValidationSet` for `object_id` in `VALIDATION_REGISTRY`, fires callbacks,
runs all rules, and returns the result.

---

## Usage Pattern

```rust
// Define rules
let mut set = ValidationSet::new("user");
set.add_rule(Box::new(Required { attribute: "email".into(), message: None }))
   .add_rule(Box::new(Regex {
       attribute: "email".into(),
       pattern: r"^[^@]+@[^@]+\.[^@]+$".into(),
       message: Some("Invalid email format".into()),
   }))
   .add_rule(Box::new(MinLength { attribute: "password".into(), min: 8, message: None }))
   .add_rule(Box::new(Matches {
       attribute: "password_confirm".into(),
       other_attribute: "password".into(),
       message: Some("Passwords do not match".into()),
   }));

register_validation_set(set);

// Validate before save
let result = gdo.validate("user");
if !result.is_valid() {
    // handle errors
}
```

---

## Integration with DataObjectManager

`DataObjectManager::save_data_object` does not run validation internally. Callers are
expected to call `gdo.validate(object_id)` and check `result.is_valid()` before calling
`save_data_object`. This keeps validation optional and composable.

---

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `ox_data_object` | `GenericDataObject`, `CallbackManager` |
| `ox_callback_manager` | Event dispatch |
| `lazy_static` | `VALIDATION_REGISTRY` |
| `regex` | `Regex` rule pattern compilation |
| `serde_json` | `constraint_json() -> serde_json::Value` on `ValidationRule` |
