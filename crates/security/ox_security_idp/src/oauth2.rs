use base64ct::{Base64Url, Encoding};
use jsonwebtoken::{encode, EncodingKey, Header, Algorithm};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{IdpConfig, OAuthClientDef};
use crate::store::{AuthCodeEntry, AuthCodeStore, RefreshTokenStore, TokenEntry, TokenStore};

pub struct Oauth2Error {
    pub status: u16,
    pub error: &'static str,
    pub description: &'static str,
}

impl Oauth2Error {
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "error": self.error,
            "error_description": self.description,
        }).to_string()
    }
}

pub fn build_encoding_key(pem: &str) -> Result<EncodingKey, String> {
    EncodingKey::from_rsa_pem(pem.as_bytes()).map_err(|e| e.to_string())
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[derive(Serialize, Deserialize)]
struct JwtClaims {
    iss: String,
    sub: String,
    aud: String,
    exp: u64,
    iat: u64,
    jti: String,
    scope: String,
}

pub fn issue_access_token(
    enc_key: &EncodingKey,
    issuer: &str,
    client_id: &str,
    principal_id: Option<&str>,
    scope: &str,
    ttl_secs: u64,
    jti: &str,
) -> Result<String, String> {
    let now = now_secs();
    let claims = JwtClaims {
        iss: issuer.to_string(),
        sub: principal_id.unwrap_or(client_id).to_string(),
        aud: client_id.to_string(),
        exp: now + ttl_secs,
        iat: now,
        jti: jti.to_string(),
        scope: scope.to_string(),
    };
    let header = Header::new(Algorithm::RS256);
    encode(&header, &claims, enc_key).map_err(|e| e.to_string())
}

/// Verify PKCE S256 challenge
pub fn verify_pkce_challenge(verifier: &str, challenge: &str) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let computed = Base64Url::encode_string(&hasher.finalize());
    computed == challenge
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn find_client<'a>(config: &'a IdpConfig, client_id: &str) -> Option<&'a OAuthClientDef> {
    config.clients.iter().find(|c| c.client_id == client_id)
}

pub fn authenticate_client(client: &OAuthClientDef, secret: Option<&str>) -> bool {
    use subtle::ConstantTimeEq;
    match (&client.client_secret_hash, secret) {
        (Some(expected_hash), Some(provided)) => {
            let provided_hash = sha256_hex(provided);
            expected_hash.as_bytes().ct_eq(provided_hash.as_bytes()).into()
        }
        (None, _) => true,
        (Some(_), None) => false,
    }
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

pub fn handle_authorize(
    config: &IdpConfig,
    code_store: &AuthCodeStore,
    query: &str,
    authenticated_principal_id: Option<&str>,
) -> Result<String, Oauth2Error> {
    let params: std::collections::HashMap<&str, &str> = query.split('&')
        .filter_map(|p| {
            let mut kv = p.splitn(2, '=');
            Some((kv.next()?, kv.next()?))
        })
        .collect();

    let client_id = params.get("client_id").copied().ok_or(Oauth2Error {
        status: 400, error: "invalid_request", description: "missing client_id",
    })?;
    let redirect_uri = params.get("redirect_uri").copied().ok_or(Oauth2Error {
        status: 400, error: "invalid_request", description: "missing redirect_uri",
    })?;
    let response_type = params.get("response_type").copied().unwrap_or("");
    let scope = params.get("scope").copied().unwrap_or("openid");
    let state = params.get("state").copied().unwrap_or("");
    let code_challenge = params.get("code_challenge").copied();
    let code_challenge_method = params.get("code_challenge_method").copied();

    if response_type != "code" {
        return Err(Oauth2Error { status: 400, error: "unsupported_response_type", description: "only 'code' supported" });
    }

    let client = find_client(config, client_id).ok_or(Oauth2Error {
        status: 400, error: "invalid_client", description: "unknown client_id",
    })?;

    if !client.redirect_uris.iter().any(|u| u == redirect_uri) {
        return Err(Oauth2Error { status: 400, error: "invalid_request", description: "redirect_uri mismatch" });
    }

    if client.client_secret_hash.is_none() && code_challenge.is_none() {
        return Err(Oauth2Error { status: 400, error: "invalid_request", description: "PKCE required for public clients" });
    }

    let principal_id = authenticated_principal_id.ok_or(Oauth2Error {
        status: 401, error: "login_required", description: "user not authenticated",
    })?;

    let code = uuid::Uuid::new_v4().to_string().replace('-', "");
    let entry = AuthCodeEntry {
        code: code.clone(),
        client_id: client_id.to_string(),
        redirect_uri: redirect_uri.to_string(),
        scope: scope.to_string(),
        principal_id: principal_id.to_string(),
        code_challenge: code_challenge.map(|s| s.to_string()),
        code_challenge_method: code_challenge_method.map(|s| s.to_string()),
        expires_at: now_secs() + 600,
    };
    code_store.insert(entry);

    let redirect = if state.is_empty() {
        format!("{}?code={}", redirect_uri, urlencoding_encode(&code))
    } else {
        format!("{}?code={}&state={}", redirect_uri, urlencoding_encode(&code), urlencoding_encode(state))
    };
    Ok(redirect)
}

pub fn handle_token(
    config: &IdpConfig,
    enc_key: &EncodingKey,
    code_store: &AuthCodeStore,
    token_store: &TokenStore,
    refresh_store: &RefreshTokenStore,
    body: &str,
) -> (u16, String) {
    let params: std::collections::HashMap<&str, &str> = body.split('&')
        .filter_map(|p| {
            let mut kv = p.splitn(2, '=');
            Some((kv.next()?, kv.next()?))
        })
        .collect();

    let grant_type = params.get("grant_type").copied().unwrap_or("");
    let client_id = params.get("client_id").copied().unwrap_or("");
    let client_secret = params.get("client_secret").copied();

    let client = match find_client(config, client_id) {
        Some(c) => c,
        None => return (401, serde_json::json!({"error":"invalid_client","error_description":"unknown client"}).to_string()),
    };

    if !authenticate_client(client, client_secret) {
        return (401, serde_json::json!({"error":"invalid_client","error_description":"invalid credentials"}).to_string());
    }

    match grant_type {
        "authorization_code" => handle_token_auth_code(config, enc_key, code_store, token_store, refresh_store, &params, client),
        "client_credentials" => handle_token_client_credentials(config, enc_key, token_store, &params, client),
        "refresh_token" => handle_token_refresh(config, enc_key, token_store, refresh_store, &params),
        _ => (400, serde_json::json!({"error":"unsupported_grant_type"}).to_string()),
    }
}

fn handle_token_auth_code(
    config: &IdpConfig,
    enc_key: &EncodingKey,
    code_store: &AuthCodeStore,
    token_store: &TokenStore,
    refresh_store: &RefreshTokenStore,
    params: &std::collections::HashMap<&str, &str>,
    client: &OAuthClientDef,
) -> (u16, String) {
    let code = match params.get("code") {
        Some(&c) => c,
        None => return (400, serde_json::json!({"error":"invalid_request","error_description":"missing code"}).to_string()),
    };
    let redirect_uri = params.get("redirect_uri").copied().unwrap_or("");

    let entry = match code_store.consume(code) {
        Some(e) => e,
        None => return (400, serde_json::json!({"error":"invalid_grant","error_description":"code not found or expired"}).to_string()),
    };

    if entry.client_id != client.client_id {
        return (400, serde_json::json!({"error":"invalid_grant","error_description":"code was not issued to this client"}).to_string());
    }
    if entry.redirect_uri != redirect_uri {
        return (400, serde_json::json!({"error":"invalid_grant","error_description":"redirect_uri mismatch"}).to_string());
    }
    if entry.expires_at < now_secs() {
        return (400, serde_json::json!({"error":"invalid_grant","error_description":"code expired"}).to_string());
    }

    if let Some(challenge) = &entry.code_challenge {
        let verifier = match params.get("code_verifier") {
            Some(&v) => v,
            None => return (400, serde_json::json!({"error":"invalid_request","error_description":"code_verifier required"}).to_string()),
        };
        if !verify_pkce_challenge(verifier, challenge) {
            return (400, serde_json::json!({"error":"invalid_grant","error_description":"PKCE verification failed"}).to_string());
        }
    }

    let jti = uuid::Uuid::new_v4().to_string();
    let access_token = match issue_access_token(enc_key, &config.issuer,
        &client.client_id, Some(&entry.principal_id), &entry.scope,
        config.access_token_ttl_secs, &jti)
    {
        Ok(t) => t,
        Err(e) => return (500, serde_json::json!({"error":"server_error","error_description":e}).to_string()),
    };

    token_store.insert(TokenEntry {
        jti: jti.clone(),
        client_id: client.client_id.clone(),
        principal_id: Some(entry.principal_id.clone()),
        scope: entry.scope.clone(),
        expires_at: now_secs() + config.access_token_ttl_secs,
        revoked: false,
        raw_jwt: Some(access_token.clone()),
    });

    let refresh_token = uuid::Uuid::new_v4().to_string();
    refresh_store.insert(TokenEntry {
        jti: refresh_token.clone(),
        client_id: client.client_id.clone(),
        principal_id: Some(entry.principal_id.clone()),
        scope: entry.scope.clone(),
        expires_at: now_secs() + config.refresh_token_ttl_secs,
        revoked: false,
        raw_jwt: None,
    });

    (200, serde_json::json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": config.access_token_ttl_secs,
        "refresh_token": refresh_token,
        "scope": entry.scope,
    }).to_string())
}

fn handle_token_client_credentials(
    config: &IdpConfig,
    enc_key: &EncodingKey,
    token_store: &TokenStore,
    params: &std::collections::HashMap<&str, &str>,
    client: &OAuthClientDef,
) -> (u16, String) {
    let scope = params.get("scope").copied().unwrap_or("");
    let jti = uuid::Uuid::new_v4().to_string();
    let access_token = match issue_access_token(enc_key, &config.issuer,
        &client.client_id, None, scope, config.access_token_ttl_secs, &jti)
    {
        Ok(t) => t,
        Err(e) => return (500, serde_json::json!({"error":"server_error","error_description":e}).to_string()),
    };
    token_store.insert(TokenEntry {
        jti,
        client_id: client.client_id.clone(),
        principal_id: None,
        scope: scope.to_string(),
        expires_at: now_secs() + config.access_token_ttl_secs,
        revoked: false,
        raw_jwt: Some(access_token.clone()),
    });
    (200, serde_json::json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": config.access_token_ttl_secs,
    }).to_string())
}

fn handle_token_refresh(
    config: &IdpConfig,
    enc_key: &EncodingKey,
    token_store: &TokenStore,
    refresh_store: &RefreshTokenStore,
    params: &std::collections::HashMap<&str, &str>,
) -> (u16, String) {
    let rt = match params.get("refresh_token") {
        Some(&t) => t,
        None => return (400, serde_json::json!({"error":"invalid_request"}).to_string()),
    };
    let entry = match refresh_store.consume(rt) {
        Some(e) => e,
        None => return (400, serde_json::json!({"error":"invalid_grant","error_description":"refresh token not found"}).to_string()),
    };
    if entry.revoked || entry.expires_at < now_secs() {
        return (400, serde_json::json!({"error":"invalid_grant","error_description":"refresh token expired or revoked"}).to_string());
    }
    let jti = uuid::Uuid::new_v4().to_string();
    let access_token = match issue_access_token(enc_key, &config.issuer,
        &entry.client_id, entry.principal_id.as_deref(), &entry.scope,
        config.access_token_ttl_secs, &jti)
    {
        Ok(t) => t,
        Err(e) => return (500, serde_json::json!({"error":"server_error","error_description":e}).to_string()),
    };
    token_store.insert(TokenEntry {
        jti,
        client_id: entry.client_id.clone(),
        principal_id: entry.principal_id.clone(),
        scope: entry.scope.clone(),
        expires_at: now_secs() + config.access_token_ttl_secs,
        revoked: false,
        raw_jwt: Some(access_token.clone()),
    });
    let new_rt = uuid::Uuid::new_v4().to_string();
    refresh_store.insert(TokenEntry {
        jti: new_rt.clone(),
        client_id: entry.client_id,
        principal_id: entry.principal_id,
        scope: entry.scope.clone(),
        expires_at: now_secs() + config.refresh_token_ttl_secs,
        revoked: false,
        raw_jwt: None,
    });
    (200, serde_json::json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": config.access_token_ttl_secs,
        "refresh_token": new_rt,
    }).to_string())
}

pub fn handle_oidc_discovery(config: &IdpConfig) -> String {
    serde_json::json!({
        "issuer": config.issuer,
        "authorization_endpoint": format!("{}/oauth2/authorize", config.issuer),
        "token_endpoint": format!("{}/oauth2/token", config.issuer),
        "introspection_endpoint": format!("{}/oauth2/introspect", config.issuer),
        "revocation_endpoint": format!("{}/oauth2/revoke", config.issuer),
        "jwks_uri": format!("{}/oidc/jwks.json", config.issuer),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "client_credentials", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
    }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_rsa_pem() -> String {
        include_str!("../tests/test_rsa_private.pem").to_string()
    }

    #[test]
    fn test_build_encoding_key_from_pem() {
        let pem = test_rsa_pem();
        let key = build_encoding_key(&pem);
        assert!(key.is_ok(), "failed to build encoding key: {:?}", key.err());
    }

    #[test]
    fn test_issue_and_verify_access_token() {
        let pem = test_rsa_pem();
        let enc_key = build_encoding_key(&pem).unwrap();
        let token = issue_access_token(&enc_key, "https://auth.example.com",
            "client-1", Some("user-1"), "openid profile", 3600, "jti-test").unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.alg, jsonwebtoken::Algorithm::RS256);
    }

    #[test]
    fn test_pkce_verifier_valid() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge = Base64Url::encode_string(&hasher.finalize());
        assert!(verify_pkce_challenge(verifier, &challenge));
    }

    #[test]
    fn test_pkce_verifier_invalid() {
        assert!(!verify_pkce_challenge("wrong", "does_not_match"));
    }

    #[test]
    fn test_urlencoding_roundtrip() {
        let s = "abc123-_~";
        assert_eq!(urlencoding_encode(s), s);
        let s2 = "hello world&foo=bar";
        let enc = urlencoding_encode(s2);
        assert!(enc.contains("%20"));
        assert!(enc.contains("%26"));
    }
}
