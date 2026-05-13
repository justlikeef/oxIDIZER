# ox_validation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `ox_validation` crate — a rule-based validation addon for `GenericDataObject` with built-in rules, composable sets, a global registry, and GDO integration.

**Architecture:** `ValidationRule` is a trait; built-in rules are structs that implement it. A `ValidationSet` owns a `Vec<Box<dyn ValidationRule>>` for one object type and collects all errors in one pass. A global `VALIDATION_REGISTRY` maps object IDs to sets. The `Validatable` trait is implemented on `GenericDataObject`, firing callbacks through the GDO's existing two-level callback dispatch before and after validation.

**Tech Stack:** Rust, `ox_data_object` (GenericDataObject), `ox_data_error` (OxDataError), `ox_callback_manager`, `lazy_static`, `regex`, `serde_json`

---

## File Structure

```
crates/data/ox_validation/
  Cargo.toml               — add all deps
  src/
    lib.rs                 — pub mod declarations + re-exports
    error.rs               — ValidationError, ValidationResult
    rule.rs                — ValidationRule trait
    rules/
      mod.rs               — pub use of each rule
      required.rs          — Required
      length.rs            — MinLength, MaxLength
      numeric.rs           — Min, Max, Range
      regex_rule.rs        — Regex rule
      one_of.rs            — OneOf, NotOneOf
      matches_rule.rs      — Matches
      custom.rs            — Custom
    set.rs                 — ValidationSet
    registry.rs            — VALIDATION_REGISTRY, register/validate/unregister functions
    validatable.rs         — Validatable trait + impl on GenericDataObject
  tests/
    integration.rs         — end-to-end tests for all rules and GDO integration
```

---

## Task 1: Core types and ValidationRule trait

**Files:**
- Modify: `crates/data/ox_validation/Cargo.toml`
- Create: `crates/data/ox_validation/src/error.rs`
- Create: `crates/data/ox_validation/src/rule.rs`
- Modify: `crates/data/ox_validation/src/lib.rs`
- Create: `crates/data/ox_validation/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

Create `crates/data/ox_validation/tests/integration.rs`:

```rust
use ox_validation::error::{ValidationError, ValidationResult};
use ox_validation::rule::ValidationRule;
use ox_data_object::GenericDataObject;
use serde_json::json;

// A minimal stub rule for testing the trait itself.
struct AlwaysPass;

impl ValidationRule for AlwaysPass {
    fn attribute(&self) -> &str { "any" }
    fn validate(&self, _gdo: &GenericDataObject) -> Result<(), ValidationError> { Ok(()) }
    fn description(&self) -> &str { "Always passes" }
    fn rule_type_name(&self) -> &'static str { "always_pass" }
    fn constraint_json(&self) -> serde_json::Value { serde_json::Value::Null }
}

struct AlwaysFail { pub attr: String }

impl ValidationRule for AlwaysFail {
    fn attribute(&self) -> &str { &self.attr }
    fn validate(&self, _gdo: &GenericDataObject) -> Result<(), ValidationError> {
        Err(ValidationError {
            attribute: self.attr.clone(),
            rule: "always_fail".to_string(),
            message: "forced failure".to_string(),
        })
    }
    fn description(&self) -> &str { "Always fails" }
    fn rule_type_name(&self) -> &'static str { "always_fail" }
    fn constraint_json(&self) -> serde_json::Value { serde_json::Value::Null }
}

#[test]
fn validation_result_is_valid_when_empty() {
    let result = ValidationResult { errors: vec![] };
    assert!(result.is_valid());
}

#[test]
fn validation_result_invalid_when_has_errors() {
    let result = ValidationResult {
        errors: vec![ValidationError {
            attribute: "name".to_string(),
            rule: "required".to_string(),
            message: "required".to_string(),
        }],
    };
    assert!(!result.is_valid());
}

#[test]
fn validation_rule_trait_object_safe() {
    let rules: Vec<Box<dyn ValidationRule>> = vec![
        Box::new(AlwaysPass),
        Box::new(AlwaysFail { attr: "x".to_string() }),
    ];
    assert_eq!(rules[0].rule_type_name(), "always_pass");
    assert_eq!(rules[1].rule_type_name(), "always_fail");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_validation 2>&1 | head -20
```
Expected: FAIL — `ox_validation::error` and `ox_validation::rule` not defined.

- [ ] **Step 3: Update `Cargo.toml`**

Replace `crates/data/ox_validation/Cargo.toml` with:

```toml
[package]
name = "ox_validation"
version = "0.1.0"
edition = "2024"

[dependencies]
ox_data_object    = { path = "../ox_data_object" }
ox_data_error     = { path = "../ox_data_error" }
ox_callback_manager = { path = "../../util/ox_callback_manager" }
lazy_static       = "1"
regex             = "1"
serde_json        = "1"

[dev-dependencies]
ox_type_converter = { path = "../ox_type_converter" }
```

- [ ] **Step 4: Create `src/error.rs`**

```rust
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub attribute: String,
    pub rule: String,
    pub message: String,
}

#[derive(Debug)]
pub struct ValidationResult {
    pub errors: Vec<ValidationError>,
}

impl ValidationResult {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}
```

- [ ] **Step 5: Create `src/rule.rs`**

```rust
use ox_data_object::GenericDataObject;
use crate::error::ValidationError;

pub trait ValidationRule: Send + Sync {
    fn attribute(&self) -> &str;
    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError>;
    fn description(&self) -> &str;
    fn rule_type_name(&self) -> &'static str;
    fn constraint_json(&self) -> serde_json::Value;
}
```

- [ ] **Step 6: Update `src/lib.rs`**

Replace the entire file:

```rust
pub mod error;
pub mod rule;
pub mod rules;
pub mod registry;
pub mod set;
pub mod validatable;

pub use error::{ValidationError, ValidationResult};
pub use rule::ValidationRule;
pub use set::ValidationSet;
pub use registry::{register_validation_set, unregister_validation_set, validate as registry_validate};
pub use validatable::Validatable;
```

- [ ] **Step 7: Create stub modules so the crate compiles**

Create `crates/data/ox_validation/src/rules/mod.rs`:
```rust
pub mod required;
pub mod length;
pub mod numeric;
pub mod regex_rule;
pub mod one_of;
pub mod matches_rule;
pub mod custom;

pub use required::Required;
pub use length::{MinLength, MaxLength};
pub use numeric::{Min, Max, Range};
pub use regex_rule::Regex;
pub use one_of::{OneOf, NotOneOf};
pub use matches_rule::Matches;
pub use custom::Custom;
```

Create each stub file with just a placeholder struct (will be expanded in later tasks):

`crates/data/ox_validation/src/rules/required.rs`:
```rust
pub struct Required { pub attribute: String, pub message: Option<String> }
```

`crates/data/ox_validation/src/rules/length.rs`:
```rust
pub struct MinLength { pub attribute: String, pub min: usize, pub message: Option<String> }
pub struct MaxLength { pub attribute: String, pub max: usize, pub message: Option<String> }
```

`crates/data/ox_validation/src/rules/numeric.rs`:
```rust
pub struct Min { pub attribute: String, pub min: f64, pub message: Option<String> }
pub struct Max { pub attribute: String, pub max: f64, pub message: Option<String> }
pub struct Range { pub attribute: String, pub min: f64, pub max: f64, pub message: Option<String> }
```

`crates/data/ox_validation/src/rules/regex_rule.rs`:
```rust
pub struct Regex { pub attribute: String, pub pattern: String, pub message: Option<String> }
```

`crates/data/ox_validation/src/rules/one_of.rs`:
```rust
pub struct OneOf { pub attribute: String, pub values: Vec<String>, pub message: Option<String> }
pub struct NotOneOf { pub attribute: String, pub values: Vec<String>, pub message: Option<String> }
```

`crates/data/ox_validation/src/rules/matches_rule.rs`:
```rust
pub struct Matches { pub attribute: String, pub other_attribute: String, pub message: Option<String> }
```

`crates/data/ox_validation/src/rules/custom.rs`:
```rust
use std::sync::Arc;
use ox_data_object::GenericDataObject;
pub struct Custom {
    pub attribute: String,
    pub description: String,
    pub rule_fn: Arc<dyn Fn(&GenericDataObject) -> Result<(), String> + Send + Sync>,
}
```

Create `crates/data/ox_validation/src/set.rs`:
```rust
// placeholder
```

Create `crates/data/ox_validation/src/registry.rs`:
```rust
use ox_data_object::GenericDataObject;
use crate::{ValidationResult, ValidationSet};

pub fn register_validation_set(_set: ValidationSet) {}
pub fn unregister_validation_set(_object_id: &str) {}
pub fn validate(_object_id: &str, _gdo: &GenericDataObject) -> ValidationResult {
    ValidationResult { errors: vec![] }
}
```

Create `crates/data/ox_validation/src/validatable.rs`:
```rust
// placeholder
pub trait Validatable {
    fn validate(&mut self, object_id: &str) -> crate::ValidationResult;
}
```

Note: `ValidationSet` doesn't exist yet — add a placeholder in `set.rs`:
```rust
use ox_data_object::GenericDataObject;
use crate::{ValidationResult, ValidationRule};

pub struct ValidationSet {
    pub object_id: String,
    pub rules: Vec<Box<dyn ValidationRule>>,
}

impl ValidationSet {
    pub fn new(object_id: &str) -> Self {
        Self { object_id: object_id.to_string(), rules: vec![] }
    }
    pub fn add_rule(&mut self, rule: Box<dyn ValidationRule>) -> &mut Self {
        self.rules.push(rule);
        self
    }
    pub fn validate(&self, _gdo: &GenericDataObject) -> ValidationResult {
        ValidationResult { errors: vec![] }
    }
}
```

- [ ] **Step 8: Run test to verify it passes**

```bash
cargo test -p ox_validation 2>&1 | tail -10
```
Expected: 3 tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/data/ox_validation
git commit -m "feat(validation): scaffold ValidationError, ValidationResult, ValidationRule trait"
```

---

## Task 2: Required, MinLength, MaxLength, OneOf, NotOneOf, Matches rules

**Files:**
- Modify: `crates/data/ox_validation/src/rules/required.rs`
- Modify: `crates/data/ox_validation/src/rules/length.rs`
- Modify: `crates/data/ox_validation/src/rules/one_of.rs`
- Modify: `crates/data/ox_validation/src/rules/matches_rule.rs`
- Modify: `crates/data/ox_validation/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

APPEND to `crates/data/ox_validation/tests/integration.rs`:

```rust
use ox_validation::rules::{Required, MinLength, MaxLength, OneOf, NotOneOf, Matches};
use ox_type_converter::ValueType;
use ox_data_object::AttributeValue;

fn gdo_with(attr: &str, val: &str) -> GenericDataObject {
    let mut gdo = GenericDataObject::new("test", None);
    gdo.set(attr, val.to_string()).unwrap();
    gdo
}

// Required
#[test]
fn required_passes_when_present() {
    let gdo = gdo_with("name", "Alice");
    let rule = Required { attribute: "name".to_string(), message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn required_fails_when_absent() {
    let gdo = GenericDataObject::new("test", None);
    let rule = Required { attribute: "name".to_string(), message: None };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "required");
    assert_eq!(err.attribute, "name");
}

#[test]
fn required_fails_when_empty_string() {
    let gdo = gdo_with("name", "");
    let rule = Required { attribute: "name".to_string(), message: None };
    assert!(rule.validate(&gdo).is_err());
}

#[test]
fn required_custom_message() {
    let gdo = GenericDataObject::new("test", None);
    let rule = Required { attribute: "email".to_string(), message: Some("Email is required".to_string()) };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.message, "Email is required");
}

// MinLength
#[test]
fn min_length_passes() {
    let gdo = gdo_with("pwd", "password123");
    let rule = MinLength { attribute: "pwd".to_string(), min: 8, message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn min_length_fails() {
    let gdo = gdo_with("pwd", "short");
    let rule = MinLength { attribute: "pwd".to_string(), min: 8, message: None };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "min_length");
}

// MaxLength
#[test]
fn max_length_passes() {
    let gdo = gdo_with("code", "ABC");
    let rule = MaxLength { attribute: "code".to_string(), max: 5, message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn max_length_fails() {
    let gdo = gdo_with("code", "TOOLONG");
    let rule = MaxLength { attribute: "code".to_string(), max: 5, message: None };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "max_length");
}

// OneOf
#[test]
fn one_of_passes() {
    let gdo = gdo_with("status", "active");
    let rule = OneOf {
        attribute: "status".to_string(),
        values: vec!["active".to_string(), "inactive".to_string()],
        message: None,
    };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn one_of_fails() {
    let gdo = gdo_with("status", "unknown");
    let rule = OneOf {
        attribute: "status".to_string(),
        values: vec!["active".to_string(), "inactive".to_string()],
        message: None,
    };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "one_of");
}

// NotOneOf
#[test]
fn not_one_of_passes() {
    let gdo = gdo_with("word", "hello");
    let rule = NotOneOf {
        attribute: "word".to_string(),
        values: vec!["admin".to_string(), "root".to_string()],
        message: None,
    };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn not_one_of_fails() {
    let gdo = gdo_with("word", "admin");
    let rule = NotOneOf {
        attribute: "word".to_string(),
        values: vec!["admin".to_string(), "root".to_string()],
        message: None,
    };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "not_one_of");
}

// Matches
#[test]
fn matches_passes() {
    let mut gdo = GenericDataObject::new("test", None);
    gdo.set("pwd", "secret".to_string()).unwrap();
    gdo.set("pwd_confirm", "secret".to_string()).unwrap();
    let rule = Matches {
        attribute: "pwd_confirm".to_string(),
        other_attribute: "pwd".to_string(),
        message: None,
    };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn matches_fails() {
    let mut gdo = GenericDataObject::new("test", None);
    gdo.set("pwd", "secret".to_string()).unwrap();
    gdo.set("pwd_confirm", "different".to_string()).unwrap();
    let rule = Matches {
        attribute: "pwd_confirm".to_string(),
        other_attribute: "pwd".to_string(),
        message: None,
    };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "matches");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_validation 2>&1 | head -20
```
Expected: FAIL — rules not implemented.

- [ ] **Step 3: Implement `src/rules/required.rs`**

```rust
use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct Required {
    pub attribute: String,
    pub message: Option<String>,
}

impl ValidationRule for Required {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if val.is_empty() {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' is required", self.attribute)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Attribute must be present and non-empty" }
    fn rule_type_name(&self) -> &'static str { "required" }
    fn constraint_json(&self) -> serde_json::Value { serde_json::Value::Null }
}
```

- [ ] **Step 4: Implement `src/rules/length.rs`**

```rust
use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct MinLength {
    pub attribute: String,
    pub min: usize,
    pub message: Option<String>,
}

impl ValidationRule for MinLength {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if val.chars().count() < self.min {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be at least {} characters", self.attribute, self.min)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Minimum character length" }
    fn rule_type_name(&self) -> &'static str { "min_length" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "min": self.min })
    }
}

pub struct MaxLength {
    pub attribute: String,
    pub max: usize,
    pub message: Option<String>,
}

impl ValidationRule for MaxLength {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if val.chars().count() > self.max {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be at most {} characters", self.attribute, self.max)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Maximum character length" }
    fn rule_type_name(&self) -> &'static str { "max_length" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "max": self.max })
    }
}
```

- [ ] **Step 5: Implement `src/rules/one_of.rs`**

```rust
use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct OneOf {
    pub attribute: String,
    pub values: Vec<String>,
    pub message: Option<String>,
}

impl ValidationRule for OneOf {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if !self.values.contains(&val) {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be one of: {}", self.attribute, self.values.join(", "))
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Value must be one of a fixed set" }
    fn rule_type_name(&self) -> &'static str { "one_of" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "values": self.values })
    }
}

pub struct NotOneOf {
    pub attribute: String,
    pub values: Vec<String>,
    pub message: Option<String>,
}

impl ValidationRule for NotOneOf {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if self.values.contains(&val) {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must not be one of: {}", self.attribute, self.values.join(", "))
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Value must not be one of a fixed set" }
    fn rule_type_name(&self) -> &'static str { "not_one_of" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "values": self.values })
    }
}
```

- [ ] **Step 6: Implement `src/rules/matches_rule.rs`**

```rust
use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct Matches {
    pub attribute: String,
    pub other_attribute: String,
    pub message: Option<String>,
}

impl ValidationRule for Matches {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        let other = gdo.get_attribute(&self.other_attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if val != other {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must match '{}'", self.attribute, self.other_attribute)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Value must equal another attribute's value" }
    fn rule_type_name(&self) -> &'static str { "matches" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "other_attribute": self.other_attribute })
    }
}
```

- [ ] **Step 7: Run test to verify it passes**

```bash
cargo test -p ox_validation 2>&1 | tail -15
```
Expected: all tests pass (3 from Task 1 + 16 new = 19 total).

- [ ] **Step 8: Commit**

```bash
git add crates/data/ox_validation
git commit -m "feat(validation): implement Required, MinLength, MaxLength, OneOf, NotOneOf, Matches rules"
```

---

## Task 3: Numeric rules (Min, Max, Range)

**Files:**
- Modify: `crates/data/ox_validation/src/rules/numeric.rs`
- Modify: `crates/data/ox_validation/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

APPEND to `crates/data/ox_validation/tests/integration.rs`:

```rust
use ox_validation::rules::{Min, Max, Range};

#[test]
fn min_passes() {
    let gdo = gdo_with("age", "25");
    let rule = Min { attribute: "age".to_string(), min: 18.0, message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn min_fails() {
    let gdo = gdo_with("age", "15");
    let rule = Min { attribute: "age".to_string(), min: 18.0, message: None };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "min");
}

#[test]
fn min_fails_non_numeric() {
    let gdo = gdo_with("age", "not-a-number");
    let rule = Min { attribute: "age".to_string(), min: 18.0, message: None };
    assert!(rule.validate(&gdo).is_err());
}

#[test]
fn max_passes() {
    let gdo = gdo_with("score", "95");
    let rule = Max { attribute: "score".to_string(), max: 100.0, message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn max_fails() {
    let gdo = gdo_with("score", "110");
    let rule = Max { attribute: "score".to_string(), max: 100.0, message: None };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "max");
}

#[test]
fn range_passes() {
    let gdo = gdo_with("pct", "50");
    let rule = Range { attribute: "pct".to_string(), min: 0.0, max: 100.0, message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn range_fails_below() {
    let gdo = gdo_with("pct", "-5");
    let rule = Range { attribute: "pct".to_string(), min: 0.0, max: 100.0, message: None };
    assert!(rule.validate(&gdo).is_err());
}

#[test]
fn range_fails_above() {
    let gdo = gdo_with("pct", "105");
    let rule = Range { attribute: "pct".to_string(), min: 0.0, max: 100.0, message: None };
    assert!(rule.validate(&gdo).is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_validation numeric 2>&1 | head -15
```
Expected: FAIL — numeric rules not implemented.

- [ ] **Step 3: Implement `src/rules/numeric.rs`**

```rust
use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct Min {
    pub attribute: String,
    pub min: f64,
    pub message: Option<String>,
}

impl ValidationRule for Min {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let raw = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        let val: f64 = raw.parse().map_err(|_| ValidationError {
            attribute: self.attribute.clone(),
            rule: self.rule_type_name().to_string(),
            message: format!("'{}' must be a number", self.attribute),
        })?;
        if val < self.min {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be at least {}", self.attribute, self.min)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Minimum numeric value" }
    fn rule_type_name(&self) -> &'static str { "min" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "min": self.min })
    }
}

pub struct Max {
    pub attribute: String,
    pub max: f64,
    pub message: Option<String>,
}

impl ValidationRule for Max {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let raw = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        let val: f64 = raw.parse().map_err(|_| ValidationError {
            attribute: self.attribute.clone(),
            rule: self.rule_type_name().to_string(),
            message: format!("'{}' must be a number", self.attribute),
        })?;
        if val > self.max {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be at most {}", self.attribute, self.max)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Maximum numeric value" }
    fn rule_type_name(&self) -> &'static str { "max" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "max": self.max })
    }
}

pub struct Range {
    pub attribute: String,
    pub min: f64,
    pub max: f64,
    pub message: Option<String>,
}

impl ValidationRule for Range {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let raw = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        let val: f64 = raw.parse().map_err(|_| ValidationError {
            attribute: self.attribute.clone(),
            rule: self.rule_type_name().to_string(),
            message: format!("'{}' must be a number", self.attribute),
        })?;
        if val < self.min || val > self.max {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be between {} and {}", self.attribute, self.min, self.max)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Numeric value within a range" }
    fn rule_type_name(&self) -> &'static str { "range" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "min": self.min, "max": self.max })
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_validation 2>&1 | tail -10
```
Expected: all tests pass (19 from Tasks 1-2 + 8 new = 27 total).

- [ ] **Step 5: Commit**

```bash
git add crates/data/ox_validation
git commit -m "feat(validation): implement Min, Max, Range numeric rules"
```

---

## Task 4: Regex and Custom rules

**Files:**
- Modify: `crates/data/ox_validation/src/rules/regex_rule.rs`
- Modify: `crates/data/ox_validation/src/rules/custom.rs`
- Modify: `crates/data/ox_validation/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

APPEND to `crates/data/ox_validation/tests/integration.rs`:

```rust
use ox_validation::rules::{Regex as RegexRule, Custom};
use std::sync::Arc;

#[test]
fn regex_passes() {
    let gdo = gdo_with("email", "user@example.com");
    let rule = RegexRule::new(
        "email",
        r"^[^@]+@[^@]+\.[^@]+$",
        None,
    ).unwrap();
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn regex_fails() {
    let gdo = gdo_with("email", "not-an-email");
    let rule = RegexRule::new("email", r"^[^@]+@[^@]+\.[^@]+$", None).unwrap();
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "regex");
}

#[test]
fn regex_invalid_pattern_returns_err() {
    let result = RegexRule::new("f", r"[invalid", None);
    assert!(result.is_err());
}

#[test]
fn custom_passes() {
    let gdo = gdo_with("score", "42");
    let rule = Custom {
        attribute: "score".to_string(),
        description: "must be 42".to_string(),
        rule_fn: Arc::new(|gdo| {
            let v = gdo.get_attribute("score").map(|a| a.to_string()).unwrap_or_default();
            if v == "42" { Ok(()) } else { Err("not 42".to_string()) }
        }),
    };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn custom_fails() {
    let gdo = gdo_with("score", "99");
    let rule = Custom {
        attribute: "score".to_string(),
        description: "must be 42".to_string(),
        rule_fn: Arc::new(|gdo| {
            let v = gdo.get_attribute("score").map(|a| a.to_string()).unwrap_or_default();
            if v == "42" { Ok(()) } else { Err("not 42".to_string()) }
        }),
    };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "custom");
    assert!(err.message.contains("not 42"));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_validation regex_rule custom 2>&1 | head -15
```
Expected: FAIL — rules not implemented.

- [ ] **Step 3: Implement `src/rules/regex_rule.rs`**

```rust
use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct Regex {
    attribute: String,
    pattern: String,
    compiled: regex::Regex,
    message: Option<String>,
}

impl Regex {
    pub fn new(attribute: &str, pattern: &str, message: Option<String>) -> Result<Self, String> {
        let compiled = regex::Regex::new(pattern)
            .map_err(|e| format!("invalid regex pattern: {}", e))?;
        Ok(Self {
            attribute: attribute.to_string(),
            pattern: pattern.to_string(),
            compiled,
            message,
        })
    }
}

impl ValidationRule for Regex {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if !self.compiled.is_match(&val) {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' does not match required pattern", self.attribute)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Value must match a regular expression" }
    fn rule_type_name(&self) -> &'static str { "regex" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "pattern": self.pattern })
    }
}
```

- [ ] **Step 4: Implement `src/rules/custom.rs`**

```rust
use std::sync::Arc;
use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct Custom {
    pub attribute: String,
    pub description: String,
    pub rule_fn: Arc<dyn Fn(&GenericDataObject) -> Result<(), String> + Send + Sync>,
}

impl ValidationRule for Custom {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        (self.rule_fn)(gdo).map_err(|msg| ValidationError {
            attribute: self.attribute.clone(),
            rule: self.rule_type_name().to_string(),
            message: msg,
        })
    }

    fn description(&self) -> &str { &self.description }
    fn rule_type_name(&self) -> &'static str { "custom" }
    fn constraint_json(&self) -> serde_json::Value { serde_json::Value::Null }
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p ox_validation 2>&1 | tail -10
```
Expected: all tests pass (27 from Tasks 1-3 + 5 new = 32 total).

- [ ] **Step 6: Commit**

```bash
git add crates/data/ox_validation
git commit -m "feat(validation): implement Regex and Custom rules"
```

---

## Task 5: ValidationSet

**Files:**
- Modify: `crates/data/ox_validation/src/set.rs`
- Modify: `crates/data/ox_validation/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

APPEND to `crates/data/ox_validation/tests/integration.rs`:

```rust
use ox_validation::ValidationSet;

#[test]
fn validation_set_collects_all_errors() {
    let mut set = ValidationSet::new("user");
    set.add_rule(Box::new(Required { attribute: "email".to_string(), message: None }))
       .add_rule(Box::new(Required { attribute: "name".to_string(), message: None }));
    let gdo = GenericDataObject::new("user", None);
    let result = set.validate(&gdo);
    assert!(!result.is_valid());
    assert_eq!(result.errors.len(), 2);
}

#[test]
fn validation_set_passes_when_all_rules_pass() {
    let mut set = ValidationSet::new("user");
    set.add_rule(Box::new(Required { attribute: "email".to_string(), message: None }));
    let gdo = gdo_with("email", "user@example.com");
    let result = set.validate(&gdo);
    assert!(result.is_valid());
}

#[test]
fn validation_set_object_id() {
    let set = ValidationSet::new("my_object");
    assert_eq!(set.object_id, "my_object");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_validation validation_set 2>&1 | head -15
```
Expected: FAIL — `ValidationSet::validate()` is still a stub.

- [ ] **Step 3: Implement `src/set.rs`**

```rust
use ox_data_object::GenericDataObject;
use crate::error::{ValidationResult};
use crate::rule::ValidationRule;

pub struct ValidationSet {
    pub object_id: String,
    pub rules: Vec<Box<dyn ValidationRule>>,
}

impl ValidationSet {
    pub fn new(object_id: &str) -> Self {
        Self { object_id: object_id.to_string(), rules: vec![] }
    }

    pub fn add_rule(&mut self, rule: Box<dyn ValidationRule>) -> &mut Self {
        self.rules.push(rule);
        self
    }

    pub fn validate(&self, gdo: &GenericDataObject) -> ValidationResult {
        let errors = self.rules.iter()
            .filter_map(|rule| rule.validate(gdo).err())
            .collect();
        ValidationResult { errors }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_validation 2>&1 | tail -10
```
Expected: all tests pass (32 from Tasks 1-4 + 3 new = 35 total).

- [ ] **Step 5: Commit**

```bash
git add crates/data/ox_validation
git commit -m "feat(validation): implement ValidationSet with multi-error collection"
```

---

## Task 6: ValidationRegistry and module-level functions

**Files:**
- Modify: `crates/data/ox_validation/src/registry.rs`
- Modify: `crates/data/ox_validation/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

APPEND to `crates/data/ox_validation/tests/integration.rs`:

```rust
use ox_validation::{register_validation_set, unregister_validation_set, registry_validate};

#[test]
fn registry_validate_registered_set() {
    let mut set = ValidationSet::new("registry_test_obj");
    set.add_rule(Box::new(Required { attribute: "title".to_string(), message: None }));
    register_validation_set(set);

    let gdo = GenericDataObject::new("registry_test_obj", None);
    let result = registry_validate("registry_test_obj", &gdo);
    assert!(!result.is_valid());
    assert_eq!(result.errors[0].attribute, "title");

    unregister_validation_set("registry_test_obj");
}

#[test]
fn registry_validate_unregistered_returns_valid() {
    let result = registry_validate("no_such_object", &GenericDataObject::new("x", None));
    assert!(result.is_valid());
}

#[test]
fn registry_unregister_removes_set() {
    let set = ValidationSet::new("temp_obj");
    register_validation_set(set);
    unregister_validation_set("temp_obj");
    let result = registry_validate("temp_obj", &GenericDataObject::new("x", None));
    assert!(result.is_valid());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_validation registry 2>&1 | head -15
```
Expected: FAIL — registry functions are stubs.

- [ ] **Step 3: Implement `src/registry.rs`**

```rust
use std::collections::HashMap;
use lazy_static::lazy_static;
use std::sync::Mutex;
use ox_data_object::GenericDataObject;
use crate::{ValidationResult, ValidationSet};

lazy_static! {
    static ref VALIDATION_REGISTRY: Mutex<HashMap<String, ValidationSet>> =
        Mutex::new(HashMap::new());
}

pub fn register_validation_set(set: ValidationSet) {
    VALIDATION_REGISTRY.lock().unwrap().insert(set.object_id.clone(), set);
}

pub fn unregister_validation_set(object_id: &str) {
    VALIDATION_REGISTRY.lock().unwrap().remove(object_id);
}

pub fn validate(object_id: &str, gdo: &GenericDataObject) -> ValidationResult {
    let registry = VALIDATION_REGISTRY.lock().unwrap();
    match registry.get(object_id) {
        Some(set) => set.validate(gdo),
        None => ValidationResult { errors: vec![] },
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_validation 2>&1 | tail -10
```
Expected: all tests pass (35 from Tasks 1-5 + 3 new = 38 total).

- [ ] **Step 5: Commit**

```bash
git add crates/data/ox_validation
git commit -m "feat(validation): implement ValidationRegistry with global thread-safe store"
```

---

## Task 7: Validatable trait on GenericDataObject with callbacks

**Files:**
- Modify: `crates/data/ox_validation/src/validatable.rs`
- Modify: `crates/data/ox_validation/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

APPEND to `crates/data/ox_validation/tests/integration.rs`:

```rust
use ox_validation::Validatable;

#[test]
fn validatable_on_gdo_valid() {
    let mut set = ValidationSet::new("gdo_valid_test");
    set.add_rule(Box::new(Required { attribute: "field".to_string(), message: None }));
    register_validation_set(set);

    let mut gdo = gdo_with("field", "value");
    let result = gdo.validate("gdo_valid_test");
    assert!(result.is_valid());

    unregister_validation_set("gdo_valid_test");
}

#[test]
fn validatable_on_gdo_invalid() {
    let mut set = ValidationSet::new("gdo_invalid_test");
    set.add_rule(Box::new(Required { attribute: "mandatory".to_string(), message: None }));
    register_validation_set(set);

    let mut gdo = GenericDataObject::new("x", None);
    let result = gdo.validate("gdo_invalid_test");
    assert!(!result.is_valid());
    assert_eq!(result.errors[0].attribute, "mandatory");

    unregister_validation_set("gdo_invalid_test");
}

#[test]
fn validatable_fires_callback_on_valid() {
    use std::sync::{Arc, Mutex};
    use ox_callback_manager::EventType;

    let mut set = ValidationSet::new("callback_valid_test");
    set.add_rule(Box::new(Required { attribute: "x".to_string(), message: None }));
    register_validation_set(set);

    let fired = Arc::new(Mutex::new(false));
    let fired_clone = fired.clone();

    let mut gdo = gdo_with("x", "hello");
    gdo.register_callback(
        EventType::new("after_validate"),
        Arc::new(move |_gdo, _params| {
            *fired_clone.lock().unwrap() = true;
            Ok(())
        }),
    );
    gdo.validate("callback_valid_test");
    assert!(*fired.lock().unwrap());

    unregister_validation_set("callback_valid_test");
}

#[test]
fn validatable_fires_callback_on_invalid() {
    use std::sync::{Arc, Mutex};
    use ox_callback_manager::EventType;

    let mut set = ValidationSet::new("callback_invalid_test");
    set.add_rule(Box::new(Required { attribute: "missing".to_string(), message: None }));
    register_validation_set(set);

    let fired = Arc::new(Mutex::new(false));
    let fired_clone = fired.clone();

    let mut gdo = GenericDataObject::new("x", None);
    gdo.register_callback(
        EventType::new("on_error_validate"),
        Arc::new(move |_gdo, _params| {
            *fired_clone.lock().unwrap() = true;
            Ok(())
        }),
    );
    gdo.validate("callback_invalid_test");
    assert!(*fired.lock().unwrap());

    unregister_validation_set("callback_invalid_test");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_validation validatable 2>&1 | head -15
```
Expected: FAIL — `Validatable` not implemented on `GenericDataObject`.

- [ ] **Step 3: Implement `src/validatable.rs`**

```rust
use ox_data_object::GenericDataObject;
use crate::registry;
use crate::error::ValidationResult;

pub trait Validatable {
    fn validate(&mut self, object_id: &str) -> ValidationResult;
}

impl Validatable for GenericDataObject {
    fn validate(&mut self, object_id: &str) -> ValidationResult {
        let _ = self.trigger_callbacks("before_validate", None, None, None);
        let result = registry::validate(object_id, self);
        if result.is_valid() {
            let _ = self.trigger_callbacks("after_validate", None, None, None);
        } else {
            let error_summary = result.errors.iter()
                .map(|e| format!("{}: {}", e.attribute, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            let _ = self.trigger_callbacks("on_error_validate", None, None, Some(&error_summary));
        }
        result
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_validation 2>&1 | tail -10
```
Expected: all tests pass (38 from Tasks 1-6 + 4 new = 42 total).

- [ ] **Step 5: Commit**

```bash
git add crates/data/ox_validation
git commit -m "feat(validation): implement Validatable on GenericDataObject with before/after/error callbacks"
```

---

## Task 8: Final verification and lib.rs clean-up

**Files:**
- Modify: `crates/data/ox_validation/src/lib.rs`

- [ ] **Step 1: Verify the full crate re-exports**

Read current `src/lib.rs`. It should already export everything from Task 1. Confirm it also re-exports the rules:

Replace `crates/data/ox_validation/src/lib.rs` with:

```rust
pub mod error;
pub mod rule;
pub mod rules;
pub mod registry;
pub mod set;
pub mod validatable;

pub use error::{ValidationError, ValidationResult};
pub use rule::ValidationRule;
pub use rules::{
    Custom, Matches, Max, MaxLength, Min, MinLength, NotOneOf, OneOf, Range, Required,
};
pub use rules::Regex;
pub use set::ValidationSet;
pub use registry::{register_validation_set, unregister_validation_set, validate as registry_validate};
pub use validatable::Validatable;
```

- [ ] **Step 2: Run full test suite**

```bash
cargo test -p ox_validation 2>&1 | tail -15
```
Expected: 42 tests pass, 0 failures.

- [ ] **Step 3: Run workspace build to confirm no breakage**

```bash
cargo build --workspace 2>&1 | grep -E "^error" | head -10
```
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/data/ox_validation
git commit -m "feat(validation): complete ox_validation — all rules, set, registry, validatable"
```
