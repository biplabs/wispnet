//! Android OS A/AAAA resolution without a Java VM. No TXT peer discovery.
use iroh::dns::{BoxIter, DnsError, DnsResolver, Resolver, TxtRecordData};
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs},
    pin::Pin,
};
type Lookup<T> = Pin<Box<dyn Future<Output = Result<BoxIter<T>, DnsError>> + Send>>;

#[derive(Debug, Clone, Copy)]
struct OsResolver;

async fn addresses(host: String) -> Result<Vec<IpAddr>, DnsError> {
    let task = tokio::task::spawn_blocking(move || {
        (host.as_str(), 0)
            .to_socket_addrs()
            .map(|iter| iter.map(|address| address.ip()).collect::<Vec<_>>())
    });
    match tokio::time::timeout(std::time::Duration::from_secs(3), task).await {
        Ok(Ok(Ok(addresses))) if !addresses.is_empty() => Ok(addresses),
        _ => Err(n0_error::e!(DnsError::NoResponse)),
    }
}
impl Resolver for OsResolver {
    fn lookup_ipv4(&self, host: String) -> Lookup<Ipv4Addr> {
        Box::pin(async move {
            let addresses = addresses(host)
                .await?
                .into_iter()
                .filter_map(|ip| match ip {
                    IpAddr::V4(ip) => Some(ip),
                    _ => None,
                })
                .collect::<Vec<_>>();
            Ok(Box::new(addresses.into_iter()) as BoxIter<Ipv4Addr>)
        })
    }
    fn lookup_ipv6(&self, host: String) -> Lookup<Ipv6Addr> {
        Box::pin(async move {
            let addresses = addresses(host)
                .await?
                .into_iter()
                .filter_map(|ip| match ip {
                    IpAddr::V6(ip) => Some(ip),
                    _ => None,
                })
                .collect::<Vec<_>>();
            Ok(Box::new(addresses.into_iter()) as BoxIter<Ipv6Addr>)
        })
    }
    fn lookup_txt(&self, _host: String) -> Lookup<TxtRecordData> {
        Box::pin(async { Err(n0_error::e!(DnsError::NoResponse)) })
    }
    fn clear_cache(&self) {} // No application DNS cache; the OS owns freshness.
    fn reset(&self) -> Box<dyn Resolver> {
        Box::new(Self)
    }
}
pub(crate) fn resolver() -> DnsResolver {
    DnsResolver::custom(OsResolver)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn system_addresses_work_after_reset_and_txt_discovery_is_disabled() {
        let dns = resolver();
        let addresses = dns
            .lookup_ipv4("localhost", std::time::Duration::from_secs(3))
            .await
            .unwrap();
        assert!(addresses.into_iter().any(|ip| ip.is_loopback()));
        dns.reset();
        assert!(
            dns.lookup_ipv4("localhost", std::time::Duration::from_secs(3))
                .await
                .is_ok()
        );
        assert!(
            dns.lookup_txt(
                "_iroh.must-not-be-queried.invalid",
                std::time::Duration::from_secs(1)
            )
            .await
            .is_err()
        );
    }
}
