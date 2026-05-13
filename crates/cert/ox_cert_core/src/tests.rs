#[cfg(test)]
mod tests {
    use crate::model::*;
    use crate::store::{CertStore, OxPersistenceCertStore};
    use crate::keystore::{KeyStore, SoftwareKeyStore};
    use crate::model::KeyStoreConfig;
    use crate::builder::CertBuilder;
    use std::path::PathBuf;
    use time::OffsetDateTime;

    // ---------------------------------------------------------------------------
    // Schema registration
    // ---------------------------------------------------------------------------

    #[test]
    fn test_register_schemas() {
        use ox_data_object_manager::DataDictionary;
        let mut dict = DataDictionary::new();
        crate::register_schemas(&mut dict).expect("schema registration failed");
        assert!(dict.objects.contains_key("certificate"));
        assert!(dict.objects.contains_key("ca_key"));
        assert!(dict.objects.contains_key("acme_account"));
        assert!(dict.objects.contains_key("ra_request"));
        assert!(dict.objects.contains_key("audit_log"));
        assert!(dict.objects.contains_key("ssh_certificate"));
        assert!(dict.objects.contains_key("acme_order"));
        assert!(dict.objects.contains_key("acme_authorization"));
    }

    // ---------------------------------------------------------------------------
    // OxPersistenceCertStore
    // ---------------------------------------------------------------------------

    fn make_store() -> OxPersistenceCertStore {
        OxPersistenceCertStore::open(":memory:").expect("store open failed")
    }

    fn sample_cert(tenant: &str, serial: &str) -> CertificateRecord {
        let now = OffsetDateTime::now_utc();
        CertificateRecord {
            serial: serial.to_string(),
            tenant_id: tenant.to_string(),
            subject_cn: "test.example.com".to_string(),
            subject_dn: "CN=test.example.com, O=Test Org, C=US".to_string(),
            sans: vec!["test.example.com".to_string()],
            issuer_dn: "CN=Test CA, O=Test Org, C=US".to_string(),
            not_before: now,
            not_after: now + time::Duration::days(365),
            key_type: "ecc-p256".to_string(),
            profile: "standard".to_string(),
            pem: "-----BEGIN CERTIFICATE-----\nMIIBIjAN...\n-----END CERTIFICATE-----\n".to_string(),
            csr_pem: None,
            private_key_encrypted: None,
            status: CertStatus::Active,
            revoked_at: None,
            revocation_reason: None,
            scts: vec![],
            policy_oids: vec![],
            enrollment_protocol: Some(EnrollmentProtocol::Rest),
            created_at: now,
        }
    }

    // These roundtrip tests require a real persistence driver; they are
    // ignored until ox_persistence_driver_db_* is wired into the test fixture.

    #[test]
    fn test_store_cert_roundtrip() {
        let store = make_store();
        let cert = sample_cert("acme-corp", "test-serial-001");
        store.store_cert("acme-corp", &cert).expect("store failed");
        let loaded = store.get_cert_by_serial("acme-corp", "test-serial-001")
            .expect("get failed")
            .expect("cert not found");
        assert_eq!(loaded.serial, "test-serial-001");
        assert_eq!(loaded.subject_cn, "test.example.com");
        assert_eq!(loaded.tenant_id, "acme-corp");
    }

    #[test]
    fn test_tenant_isolation() {
        let store = make_store();
        let cert = sample_cert("tenant-a", "isolation-serial-001");
        store.store_cert("tenant-a", &cert).expect("store failed");
        let result = store.get_cert_by_serial("tenant-b", "isolation-serial-001")
            .expect("get failed");
        assert!(result.is_none(), "tenant isolation violated");
    }

    #[test]
    fn test_get_cert_not_found() {
        let store = make_store();
        // Returns None (not an error) when not found
        let result = store.get_cert_by_serial("acme-corp", "nonexistent-serial")
            .expect("unexpected error");
        assert!(result.is_none());
    }

    #[test]
    fn test_mark_revoked() {
        let store = make_store();
        let cert = sample_cert("acme-corp", "revoke-serial-001");
        store.store_cert("acme-corp", &cert).expect("store failed");
        let ts = OffsetDateTime::now_utc();
        store.mark_revoked("acme-corp", "revoke-serial-001", RevocationReason::KeyCompromise, ts)
            .expect("revoke failed");
        let loaded = store.get_cert_by_serial("acme-corp", "revoke-serial-001")
            .expect("get failed")
            .expect("cert not found after revoke");
        assert_eq!(loaded.status, CertStatus::Revoked);
        assert_eq!(loaded.revocation_reason, Some(RevocationReason::KeyCompromise));
        assert!(loaded.revoked_at.is_some());
    }

    #[test]
    fn test_migrate_ok_after_open() {
        let store = make_store();
        store.migrate().expect("migrate should be ok after open()");
    }

    #[test]
    fn test_store_cert_no_panic() {
        // Verifies store_cert doesn't panic even with placeholder persistence.
        let store = make_store();
        let cert = sample_cert("acme-corp", "no-panic-serial");
        let _ = store.store_cert("acme-corp", &cert); // may return Ok or Err
    }

    // ---------------------------------------------------------------------------
    // CA key store roundtrip
    // ---------------------------------------------------------------------------

    fn sample_ca_key(tenant: &str, id: &str) -> CaKeyRecord {
        let now = OffsetDateTime::now_utc();
        CaKeyRecord {
            id: id.to_string(),
            tenant_id: tenant.to_string(),
            key_type: KeyType::EcP384,
            cert_pem: "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n".to_string(),
            key_ref: format!("{}/{}.key.pem", tenant, id),
            status: CaKeyStatus::Active,
            not_before: now,
            not_after: now + time::Duration::days(3650),
            name_constraints: None,
            path_length: Some(0),
            created_at: now,
        }
    }

    #[test]
    fn test_store_ca_key_roundtrip() {
        let store = make_store();
        let key = sample_ca_key("acme-corp", "intermediate-2026");
        store.store_ca_key("acme-corp", &key).expect("store ca_key failed");
        let loaded = store.get_ca_key_by_id("acme-corp", "intermediate-2026")
            .expect("get failed")
            .expect("ca key not found");
        assert_eq!(loaded.id, "intermediate-2026");
        assert_eq!(loaded.key_type, KeyType::EcP384);
        assert_eq!(loaded.status, CaKeyStatus::Active);
    }

    #[test]
    fn test_update_ca_key_status() {
        let store = make_store();
        let key = sample_ca_key("acme-corp", "retiring-key-001");
        store.store_ca_key("acme-corp", &key).expect("store failed");
        store.update_ca_key_status("acme-corp", "retiring-key-001", CaKeyStatus::Retiring)
            .expect("update status failed");
        let loaded = store.get_ca_key_by_id("acme-corp", "retiring-key-001")
            .expect("get failed")
            .expect("not found");
        assert_eq!(loaded.status, CaKeyStatus::Retiring);
    }

    // ---------------------------------------------------------------------------
    // ACME roundtrip
    // ---------------------------------------------------------------------------

    #[test]
    fn test_acme_account_roundtrip() {
        let store = make_store();
        let now = OffsetDateTime::now_utc();
        let acct = AcmeAccount {
            id: "acct-001".to_string(),
            tenant_id: "acme-corp".to_string(),
            jwk: r#"{"kty":"EC"}"#.to_string(),
            contact: vec!["mailto:admin@example.com".to_string()],
            status: AcmeAccountStatus::Valid,
            eab_kid: None,
            created_at: now,
        };
        store.store_acme_account("acme-corp", &acct).expect("store failed");
        let loaded = store.get_acme_account("acme-corp", "acct-001")
            .expect("get failed")
            .expect("not found");
        assert_eq!(loaded.id, "acct-001");
        assert_eq!(loaded.status, AcmeAccountStatus::Valid);
    }

    // ---------------------------------------------------------------------------
    // RA request roundtrip
    // ---------------------------------------------------------------------------

    #[test]
    fn test_ra_request_roundtrip() {
        let store = make_store();
        let now = OffsetDateTime::now_utc();
        let req = ApprovalRequest {
            id: "ra-001".to_string(),
            tenant_id: "acme-corp".to_string(),
            csr_pem: "-----BEGIN CERTIFICATE REQUEST-----\n...\n-----END CERTIFICATE REQUEST-----\n".to_string(),
            requester_identity: "192.168.1.1".to_string(),
            profile: "standard".to_string(),
            sans: vec!["app.example.com".to_string()],
            status: ApprovalStatus::Pending,
            reviewer: None,
            review_notes: None,
            reviewed_at: None,
            certificate_serial: None,
            created_at: now,
        };
        store.store_ra_request("acme-corp", &req).expect("store failed");
        let loaded = store.get_ra_request("acme-corp", "ra-001")
            .expect("get failed")
            .expect("not found");
        assert_eq!(loaded.id, "ra-001");
        assert_eq!(loaded.status, ApprovalStatus::Pending);

        store.update_ra_request(
            "acme-corp", "ra-001",
            ApprovalStatus::Approved,
            "admin@example.com",
            "Approved per policy",
        ).expect("update failed");

        let updated = store.get_ra_request("acme-corp", "ra-001")
            .expect("get failed").expect("not found");
        assert_eq!(updated.status, ApprovalStatus::Approved);
        assert_eq!(updated.reviewer, Some("admin@example.com".to_string()));
    }

    // ---------------------------------------------------------------------------
    // Audit log
    // ---------------------------------------------------------------------------

    #[test]
    fn test_audit_event_store() {
        let store = make_store();
        let now = OffsetDateTime::now_utc();
        let event = AuditEvent {
            id: 1,
            tenant_id: "acme-corp".to_string(),
            timestamp: now,
            action: AuditAction::Issue,
            serial: Some("audit-cert-001".to_string()),
            actor: "192.168.0.1".to_string(),
            details: serde_json::json!({ "profile": "standard" }),
        };
        store.store_audit_event("acme-corp", &event).expect("store audit failed");
    }

    // ---------------------------------------------------------------------------
    // CertBuilder — self-signed cert
    // ---------------------------------------------------------------------------

    #[test]
    fn test_certbuilder_self_signed() {
        use rcgen::KeyPair;

        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("key gen failed");

        let record = CertBuilder::new()
            .subject("CN=Test CA, O=Test Org, C=US")
            .is_ca(true, Some(0))
            .validity_seconds(365 * 86400)
            .self_sign("acme-corp", &key)
            .expect("self-sign failed");

        assert_eq!(record.subject_cn, "Test CA");
        assert!(record.pem.contains("BEGIN CERTIFICATE"));
        assert_eq!(record.status, CertStatus::Active);
        assert_eq!(record.tenant_id, "acme-corp");
    }

    #[test]
    fn test_certbuilder_leaf_signed_by_ca() {
        use rcgen::KeyPair;

        let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("ca key gen failed");
        let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("leaf key gen failed");

        let ca_builder = CertBuilder::new()
            .subject("CN=Test CA, O=Test Org, C=US")
            .is_ca(true, Some(0));
        let ca_params = ca_builder.build_params().expect("ca params failed");

        let leaf_record = CertBuilder::new()
            .subject("CN=app.example.com, O=Test Org, C=US")
            .sans(vec![SanType::Dns("app.example.com".to_string())])
            .validity_seconds(90 * 86400)
            .sign_with_issuer("acme-corp", &leaf_key, &ca_params, &ca_key)
            .expect("leaf sign failed");

        assert_eq!(leaf_record.subject_cn, "app.example.com");
        assert!(leaf_record.pem.contains("BEGIN CERTIFICATE"));
        assert!(!leaf_record.sans.is_empty());
    }

    // ---------------------------------------------------------------------------
    // SoftwareKeyStore
    // ---------------------------------------------------------------------------

    #[test]
    fn test_software_keystore_generate_and_exists() {
        let dir = tempdir();
        let config = KeyStoreConfig {
            store_type: crate::model::KeyStoreType::Software,
            key_dir: Some(PathBuf::from(&dir)),
            passphrase_env: None,
            pkcs11_module: None,
            pkcs11_slot: None,
            pkcs11_pin_env: None,
        };
        let ks = SoftwareKeyStore::open(&config).expect("open failed");
        assert!(!ks.key_exists("tenant1", "mykey").unwrap());
        ks.generate_key("tenant1", "mykey", KeyType::EcP256, false)
            .expect("generate failed");
        assert!(ks.key_exists("tenant1", "mykey").unwrap());
        let info = ks.key_info("tenant1", "mykey").expect("key_info failed");
        assert_eq!(info.key_id, "mykey");
        assert_eq!(info.key_type, KeyType::EcP256);
        ks.delete_key("tenant1", "mykey").expect("delete failed");
        assert!(!ks.key_exists("tenant1", "mykey").unwrap());
        cleanup_dir(&dir);
    }

    #[test]
    fn test_software_keystore_ed25519() {
        let dir = tempdir();
        let config = KeyStoreConfig {
            store_type: crate::model::KeyStoreType::Software,
            key_dir: Some(PathBuf::from(&dir)),
            passphrase_env: None,
            pkcs11_module: None,
            pkcs11_slot: None,
            pkcs11_pin_env: None,
        };
        let ks = SoftwareKeyStore::open(&config).expect("open failed");
        ks.generate_key("t1", "ed-key", KeyType::Ed25519, false)
            .expect("generate ed25519 failed");
        assert!(ks.key_exists("t1", "ed-key").unwrap());
        cleanup_dir(&dir);
    }

    #[test]
    fn test_software_keystore_p521_returns_error() {
        let dir = tempdir();
        let config = KeyStoreConfig {
            store_type: crate::model::KeyStoreType::Software,
            key_dir: Some(PathBuf::from(&dir)),
            passphrase_env: None,
            pkcs11_module: None,
            pkcs11_slot: None,
            pkcs11_pin_env: None,
        };
        let ks = SoftwareKeyStore::open(&config).expect("open failed");
        let result = ks.generate_key("t1", "p521-key", KeyType::EcP521, false);
        assert!(result.is_err(), "P-521 should return an error without aws_lc_rs");
        cleanup_dir(&dir);
    }

    // ---------------------------------------------------------------------------
    // Model types
    // ---------------------------------------------------------------------------

    #[test]
    fn test_cert_status_serde() {
        let s = serde_json::to_string(&CertStatus::Revoked).unwrap();
        let d: CertStatus = serde_json::from_str(&s).unwrap();
        assert_eq!(d, CertStatus::Revoked);
    }

    #[test]
    fn test_revocation_reason_repr() {
        assert_eq!(RevocationReason::KeyCompromise as u8, 1);
        assert_eq!(RevocationReason::CaCompromise as u8, 2);
        assert_eq!(RevocationReason::CessationOfOperation as u8, 5);
        assert_eq!(RevocationReason::CertificateHold as u8, 6);
    }

    #[test]
    fn test_issuance_policy_from_config() {
        let cfg = IssuancePolicyConfig {
            domain_allowlist: vec![".*\\.example\\.com$".to_string()],
            domain_blocklist: vec!["malicious\\.example\\.com$".to_string()],
            max_san_count: 5,
            wildcard_allowed: false,
            min_rsa_bits: 2048,
            require_ra_approval: false,
        };
        let policy = IssuancePolicy::from_config(&cfg).expect("policy parse failed");
        assert_eq!(policy.max_san_count, 5);
        assert!(!policy.wildcard_allowed);
        assert_eq!(policy.domain_allowlist.len(), 1);
        assert!(policy.domain_allowlist[0].is_match("app.example.com"));
        assert!(policy.domain_blocklist[0].is_match("malicious.example.com"));
    }

    // ---------------------------------------------------------------------------
    // Cross-sign: P-384 CSR with CA extensions (simulates ICA → RCA)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_cross_sign_p384_csr_with_ca_extensions() {
        use rcgen::KeyPair;
        use std::process::Command;

        // Skip if openssl not available
        if Command::new("openssl").arg("version").output().is_err() {
            eprintln!("openssl not found, skipping");
            return;
        }

        let dir = tempdir();
        let key_path = format!("{}/ica.key", dir);
        let csr_path = format!("{}/ica.csr", dir);
        let ext_conf = format!("{}/ext.cnf", dir);

        // Write openssl ext.cnf with CA extensions (same as generate-root-ca.sh)
        std::fs::write(&ext_conf, r#"
[req]
distinguished_name = req_dn
req_extensions     = v3_ca_req
prompt             = no

[req_dn]

[v3_ca_req]
basicConstraints = critical,CA:true,pathlen:1
keyUsage         = critical,keyCertSign,cRLSign
"#).unwrap();

        // Generate P-384 key
        let status = Command::new("openssl")
            .args(["genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-384", "-out", &key_path])
            .output().expect("openssl genpkey failed");
        assert!(status.status.success(), "genpkey failed: {}", String::from_utf8_lossy(&status.stderr));

        // Generate CSR with CA extensions
        let status = Command::new("openssl")
            .args(["req", "-new", "-key", &key_path, "-subj", "/CN=Test ICA,O=Test,C=US", "-out", &csr_path, "-config", &ext_conf])
            .output().expect("openssl req failed");
        assert!(status.status.success(), "req failed: {}", String::from_utf8_lossy(&status.stderr));

        let csr_pem = std::fs::read_to_string(&csr_path).expect("read csr");

        // Build a root CA to sign with
        let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)
            .expect("ca key gen failed");
        let ca_params = CertBuilder::new()
            .subject("CN=Test Root CA, O=Test, C=US")
            .is_ca(true, None)
            .build_params()
            .expect("ca params failed");

        // This is the exact call that handle_cross_sign makes
        let result = crate::builder::cross_sign_csr(
            &csr_pem,
            "default",
            3 * 365 * 86400,
            &ca_params,
            &ca_key,
        );

        let record = match result {
            Ok(r) => r,
            Err(e) => { cleanup_dir(&dir); panic!("cross-sign failed: {}", e); }
        };
        assert!(record.pem.contains("BEGIN CERTIFICATE"), "cert PEM missing");
        assert_eq!(record.profile, "ca_intermediate");

        // Verify the public key in the signed cert matches the original private key.
        let cert_path = format!("{}/signed.crt", dir);
        std::fs::write(&cert_path, &record.pem).unwrap();

        let cert_pubkey = Command::new("openssl")
            .args(["x509", "-in", &cert_path, "-noout", "-pubkey"])
            .output().expect("openssl x509 failed");
        let key_pubkey = Command::new("openssl")
            .args(["pkey", "-in", &key_path, "-pubout"])
            .output().expect("openssl pkey failed");

        cleanup_dir(&dir);

        let cert_pk = String::from_utf8_lossy(&cert_pubkey.stdout);
        let key_pk = String::from_utf8_lossy(&key_pubkey.stdout);
        assert!(cert_pubkey.status.success(), "openssl x509 -pubkey failed: {}", String::from_utf8_lossy(&cert_pubkey.stderr));
        assert!(key_pubkey.status.success(), "openssl pkey -pubout failed: {}", String::from_utf8_lossy(&key_pubkey.stderr));
        assert_eq!(cert_pk.trim(), key_pk.trim(), "public key in cert does not match original private key");
    }

    // ---------------------------------------------------------------------------
    // Cross-sign: P-384 CSR without CA extensions (production path from generate-root-ca.sh)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_cross_sign_p384_csr_without_ca_extensions() {
        use rcgen::KeyPair;
        use std::process::Command;

        if Command::new("openssl").arg("version").output().is_err() {
            eprintln!("openssl not found, skipping");
            return;
        }

        let dir = tempdir();
        let key_path = format!("{}/ica.key", dir);
        let csr_path = format!("{}/ica.csr", dir);

        // Generate P-384 key via openssl (matches generate-root-ca.sh)
        let out = Command::new("openssl")
            .args(["genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-384", "-out", &key_path])
            .output().expect("openssl genpkey failed");
        assert!(out.status.success(), "genpkey: {}", String::from_utf8_lossy(&out.stderr));

        // Generate plain CSR with no extensions (matches generate-root-ca.sh --csr-only current output)
        let out = Command::new("openssl")
            .args(["req", "-new", "-key", &key_path, "-subj", "/CN=Test ICA,O=Test,C=US", "-out", &csr_path])
            .output().expect("openssl req failed");
        assert!(out.status.success(), "req: {}", String::from_utf8_lossy(&out.stderr));

        let csr_pem = std::fs::read_to_string(&csr_path).expect("read csr");

        let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("ca key gen");
        let ca_params = CertBuilder::new()
            .subject("CN=Test Root CA, O=Test, C=US")
            .is_ca(true, None)
            .build_params()
            .expect("ca params");

        let record = match crate::builder::cross_sign_csr(&csr_pem, "default", 3 * 365 * 86400, &ca_params, &ca_key) {
            Ok(r) => r,
            Err(e) => { cleanup_dir(&dir); panic!("cross-sign failed: {}", e); }
        };

        let cert_path = format!("{}/signed.crt", dir);
        std::fs::write(&cert_path, &record.pem).unwrap();

        let cert_pk_out = Command::new("openssl")
            .args(["x509", "-in", &cert_path, "-noout", "-pubkey"])
            .output().expect("openssl x509");
        let key_pk_out = Command::new("openssl")
            .args(["pkey", "-in", &key_path, "-pubout"])
            .output().expect("openssl pkey");

        cleanup_dir(&dir);

        assert!(cert_pk_out.status.success(), "x509 -pubkey: {}", String::from_utf8_lossy(&cert_pk_out.stderr));
        assert!(key_pk_out.status.success(), "pkey -pubout: {}", String::from_utf8_lossy(&key_pk_out.stderr));
        assert_eq!(
            String::from_utf8_lossy(&cert_pk_out.stdout).trim(),
            String::from_utf8_lossy(&key_pk_out.stdout).trim(),
            "public key in signed cert does not match original private key"
        );
    }

    // ---------------------------------------------------------------------------
    // parse_dn_into — DC component handling
    // ---------------------------------------------------------------------------

    #[test]
    fn test_parse_dn_into_dc_abbreviated_form() {
        // rcgen DistinguishedName uses a HashMap keyed by DnType, so pushing the same OID
        // twice overwrites the first value. Only the last DC survives — this is a known
        // rcgen 0.14 limitation. Use cross_sign_csr_with_pem for multi-valued DC support.
        use crate::builder::{parse_dn_into, dn_to_string_pub};
        let mut dn = rcgen::DistinguishedName::new();
        parse_dn_into(&mut dn, "CN=host.example.com, O=Org, DC=example, DC=com");
        let s = dn_to_string_pub(&dn);
        assert!(s.contains("DC="), "no DC component in: {}", s);
        assert!(s.contains("CN=host.example.com"), "CN missing from: {}", s);
    }

    #[test]
    fn test_parse_dn_into_dc_raw_oid_form() {
        // x509-parser may emit the OID dotted string when the OID isn't in its registry.
        // Verify the raw OID form is recognised and stored as a DC attribute.
        use crate::builder::{parse_dn_into, dn_to_string_pub};
        let mut dn = rcgen::DistinguishedName::new();
        parse_dn_into(&mut dn, "CN=host.example.com, O=Org, 0.9.2342.19200300.100.1.25=example");
        let s = dn_to_string_pub(&dn);
        assert!(s.contains("DC=example"), "DC=example missing from: {}", s);
    }

    #[test]
    fn test_cross_sign_with_pem_preserves_dc_components() {
        use rcgen::KeyPair;
        use std::process::Command;

        if Command::new("openssl").arg("version").output().is_err() {
            eprintln!("openssl not found, skipping");
            return;
        }

        let dir = tempdir();
        let ica_key_path = format!("{}/ica.key", dir);
        let ica_csr_path = format!("{}/ica.csr", dir);
        let ca_key_path  = format!("{}/ca.key", dir);
        let ca_crt_path  = format!("{}/ca.crt", dir);
        let signed_path  = format!("{}/signed.crt", dir);

        // Generate ICA key and CSR with DC components
        let out = Command::new("openssl")
            .args(["genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-384", "-out", &ica_key_path])
            .output().expect("genpkey");
        assert!(out.status.success(), "genpkey: {}", String::from_utf8_lossy(&out.stderr));

        let out = Command::new("openssl")
            .args(["req", "-new", "-key", &ica_key_path,
                   "-subj", "/CN=gagaica01.justlikeef.com/O=Justlikeef/OU=IT-Dept/DC=justlikeef/DC=com",
                   "-out", &ica_csr_path])
            .output().expect("req");
        assert!(out.status.success(), "req: {}", String::from_utf8_lossy(&out.stderr));

        // Build a root CA and export its cert + key as PEM
        let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("ca key gen");
        let ca_record = CertBuilder::new()
            .subject("CN=Test Root CA, O=Test, C=US")
            .is_ca(true, None)
            .self_sign("default", &ca_key)
            .expect("ca self-sign");
        std::fs::write(&ca_crt_path, &ca_record.pem).unwrap();
        std::fs::write(&ca_key_path, ca_key.serialize_pem()).unwrap();

        let csr_pem     = std::fs::read_to_string(&ica_csr_path).expect("read csr");
        let ca_cert_pem = std::fs::read_to_string(&ca_crt_path).expect("read ca cert");
        let ca_key_pem  = std::fs::read_to_string(&ca_key_path).expect("read ca key");

        let record = match crate::builder::cross_sign_csr_with_pem(
            &csr_pem, &ca_cert_pem, &ca_key_pem, "default", 3 * 365,
        ) {
            Ok(r) => r,
            Err(e) => { cleanup_dir(&dir); panic!("cross-sign failed: {}", e); }
        };

        std::fs::write(&signed_path, &record.pem).unwrap();

        let subj_out = Command::new("openssl")
            .args(["x509", "-in", &signed_path, "-noout", "-subject", "-nameopt", "RFC2253"])
            .output().expect("x509 -subject");

        cleanup_dir(&dir);

        assert!(subj_out.status.success(), "x509: {}", String::from_utf8_lossy(&subj_out.stderr));
        let subj_str = String::from_utf8_lossy(&subj_out.stdout).to_string();
        assert!(subj_str.contains("DC=justlikeef"), "DC=justlikeef missing from cert: {}", subj_str);
        assert!(subj_str.contains("DC=com"),        "DC=com missing from cert: {}",        subj_str);
        assert!(record.subject_dn.contains("DC="),  "DC missing from record subject_dn: {}", record.subject_dn);
    }

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    fn tempdir() -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let dir = std::env::temp_dir().join(format!("ox_cert_test_{}", id));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    fn cleanup_dir(dir: &str) {
        let _ = std::fs::remove_dir_all(dir);
    }
}
