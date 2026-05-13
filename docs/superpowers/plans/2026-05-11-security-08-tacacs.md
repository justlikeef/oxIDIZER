# TACACS+ Auth + Accounting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `TacacsAuthDriver` in `ox_security_auth` and `TacacsAccountingDriver` in `ox_security_accounting`. Both drivers speak a custom hand-rolled TCP TACACS+ protocol (no external crate exists for Rust TACACS+). The auth driver handles PAP authentication via the three-packet AUTH flow. The accounting driver sends START/STOP accounting records as fire-and-forget. Both use an injected `TcpSendFn` for testability.

**Architecture:** `TacacsAuthDriver` accepts only `Credentials::UsernamePassword`, opens a TCP connection via the injected send-fn, sends an AUTH_START packet, reads the AUTH_REPLY, and maps `PASS → Authenticated`, `FAIL/ERROR → Reject`. `TacacsAccountingDriver` accepts `AccountingEvent`, builds an ACCT_REQUEST packet, sends fire-and-forget (errors silently swallowed). Both share a packet-building/parsing module `tacacs_proto` inside the relevant crate.

**Tech Stack:** Rust, `ox_security_core` (all shared types), `ox_security_accounting` (accounting driver trait), `async-trait`, `secrecy`, `md-5 = "0.10"` (for XOR key derivation), `tokio` (dev-dep), `rand = "0.8"` (session_id generation), futures (`futures = "0.3"` for `BoxFuture` in test inject)

---

## File Structure

```
crates/security/ox_security_auth/
  Cargo.toml                           — add md-5, rand, futures deps
  src/
    drivers/
      tacacs.rs                        — TacacsAuthDriver (replaces stub)
      tacacs_proto.rs                  — shared packet builder/parser (auth side)

crates/security/ox_security_accounting/
  Cargo.toml                           — add md-5, rand, futures deps
  src/
    drivers/
      tacacs.rs                        — TacacsAccountingDriver (new file)
      mod.rs                           — add pub use tacacs::TacacsAccountingDriver
    lib.rs                             — add re-export
```

---

## Background: TACACS+ Protocol

TACACS+ uses TCP port 49. Every packet starts with a **12-byte header**:

```
Byte 0:  version   = 0xC1  (major=0xC, minor=0x1)
Byte 1:  type      = 0x01 AUTH | 0x02 AUTHZ | 0x03 ACCT
Byte 2:  seq_no    = starts at 1, increments per exchange
Byte 3:  flags     = 0x04 unencrypted (testing) | 0x00 encrypted (production)
Bytes 4-7:  session_id  (random u32, big-endian)
Bytes 8-11: length      (body length, big-endian u32)
```

**Body encryption** (when `flags == 0x00`): XOR body bytes with the pseudo-pad
`MD5(secret || session_id_bytes || version || seq_no)` repeated to body length.
For the initial implementation use `flags = 0x04` (unencrypted) so tests are
deterministic. Encryption is wired up in Task 3.

### AUTH_START body (client → server, seq_no=1)

```
action      u8  = 0x01 (LOGIN)
authen_type u8  = 0x02 (PAP)
service     u8  = 0x01 (LOGIN)
user_len    u8
port_len    u8  = 0
rem_addr_len u8 = 0
data_len    u8  (password length)
user        [user_len bytes]
data        [data_len bytes]  (password in cleartext for PAP)
```

### AUTH_REPLY body (server → client, seq_no=2)

```
status      u8  = 0x01 PASS | 0x02 FAIL | 0x03 ERROR | 0x05 FOLLOW
flags       u8
server_msg_len  u16 (big-endian)
data_len    u16 (big-endian)
server_msg  [server_msg_len bytes]
data        [data_len bytes]
```

### ACCT_REQUEST body (client → server, seq_no=1)

```
flags       u8  = 0x02 START | 0x04 STOP | 0x06 WATCHDOG
authen_method u8 = 0x06 (TACACSPLUS)
priv_lvl    u8  = 0x01
authen_type u8  = 0x02 (PAP)
authen_service u8 = 0x01 (LOGIN)
user_len    u8
port_len    u8  = 0
rem_addr_len u8 = 0
arg_cnt     u8  (number of AV pairs)
user        [user_len bytes]
av_lengths  [arg_cnt bytes]  (each AV pair byte length)
av_pairs    [concatenated AV pair strings]  e.g. "task_id=1\0service=shell"
```

### ACCT_REPLY body (server → client, seq_no=2)

```
status      u8  = 0x01 SUCCESS | 0x02 ERROR
server_msg_len u16
data_len    u16
server_msg  [server_msg_len bytes]
data        [data_len bytes]
```

---

## Task 1: Shared packet primitives in `ox_security_auth`

**Files:**
- Modify: `crates/security/ox_security_auth/Cargo.toml`
- Create: `crates/security/ox_security_auth/src/drivers/tacacs_proto.rs`

- [ ] **Step 1: Add dependencies to `ox_security_auth/Cargo.toml`**

```toml
[package]
name = "ox_security_auth"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[dependencies]
ox_security_core = { path = "../ox_security_core" }
async-trait      = "0.1"
secrecy          = { version = "0.8", features = ["serde"] }
md-5             = "0.10"
rand             = "0.8"
futures          = "0.3"

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt"] }
```

- [ ] **Step 2: Create `src/drivers/tacacs_proto.rs`**

This module contains all packet construction and parsing. It is `pub(crate)` — not
exposed as a public API.

```rust
//! TACACS+ packet builder and parser.
//!
//! Reference: RFC 8907 (The TACACS+ Protocol).
//! This implementation supports:
//!   - AUTH packets (type 0x01): client sends AUTH_START, server replies AUTH_REPLY
//!   - ACCT packets (type 0x03): client sends ACCT_REQUEST, server replies ACCT_REPLY
//!
//! Encryption is implemented but disabled for testing via `flags = FLAG_UNENCRYPTED`.

use md5::{Md5, Digest};

// ── Constants ────────────────────────────────────────────────────────────────

pub const VERSION: u8          = 0xC1;
pub const TYPE_AUTH: u8        = 0x01;
pub const TYPE_ACCT: u8        = 0x03;
pub const FLAG_UNENCRYPTED: u8 = 0x04;
pub const FLAG_ENCRYPTED: u8   = 0x00;

pub const AUTH_ACTION_LOGIN: u8    = 0x01;
pub const AUTH_TYPE_PAP: u8        = 0x02;
pub const AUTH_SERVICE_LOGIN: u8   = 0x01;

pub const REPLY_STATUS_PASS:  u8 = 0x01;
pub const REPLY_STATUS_FAIL:  u8 = 0x02;
pub const REPLY_STATUS_ERROR: u8 = 0x03;

pub const ACCT_FLAG_START:    u8 = 0x02;
pub const ACCT_FLAG_STOP:     u8 = 0x04;
pub const ACCT_METHOD_TACACS: u8 = 0x06;
pub const ACCT_STATUS_SUCCESS: u8 = 0x01;

// ── Header ───────────────────────────────────────────────────────────────────

/// Build a 12-byte TACACS+ header.
pub fn build_header(
    pkt_type: u8,
    seq_no: u8,
    flags: u8,
    session_id: u32,
    body_len: u32,
) -> [u8; 12] {
    let mut h = [0u8; 12];
    h[0] = VERSION;
    h[1] = pkt_type;
    h[2] = seq_no;
    h[3] = flags;
    h[4..8].copy_from_slice(&session_id.to_be_bytes());
    h[8..12].copy_from_slice(&body_len.to_be_bytes());
    h
}

/// Parse a 12-byte TACACS+ header.
/// Returns `(pkt_type, seq_no, flags, session_id, body_len)`.
pub fn parse_header(buf: &[u8; 12]) -> (u8, u8, u8, u32, u32) {
    let pkt_type   = buf[1];
    let seq_no     = buf[2];
    let flags      = buf[3];
    let session_id = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let body_len   = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
    (pkt_type, seq_no, flags, session_id, body_len)
}

// ── Encryption / decryption ──────────────────────────────────────────────────

/// Compute the XOR pseudo-pad for TACACS+ body encryption.
/// pad = MD5(secret || session_id_bytes || version || seq_no) repeated to body_len.
pub fn compute_pad(secret: &[u8], session_id: u32, version: u8, seq_no: u8, body_len: usize) -> Vec<u8> {
    let session_bytes = session_id.to_be_bytes();
    let mut pad = Vec::with_capacity(body_len);
    let mut prev_hash: Option<Vec<u8>> = None;

    while pad.len() < body_len {
        let mut hasher = Md5::new();
        hasher.update(secret);
        hasher.update(session_bytes);
        hasher.update([version, seq_no]);
        if let Some(ref prev) = prev_hash {
            hasher.update(prev);
        }
        let hash = hasher.finalize().to_vec();
        prev_hash = Some(hash.clone());
        pad.extend_from_slice(&hash);
    }
    pad.truncate(body_len);
    pad
}

/// XOR-encrypt or XOR-decrypt a body buffer in-place using the shared secret.
pub fn xor_body(body: &mut [u8], secret: &[u8], session_id: u32, version: u8, seq_no: u8) {
    let pad = compute_pad(secret, session_id, version, seq_no, body.len());
    for (b, p) in body.iter_mut().zip(pad.iter()) {
        *b ^= p;
    }
}

// ── AUTH packets ─────────────────────────────────────────────────────────────

/// Build a full AUTH_START packet (header + body).
/// `flags` controls encryption: use `FLAG_UNENCRYPTED` for tests.
pub fn build_auth_start(
    username: &str,
    password: &str,
    session_id: u32,
    flags: u8,
    secret: &[u8],
) -> Vec<u8> {
    let user_bytes = username.as_bytes();
    let pass_bytes = password.as_bytes();

    // Build body
    let mut body = Vec::new();
    body.push(AUTH_ACTION_LOGIN);
    body.push(AUTH_TYPE_PAP);
    body.push(AUTH_SERVICE_LOGIN);
    body.push(user_bytes.len() as u8);
    body.push(0u8); // port_len
    body.push(0u8); // rem_addr_len
    body.push(pass_bytes.len() as u8);
    body.extend_from_slice(user_bytes);
    body.extend_from_slice(pass_bytes);

    if flags == FLAG_ENCRYPTED {
        xor_body(&mut body, secret, session_id, VERSION, 1);
    }

    let header = build_header(TYPE_AUTH, 1, flags, session_id, body.len() as u32);
    let mut pkt = header.to_vec();
    pkt.extend_from_slice(&body);
    pkt
}

/// Parse an AUTH_REPLY packet and return the status byte.
/// Returns `None` if the buffer is too short or malformed.
pub fn parse_auth_reply(buf: &[u8], secret: &[u8], session_id: u32, flags: u8) -> Option<u8> {
    if buf.len() < 12 {
        return None;
    }
    let header_arr: [u8; 12] = buf[0..12].try_into().ok()?;
    let (_, _, _, _, body_len) = parse_header(&header_arr);
    let end = 12 + body_len as usize;
    if buf.len() < end {
        return None;
    }
    let mut body = buf[12..end].to_vec();
    if flags == FLAG_ENCRYPTED {
        xor_body(&mut body, secret, session_id, VERSION, 2);
    }
    body.first().copied()
}

// ── ACCT packets ─────────────────────────────────────────────────────────────

/// Build a full ACCT_REQUEST packet.
/// `av_pairs` is a list of `"key=value"` strings.
pub fn build_acct_request(
    username: &str,
    acct_flags: u8,
    av_pairs: &[String],
    session_id: u32,
    flags: u8,
    secret: &[u8],
) -> Vec<u8> {
    let user_bytes = username.as_bytes();
    let av_bytes: Vec<Vec<u8>> = av_pairs.iter().map(|s| s.as_bytes().to_vec()).collect();

    let mut body = Vec::new();
    body.push(acct_flags);
    body.push(ACCT_METHOD_TACACS);
    body.push(0x01u8); // priv_lvl
    body.push(AUTH_TYPE_PAP);
    body.push(AUTH_SERVICE_LOGIN);
    body.push(user_bytes.len() as u8);
    body.push(0u8); // port_len
    body.push(0u8); // rem_addr_len
    body.push(av_bytes.len() as u8);

    // username
    body.extend_from_slice(user_bytes);

    // AV pair lengths
    for av in &av_bytes {
        body.push(av.len() as u8);
    }

    // AV pair data
    for av in &av_bytes {
        body.extend_from_slice(av);
    }

    if flags == FLAG_ENCRYPTED {
        xor_body(&mut body, secret, session_id, VERSION, 1);
    }

    let header = build_header(TYPE_ACCT, 1, flags, session_id, body.len() as u32);
    let mut pkt = header.to_vec();
    pkt.extend_from_slice(&body);
    pkt
}

/// Parse an ACCT_REPLY packet and return the status byte.
pub fn parse_acct_reply(buf: &[u8], secret: &[u8], session_id: u32, flags: u8) -> Option<u8> {
    if buf.len() < 12 {
        return None;
    }
    let header_arr: [u8; 12] = buf[0..12].try_into().ok()?;
    let (_, _, _, _, body_len) = parse_header(&header_arr);
    let end = 12 + body_len as usize;
    if buf.len() < end {
        return None;
    }
    let mut body = buf[12..end].to_vec();
    if flags == FLAG_ENCRYPTED {
        xor_body(&mut body, secret, session_id, VERSION, 2);
    }
    body.first().copied()
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trip() {
        let h = build_header(TYPE_AUTH, 1, FLAG_UNENCRYPTED, 0xDEADBEEF, 42);
        let (t, seq, flags, sid, blen) = parse_header(&h);
        assert_eq!(t,     TYPE_AUTH);
        assert_eq!(seq,   1);
        assert_eq!(flags, FLAG_UNENCRYPTED);
        assert_eq!(sid,   0xDEADBEEF);
        assert_eq!(blen,  42);
    }

    #[test]
    fn auth_start_round_trip_unencrypted() {
        let pkt = build_auth_start("alice", "s3cret", 0x1234, FLAG_UNENCRYPTED, b"key");
        // Header version byte
        assert_eq!(pkt[0], VERSION);
        // Type AUTH
        assert_eq!(pkt[1], TYPE_AUTH);
        // seq_no = 1
        assert_eq!(pkt[2], 1);
        // Body starts at byte 12; action=LOGIN
        assert_eq!(pkt[12], AUTH_ACTION_LOGIN);
    }

    #[test]
    fn xor_encrypt_decrypt_is_identity() {
        let secret = b"mysecret";
        let session_id = 0xABCD1234u32;
        let original = b"hello tacacs body";
        let mut buf = original.to_vec();
        xor_body(&mut buf, secret, session_id, VERSION, 1);
        assert_ne!(&buf, original);
        xor_body(&mut buf, secret, session_id, VERSION, 1);
        assert_eq!(&buf, original);
    }
}
```

- [ ] **Step 3: Verify proto module compiles**

Add `pub(crate) mod tacacs_proto;` to `src/drivers/mod.rs` temporarily (will be permanent after Task 2).

```bash
cargo build -p ox_security_auth 2>&1 | grep "^error" | head -10
```
Expected: no errors.

---

## Task 2: `TacacsAuthDriver`

**Files:**
- Modify: `crates/security/ox_security_auth/src/drivers/tacacs.rs`
- Modify: `crates/security/ox_security_auth/src/drivers/mod.rs`

- [ ] **Step 1: Write the failing tests**

APPEND to `crates/security/ox_security_auth/src/drivers/tacacs.rs`:

```rust
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
        build_header, FLAG_UNENCRYPTED, TYPE_AUTH,
        REPLY_STATUS_PASS, REPLY_STATUS_FAIL, REPLY_STATUS_ERROR,
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
        };
        let send_fn: TcpSendFn = Arc::new(move |pkt: Vec<u8>| {
            // Extract session_id from the packet header bytes 4-7
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
        if let AuthResult::Authenticated(p) = result {
            assert_eq!(p.display_name, "alice");
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
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p ox_security_auth tacacs 2>&1 | head -20
```
Expected: FAIL — `TacacsAuthDriver` is a stub; no `TacacsConfig` or `TcpSendFn` type.

- [ ] **Step 3: Implement `src/drivers/tacacs.rs`**

Replace the entire file:

```rust
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
    FLAG_UNENCRYPTED, REPLY_STATUS_PASS,
};

/// Injected TCP send/receive function used in place of a real TCP socket.
/// Accepts the raw packet bytes to send; returns the raw reply bytes.
/// The `Arc<dyn Fn...>` pattern matches the established convention in this codebase
/// (see `DbAuthDriver`, `TotpAuthDriver`).
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
                (username.as_str(), password.expose_secret())
            }
            _ => return AuthResult::Continue,
        };

        // Generate a random session ID for this exchange.
        let session_id: u32 = rand::random();
        let secret_bytes = self.config.secret.expose_secret().as_bytes().to_vec();

        let pkt = build_auth_start(
            username,
            password,
            session_id,
            FLAG_UNENCRYPTED,
            &secret_bytes,
        );

        let reply = match (self.send_fn)(pkt).await {
            Ok(r) => r,
            Err(_) => {
                return AuthResult::Reject(
                    format!("TACACS+ server '{}' unreachable", self.config.server)
                );
            }
        };

        let status = match parse_auth_reply(&reply, &secret_bytes, session_id, FLAG_UNENCRYPTED) {
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
        build_header, FLAG_UNENCRYPTED, TYPE_AUTH,
        REPLY_STATUS_PASS, REPLY_STATUS_FAIL, REPLY_STATUS_ERROR,
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
}
```

- [ ] **Step 4: Update `src/drivers/mod.rs`**

Add the proto module declaration:

```rust
pub(crate) mod ad;
pub(crate) mod api_key;
pub(crate) mod db;
pub(crate) mod kerberos;
pub(crate) mod ldap;
pub(crate) mod radius;
pub(crate) mod tacacs;
pub(crate) mod tacacs_proto;
pub(crate) mod totp;

pub use ad::AdAuthDriver;
pub use api_key::ApiKeyAuthDriver;
pub use db::DbAuthDriver;
pub use kerberos::KerberosAuthDriver;
pub use ldap::LdapAuthDriver;
pub use radius::RadiusAuthDriver;
pub use tacacs::TacacsAuthDriver;
pub use totp::TotpAuthDriver;
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p ox_security_auth tacacs 2>&1 | tail -15
```
Expected output:
```
test drivers::tacacs::tests::tacacs_authenticates_on_pass_reply ... ok
test drivers::tacacs::tests::tacacs_continues_for_non_password_creds ... ok
test drivers::tacacs::tests::tacacs_rejects_on_error_reply ... ok
test drivers::tacacs::tests::tacacs_rejects_on_fail_reply ... ok
test drivers::tacacs_proto::tests::auth_start_round_trip_unencrypted ... ok
test drivers::tacacs_proto::tests::header_round_trip ... ok
test drivers::tacacs_proto::tests::xor_encrypt_decrypt_is_identity ... ok

test result: ok. 7 passed; 0 failed
```

- [ ] **Step 6: Verify full build is clean**

```bash
cargo build -p ox_security_auth 2>&1 | grep "^error" | head -5
```
Expected: no output.

- [ ] **Step 7: Commit**

```bash
git add crates/security/ox_security_auth
git commit -m "feat(security-auth): implement TacacsAuthDriver with PAP auth and injected TCP send"
```

---

## Task 3: `TacacsAccountingDriver` in `ox_security_accounting`

**Files:**
- Modify: `crates/security/ox_security_accounting/Cargo.toml`
- Create: `crates/security/ox_security_accounting/src/drivers/tacacs.rs`
- Modify: `crates/security/ox_security_accounting/src/drivers/mod.rs`
- Modify: `crates/security/ox_security_accounting/src/lib.rs`

The accounting driver needs its own copy of the packet builder because
`tacacs_proto` lives in `ox_security_auth` (a different crate). The ACCT packet
builder is small — duplicate only the ACCT-relevant functions into a private
`acct_proto` module inside this file, keeping the dependency graph clean.

- [ ] **Step 1: Add dependencies to `ox_security_accounting/Cargo.toml`**

```toml
[package]
name = "ox_security_accounting"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[dependencies]
ox_security_core = { path = "../ox_security_core" }
async-trait      = "0.1"
serde_json       = "1"
md-5             = "0.10"
rand             = "0.8"
futures          = "0.3"

[dev-dependencies]
tokio    = { version = "1", features = ["macros", "rt"] }
tempfile = "3"
chrono   = "0.4"
```

- [ ] **Step 2: Write the failing tests**

APPEND to `crates/security/ox_security_accounting/tests/integration.rs`:

```rust
// ── Task 4 tests: TacacsAccountingDriver ─────────────────────────────────────

use ox_security_accounting::drivers::TacacsAccountingDriver;
use ox_security_accounting::drivers::tacacs::TacacsTcpSendFn;
use ox_security_core::accounting::AuthOutcome;
use secrecy::SecretString;
use std::sync::{Arc, Mutex};

fn tacacs_driver_with_capture() -> (TacacsAccountingDriver, Arc<Mutex<Vec<Vec<u8>>>>) {
    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();

    let send_fn: TacacsTcpSendFn = Arc::new(move |pkt: Vec<u8>| {
        captured_clone.lock().unwrap().push(pkt);
        // Return a minimal ACCT_REPLY: header (12) + body [status=SUCCESS, 0,0,0,0]
        let status = 0x01u8;
        let body = vec![status, 0x00, 0x00, 0x00, 0x00];
        let mut reply = vec![
            0xC1, 0x03, 0x02, 0x04,  // version, TYPE_ACCT, seq=2, flags=UNENCRYPTED
            0x00, 0x00, 0x00, 0x00,  // session_id (ignored in fire-and-forget)
            0x00, 0x00, 0x00, 0x05,  // body_len = 5
        ];
        reply.extend_from_slice(&body);
        Box::pin(async move { Ok(reply) })
    });

    let config = ox_security_accounting::drivers::tacacs::TacacsAccountingConfig {
        server: "127.0.0.1:49".to_string(),
        secret: SecretString::new("test".to_string()),
        timeout_secs: 5,
    };

    (TacacsAccountingDriver::new(config, send_fn), captured)
}

#[tokio::test]
async fn tacacs_accounting_sends_event_on_success() {
    let (driver, captured) = tacacs_driver_with_capture();
    driver.record(&test_event()).await;
    let pkts = captured.lock().unwrap();
    assert_eq!(pkts.len(), 1, "one ACCT_REQUEST packet must be sent");
    // packet type byte (index 1) must be 0x03 (ACCT)
    assert_eq!(pkts[0][1], 0x03, "packet type must be ACCT");
}

#[tokio::test]
async fn tacacs_accounting_ignores_send_failure() {
    let send_fn: TacacsTcpSendFn = Arc::new(|_pkt: Vec<u8>| {
        Box::pin(async { Err("connection refused".to_string()) })
    });
    let config = ox_security_accounting::drivers::tacacs::TacacsAccountingConfig {
        server: "127.0.0.1:49".to_string(),
        secret: SecretString::new("test".to_string()),
        timeout_secs: 5,
    };
    let driver = TacacsAccountingDriver::new(config, send_fn);
    // Must not panic or propagate error
    driver.record(&test_event()).await;
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test -p ox_security_accounting tacacs 2>&1 | head -20
```
Expected: FAIL — `TacacsAccountingDriver` does not exist.

- [ ] **Step 4: Create `src/drivers/tacacs.rs`**

```rust
//! TACACS+ accounting driver.
//!
//! Sends an ACCT_REQUEST for every `AccountingEvent`.  The send is fire-and-forget:
//! any I/O error or malformed reply is silently swallowed so the accounting
//! pipeline never blocks the request path.

use std::sync::Arc;
use async_trait::async_trait;
use futures::future::BoxFuture;
use secrecy::{ExposeSecret, SecretString};
use ox_security_core::accounting::AccountingEvent;
use ox_security_core::drivers::AccountingDriver;

// ── Inline ACCT packet primitives ────────────────────────────────────────────

const VERSION: u8          = 0xC1;
const TYPE_ACCT: u8        = 0x03;
const FLAG_UNENCRYPTED: u8 = 0x04;
const ACCT_FLAG_STOP: u8   = 0x04;
const ACCT_METHOD_TACACS: u8 = 0x06;
const AUTH_TYPE_PAP: u8    = 0x02;
const AUTH_SERVICE_LOGIN: u8 = 0x01;

fn build_header(pkt_type: u8, seq_no: u8, flags: u8, session_id: u32, body_len: u32) -> [u8; 12] {
    let mut h = [0u8; 12];
    h[0] = VERSION;
    h[1] = pkt_type;
    h[2] = seq_no;
    h[3] = flags;
    h[4..8].copy_from_slice(&session_id.to_be_bytes());
    h[8..12].copy_from_slice(&body_len.to_be_bytes());
    h
}

fn build_acct_request(username: &str, av_pairs: &[String], session_id: u32) -> Vec<u8> {
    let user_bytes = username.as_bytes();
    let av_bytes: Vec<Vec<u8>> = av_pairs.iter().map(|s| s.as_bytes().to_vec()).collect();

    let mut body = Vec::new();
    body.push(ACCT_FLAG_STOP);
    body.push(ACCT_METHOD_TACACS);
    body.push(0x01u8); // priv_lvl
    body.push(AUTH_TYPE_PAP);
    body.push(AUTH_SERVICE_LOGIN);
    body.push(user_bytes.len() as u8);
    body.push(0u8); // port_len
    body.push(0u8); // rem_addr_len
    body.push(av_bytes.len() as u8);
    body.extend_from_slice(user_bytes);
    for av in &av_bytes {
        body.push(av.len() as u8);
    }
    for av in &av_bytes {
        body.extend_from_slice(av);
    }

    let header = build_header(TYPE_ACCT, 1, FLAG_UNENCRYPTED, session_id, body.len() as u32);
    let mut pkt = header.to_vec();
    pkt.extend_from_slice(&body);
    pkt
}

// ── Public types ─────────────────────────────────────────────────────────────

/// Injected TCP send function (same shape as the auth-side `TcpSendFn`).
pub type TacacsTcpSendFn = Arc<
    dyn Fn(Vec<u8>) -> BoxFuture<'static, Result<Vec<u8>, String>>
    + Send
    + Sync
>;

pub struct TacacsAccountingConfig {
    /// TACACS+ server address, e.g. `"tacacs.example.com:49"`.
    pub server: String,
    /// Shared secret configured on the TACACS+ server.
    pub secret: SecretString,
    /// Connection timeout in seconds.
    pub timeout_secs: u64,
}

/// TACACS+ accounting driver.
///
/// Builds an ACCT_REQUEST packet from the `AccountingEvent` and sends it via
/// the injected `TacacsTcpSendFn`.  All errors are silently swallowed
/// (fire-and-forget) so a TACACS+ server failure never propagates to the caller.
pub struct TacacsAccountingDriver {
    config: TacacsAccountingConfig,
    send_fn: TacacsTcpSendFn,
}

impl TacacsAccountingDriver {
    pub fn new(config: TacacsAccountingConfig, send_fn: TacacsTcpSendFn) -> Self {
        Self { config, send_fn }
    }
}

#[async_trait]
impl AccountingDriver for TacacsAccountingDriver {
    async fn record(&self, event: &AccountingEvent) {
        let username = event
            .principal_id
            .as_ref()
            .map(|p| p.as_uuid().to_string())
            .unwrap_or_default();

        let av_pairs = vec![
            format!("service=shell"),
            format!("tenant_id={}", event.tenant_id.as_str()),
            format!("source_ip={}", event.source_ip),
            format!("auth_outcome={:?}", event.auth_outcome),
        ];

        let session_id: u32 = rand::random();
        let pkt = build_acct_request(&username, &av_pairs, session_id);

        // Fire-and-forget: ignore all errors.
        let _ = (self.send_fn)(pkt).await;
    }
}
```

- [ ] **Step 5: Update `src/drivers/mod.rs`**

```rust
pub(crate) mod db;
pub(crate) mod file;
pub(crate) mod memory;
pub(crate) mod syslog;
pub(crate) mod tacacs;

pub use db::DbAccountingDriver;
pub use db::RecordFn;
pub use file::FileAccountingDriver;
pub use memory::MemoryAccountingDriver;
pub use syslog::SyslogAccountingDriver;
pub use tacacs::TacacsAccountingDriver;
```

- [ ] **Step 6: Update `src/lib.rs`**

```rust
pub mod drivers;
pub(crate) mod event_serializer;
pub(crate) mod pipeline;

pub use drivers::{
    DbAccountingDriver, FileAccountingDriver, MemoryAccountingDriver,
    SyslogAccountingDriver, TacacsAccountingDriver,
};
pub use pipeline::AccountingPipeline;
```

- [ ] **Step 7: Add `secrecy` dep to accounting crate**

In `crates/security/ox_security_accounting/Cargo.toml` add:

```toml
secrecy = { version = "0.8", features = ["serde"] }
```

- [ ] **Step 8: Run tests to verify they pass**

```bash
cargo test -p ox_security_accounting 2>&1 | tail -15
```
Expected output (all prior tests still pass):
```
test tacacs_accounting_ignores_send_failure ... ok
test tacacs_accounting_sends_event_on_success ... ok
test db_driver_calls_injected_fn ... ok
test file_driver_appends_json_lines ... ok
test file_driver_creates_file_if_missing ... ok
test memory_driver_records_events ... ok
test pipeline_calls_all_drivers_even_when_one_is_noop ... ok
test pipeline_records_to_all_drivers ... ok
test syslog_driver_records_without_error ... ok

test result: ok. 9 passed; 0 failed
```

- [ ] **Step 9: Verify full clean build**

```bash
cargo build -p ox_security_accounting 2>&1 | grep "^error" | head -5
```
Expected: no output.

- [ ] **Step 10: Commit**

```bash
git add crates/security/ox_security_accounting
git commit -m "feat(security-accounting): implement TacacsAccountingDriver with fire-and-forget ACCT_REQUEST"
```

---

## Task 4: Body encryption (production hardening)

**Files:**
- Modify: `crates/security/ox_security_auth/src/drivers/tacacs.rs` — expose `flags` on `TacacsConfig`, default production to `FLAG_ENCRYPTED`
- Modify: `crates/security/ox_security_auth/src/drivers/tacacs_proto.rs` — confirm `xor_body` is exercised via `FLAG_ENCRYPTED` path
- Modify: `crates/security/ox_security_accounting/src/drivers/tacacs.rs` — expose `encrypted: bool` on `TacacsAccountingConfig`

This task converts the drivers from unencrypted test mode to encrypted production mode. No new public API is required — only a flag on config structs.

- [ ] **Step 1: Write the failing tests**

In `crates/security/ox_security_auth/src/drivers/tacacs.rs`, add to the `tests` module:

```rust
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
        // Verify the packet is NOT plaintext PAP (body is XOR'd)
        // We can only verify the header type + that the body differs from plaintext.
        assert_eq!(pkt[1], TYPE_AUTH);
        // The flags byte should be 0x00 (encrypted) not 0x04 (unencrypted).
        assert_eq!(pkt[3], FLAG_ENCRYPTED);

        let session_id = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);

        // Build an encrypted PASS reply
        let mut body = vec![REPLY_STATUS_PASS, 0x00, 0x00, 0x00, 0x00, 0x00];
        use crate::drivers::tacacs_proto::xor_body;
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
```

- [ ] **Step 2: Add `encrypted` field to `TacacsConfig`**

```rust
pub struct TacacsConfig {
    pub server: String,
    pub secret: SecretString,
    pub timeout_secs: u64,
    pub tenant_id: TenantId,
    /// Set to `true` in production to XOR-encrypt the packet body.
    /// Set to `false` (default) only in test environments.
    pub encrypted: bool,
}
```

Update `TacacsAuthDriver::authenticate` to derive `flags`:

```rust
let flags = if self.config.encrypted { FLAG_ENCRYPTED } else { FLAG_UNENCRYPTED };
let pkt = build_auth_start(username, password, session_id, flags, &secret_bytes);
// ...
let status = parse_auth_reply(&reply, &secret_bytes, session_id, flags);
```

Update existing test helpers: `TacacsConfig { ..., encrypted: false }`.

- [ ] **Step 3: Add `encrypted` field to `TacacsAccountingConfig`**

```rust
pub struct TacacsAccountingConfig {
    pub server: String,
    pub secret: SecretString,
    pub timeout_secs: u64,
    pub encrypted: bool,
}
```

Update `build_acct_request` call in `TacacsAccountingDriver::record` to pass the flag.

- [ ] **Step 4: Run all tests**

```bash
cargo test -p ox_security_auth 2>&1 | tail -15
cargo test -p ox_security_accounting 2>&1 | tail -15
```
Expected: all tests pass, including the new `tacacs_authenticates_with_encryption_enabled`.

- [ ] **Step 5: Commit**

```bash
git add crates/security/ox_security_auth crates/security/ox_security_accounting
git commit -m "feat(security-tacacs): add encrypted flag to TacacsConfig and TacacsAccountingConfig for production XOR body encryption"
```
