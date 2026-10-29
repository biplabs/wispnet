//! Host-side HTTP credential mediation, shared by local, private and public access.
//!
//! The caller establishes device/owner authority before calling `connect`.
//! Public callers additionally supply an authorization implementation. Backend
//! credentials are loaded only here, on the hosting machine.
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited, combinators::UnsyncBoxBody};
use hyper::{
    Request, Response,
    body::Incoming,
    header::{HeaderName, HeaderValue},
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{
    convert::Infallible,
    future::Future,
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    task::JoinHandle,
};
use wispnet_core::{host::ServiceKind, identity::EndpointId};
use wispnet_registry::backend_credentials::{self, Credential, Mode};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Body = UnsyncBoxBody<Bytes, Error>;
const UPLOAD_LIMIT: usize = 16 * 1024 * 1024;
const AUTH_INTERVAL: Duration = Duration::from_secs(2);
const AUTH_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationError {
    /// The visitor's credentials were explicitly rejected.
    Denied,
    /// Authorization could not be checked; retry without replacing credentials.
    Unavailable,
}

/// Implementations verify the original service token and current public grant.
/// They must never trust caller-provided identity headers. The same check runs
/// before each HTTP request and periodically while a stream remains open.
pub trait RequestAuthorization: Send + Sync {
    fn authorize(
        &self,
        headers: &hyper::HeaderMap,
    ) -> Pin<Box<dyn Future<Output = Result<(), AuthorizationError>> + Send + '_>>;
}

pub trait BackendIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> BackendIo for T {}

/// Dropping the front connection aborts and drains its hosting proxy work.
pub struct BackendStream {
    io: Box<dyn BackendIo>,
    worker: Option<JoinHandle<()>>,
}
impl BackendStream {
    pub fn direct(stream: TcpStream) -> Self {
        Self {
            io: Box::new(stream),
            worker: None,
        }
    }
}
impl Drop for BackendStream {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.abort();
        }
    }
}
impl AsyncRead for BackendStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buffer)
    }
}
impl AsyncWrite for BackendStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buffer)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

pub async fn connect(
    state: &Path,
    host: EndpointId,
    service: &str,
    incarnation: &str,
    backend: SocketAddr,
) -> Result<BackendStream, String> {
    connect_with(state, host, service, incarnation, backend, None).await
}
pub async fn connect_authorized(
    state: &Path,
    host: EndpointId,
    service: &str,
    incarnation: &str,
    backend: SocketAddr,
    authorization: Arc<dyn RequestAuthorization>,
) -> Result<BackendStream, String> {
    connect_with(
        state,
        host,
        service,
        incarnation,
        backend,
        Some(authorization),
    )
    .await
}

async fn connect_with(
    state: &Path,
    host: EndpointId,
    service: &str,
    incarnation: &str,
    backend: SocketAddr,
    authorization: Option<Arc<dyn RequestAuthorization>>,
) -> Result<BackendStream, String> {
    let credential = backend_credentials::load(state, host, service, incarnation, backend)?;
    if credential.mode == Mode::PassThrough && authorization.is_none() {
        let stream = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(backend))
            .await
            .map_err(|_| "Backend connection timed out")?
            .map_err(|_| "Backend unavailable")?;
        return Ok(BackendStream::direct(stream));
    }
    let (front, incoming) = tokio::io::duplex(32 * 1024);
    let context = Arc::new(Proxy {
        state: state.into(),
        host,
        service: service.into(),
        incarnation: incarnation.into(),
        backend,
        credential,
        authorization,
        current_headers: Mutex::new(None),
        tasks: Tasks::default(),
    });
    let worker = tokio::spawn(async move {
        let serve = async {
            let state = context.clone();
            let _ = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(10))
                .max_buf_size(32768)
                .max_headers(96)
                .half_close(true)
                .serve_connection(
                    TokioIo::new(incoming),
                    service_fn(move |request| state.clone().request(request)),
                )
                .with_upgrades()
                .await;
            context.tasks.finish().await;
        };
        tokio::select! {
            _ = tokio::time::timeout(Duration::from_secs(3600), serve) => {},
            _ = context.guard() => {},
        }
        context.tasks.abort();
    });
    Ok(BackendStream {
        io: Box::new(front),
        worker: Some(worker),
    })
}

struct Task(JoinHandle<()>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}
#[derive(Default)]
struct Tasks(Mutex<Vec<Task>>);
impl Tasks {
    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        let mut tasks = self.0.lock().unwrap_or_else(|e| e.into_inner());
        tasks.retain(|task| !task.0.is_finished());
        tasks.push(Task(tokio::spawn(future)));
    }
    fn abort(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    async fn finish(&self) {
        let tasks = std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()));
        for mut task in tasks {
            let _ = (&mut task.0).await;
        }
    }
}

struct Proxy {
    state: PathBuf,
    host: EndpointId,
    service: String,
    incarnation: String,
    backend: SocketAddr,
    credential: Credential,
    authorization: Option<Arc<dyn RequestAuthorization>>,
    current_headers: Mutex<Option<hyper::HeaderMap>>,
    tasks: Tasks,
}
impl Proxy {
    async fn binding_current(&self) -> bool {
        let Ok(authority) = wispnet_registry::host::Authority::open(&self.state, self.host) else {
            return false;
        };
        authority.services().await.is_ok_and(|services| {
            services.iter().any(|service| {
                service.id == self.service
                    && service.incarnation == self.incarnation
                    && service.backend == self.backend
                    && service.kind == ServiceKind::Http
            })
        })
    }
    fn credential_current(&self) -> bool {
        backend_credentials::load(
            &self.state,
            self.host,
            &self.service,
            &self.incarnation,
            self.backend,
        )
        .is_ok_and(|credential| credential.generation == self.credential.generation)
    }
    async fn allowed(&self, headers: &hyper::HeaderMap) -> Result<(), AuthorizationError> {
        let Some(authorization) = &self.authorization else {
            return Ok(());
        };
        tokio::time::timeout(AUTH_TIMEOUT, authorization.authorize(headers))
            .await
            .unwrap_or(Err(AuthorizationError::Unavailable))
    }
    async fn guard(&self) {
        let mut interval = tokio::time::interval(AUTH_INTERVAL);
        loop {
            interval.tick().await;
            if !self.credential_current() || !self.binding_current().await {
                return;
            }
            let headers = self
                .current_headers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if let Some(headers) = headers
                && self.allowed(&headers).await.is_err()
            {
                return;
            }
        }
    }
    async fn request(
        self: Arc<Self>,
        mut request: Request<Incoming>,
    ) -> Result<Response<Body>, Infallible> {
        if request.uri().scheme().is_some()
            || request.uri().authority().is_some()
            || request.headers().get_all("host").iter().count() != 1
            || request.headers().get_all("authorization").iter().count() > 1
        {
            return Ok(error(400, "Invalid service request"));
        }
        if !matches!(
            *request.method(),
            hyper::Method::GET
                | hyper::Method::HEAD
                | hyper::Method::POST
                | hyper::Method::PUT
                | hyper::Method::PATCH
                | hyper::Method::DELETE
                | hyper::Method::OPTIONS
        ) {
            return Ok(error(405, "Method not allowed"));
        }
        if !self.credential_current() || !self.binding_current().await {
            return Ok(error(403, "Service binding unavailable"));
        }
        match self.allowed(request.headers()).await {
            Ok(()) => {}
            Err(AuthorizationError::Denied) => {
                return Ok(error(401, "Service authorization required"));
            }
            Err(AuthorizationError::Unavailable) => {
                return Ok(error(503, "Service authorization unavailable"));
            }
        }
        *self
            .current_headers
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(request.headers().clone());
        if request
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|length| length > UPLOAD_LIMIT as u64)
        {
            return Ok(error(413, "Request too large"));
        }
        let upgrade = request
            .headers()
            .get("upgrade")
            .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"));
        let downstream = upgrade.then(|| hyper::upgrade::on(&mut request));
        sanitize(request.headers_mut(), upgrade);
        // This connection terminates at the registered loopback backend. In
        // particular, FastMCP validates Host and must not receive the public or
        // per-device .localhost alias as if it were its own listening address.
        request.headers_mut().insert(
            hyper::header::HOST,
            HeaderValue::from_str(&self.backend.to_string()).expect("validated backend address"),
        );
        // Public OAuth tokens terminate here. Private managed access similarly
        // never lets a caller override the host-configured backend identity.
        if self.authorization.is_some() || self.credential.mode != Mode::PassThrough {
            request.headers_mut().remove("authorization");
        }
        if let Some(secret) = self.credential.bearer() {
            let mut value = HeaderValue::from_str(&format!("Bearer {secret}"))
                .expect("validated backend credential");
            value.set_sensitive(true);
            request.headers_mut().insert("authorization", value);
        }
        let stream =
            match tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(self.backend))
                .await
            {
                Ok(Ok(stream)) => stream,
                _ => return Ok(error(502, "Backend unavailable")),
            };
        let (mut sender, connection) =
            match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await {
                Ok(connection) => connection,
                Err(_) => return Ok(error(502, "Backend connection failed")),
            };
        self.tasks.spawn(async move {
            let _ = connection.with_upgrades().await;
        });
        let (parts, body) = request.into_parts();
        let request = Request::from_parts(parts, Limited::new(body, UPLOAD_LIMIT));
        let mut response = match tokio::time::timeout(
            Duration::from_secs(120),
            sender.send_request(request),
        )
        .await
        {
            Ok(Ok(response)) => response,
            _ => return Ok(error(502, "Backend response unavailable")),
        };
        if response.status() == hyper::StatusCode::SWITCHING_PROTOCOLS {
            if !upgrade {
                return Ok(error(502, "Unexpected backend upgrade"));
            }
            let upstream = hyper::upgrade::on(&mut response);
            let downstream = downstream.expect("upgrade requested");
            self.tasks.spawn(async move {
                if let (Ok(downstream), Ok(upstream)) = tokio::join!(downstream, upstream) {
                    let mut downstream = TokioIo::new(downstream);
                    let mut upstream = TokioIo::new(upstream);
                    let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
                }
            });
        }
        let upgraded = upgrade && response.status() == hyper::StatusCode::SWITCHING_PROTOCOLS;
        sanitize(response.headers_mut(), upgraded);
        Ok(response.map(|body| {
            body.map_err(|error| -> Error { Box::new(error) })
                .boxed_unsync()
        }))
    }
}

fn sanitize(headers: &mut hyper::HeaderMap, upgrade: bool) {
    let nominated: Vec<_> = headers
        .get_all("connection")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in nominated {
        if !(upgrade && name == hyper::header::UPGRADE) {
            headers.remove(name);
        }
    }
    let untrusted: Vec<_> = headers
        .keys()
        .filter(|name| {
            name.as_str().starts_with("x-forwarded-") || name.as_str().starts_with("x-wispnet-")
        })
        .cloned()
        .collect();
    for name in untrusted {
        headers.remove(name);
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "forwarded",
    ] {
        headers.remove(name);
    }
    if upgrade {
        headers.insert("connection", HeaderValue::from_static("Upgrade"));
    } else {
        headers.remove("upgrade");
    }
}
fn error(status: u16, message: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .header("cache-control", "no-store")
        .body(
            Full::new(Bytes::from_static(message.as_bytes()))
                .map_err(|error| -> Error { match error {} })
                .boxed_unsync(),
        )
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicBool, Ordering},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    use wispnet_registry::host::{self, Mutation};

    struct Fixture {
        state: tempfile::TempDir,
        key: wispnet_core::identity::SecretKey,
        host: EndpointId,
        incarnation: String,
        backend: SocketAddr,
    }
    impl Fixture {
        fn new(backend: SocketAddr, mode: Mode) -> Self {
            let state = tempfile::tempdir().unwrap();
            std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            let key = wispnet_core::identity::SecretKey::generate();
            let host = key.public();
            host::initialize(state.path(), host).unwrap();
            let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
            let revision = store.host_snapshot(host).unwrap().revision;
            store
                .host_mutate(
                    host,
                    &owner,
                    Mutation::Service {
                        expected_revision: revision,
                        id: "mcp".into(),
                        backend,
                        allow: Vec::new(),
                        display_name: None,
                        kind: Some(ServiceKind::Http),
                    },
                )
                .unwrap();
            let incarnation = store
                .host_snapshot(host)
                .unwrap()
                .services
                .into_iter()
                .find(|service| service.id == "mcp")
                .unwrap()
                .incarnation;
            backend_credentials::set(
                state.path(),
                host,
                "mcp",
                &incarnation,
                backend,
                mode,
                (mode == Mode::Bearer).then_some("backend-fixture-key"),
            )
            .unwrap();
            Self {
                state,
                key,
                host,
                incarnation,
                backend,
            }
        }
        async fn connect(&self) -> BackendStream {
            connect(
                self.state.path(),
                self.host,
                "mcp",
                &self.incarnation,
                self.backend,
            )
            .await
            .unwrap()
        }
    }
    async fn read_until(stream: &mut (impl AsyncRead + Unpin), end: &[u8]) -> Vec<u8> {
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut bytes = Vec::new();
            let mut byte = [0];
            while !bytes.ends_with(end) {
                if stream.read(&mut byte).await.unwrap() == 0 {
                    break;
                }
                bytes.push(byte[0]);
                assert!(bytes.len() <= 32768);
            }
            bytes
        })
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn managed_backend_key_replaces_caller_credentials_and_identity_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fixture = Fixture::new(listener.local_addr().unwrap(), Mode::Bearer);
        let backend_address = fixture.backend.to_string();
        let backend = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let headers = String::from_utf8(read_until(&mut stream, b"\r\n\r\n").await).unwrap();
            assert!(headers.contains("authorization: Bearer backend-fixture-key\r\n"));
            assert!(headers.contains(&format!("host: {backend_address}\r\n")));
            assert!(!headers.contains("Host: mcp.localhost"));
            assert!(!headers.contains("caller-key"));
            assert!(!headers.to_ascii_lowercase().contains("x-wispnet-user"));
            assert!(!headers.to_ascii_lowercase().contains("x-forwarded-user"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await
                .unwrap();
        });
        let mut stream = fixture.connect().await;
        stream.write_all(b"POST /mcp HTTP/1.1\r\nHost: mcp.localhost\r\nAuthorization: Bearer caller-key\r\nX-Wispnet-User: owner\r\nX-Forwarded-User: owner\r\nContent-Length: 0\r\n\r\n").await.unwrap();
        stream.shutdown().await.unwrap();
        let response = read_until(&mut stream, b"\r\n\r\n").await;
        assert!(response.starts_with(b"HTTP/1.1 200"));
        let mut body = [0; 2];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(&body, b"ok");
        backend.await.unwrap();
    }
    #[tokio::test]
    async fn unmanaged_http_preserves_original_host_and_service_authorization() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fixture = Fixture::new(listener.local_addr().unwrap(), Mode::PassThrough);
        let backend = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let headers = String::from_utf8(read_until(&mut stream, b"\r\n\r\n").await).unwrap();
            assert!(headers.contains("Host: original.localhost\r\n"));
            assert!(headers.contains("Authorization: Bearer caller-service-key\r\n"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await
                .unwrap();
        });
        let mut stream = fixture.connect().await;
        stream.write_all(b"GET /mcp HTTP/1.1\r\nHost: original.localhost\r\nAuthorization: Bearer caller-service-key\r\n\r\n").await.unwrap();
        assert!(
            read_until(&mut stream, b"\r\n\r\n")
                .await
                .starts_with(b"HTTP/1.1 200")
        );
        backend.await.unwrap();
    }
    struct PublicAuthorization(AtomicBool);
    impl RequestAuthorization for PublicAuthorization {
        fn authorize(
            &self,
            headers: &hyper::HeaderMap,
        ) -> Pin<Box<dyn Future<Output = Result<(), AuthorizationError>> + Send + '_>> {
            let valid = self.0.load(Ordering::SeqCst)
                && headers
                    .get("authorization")
                    .is_some_and(|header| header == "Bearer public-fixture-token");
            Box::pin(async move {
                if valid {
                    Ok(())
                } else {
                    Err(AuthorizationError::Denied)
                }
            })
        }
    }
    #[tokio::test]
    async fn public_denial_never_contacts_backend() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fixture = Fixture::new(listener.local_addr().unwrap(), Mode::Bearer);
        let authorization = Arc::new(PublicAuthorization(AtomicBool::new(true)));
        let mut stream = connect_authorized(
            fixture.state.path(),
            fixture.host,
            "mcp",
            &fixture.incarnation,
            fixture.backend,
            authorization,
        )
        .await
        .unwrap();
        stream
            .write_all(b"GET /mcp HTTP/1.1\r\nHost: mcp.localhost\r\n\r\n")
            .await
            .unwrap();
        assert!(
            read_until(&mut stream, b"\r\n\r\n")
                .await
                .starts_with(b"HTTP/1.1 401")
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    }

    struct UnavailableAuthorization {
        unavailable: AtomicBool,
        timeout: bool,
    }
    impl RequestAuthorization for UnavailableAuthorization {
        fn authorize(
            &self,
            headers: &hyper::HeaderMap,
        ) -> Pin<Box<dyn Future<Output = Result<(), AuthorizationError>> + Send + '_>> {
            let unavailable = self.unavailable.load(Ordering::SeqCst);
            let valid = headers
                .get("authorization")
                .is_some_and(|header| header == "Bearer public-fixture-token");
            Box::pin(async move {
                if unavailable {
                    if self.timeout {
                        std::future::pending::<()>().await;
                    }
                    Err(AuthorizationError::Unavailable)
                } else if valid {
                    Ok(())
                } else {
                    Err(AuthorizationError::Denied)
                }
            })
        }
    }

    #[tokio::test]
    async fn public_authorization_outage_and_timeout_recover_without_new_credentials() {
        for timeout in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let fixture = Fixture::new(listener.local_addr().unwrap(), Mode::Bearer);
            let authorization = Arc::new(UnavailableAuthorization {
                unavailable: AtomicBool::new(true),
                timeout,
            });
            let request = b"GET /mcp HTTP/1.1\r\nHost: mcp.localhost\r\nAuthorization: Bearer public-fixture-token\r\nConnection: close\r\n\r\n";
            let mut stream = connect_authorized(
                fixture.state.path(),
                fixture.host,
                "mcp",
                &fixture.incarnation,
                fixture.backend,
                authorization.clone(),
            )
            .await
            .unwrap();
            stream.write_all(request).await.unwrap();
            let mut response = Vec::new();
            tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert!(response.starts_with(b"HTTP/1.1 503"));
            assert!(
                !String::from_utf8(response)
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("www-authenticate")
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err()
            );

            authorization.unavailable.store(false, Ordering::SeqCst);
            let backend = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let headers =
                    String::from_utf8(read_until(&mut stream, b"\r\n\r\n").await).unwrap();
                assert!(headers.contains("authorization: Bearer backend-fixture-key"));
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await
                    .unwrap();
            });
            let mut recovered = connect_authorized(
                fixture.state.path(),
                fixture.host,
                "mcp",
                &fixture.incarnation,
                fixture.backend,
                authorization,
            )
            .await
            .unwrap();
            recovered.write_all(request).await.unwrap();
            let mut response = Vec::new();
            tokio::time::timeout(Duration::from_secs(5), recovered.read_to_end(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert!(response.starts_with(b"HTTP/1.1 200"));
            assert!(response.ends_with(b"ok"));
            backend.await.unwrap();
        }
    }

    #[tokio::test]
    async fn sse_is_streamed_and_public_revocation_closes_active_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fixture = Fixture::new(listener.local_addr().unwrap(), Mode::Bearer);
        let backend = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let headers = String::from_utf8(read_until(&mut stream, b"\r\n\r\n").await).unwrap();
            assert!(headers.contains("authorization: Bearer backend-fixture-key"));
            assert!(!headers.contains("public-fixture-token"));
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\nb\r\ndata: one\n\n\r\n").await.unwrap();
            // The complete response never arrives; the first event must still reach the client.
            let mut byte = [0];
            let _ = stream.read(&mut byte).await;
        });
        let authorization = Arc::new(PublicAuthorization(AtomicBool::new(true)));
        let mut stream = connect_authorized(
            fixture.state.path(),
            fixture.host,
            "mcp",
            &fixture.incarnation,
            fixture.backend,
            authorization.clone(),
        )
        .await
        .unwrap();
        stream.write_all(b"GET /mcp HTTP/1.1\r\nHost: mcp.localhost\r\nAuthorization: Bearer public-fixture-token\r\n\r\n").await.unwrap();
        let first = read_until(&mut stream, b"data: one\n\n").await;
        assert!(first.ends_with(b"data: one\n\n"));
        authorization.0.store(false, Ordering::SeqCst);
        let mut remaining = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut remaining))
            .await
            .unwrap()
            .unwrap();
        backend.await.unwrap();
    }
    #[tokio::test]
    async fn websocket_upgrade_streams_and_credential_rotation_closes_it() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fixture = Fixture::new(listener.local_addr().unwrap(), Mode::Bearer);
        let backend = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let headers = String::from_utf8(read_until(&mut stream, b"\r\n\r\n").await).unwrap();
            assert!(headers.contains("authorization: Bearer backend-fixture-key"));
            stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n").await.unwrap();
            let mut frame = [0; 5];
            stream.read_exact(&mut frame).await.unwrap();
            stream.write_all(&frame).await.unwrap();
            let mut rest = Vec::new();
            let _ = stream.read_to_end(&mut rest).await;
        });
        let mut stream = fixture.connect().await;
        stream.write_all(b"GET /mcp HTTP/1.1\r\nHost: mcp.localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").await.unwrap();
        assert!(
            read_until(&mut stream, b"\r\n\r\n")
                .await
                .starts_with(b"HTTP/1.1 101")
        );
        stream.write_all(b"frame").await.unwrap();
        let mut echoed = [0; 5];
        tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut echoed))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&echoed, b"frame");
        backend_credentials::set(
            fixture.state.path(),
            fixture.host,
            "mcp",
            &fixture.incarnation,
            fixture.backend,
            Mode::Bearer,
            Some("rotated-fixture-key"),
        )
        .unwrap();
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut rest))
            .await
            .unwrap()
            .unwrap();
        backend.await.unwrap();
    }

    #[tokio::test]
    async fn private_device_grant_mediates_key_without_public_login_and_revokes_independently() {
        use std::sync::atomic::AtomicUsize;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fixture = Fixture::new(listener.local_addr().unwrap(), Mode::Bearer);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let backend = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                seen.fetch_add(1, Ordering::SeqCst);
                let headers =
                    String::from_utf8(read_until(&mut stream, b"\r\n\r\n").await).unwrap();
                assert!(headers.contains("authorization: Bearer backend-fixture-key"));
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await
                    .unwrap();
            }
        });
        let peer = wispnet_core::identity::SecretKey::generate();
        let (mut store, owner) = host::owner_store(fixture.state.path(), fixture.host).unwrap();
        let revision = store.host_snapshot(fixture.host).unwrap().revision;
        store
            .host_mutate(
                fixture.host,
                &owner,
                Mutation::Approve {
                    expected_revision: revision,
                    peer: peer.public(),
                    name: "allowed-device".into(),
                },
            )
            .unwrap();
        store
            .host_mutate(
                fixture.host,
                &owner,
                Mutation::GrantService {
                    expected_revision: revision + 1,
                    service: "mcp".into(),
                    peer: peer.public(),
                },
            )
            .unwrap();
        let transport = wispnet_core::config::TransportConfig {
            backup_relay_urls: Vec::new(),
            discovery_relay_url: None,
            relay_url: None,
            relay_only: false,
            relay_ca_file: None,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            max_connections: 8,
            max_streams: 16,
            max_streams_per_connection: 8,
        };
        let authority = host::Authority::open(fixture.state.path(), fixture.host).unwrap();
        let hosting = crate::tunnel::TunnelHost::bind_host(&fixture.key, &transport, authority)
            .await
            .unwrap();
        let endpoint = crate::tunnel::bind_client(&peer).await.unwrap();
        let connection = crate::tunnel::connect(
            &endpoint,
            fixture.host,
            hosting.endpoint().bound_sockets()[0],
        )
        .await
        .unwrap();
        let mut stream =
            crate::tunnel::open_service_bound(&connection, "mcp", &fixture.incarnation)
                .await
                .unwrap();
        stream
            .write_all(b"GET /mcp HTTP/1.1\r\nHost: private.localhost\r\n\r\n")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
        let response = read_until(&mut stream, b"\r\n\r\n").await;
        assert!(response.starts_with(b"HTTP/1.1 200"));
        let mut body = [0; 2];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(&body, b"ok");
        store
            .host_mutate(
                fixture.host,
                &owner,
                Mutation::RevokeService {
                    expected_revision: revision + 2,
                    service: "mcp".into(),
                    peer: peer.public(),
                },
            )
            .unwrap();
        assert!(
            crate::tunnel::open_service_bound(&connection, "mcp", &fixture.incarnation)
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(stream);
        endpoint.close().await;
        hosting.shutdown().await;
        backend.abort();
        let _ = backend.await;
    }
}
