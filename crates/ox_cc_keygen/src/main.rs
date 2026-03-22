/// ox_cc_keygen — generate Ed25519 signing keys and X25519 encryption keys
/// for use in the ox_cc system.
///
/// Outputs:
///   Broker signing keypair: ed25519 private key (32 raw bytes) + public key
///   Client encryption keypair: x25519 static secret (32 raw bytes) + public key
///
/// Key files are written as raw 32-byte files (not PEM/DER).
/// Public keys are also printed as base64url for use in YAML config.
use std::path::PathBuf;

use anyhow::Result;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rand::RngCore;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

#[derive(Parser, Debug)]
#[command(
    name = "ox_cc_keygen",
    about = "Generate Ed25519 signing and X25519 encryption keys for ox_cc"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Generate an Ed25519 signing keypair (for the broker signing key).
    Broker {
        /// Directory to write key files into.
        #[arg(short, long, default_value = ".")]
        out_dir: PathBuf,

        /// Base filename (default: broker_signing).
        /// Outputs: <name>.key (private, 32 bytes), <name>.pub (public, 32 bytes).
        #[arg(short, long, default_value = "broker_signing")]
        name: String,
    },

    /// Generate an X25519 encryption keypair (for a client enrollment).
    Client {
        /// Directory to write key files into.
        #[arg(short, long, default_value = ".")]
        out_dir: PathBuf,

        /// Base filename (default: client_enc).
        /// Outputs: <name>.key (private, 32 bytes), <name>.pub (public, 32 bytes).
        #[arg(short, long, default_value = "client_enc")]
        name: String,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Broker { out_dir, name } => gen_broker(&out_dir, &name),
        Command::Client { out_dir, name } => gen_client(&out_dir, &name),
    }
}

fn gen_broker(out_dir: &PathBuf, name: &str) -> Result<()> {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();

    let privkey_path = out_dir.join(format!("{}.key", name));
    let pubkey_path = out_dir.join(format!("{}.pub", name));

    std::fs::write(&privkey_path, signing_key.to_bytes())?;
    std::fs::write(&pubkey_path, verifying_key.to_bytes())?;

    set_mode_600(&privkey_path)?;

    let pubkey_b64 = URL_SAFE_NO_PAD.encode(verifying_key.to_bytes());
    println!("Broker Ed25519 signing keypair generated:");
    println!("  Private key : {} (mode 600)", privkey_path.display());
    println!("  Public key  : {}", pubkey_path.display());
    println!("  Public key (base64url): {}", pubkey_b64);
    println!();
    println!("Place the .pub file in the broker_signing_pubkeys_dir on each client.");
    println!("Keep the .key file on the broker host, referenced by signing_key_path in broker_plugin.yaml.");

    Ok(())
}

fn gen_client(out_dir: &PathBuf, name: &str) -> Result<()> {
    let secret = StaticSecret::random_from_rng(OsRng);
    let pubkey = X25519PublicKey::from(&secret);

    let privkey_path = out_dir.join(format!("{}.key", name));
    let pubkey_path = out_dir.join(format!("{}.pub", name));

    std::fs::write(&privkey_path, secret.to_bytes())?;
    std::fs::write(&pubkey_path, pubkey.to_bytes())?;

    set_mode_600(&privkey_path)?;

    let privkey_b64 = URL_SAFE_NO_PAD.encode(secret.to_bytes());
    let pubkey_b64 = URL_SAFE_NO_PAD.encode(pubkey.to_bytes());

    println!("Client X25519 encryption keypair generated:");
    println!("  Private key : {} (mode 600)", privkey_path.display());
    println!("  Public key  : {}", pubkey_path.display());
    println!("  Private key (base64url): {}", privkey_b64);
    println!("  Public key  (base64url): {}", pubkey_b64);
    println!();
    println!("Add to client.yaml:  client_enc_privkey_b64: \"{}\"", privkey_b64);
    println!("Enroll at broker:    POST /broker/clients  {{ \"client_id\": \"...\", \"enc_pubkey_b64\": \"{}\", ... }}", pubkey_b64);

    Ok(())
}

#[cfg(unix)]
fn set_mode_600(path: &PathBuf) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode_600(_path: &PathBuf) -> Result<()> {
    Ok(())
}
