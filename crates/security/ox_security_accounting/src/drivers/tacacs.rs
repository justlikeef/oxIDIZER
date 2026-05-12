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

const VERSION: u8            = 0xC1;
const TYPE_ACCT: u8          = 0x03;
const FLAG_UNENCRYPTED: u8   = 0x04;
const FLAG_ENCRYPTED: u8     = 0x00;
const ACCT_FLAG_STOP: u8     = 0x04;
const ACCT_METHOD_TACACS: u8 = 0x06;
const AUTH_TYPE_PAP: u8      = 0x02;
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

fn compute_pad(secret: &[u8], session_id: u32, version: u8, seq_no: u8, body_len: usize) -> Vec<u8> {
    use md5::{Md5, Digest};
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

fn xor_body(body: &mut [u8], secret: &[u8], session_id: u32, version: u8, seq_no: u8) {
    let pad = compute_pad(secret, session_id, version, seq_no, body.len());
    for (b, p) in body.iter_mut().zip(pad.iter()) {
        *b ^= p;
    }
}

fn build_acct_request(
    username: &str,
    av_pairs: &[String],
    session_id: u32,
    flags: u8,
    secret: &[u8],
) -> Vec<u8> {
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

    if flags == FLAG_ENCRYPTED {
        xor_body(&mut body, secret, session_id, VERSION, 1);
    }

    let header = build_header(TYPE_ACCT, 1, flags, session_id, body.len() as u32);
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
    /// Set to `true` in production to XOR-encrypt the packet body.
    /// Set to `false` (default) only in test environments.
    pub encrypted: bool,
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
        let flags = if self.config.encrypted { FLAG_ENCRYPTED } else { FLAG_UNENCRYPTED };
        let secret_bytes = self.config.secret.expose_secret().as_bytes().to_vec();
        let pkt = build_acct_request(&username, &av_pairs, session_id, flags, &secret_bytes);

        // Fire-and-forget: ignore all errors.
        let _ = (self.send_fn)(pkt).await;
    }
}
