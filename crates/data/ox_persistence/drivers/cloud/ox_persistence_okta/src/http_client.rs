//! Okta HTTP client abstraction.  Production uses RealOktaHttpClient (reqwest blocking).
//! Tests use MockOktaHttpClient which returns pre-programmed responses.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ox_data_error::OxDataError;
use serde_json::Value;

/// Minimal HTTP operations needed by the Okta driver.
pub trait OktaHttpClient: Send + Sync {
    fn get(&self, path: &str) -> Result<Value, OxDataError>;
    fn post(&self, path: &str, body: &Value) -> Result<Value, OxDataError>;
    fn put(&self, path: &str, body: &Value) -> Result<Value, OxDataError>;
    fn delete(&self, path: &str) -> Result<(), OxDataError>;
}

// ---------------------------------------------------------------------------
// Real client backed by reqwest blocking
// ---------------------------------------------------------------------------

pub struct RealOktaHttpClient {
    base_url: String,   // e.g. "https://yourorg.okta.com"
    api_token: String,  // SSWS token
    client: reqwest::blocking::Client,
}

impl RealOktaHttpClient {
    pub fn new(domain: &str, api_token: &str) -> Self {
        let base_url = if domain.starts_with("https://") {
            domain.to_string()
        } else {
            format!("https://{}", domain)
        };
        Self {
            base_url,
            api_token: api_token.to_string(),
            client: reqwest::blocking::Client::new(),
        }
    }

    fn full_url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    fn auth_header(&self) -> String {
        format!("SSWS {}", self.api_token)
    }
}

impl OktaHttpClient for RealOktaHttpClient {
    fn get(&self, path: &str) -> Result<Value, OxDataError> {
        let resp = self.client
            .get(&self.full_url(path))
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .send()
            .map_err(|e| OxDataError::DriverError(format!("Okta GET {}: {}", path, e)))?;

        if resp.status().is_success() {
            resp.json::<Value>()
                .map_err(|e| OxDataError::InternalError(format!("Okta GET parse: {}", e)))
        } else {
            Err(OxDataError::DriverError(format!("Okta GET {} returned {}", path, resp.status())))
        }
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, OxDataError> {
        let resp = self.client
            .post(&self.full_url(path))
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .json(body)
            .send()
            .map_err(|e| OxDataError::DriverError(format!("Okta POST {}: {}", path, e)))?;

        if resp.status().is_success() {
            resp.json::<Value>()
                .map_err(|e| OxDataError::InternalError(format!("Okta POST parse: {}", e)))
        } else {
            Err(OxDataError::DriverError(format!("Okta POST {} returned {}", path, resp.status())))
        }
    }

    fn put(&self, path: &str, body: &Value) -> Result<Value, OxDataError> {
        let resp = self.client
            .put(&self.full_url(path))
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .json(body)
            .send()
            .map_err(|e| OxDataError::DriverError(format!("Okta PUT {}: {}", path, e)))?;

        if resp.status().is_success() {
            resp.json::<Value>()
                .map_err(|e| OxDataError::InternalError(format!("Okta PUT parse: {}", e)))
        } else {
            Err(OxDataError::DriverError(format!("Okta PUT {} returned {}", path, resp.status())))
        }
    }

    fn delete(&self, path: &str) -> Result<(), OxDataError> {
        let resp = self.client
            .delete(&self.full_url(path))
            .header("Authorization", self.auth_header())
            .send()
            .map_err(|e| OxDataError::DriverError(format!("Okta DELETE {}: {}", path, e)))?;

        if resp.status().is_success() || resp.status().as_u16() == 204 {
            Ok(())
        } else {
            Err(OxDataError::DriverError(format!("Okta DELETE {} returned {}", path, resp.status())))
        }
    }
}

// ---------------------------------------------------------------------------
// Mock client (tests only)
// ---------------------------------------------------------------------------

/// MockOktaHttpClient stores canned GET/POST responses keyed by path and records
/// what was actually posted/put so tests can assert on outbound request bodies.
#[derive(Default)]
pub struct MockOktaHttpClient {
    get_responses:   Arc<Mutex<HashMap<String, Value>>>,
    post_responses:  Arc<Mutex<HashMap<String, Value>>>,
    put_responses:   Arc<Mutex<HashMap<String, Value>>>,
    recorded_posts:  Arc<Mutex<Vec<Value>>>,
    recorded_puts:   Arc<Mutex<Vec<Value>>>,
}

impl MockOktaHttpClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn expect_get(&self, path: &str, response: Value) {
        self.get_responses.lock().unwrap().insert(path.to_string(), response);
    }

    pub fn expect_post(&self, path: &str, response: Value) {
        self.post_responses.lock().unwrap().insert(path.to_string(), response);
    }

    pub fn expect_put(&self, path: &str, response: Value) {
        self.put_responses.lock().unwrap().insert(path.to_string(), response);
    }

    pub fn recorded_posts(&self) -> Vec<Value> {
        self.recorded_posts.lock().unwrap().clone()
    }

    pub fn recorded_puts(&self) -> Vec<Value> {
        self.recorded_puts.lock().unwrap().clone()
    }
}

impl OktaHttpClient for MockOktaHttpClient {
    fn get(&self, path: &str) -> Result<Value, OxDataError> {
        self.get_responses
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| OxDataError::DriverError(format!("MockOktaHttpClient: no GET canned for {}", path)))
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, OxDataError> {
        self.recorded_posts.lock().unwrap().push(body.clone());
        self.post_responses
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| OxDataError::DriverError(format!("MockOktaHttpClient: no POST canned for {}", path)))
    }

    fn put(&self, path: &str, body: &Value) -> Result<Value, OxDataError> {
        self.recorded_puts.lock().unwrap().push(body.clone());
        self.put_responses
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| OxDataError::DriverError(format!("MockOktaHttpClient: no PUT canned for {}", path)))
    }

    fn delete(&self, _path: &str) -> Result<(), OxDataError> {
        Ok(())
    }
}
