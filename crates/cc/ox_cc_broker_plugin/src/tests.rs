/// Integration tests for the broker plugin.
///
/// All tests use an in-memory SQLite database (via a temp file) and do not
/// require any network access. Ed25519 / X25519 keys are generated in-process.
use std::io::Write;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde_json::{json, Value};
use tempfile::NamedTempFile;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

use crate::config::{BrokerPluginConfig, ConsumerPolicy};
use crate::db::BrokerDb;
use crate::handlers;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_ed25519_keypair() -> (SigningKey, [u8; 32]) {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    (sk, seed)
}

fn make_x25519_keypair() -> (StaticSecret, X25519PublicKey) {
    let priv_key = StaticSecret::random_from_rng(OsRng);
    let pub_key = X25519PublicKey::from(&priv_key);
    (priv_key, pub_key)
}

/// Create a BrokerDb backed by a temp file, opened with the test encryption key.
fn open_test_db() -> (BrokerDb, NamedTempFile) {
    let tmp = NamedTempFile::new().unwrap();
    let db = BrokerDb::open(tmp.path().to_str().unwrap(), "test-key-do-not-use")
        .expect("BrokerDb::open should succeed");
    (db, tmp)
}

/// Write a 32-byte key to a temp file and return the file.
fn write_key_file(bytes: &[u8]) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(bytes).unwrap();
    f.flush().unwrap();
    f
}

fn make_config(
    db_path: &str,
    signing_key_path: &str,
    enc_key_path: &str,
    payload_dir: &str,
) -> BrokerPluginConfig {
    let mut policy = std::collections::HashMap::new();
    policy.insert(
        "test_consumer".to_string(),
        ConsumerPolicy {
            allowed_payload_keys: vec!["settings".to_string()],
        },
    );
    BrokerPluginConfig {
        db_path: db_path.to_string(),
        db_encryption_key: "test-key-do-not-use".to_string(),
        signing_key_path: signing_key_path.to_string(),
        enc_key_path: enc_key_path.to_string(),
        cipher: "aes256gcm".to_string(),
        pending_ttl_secs: 86_400,
        max_manifest_window_secs: 90 * 24 * 3600,
        payload_dir: payload_dir.to_string(),
        policy,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn test_healthz() {
    let resp = handlers::healthz();
    assert_eq!(resp.status, 200);
    let body: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(body["status"], "ok");
}

#[test]
fn test_register_and_list_clients() {
    let (db, _tmp) = open_test_db();
    let (_, client_pubkey) = make_x25519_keypair();
    let pubkey_b64 = URL_SAFE_NO_PAD.encode(client_pubkey.as_bytes());

    let body = json!({
        "client_id": "host1.example.com",
        "enc_pubkey_b64": pubkey_b64,
        "enrolled_by": "admin-op",
        "notes": "test client"
    })
    .to_string();

    let resp = handlers::register_client(&db, &body);
    assert_eq!(resp.status, 201);
    let body_val: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(body_val["client_id"], "host1.example.com");
    assert_eq!(body_val["status"], "enrolled");

    // List clients
    let list_resp = handlers::list_clients(&db);
    assert_eq!(list_resp.status, 200);
    let list: Value = serde_json::from_str(&list_resp.body).unwrap();
    assert_eq!(list["clients"].as_array().unwrap().len(), 1);
}

#[test]
fn test_register_client_update() {
    let (db, _tmp) = open_test_db();
    let (_, pub1) = make_x25519_keypair();
    let (_, pub2) = make_x25519_keypair();

    let register = |pk: &X25519PublicKey| {
        handlers::register_client(
            &db,
            &json!({
                "client_id": "host1.example.com",
                "enc_pubkey_b64": URL_SAFE_NO_PAD.encode(pk.as_bytes()),
                "enrolled_by": "op"
            })
            .to_string(),
        )
    };

    let r1 = register(&pub1);
    assert_eq!(r1.status, 201); // created

    let r2 = register(&pub2);
    assert_eq!(r2.status, 200); // updated
    let v: Value = serde_json::from_str(&r2.body).unwrap();
    assert_eq!(v["status"], "updated");
}

#[test]
fn test_register_client_bad_pubkey() {
    let (db, _tmp) = open_test_db();
    let resp = handlers::register_client(
        &db,
        &json!({
            "client_id": "bad.example.com",
            "enc_pubkey_b64": "not-valid-base64!!!",
            "enrolled_by": "op"
        })
        .to_string(),
    );
    assert_eq!(resp.status, 422);
}

#[test]
fn test_submit_template_unenrolled_client_rejected() {
    let (signing_sk, signing_seed) = make_ed25519_keypair();
    let (_, enc_pk) = make_x25519_keypair();
    let signing_key_file = write_key_file(&signing_seed);
    let mut enc_seed = [0u8; 32];
    OsRng.fill_bytes(&mut enc_seed);
    let enc_key_file = write_key_file(&enc_seed);

    let payload_dir = tempfile::tempdir().unwrap();
    let (db, db_tmp) = open_test_db();
    let config = make_config(
        db_tmp.path().to_str().unwrap(),
        signing_key_file.path().to_str().unwrap(),
        enc_key_file.path().to_str().unwrap(),
        payload_dir.path().to_str().unwrap(),
    );

    let body = json!({
        "template_id": uuid::Uuid::new_v4().to_string(),
        "consumer": "test_consumer",
        "name": "Test template",
        "description": "A test template",
        "expires_in_secs": 86400,
        "payload": { "settings": {} },
        "client_ids": ["not-enrolled.example.com"],
        "submitted_by": "admin-op"
    })
    .to_string();

    let resp = handlers::submit_template(&db, &config, &body);
    assert_eq!(resp.status, 422);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert!(v["error"].as_str().unwrap().contains("not enrolled"));

    drop(signing_sk); let _ = enc_pk; // suppress unused warnings
}

#[test]
fn test_submit_template_success_and_list_pending() {
    let (_, signing_seed) = make_ed25519_keypair();
    let mut enc_seed = [0u8; 32];
    OsRng.fill_bytes(&mut enc_seed);

    let signing_key_file = write_key_file(&signing_seed);
    let enc_key_file = write_key_file(&enc_seed);
    let payload_dir = tempfile::tempdir().unwrap();
    let (db, db_tmp) = open_test_db();
    let config = make_config(
        db_tmp.path().to_str().unwrap(),
        signing_key_file.path().to_str().unwrap(),
        enc_key_file.path().to_str().unwrap(),
        payload_dir.path().to_str().unwrap(),
    );

    // Enroll a client first
    let (_, client_pubkey) = make_x25519_keypair();
    handlers::register_client(
        &db,
        &json!({
            "client_id": "host1.example.com",
            "enc_pubkey_b64": URL_SAFE_NO_PAD.encode(client_pubkey.as_bytes()),
            "enrolled_by": "op"
        })
        .to_string(),
    );

    let template_id = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "template_id": template_id,
        "consumer": "test_consumer",
        "name": "Deploy settings",
        "description": "Push new settings to host1",
        "expires_in_secs": 86400,
        "payload": { "settings": { "mode": "strict" } },
        "client_ids": ["host1.example.com"],
        "submitted_by": "admin-op"
    })
    .to_string();

    let resp = handlers::submit_template(&db, &config, &body);
    assert_eq!(resp.status, 200, "submit_template body: {}", resp.body);
    let v: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(v["template_id"], template_id);
    assert_eq!(v["status"], "pending");

    // Should appear in list_pending
    let list_resp = handlers::list_pending(&db);
    assert_eq!(list_resp.status, 200);
    let list: Value = serde_json::from_str(&list_resp.body).unwrap();
    let pending = list["pending"].as_array().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["template_id"], template_id);
}

#[test]
fn test_reject_template() {
    let (_, signing_seed) = make_ed25519_keypair();
    let mut enc_seed = [0u8; 32];
    OsRng.fill_bytes(&mut enc_seed);

    let signing_key_file = write_key_file(&signing_seed);
    let enc_key_file = write_key_file(&enc_seed);
    let payload_dir = tempfile::tempdir().unwrap();
    let (db, db_tmp) = open_test_db();
    let config = make_config(
        db_tmp.path().to_str().unwrap(),
        signing_key_file.path().to_str().unwrap(),
        enc_key_file.path().to_str().unwrap(),
        payload_dir.path().to_str().unwrap(),
    );

    let (_, client_pubkey) = make_x25519_keypair();
    handlers::register_client(
        &db,
        &json!({
            "client_id": "host1.example.com",
            "enc_pubkey_b64": URL_SAFE_NO_PAD.encode(client_pubkey.as_bytes()),
            "enrolled_by": "op"
        })
        .to_string(),
    );

    let template_id = uuid::Uuid::new_v4().to_string();
    handlers::submit_template(
        &db,
        &config,
        &json!({
            "template_id": template_id,
            "consumer": "test_consumer",
            "name": "Deploy settings",
            "description": "Push settings",
            "expires_in_secs": 86400,
            "payload": { "settings": {} },
            "client_ids": ["host1.example.com"],
            "submitted_by": "admin-op"
        })
        .to_string(),
    );

    let reject_resp = handlers::reject_template(
        &db,
        &template_id,
        &json!({ "actioned_by": "approver-op", "reason": "test rejection" }).to_string(),
    );
    assert_eq!(reject_resp.status, 200);
    let v: Value = serde_json::from_str(&reject_resp.body).unwrap();
    assert_eq!(v["status"], "rejected");

    // Should no longer appear in pending
    let list: Value = serde_json::from_str(&handlers::list_pending(&db).body).unwrap();
    assert_eq!(list["pending"].as_array().unwrap().len(), 0);
}

#[test]
fn test_approve_template_produces_wire_envelope() {
    let (_, signing_seed) = make_ed25519_keypair();
    let mut enc_seed = [0u8; 32];
    OsRng.fill_bytes(&mut enc_seed);

    let signing_key_file = write_key_file(&signing_seed);
    let enc_key_file = write_key_file(&enc_seed);
    let payload_dir = tempfile::tempdir().unwrap();
    let (db, db_tmp) = open_test_db();
    let config = make_config(
        db_tmp.path().to_str().unwrap(),
        signing_key_file.path().to_str().unwrap(),
        enc_key_file.path().to_str().unwrap(),
        payload_dir.path().to_str().unwrap(),
    );

    let (client_privkey, client_pubkey) = make_x25519_keypair();
    handlers::register_client(
        &db,
        &json!({
            "client_id": "host1.example.com",
            "enc_pubkey_b64": URL_SAFE_NO_PAD.encode(client_pubkey.as_bytes()),
            "enrolled_by": "op"
        })
        .to_string(),
    );

    let template_id = uuid::Uuid::new_v4().to_string();
    handlers::submit_template(
        &db,
        &config,
        &json!({
            "template_id": template_id,
            "consumer": "test_consumer",
            "name": "Deploy settings",
            "description": "Push settings",
            "expires_in_secs": 86400,
            "payload": { "settings": { "mode": "strict" } },
            "client_ids": ["host1.example.com"],
            "submitted_by": "admin-op"
        })
        .to_string(),
    );

    let approve_resp = handlers::approve_template(
        &db,
        &config,
        &template_id,
        &json!({ "actioned_by": "approver-op" }).to_string(),
    );
    assert_eq!(approve_resp.status, 200, "approve body: {}", approve_resp.body);
    let v: Value = serde_json::from_str(&approve_resp.body).unwrap();
    assert_eq!(v["signed_count"], 1);
    assert_eq!(v["failed_client_ids"].as_array().unwrap().len(), 0);

    // Fetch the approved envelope and verify it
    let get_resp = handlers::get_approved(&db, &template_id);
    assert_eq!(get_resp.status, 200);
    let get_val: Value = serde_json::from_str(&get_resp.body).unwrap();
    let envelopes = get_val["envelopes"].as_array().unwrap();
    assert_eq!(envelopes.len(), 1);

    let wire = envelopes[0]["envelope_wire"].as_str()
        .expect("envelope_wire should be a string");

    // Verify the wire format and decrypt using ox_cc_common
    let signing_vk = SigningKey::from_bytes(&signing_seed).verifying_key();
    let manifest = ox_cc_common::verify::verify_and_decrypt(
        wire,
        "host1.example.com",
        &[signing_vk],
        &client_privkey,
        90 * 24 * 3600,
    )
    .expect("verify_and_decrypt should succeed for the produced envelope");

    assert_eq!(manifest.client_id, "host1.example.com");
    assert_eq!(manifest.consumer, "test_consumer");
    assert_eq!(manifest.payload["settings"]["mode"], "strict");

    drop(client_privkey); // suppress warning
}

#[test]
fn test_approve_unenrolled_client_partial_failure() {
    let (_, signing_seed) = make_ed25519_keypair();
    let mut enc_seed = [0u8; 32];
    OsRng.fill_bytes(&mut enc_seed);

    let signing_key_file = write_key_file(&signing_seed);
    let enc_key_file = write_key_file(&enc_seed);
    let payload_dir = tempfile::tempdir().unwrap();
    let (db, db_tmp) = open_test_db();
    let config = make_config(
        db_tmp.path().to_str().unwrap(),
        signing_key_file.path().to_str().unwrap(),
        enc_key_file.path().to_str().unwrap(),
        payload_dir.path().to_str().unwrap(),
    );

    // Enroll host1 but not host2
    let (_, pub1) = make_x25519_keypair();
    handlers::register_client(
        &db,
        &json!({
            "client_id": "host1.example.com",
            "enc_pubkey_b64": URL_SAFE_NO_PAD.encode(pub1.as_bytes()),
            "enrolled_by": "op"
        })
        .to_string(),
    );

    let template_id = uuid::Uuid::new_v4().to_string();

    // Submit template for both clients — policy won't catch host2 since policy
    // checks enrollment but host2 is enrolled in the `clients` table. Instead,
    // let's force partial failure by deleting host1's key after submission.
    // Actually simpler: submit for host1 only, then delete it from clients table,
    // then approve.
    handlers::submit_template(
        &db,
        &config,
        &json!({
            "template_id": template_id,
            "consumer": "test_consumer",
            "name": "Deploy settings",
            "description": "Push settings",
            "expires_in_secs": 86400,
            "payload": { "settings": {} },
            "client_ids": ["host1.example.com"],
            "submitted_by": "admin-op"
        })
        .to_string(),
    );

    // Delete host1 from clients table to force signing failure
    db.conn().execute("DELETE FROM clients WHERE client_id = 'host1.example.com'", []).unwrap();

    let approve_resp = handlers::approve_template(
        &db,
        &config,
        &template_id,
        &json!({ "actioned_by": "approver-op" }).to_string(),
    );
    assert_eq!(approve_resp.status, 200);
    let v: Value = serde_json::from_str(&approve_resp.body).unwrap();
    assert_eq!(v["signed_count"], 0);
    assert_eq!(v["status"], "partially_approved");
    assert_eq!(v["failed_client_ids"].as_array().unwrap().len(), 1);
}

#[test]
fn test_audit_log_populated() {
    let (_, signing_seed) = make_ed25519_keypair();
    let mut enc_seed = [0u8; 32];
    OsRng.fill_bytes(&mut enc_seed);

    let signing_key_file = write_key_file(&signing_seed);
    let enc_key_file = write_key_file(&enc_seed);
    let payload_dir = tempfile::tempdir().unwrap();
    let (db, db_tmp) = open_test_db();
    let config = make_config(
        db_tmp.path().to_str().unwrap(),
        signing_key_file.path().to_str().unwrap(),
        enc_key_file.path().to_str().unwrap(),
        payload_dir.path().to_str().unwrap(),
    );

    let (_, pub1) = make_x25519_keypair();
    handlers::register_client(
        &db,
        &json!({
            "client_id": "host1.example.com",
            "enc_pubkey_b64": URL_SAFE_NO_PAD.encode(pub1.as_bytes()),
            "enrolled_by": "op"
        })
        .to_string(),
    );

    let template_id = uuid::Uuid::new_v4().to_string();
    handlers::submit_template(
        &db,
        &config,
        &json!({
            "template_id": template_id,
            "consumer": "test_consumer",
            "name": "Deploy settings",
            "description": "Push settings",
            "expires_in_secs": 86400,
            "payload": { "settings": {} },
            "client_ids": ["host1.example.com"],
            "submitted_by": "admin-op"
        })
        .to_string(),
    );

    let audit_resp = handlers::query_audit(&db);
    assert_eq!(audit_resp.status, 200);
    let v: Value = serde_json::from_str(&audit_resp.body).unwrap();
    let entries = v["audit"].as_array().unwrap();
    // Should have at least: enroll_client + submit_template
    assert!(entries.len() >= 2);

    let actions: Vec<&str> = entries
        .iter()
        .filter_map(|e| e["action"].as_str())
        .collect();
    assert!(actions.contains(&"enroll_client"));
    assert!(actions.contains(&"submit_template"));
}

#[test]
fn test_sessions_table_exists() {
    let (db, _tmp) = open_test_db();
    // Should be able to insert into sessions table without error
    db.conn().execute(
        "INSERT INTO sessions
         (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status)
         VALUES ('s1', '2026-01-01T00:00:00Z', 'alice', '[]', '[]', 'pending')",
        [],
    ).unwrap();

    let count: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM sessions WHERE session_id = 's1'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_sessions_token_is_unique() {
    let (db, _tmp) = open_test_db();
    db.conn().execute(
        "INSERT INTO sessions (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status, token)
         VALUES ('s1', '2026-01-01T00:00:00Z', 'alice', '[]', '[]', 'approved', 'tok1')",
        [],
    ).unwrap();

    // Second insert with same token should fail
    let result = db.conn().execute(
        "INSERT INTO sessions (session_id, submitted_at, submitted_by, client_ids, allowed_commands, status, token)
         VALUES ('s2', '2026-01-01T00:00:00Z', 'bob', '[]', '[]', 'approved', 'tok1')",
        [],
    );
    assert!(result.is_err(), "duplicate token should violate UNIQUE constraint");
}
