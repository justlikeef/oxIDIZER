use std::io::BufReader;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::net::TcpSocket;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::{certs, private_key};
use rustls::server::{ClientHello, ResolvesServerCert, ResolvesServerCertUsingSni};
use rustls::sign::CertifiedKey;
use rustls::server::ServerConfig as RustlsServerConfig;
use log::{info, error, LevelFilter};
use clap::{Parser, Subcommand};
use tower::Service;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::service::TowerToHyperService;
use axum::{
    body::Body,
    http::Request,
    extract::ConnectInfo,
    Router,
};
use log4rs::config::{Appender, Config as LogConfig, Root};
use log4rs::append::console::ConsoleAppender;
use log4rs::encode::pattern::PatternEncoder;

use ox_webservice::{ServerConfig, load_config_from_path, flow::Flow};

#[derive(Debug)]
struct CustomCertResolver {
    sni_resolver: ResolvesServerCertUsingSni,
    default_cert: Option<Arc<CertifiedKey>>,
}

impl ResolvesServerCert for CustomCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        if let Some(cert) = self.sni_resolver.resolve(client_hello) {
            return Some(cert);
        }
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
    Configcheck,
    Run,
    DaemonRun,
}

#[tokio::main]
async fn main() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cli = Cli::parse();

    let stderr = ConsoleAppender::builder()
        .target(log4rs::append::console::Target::Stderr)
        .encoder(Box::new(PatternEncoder::new("{d} {l} - {m}{n}")))
        .build();
    let config = LogConfig::builder()
        .appender(Appender::builder().build("stderr", Box::new(stderr)))
        .build(Root::builder().appender("stderr").build(LevelFilter::Info))
        .unwrap();
    let log_handle = log4rs::init_config(config).unwrap();

    let server_config_path = Path::new(&cli.config);
    
    let (server_config, config_json) = match load_config_from_path(server_config_path, "info") {
        Ok(result) => result,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    match log4rs::config::load_config_file(&server_config.log4rs_config, Default::default()) {
        Ok(config) => {
            log_handle.set_config(config);
            info!("log4rs initialized successfully from file.");
            log::debug!("Fully processed config for {:?}:\n{}", server_config_path, config_json);
        },
        Err(e) => {
            eprintln!("Failed to load log4rs config from {}: {}. Continuing with default logger.", server_config.log4rs_config, e);
             std::process::exit(1);
        }
    }

    match cli.command {
        Commands::Configcheck => {
            info!("Running config check...");
            match Flow::new(&server_config, config_json.clone()) {
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
             info!("Starting ox_webservice...");
             start_server(server_config, server_config_path.to_path_buf(), config_json).await;
        }
    }
}

async fn start_server(initial_config: ServerConfig, config_path: PathBuf, config_json: String) {
    if initial_config.servers.is_empty() {
        error!("Flow configuration missing: no servers defined");
        std::process::exit(1);
    }

    let flow: Arc<Flow> = match Flow::new(&initial_config, config_json) {
        Ok(f) => Arc::new(f),
        Err(e) => {
            error!("Failed to initialize flow: {}", e);
            std::process::exit(1);
        }
    };

    let flow_holder = Arc::new(RwLock::new(flow));
    let flow_holder_clone = flow_holder.clone();
    let config_path_clone = config_path.clone();

    tokio::spawn(async move {
        let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).unwrap();
        loop {
            sighup.recv().await;
            info!("Received SIGHUP, reloading configuration...");

            match load_config_from_path(&config_path_clone, "info") {
                Ok((new_config, new_json)) => {
                    match Flow::new(&new_config, new_json) {
                        Ok(new_flow) => {
                            let mut write_guard = flow_holder_clone.write().await;
                            *write_guard = Arc::new(new_flow);
                            info!("Flow reloaded successfully.");
                        }
                        Err(e) => {
                            error!("Failed to build new flow: {}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to reload config: {}", e);
                }
            }
        }
    });

    let mut task_handles = Vec::new();

    for server_details in initial_config.servers {
        let flow_holder_server = flow_holder.clone();
        let protocol = server_details.protocol.clone();
        let bind_address = server_details.bind_address.clone();
        let port = server_details.port;
        let backlog = server_details.backlog;
        let servers = server_details.hosts.clone();

        let app = Router::new()

            .route("/", axum::routing::any({
                let flow_holder_server = flow_holder_server.clone();
                let protocol_clone = protocol.clone();
                move |ws: Option<axum::extract::ws::WebSocketUpgrade>, req: Request<Body>| async move {
                    let flow_arc = flow_holder_server.read().await.clone();
                    let connect_info = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|ci| ci.0).unwrap_or(SocketAddr::from(([0, 0, 0, 0], 0)));
                    let protocol_clone = protocol_clone.clone();
                    
                    if let Some(ws_upgrade) = ws {
                        let ws_protocol = if protocol_clone == "https" { "WSS".to_string() } else { "WS".to_string() };
                        return ws_upgrade.on_upgrade(move |socket| async move {
                            flow_arc.handle_socket(socket, connect_info, "".to_string(), ws_protocol).await;
                        });
                    }
                    
                    flow_arc.execute_request(connect_info, req, protocol_clone).await
                }
            }))
            .route("/*path", axum::routing::any({
                let flow_holder_server = flow_holder_server.clone();
                let protocol_clone = protocol.clone();
                move |ws: Option<axum::extract::ws::WebSocketUpgrade>, axum::extract::Path(path): axum::extract::Path<String>, req: Request<Body>| async move {
                    let flow_arc = flow_holder_server.read().await.clone();
                    let connect_info = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|ci| ci.0).unwrap_or(SocketAddr::from(([0, 0, 0, 0], 0)));
                    let protocol_clone = protocol_clone.clone();
                    
                    if let Some(ws_upgrade) = ws {
                        let ws_protocol = if protocol_clone == "https" { "WSS".to_string() } else { "WS".to_string() };
                        return ws_upgrade.on_upgrade(move |socket| async move {
                            flow_arc.handle_socket(socket, connect_info, path, ws_protocol).await;
                        });
                    }
                    
                    flow_arc.execute_request(connect_info, req, protocol_clone).await
                }
            }))
            .layer(tower_http::catch_panic::CatchPanicLayer::new());

        // IPv6 addresses (e.g. "::1" or "::") need bracket notation: "[::1]:port"
        let addr_str = if bind_address.contains(':') {
            format!("[{}]:{}", bind_address, port)
        } else {
            format!("{}:{}", bind_address, port)
        };
        let addr: SocketAddr = addr_str.parse().expect("Invalid bind address");
        info!("Listening on {}", addr);

        if server_details.protocol == "https" {
             let mut cert_resolver = ResolvesServerCertUsingSni::new();
             let mut default_cert = None;

             for (i, host) in servers.iter().enumerate() {
                 if let (Some(cert_path), Some(key_path)) = (&host.tls_cert_path, &host.tls_key_path) {
                      let cert_content = std::fs::read(cert_path).unwrap();
                      let key_content = std::fs::read(key_path).unwrap();
                      
                      let certs_parsed_res: Result<Vec<CertificateDer<'static>>, _> = certs(&mut BufReader::new(&cert_content[..])).collect();
                      let certs_parsed = certs_parsed_res.unwrap();
                      let key_der = private_key(&mut BufReader::new(&key_content[..]))
                          .expect("failed to parse TLS private key")
                          .expect("no private key found in TLS key file");
                      let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key_der).unwrap();
                      let certified_key = CertifiedKey::new(certs_parsed, signing_key);

                      if i == 0 { default_cert = Some(Arc::new(certified_key.clone())); }
                      let _ = cert_resolver.add(&host.name, certified_key);
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

             let rustls_config = Arc::new(tls_config);
             let tls_acceptor = tokio_rustls::TlsAcceptor::from(rustls_config);

             task_handles.push(tokio::spawn(async move {
                 let tcp_socket = match if addr.is_ipv6() { TcpSocket::new_v6() } else { TcpSocket::new_v4() } {
                     Ok(s) => s, Err(e) => { error!("Failed to create TCP socket for {}: {}", addr, e); return; }
                 };
                 if let Err(e) = tcp_socket.set_reuseaddr(true) { error!("set_reuseaddr failed for {}: {}", addr, e); return; }
                 if let Err(e) = tcp_socket.bind(addr) { error!("Failed to bind {}: {}", addr, e); return; }
                 let listener = match tcp_socket.listen(backlog) {
                     Ok(l) => l, Err(e) => { error!("Failed to listen on {}: {}", addr, e); return; }
                 };
                 info!("HTTPS listener ready on {}", addr);

                 loop {
                     let (socket, remote_addr) = match listener.accept().await {
                         Ok(res) => res,
                         Err(e) => {
                             error!("Accept error on {}: {}", addr, e);
                             tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                             continue;
                         }
                     };

                     // IP Filtering Placeholder:
                     // let ip = remote_addr.ip();
                     // if !allowed_ip(ip) { continue; }

                     let tls_acceptor_clone = tls_acceptor.clone();
                     let app_clone = app.clone();
                     let _ = socket.set_nodelay(true);

                     tokio::spawn(async move {
                         let tls_stream = match tls_acceptor_clone.accept(socket).await {
                             Ok(s) => s,
                             Err(e) => {
                                 error!("TLS error: {}", e);
                                 return;
                             }
                         };

                         let mut svc = app_clone.into_make_service_with_connect_info::<SocketAddr>();
                         let ready_svc = match svc.call(remote_addr).await { Ok(s) => s, Err(_) => return };
                         let hyper_svc = TowerToHyperService::new(ready_svc);
                         let io = TokioIo::new(tls_stream);
                         let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                             .serve_connection_with_upgrades(io, hyper_svc)
                             .await;
                     });
                 }
             }));
        } else {
            // HTTP listener — if hosts have TLS certs, also accept TLS ClientHellos on this port.
            // This lets browsers that try https:// on the same port (Chrome HTTPS-First mode)
            // get a valid TLS handshake followed by the redirect to the real HTTPS port.
            // The protocol reported to the workflow stays "http" so http-only routes fire.
            let tls_upgrade_acceptor: Option<tokio_rustls::TlsAcceptor> = {
                let has_certs = servers.iter().any(|h| h.tls_cert_path.is_some() && h.tls_key_path.is_some());
                if has_certs {
                    let mut cert_resolver = ResolvesServerCertUsingSni::new();
                    let mut default_cert = None;
                    for (i, host) in servers.iter().enumerate() {
                        if let (Some(cert_path), Some(key_path)) = (&host.tls_cert_path, &host.tls_key_path) {
                            let cert_content = std::fs::read(cert_path).unwrap();
                            let key_content = std::fs::read(key_path).unwrap();
                            let certs_parsed: Vec<CertificateDer<'static>> = certs(&mut BufReader::new(&cert_content[..])).collect::<Result<_, _>>().unwrap();
                            let key_der = private_key(&mut BufReader::new(&key_content[..])).expect("parse TLS private key").expect("no private key found");
                            let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key_der).unwrap();
                            let certified_key = CertifiedKey::new(certs_parsed, signing_key);
                            if i == 0 { default_cert = Some(Arc::new(certified_key.clone())); }
                            let _ = cert_resolver.add(&host.name, certified_key);
                        }
                    }
                    let resolver = Arc::new(CustomCertResolver { sni_resolver: cert_resolver, default_cert });
                    let mut tls_config = RustlsServerConfig::builder()
                        .with_no_client_auth()
                        .with_cert_resolver(resolver);
                    tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
                    Some(tokio_rustls::TlsAcceptor::from(Arc::new(tls_config)))
                } else {
                    None
                }
            };

            task_handles.push(tokio::spawn(async move {
                 let tcp_socket = match if addr.is_ipv6() { TcpSocket::new_v6() } else { TcpSocket::new_v4() } {
                     Ok(s) => s, Err(e) => { error!("Failed to create TCP socket for {}: {}", addr, e); return; }
                 };
                 if let Err(e) = tcp_socket.set_reuseaddr(true) { error!("set_reuseaddr failed for {}: {}", addr, e); return; }
                 if let Err(e) = tcp_socket.bind(addr) { error!("Failed to bind {}: {}", addr, e); return; }
                 let listener = match tcp_socket.listen(backlog) {
                     Ok(l) => l, Err(e) => { error!("Failed to listen on {}: {}", addr, e); return; }
                 };
                 info!("HTTP listener ready on {}", addr);

                 loop {
                     let (socket, remote_addr) = match listener.accept().await {
                         Ok(res) => res,
                         Err(e) => {
                             error!("Accept error on {}: {}", addr, e);
                             tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                             continue;
                         }
                     };

                     // IP Filtering Placeholder:
                     // let ip = remote_addr.ip();
                     // if !allowed_ip(ip) { continue; }

                     let app_clone = app.clone();
                     let _ = socket.set_nodelay(true);
                     let tls_acc = tls_upgrade_acceptor.clone();

                     tokio::spawn(async move {
                         if let Some(tls_acceptor) = tls_acc {
                             let mut peek = [0u8; 1];
                             if socket.peek(&mut peek).await.is_ok() && peek[0] == 0x16 {
                                 // TLS ClientHello — complete handshake; protocol stays "http" for routing
                                 match tls_acceptor.accept(socket).await {
                                     Ok(tls_stream) => {
                                         let mut svc = app_clone.into_make_service_with_connect_info::<SocketAddr>();
                                         let ready_svc = match svc.call(remote_addr).await { Ok(s) => s, Err(_) => return };
                                         let hyper_svc = TowerToHyperService::new(ready_svc);
                                         let io = TokioIo::new(tls_stream);
                                         let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                                             .serve_connection_with_upgrades(io, hyper_svc)
                                             .await;
                                     }
                                     Err(e) => { error!("TLS upgrade error on {}: {}", addr, e); }
                                 }
                                 return;
                             }
                         }
                         let mut svc = app_clone.into_make_service_with_connect_info::<SocketAddr>();
                         let ready_svc = match svc.call(remote_addr).await { Ok(s) => s, Err(_) => return };
                         let hyper_svc = TowerToHyperService::new(ready_svc);
                         let io = TokioIo::new(socket);
                         let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                             .serve_connection_with_upgrades(io, hyper_svc)
                             .await;
                     });
                 }
            }));
        }
    }
    
    for handle in task_handles {
        let _ = handle.await;
    }
}
