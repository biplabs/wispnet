//! Public HTTP ingress, reached only through the local HTTPS front end.
use super::{
    ALPN, AUTH_ALPN, ManagedRegistration, Registration, hostname, read_json, valid_domain,
    write_json,
};
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited, combinators::UnsyncBoxBody};
use hyper::{
    Request, Response,
    body::Incoming,
    header::{HeaderName, HeaderValue},
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use iroh::{Endpoint, EndpointId, endpoint::Connection};
use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Mutex, Semaphore},
    task::JoinSet,
};
type Error = Box<dyn std::error::Error + Send + Sync>;
type Body = UnsyncBoxBody<Bytes, Error>;
struct Route {
    owner: EndpointId,
    service: String,
    incarnation: String,
    connection: Connection,
    slots: Arc<Semaphore>,
    authorization: Option<RouteAuthorization>,
}
struct RouteAuthorization {
    issuer: String,
    resource: String,
}
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandCenterConfig {
    pub base_url: String,
    #[serde(default)]
    pub allow_legacy: bool,
    /// Missing preserves the explicit historical host-wide migration setting.
    /// An empty list denies every legacy publication, even with allow_legacy.
    #[serde(default)]
    pub legacy_publications: Option<Vec<LegacyPublication>>,
}
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyPublication {
    pub owner: EndpointId,
    pub service: String,
    pub hostname: String,
}
pub struct Ingress {
    domain: String,
    publishers: Vec<EndpointId>,
    routes: Mutex<BTreeMap<String, Arc<Route>>>,
    requests: Arc<Semaphore>,
    command_center: Option<CommandCenterConfig>,
}
impl Ingress {
    pub fn new(domain: String, publishers: Vec<EndpointId>) -> Result<Arc<Self>, String> {
        Self::with_command_center(domain, publishers, None)
    }
    pub fn with_command_center(
        domain: String,
        publishers: Vec<EndpointId>,
        mut command_center: Option<CommandCenterConfig>,
    ) -> Result<Arc<Self>, String> {
        if !valid_domain(&domain)
            || publishers.is_empty() && command_center.is_none()
            || publishers.len() > 128
        {
            return Err("Invalid public gateway domain or publisher list".into());
        }
        if let Some(command) = &mut command_center {
            command.base_url = crate::command_center::validate_url(&command.base_url)?;
            if let Some(publications) = &command.legacy_publications {
                if publications.len() > 256 {
                    return Err("Too many legacy publication exceptions".into());
                }
                let mut names = BTreeSet::new();
                let mut services = BTreeSet::new();
                for publication in publications {
                    let suffix = format!(".{domain}");
                    let label = publication.hostname.strip_suffix(&suffix).unwrap_or("");
                    if !publishers.contains(&publication.owner)
                        || !wispnet_core::protocol::valid_service_id(&publication.service)
                        || publication.hostname.len() > 253
                        || label.is_empty()
                        || label.len() > 63
                        || label.starts_with('-')
                        || label.ends_with('-')
                        || !label
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                        || !names.insert(publication.hostname.clone())
                        || !services.insert((publication.owner, publication.service.clone()))
                    {
                        return Err("Invalid or duplicate legacy publication exception".into());
                    }
                }
            }
        }
        Ok(Arc::new(Self {
            domain,
            publishers,
            routes: Mutex::new(BTreeMap::new()),
            requests: Arc::new(Semaphore::new(128)),
            command_center,
        }))
    }
    #[cfg(test)]
    pub(super) async fn test_connection(&self, name: &str) -> Connection {
        self.routes
            .lock()
            .await
            .get(name)
            .unwrap()
            .connection
            .clone()
    }
    pub async fn run(self: Arc<Self>, endpoint: Endpoint, listener: TcpListener) {
        endpoint.set_alpns(vec![ALPN.to_vec(), AUTH_ALPN.to_vec()]);
        let connections = Arc::new(Semaphore::new(256));
        let http = Arc::new(Semaphore::new(256));
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                incoming=endpoint.accept()=>{let Some(incoming)=incoming else{break};let Ok(permit)=connections.clone().try_acquire_owned()else{incoming.refuse();continue};let state=self.clone();tasks.spawn(async move{let _permit=permit;if let Ok(Ok(connection))=tokio::time::timeout(Duration::from_secs(10),incoming).await {state.register(connection).await;}});},
                accepted=listener.accept()=>{let Ok((stream,peer))=accepted else{break};if !peer.ip().is_loopback(){continue;}let Ok(permit)=http.clone().try_acquire_owned()else{continue};let state=self.clone();tasks.spawn(async move{let _permit=permit;let _=hyper::server::conn::http1::Builder::new().timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(10)).max_buf_size(32768).serve_connection(TokioIo::new(stream),service_fn(move|req|state.clone().request(req))).with_upgrades().await;});},
                _=tasks.join_next(),if !tasks.is_empty()=>{}
            }
        }
    }
    async fn register(self: Arc<Self>, connection: Connection) {
        let owner = connection.remote_id();
        let managed = connection.alpn() == AUTH_ALPN;
        if !managed
            && (!self.publishers.contains(&owner)
                || self
                    .command_center
                    .as_ref()
                    .is_some_and(|c| !c.allow_legacy))
            || managed && self.command_center.is_none()
        {
            connection.close(1u32.into(), b"publisher not authorized");
            return;
        }
        let registration = tokio::time::timeout(Duration::from_secs(if managed {10} else {5}), async {
            let (mut send, mut recv) = connection.accept_bi().await.map_err(|_| ())?;
            let (record,authorization) = if managed {
                let protected: ManagedRegistration = read_json(&mut recv).await.map_err(|_| ())?;
                let configured = self.command_center.as_ref().ok_or(())?;
                if protected.version != 2 || protected.command_center != configured.base_url { return Err(()); }
                let expected = hostname(&protected.name,owner,&protected.service,&self.domain);
                let url = url::Url::parse(&protected.resource).map_err(|_| ())?;
                if url.scheme() != "https" || url.host_str() != Some(expected.as_str()) || url.port().is_some()
                    || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some()
                    || !super::valid_mcp_path(url.path()) { return Err(()); }
                let result = crate::command_center::post(&configured.base_url,"/api/command/host/register",Some(&protected.publisher_token),
                    serde_json::json!({"host":owner,"service":protected.service,"incarnation":protected.incarnation,"name":protected.name,"resource":protected.resource})).await.map_err(|_| ())?;
                if result["ok"] != true || result["resource"] != protected.resource { return Err(()); }
                let auth = RouteAuthorization {issuer:configured.base_url.clone(),resource:protected.resource};
                (Registration {version:1,service:protected.service,incarnation:protected.incarnation,name:protected.name},Some(auth))
            } else { (read_json(&mut recv).await.map_err(|_| ())?,None) };
            if record.version != 1
                || !wispnet_core::protocol::valid_service_id(&record.service)
                || !wispnet_core::enrollment::valid_code(&record.incarnation)
                || !wispnet_core::enrollment::valid_name(&record.name)
            {
                return Err(());
            }
            let name = hostname(&record.name, owner, &record.service, &self.domain);
            if !managed && self.command_center.as_ref()
                .and_then(|command|command.legacy_publications.as_ref())
                .is_some_and(|publications|!publications.iter().any(|publication|
                    publication.owner == owner && publication.service == record.service && publication.hostname == name))
            {
                return Err(());
            }
            let mut routes = self.routes.lock().await;
            if routes.len() >= 1024
                || routes.values().filter(|r| r.owner == owner).count() >= 16
                    && !routes.contains_key(&name)
                || routes
                    .get(&name)
                    .is_some_and(|r| r.owner != owner || r.service != record.service)
            {
                return Err(());
            }
            let route = Arc::new(Route {
                owner,
                service: record.service,
                incarnation: record.incarnation,
                connection: connection.clone(),
                slots: Arc::new(Semaphore::new(16)),
                authorization,
            });
            write_json(
                &mut send,
                &serde_json::json!({"ok":true,"url":format!("https://{name}/")}),
            )
            .await
            .map_err(|_| ())?;
            send.finish().map_err(|_| ())?;
            if let Some(old) = routes.insert(name.clone(), route) {
                old.connection.close(0u32.into(), b"publication replaced");
            }
            Ok::<_, ()>(name)
        })
        .await;
        let Ok(Ok(name)) = registration else {
            connection.close(1u32.into(), b"registration rejected");
            return;
        };
        connection.closed().await;
        let mut routes = self.routes.lock().await;
        if routes
            .get(&name)
            .is_some_and(|r| r.connection.stable_id() == connection.stable_id())
        {
            routes.remove(&name);
        }
    }
    async fn request(
        self: Arc<Self>,
        mut request: Request<Incoming>,
    ) -> Result<Response<Body>, Infallible> {
        // Caddy's on-demand certificate authorization endpoint is local-only.
        if request.uri().path() == "/_wispnet/allow" {
            if !request
                .headers()
                .get("host")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|h| h.starts_with("127.0.0.1:") || h.starts_with("localhost:"))
            {
                return Ok(error(404, "Not found"));
            }
            if request.method() != hyper::Method::GET {
                return Ok(error(405, "Method not allowed"));
            }
            let domain = request.uri().query().and_then(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .find(|(k, _)| k == "domain")
                    .map(|(_, v)| v.into_owned())
            });
            let allowed = if let Some(name) = domain {
                self.routes
                    .lock()
                    .await
                    .get(&name)
                    .is_some_and(|r| r.connection.close_reason().is_none())
            } else {
                false
            };
            return Ok(error(
                if allowed { 200 } else { 403 },
                if allowed { "Allowed" } else { "Not published" },
            ));
        }
        let host = if request.headers().get_all("host").iter().count() == 1 {
            request
                .headers()
                .get("host")
                .and_then(|h| h.to_str().ok())
                .map(str::to_owned)
        } else {
            None
        };
        let Some(host) = host else {
            return Ok(error(421, "Unknown public hostname"));
        };
        if !host.ends_with(&format!(".{}", self.domain))
            || request.uri().scheme().is_some()
            || request.uri().authority().is_some()
        {
            return Ok(error(421, "Unknown public hostname"));
        }
        let route = self.routes.lock().await.get(&host).cloned();
        let Some(route) = route else {
            return Ok(error(404, "Service is not published"));
        };
        if route.connection.close_reason().is_some() {
            return Ok(error(503, "Publishing node is offline"));
        }
        if let Some(auth) = &route.authorization {
            let resource_path = url::Url::parse(&auth.resource)
                .map(|u| u.path().to_owned())
                .unwrap_or_default();
            if request.uri().path() == "/.well-known/oauth-protected-resource"
                || request.uri().path()
                    == format!("/.well-known/oauth-protected-resource{resource_path}")
            {
                if request.method() != hyper::Method::GET && request.method() != hyper::Method::HEAD
                {
                    return Ok(error(405, "Method not allowed"));
                }
                let mut response = Response::new(Full::new(Bytes::from(serde_json::json!({"resource":auth.resource,"authorization_servers":[auth.issuer],"scopes_supported":["mcp:use"],"bearer_methods_supported":["header"]}).to_string())).map_err(|never|match never {}).boxed_unsync());
                response
                    .headers_mut()
                    .insert("content-type", HeaderValue::from_static("application/json"));
                response
                    .headers_mut()
                    .insert("access-control-allow-origin", HeaderValue::from_static("*"));
                return Ok(response);
            }
            if request.method() == hyper::Method::OPTIONS {
                let mut response = error(204, "");
                public_cors(response.headers_mut());
                return Ok(response);
            }
            if request
                .headers()
                .get_all(hyper::header::AUTHORIZATION)
                .iter()
                .count()
                != 1
            {
                return Ok(challenge(&host, 401));
            }
        }
        let Ok(global) = self.requests.clone().try_acquire_owned() else {
            return Ok(error(503, "Public gateway busy"));
        };
        let Ok(local) = route.slots.clone().try_acquire_owned() else {
            return Ok(error(503, "Service busy"));
        };
        if request
            .headers()
            .get(hyper::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|n| n > 16 * 1024 * 1024)
        {
            return Ok(error(413, "Request too large"));
        }
        if request.uri().path().starts_with("/_wispnet/") {
            return Ok(error(404, "Not found"));
        }
        let upgrade = request
            .headers()
            .get(hyper::header::UPGRADE)
            .is_some_and(|h| h.as_bytes().eq_ignore_ascii_case(b"websocket"));
        let downstream = if upgrade {
            Some(hyper::upgrade::on(&mut request))
        } else {
            None
        };
        sanitize(request.headers_mut(), upgrade);
        request
            .headers_mut()
            .insert("x-forwarded-proto", HeaderValue::from_static("https"));
        let stream = match wispnet_agent::tunnel::open_service_bound(
            &route.connection,
            &route.service,
            &route.incarnation,
        )
        .await
        {
            Ok(s) => s,
            Err(_) => return Ok(error(503, "Published service unavailable")),
        };
        let (mut sender, connection) =
            match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await {
                Ok(c) => c,
                Err(_) => return Ok(error(502, "Published service connection failed")),
            };
        let permits = Arc::new((global, local));
        let connection_permits = permits.clone();
        let owner = route.connection.clone();
        tokio::spawn(async move {
            let _permits = connection_permits;
            tokio::select! {_=connection.with_upgrades()=>{},_=owner.closed()=>{}}
        });
        let (parts, body) = request.into_parts();
        let request = Request::from_parts(parts, Limited::new(body, 16 * 1024 * 1024));
        let mut response = match tokio::time::timeout(
            Duration::from_secs(120),
            sender.send_request(request),
        )
        .await
        {
            Ok(Ok(r)) => r,
            _ => return Ok(error(502, "Published service did not respond")),
        };
        if route.authorization.is_some() {
            public_cors(response.headers_mut());
            if response.status() == hyper::StatusCode::UNAUTHORIZED
                || response.status() == hyper::StatusCode::FORBIDDEN
            {
                let value = format!(
                    "Bearer resource_metadata=\"https://{host}/.well-known/oauth-protected-resource\", scope=\"mcp:use\""
                );
                if let Ok(value) = HeaderValue::from_str(&value) {
                    response
                        .headers_mut()
                        .insert(hyper::header::WWW_AUTHENTICATE, value);
                }
            }
        }
        if upgrade && response.status() == hyper::StatusCode::SWITCHING_PROTOCOLS {
            let upstream = hyper::upgrade::on(&mut response);
            let owner = route.connection.clone();
            tokio::spawn(async move {
                let _permits = permits;
                if let (Some(downstream), Ok(upstream)) = (downstream, upstream.await)
                    && let Ok(downstream) = downstream.await
                {
                    let mut a = TokioIo::new(downstream);
                    let mut b = TokioIo::new(upstream);
                    tokio::select! {_=tokio::io::copy_bidirectional(&mut a,&mut b)=>{},_=owner.closed()=>{}}
                }
            });
        }
        let (mut parts, body) = response.into_parts();
        sanitize(
            &mut parts.headers,
            upgrade && parts.status == hyper::StatusCode::SWITCHING_PROTOCOLS,
        );
        parts
            .headers
            .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
        Ok(Response::from_parts(
            parts,
            body.map_err(|e| -> Error { Box::new(e) }).boxed_unsync(),
        ))
    }
}
fn public_cors(headers: &mut hyper::HeaderMap) {
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    headers.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
    );
    headers.insert("access-control-allow-headers",HeaderValue::from_static("Authorization, Content-Type, Accept, MCP-Protocol-Version, MCP-Session-Id, Last-Event-ID"));
    headers.insert(
        "access-control-expose-headers",
        HeaderValue::from_static("WWW-Authenticate, MCP-Session-Id, MCP-Protocol-Version"),
    );
}
fn challenge(host: &str, status: u16) -> Response<Body> {
    let mut response = error(status, "Authorization required");
    let value = format!(
        "Bearer resource_metadata=\"https://{host}/.well-known/oauth-protected-resource\", scope=\"mcp:use\""
    );
    if let Ok(value) = HeaderValue::from_str(&value) {
        response
            .headers_mut()
            .insert(hyper::header::WWW_AUTHENTICATE, value);
    }
    public_cors(response.headers_mut());
    response
}
fn sanitize(headers: &mut hyper::HeaderMap, upgrade: bool) {
    let nominated: Vec<_> = headers
        .get_all("connection")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|v| HeaderName::from_bytes(v.trim().as_bytes()).ok())
        .collect();
    for name in nominated {
        if !(upgrade && name == hyper::header::UPGRADE) {
            headers.remove(name);
        }
    }
    let names: Vec<_> = headers
        .keys()
        .filter(|n| n.as_str().starts_with("x-forwarded-"))
        .cloned()
        .collect();
    for name in names {
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
        "x-wispnet-access",
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
                .map_err(|e| -> Error { match e {} })
                .boxed_unsync(),
        )
        .unwrap()
}
