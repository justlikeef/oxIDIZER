/// mTLS HTTP client for admin plugin outbound calls to broker and manifest instance.
///
/// Uses reqwest blocking client (synchronous) so handler functions remain
/// non-async. Both the broker and manifest instance require mTLS with the
/// admin role certificate.
use anyhow::Result;
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::{Certificate, Identity};
use serde_json::Value;

use crate::config::AdminPluginConfig;

pub struct AdminHttpClient {
    client: Client,
}

impl AdminHttpClient {
    pub fn new(cfg: &AdminPluginConfig) -> Result<Self> {
        let ca_pem = std::fs::read(&cfg.tls.ca_cert)?;
        let cert_pem = std::fs::read(&cfg.tls.client_cert)?;
        let key_pem = std::fs::read(&cfg.tls.client_key)?;

        let ca_cert = Certificate::from_pem(&ca_pem)?;
        let mut combined = cert_pem;
        combined.extend_from_slice(&key_pem);
        let identity = Identity::from_pem(&combined)?;

        let client = ClientBuilder::new()
            .use_rustls_tls()
            .min_tls_version(reqwest::tls::Version::TLS_1_3)
            .add_root_certificate(ca_cert)
            .identity(identity)
            .https_only(true)
            .build()?;

        Ok(Self { client })
    }

    pub fn get(&self, url: &str) -> Result<Value, String> {
        let resp = self
            .client
            .get(url)
            .send()
            .map_err(|e| format!("GET {}: {}", url, e))?;
        let status = resp.status().as_u16();
        let body: Value = resp
            .json()
            .map_err(|e| format!("GET {} parse: {}", url, e))?;
        if status >= 200 && status < 300 {
            Ok(body)
        } else {
            Err(format!("GET {} returned {}: {}", url, status, body))
        }
    }

    pub fn post(&self, url: &str, payload: &Value) -> Result<Value, String> {
        let resp = self
            .client
            .post(url)
            .json(payload)
            .send()
            .map_err(|e| format!("POST {}: {}", url, e))?;
        let status = resp.status().as_u16();
        let body: Value = resp
            .json()
            .map_err(|e| format!("POST {} parse: {}", url, e))?;
        if status >= 200 && status < 300 {
            Ok(body)
        } else {
            Err(format!("POST {} returned {}: {}", url, status, body))
        }
    }

    pub fn patch(&self, url: &str, payload: &Value) -> Result<Value, String> {
        let resp = self
            .client
            .patch(url)
            .json(payload)
            .send()
            .map_err(|e| format!("PATCH {}: {}", url, e))?;
        let status = resp.status().as_u16();
        let body: Value = resp
            .json()
            .map_err(|e| format!("PATCH {} parse: {}", url, e))?;
        if status >= 200 && status < 300 {
            Ok(body)
        } else {
            Err(format!("PATCH {} returned {}: {}", url, status, body))
        }
    }
}

/// Trait allowing test stubs to be injected instead of the real HTTP client.
pub trait HttpClient {
    fn get(&self, url: &str) -> Result<Value, String>;
    fn post(&self, url: &str, payload: &Value) -> Result<Value, String>;
    fn patch(&self, url: &str, payload: &Value) -> Result<Value, String>;
}

impl HttpClient for AdminHttpClient {
    fn get(&self, url: &str) -> Result<Value, String> {
        AdminHttpClient::get(self, url)
    }
    fn post(&self, url: &str, payload: &Value) -> Result<Value, String> {
        AdminHttpClient::post(self, url, payload)
    }
    fn patch(&self, url: &str, payload: &Value) -> Result<Value, String> {
        AdminHttpClient::patch(self, url, payload)
    }
}
