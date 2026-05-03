use serde::{Deserialize, Serialize};

/// Request sent by a client to the bootstrap server.
#[derive(Debug, Serialize, Deserialize)]
pub struct BootstrapCheckinRequest {
    /// Unique identifier for the client.
    pub client_id: String,

    /// Client's public X25519 encryption key (base64url).
    pub enc_pubkey_b64: String,

    /// Client's public Ed25519 signing key (base64url).
    /// Used for identifying the client in reports.
    pub sig_pubkey_b64: String,

    /// Metadata about the host (hostname, OS, etc).
    pub metadata: serde_json::Value,
}

/// Response returned by the bootstrap server.
#[derive(Debug, Serialize, Deserialize)]
pub struct BootstrapCheckinResponse {
    /// Broker's public Ed25519 signing keys (base64url).
    /// These will be used to verify future manifest signatures.
    pub broker_pubkeys: Vec<String>,

    /// URL to poll for manifests.
    pub manifest_url: String,

    /// URL to post reports to.
    pub report_url: String,

    /// Optional: TLS configuration or other settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_overrides: Option<serde_json::Value>,
}
