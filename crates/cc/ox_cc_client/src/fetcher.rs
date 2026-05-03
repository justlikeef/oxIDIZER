/// mTLS HTTPS polling of the Manifest instance.
///
/// Uses async reqwest with rustls (TLS 1.3 minimum) and client certificate
/// authentication. OCSP must be enabled on the CA; hard-fail mode is
/// enforced at the CA policy level (see Open Questions in DESIGN.md).
use anyhow::Result;
use reqwest::{Client, ClientBuilder, Certificate as ReqwestCert, Identity};
use serde_json::{json, Value};

use crate::config::ClientConfig;

/// Trait for posting "applied" notifications. Implemented by `Fetcher` in
/// production and by test stubs in unit tests.
pub trait Notifier {
    async fn post_applied(
        &self,
        cfg: &ClientConfig,
        manifest_id: &str,
        detail: Option<&str>,
    ) -> Result<()>;
}

/// Async HTTP client configured with mTLS credentials from the client config.
pub struct Fetcher {
    client: Client,
}

impl Notifier for Fetcher {
    async fn post_applied(
        &self,
        cfg: &ClientConfig,
        manifest_id: &str,
        detail: Option<&str>,
    ) -> Result<()> {
        let body = json!({
            "manifest_id": manifest_id,
            "report_id": uuid::Uuid::new_v4().to_string(),
            "sequence": 0,
            "status": "applied",
            "detail": detail
        });

        let report_url = cfg.report_url.as_ref()
            .ok_or_else(|| anyhow::anyhow!("report_url is not configured"))?;

        let resp = self.client
            .post(report_url)
            .json(&body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 200 || status == 201 {
            Ok(())
        } else {
            Err(anyhow::anyhow!("report POST returned {}", status))
        }
    }
}

impl Fetcher {
    pub fn new(cfg: &ClientConfig) -> Result<Self> {
        let tls = cfg.tls.as_ref()
            .ok_or_else(|| anyhow::anyhow!("TLS is not configured; cannot create mTLS Fetcher"))?;

        let ca_cert_pem = std::fs::read(&tls.ca_cert)?;
        let client_cert_pem = std::fs::read(&tls.client_cert)?;
        let client_key_pem = std::fs::read(&tls.client_key)?;

        let ca_cert = ReqwestCert::from_pem(&ca_cert_pem)?;
        // reqwest::Identity::from_pem expects cert + key concatenated in one PEM buffer
        let mut combined_pem = client_cert_pem.clone();
        combined_pem.extend_from_slice(&client_key_pem);
        let identity = Identity::from_pem(&combined_pem)?;

        let client = ClientBuilder::new()
            .use_rustls_tls()
            .min_tls_version(reqwest::tls::Version::TLS_1_3)
            .add_root_certificate(ca_cert)
            .identity(identity)
            .https_only(true)
            .build()?;

        Ok(Self { client })
    }

    /// Fetch the latest envelope wire string for this client from the manifest instance.
    /// Returns None if the response is 304 Not Modified or 404 (no manifest).
    pub async fn fetch_latest(&self, cfg: &ClientConfig) -> Result<Option<String>> {
        let manifest_url = cfg.manifest_url.as_ref()
            .ok_or_else(|| anyhow::anyhow!("manifest_url is not configured"))?;
        let url = format!("{}/cc/manifest/{}/latest", manifest_url, cfg.client_id);
        let resp = self.client.get(&url).send().await?;

        match resp.status().as_u16() {
            200 => {
                let body = resp.text().await?;
                let val: Value = serde_json::from_str(&body)?;
                let wire = val
                    .get("envelope_wire")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("manifest response missing 'envelope_wire' field"))?
                    .to_string();
                Ok(Some(wire))
            }
            304 | 404 => Ok(None),
            status => Err(anyhow::anyhow!("manifest fetch: unexpected status {}", status)),
        }
    }
}
