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
pub const SESSION_SECONDS: u64 = 600;
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
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "Cannot start loopback gateway")?;
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
            bootstrap_expires: Instant::now() + Duration::from_secs(60),
            expires: Instant::now() + Duration::from_secs(SESSION_SECONDS),
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
    }
}
async fn headers(stream: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>, ()> {
    let mut bytes = Vec::new();
    // Reading exactly through the terminator avoids losing streamed body bytes.
    while bytes.len() < 32768 {
        let byte = tokio::time::timeout(Duration::from_secs(10), stream.read_u8())
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        bytes.push(byte);
        if bytes.ends_with(b"\r\n\r\n") {
            return Ok(bytes);
        }
    }
    Err(())
}
fn parse(bytes: &[u8]) -> Result<Request, ()> {
    let mut storage = [httparse::EMPTY_HEADER; 96];
    let mut request = httparse::Request::new(&mut storage);
    if !request.parse(bytes).map_err(|_| ())?.is_complete() {
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
        let request = parse(&headers(&mut socket).await?)?;
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
        let websocket = request
            .one("upgrade")
            .is_some_and(|s| s.eq_ignore_ascii_case("websocket"));
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
        if !request.values("transfer-encoding").is_empty()
            || !request.values("expect").is_empty()
            || request.values("content-length").len() > 1
        {
            return response(
                &mut socket,
                "400 Bad Request",
                "Unsupported request framing",
                "",
            )
            .await;
        }
        let length = match request.one("content-length") {
            Some(v) => v.parse::<u64>().map_err(|_| ())?,
            None => 0,
        };
        if length > 16 * 1024 * 1024 {
            return response(&mut socket, "413 Content Too Large", "", "").await;
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
                Ok(stream) => Box::new(stream),
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
            let incoming = headers(&mut service).await?;
            let incoming = std::str::from_utf8(&incoming).map_err(|_| ())?;
            let mut lines = incoming.split("\r\n");
            let status = lines.next().ok_or(())?;
            if !status.starts_with("HTTP/1.") {
                return Err(());
            }
            let mut filtered = format!("{status}\r\n");
            for line in lines.filter(|s| !s.is_empty()) {
                let (name, value) = line.split_once(':').ok_or(())?;
                if name.eq_ignore_ascii_case("set-cookie") {
                    if let Some(cookie) = clean_cookie(value.trim()) {
                        filtered.push_str(&format!("Set-Cookie: {cookie}\r\n"));
                    }
                } else if name.eq_ignore_ascii_case("location") {
                    let value = value.trim();
                    if value.starts_with("//")
                        || (value
                            .split(['/', '?', '#'])
                            .next()
                            .unwrap_or("")
                            .contains(':')
                            && !value.starts_with(&format!("{origin}/")))
                    {
                        return response(
                            &mut socket,
                            "502 Bad Gateway",
                            "Cross-origin redirect blocked",
                            "",
                        )
                        .await;
                    }
                    filtered.push_str(&format!("Location: {value}\r\n"));
                } else {
                    filtered.push_str(line);
                    filtered.push_str("\r\n");
                }
            }
            filtered.push_str("Referrer-Policy: no-referrer\r\n\r\n");
            socket
                .write_all(filtered.as_bytes())
                .await
                .map_err(|_| ())?;
            if websocket && status.contains(" 101 ") {
                let _ = tokio::io::copy_bidirectional(&mut socket, &mut service).await;
            } else {
                let _ = tokio::io::copy(&mut service, &mut socket).await;
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
fn clean_cookie(value: &str) -> Option<String> {
    let name = value.split(';').next()?.split('=').next()?.trim();
    if name == COOKIE {
        return None;
    }
    Some(
        value
            .split(';')
            .filter(|part| {
                !part
                    .split('=')
                    .next()
                    .is_some_and(|name| name.trim().eq_ignore_ascii_case("domain"))
            })
            .collect::<Vec<_>>()
            .join(";"),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
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
            clean_cookie("app=yes; Domain=.localhost; Path=/"),
            Some("app=yes; Path=/".into())
        );
        assert_eq!(clean_cookie("_wipsnet_session=bad; Path=/"), None);
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
