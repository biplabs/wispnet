//! HTTP-aware loopback gateway. Composite peer/service routes, never arbitrary upstreams.
use crate::{Client, token};
use iroh::{EndpointId, endpoint::Connection};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Semaphore},
    task::{JoinHandle, JoinSet},
};
#[path = "gateway_http.rs"]
mod http;
pub const SESSION_SECONDS: u64 = 600;
struct Limits {
    session: Duration,
    bootstrap: Duration,
}
const COOKIE: &str = "_wipsnet_session";
const START: &str = r#"<!doctype html><meta name="referrer" content="no-referrer"><title>Wispnet</title><p id="status">Opening service…</p><script>const token=location.hash.slice(1);history.replaceState(null,'',location.pathname);fetch('/_wipsnet/session',{method:'POST',headers:{Authorization:'Bearer '+token}}).then(r=>{if(!r.ok)throw Error();location.replace('/')}).catch(()=>document.getElementById('status').textContent='Access expired. Open this service again in Wispnet.');</script>"#;
pub struct Gateways {
    enabled: AtomicBool,
    workers: Mutex<Vec<JoinHandle<()>>>,
    connections: Arc<Semaphore>,
}
impl Default for Gateways {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            workers: Mutex::new(Vec::new()),
            connections: Arc::new(Semaphore::new(32)),
        }
    }
}
impl Gateways {
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }
    pub async fn stop(&self) {
        self.enabled.store(false, Ordering::Release);
        self.interrupt().await;
    }
    /// Close existing browser sessions without changing the user's access setting.
    /// Network changes never replay an in-flight HTTP request.
    pub async fn interrupt(&self) {
        let mut guard = self.workers.lock().await;
        let workers = std::mem::take(&mut *guard);
        for worker in &workers {
            worker.abort();
        }
        for worker in workers {
            let _ = worker.await;
        }
    }
    pub async fn open(
        &self,
        client: Arc<Client>,
        peer: EndpointId,
        service: String,
        incarnation: String,
    ) -> Result<String, String> {
        self.open_for(
            client,
            peer,
            service,
            incarnation,
            Limits {
                session: Duration::from_secs(SESSION_SECONDS),
                bootstrap: Duration::from_secs(60),
            },
            None,
        )
        .await
    }
    async fn open_for(
        &self,
        client: Arc<Client>,
        peer: EndpointId,
        service: String,
        incarnation: String,
        limits: Limits,
        listener: Option<TcpListener>,
    ) -> Result<String, String> {
        if !self.enabled.load(Ordering::Acquire) {
            return Err("Start browser access first".into());
        }
        let local_binding = if peer == client.endpoint.id() {
            if let Some(authority) = &client.local_authority {
                Some(
                    authority
                        .services()
                        .await
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .find(|s| {
                            s.id == service
                                && s.incarnation == incarnation
                                && s.kind == wipsnet_core::host::ServiceKind::Http
                        })
                        .ok_or("Service no longer permitted")?
                        .backend,
                )
            } else {
                None
            }
        } else {
            None
        };
        let listener = match listener {
            Some(listener) => listener,
            None => TcpListener::bind("127.0.0.1:0")
                .await
                .map_err(|_| "Cannot start loopback gateway")?,
        };
        if !listener
            .local_addr()
            .map_err(|_| "Cannot inspect gateway")?
            .ip()
            .is_loopback()
        {
            return Err("Gateway must be loopback-only".into());
        }
        let authority = format!(
            "w{}.localhost:{}",
            &token()[..32],
            listener
                .local_addr()
                .map_err(|_| "Cannot inspect loopback gateway")?
                .port()
        );
        let bootstrap = token();
        let url = format!("http://{authority}/_wipsnet/start#{bootstrap}");
        let route = Arc::new(Route {
            client,
            peer,
            service,
            incarnation,
            local_binding,
            authority,
            bootstrap: Mutex::new(Some(bootstrap)),
            credential: token(),
            bootstrap_expires: Instant::now() + limits.bootstrap,
            expires: Instant::now() + limits.session,
        });
        let mut workers = self.workers.lock().await;
        workers.retain(|w| !w.is_finished());
        if !self.enabled.load(Ordering::Acquire) || workers.len() >= 16 {
            return Err("Access stopped or too many open services".into());
        }
        let connections = self.connections.clone();
        workers.push(tokio::spawn(async move {
   let mut tasks=JoinSet::new();
   loop {tokio::select!{
    _=tokio::time::sleep_until(tokio::time::Instant::from_std(route.expires))=>break,
    Some(_)=tasks.join_next(),if !tasks.is_empty()=>{},
    accepted=listener.accept()=>{
     let Ok((socket,address))=accepted else{break;};if !address.ip().is_loopback(){continue;}
     let Ok(permit)=connections.clone().try_acquire_owned() else{continue;};let route=route.clone();
     tasks.spawn(async move {let _permit=permit;let _=tokio::time::timeout_at(tokio::time::Instant::from_std(route.expires),route.handle(socket)).await;});
    }
   }}
  }));
        Ok(url)
    }
}
trait ServiceIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ServiceIo for T {}

struct Route {
    client: Arc<Client>,
    peer: EndpointId,
    service: String,
    incarnation: String,
    local_binding: Option<std::net::SocketAddr>,
    authority: String,
    bootstrap: Mutex<Option<String>>,
    credential: String,
    bootstrap_expires: Instant,
    expires: Instant,
}
struct Close(Connection);
impl Drop for Close {
    fn drop(&mut self) {
        self.0.close(0u32.into(), b"browser request complete");
    }
}
#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}
impl Request {
    fn values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }
    fn one(&self, name: &str) -> Option<&str> {
        let values = self.values(name);
        if values.len() == 1 {
            Some(values[0])
        } else {
            None
        }
    }
    fn safe(&self, authority: &str) -> bool {
        self.values("host") == [authority]
            && self.path.starts_with('/')
            && !self.path.starts_with("//")
            && !self.path.contains(['\r', '\n', '\\'])
            && self.method.bytes().all(|b| b.is_ascii_uppercase())
    }
    fn same_origin(&self, origin: &str) -> bool {
        self.values("origin").iter().all(|v| *v == origin)
            && self
                .one("sec-fetch-site")
                .is_none_or(|s| s == "none" || s == "same-origin")
            && self.values("origin").len() <= 1
            && self.values("sec-fetch-site").len() <= 1
            && self.values("sec-fetch-mode").len() <= 1
            && self.values("sec-fetch-dest").len() <= 1
    }
}
async fn headers(stream: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>, ()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut bytes = Vec::new();
        while bytes.len() < 32768 {
            bytes.push(stream.read_u8().await.map_err(|_| ())?);
            if bytes.ends_with(b"\r\n\r\n") {
                return Ok(bytes);
            }
        }
        Err(())
    })
    .await
    .map_err(|_| ())?
}
fn parse(bytes: &[u8]) -> Result<Request, ()> {
    let mut storage = [httparse::EMPTY_HEADER; 96];
    let mut request = httparse::Request::new(&mut storage);
    if !request.parse(bytes).map_err(|_| ())?.is_complete() || request.version != Some(1) {
        return Err(());
    }
    Ok(Request {
        method: request.method.ok_or(())?.into(),
        path: request.path.ok_or(())?.into(),
        headers: request
            .headers
            .iter()
            .map(|h| {
                Ok((
                    h.name.to_string(),
                    std::str::from_utf8(h.value).map_err(|_| ())?.to_string(),
                ))
            })
            .collect::<Result<_, ()>>()?,
    })
}
fn constant_equal(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |d, (a, b)| d | (a ^ b)) == 0
}
async fn response(socket: &mut TcpStream, status: &str, body: &str, extra: &str) -> Result<(), ()> {
    let value = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\n{extra}\r\n{body}",
        body.len()
    );
    socket.write_all(value.as_bytes()).await.map_err(|_| ())
}
impl Route {
    async fn handle(&self, mut socket: TcpStream) -> Result<(), ()> {
        let request = match headers(&mut socket).await.and_then(|bytes| parse(&bytes)) {
            Ok(request) => request,
            Err(()) => {
                return response(&mut socket, "400 Bad Request", "Invalid HTTP headers", "").await;
            }
        };
        let origin = format!("http://{}", self.authority);
        if !request.safe(&self.authority) {
            return response(
                &mut socket,
                "421 Misdirected Request",
                "Unknown local authority",
                "",
            )
            .await;
        }
        let (websocket, length) = match http::framing(&request) {
            Ok(result) => result,
            Err(()) => {
                return response(
                    &mut socket,
                    "400 Bad Request",
                    "Unsupported request framing",
                    "",
                )
                .await;
            }
        };
        if length > 16 * 1024 * 1024 {
            return response(&mut socket, "413 Content Too Large", "", "").await;
        }
        if request.path == "/_wipsnet/start" && request.method == "GET" {
            let navigation = request.one("sec-fetch-mode") == Some("navigate")
                && request.one("sec-fetch-dest") == Some("document");
            if !request.same_origin(&origin) && !navigation {
                return response(&mut socket, "403 Forbidden", "Embedding denied", "").await;
            }
            return response(&mut socket,"200 OK",START,"Content-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'\r\n").await;
        }
        if !request.same_origin(&origin) {
            return response(
                &mut socket,
                "403 Forbidden",
                "Cross-origin request denied",
                "",
            )
            .await;
        }
        if request.path == "/_wipsnet/session" {
            let mut bootstrap = self.bootstrap.lock().await;
            if Instant::now() >= self.bootstrap_expires
                || request.method != "POST"
                || length != 0
                || websocket
                || request.one("origin") != Some(origin.as_str())
                || !bootstrap.as_ref().is_some_and(|t| {
                    request
                        .one("authorization")
                        .is_some_and(|a| constant_equal(a, &format!("Bearer {t}")))
                })
            {
                return response(&mut socket, "403 Forbidden", "Bootstrap denied", "").await;
            }
            *bootstrap = None;
            return response(
                &mut socket,
                "204 No Content",
                "",
                &format!(
                    "Set-Cookie: {COOKIE}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_SECONDS}\r\n",
                    self.credential
                ),
            )
            .await;
        }
        if request.path.starts_with("/_wipsnet/") {
            return response(&mut socket, "404 Not Found", "", "").await;
        }
        if !request.values("service-worker").is_empty()
            || request.one("sec-fetch-dest") == Some("serviceworker")
        {
            return response(
                &mut socket,
                "403 Forbidden",
                "Service workers are unsupported",
                "",
            )
            .await;
        }
        if !["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
            .contains(&request.method.as_str())
        {
            return response(
                &mut socket,
                "405 Method Not Allowed",
                "Unsupported HTTP method",
                "",
            )
            .await;
        }
        if http::cookies(&request).is_err() {
            return response(&mut socket, "400 Bad Request", "Invalid cookies", "").await;
        }
        let cookies = request.values("cookie");
        let sessions: Vec<_> = cookies
            .iter()
            .flat_map(|v| v.split(';'))
            .map(str::trim)
            .filter(|s| s.split('=').next() == Some(COOKIE))
            .collect();
        if sessions.len() != 1
            || !constant_equal(sessions[0], &format!("{COOKIE}={}", self.credential))
        {
            return response(
                &mut socket,
                "401 Unauthorized",
                "Open this service from Wispnet",
                "",
            )
            .await;
        }
        if (websocket || !["GET", "HEAD"].contains(&request.method.as_str()))
            && request.one("origin") != Some(origin.as_str())
        {
            return response(
                &mut socket,
                "403 Forbidden",
                "Same-origin caller required",
                "",
            )
            .await;
        }
        let local = if self.peer == self.client.endpoint.id() {
            self.client.local_authority.as_ref()
        } else {
            None
        };
        // Resolve the binding from current durable owner state, never from UI/cache input.
        let binding = if let Some(authority) = local {
            match authority.services().await {
                Ok(services) => services
                    .into_iter()
                    .find(|s| {
                        s.id == self.service
                            && s.incarnation == self.incarnation
                            && s.kind == wipsnet_core::host::ServiceKind::Http
                            && Some(s.backend) == self.local_binding
                    })
                    .map(|s| s.backend),
                Err(_) => None,
            }
        } else {
            None
        };
        let _connection;
        let mut service: Box<dyn ServiceIo> = if local.is_some() {
            let Some(binding) = binding else {
                return response(
                    &mut socket,
                    "403 Forbidden",
                    "Local service unavailable",
                    "",
                )
                .await;
            };
            match TcpStream::connect(binding).await {
                Ok(stream) => Box::new(stream),
                Err(_) => {
                    return response(&mut socket, "502 Bad Gateway", "Backend unavailable", "")
                        .await;
                }
            }
        } else {
            let target = self.client.target(self.peer).map_err(|_| ())?;
            _connection = match wipsnet_agent::tunnel::connect_target(&self.client.endpoint, target)
                .await
            {
                Ok(c) => Close(c),
                Err(_) => {
                    return response(&mut socket, "502 Bad Gateway", "Host unavailable", "").await;
                }
            };
            match wipsnet_agent::tunnel::open_service_bound(
                &_connection.0,
                &self.service,
                &self.incarnation,
            )
            .await
            {
                Ok(stream) => {
                    self.client.record_telemetry(self.peer, &_connection.0);
                    Box::new(stream)
                }
                Err(_) => {
                    return response(
                        &mut socket,
                        "403 Forbidden",
                        "Host denied service access",
                        "",
                    )
                    .await;
                }
            }
        };
        let forwarding = async {
            let mut outgoing = format!("{} {} HTTP/1.1\r\n", request.method, request.path);
            for (name, value) in &request.headers {
                let lower = name.to_ascii_lowercase();
                if lower == "cookie" {
                    let clean = value
                        .split(';')
                        .map(str::trim)
                        .filter(|c| c.split('=').next() != Some(COOKIE))
                        .collect::<Vec<_>>()
                        .join("; ");
                    if !clean.is_empty() {
                        outgoing.push_str(&format!("Cookie: {clean}\r\n"));
                    }
                    continue;
                }
                if [
                    "connection",
                    "keep-alive",
                    "te",
                    "trailer",
                    "proxy-authenticate",
                    "proxy-authorization",
                    "proxy-connection",
                    "forwarded",
                    "sec-fetch-site",
                ]
                .contains(&lower.as_str())
                    || lower.starts_with("x-forwarded-")
                {
                    continue;
                }
                outgoing.push_str(&format!("{name}: {value}\r\n"));
            }
            outgoing.push_str(if websocket {
                "Connection: Upgrade\r\n\r\n"
            } else {
                "Connection: close\r\n\r\n"
            });
            service
                .write_all(outgoing.as_bytes())
                .await
                .map_err(|_| ())?;
            let copied = tokio::time::timeout(
                Duration::from_secs(30),
                tokio::io::copy(&mut (&mut socket).take(length), &mut service),
            )
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
            if copied != length {
                return Err(());
            }
            let head = match headers(&mut service)
                .await
                .and_then(|bytes| http::response(&bytes, &request, &origin, binding, websocket))
            {
                Ok(head) => head,
                Err(()) => {
                    return response(
                        &mut socket,
                        "502 Bad Gateway",
                        "Unsupported or malformed backend response",
                        "",
                    )
                    .await;
                }
            };
            socket.write_all(&head.bytes).await.map_err(|_| ())?;
            if matches!(head.body, http::Body::WebSocket) {
                let _ = tokio::io::copy_bidirectional(&mut socket, &mut service).await;
            } else {
                http::body(&mut service, &mut socket, head.body).await?;
            }
            Ok(())
        };
        let policy = async {
            let Some(authority) = local else {
                std::future::pending::<()>().await;
                return;
            };
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let current =
                    tokio::time::timeout(Duration::from_secs(1), authority.services()).await;
                if !matches!(current, Ok(Ok(ref services)) if services.iter().any(|s| s.id == self.service && s.incarnation == self.incarnation && s.kind == wipsnet_core::host::ServiceKind::Http && Some(s.backend) == binding))
                {
                    return;
                }
            }
        };
        tokio::select! { result = forwarding => result, _ = policy => Err(()) }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    async fn raw(url: &str, method: &str, path: &str, extra: &str) -> String {
        let u = url::Url::parse(url).unwrap();
        let mut socket = TcpStream::connect(("127.0.0.1", u.port().unwrap()))
            .await
            .unwrap();
        socket
            .write_all(
                format!(
                    "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\n{extra}\r\n",
                    u.host_str().unwrap(),
                    u.port().unwrap()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut data = Vec::new();
        socket.read_to_end(&mut data).await.unwrap();
        String::from_utf8(data).unwrap()
    }
    fn exchange(url: &str) -> String {
        let u = url::Url::parse(url).unwrap();
        format!(
            "Origin: http://{}:{}\r\nAuthorization: Bearer {}\r\nContent-Length: 0\r\n",
            u.host_str().unwrap(),
            u.port().unwrap(),
            u.fragment().unwrap()
        )
    }
    fn cookie(reply: &str) -> String {
        reply
            .lines()
            .find_map(|l| l.strip_prefix("Set-Cookie: "))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .into()
    }
    #[tokio::test]
    async fn expired_sessions_and_reused_ports_cannot_substitute_hosts_or_credentials() {
        let state = tempfile::tempdir().unwrap();
        let config =
            serde_json::from_value(serde_json::json!({"bind_addr":"127.0.0.1:0"})).unwrap();
        let client = Client::new(
            wipsnet_core::identity::SecretKey::generate(),
            state.path().into(),
            config,
        )
        .await
        .unwrap();
        client.gateway.enable();
        let peer = wipsnet_core::identity::SecretKey::generate().public();
        let boot = client
            .gateway
            .open_for(
                client.clone(),
                peer,
                "website".into(),
                "a".repeat(64),
                Limits {
                    session: Duration::from_secs(5),
                    bootstrap: Duration::from_millis(20),
                },
                None,
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(
            raw(&boot, "POST", "/_wipsnet/session", &exchange(&boot))
                .await
                .starts_with("HTTP/1.1 403")
        );
        client.gateway.interrupt().await;
        let old = client
            .gateway
            .open_for(
                client.clone(),
                peer,
                "website".into(),
                "a".repeat(64),
                Limits {
                    session: Duration::from_millis(150),
                    bootstrap: Duration::from_secs(1),
                },
                None,
            )
            .await
            .unwrap();
        let reply = raw(&old, "POST", "/_wipsnet/session", &exchange(&old)).await;
        assert!(reply.starts_with("HTTP/1.1 204"));
        let old_cookie = cookie(&reply);
        let port = url::Url::parse(&old).unwrap().port().unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        let other = wipsnet_core::identity::SecretKey::generate().public();
        let new = client
            .gateway
            .open_for(
                client.clone(),
                other,
                "website".into(),
                "b".repeat(64),
                Limits {
                    session: Duration::from_secs(5),
                    bootstrap: Duration::from_secs(1),
                },
                Some(listener),
            )
            .await
            .unwrap();
        assert_ne!(
            url::Url::parse(&old).unwrap().host_str(),
            url::Url::parse(&new).unwrap().host_str()
        );
        assert!(
            raw(&old, "GET", "/", &format!("Cookie: {old_cookie}\r\n"))
                .await
                .starts_with("HTTP/1.1 421")
        );
        assert!(
            raw(&new, "GET", "/", &format!("Cookie: {old_cookie}\r\n"))
                .await
                .starts_with("HTTP/1.1 401")
        );
        assert!(
            raw(&new, "POST", "/_wipsnet/session", &exchange(&old))
                .await
                .starts_with("HTTP/1.1 403")
        );
        let reply = raw(&new, "POST", "/_wipsnet/session", &exchange(&new)).await;
        assert!(reply.starts_with("HTTP/1.1 204"));
        assert_ne!(cookie(&reply), old_cookie);
        client.gateway.stop().await;
        client.endpoint.close().await;
    }
    #[tokio::test]
    async fn route_and_socket_limits_release_capacity_and_header_deadline_is_total() {
        let state = tempfile::tempdir().unwrap();
        let config =
            serde_json::from_value(serde_json::json!({"bind_addr":"127.0.0.1:0"})).unwrap();
        let client = Client::new(
            wipsnet_core::identity::SecretKey::generate(),
            state.path().into(),
            config,
        )
        .await
        .unwrap();
        client.gateway.enable();
        let peer = wipsnet_core::identity::SecretKey::generate().public();
        let mut urls = Vec::new();
        for _ in 0..16 {
            urls.push(
                client
                    .gateway
                    .open(client.clone(), peer, "website".into(), "a".repeat(64))
                    .await
                    .unwrap(),
            );
        }
        assert!(
            client
                .gateway
                .open(client.clone(), peer, "website".into(), "a".repeat(64))
                .await
                .is_err()
        );
        let port = url::Url::parse(&urls[0]).unwrap().port().unwrap();
        let mut sockets = Vec::new();
        for _ in 0..32 {
            sockets.push(TcpStream::connect(("127.0.0.1", port)).await.unwrap());
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while client.gateway.connections.available_permits() != 0 {
            assert!(tokio::time::Instant::now() < deadline);
            tokio::task::yield_now().await;
        }
        let mut overflow = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), overflow.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        // A continuously trickling header cannot reset the overall ten-second deadline.
        let (mut reader, mut writer) = tokio::io::duplex(64);
        let trickle = tokio::spawn(async move {
            loop {
                if writer.write_all(b"x").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        });
        let started = tokio::time::Instant::now();
        assert!(headers(&mut reader).await.is_err());
        assert!(started.elapsed() < Duration::from_secs(12));
        trickle.abort();
        client.gateway.stop().await;
        assert_eq!(client.gateway.connections.available_permits(), 32);
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
        client.gateway.enable();
        assert!(
            client
                .gateway
                .open(client.clone(), peer, "website".into(), "a".repeat(64))
                .await
                .is_ok()
        );
        client.gateway.stop().await;
        client.endpoint.close().await;
    }
    #[test]
    fn authority_origin_framing_and_cookie_isolation() {
        let request=parse(b"GET / HTTP/1.1\r\nHost: a.localhost:123\r\nOrigin: http://a.localhost:123\r\nSec-Fetch-Site: same-origin\r\n\r\n").unwrap();
        assert!(request.safe("a.localhost:123"));
        assert!(!request.safe("b.localhost:123"));
        assert!(request.same_origin("http://a.localhost:123"));
        assert!(!request.same_origin("http://b.localhost:123"));
        let duplicate =
            parse(b"GET / HTTP/1.1\r\nHost: a.localhost:123\r\nHost: b.localhost:123\r\n\r\n")
                .unwrap();
        assert!(!duplicate.safe("a.localhost:123"));
        assert_eq!(
            http::clean_cookie("app=yes; Domain=.localhost; Path=/"),
            Ok(Some("app=yes; Path=/".into()))
        );
        assert_eq!(http::clean_cookie("_wipsnet_session=bad; Path=/"), Ok(None));
    }
    #[tokio::test]
    async fn live_gateway_requires_one_use_session_and_stop_closes_listener() {
        let state = tempfile::tempdir().unwrap();
        let config: wipsnet_core::config::TransportConfig =
            serde_json::from_value(serde_json::json!({"bind_addr":"127.0.0.1:0"})).unwrap();
        let client = Client::new(
            wipsnet_core::identity::SecretKey::generate(),
            state.path().into(),
            config,
        )
        .await
        .unwrap();
        let peer = wipsnet_core::identity::SecretKey::generate().public();
        assert!(
            client
                .gateway
                .open(client.clone(), peer, "website".into(), "a".repeat(64))
                .await
                .is_err()
        );
        client.gateway.enable();
        let url = client
            .gateway
            .open(client.clone(), peer, "website".into(), "a".repeat(64))
            .await
            .unwrap();
        let (prefix, bootstrap) = url.split_once('#').unwrap();
        let authority = prefix
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        let port = authority.rsplit(':').next().unwrap();
        async fn request(
            port: &str,
            authority: &str,
            path: &str,
            extra: &str,
            method: &str,
        ) -> String {
            let mut socket = TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            socket.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Length: 0\r\n{extra}\r\n").as_bytes()).await.unwrap();
            let mut bytes = Vec::new();
            socket.read_to_end(&mut bytes).await.unwrap();
            String::from_utf8(bytes).unwrap()
        }
        assert!(
            request(port, "forged.localhost", "/", "", "GET")
                .await
                .starts_with("HTTP/1.1 421")
        );
        assert!(
            request(port, authority, "/", "", "GET")
                .await
                .starts_with("HTTP/1.1 401")
        );
        assert!(
            request(
                port,
                authority,
                "/",
                "Origin: https://attacker.invalid\r\n",
                "GET"
            )
            .await
            .starts_with("HTTP/1.1 403")
        );
        let exchange =
            format!("Origin: http://{authority}\r\nAuthorization: Bearer {bootstrap}\r\n");
        let reply = request(port, authority, "/_wipsnet/session", &exchange, "POST").await;
        assert!(reply.starts_with("HTTP/1.1 204"));
        assert!(reply.contains("HttpOnly; SameSite=Strict"));
        assert!(
            request(port, authority, "/_wipsnet/session", &exchange, "POST")
                .await
                .starts_with("HTTP/1.1 403")
        );
        client.gateway.interrupt().await;
        assert!(
            TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .is_err()
        );
        // A network interruption invalidates sessions but preserves explicit access.
        let reopened = client
            .gateway
            .open(client.clone(), peer, "website".into(), "a".repeat(64))
            .await
            .unwrap();
        let new_port = reopened
            .split('#')
            .next()
            .unwrap()
            .split('/')
            .nth(2)
            .unwrap()
            .rsplit(':')
            .next()
            .unwrap();
        client.gateway.stop().await;
        assert!(
            TcpStream::connect(format!("127.0.0.1:{new_port}"))
                .await
                .is_err()
        );
        assert!(
            client
                .gateway
                .open(client.clone(), peer, "website".into(), "a".repeat(64))
                .await
                .is_err()
        );
        client.endpoint.close().await;
    }
}
