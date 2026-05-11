/// A single permission grant stored against a principal or group.
///
/// `resource_pattern` is `None` to match all resources for the operation,
/// or `Some(pattern)` where `pattern` may be:
///   - an exact string, e.g. `"files/readme.txt"`
///   - a wildcard-suffix string ending with `/*`, e.g. `"files/*"` — matches any
///     resource whose path starts with `"files/"`.
#[derive(Debug, Clone)]
pub struct PermissionGrant {
    pub operation: String,
    pub resource_pattern: Option<String>,
}
