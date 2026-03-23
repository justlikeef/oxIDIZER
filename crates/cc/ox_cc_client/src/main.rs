/// ox_cc_client — secure configuration client daemon.
///
/// Runs as a non-privileged service account (group ox_cc).
/// Polls the Manifest instance for signed configuration envelopes,
/// verifies and decrypts them, and writes manifest.json atomically.
use anyhow::Result;
use clap::Parser;

mod applier;
mod config;
mod db;
mod fetcher;

use config::ClientConfig;
use fetcher::Notifier;

#[derive(Parser, Debug)]
#[command(name = "ox_cc_client", about = "ox_cc secure configuration client")]
struct Args {
    /// Path to the client configuration file.
    #[arg(short, long, default_value = "/etc/ox_cc/client.yaml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let cfg = ClientConfig::load(&args.config)?;

    tracing::info!(
        client_id = %cfg.client_id,
        manifest_url = %cfg.manifest_url,
        "ox_cc_client starting"
    );

    let db = db::ClientDb::open(&cfg.db_path, &cfg.db_encryption_key)?;
    let fetcher = fetcher::Fetcher::new(&cfg)?;

    loop {
        if let Err(e) = poll_cycle(&cfg, &db, &fetcher).await {
            tracing::warn!(error = %e, "poll cycle error; retrying after interval");
        }
        tokio::time::sleep(std::time::Duration::from_secs(cfg.poll_interval_secs)).await;
    }
}

async fn poll_cycle(
    cfg: &ClientConfig,
    db: &db::ClientDb,
    fetcher: &fetcher::Fetcher,
) -> Result<()> {
    // Retry any pending applied notifications before polling for new manifests
    db.retry_pending_notifications(fetcher, cfg).await?;

    // Fetch the latest envelope from the manifest instance
    let wire = match fetcher.fetch_latest(cfg).await? {
        Some(w) => w,
        None => {
            tracing::debug!("no new manifest (304 or empty)");
            return Ok(());
        }
    };

    // Verify signature (multi-key), then decrypt
    let broker_keys = cfg.load_broker_verifying_keys()?;
    let manifest = ox_cc_common::verify::verify_and_decrypt(
        &wire,
        &cfg.client_id,
        &broker_keys,
        &cfg.client_enc_privkey()?,
        cfg.max_manifest_window_secs,
    )?;

    // Skip if already applied
    if db.is_applied(&manifest.manifest_id)? {
        tracing::debug!(manifest_id = %manifest.manifest_id, "already applied, skipping");
        return Ok(());
    }

    // Dispatch to the registered consumer handler
    let consumer_dir = cfg
        .consumer_dirs
        .get(&manifest.consumer)
        .ok_or_else(|| anyhow::anyhow!("no consumer dir for '{}'", manifest.consumer))?;

    applier::apply(consumer_dir, cfg, &manifest).await?;
    db.record_applied(&manifest)?;

    tracing::info!(
        manifest_id = %manifest.manifest_id,
        consumer = %manifest.consumer,
        "manifest applied"
    );

    // Execute commandset if the payload contains one
    let exec_detail: Option<String> = if let Some(cs) = manifest.payload.get("commandset") {
        match serde_json::from_value::<Vec<ox_cc_common::CommandEntry>>(cs.clone()) {
            Ok(commands) => {
                tracing::info!(
                    manifest_id = %manifest.manifest_id,
                    command_count = %commands.len(),
                    "executing commandset"
                );
                let result = ox_cc_executor::run(&commands, cfg.plugin_dir.as_deref()).await;
                tracing::info!(
                    manifest_id = %manifest.manifest_id,
                    status = ?result.status,
                    "commandset complete"
                );
                Some(result.to_detail_json())
            }
            Err(e) => {
                tracing::warn!(manifest_id = %manifest.manifest_id, error = %e, "commandset parse failed; skipping");
                None
            }
        }
    } else {
        None
    };

    // Send applied notification immediately with executor detail
    match fetcher.post_applied(cfg, &manifest.manifest_id, exec_detail.as_deref()).await {
        Ok(_) => {
            db.mark_notified(&manifest.manifest_id)?;
            tracing::info!(manifest_id = %manifest.manifest_id, "applied notification sent");
        }
        Err(e) => {
            tracing::warn!(manifest_id = %manifest.manifest_id, error = %e, "applied notification failed; will retry");
        }
    }

    Ok(())
}
