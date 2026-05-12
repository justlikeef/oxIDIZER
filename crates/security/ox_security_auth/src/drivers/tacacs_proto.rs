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
///
/// Returns `None` if `username` or `password` exceeds 255 bytes (the maximum
/// encodable in the 1-byte length fields defined by RFC 8907 §5.1).
pub fn build_auth_start(
    username: &str,
    password: &str,
    session_id: u32,
    flags: u8,
    secret: &[u8],
) -> Option<Vec<u8>> {
    let user_bytes = username.as_bytes();
    let pass_bytes = password.as_bytes();

    if user_bytes.len() > 255 || pass_bytes.len() > 255 {
        return None;
    }

    // RFC 8907 §5.1 AUTH_START fixed header (8 bytes):
    //   action(1), priv_lvl(1), authen_type(1), authen_service(1),
    //   user_len(1), port_len(1), rem_addr_len(1), data_len(1)
    let mut body = Vec::new();
    body.push(AUTH_ACTION_LOGIN);
    body.push(0x00u8);          // priv_lvl = 0 (user)
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
    Some(pkt)
}

/// Parse an AUTH_REPLY packet and return the status byte.
/// Returns `None` if the buffer is too short or malformed.
pub fn parse_auth_reply(buf: &[u8], secret: &[u8], session_id: u32, flags: u8) -> Option<u8> {
    if buf.len() < 12 {
        return None;
    }
    if buf[1] != TYPE_AUTH {
        return None;
    }
    let header_arr: [u8; 12] = buf[0..12].try_into().ok()?;
    let (_, _, _, _, body_len) = parse_header(&header_arr);
    let end = 12usize.checked_add(body_len as usize)?;
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
    let end = 12usize.checked_add(body_len as usize)?;
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
        let pkt = build_auth_start("alice", "s3cret", 0x1234, FLAG_UNENCRYPTED, b"key")
            .expect("build_auth_start should succeed for short inputs");
        // Header version byte
        assert_eq!(pkt[0], VERSION);
        // Type AUTH
        assert_eq!(pkt[1], TYPE_AUTH);
        // seq_no = 1
        assert_eq!(pkt[2], 1);
        // Body starts at byte 12; action=LOGIN
        assert_eq!(pkt[12], AUTH_ACTION_LOGIN);
        // RFC 8907 §5.1: byte 13 is priv_lvl (must be 0 for user-level)
        assert_eq!(pkt[13], 0x00);
    }

    #[test]
    fn auth_start_rejects_overlong_username() {
        let long = "a".repeat(256);
        assert!(build_auth_start(&long, "pass", 0x1, FLAG_UNENCRYPTED, b"key").is_none());
    }

    #[test]
    fn auth_start_rejects_overlong_password() {
        let long = "a".repeat(256);
        assert!(build_auth_start("user", &long, 0x1, FLAG_UNENCRYPTED, b"key").is_none());
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
