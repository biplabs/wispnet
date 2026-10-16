//! Disposable, loopback-only TLS relay fixture for native/device smoke checks.
//! This is a test harness, not the deployed relay from #18.
use irelay::server::{CertConfig, RelayConfig, Server, ServerConfig, TlsConfig};
use std::{io::Write, net::SocketAddr, os::unix::fs::OpenOptionsExt, path::PathBuf};

async fn start(
    tls: &rustls::ServerConfig,
    address: SocketAddr,
) -> Result<Server, Box<dyn std::error::Error>> {
    let mut relay = RelayConfig::new("127.0.0.1:0".parse::<SocketAddr>()?);
    relay.tls = Some(TlsConfig::new(
        address,
        CertConfig::Manual {
            server_config: tls.clone(),
        },
    ));
    let mut config = ServerConfig::default();
    config.relay = Some(relay);
    Ok(Server::spawn(config).await?)
}

fn report(event: &str, address: SocketAddr) {
    println!(
        "{}",
        serde_json::json!({"event":event,"relay_url":format!("https://{address}")})
    );
    std::io::stdout().flush().expect("fixture output closed");
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 || args[0] != "--ca-file" {
        return Err("usage: relay_fixture --ca-file /absolute/new/public-certificate.pem".into());
    }
    let path = PathBuf::from(&args[1]);
    if !path.is_absolute() {
        return Err("fixture CA path must be absolute".into());
    }
    let certificate = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()])?;
    let tls = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.cert.der().clone()],
        rustls_pki_types::PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der()).into(),
    )?;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?
        .write_all(certificate.cert.pem().as_bytes())?;
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut restart =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1())?;
    let mut server = start(&tls, "127.0.0.1:0".parse()?).await?;
    let address = server.https_addr().ok_or("fixture HTTPS not bound")?;
    report("started", address);
    loop {
        tokio::select! {
            _ = term.recv() => break,
            _ = interrupt.recv() => break,
            _ = restart.recv() => {
                server.shutdown().await?;
                server = start(&tls, address).await?;
                report("restarted", address);
            }
        }
    }
    server.shutdown().await?;
    Ok(())
}
