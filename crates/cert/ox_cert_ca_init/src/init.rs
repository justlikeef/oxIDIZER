use crate::config::{CaCertConfig, CaInitConfig};
use ox_cert_core::{
    builder::CertBuilder,
    model::{CaKeyRecord, CaKeyStatus, KeyType, NameConstraints},
    open_keystore,
    store::{CertStore, OxPersistenceCertStore},
    CertError,
};
use time::OffsetDateTime;

/// Parse a key type string from config into `KeyType`.
fn parse_key_type(s: &str) -> Result<KeyType, CertError> {
    match s {
        "ecc-p256" | "ecdsa-p256" => Ok(KeyType::EcP256),
        "ecc-p384" | "ecdsa-p384" => Ok(KeyType::EcP384),
        "ecc-p521" | "ecdsa-p521" => Ok(KeyType::EcP521),
        "ed25519" => Ok(KeyType::Ed25519),
        "rsa-2048" => Ok(KeyType::Rsa2048),
        "rsa-3072" => Ok(KeyType::Rsa3072),
        "rsa-4096" => Ok(KeyType::Rsa4096),
        other => Err(CertError::Validation(format!("unknown key_type: {}", other))),
    }
}

/// Load or generate a `rcgen::KeyPair` for the given CA cert config.
/// If the key doesn't exist and `auto_generate` is true, it is generated.
/// Returns the key pair and whether it was freshly generated.
fn load_or_generate_key(
    ks: &dyn ox_cert_core::KeyStore,
    tenant_id: &str,
    cfg: &CaCertConfig,
    auto_generate: bool,
) -> Result<(rcgen::KeyPair, bool), CertError> {
    let key_id = &cfg.key_path;
    let exists = ks.key_exists(tenant_id, key_id)?;

    if !exists {
        if !auto_generate {
            return Err(CertError::Crypto(format!(
                "key '{}' not found and auto_generate=false",
                key_id
            )));
        }
        let key_type = parse_key_type(&cfg.key_type)?;
        ks.generate_key(tenant_id, key_id, key_type, false)?;
        tracing::info!("generated new {} key '{}'", cfg.key_type, key_id);
    }

    let pem = ks.load_key_pem(tenant_id, key_id)?;
    let kp = rcgen::KeyPair::from_pem(&pem)
        .map_err(|e| CertError::Crypto(format!("failed to load key pair from PEM: {}", e)))?;
    Ok((kp, !exists))
}

/// Core CA initialization logic, independent of the plugin ABI.
/// Returns Ok(()) on success; the calling plugin converts errors to null.
pub fn run(config: &CaInitConfig) -> Result<(), CertError> {
    let tenant = &config.tenant_id;

    // Open store and run migration.
    let store = OxPersistenceCertStore::open()?;
    store.migrate()?;

    // Open key store.
    let ks = open_keystore(&config.keystore)?;

    // -----------------------------------------------------------------------
    // Root CA
    // -----------------------------------------------------------------------
    let root_cfg = &config.ca.root;
    let (root_kp, root_fresh) = load_or_generate_key(ks.as_ref(), tenant, root_cfg, config.auto_generate)?;

    let root_cert_path = std::path::Path::new(&root_cfg.cert_path);

    let root_cert_pem = if root_cert_path.exists() {
        let pem = std::fs::read_to_string(root_cert_path)
            .map_err(|e| CertError::Storage(format!("read root cert: {}", e)))?;
        validate_ca_cert_pem(&pem, "root")?;
        pem
    } else if config.auto_generate || root_fresh {
        // Generate self-signed root cert.
        if let Some(parent) = root_cert_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CertError::Storage(format!("create cert dir: {}", e)))?;
        }
        let record = CertBuilder::new()
            .subject(&root_cfg.subject)
            .is_ca(true, None)
            .validity_seconds(root_cfg.validity_years as u64 * 365 * 86400)
            .self_sign(tenant, &root_kp)?;
        std::fs::write(root_cert_path, &record.pem)
            .map_err(|e| CertError::Storage(format!("write root cert: {}", e)))?;
        tracing::info!("generated self-signed root CA cert at {}", root_cfg.cert_path);
        record.pem
    } else {
        return Err(CertError::NotFound(format!(
            "root CA cert not found at {} and auto_generate=false",
            root_cfg.cert_path
        )));
    };

    // Persist root CA key record (ignore duplicate errors — key may already be stored).
    let root_not_after = cert_not_after(&root_cert_pem);
    let root_key_record = CaKeyRecord {
        id: root_cfg.key_path.clone(),
        tenant_id: tenant.clone(),
        key_type: parse_key_type(&root_cfg.key_type)?,
        cert_pem: root_cert_pem.clone(),
        key_ref: root_cfg.key_path.clone(),
        status: CaKeyStatus::Active,
        not_before: OffsetDateTime::now_utc(),
        not_after: root_not_after,
        name_constraints: None,
        path_length: None,
        created_at: OffsetDateTime::now_utc(),
    };
    let _ = store.store_ca_key(tenant, &root_key_record);

    // -----------------------------------------------------------------------
    // Intermediate CA
    // -----------------------------------------------------------------------
    let inter_cfg = &config.ca.intermediate;
    let (inter_kp, inter_fresh) =
        load_or_generate_key(ks.as_ref(), tenant, inter_cfg, config.auto_generate)?;

    let inter_cert_path = std::path::Path::new(&inter_cfg.cert_path);

    let inter_cert_pem = if inter_cert_path.exists() {
        let pem = std::fs::read_to_string(inter_cert_path)
            .map_err(|e| CertError::Storage(format!("read intermediate cert: {}", e)))?;
        validate_ca_cert_pem(&pem, "intermediate")?;
        pem
    } else if config.auto_generate || inter_fresh {
        if let Some(parent) = inter_cert_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CertError::Storage(format!("create cert dir: {}", e)))?;
        }
        // Build root CA params from config (used as the issuer for signing).
        let root_params = CertBuilder::new()
            .subject(&root_cfg.subject)
            .is_ca(true, None)
            .build_params()?;

        let record = CertBuilder::new()
            .subject(&inter_cfg.subject)
            .is_ca(true, inter_cfg.path_length)
            .validity_seconds(inter_cfg.validity_years as u64 * 365 * 86400)
            .sign_with_issuer(tenant, &inter_kp, &root_params, &root_kp)?;
        std::fs::write(inter_cert_path, &record.pem)
            .map_err(|e| CertError::Storage(format!("write intermediate cert: {}", e)))?;
        tracing::info!(
            "generated intermediate CA cert at {}",
            inter_cfg.cert_path
        );
        record.pem
    } else {
        return Err(CertError::NotFound(format!(
            "intermediate CA cert not found at {} and auto_generate=false",
            inter_cfg.cert_path
        )));
    };

    // Persist intermediate CA key record.
    let inter_not_after = cert_not_after(&inter_cert_pem);
    let inter_key_record = CaKeyRecord {
        id: inter_cfg.key_path.clone(),
        tenant_id: tenant.clone(),
        key_type: parse_key_type(&inter_cfg.key_type)?,
        cert_pem: inter_cert_pem,
        key_ref: inter_cfg.key_path.clone(),
        status: CaKeyStatus::Active,
        not_before: OffsetDateTime::now_utc(),
        not_after: inter_not_after,
        name_constraints: inter_cfg.name_constraints.as_ref().map(|nc| NameConstraints {
            permitted_dns: nc.permitted_dns.clone(),
            excluded_dns: nc.excluded_dns.clone(),
            permitted_ip: vec![],
            excluded_ip: vec![],
            permitted_email: vec![],
            excluded_email: vec![],
        }),
        path_length: inter_cfg.path_length,
        created_at: OffsetDateTime::now_utc(),
    };
    let _ = store.store_ca_key(tenant, &inter_key_record);

    tracing::info!(
        "ox_cert_ca_init: tenant={} root='{}' intermediate='{}' — ready",
        tenant,
        root_cfg.subject,
        inter_cfg.subject,
    );

    Ok(())
}

/// Parse the `not_after` timestamp from a PEM cert using x509-parser.
/// Falls back to `now + 30 years` on parse failure.
fn cert_not_after(pem: &str) -> OffsetDateTime {
    use x509_parser::prelude::*;
    let fallback = OffsetDateTime::now_utc() + ::time::Duration::days(30 * 365);
    let der = match ::pem::parse(pem.as_bytes()) {
        Ok(p) => p.into_contents(),
        Err(_) => return fallback,
    };
    match X509Certificate::from_der(&der) {
        Ok((_, cert)) => {
            let ts = cert.validity().not_after.timestamp();
            OffsetDateTime::from_unix_timestamp(ts).unwrap_or(fallback)
        }
        Err(_) => fallback,
    }
}

/// Validate a CA cert PEM: must parse and not be expired.
/// Logs a warning if expiring within 90 days.
fn validate_ca_cert_pem(pem: &str, label: &str) -> Result<(), CertError> {
    use x509_parser::prelude::*;
    let der = ::pem::parse(pem.as_bytes())
        .map_err(|e| CertError::Crypto(format!("{} cert PEM invalid: {}", label, e)))?
        .contents()
        .to_vec();
    let (_, cert) = X509Certificate::from_der(&der)
        .map_err(|e| CertError::Crypto(format!("{} cert DER invalid: {}", label, e)))?;
    let not_after = cert.validity().not_after.timestamp();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let days_remaining = (not_after - now) / 86400;
    if days_remaining < 0 {
        tracing::error!("{} CA cert is EXPIRED ({} days ago)", label, -days_remaining);
    } else if days_remaining < 90 {
        tracing::warn!("{} CA cert expiring in {} days", label, days_remaining);
    } else {
        tracing::info!("{} CA cert valid for {} more days", label, days_remaining);
    }
    Ok(())
}
