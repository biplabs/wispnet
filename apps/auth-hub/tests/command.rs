use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use rusqlite::params;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use tower::ServiceExt;
use wispnet_command_center::{Config, Hub, Shared, digest, now, token};
use wispnet_core::identity::SecretKey;

struct Fixture {
    dir: tempfile::TempDir,
    hub: Shared,
}
struct Actor {
    id: String,
    cookie: String,
}
struct Host {
    id: String,
    publisher: String,
    key: SecretKey,
    start: Value,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let hub = Hub::open(Config {
            base_url: "http://127.0.0.1:43210".into(),
            state_dir: dir.path().into(),
            public_domain: "wispnet.test".into(),
            github_client_id: String::new(),
            github_client_secret: String::new(),
            test_mode: true,
        })
        .unwrap();
        Self { dir, hub }
    }
    fn actor(&self, name: &str) -> Actor {
        let id = token("");
        let session = token("");
        let db = self.hub.db.lock().unwrap();
        db.execute(
            "INSERT INTO users(id,github_id,name,email) VALUES(?,?,?,?)",
            params![id, id, name, format!("{name}@example.test")],
        )
        .unwrap();
        db.execute(
            "INSERT INTO sessions(token_hash,user_id,expires) VALUES(?,?,?)",
            params![digest(&session), id, now() + 600],
        )
        .unwrap();
        Actor {
            id,
            cookie: format!("wispnet_session={session}"),
        }
    }
    async fn request(
        &self,
        path: &str,
        body: Option<Value>,
        actor: Option<&Actor>,
        publisher: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(if body.is_some() { "POST" } else { "GET" })
            .uri(path)
            .header("origin", &self.hub.config.base_url);
        if let Some(actor) = actor {
            request = request.header("cookie", &actor.cookie);
        }
        if let Some(token) = publisher {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let request = if let Some(body) = body {
            request
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        } else {
            request.body(Body::empty()).unwrap()
        };
        let response = self.hub.router().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 100_000).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    async fn host(&self, actor: &Actor, key: Option<SecretKey>) -> Host {
        let key = key.unwrap_or_else(SecretKey::generate);
        let id = key.public().to_string();
        let nonce = digest(&token(""));
        let name = "j-desktop";
        let proof = format!(
            "wispnet-command-link-v1\n{}\n{id}\n{name}\n{nonce}",
            self.hub.config.base_url
        );
        let signature: String = key
            .sign(proof.as_bytes())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let start = json!({"host":id,"name":name,"nonce":nonce,"signature":signature});
        let (status, codes) = self
            .request("/api/command/links/start", Some(start.clone()), None, None)
            .await;
        assert_eq!(status, 200, "{codes}");
        let (status, pending) = self
            .request(
                "/api/command/links/poll",
                Some(json!({"device_code":codes["device_code"]})),
                None,
                None,
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(pending["state"], "pending");
        assert_eq!(
            self.request(
                "/api/command/links/approve",
                Some(json!({"user_code":codes["user_code"]})),
                Some(actor),
                None
            )
            .await
            .0,
            200
        );
        let (status, linked) = self
            .request(
                "/api/command/links/poll",
                Some(json!({"device_code":codes["device_code"]})),
                None,
                None,
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(
            self.request(
                "/api/command/links/poll",
                Some(json!({"device_code":codes["device_code"]})),
                None,
                None
            )
            .await
            .0,
            404
        );
        Host {
            id,
            publisher: linked["publisher_token"].as_str().unwrap().into(),
            key,
            start,
        }
    }
    async fn service(&self, host: &Host, id: &str) -> Value {
        let resource = format!("https://j-desktop-{}-{id}.wispnet.test/MCP", &host.id[..12]);
        let body = json!({"host":host.id,"service":id,"incarnation":digest(&token("")),"name":"j-desktop","resource":resource});
        let (status, registered) = self
            .request(
                "/api/command/host/register",
                Some(body.clone()),
                None,
                Some(&host.publisher),
            )
            .await;
        assert_eq!(status, 200, "{registered}");
        let mut body = body;
        body["id"] = registered["resource_id"].clone();
        body
    }
    async fn key(&self, user: &Actor, service: &Value) -> Value {
        let (status, key) = self
            .request(
                "/api/command/keys/create",
                Some(json!({"service_id":service["id"],"name":"Agent fixture"})),
                Some(user),
                None,
            )
            .await;
        assert_eq!(status, 200, "{key}");
        key
    }
    async fn authorize(&self, host: &Host, service: &Value, key: &Value) -> StatusCode {
        let mut body = service.clone();
        body["token"] = key["key"].clone();
        self.request(
            "/api/command/authorize",
            Some(body),
            None,
            Some(&host.publisher),
        )
        .await
        .0
    }
}

#[tokio::test]
async fn signed_host_linking_and_publisher_resource_binding_preserve_contract() {
    let f = Fixture::new();
    let owner = f.actor("Owner");
    let alice = f.actor("Alice");
    let host = f.host(&owner, None).await;
    assert_eq!(
        f.request(
            "/api/command/links/start",
            Some(host.start.clone()),
            None,
            None
        )
        .await
        .0,
        409
    );
    let mut tampered = host.start.clone();
    tampered["nonce"] = json!(digest(&token("")));
    assert_eq!(
        f.request("/api/command/links/start", Some(tampered), None, None)
            .await
            .0,
        403
    );
    let service = f.service(&host, "gike-mcp").await;
    let mut invalid = service.clone();
    invalid["resource"] = json!("https://attacker.test/MCP");
    assert_eq!(
        f.request(
            "/api/command/host/register",
            Some(invalid),
            None,
            Some(&host.publisher)
        )
        .await
        .0,
        400
    );
    assert_eq!(
        f.request(
            "/api/command/host/register",
            Some(service.clone()),
            None,
            None
        )
        .await
        .0,
        403
    );
    assert_eq!(
        f.request(
            "/api/command/hosts/unlink",
            Some(json!({"host":host.id})),
            Some(&alice),
            None
        )
        .await
        .0,
        403
    );
    let (_, dashboard) = f
        .request("/api/command/dashboard", None, Some(&owner), None)
        .await;
    assert!(!dashboard.to_string().contains(&host.publisher));
    assert_eq!(dashboard["services"][0]["resource"], service["resource"]);
    assert_eq!(
        f.request("/api/command/dashboard", None, None, None)
            .await
            .0,
        401
    );
    let stored = std::fs::read(f.dir.path().join("command.sqlite")).unwrap();
    assert!(
        !stored
            .windows(host.publisher.len())
            .any(|s| s == host.publisher.as_bytes())
    );
}

#[tokio::test]
async fn bearer_keys_are_service_scoped_and_never_revive_after_regrant_or_relink() {
    let f = Fixture::new();
    let owner = f.actor("Owner");
    let alice = f.actor("Alice");
    let host = f.host(&owner, None).await;
    let service = f.service(&host, "gike-mcp").await;
    let other = f.service(&host, "other").await;
    assert_eq!(
        f.request(
            "/api/command/keys/create",
            Some(json!({"service_id":service["id"],"name":"Denied"})),
            Some(&alice),
            None
        )
        .await
        .0,
        403
    );
    let grant = json!({"service_id":service["id"],"user_id":alice.id,"active":true});
    assert_eq!(
        f.request(
            "/api/command/grants",
            Some(grant.clone()),
            Some(&owner),
            None
        )
        .await
        .0,
        200
    );
    let key = f.key(&alice, &service).await;
    assert_eq!(f.authorize(&host, &service, &key).await, 200);
    assert_eq!(f.authorize(&host, &other, &key).await, 403);
    let (_, dashboard) = f
        .request("/api/command/dashboard", None, Some(&alice), None)
        .await;
    assert!(!dashboard.to_string().contains(key["key"].as_str().unwrap()));
    let mut revoked = grant.clone();
    revoked["active"] = json!(false);
    assert_eq!(
        f.request("/api/command/grants", Some(revoked), Some(&owner), None)
            .await
            .0,
        200
    );
    assert_eq!(f.authorize(&host, &service, &key).await, 403);
    assert_eq!(
        f.request("/api/command/grants", Some(grant), Some(&owner), None)
            .await
            .0,
        200
    );
    assert_eq!(f.authorize(&host, &service, &key).await, 403);
    let fresh = f.key(&alice, &service).await;
    assert_eq!(f.authorize(&host, &service, &fresh).await, 200);
    let owner_key = f.key(&owner, &service).await;
    assert_eq!(
        f.request(
            "/api/command/host/unlink",
            Some(json!({"host":host.id})),
            None,
            Some(&host.publisher)
        )
        .await
        .0,
        200
    );
    let host = f.host(&owner, Some(host.key)).await;
    let (_, dashboard) = f
        .request("/api/command/dashboard", None, Some(&owner), None)
        .await;
    assert_eq!(
        dashboard["services"],
        json!([]),
        "relinking must not present disabled services as published"
    );
    let service = f.service(&host, "gike-mcp").await;
    assert_eq!(f.authorize(&host, &service, &owner_key).await, 403);
    assert_eq!(f.authorize(&host, &service, &fresh).await, 403);
}

#[tokio::test]
async fn owner_key_revocation_mode_changes_and_service_replacement_stop_credentials() {
    let f = Fixture::new();
    let owner = f.actor("Owner");
    let host = f.host(&owner, None).await;
    let service = f.service(&host, "gike-mcp").await;
    let key = f.key(&owner, &service).await;
    assert_eq!(f.authorize(&host, &service, &key).await, 200);
    let revoke = json!({"key_id":key["id"]});
    for _ in 0..2 {
        assert_eq!(
            f.request(
                "/api/command/keys/revoke",
                Some(revoke.clone()),
                Some(&owner),
                None
            )
            .await
            .0,
            200
        );
    }
    assert_eq!(f.authorize(&host, &service, &key).await, 403);
    let fresh = f.key(&owner, &service).await;
    assert_eq!(
        f.request(
            "/api/command/services/mode",
            Some(json!({"service_id":service["id"],"auth_mode":"oauth"})),
            Some(&owner),
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(f.authorize(&host, &service, &fresh).await, 403);
    assert_eq!(
        f.request(
            "/api/command/services/mode",
            Some(json!({"service_id":service["id"],"auth_mode":"both"})),
            Some(&owner),
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(f.authorize(&host, &service, &fresh).await, 403);
    let new = f.key(&owner, &service).await;
    let replaced = f.service(&host, "gike-mcp").await;
    assert_eq!(f.authorize(&host, &replaced, &new).await, 403);
}

#[tokio::test]
async fn malformed_cross_origin_duplicate_auth_and_oversized_requests_are_denied() {
    let f = Fixture::new();
    let owner = f.actor("Owner");
    for (headers, body, expected) in [
        (
            vec![
                ("origin", "https://evil.test"),
                ("content-type", "application/json"),
            ],
            "{}".into(),
            403,
        ),
        (
            vec![
                ("origin", f.hub.config.base_url.as_str()),
                ("content-type", "application/json"),
            ],
            "[]".into(),
            400,
        ),
        (
            vec![
                ("authorization", "Bearer first"),
                ("authorization", "Bearer second"),
            ],
            "{}".into(),
            400,
        ),
        (vec![], "x".repeat(65537), 413),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/command/keys/create")
            .header("cookie", &owner.cookie);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = f
            .hub
            .router()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    assert_eq!(
        f.request(
            "/api/auth/sign-up/email",
            Some(json!({"email":"no@example.test","password":"password"})),
            None,
            None
        )
        .await
        .0,
        404
    );
}

#[test]
fn production_requires_github_and_database_permissions_reject_legacy_or_linked_state() {
    let f = Fixture::new();
    let mut config = f.hub.config.clone();
    config.test_mode = false;
    config.base_url = "https://cmd.wispnet.app".into();
    assert!(config.validate().is_err());
    let linked = f.dir.path().join("linked");
    std::os::unix::fs::symlink(f.dir.path(), &linked).unwrap();
    let mut config = f.hub.config.clone();
    config.state_dir = linked;
    assert!(Hub::open(config).is_err());
    let legacy = tempfile::tempdir().unwrap();
    std::fs::set_permissions(legacy.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = legacy.path().join("command.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE session(id TEXT)").unwrap();
    drop(db);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut config = f.hub.config.clone();
    config.state_dir = legacy.path().into();
    assert!(Hub::open(config).is_err());
}

#[tokio::test]
async fn unlink_invalidates_previously_approved_pending_host_links() {
    let f = Fixture::new();
    let owner = f.actor("Owner");
    let host = f.host(&owner, None).await;
    let nonce = digest(&token(""));
    let proof = format!(
        "wispnet-command-link-v1\n{}\n{}\nj-desktop\n{nonce}",
        f.hub.config.base_url, host.id
    );
    let signature: String = host
        .key
        .sign(proof.as_bytes())
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let (status, codes) = f
        .request(
            "/api/command/links/start",
            Some(json!({"host":host.id,"name":"j-desktop","nonce":nonce,"signature":signature})),
            None,
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        f.request(
            "/api/command/links/approve",
            Some(json!({"user_code":codes["user_code"]})),
            Some(&owner),
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(
        f.request(
            "/api/command/hosts/unlink",
            Some(json!({"host":host.id})),
            Some(&owner),
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(
        f.request(
            "/api/command/links/poll",
            Some(json!({"device_code":codes["device_code"]})),
            None,
            None
        )
        .await
        .0,
        404
    );
    let fresh = f.host(&owner, Some(host.key)).await;
    assert_ne!(fresh.publisher, host.publisher);
}

#[test]
fn partial_legacy_and_newer_databases_fail_without_schema_rewrites() {
    for (schema, version) in [
        ("CREATE TABLE command_keys(id TEXT)", 0),
        ("CREATE TABLE users(id TEXT)", 2),
        ("CREATE TABLE users(id TEXT)", 1),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("command.sqlite");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(schema).unwrap();
        db.pragma_update(None, "user_version", version).unwrap();
        drop(db);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let fixture = Fixture::new();
        let mut config = fixture.hub.config.clone();
        config.state_dir = dir.path().into();
        assert!(Hub::open(config).is_err());
        let db = rusqlite::Connection::open(path).unwrap();
        let actual: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(actual, version);
        let tables: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 1);
    }
}

#[tokio::test(start_paused = true)]
async fn stalled_bodies_time_out_and_inflight_limits_recover() {
    fn stalled() -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/command/links/start")
            .body(Body::from_stream(futures_util::stream::pending::<
                std::result::Result<axum::body::Bytes, std::convert::Infallible>,
            >()))
            .unwrap()
    }
    let fixture = Fixture::new();
    let mut tasks = Vec::new();
    for _ in 0..128 {
        let router = fixture.hub.router();
        tasks.push(tokio::spawn(async move {
            router.oneshot(stalled()).await.unwrap().status()
        }));
    }
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    let busy = fixture
        .hub
        .router()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(busy.status(), 503);
    tokio::time::advance(std::time::Duration::from_secs(11)).await;
    for task in tasks {
        assert_eq!(task.await.unwrap(), 408);
    }
    assert_eq!(fixture.request("/healthz", None, None, None).await.0, 200);
}
