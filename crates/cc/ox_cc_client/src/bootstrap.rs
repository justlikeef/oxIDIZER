use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::json;
use std::path::Path;
use trust_dns_resolver::TokioAsyncResolver;
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};

use ox_cc_common::bootstrap::{BootstrapCheckinRequest, BootstrapCheckinResponse};
use ox_cc_common::keys::{generate_encryption_key, generate_signing_key};
use crate::config::ClientConfig;

/// Performs the initial bootstrap and trust exchange.
/// 
/// 1. Discovers the server's public key via DNSSEC TXT lookup.
    /// 2. Generates local X25519 and Ed25519 keypairs.
    /// 3. Performs check-in to the bootstrap server.
    /// 4. Updates client.yaml with the received trust information.
pub async fn run(config_path: &str, cfg: &ClientConfig) -> Result<()> {
    let bootstrap_url = cfg.bootstrap_url.as_ref()
        .context("bootstrap_url is required for initial checkin")?;

    tracing::info!(bootstrap_url = %bootstrap_url, "starting bootstrap process");

    // 1. DNSSEC Discovery
    // We lookup _oxcc_pubkey.<domain> for a TXT record.
    let domain = url::Url::parse(bootstrap_url)?
        .host_str()
        .context("invalid bootstrap_url host")?
        .to_string();
    
    let txt_name = format!("_oxcc_pubkey.{}", domain);
    tracing::info!(record = %txt_name, "performing DNSSEC TXT lookup");

    // Initialize resolver with DNSSEC enabled
    let mut opts = ResolverOpts::default();
    opts.validate = true; // Enable DNSSEC validation
    
    let resolver = TokioAsyncResolver::tokio(
        ResolverConfig::default(),
        opts
    );

    let lookup = resolver.txt_lookup(txt_name).await
        .context("DNSSEC TXT lookup failed")?;
    
    // For this implementation, we expect a TXT record like "oxcc-pubkey:<base64>"
    let mut server_pubkey_b64 = None;
    for txt in lookup.iter() {
        for data in txt.iter() {
            let s = String::from_utf8_lossy(data);
            if let Some(stripped) = s.strip_prefix("oxcc-pubkey:") {
                server_pubkey_b64 = Some(stripped.trim().to_string());
                break;
            }
        }
    }

    let server_pubkey_b64 = server_pubkey_b64
        .context("no 'oxcc-pubkey:' TXT record found or DNSSEC validation failed")?;
    
    tracing::info!("discovered server public key via DNSSEC");

    // 2. Key Generation
    tracing::info!("generating local keys");
    let enc_privkey = generate_encryption_key();
    let sig_key = generate_signing_key();

    let enc_pubkey_b64 = URL_SAFE_NO_PAD.encode(x25519_dalek::PublicKey::from(&enc_privkey).as_bytes());
    let sig_pubkey_b64 = URL_SAFE_NO_PAD.encode(sig_key.verifying_key().as_bytes());
    let enc_privkey_b64 = URL_SAFE_NO_PAD.encode(enc_privkey.to_bytes());

    // 3. Check-in
    tracing::info!(client_id = %cfg.client_id, "checking in to bootstrap server");
    
    let request = BootstrapCheckinRequest {
        client_id: cfg.client_id.clone(),
        enc_pubkey_b64,
        sig_pubkey_b64,
        metadata: json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "hostname": gethostname::gethostname().to_string_lossy(),
        }),
    };

    let client = reqwest::Client::new();
    let resp: BootstrapCheckinResponse = client
        .post(bootstrap_url)
        .json(&request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    tracing::info!("bootstrap check-in successful; received broker keys");

    // 4. Persist to client.yaml
    // We need to update the local config file.
    let mut new_yaml: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(config_path)?
    )?;

    // Update fields
    new_yaml["manifest_url"] = serde_yaml::Value::String(resp.manifest_url);
    new_yaml["report_url"] = serde_yaml::Value::String(resp.report_url);
    new_yaml["client_enc_privkey_b64"] = serde_yaml::Value::String(enc_privkey_b64);
    
    // Save broker keys to a directory
    let pubkey_dir = cfg.broker_signing_pubkeys_dir.as_ref()
        .map(String::as_str)
        .unwrap_or("conf/broker_keys");
    
    std::fs::create_dir_all(pubkey_dir)?;
    for (i, key_b64) in resp.broker_pubkeys.iter().enumerate() {
        let bytes = URL_SAFE_NO_PAD.decode(key_b64)?;
        let path = Path::new(pubkey_dir).join(format!("broker_{}.pub", i));
        std::fs::write(path, bytes)?;
    }
    new_yaml["broker_signing_pubkeys_dir"] = serde_yaml::Value::String(pubkey_dir.to_string());

    // Write back client.yaml
    std::fs::write(config_path, serde_yaml::to_string(&new_yaml)?)?;
    
    tracing::info!(config_path = %config_path, "trust established and configuration updated");

    Ok(())
}
