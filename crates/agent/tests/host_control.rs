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
