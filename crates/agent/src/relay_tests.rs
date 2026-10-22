//! Real DNS/TLS recovery fixtures; no hosts-file edits or live DNS dependencies.
use super::*;
use crate::control;
use iroh::dns::{DnsResolver, NameserverConfig};
use n0_future::StreamExt;
use std::{
    net::{Ipv4Addr, SocketAddr},
    os::unix::fs::PermissionsExt,
    sync::Mutex,
};
use wispnet_core::identity::Identity;
use wispnet_registry::host::{self, Authority, Mutation};

struct DnsFixture {
    address: SocketAddr,
    replacement: Arc<std::sync::atomic::AtomicBool>,
    questions: Arc<Mutex<Vec<String>>>,
    worker: JoinHandle<()>,
}
impl DnsFixture {
    async fn start() -> Self {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let replacement = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let questions = Arc::new(Mutex::new(Vec::new()));
        let replaced = replacement.clone();
        let log = questions.clone();
        let worker = tokio::spawn(async move {
            let mut buffer = [0; 4096];
            loop {
                let (size, source) = socket.recv_from(&mut buffer).await.unwrap();
                let packet = &buffer[..size];
                assert!(size >= 17 && packet[4..6] == [0, 1]);
                let mut cursor = 12;
                let mut labels = Vec::new();
                while packet[cursor] != 0 {
                    let length = packet[cursor] as usize;
                    cursor += 1;
                    labels.push(std::str::from_utf8(&packet[cursor..cursor + length]).unwrap());
                    cursor += length;
                }
                cursor += 1;
                let kind = u16::from_be_bytes([packet[cursor], packet[cursor + 1]]);
                let name = labels.join(".");
                log.lock().unwrap().push(name.clone());
                assert_eq!(name, "relay.test", "unexpected node discovery DNS query");
                let mut response = packet[..2].to_vec();
                let ipv6 = replaced.load(Ordering::SeqCst);
                let answers = (!ipv6 && kind == 1) || (ipv6 && kind == 28);
                response.extend_from_slice(&[0x81, 0x80, 0, 1, 0, u8::from(answers), 0, 0, 0, 0]);
                response.extend_from_slice(&packet[12..cursor + 4]);
                if answers {
                    // Switch loopback IP families without privileged interface
                    // aliases: macOS does not bind arbitrary 127/8 addresses.
                    // One-second TTL still exercises ordinary cache expiry.
                    let address = if ipv6 {
                        std::net::Ipv6Addr::LOCALHOST.octets().to_vec()
                    } else {
                        Ipv4Addr::LOCALHOST.octets().to_vec()
                    };
                    response.extend_from_slice(&[0xc0, 0x0c]);
                    response.extend_from_slice(&kind.to_be_bytes());
                    response.extend_from_slice(&[0, 1, 0, 0, 0, 1, 0, address.len() as u8]);
                    response.extend_from_slice(&address);
                }
                socket.send_to(&response, source).await.unwrap();
            }
        });
        Self {
            address,
            replacement,
            questions,
            worker,
        }
    }
    fn resolver(&self) -> DnsResolver {
        DnsResolver::builder()
            .add_nameserver_config(
                NameserverConfig::udp(self.address.ip()).with_port(self.address.port()),
            )
            .disable_fallback()
            .build()
    }
}
impl Drop for DnsFixture {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

async fn relay(
    cert: &rcgen::CertifiedKey<rcgen::KeyPair>,
    address: SocketAddr,
) -> iroh_relay::server::Server {
    relay_with_access(cert, address, Arc::new(iroh_relay::server::AllowAll)).await
}

async fn relay_with_access(
    cert: &rcgen::CertifiedKey<rcgen::KeyPair>,
    address: SocketAddr,
    access: Arc<dyn iroh_relay::server::DynAccessControl>,
) -> iroh_relay::server::Server {
    use iroh_relay::server::{CertConfig, RelayConfig, Server, ServerConfig, TlsConfig};
    let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.cert.der().clone()],
        rustls_pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
    )
    .unwrap();
    let mut config = RelayConfig::new(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)));
    config.access = access;
    config.tls = Some(TlsConfig::new(
        address,
        CertConfig::Manual { server_config: tls },
    ));
    let mut server = ServerConfig::default();
    server.relay = Some(config);
    Server::spawn(server).await.unwrap()
}

#[tokio::test]
async fn relay_dns_ip_replacement_recovers_without_endpoint_or_policy_reset() {
    let dns = DnsFixture::start().await;
    let cert = rcgen::generate_simple_self_signed(vec!["relay.test".into()]).unwrap();
    let original = relay(&cert, "127.0.0.1:0".parse().unwrap()).await;
    let port = original.https_addr().unwrap().port();
    let state = tempfile::tempdir().unwrap();
    std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let ca = state.path().join("relay.pem");
    std::fs::write(&ca, cert.cert.pem()).unwrap();
    let config = TransportConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        discovery_relay_url: None,
        relay_url: Some(format!("https://relay.test:{port}")),
        relay_only: true,
        relay_ca_file: Some(ca),
        max_connections: 8,
        max_streams: 8,
        max_streams_per_connection: 4,
    };
    let host_key = Identity::generate();
    let client_key = Identity::generate();
    host::initialize(state.path(), host_key.endpoint_id()).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host_key.endpoint_id()).unwrap();
    store
        .host_mutate(
            host_key.endpoint_id(),
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer: client_key.endpoint_id(),
                name: "client".into(),
            },
        )
        .unwrap();
    drop(store);
    drop(owner);
    let authority = Authority::open(state.path(), host_key.endpoint_id()).unwrap();
    let builder = configured_builder(host_key.secret_key(), Some(&config))
        .unwrap()
        .dns_resolver(dns.resolver());
    let host = TunnelHost::bind_builder(&config, Services::Host(authority.clone()), builder)
        .await
        .unwrap();
    let client = configured_builder(client_key.secret_key(), Some(&config))
        .unwrap()
        .dns_resolver(dns.resolver())
        .bind()
        .await
        .unwrap();
    timeout(Duration::from_secs(10), async {
        tokio::join!(host.endpoint().online(), client.online());
    })
    .await
    .unwrap();
    // A stale direct hint must not prevent reaching the pinned relay-only host.
    let target = peer_target(
        host_key.endpoint_id(),
        Some(&config),
        Some("127.0.0.9:9".parse().unwrap()),
    )
    .unwrap();
    let diagnostic = crate::diagnostics::explain_path(
        &client,
        host_key.endpoint_id(),
        target.clone(),
        true,
        true,
    )
    .await
    .unwrap();
    assert_eq!(diagnostic["selected_path"], "relay");
    assert_eq!(
        diagnostic["reason"],
        "This device is configured to use relay only."
    );
    assert!(diagnostic["network_report"].is_null() || diagnostic["network_report"].is_object());
    assert!(
        crate::diagnostics::explain_path(
            &client,
            client_key.endpoint_id(),
            target.clone(),
            false,
            true,
        )
        .await
        .is_err()
    );
    let before_connection = control::connect_target(&client, target.clone())
        .await
        .unwrap();
    let before = control::catalog_on(&before_connection, host_key.endpoint_id())
        .await
        .unwrap();
    let observation = tokio::time::Instant::now();
    crate::diagnostics::settle_path(&before_connection).await;
    assert!(observation.elapsed() >= Duration::from_secs(1));
    assert_eq!(
        crate::diagnostics::selected_path(&before_connection),
        "relay"
    );
    before_connection.close(0u32.into(), b"fixture reconnect");
    // Once closed, observation must return promptly instead of waiting again.
    timeout(
        Duration::from_millis(100),
        crate::diagnostics::settle_path(&before_connection),
    )
    .await
    .unwrap();
    original.shutdown().await.unwrap();
    let changed = relay(
        &cert,
        SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port)),
    )
    .await;
    dns.replacement.store(true, Ordering::SeqCst);
    // Do not reset endpoints, resolver caches, or peer pins. Recovery must use
    // ordinary TTL expiry plus Iroh's independent outbound reconnect/backoff.
    timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(connection) = control::connect_target(&client, target.clone()).await
                && let Ok(after) = control::catalog_on(&connection, host_key.endpoint_id()).await
            {
                assert_eq!(
                    serde_json::to_value(&after).unwrap(),
                    serde_json::to_value(&before).unwrap()
                );
                assert!(connection.paths().iter().any(|p| p.is_relay()));
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(host.endpoint().id(), host_key.endpoint_id());
    assert_eq!(client.id(), client_key.endpoint_id());
    assert!(authority.approved(client_key.endpoint_id()).await);
    {
        let questions = dns.questions.lock().unwrap();
        assert!(questions.len() >= 4);
        assert!(questions.iter().all(|name| name == "relay.test"));
    }
    assert!(
        !dns.worker.is_finished(),
        "DNS fixture failed while observing queries"
    );
    client.close().await;
    host.shutdown().await;
    changed.shutdown().await.unwrap();
}

#[derive(Debug)]
struct DenyAdmission;
impl iroh_relay::server::AccessControl for DenyAdmission {
    async fn on_connect(
        &self,
        _: &iroh_relay::server::ClientRequest,
    ) -> iroh_relay::server::Access {
        iroh_relay::server::Access::Deny {
            reason: Some("SECRET-SENTINEL untrusted relay reason".into()),
        }
    }
}

#[tokio::test]
async fn relay_admission_diagnostic_is_distinct_and_redacts_server_reason() {
    use iroh::Watcher;
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let relay = relay_with_access(
        &cert,
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(DenyAdmission),
    )
    .await;
    let trust = tempfile::tempdir().unwrap();
    let ca = trust.path().join("relay.pem");
    std::fs::write(&ca, cert.cert.pem()).unwrap();
    let config = TransportConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        discovery_relay_url: None,
        relay_url: Some(format!("https://{}", relay.https_addr().unwrap())),
        relay_only: true,
        relay_ca_file: Some(ca),
        max_connections: 8,
        max_streams: 8,
        max_streams_per_connection: 4,
    };
    let key = Identity::generate();
    let endpoint = bind_configured_client(key.secret_key(), Some(&config))
        .await
        .unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if endpoint
                .home_relay_status()
                .get()
                .iter()
                .any(|r| r.auth_denied_reason().is_some())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let target = peer_target(Identity::generate().endpoint_id(), Some(&config), None).unwrap();
    let error = control::connect_target(&endpoint, target)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        TunnelError::Transport("configured relay denied endpoint admission")
    );
    assert!(!format!("{error} {error:?}").contains("SECRET-SENTINEL"));
    endpoint.close().await;
    relay.shutdown().await.unwrap();
}

#[test]
fn outside_discovery_is_not_a_packet_relay_destination() {
    let mut config: TransportConfig = serde_json::from_value(serde_json::json!({
        "bind_addr":"0.0.0.0:0", "relay_url":"https://home.example"
    }))
    .unwrap();
    let legacy = configured_relay_mode(&config).unwrap().unwrap().relay_map();
    let home: iroh::RelayUrl = "https://home.example".parse().unwrap();
    let outside: iroh::RelayUrl = "https://outside.example".parse().unwrap();
    assert!(legacy.get(&home).unwrap().quic.is_some());
    assert_eq!(legacy.len(), 1);
    config.discovery_relay_url = Some(outside.to_string());
    let map = configured_relay_mode(&config).unwrap().unwrap().relay_map();
    assert_eq!(map.len(), 1);
    assert!(map.contains(&home));
    assert!(!map.contains(&outside));
    let discovery = configured_discovery_relay(&config).unwrap().unwrap();
    assert_eq!(discovery.url, outside);
    assert_ne!(discovery.url, home);
    assert_eq!(discovery.quic.as_ref().unwrap().port, 7842);
    // Pairing/bootstrap still names the primary, regardless of relay selection.
    let target = peer_target(Identity::generate().endpoint_id(), Some(&config), None).unwrap();
    assert_eq!(target.relay_urls().next(), Some(&home));
}

#[tokio::test]
async fn primary_discovery_timeout_uses_same_socket_fallback_without_changing_home_relay() {
    use iroh::Watcher;
    use iroh_relay::server::{
        CertConfig, QuicConfig, RelayConfig, Server, ServerConfig, TlsConfig,
    };
    let cert = rcgen::generate_simple_self_signed(vec!["relay.test".into()]).unwrap();
    let primary = relay(&cert, "127.0.0.1:0".parse().unwrap()).await;
    let primary_url: iroh::RelayUrl = format!(
        "https://relay.test:{}",
        primary.https_addr().unwrap().port()
    )
    .parse()
    .unwrap();
    let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.cert.der().clone()],
        rustls_pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
    )
    .unwrap();
    let mut relay_config = RelayConfig::new("127.0.0.1:0".parse::<SocketAddr>().unwrap());
    relay_config.tls = Some(TlsConfig::new(
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        CertConfig::Manual { server_config: tls },
    ));
    let mut server_config = ServerConfig::default();
    server_config.relay = Some(relay_config);
    server_config.quic = Some(QuicConfig::new(
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    ));
    let observer = Server::spawn(server_config).await.unwrap();
    let observer_url: iroh::RelayUrl = format!(
        "https://relay.test:{}",
        observer.https_addr().unwrap().port()
    )
    .parse()
    .unwrap();
    let fallback = iroh::RelayConfig::new(
        observer_url.clone(),
        Some(iroh_relay::RelayQuicConfig::new(
            observer.quic_addr().unwrap().port(),
        )),
    );
    let unused = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let unused_port = unused.local_addr().unwrap().port();
    drop(unused);
    let primary_config = iroh::RelayConfig::new(
        primary_url.clone(),
        Some(iroh_relay::RelayQuicConfig::new(unused_port)),
    );
    let dns = DnsFixture::start().await;
    let endpoint = Endpoint::builder(presets::Minimal)
        .dns_resolver(dns.resolver())
        .relay_mode(iroh::RelayMode::Custom(
            [primary_config].into_iter().collect(),
        ))
        .ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots([cert
            .cert
            .der()
            .clone()]))
        .net_report_config(
            iroh::endpoint::NetReportConfig::default().quic_discovery_fallback(fallback),
        )
        .bind()
        .await
        .unwrap();
    let report = timeout(Duration::from_secs(8), async {
        let mut reports = endpoint.net_report().stream();
        while let Some(report) = reports.next().await {
            if let Some(report) = report
                && report.has_udp()
                && report.preferred_relay.as_ref() == Some(&primary_url)
            {
                return report;
            }
        }
        panic!("network report stream ended");
    })
    .await
    .unwrap();
    // Primary has no QAD listener: this response can only come from the observer.
    // Its observed source port must belong to this endpoint, not a helper socket.
    let observed_port = report.global_v4.unwrap().port();
    assert!(
        endpoint
            .bound_sockets()
            .iter()
            .any(|a| a.is_ipv4() && a.port() == observed_port)
    );
    timeout(Duration::from_secs(5), endpoint.online())
        .await
        .unwrap();
    let homes = endpoint.home_relay_status().get();
    assert!(
        homes
            .iter()
            .any(|r| r.url() == &primary_url && r.is_connected())
    );
    assert!(homes.iter().all(|r| r.url() != &observer_url));
    endpoint.close().await;
    observer.shutdown().await.unwrap();
    primary.shutdown().await.unwrap();
}

// Exercise the exact generic policy used by the vendored dependency without
// pulling its unrelated upstream development dependencies into this workspace.
#[path = "../../../vendor/iroh/src/net_report/fallback.rs"]
mod discovery_fallback_policy;
