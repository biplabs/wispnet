//! Persistent, isolated loopback browser routes; authority is checked per request.
use crate::{Client, token};
use iroh::{EndpointId, endpoint::Connection};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Semaphore},
    task::{JoinHandle, JoinSet},
};
#[path = "gateway_http.rs"]
mod http;
#[path = "gateway_store.rs"]
mod store;
const COOKIE: &str = "_wipsnet_session";
const START: &str = r#"<!doctype html><meta name="referrer" content="no-referrer"><title>Wispnet</title><p id="status">Opening service…</p><script>const token=location.hash.slice(1);history.replaceState(null,'',location.pathname);fetch('/_wipsnet/session',{method:'POST',headers:{Authorization:'Bearer '+token}}).then(r=>{if(!r.ok)throw Error();location.replace('/')}).catch(()=>document.getElementById('status').textContent='Open this service again in Wispnet.');</script>"#;
struct Worker {
    binding: store::Binding,
    task: JoinHandle<()>,
    route: Arc<Route>,
}
pub struct Gateways {
    enabled: AtomicBool,
    workers: Mutex<Vec<Worker>>,
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
    /// Interrupt sockets without replaying requests or discarding saved addresses.
    pub async fn interrupt(&self) {
        let mut guard = self.workers.lock().await;
        for worker in guard.drain(..) {
            worker.task.abort();
            let _ = worker.task.await;
        }
    }
    /// App handoff proves an explicit Open even when Android Chrome marks it cross-site.
    /// This short-lived token never changes the bookmarked root address.
    pub async fn launch_url(&self, peer: EndpointId, service: &str) -> Result<String, String> {
        let workers = self.workers.lock().await;
        let worker = workers
            .iter()
            .find(|w| w.binding.peer == peer && w.binding.service == service)
            .ok_or("Browser route unavailable")?;
        let mut tokens = worker.route.bootstraps.lock().await;
        tokens.retain(|(_, expires)| *expires > std::time::Instant::now());
        if tokens.len() >= 8 {
            tokens.remove(0);
        }
        let token = token();
        tokens.push((
            token.clone(),
            std::time::Instant::now() + Duration::from_secs(60),
        ));
        Ok(format!(
            "http://{}/_wipsnet/start#{token}",
            worker.binding.authority()
        ))
    }
    /// Rebind exactly the saved ports. Never silently move a bookmark to a new port.
    pub async fn restore(&self, client: Arc<Client>) -> Result<(), String> {
        for binding in store::load(&client.state, client.endpoint.id())? {
            self.open(
                client.clone(),
                binding.peer,
                binding.service,
                binding.incarnation,
            )
            .await?;
        }
        Ok(())
    }
    pub async fn open(
        &self,
        client: Arc<Client>,
        peer: EndpointId,
        service: String,
        incarnation: String,
    ) -> Result<String, String> {
        self.open_named(client, peer, service, incarnation, None)
            .await
    }
    pub async fn open_named(
        &self,
        client: Arc<Client>,
        peer: EndpointId,
        service: String,
        incarnation: String,
        system_name: Option<&str>,
    ) -> Result<String, String> {
        if system_name.is_some_and(|s| !wipsnet_core::enrollment::valid_name(s)) {
            return Err("Invalid system display name".into());
        }
        if !wipsnet_core::protocol::valid_service_id(&service)
            || !wipsnet_core::enrollment::valid_code(&incarnation)
        {
            return Err("Invalid browser route".into());
        }
        let mut workers = self.workers.lock().await;
        workers.retain(|w| !w.task.is_finished());
        if !self.is_enabled() {
            return Err("Start browser access first".into());
        }
        let mut saved = store::load(&client.state, client.endpoint.id())?;
        let index = saved
            .iter()
            .position(|b| b.peer == peer && b.service == service);
        if index.is_none() && saved.len() >= 16 {
            return Err("Too many saved browser services".into());
        }
        let old = index.map(|i| &saved[i]);
        let migrate =
            old.is_some_and(|b| store::legacy_hostname(&b.hostname)) && system_name.is_some();
        let hostname = match old {
            Some(binding) if !migrate => binding.hostname.clone(),
            _ => store::readable_hostname(system_name, &service, &saved),
        };
        let aliases = if migrate {
            vec![old.unwrap().hostname.clone()]
        } else {
            old.map(|b| b.aliases.clone()).unwrap_or_default()
        };
        if let Some(worker) = workers.iter().find(|w| {
            w.binding.peer == peer
                && w.binding.service == service
                && w.binding.incarnation == incarnation
                && w.binding.hostname == hostname
        }) {
            return Ok(worker.binding.url());
        }
        let port = index.map(|i| saved[i].port).unwrap_or(0);
        // An explicit new incarnation replaces the binding, preserving its origin.
        if let Some(i) = workers
            .iter()
            .position(|w| w.binding.peer == peer && w.binding.service == service)
        {
            let old = workers.remove(i);
            old.task.abort();
            let _ = old.task.await;
        }
        let listener = TcpListener::bind(("127.0.0.1", port)).await.map_err(
            |_| "Saved browser port unavailable; close the conflicting application and try again",
        )?;
        let binding = store::Binding {
            peer,
            service,
            incarnation,
            hostname,
            aliases,
            port: listener
                .local_addr()
                .map_err(|_| "Cannot inspect browser port")?
                .port(),
        };
        if let Some(i) = index {
            saved[i] = binding.clone();
        } else {
            saved.push(binding.clone());
        }
        store::save(&client.state, client.endpoint.id(), &saved)?;
        let local_binding = if peer == client.endpoint.id() {
            if let Some(authority) = &client.local_authority {
                authority
                    .services()
                    .await
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|s| {
                        s.id == binding.service
                            && s.incarnation == binding.incarnation
                            && s.kind == wipsnet_core::host::ServiceKind::Http
                    })
                    .map(|s| s.backend)
            } else {
                None
            }
        } else {
            None
        };
        let route = Arc::new(Route {
            client,
            peer,
            service: binding.service.clone(),
            incarnation: binding.incarnation.clone(),
            local_binding,
            authorities: binding
                .names()
                .map(|name| format!("{}:{}", name, binding.port))
                .collect(),
            credential: token(),
            bootstraps: Mutex::new(Vec::new()),
        });
        let connections = self.connections.clone();
        let active_route = route.clone();
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((socket, address)) = accepted else { break; };
                        if !address.ip().is_loopback() { continue; }
                        let Ok(permit) = connections.clone().try_acquire_owned() else { continue; };
                        let route = route.clone();
                        tasks.spawn(async move { let _permit=permit; let _=route.handle(socket).await; });
                    }
                }
            }
        });
        let url = binding.url();
        workers.push(Worker {
            binding,
            task,
            route: active_route,
        });
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
    authorities: Vec<String>,
    credential: String,
    bootstraps: Mutex<Vec<(String, std::time::Instant)>>,
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
        if !self
            .authorities
            .iter()
            .any(|authority| request.safe(authority))
        {
            return response(
                &mut socket,
                "421 Misdirected Request",
                "Unknown local authority",
                "",
            )
            .await;
        }
        let origin = format!("http://{}", request.one("host").ok_or(())?);
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
        if request.path == "/_wipsnet/session" {
            let mut tokens = self.bootstraps.lock().await;
            tokens.retain(|(_, expires)| *expires > std::time::Instant::now());
            let index = tokens.iter().position(|(t, _)| {
                request
                    .one("authorization")
                    .is_some_and(|a| constant_equal(a, &format!("Bearer {t}")))
            });
            if !request.same_origin(&origin)
                || request.one("origin") != Some(origin.as_str())
                || request.method != "POST"
                || length != 0
                || websocket
                || index.is_none()
            {
                return response(&mut socket, "403 Forbidden", "Bootstrap denied", "").await;
            }
            tokens.remove(index.unwrap());
            return response(
                &mut socket,
                "204 No Content",
                "",
                &format!(
                    "Set-Cookie: {COOKIE}={}; Path=/; HttpOnly; SameSite=Strict\r\n",
                    self.credential
                ),
            )
            .await;
        }
        // Only a direct address-bar/bookmark/app navigation can establish a session.
        // Cross-site links, frames, fetches and unsafe methods cannot mint cookies.
        let navigation = request.method == "GET"
            && length == 0
            && !websocket
            && request.one("sec-fetch-site") == Some("none")
            && request.one("sec-fetch-mode") == Some("navigate")
            && request.one("sec-fetch-dest") == Some("document")
            && request.values("origin").is_empty();
        if !request.same_origin(&origin) {
            return response(
                &mut socket,
                "403 Forbidden",
                "Cross-origin request denied",
                "",
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
            if navigation {
                return response(&mut socket, "303 See Other", "", &format!("Location: {}\r\nSet-Cookie: {COOKIE}={}; Path=/; HttpOnly; SameSite=Strict\r\n", request.path, self.credential)).await;
            }
            return response(
                &mut socket,
                "401 Unauthorized",
                "Open this address directly in your browser while Wispnet is online",
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
            if !matches!(
                wipsnet_agent::peers::load(&self.client.state, self.peer),
                Ok(Some(_))
            ) {
                return response(
                    &mut socket,
                    "403 Forbidden",
                    "Pairing no longer available",
                    "",
                )
                .await;
            }
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
    use std::os::unix::fs::PermissionsExt;
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
    const NAV: &str =
        "Sec-Fetch-Site: none\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: document\r\n";
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
    async fn client(
        state: &std::path::Path,
        key: wipsnet_core::identity::SecretKey,
    ) -> Arc<Client> {
        Client::new(
            key,
            state.into(),
            serde_json::from_value(serde_json::json!({"bind_addr":"127.0.0.1:0"})).unwrap(),
        )
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn readable_names_migrate_preserve_aliases_and_survive_duplicate_system_names() {
        let state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let key = wipsnet_core::identity::SecretKey::generate();
        let c = client(state.path(), key.clone()).await;
        let peer = wipsnet_core::identity::SecretKey::generate().public();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let binding = store::Binding {
            peer,
            service: "llama".into(),
            incarnation: "a".repeat(64),
            hostname: format!("w{}.localhost", "b".repeat(32)),
            port: listener.local_addr().unwrap().port(),
            aliases: vec![],
        };
        drop(listener);
        store::save(
            state.path(),
            c.endpoint.id(),
            std::slice::from_ref(&binding),
        )
        .unwrap();
        let path = state.path().join("browser-routes.json");
        let mut v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        v["version"] = 1.into();
        std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        c.command(&serde_json::json!({"op":"enable"}))
            .await
            .unwrap();
        let url = c
            .gateway
            .open_named(
                c.clone(),
                peer,
                "llama".into(),
                "a".repeat(64),
                Some("J Strix"),
            )
            .await
            .unwrap();
        assert_eq!(
            url,
            format!("http://j-strix-llama.localhost:{}/", binding.port)
        );
        assert!(
            raw(&binding.url(), "GET", "/", NAV)
                .await
                .starts_with("HTTP/1.1 303")
        );
        assert!(raw(&url, "GET", "/", NAV).await.starts_with("HTTP/1.1 303"));
        let other_peer = wipsnet_core::identity::SecretKey::generate().public();
        let other = c
            .gateway
            .open_named(
                c.clone(),
                other_peer,
                "llama".into(),
                "a".repeat(64),
                Some("J Strix"),
            )
            .await
            .unwrap();
        assert_eq!(
            url::Url::parse(&other).unwrap().host_str(),
            Some("j-strix-llama-2.localhost")
        );
        c.gateway.stop().await;
        c.endpoint.close().await;
        drop(c);
        let c = client(state.path(), key).await;
        c.command(&serde_json::json!({"op":"enable"}))
            .await
            .unwrap();
        assert_eq!(
            url,
            c.gateway
                .open_named(
                    c.clone(),
                    peer,
                    "llama".into(),
                    "a".repeat(64),
                    Some("Renamed")
                )
                .await
                .unwrap()
        );
        assert_eq!(
            other,
            c.gateway
                .open_named(
                    c.clone(),
                    other_peer,
                    "llama".into(),
                    "a".repeat(64),
                    Some("Renamed")
                )
                .await
                .unwrap()
        );
        assert!(
            raw(&binding.url(), "GET", "/", NAV)
                .await
                .starts_with("HTTP/1.1 303")
        );
        c.gateway.stop().await;
        c.endpoint.close().await;
    }
    #[tokio::test]
    async fn stable_bookmarks_restore_ports_rotate_credentials_and_reject_cross_origin() {
        let state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let key = wipsnet_core::identity::SecretKey::generate();
        let c = client(state.path(), key.clone()).await;
        let peer = wipsnet_core::identity::SecretKey::generate().public();
        assert!(
            c.gateway
                .open(c.clone(), peer, "website".into(), "a".repeat(64))
                .await
                .is_err()
        );
        c.command(&serde_json::json!({"op":"enable"}))
            .await
            .unwrap();
        let url = c
            .gateway
            .open(c.clone(), peer, "website".into(), "a".repeat(64))
            .await
            .unwrap();
        let launch = c.gateway.launch_url(peer, "website").await.unwrap();
        let launch = url::Url::parse(&launch).unwrap();
        let exchange = format!(
            "Origin: http://{}\r\nAuthorization: Bearer {}\r\n",
            launch.authority(),
            launch.fragment().unwrap()
        );
        assert!(
            raw(&url, "POST", "/_wipsnet/session", &exchange)
                .await
                .starts_with("HTTP/1.1 204")
        );
        assert!(
            raw(&url, "POST", "/_wipsnet/session", &exchange)
                .await
                .starts_with("HTTP/1.1 403")
        );
        assert!(url.ends_with('/') && !url.contains('#'));
        assert_eq!(
            url,
            c.gateway
                .open(c.clone(), peer, "website".into(), "a".repeat(64))
                .await
                .unwrap()
        );
        assert!(raw(&url, "GET", "/", "").await.starts_with("HTTP/1.1 401"));
        for headers in [
            "Origin: https://attacker.invalid\r\n",
            "Sec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: document\r\n",
            "Sec-Fetch-Site: none\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: iframe\r\n",
        ] {
            assert!(!raw(&url, "GET", "/", headers).await.contains("Set-Cookie:"));
        }
        let reply = raw(&url, "GET", "/bookmarked?x=1", NAV).await;
        assert!(reply.starts_with("HTTP/1.1 303") && reply.contains("Location: /bookmarked?x=1"));
        assert!(reply.contains("HttpOnly; SameSite=Strict") && !reply.contains("Max-Age="));
        let old_cookie = cookie(&reply);
        let other = c
            .gateway
            .open(c.clone(), peer, "other".into(), "b".repeat(64))
            .await
            .unwrap();
        assert_ne!(
            url::Url::parse(&url).unwrap().host_str(),
            url::Url::parse(&other).unwrap().host_str()
        );
        assert!(
            raw(&other, "GET", "/", &format!("Cookie: {old_cookie}\r\n"))
                .await
                .starts_with("HTTP/1.1 401")
        );
        c.gateway.stop().await;
        let port = url::Url::parse(&url).unwrap().port().unwrap();
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
        let blocker = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        assert!(
            c.command(&serde_json::json!({"op":"enable"}))
                .await
                .is_err()
        );
        assert!(!c.gateway.is_enabled());
        drop(blocker);
        c.endpoint.close().await;
        drop(c);
        let c = client(state.path(), key).await;
        c.command(&serde_json::json!({"op":"enable"}))
            .await
            .unwrap();
        assert_eq!(
            url,
            c.gateway
                .open(c.clone(), peer, "website".into(), "a".repeat(64))
                .await
                .unwrap()
        );
        assert!(
            raw(&url, "GET", "/", &format!("Cookie: {old_cookie}\r\n"))
                .await
                .starts_with("HTTP/1.1 401")
        );
        assert_ne!(old_cookie, cookie(&raw(&url, "GET", "/", NAV).await));
        c.command(&serde_json::json!({"op":"network"}))
            .await
            .unwrap();
        assert!(raw(&url, "GET", "/", NAV).await.starts_with("HTTP/1.1 303"));
        c.gateway.stop().await;
        c.endpoint.close().await;
        let different = client(state.path(), wipsnet_core::identity::SecretKey::generate()).await;
        assert!(
            different
                .command(&serde_json::json!({"op":"enable"}))
                .await
                .is_err()
        );
        different.endpoint.close().await;
    }
    #[tokio::test]
    async fn persistent_route_checks_local_policy_on_every_request_and_survives_old_deadline() {
        use wipsnet_registry::host::{self, Authority, Mutation};
        let state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let key = wipsnet_core::identity::SecretKey::generate();
        let peer = key.public();
        host::initialize(state.path(), peer).unwrap();
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = backend.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = backend.accept().await else {
                    break;
                };
                let _ = headers(&mut s).await;
                let _ = s
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
            }
        });
        {
            let (mut store, owner) = host::owner_store(state.path(), peer).unwrap();
            store
                .host_mutate(
                    peer,
                    &owner,
                    Mutation::Service {
                        expected_revision: 0,
                        id: "site".into(),
                        backend: address,
                        allow: vec![],
                        display_name: None,
                        kind: None,
                    },
                )
                .unwrap();
        }
        let authority = Authority::open(state.path(), peer).unwrap();
        let service = authority.services().await.unwrap().remove(0);
        let config =
            serde_json::from_value(serde_json::json!({"bind_addr":"127.0.0.1:0"})).unwrap();
        let endpoint = wipsnet_agent::tunnel::bind_configured_client(&key, Some(&config))
            .await
            .unwrap();
        let c = Client::from_endpoint(
            endpoint,
            key,
            state.path().into(),
            config,
            authority.clone(),
        );
        c.gateway.enable();
        let url = c
            .gateway
            .open(c.clone(), peer, "site".into(), service.incarnation)
            .await
            .unwrap();
        let session = cookie(&raw(&url, "GET", "/", NAV).await);
        assert!(
            raw(&url, "GET", "/", &format!("Cookie: {session}\r\n"))
                .await
                .ends_with("ok")
        );
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(601)).await;
        tokio::time::resume();
        assert!(
            raw(&url, "GET", "/", &format!("Cookie: {session}\r\n"))
                .await
                .ends_with("ok")
        );
        {
            let (mut store, owner) = host::owner_store(state.path(), peer).unwrap();
            store
                .host_mutate(
                    peer,
                    &owner,
                    Mutation::Remove {
                        expected_revision: 1,
                        id: "site".into(),
                    },
                )
                .unwrap();
        }
        assert!(
            raw(&url, "GET", "/", &format!("Cookie: {session}\r\n"))
                .await
                .starts_with("HTTP/1.1 403")
        );
        c.gateway.stop().await;
        c.endpoint.close().await;
        server.abort();
    }
    #[tokio::test]
    async fn route_and_socket_limits_release_capacity_and_header_deadline_is_total() {
        let state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
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
        for i in 0..16 {
            urls.push(
                client
                    .gateway
                    .open(client.clone(), peer, format!("website-{i}"), "a".repeat(64))
                    .await
                    .unwrap(),
            );
        }
        assert!(
            client
                .gateway
                .open(client.clone(), peer, "overflow".into(), "a".repeat(64))
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
                .open(client.clone(), peer, "website-0".into(), "a".repeat(64))
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
}
