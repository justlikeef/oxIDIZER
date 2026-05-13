use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenEntry {
    pub jti: String,
    pub client_id: String,
    pub principal_id: Option<String>,
    pub scope: String,
    pub expires_at: u64,
    pub revoked: bool,
    pub raw_jwt: Option<String>, // The original signed JWT string, for introspect/revoke lookup
}

#[derive(Clone)]
pub struct TokenStore {
    inner: Arc<Mutex<HashMap<String, TokenEntry>>>,
}

impl TokenStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn insert(&self, entry: TokenEntry) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).insert(entry.jti.clone(), entry);
    }

    pub fn get(&self, jti: &str) -> Option<TokenEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).get(jti).cloned()
    }

    pub fn revoke(&self, jti: &str) {
        if let Some(e) = self.inner.lock().unwrap_or_else(|p| p.into_inner()).get_mut(jti) {
            e.revoked = true;
        }
    }

    pub fn list_active(&self) -> Vec<TokenEntry> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).values()
            .filter(|e| !e.revoked && e.expires_at > now)
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthCodeEntry {
    pub code: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub principal_id: String,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub expires_at: u64,
}

#[derive(Clone)]
pub struct AuthCodeStore {
    inner: Arc<Mutex<HashMap<String, AuthCodeEntry>>>,
}

impl AuthCodeStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn insert(&self, entry: AuthCodeEntry) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).insert(entry.code.clone(), entry);
    }

    pub fn consume(&self, code: &str) -> Option<AuthCodeEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).remove(code)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamlSessionEntry {
    pub session_id: String,
    pub sp_entity_id: String,
    pub principal_id: String,
    pub name_id: String,
    pub created_at: u64,
}

#[derive(Clone)]
pub struct SamlSessionStore {
    inner: Arc<Mutex<HashMap<String, SamlSessionEntry>>>,
}

impl SamlSessionStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn insert(&self, entry: SamlSessionEntry) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).insert(entry.session_id.clone(), entry);
    }

    pub fn get(&self, session_id: &str) -> Option<SamlSessionEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).get(session_id).cloned()
    }

    pub fn remove(&self, session_id: &str) -> Option<SamlSessionEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).remove(session_id)
    }

    pub fn list(&self) -> Vec<SamlSessionEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).values().cloned().collect()
    }
}

#[derive(Clone)]
pub struct RefreshTokenStore {
    inner: Arc<Mutex<HashMap<String, TokenEntry>>>,
}

impl RefreshTokenStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn insert(&self, entry: TokenEntry) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).insert(entry.jti.clone(), entry);
    }

    pub fn consume(&self, token: &str) -> Option<TokenEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).remove(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_store_insert_and_lookup() {
        let store = TokenStore::new();
        let entry = TokenEntry {
            jti: "jti-1".to_string(),
            client_id: "client-1".to_string(),
            principal_id: Some("user-1".to_string()),
            scope: "openid".to_string(),
            expires_at: u64::MAX,
            revoked: false,
            raw_jwt: None,
        };
        store.insert(entry);
        let found = store.get("jti-1").unwrap();
        assert_eq!(found.client_id, "client-1");
        assert!(!found.revoked);
    }

    #[test]
    fn test_token_store_revoke() {
        let store = TokenStore::new();
        store.insert(TokenEntry {
            jti: "jti-2".to_string(),
            client_id: "c".to_string(),
            principal_id: None,
            scope: "".to_string(),
            expires_at: u64::MAX,
            revoked: false,
            raw_jwt: None,
        });
        store.revoke("jti-2");
        assert!(store.get("jti-2").unwrap().revoked);
    }

    #[test]
    fn test_auth_code_consume() {
        let store = AuthCodeStore::new();
        store.insert(AuthCodeEntry {
            code: "abc123".to_string(),
            client_id: "c".to_string(),
            redirect_uri: "https://app.example.com/cb".to_string(),
            scope: "openid".to_string(),
            principal_id: "u1".to_string(),
            code_challenge: None,
            code_challenge_method: None,
            expires_at: u64::MAX,
        });
        let entry = store.consume("abc123");
        assert!(entry.is_some());
        assert!(store.consume("abc123").is_none());
    }
}
