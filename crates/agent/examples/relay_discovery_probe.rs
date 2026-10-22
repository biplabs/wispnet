//! Disposable UDP discovery check; no device identity, pairing or service state.
use iroh::{Endpoint, RelayConfig, RelayMode, Watcher, endpoint::presets};
use iroh_relay::RelayQuicConfig;
use std::time::{Duration, Instant};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() > 3 {
        return Err("usage: relay_discovery_probe [RELAY_URL [UDP_PORT [CA_FILE]]]".into());
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
            "event":"relay_udp_discovery", "ok":ok, "relay":url, "udp_port":port,
            "elapsed_ms":started.elapsed().as_millis(),
            "network_report":observed.map(|r| serde_json::json!({
                "udp_v4":r.udp_v4, "udp_v6":r.udp_v6,
                "public_ipv4_discovered":r.global_v4.is_some(),
                "public_ipv6_discovered":r.global_v6.is_some()
            }))
        })
    );
    endpoint.close().await;
    if !ok {
        return Err("No authenticated UDP discovery response within 20 seconds".into());
    }
    Ok(())
}
