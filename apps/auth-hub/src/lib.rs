//! Optional account and public authorization server. Private Wispnet never calls it.
mod command;
pub mod config;
pub mod db;
pub mod identity;
pub mod oauth;
mod server;
pub mod ui;

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
pub use config::Config;
use rand::Rng;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

pub type Shared = Arc<Hub>;
pub type Result<T> = std::result::Result<T, Error>;
pub struct Hub {
    pub config: Config,
    pub db: Mutex<Connection>,
    pub http: reqwest::Client,
    limits: Mutex<HashMap<String, Vec<i64>>>,
    requests: tokio::sync::Semaphore,
}
#[derive(Debug)]
pub struct Error {
    pub status: StatusCode,
    pub message: String,
    pub oauth: bool,
}
impl Error {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            message: message.into(),
            oauth: false,
        }
    }
    pub fn oauth(status: u16, code: &str) -> Self {
        Self {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            message: code.into(),
            oauth: true,
        }
    }
}
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::new(500, "Command center storage failed")
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let message = if self.status.is_server_error() {
            "Command center request failed".to_owned()
        } else {
            self.message
        };
        (self.status, Json(json!({"error":message}))).into_response()
    }
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub fn token(prefix: &str) -> String {
    let mut bytes = [0; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}
pub fn json(value: Value) -> Response {
    Json(value).into_response()
}
pub fn required<'a>(body: &'a Value, key: &str, max: usize) -> Result<&'a str> {
    body[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control))
        .ok_or_else(|| Error::new(400, "Invalid request"))
}
pub fn same_origin(hub: &Hub, headers: &HeaderMap) -> Result<()> {
    if headers.get("origin").and_then(|h| h.to_str().ok()) != Some(hub.config.base_url.as_str())
        || !headers
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|s| s.starts_with("application/json"))
    {
        return Err(Error::new(403, "Same-origin JSON request required"));
    }
    Ok(())
}
pub async fn body_json(request: Request<Body>) -> Result<(HeaderMap, Value)> {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 65536)
        .await
        .map_err(|_| Error::new(413, "Request too large"))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| Error::new(400, "A JSON object is required"))?;
    if !value.is_object() {
        return Err(Error::new(400, "A JSON object is required"));
    }
    Ok((parts.headers, value))
}
impl Hub {
    pub fn open(config: Config) -> Result<Shared> {
        let config = config.validate()?;
        let db = db::open(&config.state_dir)?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(5))
            .user_agent("Wispnet-Command-Center")
            .build()
            .map_err(|_| Error::new(500, "HTTP client failed"))?;
        Ok(Arc::new(Self {
            config,
            db: Mutex::new(db),
            http,
            limits: Mutex::new(HashMap::new()),
            requests: tokio::sync::Semaphore::new(128),
        }))
    }
    pub fn router(self: &Arc<Self>) -> Router {
        Router::new().fallback(dispatch).with_state(self.clone())
    }
    pub async fn serve(self: Arc<Self>, listener: tokio::net::TcpListener) -> std::io::Result<()> {
        self.serve_until(listener, std::future::pending()).await
    }
    pub async fn serve_until<F: std::future::Future<Output = ()>>(
        self: Arc<Self>,
        listener: tokio::net::TcpListener,
        shutdown: F,
    ) -> std::io::Result<()> {
        server::serve_until(self, listener, shutdown).await
    }
    pub fn throttle(&self, key: &str, max: usize) -> Result<()> {
        let mut limits = self.limits.lock().unwrap();
        let cutoff = now() - 60;
        if limits.len() > 4096 {
            limits.retain(|_, v| v.iter().any(|t| *t > cutoff));
            if limits.len() > 4096 && !limits.contains_key(key) {
                return Err(Error::new(429, "Try again later"));
            }
        }
        let rows = limits.entry(key.into()).or_default();
        rows.retain(|t| *t > cutoff);
        if rows.len() >= max {
            return Err(Error::new(429, "Try again later"));
        }
        rows.push(now());
        Ok(())
    }
}
async fn dispatch(State(hub): State<Shared>, request: Request<Body>) -> Response {
    let permit = hub.requests.try_acquire();
    let result = if permit.is_err() {
        Err(Error::new(503, "Command center busy"))
    } else {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            dispatch_inner(hub.clone(), request),
        )
        .await
        .unwrap_or_else(|_| Err(Error::new(408, "Request timed out")))
    };
    let mut response = result.unwrap_or_else(IntoResponse::into_response);
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' https://avatars.githubusercontent.com; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    ] {
        response
            .headers_mut()
            .insert(name, axum::http::HeaderValue::from_static(value));
    }
    response
}
async fn dispatch_inner(hub: Shared, request: Request<Body>) -> Result<Response> {
    if request.headers().get_all("authorization").iter().count() > 1 {
        return Err(Error::new(400, "Invalid request"));
    }
    let (parts, body) = request.into_parts();
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        axum::body::to_bytes(body, 65536),
    )
    .await
    .map_err(|_| Error::new(408, "Request body timed out"))?
    .map_err(|_| Error::new(413, "Request too large"))?;
    let request = Request::from_parts(parts, Body::from(bytes));
    let path = request.uri().path();
    if identity::handles(path) {
        identity::handle(hub, request).await
    } else if oauth::handles(path) {
        oauth::handle(hub, request).await
    } else {
        command::handle(hub, request).await
    }
}
