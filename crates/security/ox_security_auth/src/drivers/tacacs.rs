use std::sync::Arc;
use async_trait::async_trait;
use futures::future::BoxFuture;
use secrecy::{ExposeSecret, SecretString};
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials,
    Principal, PrincipalId, AuthSource, TenantId,
    drivers::AuthDriver,
};
use crate::drivers::tacacs_proto::{
    build_auth_start, parse_auth_reply,
    FLAG_UNENCRYPTED, FLAG_ENCRYPTED, REPLY_STATUS_PASS,
};

/// Injected TCP send/receive function used in place of a real TCP socket.
/// Accepts the raw packet bytes to send; returns the raw reply bytes.
/// The `Arc<dyn Fn...>` pattern matches the established convention in this codebase
/// (see `RadiusAuthDriver`, `KerberosAuthDriver`).
pub type TcpSendFn = Arc<
    dyn Fn(Vec<u8>) -> BoxFuture<'static, Result<Vec<u8>, String>>
    + Send
    + Sync
>;

/// Configuration for the TACACS+ auth driver.
pub struct TacacsConfig {
    /// TACACS+ server address, e.g. `"tacacs.example.com:49"`.
    pub server: String,
    /// Shared secret configured on the TACACS+ server.
    pub secret: SecretString,
    /// Connection and reply timeout in seconds. Default: 5.
    pub timeout_secs: u64,
    pub tenant_id: TenantId,
    /// Set to `true` in production to XOR-encrypt the packet body.
    /// Set to `false` (default) only in test environments.
    pub encrypted: bool,
}

/// TACACS+ authentication driver.
///
/// Handles `Credentials::UsernamePassword` only; returns `Continue` for all
/// other credential types so the pipeline can fall through to the next driver.
///
/// Auth flow:
///   1. Build an AUTH_START packet (PAP, action=LOGIN).
///   2. Send it via the injected `TcpSendFn`.
///   3. Parse the AUTH_REPLY status byte.
///   4. `PASS` → `Authenticated`; `FAIL` or `ERROR` → `Reject`.
///   5. Transport errors → `Continue` (infrastructure outage; lets pipeline try next driver).
pub struct TacacsAuthDriver {
    config: TacacsConfig,
    send_fn: TcpSendFn,
}

impl TacacsAuthDriver {
    pub fn new(config: TacacsConfig, send_fn: TcpSendFn) -> Self {
        Self { config, send_fn }
    }
}

#[async_trait]
impl AuthDriver for TacacsAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let (username, password) = match credentials {
            Credentials::UsernamePassword { username, password } => {
                (username.as_str(), password.expose_secret().as_str())
            }
            _ => return AuthResult::Continue,
        };

        // Generate a random session ID for this exchange.
        let session_id: u32 = rand::random();
        let secret_bytes = self.config.secret.expose_secret().as_bytes().to_vec();
        let flags = if self.config.encrypted { FLAG_ENCRYPTED } else { FLAG_UNENCRYPTED };

        let pkt = match build_auth_start(username, password, session_id, flags, &secret_bytes) {
            Some(p) => p,
            None => {
                return AuthResult::Reject(
                    "TACACS+: username or password exceeds 255 bytes".to_string(),
                );
            }
        };

        let reply = match (self.send_fn)(pkt).await {
            Ok(r) => r,
            Err(_) => {
                // Infrastructure/transport errors return Continue per codebase convention
                // so the pipeline can fall through to the next driver on outages.
                return AuthResult::Continue;
            }
        };

        let status = match parse_auth_reply(&reply, &secret_bytes, session_id, flags) {
            Some(s) => s,
            None => {
                return AuthResult::Reject("TACACS+ malformed reply".to_string());
            }
        };

        if status == REPLY_STATUS_PASS {
            AuthResult::Authenticated(Principal {
                id: PrincipalId::new(),
                display_name: username.to_string(),
                source: AuthSource::Tacacs,
                groups: vec![],
                tenant_id: self.config.tenant_id.clone(),
                session_id: None,
            })
        } else {
            AuthResult::Reject(format!("TACACS+ rejected user '{}' (status={})", username, status))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::str::FromStr;
    use ox_security_core::{
        AuthPipelineContext, Credentials, TenantId,
    };
    use secrecy::SecretString;
    use crate::drivers::tacacs_proto::{
        build_header, FLAG_UNENCRYPTED, FLAG_ENCRYPTED, TYPE_AUTH,
        REPLY_STATUS_PASS, REPLY_STATUS_FAIL, REPLY_STATUS_ERROR, VERSION,
        xor_body,
    };

    fn make_ctx() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    fn up_creds(user: &str, pass: &str) -> Credentials {
        Credentials::UsernamePassword {
            username: user.to_string(),
            password: SecretString::new(pass.to_string()),
        }
    }

    /// Build a minimal AUTH_REPLY packet with the given status byte.
    fn make_reply(status: u8, session_id: u32) -> Vec<u8> {
        // AUTH_REPLY body: status, flags, server_msg_len(2), data_len(2)
        let body = vec![status, 0x00, 0x00, 0x00, 0x00, 0x00];
        let header = build_header(TYPE_AUTH, 2, FLAG_UNENCRYPTED, session_id, body.len() as u32);
        let mut pkt = header.to_vec();
        pkt.extend_from_slice(&body);
        pkt
    }

    fn driver_with_reply(status: u8) -> TacacsAuthDriver {
        let config = TacacsConfig {
            server: "127.0.0.1:49".to_string(),
            secret: SecretString::new("test_secret".to_string()),
            timeout_secs: 5,
            tenant_id: TenantId::from_str("test").unwrap(),
            encrypted: false,
        };
        let send_fn: TcpSendFn = Arc::new(move |pkt: Vec<u8>| {
            let session_id = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
            let reply = make_reply(status, session_id);
            Box::pin(async move { Ok(reply) })
        });
        TacacsAuthDriver::new(config, send_fn)
    }

    #[tokio::test]
    async fn tacacs_continues_for_non_password_creds() {
        let driver = driver_with_reply(REPLY_STATUS_PASS);
        let creds = Credentials::BearerToken { token: "tok".to_string() };
        let mut ctx = make_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn tacacs_authenticates_on_pass_reply() {
        let driver = driver_with_reply(REPLY_STATUS_PASS);
        let mut ctx = make_ctx();
        let result = driver.authenticate(&up_creds("alice", "secret"), &mut ctx).await;
        assert!(matches!(result, AuthResult::Authenticated(_)));
        if let AuthResult::Authenticated(ref p) = result {
            assert_eq!(p.display_name, "alice");
            assert!(matches!(p.source, AuthSource::Tacacs));
        }
    }

    #[tokio::test]
    async fn tacacs_rejects_on_fail_reply() {
        let driver = driver_with_reply(REPLY_STATUS_FAIL);
        let mut ctx = make_ctx();
        let result = driver.authenticate(&up_creds("alice", "wrong"), &mut ctx).await;
        assert!(matches!(result, AuthResult::Reject(_)));
    }

    #[tokio::test]
    async fn tacacs_rejects_on_error_reply() {
        let driver = driver_with_reply(REPLY_STATUS_ERROR);
        let mut ctx = make_ctx();
        let result = driver.authenticate(&up_creds("alice", "x"), &mut ctx).await;
        assert!(matches!(result, AuthResult::Reject(_)));
    }

    #[tokio::test]
    async fn tacacs_continues_on_transport_error() {
        let config = TacacsConfig {
            server: "127.0.0.1:49".to_string(),
            secret: SecretString::new("test_secret".to_string()),
            timeout_secs: 5,
            tenant_id: TenantId::from_str("test").unwrap(),
            encrypted: false,
        };
        let send_fn: TcpSendFn = Arc::new(|_pkt: Vec<u8>| {
            Box::pin(async { Err("connection refused".to_string()) })
        });
        let driver = TacacsAuthDriver::new(config, send_fn);
        let mut ctx = make_ctx();
        let result = driver.authenticate(&up_creds("alice", "secret"), &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn tacacs_authenticates_with_encryption_enabled() {
        let secret_str = "prod_secret".to_string();
        let config = TacacsConfig {
            server: "127.0.0.1:49".to_string(),
            secret: SecretString::new(secret_str.clone()),
            timeout_secs: 5,
            tenant_id: TenantId::from_str("test").unwrap(),
            encrypted: true,
        };
        let send_fn: TcpSendFn = Arc::new(move |pkt: Vec<u8>| {
            // Verify the packet type is AUTH
            assert_eq!(pkt[1], TYPE_AUTH);
            // The flags byte should be 0x00 (encrypted) not 0x04 (unencrypted).
            assert_eq!(pkt[3], FLAG_ENCRYPTED);

            let session_id = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);

            // Build an encrypted PASS reply
            let mut body = vec![REPLY_STATUS_PASS, 0x00, 0x00, 0x00, 0x00, 0x00];
            xor_body(&mut body, "prod_secret".as_bytes(), session_id, VERSION, 2);
            let header = build_header(TYPE_AUTH, 2, FLAG_ENCRYPTED, session_id, body.len() as u32);
            let mut reply = header.to_vec();
            reply.extend_from_slice(&body);
            Box::pin(async move { Ok(reply) })
        });
        let driver = TacacsAuthDriver::new(config, send_fn);
        let mut ctx = make_ctx();
        let result = driver.authenticate(&up_creds("bob", "secret"), &mut ctx).await;
        assert!(matches!(result, AuthResult::Authenticated(_)));
    }
}
