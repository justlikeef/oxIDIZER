use serde_json::{Map, Value};
use ox_security_core::accounting::{AccountingEvent, AuthOutcome, AuthzOutcome};

/// Converts an `AccountingEvent` to a `serde_json::Map` for serialisation.
/// `AccountingEvent` does not derive `Serialize` (it contains `IpAddr`), so we
/// map each field manually.
pub fn serialize_event(event: &AccountingEvent) -> Map<String, Value> {
    let mut map = Map::new();

    // auth_outcome
    let auth_outcome_str = match &event.auth_outcome {
        AuthOutcome::Authenticated => "Authenticated".to_string(),
        AuthOutcome::Failed(reason) => format!("Failed({})", reason),
        AuthOutcome::MfaRequired => "MfaRequired".to_string(),
        AuthOutcome::MfaFailed(reason) => format!("MfaFailed({})", reason),
    };
    map.insert("auth_outcome".to_string(), Value::String(auth_outcome_str));

    // authz_outcome — optional
    let authz_str = match &event.authz_outcome {
        None => Value::Null,
        Some(AuthzOutcome::Allowed) => Value::String("Allowed".to_string()),
        Some(AuthzOutcome::Denied { path, operation_name }) => {
            Value::String(format!("Denied(path={}, op={})", path, operation_name))
        }
    };
    map.insert("authz_outcome".to_string(), authz_str);

    // timestamp as Unix epoch seconds (i64) via chrono
    map.insert(
        "timestamp".to_string(),
        Value::Number(event.timestamp.timestamp().into()),
    );

    // source_ip as string
    map.insert(
        "source_ip".to_string(),
        Value::String(event.source_ip.to_string()),
    );

    // tenant_id
    map.insert(
        "tenant_id".to_string(),
        Value::String(event.tenant_id.as_str().to_string()),
    );

    // session_id — optional; SessionId derives Serialize so use serde_json::to_value
    let session_val = event
        .session_id
        .as_ref()
        .and_then(|s| serde_json::to_value(s).ok())
        .unwrap_or(Value::Null);
    map.insert("session_id".to_string(), session_val);

    // principal_id — optional UUID string via as_uuid()
    let principal_str = event
        .principal_id
        .as_ref()
        .map(|p| Value::String(p.as_uuid().to_string()))
        .unwrap_or(Value::Null);
    map.insert("principal_id".to_string(), principal_str);

    // call_context
    map.insert(
        "call_context".to_string(),
        Value::String(event.call_context.clone()),
    );

    // object_fragment — optional
    map.insert(
        "object_fragment".to_string(),
        event
            .object_fragment
            .as_ref()
            .map(|s| Value::String(s.clone()))
            .unwrap_or(Value::Null),
    );

    // operation_name — optional
    map.insert(
        "operation_name".to_string(),
        event
            .operation_name
            .as_ref()
            .map(|s| Value::String(s.clone()))
            .unwrap_or(Value::Null),
    );

    map
}
