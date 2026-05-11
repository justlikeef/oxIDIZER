pub mod error;
pub mod rule;
pub mod rules;
pub mod registry;
pub mod set;
pub mod validatable;

pub use error::{ValidationError, ValidationResult};
pub use rule::ValidationRule;
pub use rules::{Custom, Matches, Max, MaxLength, Min, MinLength, NotOneOf, OneOf, Range, Required};
pub use rules::Regex;
pub use set::ValidationSet;
pub use registry::{register_validation_set, unregister_validation_set, validate as registry_validate};
pub use validatable::Validatable;
