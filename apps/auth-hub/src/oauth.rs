//! The public-client OAuth profile used by Wispnet's HTTP MCP services.
//! Tokens are opaque, hashed at rest, and checked against live service grants.
use crate::{
    Error, Result, Shared, body_json, db, digest, identity, json, now, same_origin, token,
};
use axum::{
    body::Body,
    http::{HeaderValue, Method, Request, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use url::Url;

const AUTH: &str = "/api/auth/oauth2/";
const ACCESS_LIFETIME: i64 = 300;
const REFRESH_LIFETIME: i64 = 30 * 24 * 3600;
const MAX_REGISTERED_CLIENTS: i64 = 10_000;
type Fields = BTreeMap<String, String>;

pub fn initialize(db: &Connection) -> Result<()> {
    if let Err(existing) = jsonwebtoken::crypto::rust_crypto::DEFAULT_PROVIDER.install_default()
        && !std::ptr::eq(
            existing,
            &jsonwebtoken::crypto::rust_crypto::DEFAULT_PROVIDER,
        )
    {
        return Err(Error::new(500, "Unexpected JWT crypto provider"));
    }
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS oauth_clients (
            client_id TEXT PRIMARY KEY, metadata TEXT NOT NULL, source TEXT NOT NULL,
            expires INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS oauth_pending (
            id TEXT PRIMARY KEY, user_id TEXT NOT NULL, session_hash TEXT NOT NULL,
            request TEXT NOT NULL, expires INTEGER NOT NULL, consumed INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS oauth_codes (
            token_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL, client_id TEXT NOT NULL,
            redirect_uri TEXT NOT NULL, resource TEXT NOT NULL, scope TEXT NOT NULL,
            challenge TEXT NOT NULL, expires INTEGER NOT NULL, consumed INTEGER NOT NULL DEFAULT 0,
            service_id TEXT NOT NULL, service_generation INTEGER NOT NULL, grant_generation INTEGER NOT NULL,
            connection_id TEXT NOT NULL, connection_generation INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS oauth_access_tokens (
            token_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL, client_id TEXT NOT NULL,
            resource TEXT NOT NULL, scope TEXT NOT NULL, expires INTEGER NOT NULL,
            service_id TEXT NOT NULL, service_generation INTEGER NOT NULL, grant_generation INTEGER NOT NULL,
            connection_id TEXT NOT NULL, connection_generation INTEGER NOT NULL, family_id TEXT NOT NULL,
            revoked INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS oauth_refresh_tokens (
            token_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL, client_id TEXT NOT NULL,
            resource TEXT NOT NULL, scope TEXT NOT NULL, expires INTEGER NOT NULL,
            service_id TEXT NOT NULL, service_generation INTEGER NOT NULL, grant_generation INTEGER NOT NULL,
            connection_id TEXT NOT NULL, connection_generation INTEGER NOT NULL, family_id TEXT NOT NULL,
            used INTEGER NOT NULL DEFAULT 0, revoked INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS oauth_jwks (
            client_id TEXT PRIMARY KEY,uri TEXT NOT NULL,document TEXT NOT NULL,expires INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS oauth_assertions(jti_hash TEXT PRIMARY KEY,expires INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS oauth_access_connection ON oauth_access_tokens(connection_id);
        CREATE INDEX IF NOT EXISTS oauth_refresh_connection ON oauth_refresh_tokens(connection_id);
        CREATE INDEX IF NOT EXISTS oauth_pending_expiry ON oauth_pending(expires);
        CREATE INDEX IF NOT EXISTS oauth_code_expiry ON oauth_codes(expires);
        CREATE INDEX IF NOT EXISTS oauth_access_expiry ON oauth_access_tokens(expires);
        CREATE INDEX IF NOT EXISTS oauth_refresh_expiry ON oauth_refresh_tokens(expires);
        CREATE INDEX IF NOT EXISTS oauth_code_client ON oauth_codes(client_id);
        CREATE INDEX IF NOT EXISTS oauth_access_client ON oauth_access_tokens(client_id);
        CREATE INDEX IF NOT EXISTS oauth_refresh_client ON oauth_refresh_tokens(client_id);
        CREATE INDEX IF NOT EXISTS oauth_assertion_expiry ON oauth_assertions(expires);",
    )?;
    Ok(())
}

pub fn handles(path: &str) -> bool {
    path.starts_with(AUTH)
        || path == "/.well-known/oauth-authorization-server"
        || path == "/.well-known/oauth-authorization-server/api/auth"
}

pub async fn handle(hub: Shared, request: Request<Body>) -> Result<Response> {
    let public = !matches!(request.uri().path(), "/api/auth/oauth2/consent");
    let mut response = route(hub, request)
        .await
        .unwrap_or_else(IntoResponse::into_response);
    if public {
        for (name, value) in [
            ("access-control-allow-origin", "*"),
            ("access-control-allow-methods", "GET, POST, OPTIONS"),
            (
                "access-control-allow-headers",
                "Authorization, Content-Type",
            ),
        ] {
            response
                .headers_mut()
                .insert(name, HeaderValue::from_static(value));
        }
    }
    Ok(response)
}

async fn route(hub: Shared, request: Request<Body>) -> Result<Response> {
    cleanup(&hub)?;
    let path = request.uri().path().to_owned();
    if request.method() == Method::OPTIONS {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    if request.method() == Method::GET && path.starts_with("/.well-known/") {
        let root = &hub.config.base_url;
        return Ok(json(json!({
            "issuer":root,
            "authorization_endpoint":format!("{root}{AUTH}authorize"),
            "token_endpoint":format!("{root}{AUTH}token"),
            "registration_endpoint":format!("{root}{AUTH}register"),
            "revocation_endpoint":format!("{root}{AUTH}revoke"),
            "response_types_supported":["code"],
            "response_modes_supported":["query"],
            "grant_types_supported":["authorization_code","refresh_token"],
            "scopes_supported":["mcp:use","offline_access"],
            "token_endpoint_auth_methods_supported":["none","private_key_jwt"],
            "revocation_endpoint_auth_methods_supported":["none","private_key_jwt"],
            "token_endpoint_auth_signing_alg_values_supported":["RS256"],
            "code_challenge_methods_supported":["S256"],
            "client_id_metadata_document_supported":true,
            "authorization_response_iss_parameter_supported":true
        })));
    }
    match (request.method(), path.as_str()) {
        (&Method::POST, "/api/auth/oauth2/register") => register(hub, request).await,
        (&Method::GET, "/api/auth/oauth2/authorize") => authorize(hub, request).await,
        (&Method::POST, "/api/auth/oauth2/consent") => consent(hub, request).await,
        (&Method::POST, "/api/auth/oauth2/token") => exchange(hub, request).await,
        (&Method::POST, "/api/auth/oauth2/revoke") => revoke(hub, request).await,
        _ => Err(Error::new(404, "Not found")),
    }
}

fn cleanup(hub: &Shared) -> Result<()> {
    if hub.throttle("oauth:cleanup", 1).is_ok() {
        let db = hub.db.lock().unwrap();
        // Delete only bounded batches of expired rows. Used refresh credentials
        // remain replay evidence until their original family's expiry.
        for table in [
            "oauth_pending",
            "oauth_codes",
            "oauth_access_tokens",
            "oauth_refresh_tokens",
            "oauth_assertions",
            "oauth_jwks",
        ] {
            db.execute(&format!("DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} WHERE expires<=? LIMIT 2048)"), [now()])?;
        }
        // Bound anonymous abandoned registrations while retaining clients that
        // have a live grant or are in the middle of an authorization flow.
        db.execute("DELETE FROM oauth_clients WHERE client_id IN (
            SELECT c.client_id FROM oauth_clients c WHERE c.source='dcr' AND c.expires<=?
            AND NOT EXISTS(SELECT 1 FROM command_oauth_connections x WHERE x.client_id=c.client_id AND x.active=1)
            AND NOT EXISTS(SELECT 1 FROM oauth_pending p WHERE p.expires>? AND p.consumed=0 AND json_extract(p.request,'$.client_id')=c.client_id)
            AND NOT EXISTS(SELECT 1 FROM oauth_codes x WHERE x.client_id=c.client_id AND x.expires>? AND x.consumed=0)
            AND NOT EXISTS(SELECT 1 FROM oauth_access_tokens x WHERE x.client_id=c.client_id AND x.expires>? AND x.revoked=0)
            AND NOT EXISTS(SELECT 1 FROM oauth_refresh_tokens x WHERE x.client_id=c.client_id AND x.expires>? AND x.revoked=0)
            LIMIT 256)",params![now(),now(),now(),now(),now()])?;
        db.execute("DELETE FROM oauth_clients WHERE client_id IN (SELECT client_id FROM oauth_clients WHERE source='cimd' AND expires<=? LIMIT 256)",[now()])?;
    }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
struct Client {
    client_id: String,
    client_name: String,
    redirect_uris: Vec<String>,
    grant_types: Vec<String>,
    response_types: Vec<String>,
    token_endpoint_auth_method: String,
    scope: String,
    application_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    jwks_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_endpoint_auth_signing_alg: Option<String>,
}

fn string_list(value: Option<&Value>, default: &[&str], max: usize) -> Result<Vec<String>> {
    match value {
        None => Ok(default.iter().map(|s| (*s).into()).collect()),
        Some(value) => {
            let values = value
                .as_array()
                .filter(|v| !v.is_empty() && v.len() <= max)
                .ok_or_else(|| Error::oauth(400, "invalid_client_metadata"))?;
            values
                .iter()
                .map(|v| {
                    v.as_str()
                        .filter(|s| {
                            !s.is_empty() && s.len() <= 2048 && !s.chars().any(char::is_control)
                        })
                        .map(str::to_owned)
                        .ok_or_else(|| Error::oauth(400, "invalid_client_metadata"))
                })
                .collect()
        }
    }
}

fn loopback_redirect(url: &Url) -> bool {
    url.scheme() == "http"
        && url.host().is_some_and(|host| match host {
            url::Host::Ipv4(ip) => ip.is_loopback(),
            url::Host::Ipv6(ip) => ip.is_loopback(),
            url::Host::Domain(name) => name == "localhost",
        })
}

fn redirect_matches(client: &Client, requested: &str) -> bool {
    client.redirect_uris.iter().any(|registered| {
        if registered == requested {
            return true;
        }
        if client.application_type != "native" {
            return false;
        }
        let (Ok(mut registered), Ok(mut requested)) =
            (Url::parse(registered), Url::parse(requested))
        else {
            return false;
        };
        if !loopback_redirect(&registered) || !loopback_redirect(&requested) {
            return false;
        }
        // RFC 8252 permits a native listener's random port, keeping all other
        // URI components bound. Token exchange still requires the exact URI
        // from the individual authorization request.
        let _ = registered.set_port(None);
        let _ = requested.set_port(None);
        registered == requested
    })
}

fn valid_redirect(value: &str, native: bool) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url
            .query_pairs()
            .any(|(k, _)| matches!(k.as_ref(), "code" | "error" | "state" | "iss"))
    {
        return false;
    }
    if url.scheme() == "https" {
        return url.host_str().is_some();
    }
    native
        && (loopback_redirect(&url)
            || url.scheme().contains('.') && url.scheme() != "javascript" && url.scheme() != "data")
}

fn scopes(value: &str) -> Result<String> {
    let mut result = Vec::new();
    for scope in value.split_whitespace() {
        if !matches!(scope, "mcp:use" | "offline_access") {
            return Err(Error::oauth(400, "invalid_scope"));
        }
        if !result.contains(&scope) {
            result.push(scope);
        }
    }
    if !result.contains(&"mcp:use") {
        return Err(Error::oauth(400, "invalid_scope"));
    }
    Ok(result.join(" "))
}

fn metadata(value: &Value, id: String, negotiate: bool) -> Result<Client> {
    if !value.is_object() {
        return Err(Error::oauth(400, "invalid_client_metadata"));
    }
    let redirect_uris = string_list(value.get("redirect_uris"), &[], 16)?;
    if redirect_uris.is_empty() {
        return Err(Error::oauth(400, "invalid_redirect_uri"));
    }
    let inferred_native = redirect_uris
        .iter()
        .all(|r| Url::parse(r).is_ok_and(|u| loopback_redirect(&u) || u.scheme().contains('.')));
    let application_type = value
        .get("application_type")
        .and_then(Value::as_str)
        .unwrap_or(if inferred_native { "native" } else { "web" });
    if !matches!(application_type, "native" | "web")
        || redirect_uris
            .iter()
            .any(|r| !valid_redirect(r, application_type == "native"))
    {
        return Err(Error::oauth(400, "invalid_redirect_uri"));
    }
    let declared_grants = string_list(
        value.get("grant_types"),
        &["authorization_code", "refresh_token"],
        16,
    )?;
    if !declared_grants.iter().any(|g| g == "authorization_code")
        || !negotiate
            && declared_grants
                .iter()
                .any(|g| !matches!(g.as_str(), "authorization_code" | "refresh_token"))
    {
        return Err(Error::oauth(400, "invalid_client_metadata"));
    }
    // Global client metadata can list capabilities for other authorization
    // servers. Persist only the intersection this server actually supports.
    let grant_types = declared_grants
        .into_iter()
        .filter(|g| matches!(g.as_str(), "authorization_code" | "refresh_token"))
        .collect();
    let declared_responses = string_list(value.get("response_types"), &["code"], 16)?;
    if !declared_responses.iter().any(|s| s == "code")
        || !negotiate && declared_responses.iter().any(|s| s != "code")
    {
        return Err(Error::oauth(400, "invalid_client_metadata"));
    }
    let response_types = vec!["code".into()];
    let method = if let Some(method) = value.get("token_endpoint_auth_method") {
        method
            .as_str()
            .ok_or_else(|| Error::oauth(400, "invalid_client_metadata"))?
    } else if let Some(methods) = value.get("token_endpoint_auth_methods_supported") {
        let methods = string_list(Some(methods), &[], 16)?;
        if negotiate && methods.iter().any(|s| s == "private_key_jwt") {
            "private_key_jwt"
        } else if methods.iter().any(|s| s == "none") {
            "none"
        } else {
            return Err(Error::oauth(400, "invalid_client_metadata"));
        }
    } else {
        "none"
    };
    if method != "none" && !(negotiate && method == "private_key_jwt") {
        return Err(Error::oauth(400, "invalid_client_metadata"));
    }
    let jwks_uri = if method == "private_key_jwt" {
        let uri = value
            .get("jwks_uri")
            .and_then(Value::as_str)
            .filter(|s| s.len() <= 2048)
            .ok_or_else(|| Error::oauth(400, "invalid_client_metadata"))?;
        secure_metadata_url(uri)?;
        if value
            .get("token_endpoint_auth_signing_alg")
            .and_then(Value::as_str)
            .is_some_and(|s| s != "RS256")
        {
            return Err(Error::oauth(400, "invalid_client_metadata"));
        }
        Some(uri.to_owned())
    } else {
        None
    };
    let client_name = value
        .get("client_name")
        .and_then(Value::as_str)
        .unwrap_or("MCP client");
    if client_name.len() > 120 || client_name.chars().any(char::is_control) {
        return Err(Error::oauth(400, "invalid_client_metadata"));
    }
    Ok(Client {
        client_id: id,
        client_name: client_name.into(),
        redirect_uris,
        grant_types,
        response_types,
        token_endpoint_auth_method: method.into(),
        scope: scopes(
            value
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("mcp:use offline_access"),
        )?,
        application_type: application_type.into(),
        jwks_uri,
        token_endpoint_auth_signing_alg: (method == "private_key_jwt").then(|| "RS256".into()),
    })
}

async fn register(hub: Shared, request: Request<Body>) -> Result<Response> {
    hub.throttle("oauth:register", 60)?;
    let (_, value) = body_json(request).await?;
    let client = metadata(&value, token("woc_"), false)?;
    let encoded = serde_json::to_string(&client).map_err(|_| Error::new(500, "Invalid client"))?;
    let db = hub.db.lock().unwrap();
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM oauth_clients WHERE source='dcr'",
        [],
        |r| r.get(0),
    )?;
    if count >= MAX_REGISTERED_CLIENTS {
        return Err(Error::new(429, "Client registration capacity reached"));
    }
    db.execute(
        "INSERT INTO oauth_clients(client_id,metadata,source,expires) VALUES(?,?,'dcr',?)",
        params![client.client_id, encoded, now() + 86400],
    )?;
    let mut response =
        json(serde_json::to_value(client).map_err(|_| Error::new(500, "Invalid client"))?);
    *response.status_mut() = StatusCode::CREATED;
    Ok(response)
}

fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_multicast()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && b == 0)
                && !(a == 198 && (b == 18 || b == 19))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_address(IpAddr::V4(v4));
            }
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
        }
    }
}

async fn client(hub: &Shared, id: &str) -> Result<Client> {
    if id.len() > 2048 || id.chars().any(char::is_control) {
        return Err(Error::oauth(400, "invalid_client"));
    }
    let cached: Option<(String, String, i64)> = hub
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT metadata,source,expires FROM oauth_clients WHERE client_id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((encoded, source, expires)) = cached
        && (source == "dcr" || expires > now())
    {
        return serde_json::from_str(&encoded).map_err(|_| Error::oauth(400, "invalid_client"));
    }
    hub.throttle("oauth:cimd", 60)?;
    if secure_metadata_url(id)?.path() == "/" {
        return Err(Error::oauth(400, "invalid_client"));
    }
    let response = remote_response(id).await?;
    let parsed = document(response, id).await?;
    let encoded = serde_json::to_string(&parsed).map_err(|_| Error::new(500, "Invalid client"))?;
    hub.db.lock().unwrap().execute("INSERT INTO oauth_clients(client_id,metadata,source,expires) VALUES(?,?,'cimd',?) ON CONFLICT(client_id) DO UPDATE SET metadata=excluded.metadata,expires=excluded.expires", params![id,encoded,now()+300])?;
    Ok(parsed)
}

fn secure_metadata_url(id: &str) -> Result<Url> {
    let uri = Url::parse(id).map_err(|_| Error::oauth(400, "invalid_client"))?;
    if uri.scheme() != "https"
        || uri.host_str().is_none()
        || !uri.username().is_empty()
        || uri.password().is_some()
        || uri.fragment().is_some()
        || uri.port_or_known_default() != Some(443)
        || uri.as_str() != id
    {
        return Err(Error::oauth(400, "invalid_client"));
    }
    Ok(uri)
}

async fn remote_response(id: &str) -> Result<reqwest::Response> {
    let uri = secure_metadata_url(id)?;
    let host = uri.host_str().unwrap();
    let addresses: Vec<SocketAddr> =
        tokio::time::timeout(Duration::from_secs(2), tokio::net::lookup_host((host, 443)))
            .await
            .map_err(|_| Error::oauth(400, "invalid_client"))?
            .map_err(|_| Error::oauth(400, "invalid_client"))?
            .collect();
    if addresses.is_empty() || addresses.iter().any(|a| !public_address(a.ip())) {
        return Err(Error::oauth(400, "invalid_client"));
    }
    // Pin the addresses that were checked; a second DNS lookup would permit rebinding.
    let http = reqwest::Client::builder()
        .no_proxy()
        .user_agent("Wispnet-Command-Center")
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| Error::oauth(400, "invalid_client"))?;
    http.get(id)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|_| Error::oauth(400, "invalid_client"))
}

async fn remote_value(mut response: reqwest::Response) -> Result<Value> {
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if !response.status().is_success()
        || response.content_length().is_some_and(|n| n > 32768)
        || !(content_type == "application/json" || content_type.ends_with("+json"))
    {
        return Err(Error::oauth(400, "invalid_client"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Error::oauth(400, "invalid_client"))?
    {
        if bytes.len() + chunk.len() > 32768 {
            return Err(Error::oauth(400, "invalid_client"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::oauth(400, "invalid_client"))
}

async fn document(response: reqwest::Response, id: &str) -> Result<Client> {
    let value = remote_value(response).await?;
    if value.get("client_id").and_then(Value::as_str) != Some(id) {
        return Err(Error::oauth(400, "invalid_client"));
    }
    metadata(&value, id.into(), true)
}

fn fields(value: &str) -> Result<Fields> {
    if value.len() > 32768 {
        return Err(Error::oauth(400, "invalid_request"));
    }
    let mut result = Fields::new();
    for (key, value) in url::form_urlencoded::parse(value.as_bytes()) {
        if key.len() > 128
            || value.len() > 8192
            || key.chars().chain(value.chars()).any(char::is_control)
            || result
                .insert(key.into_owned(), value.into_owned())
                .is_some()
        {
            return Err(Error::oauth(400, "invalid_request"));
        }
    }
    Ok(result)
}
fn field<'a>(values: &'a Fields, key: &str) -> Result<&'a str> {
    values
        .get(key)
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::oauth(400, "invalid_request"))
}
fn encoded(values: &Fields) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(values)
        .finish()
}

#[derive(Serialize, Deserialize)]
struct Pending {
    query: Fields,
    user_id: String,
    client_id: String,
    redirect_uri: String,
    resource: String,
    scope: String,
    challenge: String,
    service_id: String,
    service_generation: i64,
    grant_generation: i64,
    refresh_allowed: bool,
}

async fn authorize(hub: Shared, request: Request<Body>) -> Result<Response> {
    hub.throttle("oauth:authorize", 240)?;
    let query = fields(request.uri().query().unwrap_or(""))?;
    let id = field(&query, "client_id")?;
    let client = client(&hub, id).await?;
    let redirect = field(&query, "redirect_uri")?;
    if !redirect_matches(&client, redirect) {
        return Err(Error::oauth(400, "invalid_redirect_uri"));
    }
    if field(&query, "response_type")? != "code"
        || query.get("response_mode").is_some_and(|s| s != "query")
    {
        return Err(Error::oauth(400, "unsupported_response_type"));
    }
    let challenge = field(&query, "code_challenge")?;
    if query.get("code_challenge_method").map(String::as_str) != Some("S256")
        || challenge.len() != 43
        || URL_SAFE_NO_PAD
            .decode(challenge)
            .map_or(true, |v| v.len() != 32)
    {
        return Err(Error::oauth(400, "invalid_request"));
    }
    let scope = scopes(query.get("scope").map(String::as_str).unwrap_or("mcp:use"))?;
    if scope
        .split(' ')
        .any(|s| !client.scope.split(' ').any(|allowed| s == allowed))
    {
        return Err(Error::oauth(400, "invalid_scope"));
    }
    let resource = field(&query, "resource")?;
    let user = match identity::session(&hub, request.headers()) {
        Ok(user) => user,
        Err(error) if error.status == StatusCode::UNAUTHORIZED => {
            let mut url = Url::parse(&format!("{}/sign-in", hub.config.base_url))
                .map_err(|_| Error::new(500, "Invalid URL"))?;
            url.query_pairs_mut()
                .append_pair("returnTo", &format!("{AUTH}authorize?{}", encoded(&query)));
            return redirect_response(url.as_str());
        }
        Err(error) => return Err(error),
    };
    let binding = identity::session_binding(&hub, request.headers())?;
    let db = hub.db.lock().unwrap();
    let service =
        db::resource(&db, resource)?.ok_or_else(|| Error::oauth(400, "invalid_target"))?;
    let grant = db::grant(&db, &service, &user.id)?
        .filter(|_| service.auth_mode != "bearer")
        .ok_or_else(|| Error::oauth(403, "access_denied"))?;
    let pending = Pending {
        query: query.clone(),
        user_id: user.id,
        client_id: id.into(),
        redirect_uri: redirect.into(),
        resource: resource.into(),
        scope,
        challenge: challenge.into(),
        service_id: service.id,
        service_generation: service.generation,
        grant_generation: grant,
        refresh_allowed: client.grant_types.iter().any(|g| g == "refresh_token"),
    };
    let key = token("wop_");
    db.execute(
        "INSERT INTO oauth_pending(id,user_id,session_hash,request,expires) VALUES(?,?,?,?,?)",
        params![
            digest(&key),
            pending.user_id,
            binding,
            serde_json::to_string(&pending).map_err(|_| Error::new(500, "Invalid request"))?,
            now() + 600
        ],
    )?;
    let mut consent = query;
    consent.insert("_request".into(), key);
    redirect_response(&format!(
        "{}/consent?{}",
        hub.config.base_url,
        encoded(&consent)
    ))
}

fn redirect_response(target: &str) -> Result<Response> {
    let value = HeaderValue::from_str(target).map_err(|_| Error::oauth(400, "invalid_request"))?;
    let mut response = StatusCode::FOUND.into_response();
    response.headers_mut().insert("location", value);
    Ok(response)
}
fn callback(hub: &Shared, pending: &Pending, name: &str, value: &str) -> Result<String> {
    let mut url =
        Url::parse(&pending.redirect_uri).map_err(|_| Error::oauth(400, "invalid_redirect_uri"))?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair(name, value)
            .append_pair("iss", &hub.config.base_url);
        if let Some(state) = pending.query.get("state") {
            query.append_pair("state", state);
        }
    }
    Ok(url.into())
}

async fn consent(hub: Shared, request: Request<Body>) -> Result<Response> {
    same_origin(&hub, request.headers())?;
    let user = identity::session(&hub, request.headers())?;
    let binding = identity::session_binding(&hub, request.headers())?;
    hub.throttle(&format!("oauth:consent:{}", user.id), 60)?;
    let (_, body) = body_json(request).await?;
    let accept = body
        .get("accept")
        .and_then(Value::as_bool)
        .ok_or_else(|| Error::oauth(400, "invalid_request"))?;
    let mut query = fields(
        body.get("oauth_query")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::oauth(400, "invalid_request"))?,
    )?;
    let key = query
        .remove("_request")
        .ok_or_else(|| Error::oauth(400, "invalid_request"))?;
    let db = hub.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let stored:Option<String>=tx.query_row("SELECT request FROM oauth_pending WHERE id=? AND user_id=? AND session_hash=? AND expires>? AND consumed=0",params![digest(&key),user.id,binding,now()],|r|r.get(0)).optional()?;
    let pending: Pending =
        serde_json::from_str(&stored.ok_or_else(|| Error::oauth(400, "invalid_request"))?)
            .map_err(|_| Error::new(500, "Invalid request"))?;
    if pending.query != query || pending.user_id != user.id {
        return Err(Error::oauth(400, "invalid_request"));
    }
    tx.execute(
        "UPDATE oauth_pending SET consumed=1 WHERE id=?",
        [digest(&key)],
    )?;
    if !accept {
        let url = callback(&hub, &pending, "error", "access_denied")?;
        tx.commit()?;
        return Ok(json(json!({"url":url})));
    }
    let service =
        db::resource(&tx, &pending.resource)?.ok_or_else(|| Error::oauth(403, "access_denied"))?;
    if service.id != pending.service_id
        || service.generation != pending.service_generation
        || service.auth_mode == "bearer"
        || db::grant(&tx, &service, &user.id)? != Some(pending.grant_generation)
    {
        return Err(Error::oauth(403, "access_denied"));
    }
    let existing:Option<(String,i64,i64)>=tx.query_row("SELECT id,generation,active FROM command_oauth_connections WHERE user_id=? AND service_id=? AND client_id=?",params![user.id,service.id,pending.client_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let (connection, generation) = match existing {
        Some((id, generation, 1)) => (id, generation),
        Some((id, generation, _)) => {
            tx.execute(
                "UPDATE command_oauth_connections SET active=1,generation=generation+1 WHERE id=?",
                [&id],
            )?;
            (id, generation + 1)
        }
        None => {
            let id = token("wcn_");
            tx.execute("INSERT INTO command_oauth_connections(id,user_id,service_id,client_id) VALUES(?,?,?,?)",params![id,user.id,service.id,pending.client_id])?;
            (id, 1)
        }
    };
    let code = token("wcode_");
    // Whether refresh was supported is recorded in the granted scope itself.
    let granted_scope = if pending.refresh_allowed {
        pending.scope.clone()
    } else {
        "mcp:use".into()
    };
    tx.execute("INSERT INTO oauth_codes(token_hash,user_id,client_id,redirect_uri,resource,scope,challenge,expires,service_id,service_generation,grant_generation,connection_id,connection_generation) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",params![digest(&code),user.id,pending.client_id,pending.redirect_uri,pending.resource,granted_scope,pending.challenge,now()+60,service.id,service.generation,pending.grant_generation,connection,generation])?;
    let url = callback(&hub, &pending, "code", &code)?;
    tx.commit()?;
    Ok(json(json!({"url":url})))
}

#[derive(Clone)]
struct Credential {
    user: String,
    client: String,
    resource: String,
    scope: String,
    expires: i64,
    service: String,
    service_generation: i64,
    grant_generation: i64,
    connection: String,
    connection_generation: i64,
    family: String,
    invalid: i64,
}
fn credential(row: &rusqlite::Row<'_>) -> rusqlite::Result<Credential> {
    Ok(Credential {
        user: row.get(0)?,
        client: row.get(1)?,
        resource: row.get(2)?,
        scope: row.get(3)?,
        expires: row.get(4)?,
        service: row.get(5)?,
        service_generation: row.get(6)?,
        grant_generation: row.get(7)?,
        connection: row.get(8)?,
        connection_generation: row.get(9)?,
        family: row.get(10)?,
        invalid: row.get(11)?,
    })
}
const TOKEN_COLUMNS: &str = "user_id,client_id,resource,scope,expires,service_id,service_generation,grant_generation,connection_id,connection_generation,family_id";
fn live(db: &Connection, value: &Credential, service: &db::Service) -> Result<bool> {
    if value.invalid != 0
        || value.expires <= now()
        || value.service != service.id
        || value.resource != service.resource
        || value.service_generation != service.generation
        || service.auth_mode == "bearer"
        || db::grant(db, service, &value.user)? != Some(value.grant_generation)
    {
        return Ok(false);
    }
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM command_oauth_connections c JOIN users u ON u.id=c.user_id WHERE c.id=? AND c.user_id=? AND c.service_id=? AND c.client_id=? AND c.generation=? AND c.active=1)",params![value.connection,value.user,value.service,value.client,value.connection_generation],|r|r.get(0))?;
    Ok(valid)
}

pub fn authorize_token(db: &Connection, raw: &str, service: &db::Service) -> Result<String> {
    let value: Option<Credential> = db
        .query_row(
            &format!("SELECT {TOKEN_COLUMNS},revoked FROM oauth_access_tokens WHERE token_hash=?"),
            [digest(raw)],
            credential,
        )
        .optional()?;
    if let Some(value) = value
        && live(db, &value, service)?
    {
        return Ok(value.user);
    }
    Err(Error::new(403, "Service access denied"))
}

pub fn revoke_connection_tokens(db: &Connection, id: &str) -> Result<()> {
    db.execute(
        "UPDATE oauth_access_tokens SET revoked=1 WHERE connection_id=?",
        [id],
    )?;
    db.execute(
        "UPDATE oauth_refresh_tokens SET revoked=1 WHERE connection_id=?",
        [id],
    )?;
    Ok(())
}
fn revoke_generation(db: &Connection, value: &Credential) -> Result<()> {
    // A replay of an old grant must not revoke a newly approved connection.
    let changed=db.execute("UPDATE command_oauth_connections SET active=0,generation=generation+1 WHERE id=? AND generation=? AND active=1",params![value.connection,value.connection_generation])?;
    if changed > 0 {
        revoke_connection_tokens(db, &value.connection)?;
    }
    Ok(())
}

fn assertion_key(document: &Value, kid: Option<&str>) -> Result<Option<jsonwebtoken::DecodingKey>> {
    let keys = document
        .get("keys")
        .and_then(Value::as_array)
        .filter(|keys| !keys.is_empty() && keys.len() <= 32)
        .ok_or_else(|| Error::oauth(401, "invalid_client"))?;
    let matching: Vec<_> = keys
        .iter()
        .filter(|key| {
            key.get("kty").and_then(Value::as_str) == Some("RSA")
                && key
                    .get("alg")
                    .and_then(Value::as_str)
                    .is_none_or(|alg| alg == "RS256")
                && key
                    .get("use")
                    .and_then(Value::as_str)
                    .is_none_or(|usage| usage == "sig")
                && key.get("key_ops").is_none_or(|ops| {
                    ops.as_array()
                        .is_some_and(|ops| ops.iter().any(|op| op.as_str() == Some("verify")))
                })
                && kid.is_none_or(|id| key.get("kid").and_then(Value::as_str) == Some(id))
        })
        .collect();
    if matching.is_empty() {
        return Ok(None);
    }
    if matching.len() != 1 {
        return Err(Error::oauth(401, "invalid_client"));
    }
    let key = matching[0];
    let modulus = key
        .get("n")
        .and_then(Value::as_str)
        .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
        .filter(|n| (256..=1024).contains(&n.len()))
        .ok_or_else(|| Error::oauth(401, "invalid_client"))?;
    if modulus[0] == 0 || modulus.len() == 256 && modulus[0] & 0x80 == 0 {
        return Err(Error::oauth(401, "invalid_client"));
    }
    let exponent = key
        .get("e")
        .and_then(Value::as_str)
        .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
        .filter(|e| !e.is_empty() && e.len() <= 8)
        .ok_or_else(|| Error::oauth(401, "invalid_client"))?;
    if exponent[0] == 0 || exponent.last().unwrap() & 1 == 0 || exponent == [1] {
        return Err(Error::oauth(401, "invalid_client"));
    }
    let jwk: jsonwebtoken::jwk::Jwk =
        serde_json::from_value(key.clone()).map_err(|_| Error::oauth(401, "invalid_client"))?;
    jsonwebtoken::DecodingKey::from_jwk(&jwk)
        .map(Some)
        .map_err(|_| Error::oauth(401, "invalid_client"))
}

async fn client_key(
    hub: &Shared,
    client: &Client,
    kid: Option<&str>,
) -> Result<jsonwebtoken::DecodingKey> {
    let uri = client
        .jwks_uri
        .as_deref()
        .ok_or_else(|| Error::oauth(401, "invalid_client"))?;
    let cached: Option<(String, String, i64)> = hub
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT uri,document,expires FROM oauth_jwks WHERE client_id=?",
            [&client.client_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((cached_uri, document, expires)) = cached
        && cached_uri == uri
        && expires > now()
    {
        let document: Value =
            serde_json::from_str(&document).map_err(|_| Error::oauth(401, "invalid_client"))?;
        if let Some(key) = assertion_key(&document, kid)? {
            return Ok(key);
        }
    }
    // Unknown key IDs can trigger refresh, but not unbounded external fetches.
    hub.throttle("oauth:jwks", 60)
        .map_err(|_| Error::oauth(401, "invalid_client"))?;
    hub.throttle(&format!("oauth:jwks:{}", client.client_id), 2)
        .map_err(|_| Error::oauth(401, "invalid_client"))?;
    let document = remote_value(
        remote_response(uri)
            .await
            .map_err(|_| Error::oauth(401, "invalid_client"))?,
    )
    .await
    .map_err(|_| Error::oauth(401, "invalid_client"))?;
    let key = assertion_key(&document, kid)?.ok_or_else(|| Error::oauth(401, "invalid_client"))?;
    hub.db.lock().unwrap().execute("INSERT INTO oauth_jwks(client_id,uri,document,expires) VALUES(?,?,?,?) ON CONFLICT(client_id) DO UPDATE SET uri=excluded.uri,document=excluded.document,expires=excluded.expires",params![client.client_id,uri,document.to_string(),now()+300])?;
    Ok(key)
}

#[derive(Deserialize)]
struct AssertionClaims {
    iss: String,
    sub: String,
    exp: i64,
    jti: String,
    #[serde(default)]
    iat: Option<i64>,
    #[serde(default)]
    nbf: Option<i64>,
}

async fn authenticate_client(
    hub: &Shared,
    client: &Client,
    values: &Fields,
    endpoint: &str,
) -> Result<()> {
    if client.token_endpoint_auth_method == "none" {
        if values.contains_key("client_assertion") || values.contains_key("client_assertion_type") {
            return Err(Error::oauth(401, "invalid_client"));
        }
        return Ok(());
    }
    if client.token_endpoint_auth_method != "private_key_jwt"
        || values.get("client_assertion_type").map(String::as_str)
            != Some("urn:ietf:params:oauth:client-assertion-type:jwt-bearer")
    {
        return Err(Error::oauth(401, "invalid_client"));
    }
    let assertion = values
        .get("client_assertion")
        .filter(|s| s.len() <= 8192 && !s.is_empty())
        .ok_or_else(|| Error::oauth(401, "invalid_client"))?;
    let header =
        jsonwebtoken::decode_header(assertion).map_err(|_| Error::oauth(401, "invalid_client"))?;
    if header.alg != jsonwebtoken::Algorithm::RS256
        || header
            .crit
            .as_ref()
            .is_some_and(|fields| !fields.is_empty())
        || header
            .extras
            .get::<bool>("b64")
            .map_or(true, |value| value == Some(false))
        || header.jku.is_some()
        || header.jwk.is_some()
        || header.x5u.is_some()
        || header
            .kid
            .as_ref()
            .is_some_and(|kid| kid.len() > 256 || kid.chars().any(char::is_control))
    {
        return Err(Error::oauth(401, "invalid_client"));
    }
    let key = client_key(hub, client, header.kid.as_deref()).await?;
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.leeway = 30;
    validation.validate_nbf = true;
    validation.set_required_spec_claims(&["iss", "sub", "aud", "exp"]);
    validation.set_issuer(&[&client.client_id]);
    validation.sub = Some(client.client_id.clone());
    // RFC 7523 permits the authorization-server identifier or token URL as aud.
    validation.set_audience(&[
        hub.config.base_url.clone(),
        format!("{}{AUTH}token", hub.config.base_url),
        format!("{}{}", hub.config.base_url, endpoint),
    ]);
    let claims = jsonwebtoken::decode::<AssertionClaims>(assertion, &key, &validation)
        .map_err(|_| Error::oauth(401, "invalid_client"))?
        .claims;
    let current = now();
    if claims.iss != client.client_id
        || claims.sub != client.client_id
        || claims.exp <= current
        || claims.exp > current + 330
        || claims
            .iat
            .is_some_and(|iat| iat > current + 30 || iat < current - 330 || claims.exp - iat > 300)
        || claims.nbf.is_some_and(|nbf| nbf > current + 30)
        || claims.jti.is_empty()
        || claims.jti.len() > 256
        || claims.jti.chars().any(char::is_control)
    {
        return Err(Error::oauth(401, "invalid_client"));
    }
    let key = digest(&format!("{}\n{}", client.client_id, claims.jti));
    let changed = hub.db.lock().unwrap().execute(
        "INSERT OR IGNORE INTO oauth_assertions(jti_hash,expires) VALUES(?,?)",
        params![key, claims.exp],
    )?;
    if changed != 1 {
        return Err(Error::oauth(401, "invalid_client"));
    }
    Ok(())
}

async fn form(request: Request<Body>) -> Result<Fields> {
    if request.headers().contains_key("authorization")
        || !request
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|s| s.starts_with("application/x-www-form-urlencoded"))
    {
        return Err(Error::oauth(400, "invalid_request"));
    }
    let bytes = axum::body::to_bytes(request.into_body(), 32768)
        .await
        .map_err(|_| Error::oauth(400, "invalid_request"))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Error::oauth(400, "invalid_request"))?;
    let value = fields(text)?;
    if value.get("client_secret").is_some_and(|s| !s.is_empty()) {
        return Err(Error::oauth(401, "invalid_client"));
    }
    Ok(value)
}

async fn exchange(hub: Shared, request: Request<Body>) -> Result<Response> {
    hub.throttle("oauth:token", 1200)?;
    let fields = form(request).await?;
    let client_id = field(&fields, "client_id")?;
    let client = client(&hub, client_id).await?;
    authenticate_client(&hub, &client, &fields, "/api/auth/oauth2/token").await?;
    let resource = field(&fields, "resource")?;
    let db = hub.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    let response = match field(&fields, "grant_type")? {
        "authorization_code" => {
            let code = field(&fields, "code")?;
            let redirect = field(&fields, "redirect_uri")?;
            let verifier = field(&fields, "code_verifier")?;
            let row:Option<(Credential,String,String,i64)>=tx.query_row("SELECT user_id,client_id,resource,scope,expires,service_id,service_generation,grant_generation,connection_id,connection_generation,token_hash,0,redirect_uri,challenge,consumed FROM oauth_codes WHERE token_hash=?",[digest(code)],|r|Ok((credential(r)?,r.get(12)?,r.get(13)?,r.get(14)?))).optional()?;
            let (value, registered_redirect, challenge, consumed) =
                row.ok_or_else(|| Error::oauth(400, "invalid_grant"))?;
            if value.expires <= now() {
                return Err(Error::oauth(400, "invalid_grant"));
            }
            let verifier_valid = (43..=128).contains(&verifier.len())
                && verifier
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b));
            if value.client != client_id
                || value.resource != resource
                || registered_redirect != redirect
                || !redirect_matches(&client, redirect)
                || !verifier_valid
                || URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != challenge
            {
                return Err(Error::oauth(400, "invalid_grant"));
            }
            if consumed != 0 {
                revoke_generation(&tx, &value)?;
                tx.commit()?;
                return Err(Error::oauth(400, "invalid_grant"));
            }
            let service =
                db::resource(&tx, resource)?.ok_or_else(|| Error::oauth(400, "invalid_grant"))?;
            if !live(&tx, &value, &service)? {
                return Err(Error::oauth(400, "invalid_grant"));
            }
            tx.execute(
                "UPDATE oauth_codes SET consumed=1 WHERE token_hash=?",
                [digest(code)],
            )?;
            issue(&tx, &value, now() + REFRESH_LIFETIME)?
        }
        "refresh_token" => {
            if !client.grant_types.iter().any(|g| g == "refresh_token") {
                return Err(Error::oauth(400, "unauthorized_client"));
            }
            let raw = field(&fields, "refresh_token")?;
            let row:Option<(Credential,i64)>=tx.query_row(&format!("SELECT {TOKEN_COLUMNS},revoked,used FROM oauth_refresh_tokens WHERE token_hash=?"),[digest(raw)],|r|Ok((credential(r)?,r.get(12)?))).optional()?;
            let (mut value, used) = row.ok_or_else(|| Error::oauth(400, "invalid_grant"))?;
            if value.expires <= now() {
                return Err(Error::oauth(400, "invalid_grant"));
            }
            if value.client != client_id || value.resource != resource {
                return Err(Error::oauth(400, "invalid_grant"));
            }
            if let Some(scope) = fields.get("scope") {
                let narrowed = scopes(scope)?;
                if narrowed
                    .split(' ')
                    .any(|s| !value.scope.split(' ').any(|old| s == old))
                {
                    return Err(Error::oauth(400, "invalid_scope"));
                }
                value.scope = narrowed;
            }
            if used != 0 {
                revoke_generation(&tx, &value)?;
                tx.commit()?;
                return Err(Error::oauth(400, "invalid_grant"));
            }
            let service =
                db::resource(&tx, resource)?.ok_or_else(|| Error::oauth(400, "invalid_grant"))?;
            if !live(&tx, &value, &service)? {
                return Err(Error::oauth(400, "invalid_grant"));
            }
            tx.execute(
                "UPDATE oauth_refresh_tokens SET used=1 WHERE token_hash=?",
                [digest(raw)],
            )?;
            issue(&tx, &value, value.expires)?
        }
        _ => return Err(Error::oauth(400, "unsupported_grant_type")),
    };
    tx.commit()?;
    Ok(json(response))
}

fn issue(db: &Connection, value: &Credential, refresh_expiry: i64) -> Result<Value> {
    let access = token("woa_");
    db.execute("INSERT INTO oauth_access_tokens(token_hash,user_id,client_id,resource,scope,expires,service_id,service_generation,grant_generation,connection_id,connection_generation,family_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",params![digest(&access),value.user,value.client,value.resource,value.scope,now()+ACCESS_LIFETIME,value.service,value.service_generation,value.grant_generation,value.connection,value.connection_generation,value.family])?;
    let mut response = json!({"access_token":access,"token_type":"Bearer","expires_in":ACCESS_LIFETIME,"scope":value.scope});
    if value.scope.split(' ').any(|s| s == "offline_access") {
        let refresh = token("wor_");
        db.execute("INSERT INTO oauth_refresh_tokens(token_hash,user_id,client_id,resource,scope,expires,service_id,service_generation,grant_generation,connection_id,connection_generation,family_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",params![digest(&refresh),value.user,value.client,value.resource,value.scope,refresh_expiry,value.service,value.service_generation,value.grant_generation,value.connection,value.connection_generation,value.family])?;
        response["refresh_token"] = json!(refresh);
    }
    Ok(response)
}

async fn revoke(hub: Shared, request: Request<Body>) -> Result<Response> {
    hub.throttle("oauth:revoke", 120)?;
    let fields = form(request).await?;
    let id = field(&fields, "client_id")?;
    let client = client(&hub, id).await?;
    authenticate_client(&hub, &client, &fields, "/api/auth/oauth2/revoke").await?;
    let raw = field(&fields, "token")?;
    let db = hub.db.lock().unwrap();
    let tx = db.unchecked_transaction()?;
    for table in ["oauth_access_tokens", "oauth_refresh_tokens"] {
        let value: Option<Credential> = tx
            .query_row(
                &format!("SELECT {TOKEN_COLUMNS},revoked FROM {table} WHERE token_hash=?"),
                [digest(raw)],
                credential,
            )
            .optional()?;
        if let Some(value) = value {
            if value.client != id {
                return Err(Error::oauth(400, "invalid_grant"));
            }
            if value.expires > now() {
                revoke_generation(&tx, &value)?;
            }
            break;
        }
    }
    tx.commit()?;
    Ok(json(json!({})))
}

#[cfg(test)]
mod unit {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn fixture_response(
        body: Vec<u8>,
        content_type: &str,
        chunked: bool,
        status: u16,
    ) -> reqwest::Response {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let content_type = content_type.to_owned();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while request.len() < 8192 && !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                if socket.read_exact(&mut byte).await.is_err() {
                    return;
                }
                request.push(byte[0]);
            }
            let framing = if chunked {
                "Transfer-Encoding: chunked".into()
            } else {
                format!("Content-Length: {}", body.len())
            };
            let header = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Type: {content_type}\r\n{framing}\r\nConnection: close\r\n\r\n"
            );
            if socket.write_all(header.as_bytes()).await.is_err() {
                return;
            }
            let bytes = if chunked {
                [
                    format!("{:x}\r\n", body.len()).as_bytes(),
                    &body,
                    b"\r\n0\r\n\r\n",
                ]
                .concat()
            } else {
                body
            };
            let _ = socket.write_all(&bytes).await;
        });
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
            .get(format!("http://{address}/metadata.json"))
            .send()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn cimd_document_network_body_checks_identity_metadata_and_both_size_framings() {
        let id = "https://client.example/metadata.json";
        let valid = json!({"client_id":id,"client_name":"Fixture","redirect_uris":["http://127.0.0.1:9000/callback"],"token_endpoint_auth_method":"none","scope":"mcp:use offline_access","grant_types":["authorization_code","refresh_token","urn:ietf:params:oauth:grant-type:jwt-bearer"],"response_types":["code","id_token"]});
        let response = fixture_response(
            valid.to_string().into_bytes(),
            "application/json; charset=utf-8",
            true,
            200,
        )
        .await;
        let client = document(response, id).await.unwrap();
        assert_eq!(client.client_id, id);
        assert_eq!(client.grant_types, ["authorization_code", "refresh_token"]);
        assert_eq!(client.response_types, ["code"]);
        assert!(
            metadata(&valid, id.into(), false).is_err(),
            "DCR remains strict even though global CIMD capabilities negotiate"
        );
        let mut private = valid.clone();
        private["token_endpoint_auth_method"] = json!("private_key_jwt");
        private["jwks_uri"] = json!("https://client.example/");
        private["token_endpoint_auth_signing_alg"] = json!("RS256");
        private["token_endpoint_auth_methods_supported"] = json!(["none", "private_key_jwt"]);
        let parsed = document(
            fixture_response(
                private.to_string().into_bytes(),
                "application/json",
                false,
                200,
            )
            .await,
            id,
        )
        .await
        .unwrap();
        assert_eq!(parsed.token_endpoint_auth_method, "private_key_jwt");
        assert_eq!(parsed.jwks_uri.as_deref(), Some("https://client.example/"));
        assert_eq!(client.redirect_uris, ["http://127.0.0.1:9000/callback"]);
        let mut mismatch = valid.clone();
        mismatch["client_id"] = json!("https://other.example/metadata.json");
        assert!(
            document(
                fixture_response(
                    mismatch.to_string().into_bytes(),
                    "application/json",
                    false,
                    200
                )
                .await,
                id
            )
            .await
            .is_err()
        );
        let mut malformed = valid.clone();
        malformed["redirect_uris"] = json!(["https://client.example/callback#bad"]);
        assert!(
            document(
                fixture_response(
                    malformed.to_string().into_bytes(),
                    "application/json",
                    false,
                    200
                )
                .await,
                id
            )
            .await
            .is_err()
        );
        for chunked in [false, true] {
            assert!(
                document(
                    fixture_response(vec![b'x'; 32769], "application/json", chunked, 200).await,
                    id
                )
                .await
                .is_err()
            );
        }
        assert!(
            document(
                fixture_response(valid.to_string().into_bytes(), "text/html", false, 200).await,
                id
            )
            .await
            .is_err()
        );
        assert!(
            document(
                fixture_response(
                    valid.to_string().into_bytes(),
                    "application/json",
                    false,
                    302
                )
                .await,
                id
            )
            .await
            .is_err()
        );
    }

    #[test]
    fn cleanup_is_bounded_and_preserves_unexpired_refresh_replay_evidence() {
        let state = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let hub = crate::Hub::open(crate::Config {
            base_url: "http://127.0.0.1:43210".into(),
            state_dir: state.path().into(),
            public_domain: "wispnet.test".into(),
            github_client_id: String::new(),
            github_client_secret: String::new(),
            test_mode: true,
        })
        .unwrap();
        {
            let db = hub.db.lock().unwrap();
            for i in 0..2050 {
                db.execute("INSERT INTO oauth_pending(id,user_id,session_hash,request,expires) VALUES(?,'user','hash','{}',?)",params![format!("expired-{i}"),now()-1]).unwrap();
            }
            db.execute("INSERT INTO oauth_pending(id,user_id,session_hash,request,expires) VALUES('valid','user','hash','{}',?)",[now()+100]).unwrap();
            db.execute("INSERT INTO oauth_refresh_tokens(token_hash,user_id,client_id,resource,scope,expires,service_id,service_generation,grant_generation,connection_id,connection_generation,family_id,used) VALUES('replay','user','client','resource','mcp:use',?,'service',1,0,'connection',1,'family',1)",[now()+100]).unwrap();
        }
        cleanup(&hub).unwrap();
        cleanup(&hub).unwrap();
        let db = hub.db.lock().unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM oauth_pending", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM oauth_refresh_tokens WHERE token_hash='replay'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn redirect_and_metadata_fetch_addresses_are_bounded() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "169.254.169.254",
            "100.64.1.2",
            "192.0.0.1",
            "198.18.1.2",
            "::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
            "fe80::1",
            "fc00::1",
        ] {
            assert!(!public_address(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_address("8.8.8.8".parse().unwrap()));
        assert!(public_address("2606:4700:4700::1111".parse().unwrap()));
        assert!(!valid_redirect(
            "https://example.test/callback#fragment",
            false
        ));
        assert!(!valid_redirect("http://example.test/callback", true));
        assert!(!valid_redirect(
            "https://example.test/callback?code=conflict",
            false
        ));
        assert!(valid_redirect("http://127.0.0.1:1234/callback", true));
        assert!(fields("resource=one&resource=two").is_err());
        let native = metadata(
            &json!({"redirect_uris":["http://127.0.0.1:9000/callback"]}),
            "fixture".into(),
            false,
        )
        .unwrap();
        assert!(redirect_matches(&native, "http://127.0.0.1:53321/callback"));
        assert!(!redirect_matches(
            &native,
            "http://127.0.0.2:53321/callback"
        ));
        assert!(!redirect_matches(&native, "http://127.0.0.1:53321/other"));
        let web = metadata(
            &json!({"redirect_uris":["https://client.example/callback"]}),
            "fixture".into(),
            false,
        )
        .unwrap();
        assert!(!redirect_matches(
            &web,
            "https://client.example:444/callback"
        ));
    }

    #[test]
    fn rsa_client_keys_are_public_signing_keys_with_bounded_parameters() {
        let mut n = vec![0x80; 256];
        n[255] = 1;
        let key = json!({"kty":"RSA","kid":"key","alg":"RS256","use":"sig","key_ops":["verify"],"n":URL_SAFE_NO_PAD.encode(n),"e":"AQAB"});
        assert!(
            assertion_key(&json!({"keys":[key.clone()]}), Some("key"))
                .unwrap()
                .is_some()
        );
        assert!(
            assertion_key(&json!({"keys":[key.clone()]}), Some("unknown"))
                .unwrap()
                .is_none()
        );
        for (field, value) in [
            ("alg", json!("HS256")),
            ("use", json!("enc")),
            ("key_ops", json!(["encrypt"])),
        ] {
            let mut wrong = key.clone();
            wrong[field] = value;
            assert!(
                assertion_key(&json!({"keys":[wrong]}), Some("key"))
                    .unwrap()
                    .is_none()
            );
        }
        for bytes in [vec![0x80; 255], vec![0x80; 1025], vec![0x40; 256]] {
            let mut wrong = key.clone();
            wrong["n"] = json!(URL_SAFE_NO_PAD.encode(bytes));
            assert!(assertion_key(&json!({"keys":[wrong]}), Some("key")).is_err());
        }
        let mut wrong = key.clone();
        wrong["e"] = json!(URL_SAFE_NO_PAD.encode([0x80; 9]));
        assert!(assertion_key(&json!({"keys":[wrong]}), Some("key")).is_err());
        assert!(assertion_key(&json!({"keys":[key.clone(),key]}), Some("key")).is_err());
    }
}
