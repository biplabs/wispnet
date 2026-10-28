//! Real Iroh/public HTTP integration with a local authorization service and an
//! unchanged, bearer-protected backend. No external accounts or endpoints.
use super::*;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use std::{
    convert::Infallible,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use wispnet_agent::tunnel::TunnelHost;
use wispnet_core::identity::SecretKey;
use wispnet_registry::{
    backend_credentials::{self, Mode},
    host::{self, Authority, Mutation},
};

const PUBLISHER: &str = "host-publisher-fixture-credential-123456789";
const VISITOR: &str = "visitor-service-fixture-token";
const BACKEND: &str = "backend-fixture";
const SERVICE: &str = "gike-mcp";
const NAME: &str = "Desktop fixture";
const DOMAIN: &str = "wispnet.test";

async fn headers(stream: &mut (impl AsyncRead + Unpin)) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        while bytes.len() < 32768 {
            let mut byte = [0];
            stream.read_exact(&mut byte).await.unwrap();
            bytes.push(byte[0]);
            if bytes.ends_with(b"\r\n\r\n") {
                return bytes;
            }
        }
        panic!("fixture header limit");
    })
    .await
    .expect("fixture headers timed out")
}

async fn request(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    host: &str,
    path: &str,
    extra: &str,
) -> String {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    stream
        .write_all(
            format!(
                "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(8), stream.read_to_end(&mut bytes))
        .await
        .expect("fixture request timed out")
        .unwrap();
    String::from_utf8(bytes).unwrap()
}

struct Fixture {
    state: tempfile::TempDir,
    client: Arc<Client>,
    private_host: Option<TunnelHost>,
    peer: SecretKey,
    config: wispnet_core::config::TransportConfig,
    incarnation: String,
    name: String,
    resource: String,
    gateway: iroh::Endpoint,
    gateway_address: SocketAddr,
    port: u16,
    allowed: Arc<AtomicBool>,
    backend_calls: Arc<AtomicUsize>,
    authorizations: Arc<AtomicUsize>,
    hub: JoinHandle<()>,
    backend: JoinHandle<()>,
    ingress: JoinHandle<()>,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_backend(None, "/MCP").await
    }

    async fn with_backend(external: Option<SocketAddr>, mcp_path: &str) -> Self {
        let state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let key = SecretKey::generate();
        let owner = key.public();
        let peer = SecretKey::generate();
        host::initialize(state.path(), owner).unwrap();

        let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend_address = external.unwrap_or_else(|| backend_listener.local_addr().unwrap());
        let backend_calls = Arc::new(AtomicUsize::new(0));
        let calls = backend_calls.clone();
        let backend = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = backend_listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        calls.fetch_add(1, Ordering::SeqCst);
                        tasks.spawn(async move {
                            let head = String::from_utf8(headers(&mut socket).await).unwrap();
                            let lower = head.to_ascii_lowercase();
                            assert!(lower.contains(&format!("authorization: bearer {BACKEND}\r\n")));
                            assert_eq!(lower.matches("\r\nauthorization:").count(), 1);
                            assert!(!lower.contains(VISITOR));
                            assert!(!lower.contains("caller-override"));
                            assert!(!lower.contains("x-wispnet-user"));
                            if lower.starts_with("get /hold ") {
                                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\nd\r\ndata: first\n\n\r\n").await.unwrap();
                                std::future::pending::<()>().await;
                            }
                            assert!(head.starts_with("POST /MCP "));
                            let length = lower.lines().find_map(|line| {
                                line.strip_prefix("content-length:")?.trim().parse::<usize>().ok()
                            }).unwrap();
                            let mut body = vec![0; length];
                            socket.read_exact(&mut body).await.unwrap();
                            let message: Value = serde_json::from_slice(&body).unwrap();
                            assert_eq!(message["method"], "tools/list");
                            let reply = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
                            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes()).await.unwrap();
                        });
                    },
                    Some(result) = tasks.join_next(), if !tasks.is_empty() => { result.unwrap(); },
                }
            }
        });

        let (mut store, secret) = host::owner_store(state.path(), owner).unwrap();
        store
            .host_mutate(
                owner,
                &secret,
                Mutation::Approve {
                    expected_revision: 0,
                    peer: peer.public(),
                    name: "Alice computer".into(),
                },
            )
            .unwrap();
        store
            .host_mutate(
                owner,
                &secret,
                Mutation::Service {
                    expected_revision: 1,
                    id: SERVICE.into(),
                    backend: backend_address,
                    allow: vec![peer.public()],
                    display_name: None,
                    kind: None,
                },
            )
            .unwrap();
        drop(store);
        let authority = Authority::open(state.path(), owner).unwrap();
        let incarnation = authority
            .services()
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.id == SERVICE)
            .unwrap()
            .incarnation;
        backend_credentials::set(
            state.path(),
            owner,
            SERVICE,
            &incarnation,
            backend_address,
            if external.is_some() {
                Mode::None
            } else {
                Mode::Bearer
            },
            external.is_none().then_some(BACKEND),
        )
        .unwrap();
        let name = hostname(NAME, owner, SERVICE, DOMAIN);
        let resource = format!("https://{name}{mcp_path}");
        let hub_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", hub_listener.local_addr().unwrap());
        let allowed = Arc::new(AtomicBool::new(true));
        let authorizations = Arc::new(AtomicUsize::new(0));
        let (allowed_inner, count, incarnation_inner, resource_inner) = (
            allowed.clone(),
            authorizations.clone(),
            incarnation.clone(),
            resource.clone(),
        );
        let hub = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = hub_listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        let allowed = allowed_inner.clone();
                        let count = count.clone();
                        let incarnation = incarnation_inner.clone();
                        let resource = resource_inner.clone();
                        tasks.spawn(async move {
                            let serve = service_fn(move |request: Request<Incoming>| {
                                let allowed = allowed.clone();
                                let count = count.clone();
                                let incarnation = incarnation.clone();
                                let resource = resource.clone();
                                async move {
                                    let path = request.uri().path().to_owned();
                                    let publisher = request.headers().get("authorization").and_then(|h| h.to_str().ok()) == Some(&format!("Bearer {PUBLISHER}"));
                                    let body = request.into_body().collect().await.unwrap().to_bytes();
                                    let value: Value = serde_json::from_slice(&body).unwrap();
                                    let binding = publisher && value["host"] == owner.to_string() && value["service"] == SERVICE && value["incarnation"] == incarnation && value["resource"] == resource;
                                    let (status, response) = match path.as_str() {
                                        "/api/command/host/register" if binding => (200, json!({"ok":true,"resource":resource,"resource_id":"resource-fixture","auth_mode":"both"})),
                                        "/api/command/host/unlink" if publisher && value["host"] == owner.to_string() && allowed.load(Ordering::SeqCst) => (200, json!({"ok":true})),
                                        "/api/command/authorize" => {
                                            count.fetch_add(1, Ordering::SeqCst);
                                            if binding && value["token"] == VISITOR && allowed.load(Ordering::SeqCst) {
                                                (200, json!({"allowed":true,"subject":"alice","lease_seconds":5}))
                                            } else { (403, json!({"allowed":false})) }
                                        },
                                        _ => (403, json!({"error":"denied"})),
                                    };
                                    Ok::<_, Infallible>(Response::builder().status(status).header("content-type", "application/json").body(Full::new(Bytes::from(response.to_string()))).unwrap())
                                }
                            });
                            let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(socket), serve).await;
                        });
                    },
                    Some(result) = tasks.join_next(), if !tasks.is_empty() => { result.unwrap(); },
                }
            }
        });
        std::fs::write(
            state.path().join("command-center.json"),
            json!({"version":1,"owner":owner,"base_url":base_url,"publisher_token":PUBLISHER,"account":{"id":"owner","name":"Owner"},"pending":null}).to_string(),
        )
        .unwrap();
        std::fs::set_permissions(
            state.path().join("command-center.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let config: wispnet_core::config::TransportConfig =
            serde_json::from_value(json!({"bind_addr":"127.0.0.1:0"})).unwrap();
        let private_host = TunnelHost::bind_host(&key, &config, authority.clone())
            .await
            .unwrap();
        let client = Client::from_endpoint(
            private_host.endpoint().clone(),
            key,
            state.path().into(),
            config.clone(),
            authority,
        );
        let gateway =
            wispnet_agent::tunnel::bind_configured_client(&SecretKey::generate(), Some(&config))
                .await
                .unwrap();
        let mut gateway_address = gateway.bound_sockets()[0];
        gateway_address.set_ip("127.0.0.1".parse().unwrap());
        let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = http_listener.local_addr().unwrap().port();
        let ingress = ingress::Ingress::with_command_center(
            DOMAIN.into(),
            vec![owner],
            Some(ingress::CommandCenterConfig {
                // Accepted origin spellings must advertise and match the same
                // canonical issuer as the linked host and authorization server.
                base_url: format!("{base_url}/"),
                allow_legacy: false,
            }),
        )
        .unwrap();
        let ingress = tokio::spawn(ingress.run(gateway.clone(), http_listener));
        client.command(&json!({"op":"configure_public_gateway","gateway":{"endpoint_id":gateway.id(),"relay_url":"https://relay.invalid","domain":DOMAIN,"direct_addr":gateway_address}})).await.unwrap();
        client.command(&json!({"op":"configure_public","service":SERVICE,"incarnation":incarnation,"enabled":true,"name":NAME,"managed":true,"mcp_path":mcp_path})).await.unwrap();
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let status = client.publications.status(&client).await.unwrap();
                if status["services"][0]["state"] == "online" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("managed fixture did not publish");
        Self {
            state,
            client,
            private_host: Some(private_host),
            peer,
            config,
            incarnation,
            name,
            resource,
            gateway,
            gateway_address,
            port,
            allowed,
            backend_calls,
            authorizations,
            hub,
            backend,
            ingress,
        }
    }

    async fn public(&self, extra: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).await.unwrap();
        request(&mut stream, &self.name, "/MCP", extra).await
    }

    async fn metadata(&self, path: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).await.unwrap();
        stream
            .write_all(
                format!(
                    "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                    self.name
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        String::from_utf8(bytes).unwrap()
    }

    async fn private(&self, key: &SecretKey) -> Result<String, String> {
        let endpoint = wispnet_agent::tunnel::bind_configured_client(key, Some(&self.config))
            .await
            .unwrap();
        let mut address = self.client.endpoint.bound_sockets()[0];
        address.set_ip("127.0.0.1".parse().unwrap());
        let connection = endpoint
            .connect(
                iroh::EndpointAddr::new(self.client.endpoint.id()).with_ip_addr(address),
                protocol::ALPN,
            )
            .await
            .unwrap();
        let response = match wispnet_agent::tunnel::open_service_bound(
            &connection,
            SERVICE,
            &self.incarnation,
        )
        .await
        {
            Ok(mut stream) => Ok(request(
                &mut stream,
                "fixture.localhost",
                "/MCP",
                "Authorization: Bearer caller-override\r\nX-Wispnet-User: fake-owner\r\n",
            )
            .await),
            Err(_) => Err("device denied".into()),
        };
        connection.close(0u32.into(), b"fixture complete");
        endpoint.close().await;
        response
    }

    async fn shutdown(mut self) {
        self.client.publications.stop().await;
        self.ingress.abort();
        self.gateway.close().await;
        self.private_host.take().unwrap().shutdown().await;
        self.backend.abort();
        self.hub.abort();
    }
}

#[tokio::test]
async fn managed_public_authentication_mediates_credentials_and_keeps_private_grants() {
    let fixture = Fixture::new().await;
    let response = fixture.public("").await;
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");
    assert!(
        response
            .to_ascii_lowercase()
            .contains("www-authenticate: bearer resource_metadata=")
    );
    assert_eq!(fixture.backend_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.authorizations.load(Ordering::SeqCst), 0);
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/MCP",
    ] {
        let metadata = fixture.metadata(path).await;
        assert!(metadata.starts_with("HTTP/1.1 200"), "{metadata}");
        let body: Value = serde_json::from_str(metadata.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["resource"], fixture.resource);
        assert_eq!(
            body["authorization_servers"][0],
            crate::command_center::settings(fixture.state.path(), fixture.client.endpoint.id())
                .unwrap()
                .base_url
        );
        assert_eq!(body["scopes_supported"], json!(["mcp:use"]));
    }
    for extra in [
        "Authorization: Bearer wrong-resource-token\r\n",
        "Authorization: Bearer wrong-token\r\nX-Wispnet-User: owner\r\n",
        "Authorization: Bearer visitor-service-fixture-token\r\nAuthorization: Bearer visitor-service-fixture-token\r\n",
    ] {
        let denied = fixture.public(extra).await;
        assert!(denied.starts_with("HTTP/1.1 401"), "{denied}");
        assert_eq!(fixture.backend_calls.load(Ordering::SeqCst), 0);
    }
    let response = fixture
        .public(&format!(
            "Authorization: Bearer {VISITOR}\r\nX-Wispnet-User: fake-owner\r\n"
        ))
        .await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains(r#""tools":[]"#), "{response}");
    assert_eq!(fixture.backend_calls.load(Ordering::SeqCst), 1);
    fixture.allowed.store(false, Ordering::SeqCst);
    assert!(
        fixture
            .public(&format!("Authorization: Bearer {VISITOR}\r\n"))
            .await
            .starts_with("HTTP/1.1 401")
    );
    assert_eq!(fixture.backend_calls.load(Ordering::SeqCst), 1);
    fixture.hub.abort();
    assert!(fixture.private(&SecretKey::generate()).await.is_err());
    assert_eq!(fixture.backend_calls.load(Ordering::SeqCst), 1);
    let private = fixture.private(&fixture.peer).await.unwrap();
    assert!(private.starts_with("HTTP/1.1 200"), "{private}");
    assert_eq!(fixture.backend_calls.load(Ordering::SeqCst), 2);
    let service = fixture
        .client
        .local_authority
        .as_ref()
        .unwrap()
        .services()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == SERVICE)
        .unwrap();
    assert_eq!(service.allow, vec![fixture.peer.public()]);
    let status = crate::command_center::status(&fixture.client)
        .await
        .unwrap();
    let text = status.to_string();
    assert!(!text.contains(PUBLISHER));
    assert!(!text.contains(BACKEND));
    assert_eq!(
        std::fs::metadata(fixture.state.path().join("command-center.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn managed_public_local_unlink_recovers_after_remote_revocation_or_hub_failure() {
    for remote in ["confirmed", "revoked", "offline"] {
        let fixture = Fixture::new().await;
        if remote == "revoked" {
            fixture.allowed.store(false, Ordering::SeqCst);
        } else if remote == "offline" {
            fixture.hub.abort();
            while !fixture.hub.is_finished() {
                tokio::task::yield_now().await;
            }
        }
        let response = fixture
            .client
            .command(&json!({"op":"command_center_unlink"}))
            .await
            .unwrap();
        assert_eq!(response["linked"], false);
        assert_eq!(
            response["remote_revocation"],
            if remote == "confirmed" {
                "confirmed"
            } else {
                "unconfirmed"
            }
        );
        assert_eq!(response["warning"].is_string(), remote != "confirmed");
        assert!(!response.to_string().contains(PUBLISHER));
        assert_eq!(
            fixture
                .client
                .publications
                .status(&fixture.client)
                .await
                .unwrap()["services"],
            json!([])
        );
        assert!(
            crate::command_center::linked(fixture.state.path(), fixture.client.endpoint.id())
                .is_err()
        );
        let saved =
            std::fs::read_to_string(fixture.state.path().join("command-center.json")).unwrap();
        assert!(!saved.contains(PUBLISHER));
        let private = fixture.private(&fixture.peer).await.unwrap();
        assert!(private.starts_with("HTTP/1.1 200"), "{private}");
        let repeated = fixture
            .client
            .command(&json!({"op":"command_center_unlink"}))
            .await
            .unwrap();
        assert_eq!(repeated["linked"], false);
        assert_eq!(repeated["remote_revocation"], "not_requested");
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn managed_public_revoke_cancels_an_active_sse_stream() {
    let fixture = Fixture::new().await;
    let mut stream = TcpStream::connect(("127.0.0.1", fixture.port))
        .await
        .unwrap();
    stream
        .write_all(
            format!(
                "GET /hold HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {VISITOR}\r\n\r\n",
                fixture.name
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let head = String::from_utf8(headers(&mut stream).await).unwrap();
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    let mut initial = [0; 16];
    tokio::time::timeout(Duration::from_secs(1), stream.read_exact(&mut initial))
        .await
        .expect("SSE was buffered")
        .unwrap();
    assert!(String::from_utf8_lossy(&initial).contains("data: first"));
    fixture.allowed.store(false, Ordering::SeqCst);
    let mut rest = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut rest))
        .await
        .expect("revoked stream survived its authorization bound");
    assert_eq!(fixture.backend_calls.load(Ordering::SeqCst), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn managed_public_strict_gateway_rejects_legacy_and_version_downgrade() {
    let fixture = Fixture::new().await;
    for alpn in [ALPN, AUTH_ALPN] {
        if let Ok(connection) = fixture
            .client
            .endpoint
            .connect(
                iroh::EndpointAddr::new(fixture.gateway.id()).with_ip_addr(fixture.gateway_address),
                alpn,
            )
            .await
        {
            if alpn == AUTH_ALPN {
                let (mut send, _recv) = connection.open_bi().await.unwrap();
                write_json(
                    &mut send,
                    &Registration {
                        version: 1,
                        service: "legacy".into(),
                        incarnation: fixture.incarnation.clone(),
                        name: NAME.into(),
                    },
                )
                .await
                .unwrap();
                send.finish().unwrap();
            }
            tokio::time::timeout(Duration::from_secs(2), connection.closed())
                .await
                .expect("legacy/downgraded registration stayed connected");
        }
    }
    assert!(fixture.public("").await.starts_with("HTTP/1.1 401"));
    assert_eq!(fixture.backend_calls.load(Ordering::SeqCst), 0);
    fixture.shutdown().await;
}

fn decoded_http_body(response: &str) -> Vec<u8> {
    let (head, body) = response
        .split_once("\r\n\r\n")
        .expect("HTTP response headers");
    if !head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return body.as_bytes().to_vec();
    }
    let mut input = body.as_bytes();
    let mut output = Vec::new();
    loop {
        let line = input
            .windows(2)
            .position(|w| w == b"\r\n")
            .expect("chunk length");
        let length = std::str::from_utf8(&input[..line])
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let length = usize::from_str_radix(length, 16).expect("hex chunk length");
        if length == 0 {
            return output;
        }
        input = &input[line + 2..];
        assert!(input.len() >= length + 2, "complete chunk");
        output.extend_from_slice(&input[..length]);
        assert_eq!(&input[length..length + 2], b"\r\n");
        input = &input[length + 2..];
    }
}

/// Manual opt-in only: exercises the real local GIKE through an isolated
/// authenticated public gateway without changing GIKE source, policy or data.
#[tokio::test]
#[ignore = "requires an explicitly selected running local GIKE MCP"]
async fn managed_public_live_gike_initialize_without_backend_changes() {
    let address: SocketAddr = std::env::var("WISPNET_COMMAND_GIKE_ADDR")
        .expect("set WISPNET_COMMAND_GIKE_ADDR to the running GIKE loopback address")
        .parse()
        .expect("GIKE address must be an IP and port");
    assert!(
        address.ip().is_loopback() && address.port() != 0,
        "live fixture is loopback-only"
    );
    let path = std::env::var("WISPNET_COMMAND_GIKE_PATH").unwrap_or_else(|_| "/mcp".into());
    assert!(valid_mcp_path(&path), "valid GIKE MCP endpoint path");
    let fixture = Fixture::with_backend(Some(address), &path).await;
    let initialize = json!({
        "jsonrpc":"2.0", "id":1, "method":"initialize",
        "params": {
            "protocolVersion":"2025-06-18", "capabilities":{},
            "clientInfo":{"name":"wispnet-managed-public-live-fixture","version":"0.1"}
        }
    })
    .to_string();
    let call = |bearer: bool| {
        let initialize = initialize.clone();
        let path = path.clone();
        let name = fixture.name.clone();
        let port = fixture.port;
        async move {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let auth = if bearer {
                format!("Authorization: Bearer {VISITOR}\r\n")
            } else {
                String::new()
            };
            stream.write_all(format!("POST {path} HTTP/1.1\r\nHost: {name}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{auth}\r\n{initialize}", initialize.len()).as_bytes()).await.unwrap();
            let mut bytes = Vec::new();
            tokio::time::timeout(Duration::from_secs(15), stream.read_to_end(&mut bytes))
                .await
                .expect("live GIKE initialize timed out")
                .unwrap();
            String::from_utf8(bytes).unwrap()
        }
    };
    let denied = call(false).await;
    assert!(
        denied.starts_with("HTTP/1.1 401"),
        "unauthenticated GIKE public request denied"
    );
    let response = call(true).await;
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "authenticated GIKE initialize returned an unexpected status"
    );
    let body = decoded_http_body(&response);
    let text = std::str::from_utf8(&body).unwrap();
    let message: Value = serde_json::from_slice(&body).unwrap_or_else(|_| {
        text.lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim))
            .find_map(|line| {
                serde_json::from_str::<Value>(line)
                    .ok()
                    .filter(|v| v["id"] == 1)
            })
            .expect("GIKE initialize JSON or SSE result")
    });
    let name = message["result"]["serverInfo"]["name"]
        .as_str()
        .expect("GIKE serverInfo name");
    assert!(
        name.to_ascii_lowercase().contains("gike"),
        "initialized the selected GIKE server"
    );
    assert_eq!(
        fixture.backend_calls.load(Ordering::SeqCst),
        0,
        "real GIKE used, not the fixture backend"
    );
    assert!(fixture.authorizations.load(Ordering::SeqCst) > 0);
    fixture.shutdown().await;
}
