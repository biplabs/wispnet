use std::{os::unix::fs::PermissionsExt, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::timeout,
};
use wipsnet_core::{config::TransportConfig, identity::Identity};
use wipsnet_registry::host::{self, Authority, Mutation};

async fn command(config: &Path, args: &[&str], input: &[u8]) -> String {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_wipsnet"))
        .arg(args[0])
        .arg("--config")
        .arg(config)
        .args(&args[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(input).await.unwrap();
    drop(stdin);
    let output = timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[tokio::test]
async fn native_catalog_and_service_need_no_peer_address() {
    use irelay::server::{CertConfig, RelayConfig, Server, ServerConfig, TlsConfig};
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let tls = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
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
    let mut relay = RelayConfig::new("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap());
    relay.tls = Some(TlsConfig::new(
        "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap(),
        CertConfig::Manual { server_config: tls },
    ));
    let mut server = ServerConfig::default();
    server.relay = Some(relay);
    let relay = Server::spawn(server).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let ca = tmp.path().join("relay.pem");
    std::fs::write(&ca, cert.cert.pem()).unwrap();
    let relay_url = format!("https://{}", relay.https_addr().unwrap());
    let client_config = tmp.path().join("client.toml");
    std::fs::write(&client_config, format!("schema_version=1\ndevice_name='client'\nstate_dir='{}'\nlog_level='info'\n[transport]\nbind_addr='127.0.0.1:0'\nrelay_url='{relay_url}'\nrelay_only=true\nrelay_ca_file='{}'\n", tmp.path().join("client").display(), ca.display())).unwrap();
    let initialized = command(&client_config, &["identity-init"], b"").await;
    let peer = initialized
        .split("Endpoint ID: ")
        .nth(1)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let host_state = tmp.path().join("host");
    std::fs::create_dir(&host_state).unwrap();
    std::fs::set_permissions(&host_state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let identity = Identity::generate();
    host::initialize(&host_state, identity.endpoint_id()).unwrap();
    let backend = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (mut store, owner) = host::owner_store(&host_state, identity.endpoint_id()).unwrap();
    store
        .host_mutate(
            identity.endpoint_id(),
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer,
                name: "client".into(),
            },
        )
        .unwrap();
    store
        .host_mutate(
            identity.endpoint_id(),
            &owner,
            Mutation::Service {
                expected_revision: 1,
                id: "echo".into(),
                backend: backend.local_addr().unwrap(),
                allow: vec![peer],
            },
        )
        .unwrap();
    drop(store);
    drop(owner);
    let cfg = TransportConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        relay_url: Some(relay_url),
        relay_only: true,
        relay_ca_file: Some(ca),
        max_connections: 8,
        max_streams: 8,
        max_streams_per_connection: 4,
    };
    let host = wipsnet_agent::tunnel::TunnelHost::bind_host(
        identity.secret_key(),
        &cfg,
        Authority::open(&host_state, identity.endpoint_id()).unwrap(),
    )
    .await
    .unwrap();
    timeout(Duration::from_secs(10), host.endpoint().online())
        .await
        .unwrap();
    let pin = identity.endpoint_id().to_string();
    let catalog = command(&client_config, &["host-catalog", "--peer", &pin], b"").await;
    let catalog: serde_json::Value = serde_json::from_str(&catalog).unwrap();
    assert_eq!(catalog["host"], pin);
    assert_eq!(catalog["services"][0]["id"], "echo");
    let echo = async {
        let (mut socket, _) = backend.accept().await.unwrap();
        let mut request = Vec::new();
        socket.read_to_end(&mut request).await.unwrap();
        assert_eq!(request, b"request");
        socket.write_all(b"response").await.unwrap();
        socket.shutdown().await.unwrap();
    };
    let tunnel_args = ["tunnel", "--peer", &pin, "--service", "echo"];
    let (output, ()) = timeout(Duration::from_secs(10), async {
        tokio::join!(command(&client_config, &tunnel_args, b"request"), echo)
    })
    .await
    .unwrap();
    assert_eq!(output, "response");
    host.shutdown().await;
    relay.shutdown().await.unwrap();
}
