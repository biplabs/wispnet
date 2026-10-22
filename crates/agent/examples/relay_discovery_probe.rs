//! Disposable UDP discovery check; no device identity, pairing or service state.
use iroh::{Endpoint, RelayConfig, RelayMode, Watcher, endpoint::presets};
use iroh_relay::RelayQuicConfig;
use std::time::{Duration, Instant};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let bind_addr = if let Some(index) = args.iter().position(|arg| arg == "--bind-addr") {
        if index + 1 >= args.len() {
            return Err("--bind-addr requires IP:PORT".into());
        }
        let value = args.remove(index + 1).parse::<std::net::SocketAddr>()?;
        args.remove(index);
        Some(value)
    } else {
        None
    };
    if args.len() > 3 {
        return Err(
            "usage: relay_discovery_probe [--bind-addr IP:PORT] [RELAY_URL [UDP_PORT [CA_FILE]]]"
                .into(),
        );
    }
    let url = args
        .first()
        .map(String::as_str)
        .unwrap_or("https://relay.wispnet.app");
    let port: u16 = args.get(1).map(String::as_str).unwrap_or("7842").parse()?;
    if port == 0 {
        return Err("UDP port must be nonzero".into());
    }
    let relay = RelayConfig::new(url.parse()?, Some(RelayQuicConfig::new(port)));
    let mut builder = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Custom([relay].into_iter().collect()));
    if let Some(path) = args.get(2) {
        use rustls_pki_types::pem::PemObject;
        let pem = std::fs::read(path)?;
        let cert = rustls_pki_types::CertificateDer::from_pem_slice(&pem)?;
        builder = builder.ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots([cert]));
    }
    if let Some(address) = bind_addr {
        builder = builder.clear_ip_transports().bind_addr(address)?;
    }
    let started = Instant::now();
    let endpoint = builder.bind().await?;
    let mut observed = None;
    while started.elapsed() < Duration::from_secs(20) {
        observed = endpoint.net_report().get();
        if observed.as_ref().is_some_and(|r| r.has_udp()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let ok = observed.as_ref().is_some_and(|r| r.has_udp());
    println!(
        "{}",
        serde_json::json!({
            "event":"relay_udp_discovery", "ok":ok, "bind_addr":bind_addr, "relay":url, "udp_port":port,
            "elapsed_ms":started.elapsed().as_millis(),
            "network_report":observed.map(|r| serde_json::json!({
                "udp_v4":r.udp_v4, "udp_v6":r.udp_v6,
                "public_ipv4_discovered":r.global_v4.is_some_and(|a|wispnet_agent::diagnostics::public_ip((*a.ip()).into())),
                "observed_ipv4_is_private":r.global_v4.map(|a|a.ip().is_private()),
                "observed_ipv4_is_loopback":r.global_v4.map(|a|a.ip().is_loopback()),
                "public_ipv6_discovered":r.global_v6.is_some_and(|a|wispnet_agent::diagnostics::public_ip((*a.ip()).into()))
            }))
        })
    );
    endpoint.close().await;
    if !ok {
        return Err("No authenticated UDP discovery response within 20 seconds".into());
    }
    Ok(())
}
