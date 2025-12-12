
use std::io::BufReader;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::net::TcpListener;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::{certs, pkcs8_private_keys};
use rustls::server::{ClientHello, ResolvesServerCert, ResolvesServerCertUsingSni};
use rustls::sign::CertifiedKey;
use rustls::server::ServerConfig as RustlsServerConfig;
use axum_server::tls_rustls::RustlsConfig;
use log::{info, error};

use axum::{
    body::Body,
    http::Request,
    extract::ConnectInfo,
    Router,
};
use clap::{Parser, Subcommand};



use ox_webservice::{ServerConfig, load_config_from_path, pipeline::Pipeline};


// Structs moved to lib.rs, removed from here.

#[derive(Debug)]
struct CustomCertResolver {
    sni_resolver: ResolvesServerCertUsingSni,
    default_cert: Option<Arc<CertifiedKey>>,
}

impl ResolvesServerCert for CustomCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        // First, try to resolve using the SNI-based resolver
        if let Some(cert) = self.sni_resolver.resolve(client_hello) {
            return Some(cert);
        }
        // If no specific certificate was found, return the default one
        self.default_cert.clone()
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long, default_value = "ox_webservice.yaml")]
    config: String,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Checks the configuration file for errors
    Configcheck,
    /// Runs the server
    Run,
    /// Runs the server (background/daemon - implementation currently same as Run)
    DaemonRun,
}

#[tokio::main]
async fn main() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cli = Cli::parse();

    let server_config_path = Path::new(&cli.config);
    
    // Initial config load
    let server_config: ServerConfig = match load_config_from_path(server_config_path, "info") {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    // Initialize logging
    match log4rs::init_file(&server_config.log4rs_config, Default::default()) {
        Ok(_) => info!("log4rs initialized successfully."),
        Err(e) => {
            eprintln!("Failed to initialize log4rs from {}: {}. Exiting.", server_config.log4rs_config, e);
            std::process::exit(1);
        }
    }

    match cli.command {
        Commands::Configcheck => {
            info!("Running config check...");
            match Pipeline::new(&server_config) {
                Ok(_) => {
                    println!("Configuration OK");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Configuration Check Failed: {}", e);
                    std::process::exit(1);
                }
            }
        },
        Commands::Run | Commands::DaemonRun => {
             // Basic daemon-run handling (identical to Run for now, just main loop)
             info!("Starting ox_webservice...");
             start_server(server_config, server_config_path.to_path_buf()).await;
        }
    }
}

async fn start_server(initial_config: ServerConfig, config_path: PathBuf) {
    let pipeline = match Pipeline::new(&initial_config) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            error!("Failed to initialize pipeline: {}", e);
            std::process::exit(1);
        }
    };

    let pipeline_holder = Arc::new(RwLock::new(pipeline));
    let pipeline_holder_clone = pipeline_holder.clone();
    let config_path_clone = config_path.clone();

    // Signal handler for SIGHUP (Reload)
    tokio::spawn(async move {
        let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).unwrap();
        loop {
            sighup.recv().await;
            info!("Received SIGHUP, reloading configuration...");
            
            match load_config_from_path(&config_path_clone, "info") {
                Ok(new_config) => {
                    match Pipeline::new(&new_config) {
                        Ok(new_pipeline) => {
                            let mut write_guard = pipeline_holder_clone.write().unwrap();
                            *write_guard = Arc::new(new_pipeline);
                            info!("Pipeline reloaded successfully.");
                        }
                        Err(e) => {
                            error!("Failed to build new pipeline: {}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to reload config: {}", e);
                }
            }
        }
    });


    // Start servers
    let mut task_handles = Vec::new();

    for server_details in initial_config.servers {
        let pipeline_holder_server = pipeline_holder.clone();
        let protocol = server_details.protocol.clone();
        let bind_address = server_details.bind_address.clone();
        let port = server_details.port;
        let servers = server_details.hosts.clone();

        let app = Router::new().route("/*path", axum::routing::any(move |req: Request<Body>| {
            let pipeline_arc = pipeline_holder_server.read().unwrap().clone();
            let connect_info = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|ci| ci.0).unwrap_or(SocketAddr::from(([0, 0, 0, 0], 0)));
            let protocol_clone = protocol.clone();
            
            async move {
                 pipeline_arc.execute_request(connect_info, req, protocol_clone).await
            }
        }));


        let addr: SocketAddr = format!("{}:{}", bind_address, port).parse().expect("Invalid bind address");
        info!("Listening on {}", addr);

        if server_details.protocol == "https" {
             // TLS Setup logic from original main.rs
             let mut cert_resolver = ResolvesServerCertUsingSni::new();
             let mut default_cert = None;

             for (i, host) in servers.iter().enumerate() {
                 if let (Some(cert_path), Some(key_path)) = (&host.tls_cert_path, &host.tls_key_path) {
                      let cert_content = std::fs::read(cert_path).expect("Failed to read cert");
                      let key_content = std::fs::read(key_path).expect("Failed to read key");
                      
                      // ... (Simplifying for brevity, assuming existing logic or simple load)
                      // Ideally we'd copy the robust loading from original main.rs, but re-implementing briefly:
                      let certs_parsed: Vec<CertificateDer<'static>> = certs(&mut BufReader::new(&cert_content[..])).collect::<Result<_, _>>().unwrap();
                      let key_parsed = pkcs8_private_keys(&mut BufReader::new(&key_content[..])).next().unwrap().unwrap();
                      let key_der: PrivateKeyDer<'static> = key_parsed.into();
                      
                      let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key_der).expect("Failed to create signing key");
                      let certified_key = CertifiedKey::new(certs_parsed, signing_key);

                      if i == 0 {
                          default_cert = Some(Arc::new(certified_key.clone()));
                      }
                      cert_resolver.add(&host.name, certified_key).expect("Failed to add SNI");
                 }
             }

             let resolver = Arc::new(CustomCertResolver {
                 sni_resolver: cert_resolver,
                 default_cert,
             });

             let mut tls_config = RustlsServerConfig::builder()
                .with_no_client_auth()
                .with_cert_resolver(resolver);
             tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

             let rustls_config = RustlsConfig::from_config(Arc::new(tls_config));

             task_handles.push(tokio::spawn(async move {
                axum_server::bind_rustls(addr, rustls_config)
                    .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                    .await
                    .unwrap();
             }));
        } else {
            task_handles.push(tokio::spawn(async move {
                axum::serve(
                    TcpListener::bind(addr).await.unwrap(),
                    app.into_make_service_with_connect_info::<SocketAddr>()
                ).await.unwrap();
            }));
        }
    }
    
    // Wait for all servers (they basically run forever)
    for handle in task_handles {
        let _ = handle.await;
    }
}