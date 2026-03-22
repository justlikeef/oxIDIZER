use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Plaintext inner manifest. Produced by the broker after encryption,
/// decrypted by the client after signature verification.
///
/// The `payload` field is opaque to the broker and the client core;
/// it is forwarded verbatim to the consumer (e.g. arcnition).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Protocol version. Currently "1".
    pub version: String,

    /// UUIDv4. Must match the `manifest_id` in the outer envelope.
    pub manifest_id: String,

    /// Must match the `client_id` in the outer envelope.
    pub client_id: String,

    /// Consumer tag (e.g. "arcnition").
    pub consumer: String,

    /// Human-readable name. Validated by broker: max 200 chars, no HTML/control chars.
    pub name: String,

    /// Human-readable description. Validated by broker: max 2000 chars, no HTML/control chars.
    pub description: String,

    /// ISO 8601 UTC timestamp at which the broker signed this manifest.
    /// Checked by client post-decryption: must be ≤ now (±60s clock skew tolerance).
    pub issued_at: String,

    /// ISO 8601 UTC timestamp after which the client must not apply this manifest.
    /// Also present on the outer envelope for pre-decryption expiry checks.
    pub expires_at: String,

    /// Consumer-specific payload. The broker validates its structure against
    /// the per-consumer allowlist but does not interpret it further.
    /// The client writes this field verbatim into `manifest.json`.
    pub payload: Value,
}

/// The single file written atomically by the applier to the consumer's
/// manifest directory. Combines payload with report metadata so the
/// consuming agent can POST progress reports without additional config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplierManifest {
    /// From the inner `Manifest`.
    pub manifest_id: String,

    /// URL to POST progress reports to (e.g. `https://manifest.example.com/cc/report/{client_id}`).
    pub report_url: String,

    /// Path to the client TLS certificate for mTLS POST to the report endpoint.
    pub client_cert: String,

    /// Path to the client TLS private key.
    pub client_key: String,

    /// Path to the server CA certificate.
    pub ca_cert: String,

    /// Verbatim `payload` from the inner `Manifest`.
    pub payload: Value,
}

/// A single step in a commandset payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEntry {
    /// Name of the command to dispatch (built-in or external binary name).
    pub command: String,

    /// What to do if this command fails. Defaults to `Fail`.
    #[serde(default)]
    pub on_failure: OnFailure,

    /// Parameters passed to the command.
    #[serde(default)]
    pub params: serde_json::Map<String, serde_json::Value>,
}

/// Controls execution behaviour when a command exits with an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnFailure {
    /// Stop the commandset immediately. **Default.**
    #[default]
    Fail,
    /// Record the failure and continue to the next command.
    Continue,
}

#[cfg(test)]
mod command_entry_tests {
    use super::*;

    #[test]
    fn test_command_entry_defaults_to_fail() {
        let json = r#"{"command":"download","params":{"url":"https://example.com"}}"#;
        let entry: CommandEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.command, "download");
        assert!(matches!(entry.on_failure, OnFailure::Fail));
    }

    #[test]
    fn test_command_entry_continue() {
        let json = r#"{"command":"log_info","on_failure":"continue","params":{}}"#;
        let entry: CommandEntry = serde_json::from_str(json).unwrap();
        assert!(matches!(entry.on_failure, OnFailure::Continue));
    }

    #[test]
    fn test_commandset_array_order_preserved() {
        let json = r#"[
            {"command":"a","params":{}},
            {"command":"b","params":{}},
            {"command":"c","params":{}}
        ]"#;
        let entries: Vec<CommandEntry> = serde_json::from_str(json).unwrap();
        assert_eq!(entries[0].command, "a");
        assert_eq!(entries[1].command, "b");
        assert_eq!(entries[2].command, "c");
    }
}
