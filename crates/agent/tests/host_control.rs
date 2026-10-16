use std::os::unix::fs::PermissionsExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wipsnet_agent::{control, tunnel};
use wipsnet_core::{
    config::TransportConfig,
    identity::{EndpointId, Identity},
};
use wipsnet_registry::host::{self, Authority, Mutation};
fn dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    d
}
fn mutate(directory: &std::path::Path, host: EndpointId, operation: Mutation) {
    let (mut store, secret) = host::owner_store(directory, host).unwrap();
    store.host_mutate(host, &secret, operation).unwrap();
}
fn approve(directory: &std::path::Path, host: EndpointId, peer: EndpointId, rev: u64) {
    mutate(
        directory,
        host,
        Mutation::Approve {
            expected_revision: rev,
            peer,
            name: "client".into(),
        },
    );
}
fn service(
    directory: &std::path::Path,
    host: EndpointId,
    peer: EndpointId,
    backend: std::net::SocketAddr,
    rev: u64,
) {
    mutate(
        directory,
        host,
        Mutation::Service {
            expected_revision: rev,
            id: "llama".into(),
            backend,
            allow: vec![peer],
        },
    );
}
fn config() -> TransportConfig {
    TransportConfig {
        relay_url: None,
        relay_only: false,
        relay_ca_file: None,
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        max_connections: 8,
        max_streams: 8,
        max_streams_per_connection: 4,
    }
}
#[tokio::test]
async fn symmetric_hosts_share_protocol_endpoint_and_isolate_local_authority() {
    let a = Identity::generate();
    let b = Identity::generate();
    let client = Identity::generate();
    let da = dir();
    let db = dir();
    host::initialize(da.path(), a.endpoint_id()).unwrap();
    host::initialize(db.path(), b.endpoint_id()).unwrap();
    approve(da.path(), a.endpoint_id(), client.endpoint_id(), 0);
    let backend = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    service(
        da.path(),
        a.endpoint_id(),
        client.endpoint_id(),
        backend.local_addr().unwrap(),
        1,
    );
    let ah = tunnel::TunnelHost::bind_host(
        a.secret_key(),
        &config(),
        Authority::open(da.path(), a.endpoint_id()).unwrap(),
    )
    .await
    .unwrap();
    let bh = tunnel::TunnelHost::bind_host(
        b.secret_key(),
        &config(),
        Authority::open(db.path(), b.endpoint_id()).unwrap(),
    )
    .await
    .unwrap();
    let outgoing = tunnel::bind_client(client.secret_key()).await.unwrap();
    let aa = ah.endpoint().bound_sockets()[0];
    let ba = bh.endpoint().bound_sockets()[0];
    let cc = control::connect(&outgoing, a.endpoint_id(), aa)
        .await
        .unwrap();
    let catalog = control::catalog_on(&cc, a.endpoint_id()).await.unwrap();
    assert_eq!(catalog.host, a.endpoint_id());
    assert_eq!(catalog.services[0].id, "llama");
    assert_eq!(catalog.revision, 2);
    let denied = control::connect(&outgoing, b.endpoint_id(), ba)
        .await
        .unwrap();
    assert!(control::catalog_on(&denied, b.endpoint_id()).await.is_err());
    // Control and service coexist at exactly the same endpoint/UDP address.
    let sc = tunnel::connect(&outgoing, a.endpoint_id(), aa)
        .await
        .unwrap();
    let (parallel_catalog, parallel_stream) = tokio::join!(
        control::catalog_on(&cc, a.endpoint_id()),
        tunnel::open_service(&sc, "llama")
    );
    assert_eq!(parallel_catalog.unwrap().host, a.endpoint_id());
    let mut stream = parallel_stream.unwrap();
    let (mut tcp, _) = backend.accept().await.unwrap();
    tcp.write_all(b"hello").await.unwrap();
    let mut data = [0; 5];
    stream.read_exact(&mut data).await.unwrap();
    assert_eq!(&data, b"hello");
    drop(stream);
    drop(tcp);
    // Owner credentials are local and host-scoped, not delegated by peer approval.
    let (_, a_owner) = host::owner_store(da.path(), a.endpoint_id()).unwrap();
    let (mut b_store, _) = host::owner_store(db.path(), b.endpoint_id()).unwrap();
    assert!(
        b_store
            .host_mutate(
                b.endpoint_id(),
                &a_owner,
                Mutation::Approve {
                    expected_revision: 0,
                    peer: client.endpoint_id(),
                    name: "transitive".into()
                }
            )
            .is_err()
    );
    // A goes offline. B can approve/register/serve and restart using only B state.
    ah.shutdown().await;
    approve(db.path(), b.endpoint_id(), client.endpoint_id(), 0);
    service(
        db.path(),
        b.endpoint_id(),
        client.endpoint_id(),
        backend.local_addr().unwrap(),
        1,
    );
    let bc = control::connect(&outgoing, b.endpoint_id(), ba)
        .await
        .unwrap();
    let catalog = control::catalog_on(&bc, b.endpoint_id()).await.unwrap();
    assert_eq!(catalog.host, b.endpoint_id());
    assert_eq!(catalog.revision, 2);
    assert_eq!(catalog.services[0].id, "llama");
    let bs = tunnel::connect(&outgoing, b.endpoint_id(), ba)
        .await
        .unwrap();
    let stream = tunnel::open_service(&bs, "llama").await.unwrap();
    let (tcp, _) = backend.accept().await.unwrap();
    drop(stream);
    drop(tcp);
    // Revocation is read again on an existing control/service connection.
    mutate(
        db.path(),
        b.endpoint_id(),
        Mutation::Revoke {
            expected_revision: 2,
            peer: client.endpoint_id(),
        },
    );
    assert!(control::catalog_on(&bc, b.endpoint_id()).await.is_err());
    assert!(tunnel::open_service(&bs, "llama").await.is_err());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), backend.accept())
            .await
            .is_err()
    );
    bh.shutdown().await;
    let b_restart = tunnel::TunnelHost::bind_host(
        b.secret_key(),
        &config(),
        Authority::open(db.path(), b.endpoint_id()).unwrap(),
    )
    .await
    .unwrap();
    let denied = control::connect(
        &outgoing,
        b.endpoint_id(),
        b_restart.endpoint().bound_sockets()[0],
    )
    .await
    .unwrap();
    assert!(control::catalog_on(&denied, b.endpoint_id()).await.is_err());
    b_restart.shutdown().await;
    outgoing.close().await;
}
#[tokio::test]
async fn malformed_control_cannot_mutate_or_open_backends_and_shutdown_drains() {
    let host_key = Identity::generate();
    let client = Identity::generate();
    let d = dir();
    host::initialize(d.path(), host_key.endpoint_id()).unwrap();
    approve(d.path(), host_key.endpoint_id(), client.endpoint_id(), 0);
    let host = tunnel::TunnelHost::bind_host(
        host_key.secret_key(),
        &config(),
        Authority::open(d.path(), host_key.endpoint_id()).unwrap(),
    )
    .await
    .unwrap();
    let counts = host.counts();
    let endpoint = tunnel::bind_client(client.secret_key()).await.unwrap();
    let connection = control::connect(
        &endpoint,
        host_key.endpoint_id(),
        host.endpoint().bound_sockets()[0],
    )
    .await
    .unwrap();
    for bytes in [
        br#"{"op":"approve","peer":"forged"}"#.as_slice(),
        br#"{"op":"catalog","host":"forged"}"#.as_slice(),
    ] {
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        send.write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .unwrap();
        send.write_all(bytes).await.unwrap();
        send.finish().unwrap();
        assert!(recv.read_to_end(1024).await.is_err());
    }
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    send.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
    send.finish().unwrap();
    assert!(recv.read_to_end(1024).await.is_err());
    assert_eq!(
        control::catalog_on(&connection, host_key.endpoint_id())
            .await
            .unwrap()
            .revision,
        1
    );
    let (mut stalled, _) = connection.open_bi().await.unwrap();
    stalled.write_all(&[0, 0]).await.unwrap();
    host.shutdown().await;
    assert_eq!(counts.connections(), 0);
    assert_eq!(counts.streams(), 0);
    endpoint.close().await;
}

async fn tls_relay(
    certificate: &rcgen::CertifiedKey<rcgen::KeyPair>,
    address: std::net::SocketAddr,
) -> iroh_relay::server::Server {
    use iroh_relay::server::{CertConfig, RelayConfig, Server, ServerConfig, TlsConfig};
    let tls = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.cert.der().clone()],
        rustls_pki_types::PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der()).into(),
    )
    .unwrap();
    let mut relay = RelayConfig::new("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap());
    relay.tls = Some(TlsConfig::new(
        address,
        CertConfig::Manual { server_config: tls },
    ));
    let mut cfg = ServerConfig::default();
    cfg.relay = Some(relay);
    Server::spawn(cfg).await.unwrap()
}

#[tokio::test]
async fn explicit_tls_relay_without_udp_pins_identity_enforces_grants_and_recovers() {
    use std::time::Duration;
    use tokio::time::timeout;
    let certificate = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let relay = tls_relay(&certificate, "127.0.0.1:0".parse().unwrap()).await;
    let relay_addr = relay.https_addr().unwrap();
    let trust = dir();
    let ca = trust.path().join("relay.pem");
    std::fs::write(&ca, certificate.cert.pem()).unwrap();
    std::fs::set_permissions(&ca, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut cfg = config();
    cfg.relay_url = Some(format!("https://{relay_addr}"));
    cfg.relay_only = true;
    cfg.relay_ca_file = Some(ca);
    let key = Identity::generate();
    let client = Identity::generate();
    let state = dir();
    host::initialize(state.path(), key.endpoint_id()).unwrap();
    approve(state.path(), key.endpoint_id(), client.endpoint_id(), 0);
    let backend = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    service(
        state.path(),
        key.endpoint_id(),
        client.endpoint_id(),
        backend.local_addr().unwrap(),
        1,
    );
    let host = tunnel::TunnelHost::bind_host(
        key.secret_key(),
        &cfg,
        Authority::open(state.path(), key.endpoint_id()).unwrap(),
    )
    .await
    .unwrap();
    let outgoing = tunnel::bind_configured_client(client.secret_key(), Some(&cfg))
        .await
        .unwrap();
    assert!(host.endpoint().bound_sockets().is_empty());
    assert!(outgoing.bound_sockets().is_empty());
    timeout(Duration::from_secs(10), async {
        tokio::join!(host.endpoint().online(), outgoing.online());
    })
    .await
    .unwrap();
    let target = tunnel::peer_target(key.endpoint_id(), Some(&cfg), None).unwrap();
    let connection = control::connect_target(&outgoing, target.clone())
        .await
        .unwrap();
    assert!(connection.paths().iter().any(|p| p.is_relay()));
    assert_eq!(
        control::catalog_on(&connection, key.endpoint_id())
            .await
            .unwrap()
            .services
            .len(),
        1
    );
    assert!(
        control::catalog_on(&connection, client.endpoint_id())
            .await
            .is_err()
    );
    let service_connection = tunnel::connect_target(&outgoing, target.clone())
        .await
        .unwrap();
    let mut stream = tunnel::open_service(&service_connection, "llama")
        .await
        .unwrap();
    let (mut tcp, _) = timeout(Duration::from_secs(2), backend.accept())
        .await
        .unwrap()
        .unwrap();
    stream.write_all(b"relay-only").await.unwrap();
    let mut data = [0; 10];
    timeout(Duration::from_secs(2), tcp.read_exact(&mut data))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&data, b"relay-only");
    tcp.write_all(b"reply").await.unwrap();
    let mut reply = [0; 5];
    timeout(Duration::from_secs(2), stream.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply, b"reply");
    drop(stream);
    drop(tcp);
    // Admission at the relay is deliberately AllowAll, but host approval is separate.
    let stranger = tunnel::bind_configured_client(Identity::generate().secret_key(), Some(&cfg))
        .await
        .unwrap();
    timeout(Duration::from_secs(10), stranger.online())
        .await
        .unwrap();
    if let Ok(denied) = control::connect_target(&stranger, target.clone()).await {
        assert!(
            control::catalog_on(&denied, key.endpoint_id())
                .await
                .is_err()
        );
    }
    // One unavailable pinned peer never holds up an independent live-peer request.
    let absent = tunnel::peer_target(Identity::generate().endpoint_id(), Some(&cfg), None).unwrap();
    let (missing, live) = tokio::join!(
        control::connect_target(&outgoing, absent),
        timeout(
            Duration::from_secs(2),
            control::catalog_on(&connection, key.endpoint_id())
        )
    );
    assert!(missing.is_err());
    assert!(live.unwrap().is_ok());
    // Rebuild a stateless relay at the same address; no endpoint/key/grant replacement.
    connection.close(0u32.into(), b"test reconnect");
    service_connection.close(0u32.into(), b"test reconnect");
    relay.shutdown().await.unwrap();
    assert!(
        control::connect_target(&outgoing, target.clone())
            .await
            .is_err()
    );
    let relay = tls_relay(&certificate, relay_addr).await;
    timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(c) = control::connect_target(&outgoing, target.clone()).await
                && control::catalog_on(&c, key.endpoint_id()).await.is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(outgoing.id(), client.endpoint_id());
    assert_eq!(host.endpoint().id(), key.endpoint_id());
    // No replay of the previous service operation during relay recovery.
    assert!(
        timeout(Duration::from_millis(100), backend.accept())
            .await
            .is_err()
    );
    // The explicit certificate is required; no insecure TLS fallback.
    cfg.relay_ca_file = None;
    let untrusted = tunnel::bind_configured_client(Identity::generate().secret_key(), Some(&cfg))
        .await
        .unwrap();
    assert!(
        timeout(Duration::from_secs(1), untrusted.online())
            .await
            .is_err()
    );
    untrusted.close().await;
    stranger.close().await;
    outgoing.close().await;
    host.shutdown().await;
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn existing_direct_path_survives_configured_relay_outage() {
    use std::time::Duration;
    use tokio::time::timeout;
    let certificate = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let relay = tls_relay(&certificate, "127.0.0.1:0".parse().unwrap()).await;
    let trust = dir();
    let ca = trust.path().join("relay.pem");
    std::fs::write(&ca, certificate.cert.pem()).unwrap();
    let mut cfg = config();
    cfg.relay_url = Some(format!("https://{}", relay.https_addr().unwrap()));
    cfg.relay_ca_file = Some(ca);
    let key = Identity::generate();
    let client = Identity::generate();
    let state = dir();
    host::initialize(state.path(), key.endpoint_id()).unwrap();
    approve(state.path(), key.endpoint_id(), client.endpoint_id(), 0);
    let host = tunnel::TunnelHost::bind_host(
        key.secret_key(),
        &cfg,
        Authority::open(state.path(), key.endpoint_id()).unwrap(),
    )
    .await
    .unwrap();
    let outgoing = tunnel::bind_configured_client(client.secret_key(), Some(&cfg))
        .await
        .unwrap();
    let target = tunnel::peer_target(
        key.endpoint_id(),
        Some(&cfg),
        Some(host.endpoint().bound_sockets()[0]),
    )
    .unwrap();
    let connection = control::connect_target(&outgoing, target).await.unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if connection
                .paths()
                .iter()
                .any(|p| p.is_ip() && p.is_selected())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    relay.shutdown().await.unwrap();
    assert_eq!(
        control::catalog_on(&connection, key.endpoint_id())
            .await
            .unwrap()
            .host,
        key.endpoint_id()
    );
    outgoing.close().await;
    host.shutdown().await;
}
