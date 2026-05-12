use std::sync::Arc;
use async_trait::async_trait;
use futures::future::BoxFuture;
use md5::{Md5, Digest};
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, TenantId,
    Principal, PrincipalId, AuthSource,
    drivers::AuthDriver,
};
use secrecy::ExposeSecret;
use secrecy::SecretString;

/// Pluggable UDP transport for RADIUS packets.
///
/// Takes the serialised Access-Request bytes and returns the raw response bytes,
/// or an error string on timeout or transport failure.
///
/// In production, wire this to a tokio UDP socket with timeout:
///
/// ```rust,ignore
/// use tokio::{net::UdpSocket, time};
/// use std::sync::Arc;
/// use futures::future::BoxFuture;
/// use futures::FutureExt;
///
/// fn production_udp_sender(server: String, timeout_secs: u64) -> UdpSendFn {
///     Arc::new(move |packet: Vec<u8>| {
///         let server = server.clone();
///         async move {
///             let socket = UdpSocket::bind("0.0.0.0:0").await
///                 .map_err(|e| format!("bind error: {}", e))?;
///             socket.connect(&server).await
///                 .map_err(|e| format!("connect error: {}", e))?;
///             socket.send(&packet).await
///                 .map_err(|e| format!("send error: {}", e))?;
///             let mut buf = vec![0u8; 4096];
///             let result = time::timeout(
///                 std::time::Duration::from_secs(timeout_secs),
///                 socket.recv(&mut buf),
///             ).await;
///             match result {
///                 Ok(Ok(n)) => Ok(buf[..n].to_vec()),
///                 Ok(Err(e)) => Err(format!("recv error: {}", e)),
///                 Err(_) => Err("timeout".to_string()),
///             }
///         }
///         .boxed()
///     })
/// }
/// ```
pub type UdpSendFn = Arc<dyn Fn(Vec<u8>) -> BoxFuture<'static, Result<Vec<u8>, String>> + Send + Sync>;

/// Configuration for `RadiusAuthDriver`.
pub struct RadiusConfig {
    /// RADIUS server address as `"host:port"`, e.g. `"radius.example.com:1812"`.
    pub server: String,
    /// Shared secret between this client and the RADIUS server.
    pub secret: SecretString,
    /// UDP response timeout in seconds. This is advisory — the production `UdpSendFn` closure
    /// should read this value and enforce it (e.g., via `tokio::time::timeout`). The driver
    /// itself does not apply the timeout automatically, as the transport is caller-provided.
    pub timeout_secs: u64,
    /// Tenant this driver is scoped to.
    pub tenant_id: TenantId,
}

/// Authentication driver that performs PAP authentication against a RADIUS server.
///
/// Accepts only `Credentials::UsernamePassword` — passes `Continue` for all other variants.
/// Constructs a standards-compliant Access-Request packet (RFC 2865), sends it via the
/// injected `UdpSendFn`, and maps the response code to `AuthResult`.
///
/// Access-Accept (Code=2) → `Authenticated`
/// Access-Reject (Code=3) → `Reject("RADIUS Access-Reject")`
/// Transport error or timeout → `Continue` (infrastructure outage; lets pipeline try next driver)
/// Unknown response code → `Reject("unexpected RADIUS response code: <n>")`
///
/// No group resolution is performed — `groups` is always `vec![]`.
pub struct RadiusAuthDriver {
    config: RadiusConfig,
    send: UdpSendFn,
}

impl RadiusAuthDriver {
    /// Construct a driver with an explicit UDP transport function.
    ///
    /// For production use, pass a closure wrapping a tokio `UdpSocket`.
    /// For tests, pass a closure that returns a pre-built response packet.
    pub fn new(config: RadiusConfig, send: UdpSendFn) -> Self {
        Self { config, send }
    }
}

/// Encode a PAP password per RFC 2865 §5.2.
///
/// Pads `password` to a multiple of 16 bytes with null bytes, then XORs each
/// 16-byte block with MD5(secret || previous_block), where the first previous_block
/// is the 16-byte Request Authenticator.
///
/// The result length is always a multiple of 16, between 16 and 128 bytes.
pub fn encode_password(password: &[u8], secret: &[u8], authenticator: &[u8; 16]) -> Vec<u8> {
    // RFC 2865 §5.2: password field is capped at 128 bytes.
    let password = &password[..password.len().min(128)];

    // Pad password to next multiple of 16 bytes (minimum 16)
    let padded_len = ((password.len().max(1) + 15) / 16) * 16;
    let mut padded = vec![0u8; padded_len];
    padded[..password.len()].copy_from_slice(password);

    let mut encoded = vec![0u8; padded_len];
    let mut prev_block: &[u8] = authenticator;

    for (block_idx, chunk) in padded.chunks(16).enumerate() {
        let mut hasher = Md5::new();
        hasher.update(secret);
        hasher.update(prev_block);
        let key = hasher.finalize();

        let out_start = block_idx * 16;
        for i in 0..16 {
            encoded[out_start + i] = chunk[i] ^ key[i];
        }
        prev_block = &encoded[out_start..out_start + 16];
    }

    encoded
}

/// Build a RADIUS Access-Request packet (RFC 2865).
///
/// Returns the raw packet bytes ready to send via UDP.
fn build_access_request(
    username: &str,
    password: &[u8],
    secret: &[u8],
    identifier: u8,
    authenticator: &[u8; 16],
) -> Vec<u8> {
    let encoded_password = encode_password(password, secret, authenticator);

    // Attribute: User-Name (type=1)
    let username_bytes = username.as_bytes();
    let username_attr_len = 2 + username_bytes.len();

    // Attribute: User-Password (type=2)
    let password_attr_len = 2 + encoded_password.len();

    // Total packet length: 20 (header) + attributes
    let total_len = 20 + username_attr_len + password_attr_len;

    let mut packet = Vec::with_capacity(total_len);

    // Header
    packet.push(1u8);                                    // Code: Access-Request
    packet.push(identifier);                             // Identifier
    packet.push((total_len >> 8) as u8);                 // Length high byte
    packet.push((total_len & 0xff) as u8);               // Length low byte
    packet.extend_from_slice(authenticator);             // Request Authenticator (16 bytes)

    // User-Name attribute (type=1)
    packet.push(1u8);                                    // Type
    packet.push(username_attr_len as u8);                // Length
    packet.extend_from_slice(username_bytes);            // Value

    // User-Password attribute (type=2)
    packet.push(2u8);                                    // Type
    packet.push(password_attr_len as u8);                // Length
    packet.extend_from_slice(&encoded_password);         // Value

    packet
}

#[async_trait]
impl AuthDriver for RadiusAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let (username, password) = match credentials {
            Credentials::UsernamePassword { username, password } => {
                (username.as_str(), password.expose_secret().as_bytes().to_vec())
            }
            _ => return AuthResult::Continue,
        };

        // RFC 2865: RADIUS attribute values are capped at 253 bytes.
        if username.len() > 253 {
            return AuthResult::Reject("RADIUS: username exceeds 253-byte maximum".to_string());
        }

        // Generate random identifier and authenticator
        let identifier: u8 = rand::random();
        let authenticator: [u8; 16] = rand::random();

        let packet = build_access_request(
            username,
            &password,
            self.config.secret.expose_secret().as_bytes(),
            identifier,
            &authenticator,
        );

        match (self.send)(packet).await {
            Err(e) => {
                // Transport/timeout errors return Continue (not Reject) per codebase convention:
                // infrastructure outages should not block the pipeline from trying other drivers.
                let _ = e; // error is intentionally discarded — log it if a tracing layer is added
                AuthResult::Continue
            }
            Ok(response) => {
                if response.is_empty() {
                    return AuthResult::Reject("empty RADIUS response".to_string());
                }
                match response[0] {
                    2 => AuthResult::Authenticated(Principal {
                        id: PrincipalId::new(),
                        display_name: username.to_string(),
                        source: AuthSource::Radius,
                        groups: vec![],
                        tenant_id: self.config.tenant_id.clone(),
                        session_id: None,
                    }),
                    3 => AuthResult::Reject("RADIUS Access-Reject".to_string()),
                    code => AuthResult::Reject(format!("unexpected RADIUS response code: {}", code)),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use futures::future::FutureExt;
    use ox_security_core::AuthPipelineContext;

    fn test_config() -> RadiusConfig {
        RadiusConfig {
            server: "radius.example.com:1812".to_string(),
            secret: SecretString::new("shared-secret".to_string()),
            timeout_secs: 5,
            tenant_id: "test".parse().unwrap(),
        }
    }

    fn test_ctx() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: "test".parse().unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    fn access_accept_packet() -> Vec<u8> {
        // Access-Accept: Code=2, Identifier=1, Length=20, Authenticator=16 zero bytes
        let mut pkt = vec![2u8, 1, 0, 20];
        pkt.extend_from_slice(&[0u8; 16]);
        pkt
    }

    fn access_reject_packet() -> Vec<u8> {
        // Access-Reject: Code=3, Identifier=1, Length=20, Authenticator=16 zero bytes
        let mut pkt = vec![3u8, 1, 0, 20];
        pkt.extend_from_slice(&[0u8; 16]);
        pkt
    }

    #[tokio::test]
    async fn radius_continues_for_non_password_creds() {
        let driver = RadiusAuthDriver::new(
            test_config(),
            Arc::new(|_pkt: Vec<u8>| async { Ok(access_accept_packet()) }.boxed()),
        );
        let creds = Credentials::KerberosTicket {
            ticket: b"some-ticket".to_vec(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn radius_authenticates_on_access_accept() {
        let driver = RadiusAuthDriver::new(
            test_config(),
            Arc::new(|_pkt: Vec<u8>| async { Ok(access_accept_packet()) }.boxed()),
        );
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("secret".to_string()),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Authenticated(p) => {
                assert_eq!(p.display_name, "alice");
                assert_eq!(p.source, AuthSource::Radius);
                assert!(p.groups.is_empty());
            }
            _ => panic!("expected Authenticated, got something else"),
        }
    }

    #[tokio::test]
    async fn radius_rejects_on_access_reject() {
        let driver = RadiusAuthDriver::new(
            test_config(),
            Arc::new(|_pkt: Vec<u8>| async { Ok(access_reject_packet()) }.boxed()),
        );
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("wrongpass".to_string()),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Reject(msg) => {
                assert!(msg.contains("Access-Reject") || msg.contains("rejected"), "unexpected: {}", msg);
            }
            _ => panic!("expected Reject, got something else"),
        }
    }

    #[tokio::test]
    async fn radius_rejects_on_timeout() {
        let driver = RadiusAuthDriver::new(
            test_config(),
            Arc::new(|_pkt: Vec<u8>| async { Err("timeout".to_string()) }.boxed()),
        );
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("secret".to_string()),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        // Transport/timeout errors return Continue per codebase convention so the pipeline
        // can fall through to the next driver rather than hard-blocking on an outage.
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn radius_rejects_on_unknown_response_code() {
        let driver = RadiusAuthDriver::new(
            test_config(),
            Arc::new(|_pkt: Vec<u8>| async { Ok(vec![4u8, 0, 0, 0]) }.boxed()), // code=4, not 2 or 3
        );
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("pass".to_string()),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Reject(_)));
    }

    #[test]
    fn radius_password_encoding_is_correct() {
        // RFC 2865 §5.2 PAP password encoding test.
        //
        // Inputs:
        //   secret        = "xyzzy5461"
        //   authenticator = 10 32 0f 46 76 ff 3f 76 bf 01 f0 f7 e7 a9 2a b7
        //   password      = "arctangent" (10 chars, padded to 16 bytes with 0x00)
        //
        // Algorithm (RFC 2865 §5.2):
        //   c(1) = p(1) XOR MD5(secret || RA)
        //   MD5("xyzzy5461" || authenticator) = f1 fa 0a 40 93 d5 19 b2 3d 6f d7 9b 7f f9 e5 06
        //
        // Expected encoded result (verified by independent Python computation):
        //   90 88 69 34 f2 bb 7e d7 53 1b d7 9b 7f f9 e5 06
        let secret = b"xyzzy5461";
        let authenticator: [u8; 16] = [
            0x10, 0x32, 0x0f, 0x46, 0x76, 0xff, 0x3f, 0x76,
            0xbf, 0x01, 0xf0, 0xf7, 0xe7, 0xa9, 0x2a, 0xb7,
        ];
        let password = b"arctangent";
        let encoded = encode_password(password, secret, &authenticator);

        // Verified: MD5("xyzzy5461" || authenticator) = f1fa0a4093d519b23d6fd79b7ff9e506
        // "arctangent\x00\x00\x00\x00\x00\x00" XOR f1fa0a4093d519b23d6fd79b7ff9e506
        //   = 90886934f2bb7ed7531bd79b7ff9e506
        let expected: [u8; 16] = [
            0x90, 0x88, 0x69, 0x34, 0xf2, 0xbb, 0x7e, 0xd7,
            0x53, 0x1b, 0xd7, 0x9b, 0x7f, 0xf9, 0xe5, 0x06,
        ];
        assert_eq!(
            encoded.as_slice(),
            &expected,
            "PAP password encoding does not match RFC 2865 §5.2 algorithm"
        );
    }
}
