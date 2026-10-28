//! GitHub-only human identity and opaque browser sessions.
use crate::{
    Error, Hub, Result, Shared, body_json, digest, json, now, required, same_origin, token,
};
use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Method, Request, header},
    response::{IntoResponse, Redirect, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const SESSION_COOKIE: &str = "wispnet_session";
const LOGIN_COOKIE: &str = "wispnet_github";
const SESSION_SECONDS: i64 = 7 * 24 * 60 * 60;
const LOGIN_SECONDS: i64 = 600;
const MAX_PROVIDER_RESPONSE: usize = 65536;

#[derive(Clone, Debug, Serialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub email: String,
}

struct GitHubEndpoints {
    authorize: String,
    token: String,
    profile: String,
    emails: String,
}
impl GitHubEndpoints {
    fn production() -> Self {
        Self {
            authorize: "https://github.com/login/oauth/authorize".into(),
            token: "https://github.com/login/oauth/access_token".into(),
            profile: "https://api.github.com/user".into(),
            emails: "https://api.github.com/user/emails".into(),
        }
    }
}

pub fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS identity_login_states (
            state_hash TEXT PRIMARY KEY, browser_hash TEXT NOT NULL,
            verifier TEXT NOT NULL, callback_url TEXT NOT NULL, expires INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

pub fn handles(path: &str) -> bool {
    matches!(
        path,
        "/api/auth/sign-in/social"
            | "/api/auth/callback/github"
            | "/api/auth/sign-out"
            | "/api/auth/get-session"
    )
}

// Reject duplicate same-name cookies rather than accepting a shadowed value.
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        let header = header.to_str().ok()?;
        for item in header.split(';') {
            let Some((key, value)) = item.trim().split_once('=') else {
                continue;
            };
            if key != name {
                continue;
            }
            if found.is_some()
                || !(32..=128).contains(&value.len())
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return None;
            }
            found = Some(value.to_owned());
        }
    }
    found
}

fn cookie_name(hub: &Hub, name: &str) -> String {
    if hub.config.test_mode {
        name.into()
    } else {
        format!("__Host-{name}")
    }
}

fn set_cookie(hub: &Hub, name: &str, value: &str, max_age: i64) -> HeaderValue {
    // Production validation requires HTTPS. Local tests can use loopback HTTP.
    let secure = if hub.config.base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    let name = cookie_name(hub, name);
    HeaderValue::from_str(&format!(
        "{name}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}{secure}"
    ))
    .expect("cookie names and values are generated ASCII")
}

pub fn session(hub: &Hub, headers: &HeaderMap) -> Result<User> {
    let session = cookie(headers, &cookie_name(hub, SESSION_COOKIE))
        .ok_or_else(|| Error::new(401, "Sign in with GitHub first"))?;
    hub.db
        .lock()
        .unwrap()
        .query_row(
            "SELECT u.id,u.name,u.email FROM sessions s JOIN users u ON u.id=s.user_id
            WHERE s.token_hash=? AND s.expires>?",
            params![digest(&session), now()],
            |row| {
                Ok(User {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    email: row.get(2)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| Error::new(401, "Sign in with GitHub first"))
}

/// A consent request binds to the exact browser credential as well as its user.
/// Callers authenticate with `session` before relying on this binding.
pub fn session_binding(hub: &Hub, headers: &HeaderMap) -> Result<String> {
    cookie(headers, &cookie_name(hub, SESSION_COOKIE))
        .map(|value| digest(&value))
        .ok_or_else(|| Error::new(401, "Sign in with GitHub first"))
}

pub async fn handle(hub: Shared, request: Request<Body>) -> Result<Response> {
    handle_with_endpoints(hub, request, &GitHubEndpoints::production()).await
}

async fn handle_with_endpoints(
    hub: Shared,
    request: Request<Body>,
    endpoints: &GitHubEndpoints,
) -> Result<Response> {
    match (request.method(), request.uri().path()) {
        (&Method::POST, "/api/auth/sign-in/social") => start(hub, request, endpoints).await,
        (&Method::GET, "/api/auth/callback/github") => {
            let mut response = callback(hub.clone(), request, endpoints)
                .await
                .unwrap_or_else(IntoResponse::into_response);
            response
                .headers_mut()
                .append(header::SET_COOKIE, set_cookie(&hub, LOGIN_COOKIE, "", 0));
            Ok(response)
        }
        (&Method::GET, "/api/auth/get-session") => {
            Ok(json(json!({"user":session(&hub,request.headers())?})))
        }
        (&Method::POST, "/api/auth/sign-out") => {
            same_origin(&hub, request.headers())?;
            if let Some(value) = cookie(request.headers(), &cookie_name(&hub, SESSION_COOKIE)) {
                hub.db
                    .lock()
                    .unwrap()
                    .execute("DELETE FROM sessions WHERE token_hash=?", [digest(&value)])?;
            }
            let mut response = json(json!({"success":true}));
            response
                .headers_mut()
                .append(header::SET_COOKIE, set_cookie(&hub, SESSION_COOKIE, "", 0));
            Ok(response)
        }
        _ => Err(Error::new(405, "Method not allowed")),
    }
}

fn callback_destination(hub: &Hub, value: &str) -> Result<String> {
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(Error::new(400, "Invalid callback URL"));
    }
    let parsed = url::Url::parse(value).map_err(|_| Error::new(400, "Invalid callback URL"))?;
    let base = url::Url::parse(&hub.config.base_url)
        .map_err(|_| Error::new(500, "Invalid server origin"))?;
    if parsed.origin() != base.origin()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.path(), "/" | "/sign-in" | "/connect" | "/consent")
    {
        return Err(Error::new(400, "Invalid callback URL"));
    }
    Ok(parsed.into())
}

async fn start(
    hub: Shared,
    request: Request<Body>,
    endpoints: &GitHubEndpoints,
) -> Result<Response> {
    same_origin(&hub, request.headers())?;
    hub.throttle("identity-start", 60)?;
    let (_, body) = body_json(request).await?;
    if required(&body, "provider", 32)? != "github" {
        return Err(Error::new(400, "Only GitHub login is supported"));
    }
    let callback = callback_destination(&hub, required(&body, "callbackURL", 4096)?)?;
    let state = token("wgs_");
    let browser = token("wgb_");
    let verifier = token("");
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    {
        let db = hub.db.lock().unwrap();
        db.execute(
            "DELETE FROM identity_login_states WHERE expires<=?",
            [now()],
        )?;
        db.execute("DELETE FROM sessions WHERE expires<=?", [now()])?;
        db.execute("INSERT INTO identity_login_states(state_hash,browser_hash,verifier,callback_url,expires) VALUES(?,?,?,?,?)",
            params![digest(&state),digest(&browser),verifier,callback,now()+LOGIN_SECONDS])?;
    }
    let mut authorize = url::Url::parse(&endpoints.authorize)
        .map_err(|_| Error::new(500, "GitHub login is unavailable"))?;
    authorize.query_pairs_mut().extend_pairs([
        ("client_id", hub.config.github_client_id.as_str()),
        (
            "redirect_uri",
            &format!("{}/api/auth/callback/github", hub.config.base_url),
        ),
        ("scope", "read:user user:email"),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    let mut response = json(json!({"url":authorize.as_str(),"redirect":true}));
    response.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(&hub, LOGIN_COOKIE, &browser, LOGIN_SECONDS),
    );
    Ok(response)
}

async fn provider_json(request: reqwest::RequestBuilder) -> Result<Value> {
    let mut response = request
        .send()
        .await
        .map_err(|_| Error::new(502, "GitHub login is unavailable"))?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|n| n > MAX_PROVIDER_RESPONSE as u64)
    {
        return Err(Error::new(502, "GitHub login is unavailable"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Error::new(502, "GitHub login is unavailable"))?
    {
        if body.len() + chunk.len() > MAX_PROVIDER_RESPONSE {
            return Err(Error::new(502, "GitHub login response was too large"));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| Error::new(502, "Invalid GitHub login response"))
}

async fn callback(
    hub: Shared,
    request: Request<Body>,
    endpoints: &GitHubEndpoints,
) -> Result<Response> {
    let mut query = HashMap::new();
    for (key, value) in url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
    {
        if query.insert(key.to_string(), value.to_string()).is_some() {
            return Err(Error::new(400, "Invalid GitHub callback"));
        }
    }
    let state = query
        .get("state")
        .filter(|s| s.len() <= 128)
        .ok_or_else(|| Error::new(400, "Invalid GitHub login state"))?;
    let browser = cookie(request.headers(), &cookie_name(&hub, LOGIN_COOKIE))
        .ok_or_else(|| Error::new(400, "GitHub login must finish in the same browser"))?;
    let record: Option<(String, String)> = {
        let db = hub.db.lock().unwrap();
        db.query_row("DELETE FROM identity_login_states WHERE state_hash=? AND browser_hash=? AND expires>? RETURNING callback_url,verifier",
            params![digest(state),digest(&browser),now()],|row|Ok((row.get(0)?,row.get(1)?))).optional()?
    };
    let (destination, verifier) =
        record.ok_or_else(|| Error::new(400, "GitHub login expired or already completed"))?;
    if query.contains_key("error") {
        return Err(Error::new(401, "GitHub login was declined"));
    }
    let code = query
        .get("code")
        .filter(|s| !s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control))
        .ok_or_else(|| Error::new(400, "Invalid GitHub authorization code"))?;
    let redirect = format!("{}/api/auth/callback/github", hub.config.base_url);
    let access = provider_json(
        hub.http
            .post(&endpoints.token)
            .header("accept", "application/json")
            .form(&[
                ("client_id", hub.config.github_client_id.as_str()),
                ("client_secret", hub.config.github_client_secret.as_str()),
                ("code", code.as_str()),
                ("redirect_uri", redirect.as_str()),
                ("code_verifier", verifier.as_str()),
            ]),
    )
    .await?;
    let access_token = access["access_token"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control))
        .ok_or_else(|| Error::new(401, "GitHub login was not authorized"))?;
    if !access["token_type"]
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("bearer"))
    {
        return Err(Error::new(502, "Invalid GitHub login response"));
    }
    let profile = provider_json(
        hub.http
            .get(&endpoints.profile)
            .bearer_auth(access_token)
            .header("accept", "application/vnd.github+json"),
    )
    .await?;
    let github_id = profile["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or_else(|| Error::new(502, "Invalid GitHub identity"))?
        .to_string();
    let login = profile["login"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 100 && !s.chars().any(char::is_control))
        .ok_or_else(|| Error::new(502, "Invalid GitHub identity"))?;
    let name = profile["name"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 160 && !s.chars().any(char::is_control))
        .unwrap_or(login);
    // Email is display metadata only. Stable GitHub ID is the sole linking key.
    let emails = provider_json(
        hub.http
            .get(&endpoints.emails)
            .bearer_auth(access_token)
            .header("accept", "application/vnd.github+json"),
    )
    .await?;
    let emails = emails
        .as_array()
        .ok_or_else(|| Error::new(502, "Invalid GitHub email response"))?;
    let verified = |entry: &&Value| {
        entry["verified"] == true
            && entry["email"].as_str().is_some_and(|s| {
                !s.is_empty() && s.len() <= 320 && !s.chars().any(char::is_control)
            })
    };
    let email = emails
        .iter()
        .find(|entry| verified(entry) && entry["primary"] == true)
        .or_else(|| emails.iter().find(verified))
        .and_then(|entry| entry["email"].as_str())
        .unwrap_or("");
    let session_token = token("wss_");
    {
        let mut db = hub.db.lock().unwrap();
        let tx = db.transaction()?;
        let new_id = token("usr_");
        tx.execute("INSERT INTO users(id,github_id,name,email) VALUES(?,?,?,?) ON CONFLICT(github_id) DO UPDATE SET name=excluded.name,email=excluded.email",
            params![new_id,github_id,name,email])?;
        let user_id: String = tx.query_row(
            "SELECT id FROM users WHERE github_id=?",
            [github_id],
            |row| row.get(0),
        )?;
        if let Some(previous) = cookie(request.headers(), &cookie_name(&hub, SESSION_COOKIE)) {
            tx.execute(
                "DELETE FROM sessions WHERE token_hash=?",
                [digest(&previous)],
            )?;
        }
        tx.execute(
            "INSERT INTO sessions(token_hash,user_id,expires) VALUES(?,?,?)",
            params![digest(&session_token), user_id, now() + SESSION_SECONDS],
        )?;
        tx.commit()?;
    }
    let mut response = Redirect::to(&destination).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(&hub, SESSION_COOKIE, &session_token, SESSION_SECONDS),
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::{Form, State},
        routing::{get, post},
    };
    use std::{
        os::unix::fs::PermissionsExt,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        },
    };

    #[derive(Clone)]
    struct Provider {
        id: Arc<AtomicU64>,
        calls: Arc<AtomicUsize>,
        challenge: Arc<Mutex<String>>,
        oversized: Arc<AtomicBool>,
    }
    struct Fixture {
        _state: tempfile::TempDir,
        hub: Shared,
        endpoints: GitHubEndpoints,
        provider: Provider,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    impl Fixture {
        async fn new() -> Self {
            let state = tempfile::tempdir().unwrap();
            std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            let hub = Hub::open(crate::Config {
                base_url: "https://cmd.wispnet.test".into(),
                state_dir: state.path().into(),
                public_domain: "wispnet.test".into(),
                github_client_id: "fixture-client".into(),
                github_client_secret: "fixture-github-secret".into(),
                test_mode: true,
            })
            .unwrap();
            let provider = Provider {
                id: Arc::new(AtomicU64::new(42)),
                calls: Arc::new(AtomicUsize::new(0)),
                challenge: Arc::new(Mutex::new(String::new())),
                oversized: Arc::new(AtomicBool::new(false)),
            };
            async fn exchange(
                State(provider): State<Provider>,
                Form(body): Form<HashMap<String, String>>,
            ) -> Json<Value> {
                provider.calls.fetch_add(1, Ordering::SeqCst);
                assert_eq!(body["client_id"], "fixture-client");
                assert_eq!(body["client_secret"], "fixture-github-secret");
                assert_eq!(
                    body["redirect_uri"],
                    "https://cmd.wispnet.test/api/auth/callback/github"
                );
                assert_eq!(body["code"], "fixture-authorization-code");
                assert_eq!(
                    URL_SAFE_NO_PAD.encode(Sha256::digest(body["code_verifier"].as_bytes())),
                    *provider.challenge.lock().unwrap()
                );
                Json(json!({"access_token":"fixture-github-access-token","token_type":"bearer"}))
            }
            async fn profile(State(provider): State<Provider>, headers: HeaderMap) -> Json<Value> {
                assert_eq!(
                    headers.get("authorization").unwrap(),
                    "Bearer fixture-github-access-token"
                );
                if provider.oversized.load(Ordering::SeqCst) {
                    return Json(json!({"huge":"x".repeat(MAX_PROVIDER_RESPONSE+1)}));
                }
                Json(
                    json!({"id":provider.id.load(Ordering::SeqCst),"login":"alice","name":"Alice <script>","email":"unverified@example.test"}),
                )
            }
            async fn emails(headers: HeaderMap) -> Json<Value> {
                assert_eq!(
                    headers.get("authorization").unwrap(),
                    "Bearer fixture-github-access-token"
                );
                Json(
                    json!([{"email":"unverified@example.test","primary":true,"verified":false},{"email":"verified@example.test","primary":true,"verified":true}]),
                )
            }
            let router = Router::new()
                .route("/token", post(exchange))
                .route("/profile", get(profile))
                .route("/emails", get(emails))
                .with_state(provider.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let task = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            Self {
                _state: state,
                hub,
                endpoints: GitHubEndpoints {
                    authorize: format!("{origin}/authorize"),
                    token: format!("{origin}/token"),
                    profile: format!("{origin}/profile"),
                    emails: format!("{origin}/emails"),
                },
                provider,
                task,
            }
        }
        fn request(&self, path: &str, body: Option<Value>, cookie: Option<&str>) -> Request<Body> {
            let mut request = Request::builder()
                .method(if body.is_some() { "POST" } else { "GET" })
                .uri(path)
                .header("origin", &self.hub.config.base_url)
                .header("content-type", "application/json");
            if let Some(cookie) = cookie {
                request = request.header("cookie", cookie);
            }
            request
                .body(
                    body.map(|value| Body::from(value.to_string()))
                        .unwrap_or_else(Body::empty),
                )
                .unwrap()
        }
        async fn start(&self) -> (String, String) {
            let response=handle_with_endpoints(self.hub.clone(),self.request("/api/auth/sign-in/social",Some(json!({"provider":"github","callbackURL":"https://cmd.wispnet.test/sign-in?returnTo=%2Fapi%2Fauth%2Foauth2%2Fauthorize%3Fclient_id%3Dclient"})),None),&self.endpoints).await.unwrap();
            let cookie = response.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_owned();
            assert!(
                response.headers()[header::SET_COOKIE]
                    .to_str()
                    .unwrap()
                    .contains("HttpOnly; SameSite=Lax; Path=/")
            );
            assert!(
                response.headers()[header::SET_COOKIE]
                    .to_str()
                    .unwrap()
                    .ends_with("; Secure")
            );
            let body: Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 65536)
                    .await
                    .unwrap(),
            )
            .unwrap();
            let uri = url::Url::parse(body["url"].as_str().unwrap()).unwrap();
            let query: HashMap<_, _> = uri.query_pairs().into_owned().collect();
            assert_eq!(query["scope"], "read:user user:email");
            assert_eq!(query["code_challenge_method"], "S256");
            *self.provider.challenge.lock().unwrap() = query["code_challenge"].clone();
            (
                format!(
                    "/api/auth/callback/github?state={}&code=fixture-authorization-code",
                    query["state"]
                ),
                cookie,
            )
        }
        async fn login(&self, old_session: Option<&str>) -> String {
            let (path, cookie) = self.start().await;
            let cookie = old_session
                .map(|old| format!("{cookie}; {old}"))
                .unwrap_or(cookie);
            let response = handle_with_endpoints(
                self.hub.clone(),
                self.request(&path, None, Some(&cookie)),
                &self.endpoints,
            )
            .await
            .unwrap();
            assert_eq!(response.status(), 303);
            assert_eq!(
                response.headers()[header::LOCATION],
                "https://cmd.wispnet.test/sign-in?returnTo=%2Fapi%2Fauth%2Foauth2%2Fauthorize%3Fclient_id%3Dclient"
            );
            response
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .find(|v| v.starts_with("wispnet_session="))
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_owned()
        }
        fn user(&self, value: &str) -> User {
            let mut headers = HeaderMap::new();
            headers.insert("cookie", HeaderValue::from_str(value).unwrap());
            session(&self.hub, &headers).unwrap()
        }
    }

    #[tokio::test]
    async fn github_pkce_browser_binding_one_use_state_and_hashed_sessions() {
        let f = Fixture::new().await;
        let (path, cookie) = f.start().await;
        let denied =
            handle_with_endpoints(f.hub.clone(), f.request(&path, None, None), &f.endpoints)
                .await
                .unwrap();
        assert_eq!(denied.status(), 400);
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
        let response = handle_with_endpoints(
            f.hub.clone(),
            f.request(&path, None, Some(&cookie)),
            &f.endpoints,
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 303);
        let session_cookie = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with("wispnet_session="))
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let user = f.user(session_cookie);
        assert_eq!(user.name, "Alice <script>");
        assert_eq!(user.email, "verified@example.test");
        let replay = handle_with_endpoints(
            f.hub.clone(),
            f.request(&path, None, Some(&cookie)),
            &f.endpoints,
        )
        .await
        .unwrap();
        assert_eq!(replay.status(), 400);
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
        let db = f.hub.db.lock().unwrap();
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM identity_login_states", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        let stored: String = db
            .query_row("SELECT token_hash FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stored, digest(session_cookie.split_once('=').unwrap().1));
        assert!(!stored.contains(session_cookie.split_once('=').unwrap().1));
    }

    #[tokio::test]
    async fn github_id_is_stable_email_never_links_accounts_and_login_rotates_session() {
        let f = Fixture::new().await;
        let first = f.login(None).await;
        let identity = f.user(&first).id;
        let rotated = f.login(Some(&first)).await;
        assert_eq!(identity, f.user(&rotated).id);
        let mut headers = HeaderMap::new();
        headers.insert("cookie", HeaderValue::from_str(&first).unwrap());
        assert_eq!(session(&f.hub, &headers).unwrap_err().status, 401);
        f.provider.id.store(43, Ordering::SeqCst);
        let other = f.login(None).await;
        assert_ne!(identity, f.user(&other).id);
        assert_eq!(
            f.hub
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM users WHERE email='verified@example.test'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn expired_state_oversized_provider_response_and_duplicate_cookies_fail_closed() {
        let f = Fixture::new().await;
        let (path, cookie) = f.start().await;
        f.hub
            .db
            .lock()
            .unwrap()
            .execute("UPDATE identity_login_states SET expires=?", [now() - 1])
            .unwrap();
        assert_eq!(
            handle_with_endpoints(
                f.hub.clone(),
                f.request(&path, None, Some(&cookie)),
                &f.endpoints
            )
            .await
            .unwrap()
            .status(),
            400
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
        let (path, cookie) = f.start().await;
        f.provider.oversized.store(true, Ordering::SeqCst);
        assert_eq!(
            handle_with_endpoints(
                f.hub.clone(),
                f.request(&path, None, Some(&cookie)),
                &f.endpoints
            )
            .await
            .unwrap()
            .status(),
            502
        );
        assert_eq!(
            f.hub
                .db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM sessions", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let headers = HeaderMap::from_iter([(
            header::COOKIE,
            HeaderValue::from_str(&format!("{cookie}; {cookie}")).unwrap(),
        )]);
        assert!(super::cookie(&headers, LOGIN_COOKIE).is_none());
    }

    #[tokio::test]
    async fn expired_sessions_deny_access_and_login_prunes_expired_identity_records() {
        let f = Fixture::new().await;
        let cookie = f.login(None).await;
        f.hub
            .db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET expires=?", [now() - 1])
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("cookie", HeaderValue::from_str(&cookie).unwrap());
        assert_eq!(session(&f.hub, &headers).unwrap_err().status, 401);
        let (expired, browser) = f.start().await;
        f.hub
            .db
            .lock()
            .unwrap()
            .execute("UPDATE identity_login_states SET expires=?", [now() - 1])
            .unwrap();
        f.start().await;
        {
            let db = f.hub.db.lock().unwrap();
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM sessions", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM identity_login_states", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
                1
            );
        }
        assert_eq!(
            handle_with_endpoints(
                f.hub.clone(),
                f.request(&expired, None, Some(&browser)),
                &f.endpoints
            )
            .await
            .unwrap()
            .status(),
            400
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn login_callbacks_stay_on_command_pages_and_signout_checks_origin() {
        let f = Fixture::new().await;
        for callback in [
            "https://evil.test/",
            "https://cmd.wispnet.test.evil.test/",
            "https://cmd.wispnet.test/api/command/authorize",
            "https://user@cmd.wispnet.test/",
            "https://cmd.wispnet.test/#fragment",
        ] {
            assert_eq!(
                handle_with_endpoints(
                    f.hub.clone(),
                    f.request(
                        "/api/auth/sign-in/social",
                        Some(json!({"provider":"github","callbackURL":callback})),
                        None
                    ),
                    &f.endpoints
                )
                .await
                .unwrap_err()
                .status,
                400
            );
        }
        let cookie = f.login(None).await;
        let mut request = f.request("/api/auth/sign-out", Some(json!({})), Some(&cookie));
        request
            .headers_mut()
            .insert("origin", HeaderValue::from_static("https://evil.test"));
        assert_eq!(
            handle(f.hub.clone(), request).await.unwrap_err().status,
            403
        );
        f.user(&cookie);
        let response = handle(
            f.hub.clone(),
            f.request("/api/auth/sign-out", Some(json!({})), Some(&cookie)),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 200);
        assert!(
            response.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
        let mut headers = HeaderMap::new();
        headers.insert("cookie", HeaderValue::from_str(&cookie).unwrap());
        assert_eq!(session(&f.hub, &headers).unwrap_err().status, 401);
    }

    #[tokio::test]
    async fn production_cookies_use_host_prefix_against_sibling_public_services() {
        let mut f = Fixture::new().await;
        let mut config = f.hub.config.clone();
        config.test_mode = false;
        f.hub = Hub::open(config).unwrap();
        let result = handle_with_endpoints(
            f.hub.clone(),
            f.request(
                "/api/auth/sign-in/social",
                Some(json!({"provider":"github","callbackURL":"https://cmd.wispnet.test/"})),
                None,
            ),
            &f.endpoints,
        )
        .await
        .unwrap();
        let cookie = result.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.starts_with("__Host-wispnet_github="));
        assert!(cookie.ends_with("; Secure"));
        assert!(!cookie.contains("Domain="));
        let unprefixed = token("wss_");
        let db = f.hub.db.lock().unwrap();
        db.execute(
            "INSERT INTO users(id,github_id,name,email) VALUES('user','9','User','')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO sessions(token_hash,user_id,expires) VALUES(?,'user',?)",
            params![digest(&unprefixed), now() + 100],
        )
        .unwrap();
        drop(db);
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            HeaderValue::from_str(&format!("wispnet_session={unprefixed}")).unwrap(),
        );
        assert_eq!(session(&f.hub, &headers).unwrap_err().status, 401);
        headers.insert(
            "cookie",
            HeaderValue::from_str(&format!("__Host-wispnet_session={unprefixed}")).unwrap(),
        );
        assert_eq!(session(&f.hub, &headers).unwrap().id, "user");
    }
}
