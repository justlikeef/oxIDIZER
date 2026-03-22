/// Policy engine: validate template submission against per-consumer allowlists.
///
/// Called before creating any signing_request rows. If any client or any
/// payload field violates policy, the entire batch is rejected (422).
use rusqlite::{params, Connection};
use serde_json::Value;
use std::collections::HashMap;

use crate::config::ConsumerPolicy;

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("client_id '{0}' is not enrolled")]
    ClientNotEnrolled(String),

    #[error("payload key '{0}' is not allowed for consumer '{1}'")]
    DisallowedPayloadKey(String, String),

    #[error("name is required, max 200 chars, no HTML or control characters")]
    InvalidName,

    #[error("description is required, max 2000 chars, no HTML or control characters")]
    InvalidDescription,
}

/// Validate the template name field.
pub fn validate_name(name: &str) -> Result<(), PolicyError> {
    if name.is_empty() || name.len() > 200 || contains_html_or_control(name) {
        return Err(PolicyError::InvalidName);
    }
    Ok(())
}

/// Validate the template description field.
pub fn validate_description(desc: &str) -> Result<(), PolicyError> {
    if desc.is_empty() || desc.len() > 2000 || contains_html_or_control(desc) {
        return Err(PolicyError::InvalidDescription);
    }
    Ok(())
}

/// Validate that every client_id in the batch is enrolled in the `clients` table.
pub fn validate_clients(
    conn: &Connection,
    client_ids: &[String],
) -> Result<(), PolicyError> {
    for cid in client_ids {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM clients WHERE client_id = ?1",
                params![cid],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if count == 0 {
            return Err(PolicyError::ClientNotEnrolled(cid.clone()));
        }
    }
    Ok(())
}

/// Validate payload against the consumer's allowlist.
/// Only the top-level keys of the payload object are checked.
pub fn validate_payload(
    payload: &Value,
    consumer: &str,
    policies: &HashMap<String, ConsumerPolicy>,
) -> Result<(), PolicyError> {
    let policy = match policies.get(consumer) {
        Some(p) => p,
        None => return Ok(()), // no policy defined = no restriction
    };
    if policy.allowed_payload_keys.is_empty() {
        return Ok(());
    }
    if let Some(obj) = payload.as_object() {
        for key in obj.keys() {
            if !policy.allowed_payload_keys.iter().any(|k| k == key) {
                return Err(PolicyError::DisallowedPayloadKey(
                    key.clone(),
                    consumer.to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn contains_html_or_control(s: &str) -> bool {
    s.contains('<') || s.contains('>') || s.contains('&') || s.chars().any(|c| c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConsumerPolicy;

    fn make_policy(allowed_keys: &[&str]) -> HashMap<String, ConsumerPolicy> {
        let mut map = HashMap::new();
        map.insert(
            "consumer_a".to_string(),
            ConsumerPolicy {
                allowed_payload_keys: allowed_keys.iter().map(|s| s.to_string()).collect(),
            },
        );
        map
    }

    // ── validate_name ─────────────────────────────────────────────────────────

    #[test]
    fn test_validate_name_ok() {
        assert!(validate_name("nginx 1.24 rollout").is_ok());
    }

    #[test]
    fn test_validate_name_empty() {
        assert!(matches!(validate_name(""), Err(PolicyError::InvalidName)));
    }

    #[test]
    fn test_validate_name_too_long() {
        let long = "x".repeat(201);
        assert!(matches!(validate_name(&long), Err(PolicyError::InvalidName)));
    }

    #[test]
    fn test_validate_name_exactly_200_ok() {
        let exactly = "x".repeat(200);
        assert!(validate_name(&exactly).is_ok());
    }

    #[test]
    fn test_validate_name_html_tag_rejected() {
        assert!(matches!(validate_name("<script>alert(1)</script>"), Err(PolicyError::InvalidName)));
    }

    #[test]
    fn test_validate_name_ampersand_rejected() {
        assert!(matches!(validate_name("foo & bar"), Err(PolicyError::InvalidName)));
    }

    #[test]
    fn test_validate_name_control_char_rejected() {
        assert!(matches!(validate_name("foo\x00bar"), Err(PolicyError::InvalidName)));
        assert!(matches!(validate_name("foo\nbar"), Err(PolicyError::InvalidName)));
    }

    // ── validate_description ──────────────────────────────────────────────────

    #[test]
    fn test_validate_description_ok() {
        assert!(validate_description("Deploys nginx with updated config.").is_ok());
    }

    #[test]
    fn test_validate_description_empty() {
        assert!(matches!(validate_description(""), Err(PolicyError::InvalidDescription)));
    }

    #[test]
    fn test_validate_description_too_long() {
        let long = "x".repeat(2001);
        assert!(matches!(validate_description(&long), Err(PolicyError::InvalidDescription)));
    }

    #[test]
    fn test_validate_description_exactly_2000_ok() {
        let exactly = "x".repeat(2000);
        assert!(validate_description(&exactly).is_ok());
    }

    #[test]
    fn test_validate_description_html_rejected() {
        assert!(matches!(
            validate_description("<b>bold</b>"),
            Err(PolicyError::InvalidDescription)
        ));
    }

    // ── validate_clients ──────────────────────────────────────────────────────

    #[test]
    fn test_validate_clients_enrolled() {
        use crate::db::BrokerDb;
        use tempfile::NamedTempFile;
        let tmp = NamedTempFile::new().unwrap();
        let db = BrokerDb::open(tmp.path().to_str().unwrap(), "key").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO clients (client_id, enc_pubkey_b64, enrolled_at, enrolled_by)
             VALUES ('c1', 'abc', '2026-01-01T00:00:00Z', 'admin')",
            [],
        ).unwrap();
        assert!(validate_clients(conn, &["c1".to_string()]).is_ok());
    }

    #[test]
    fn test_validate_clients_unenrolled() {
        use crate::db::BrokerDb;
        use tempfile::NamedTempFile;
        let tmp = NamedTempFile::new().unwrap();
        let db = BrokerDb::open(tmp.path().to_str().unwrap(), "key").unwrap();
        let result = validate_clients(db.conn(), &["not-enrolled".to_string()]);
        assert!(matches!(result, Err(PolicyError::ClientNotEnrolled(id)) if id == "not-enrolled"));
    }

    #[test]
    fn test_validate_clients_empty_list_ok() {
        use crate::db::BrokerDb;
        use tempfile::NamedTempFile;
        let tmp = NamedTempFile::new().unwrap();
        let db = BrokerDb::open(tmp.path().to_str().unwrap(), "key").unwrap();
        assert!(validate_clients(db.conn(), &[]).is_ok());
    }

    // ── validate_payload ──────────────────────────────────────────────────────

    #[test]
    fn test_validate_payload_no_policy_allows_anything() {
        let policies = HashMap::new();
        let payload = serde_json::json!({ "any_key": "value" });
        assert!(validate_payload(&payload, "unknown_consumer", &policies).is_ok());
    }

    #[test]
    fn test_validate_payload_allowed_key() {
        let policies = make_policy(&["settings", "version"]);
        let payload = serde_json::json!({ "settings": { "workers": 4 }, "version": "1.2" });
        assert!(validate_payload(&payload, "consumer_a", &policies).is_ok());
    }

    #[test]
    fn test_validate_payload_disallowed_key() {
        let policies = make_policy(&["settings"]);
        let payload = serde_json::json!({ "settings": {}, "extra_field": "bad" });
        let result = validate_payload(&payload, "consumer_a", &policies);
        assert!(matches!(
            result,
            Err(PolicyError::DisallowedPayloadKey(key, consumer))
                if key == "extra_field" && consumer == "consumer_a"
        ));
    }

    #[test]
    fn test_validate_payload_empty_allowlist_allows_anything() {
        let policies = make_policy(&[]); // empty list = no restriction
        let payload = serde_json::json!({ "any": "thing" });
        assert!(validate_payload(&payload, "consumer_a", &policies).is_ok());
    }

    #[test]
    fn test_validate_payload_non_object_is_ok() {
        let policies = make_policy(&["x"]);
        // Array payload: no object keys to check
        let payload = serde_json::json!([1, 2, 3]);
        assert!(validate_payload(&payload, "consumer_a", &policies).is_ok());
    }
}
