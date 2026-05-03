# ox_validation

Validation addon for `GenericDataObject`. Defines composable validation rules,
`ValidationSet` collections, and a global `VALIDATION_REGISTRY`. Entirely separate from
the data dictionary and from `DataObjectManager`.

---

## ValidationRule Trait

```rust
pub trait ValidationRule: Send + Sync {
    fn attribute(&self) -> &str;
    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError>;
    fn description(&self) -> &str;
    fn rule_type_name(&self) -> &'static str;
    fn constraint_json(&self) -> serde_json::Value;
}
```

`validate()` receives the full GDO — rules can cross-reference other attributes.
`constraint_json()` returns rule parameters for form rendering (e.g., `{"min": 8}`).

---

## Built-in Rules

| Rule | Type name | Description |
|---|---|---|
| `Required` | `"required"` | Attribute must be present and non-empty |
| `MinLength` | `"min_length"` | String length ≥ `min` |
| `MaxLength` | `"max_length"` | String length ≤ `max` |
| `Min` | `"min"` | Numeric value ≥ `min` (f64) |
| `Max` | `"max"` | Numeric value ≤ `max` (f64) |
| `Range` | `"range"` | Numeric value between `min` and `max` |
| `Regex` | `"regex"` | Value matches compiled regex pattern |
| `OneOf` | `"one_of"` | Value is in a list of allowed strings |
| `NotOneOf` | `"not_one_of"` | Value is not in a list of blocked strings |
| `Matches` | `"matches"` | Value equals another attribute's value |
| `Custom` | `"custom"` | Arbitrary closure returning `Ok(())` or `Err(message)` |

---

## ValidationSet

```rust
pub struct ValidationSet {
    pub object_id: String,
    pub rules: Vec<Box<dyn ValidationRule>>,
}
```

`validate(gdo)` runs all rules and collects all failures (does not stop at first error).
Returns `ValidationResult { errors: Vec<ValidationError> }`.

---

## VALIDATION_REGISTRY

```rust
lazy_static! {
    pub static ref VALIDATION_REGISTRY: Mutex<ValidationRegistry> = ...;
}
```

Functions: `register_validation_set(set)`, `validate(object_id, gdo)`,
`unregister_validation_set(object_id)`.

---

## Validatable Trait

```rust
pub trait Validatable {
    fn validate(&self, object_id: &str) -> ValidationResult;
}
```

Implemented on `GenericDataObject`. Looks up `ValidationSet` in `VALIDATION_REGISTRY`,
fires `before_validate` / `after_validate` / `on_error_validate` callbacks, runs all
rules, returns result.

---

## Integration with DataObjectManager

`DataObjectManager::save_data_object` does NOT run validation automatically. The
recommended pattern is to call `gdo.validate(object_id)` before `save_data_object` and
check `result.is_valid()`. Alternatively, register a `before_save` callback that calls
`validate` and returns `Err` if invalid.

---

## Example

```rust
let mut set = ValidationSet::new("user");
set.add_rule(Box::new(Required { attribute: "email".into(), message: None }))
   .add_rule(Box::new(Regex {
       attribute: "email".into(),
       pattern: r"^[^@]+@[^@]+\.[^@]+$".into(),
       message: None,
   }))
   .add_rule(Box::new(MinLength { attribute: "password".into(), min: 8, message: None }));

register_validation_set(set);

let result = gdo.validate("user");
if !result.is_valid() {
    for err in &result.errors {
        eprintln!("[{}] {}: {}", err.attribute, err.rule, err.message);
    }
}
```
