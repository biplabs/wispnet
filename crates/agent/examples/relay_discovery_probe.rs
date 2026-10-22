//! Disposable UDP discovery check; no device identity, pairing or service state.
use iroh::dns::{BoxIter, DnsError, DnsResolver, Resolver, TxtRecordData};
use iroh::{Endpoint, RelayConfig, RelayMode, Watcher, endpoint::presets};
use iroh_relay::RelayQuicConfig;
use std::time::{Duration, Instant};
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    pin::Pin,
};

type Lookup<T> = Pin<Box<dyn Future<Output = Result<BoxIter<T>, DnsError>> + Send>>;

/// Probe one selected address while retaining the relay hostname for TLS checks.
#[derive(Debug, Clone)]
struct ProbeResolver {
    host: String,
    address: IpAddr,
}
impl Resolver for ProbeResolver {
    fn lookup_ipv4(&self, host: String) -> Lookup<Ipv4Addr> {
        let address = match self.address {
            IpAddr::V4(ip) if host == self.host => Some(ip),
            _ => None,
        };
        Box::pin(async move { Ok(Box::new(address.into_iter()) as BoxIter<Ipv4Addr>) })
    }
    fn lookup_ipv6(&self, host: String) -> Lookup<Ipv6Addr> {
        let address = match self.address {
            IpAddr::V6(ip) if host == self.host => Some(ip),
            _ => None,
        };
        Box::pin(async move { Ok(Box::new(address.into_iter()) as BoxIter<Ipv6Addr>) })
    }
    fn lookup_txt(&self, _host: String) -> Lookup<TxtRecordData> {
        Box::pin(async { Err(n0_error::e!(DnsError::NoResponse)) })
    }
    fn clear_cache(&self) {}
    fn reset(&self) -> Box<dyn Resolver> {
        Box::new(self.clone())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let resolve = if let Some(index) = args.iter().position(|arg| arg == "--resolve") {
        if index + 1 >= args.len() {
            return Err("--resolve requires an IP address".into());
        }
        let address = args.remove(index + 1).parse::<IpAddr>()?;
        args.remove(index);
        Some(address)
    } else {
        None
    };
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
            "usage: relay_discovery_probe [--resolve IP] [--bind-addr IP:PORT] [RELAY_URL [UDP_PORT [CA_FILE]]]"
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
    if let Some(address) = resolve {
        let relay_url: iroh::RelayUrl = url.parse()?;
        let host = relay_url
            .host_str()
            .ok_or("relay URL requires a hostname")?
            .to_owned();
        builder = builder.dns_resolver(DnsResolver::custom(ProbeResolver { host, address }));
    }
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
