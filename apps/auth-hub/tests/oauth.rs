use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
    response::Response,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use tower::ServiceExt;
use url::Url;
use wispnet_command_center::{Config, Hub, Shared, db, digest, now, oauth, token};

const ROOT: &str = "http://127.0.0.1:43210";
const RESOURCE: &str = "https://desktop-gike.wispnet.test/MCP";
const OTHER: &str = "https://desktop-other.wispnet.test/MCP";
const REDIRECT: &str = "http://127.0.0.1:9911/callback";
// Publicly committed test vector generated solely for these tests. It is never
// used by the server, which only verifies keys published by OAuth clients.
const JWT_FIXTURE_DER: &str = "MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQCJSycy13LOYfgW+2gjFy3b5tIhp9AgX+jZ2Zag1evDToJAkpORVSCQPNZmBqRUoUxtenGC6nWgtKwowKblFNkudR/gRkuaJhvCuUAV3SE8wOi/XnDtG4VxZTrh1sjm9B64sxGYpyfSCtyd8LQxb2acT1Os9G/nAbygG9tdKqlxMvXQoe+cfT51b3OxgdDs7bQB8tAjcW0YfOgaXlTslM4aogOk7FAvDrM2mHOTfYC/CntQ8iqWmUmXsVPYgIkSrCyfMbtY+zsORd6h2V3aQuRTAEwtwseoPTO410soUK41D6J/xWjrWpuczJQxEPYiiBblOohcjM3nfmHyCZh9Tz0tAgMBAAECggEAIFW13rCFVnwWXPtLFHCiGVdsiAao6ucLxTsM0cZwJ2h3HtQmsyTUbYQWjqLj0xbGFEaxMz8bAwVqQpLU+8EEQX/OSRjlN2HgSeUtZbjV2iQo7OFGQWGo9QL9cjEO0e4OyjtKWqSLqVlrijslhTsSdJ/uOh67uDFdhmvpJzEkBQe+NCIH1tP7HoFG38bis9IaGYD/MZmPPERR4PavZDomjFWwBq1pKQj/quVbkGFEMrUgQ6ELjLCO/677rgDqc9tyn2jESgCZV/9hG8vTZqwvPFItf0QM7EEmFM3xGR+gNNfa2ulGR+Ys/QF0nh+TcQrDAC8L3vsGOVWNDPDpkvlptQKBgQC8CSkU8X7y4ZERknRnxZJgDxYnT9GSGXbvs5nRx5L2EByWedevPzXvXQzW3FCqydAUiAEnPyyOt2uAglJyGeFeMMyuxvFfnxLwZTL+CJQF3yWwB3jRma9vGSR9qh0pOqUFWK2Oz7PTV4PooqUlqVVPcTL5nj5x53HHoctmZsEYowKBgQC66thwG3a9x0+XQsxrjAS8QzfNhVgXiWvvyeyPkZWtAoGjkDI8noqEyvMVJgUi+3V/K4FJ9715aIqANZzKpfcITRFQILXcjWO1FGoEBpCq/tGKZUFzF1wOIeELknT6R+ubERkxNo/+1aNmauhyRel8fKL/aNNBzjJHcXb1vBSf7wKBgA6Hhq0fm/eYhmUdM/FwysaxY7zV5hOr9AzBi/kxuqFYzVyzPX9FGx3LytovW+xWe8WaXLxznTXJ5nyflU9LwKZOjwJSTQReflAlAbOvZgCuJXNcoHFRtsSW5MLxCh/NcELhf0extOt3r7GiH7CxTO4+iPnQyHJ0t839KoJXHkSRAoGACrt/T/iTudPe4w2H1UkF4JVvd9KHO1q9otgSSDAQyw1QykZ2S/rCc1t/16AGPXr4xSOHWerqqzTwduodJZmf9I8QRmyKZK6NohEZAUArQFY/S9YCiww1Ue5qEJMJOnG0uoOh0ayTWguERVYwIJeZD3fMmCO5tZaUVnvSW8u/12sCgYBBgpu860RU79vuPUrxfc0M4Wd04GcspPja3QjfKUHoxprUTZ+n+OnS7lHJGmyoiOe09ckltMSxEY1lm6EU5DIvs3jdFmRA3S2/Mv099U9FM4e9Ydr+0Zvlu2FtLvjjbOZPw9UaP7/fr7YmX4pgBP9SUjUqrNPRBEdc9bf1HRwWHg==";
const JWT_FIXTURE_N: &str = "iUsnMtdyzmH4FvtoIxct2-bSIafQIF_o2dmWoNXrw06CQJKTkVUgkDzWZgakVKFMbXpxgup1oLSsKMCm5RTZLnUf4EZLmiYbwrlAFd0hPMDov15w7RuFcWU64dbI5vQeuLMRmKcn0grcnfC0MW9mnE9TrPRv5wG8oBvbXSqpcTL10KHvnH0-dW9zsYHQ7O20AfLQI3FtGHzoGl5U7JTOGqIDpOxQLw6zNphzk32Avwp7UPIqlplJl7FT2ICJEqwsnzG7WPs7DkXeodld2kLkUwBMLcLHqD0zuNdLKFCuNQ-if8Vo61qbnMyUMRD2IogW5TqIXIzN535h8gmYfU89LQ";
const ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

fn signed_assertion(id: &str, changes: Value) -> String {
    let mut claims = json!({"iss":id,"sub":id,"aud":format!("{ROOT}/api/auth/oauth2/token"),"exp":now()+120,"iat":now(),"jti":token("")});
    for (key, value) in changes.as_object().unwrap() {
        claims[key] = value.clone();
    }
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("fixture-key".into());
    let der = base64::engine::general_purpose::STANDARD
        .decode(JWT_FIXTURE_DER)
        .unwrap();
    jsonwebtoken::encode(
        &header,
        &claims,
        // The fixed PKCS#8 vector wraps its PKCS#1 RSA key after 26 bytes.
        &jsonwebtoken::EncodingKey::from_rsa_der(&der[26..]),
    )
    .unwrap()
}
struct Fixture {
    hub: Shared,
    _state: tempfile::TempDir,
    owner: String,
    alice: String,
}
impl Fixture {
    fn new() -> Self {
        let state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let hub = Hub::open(Config {
            base_url: ROOT.into(),
            state_dir: state.path().into(),
            public_domain: "wispnet.test".into(),
            github_client_id: String::new(),
            github_client_secret: String::new(),
            test_mode: true,
        })
        .unwrap();
        let owner = token("session_");
        let alice = token("session_");
        {
            let db = hub.db.lock().unwrap();
            for (id, session) in [("owner", &owner), ("alice", &alice)] {
                db.execute(
                    "INSERT INTO users(id,github_id,name,email) VALUES(?,?,?,?)",
                    params![id, id, id, format!("{id}@example.test")],
                )
                .unwrap();
                db.execute(
                    "INSERT INTO sessions(token_hash,user_id,expires) VALUES(?,?,?)",
                    params![digest(session), id, now() + 3600],
                )
                .unwrap();
            }
            db.execute("INSERT INTO command_hosts(host,owner_id,name,publisher_hash) VALUES('host','owner','Desktop','publisher-hash')",[]).unwrap();
            for (id, resource) in [("gike", RESOURCE), ("other", OTHER)] {
                db.execute("INSERT INTO command_services(id,host,owner_id,service,incarnation,name,resource) VALUES(?,'host','owner',?,'incarnation','Desktop',?)",params![id,id,resource]).unwrap();
            }
        }
        Self {
            hub,
            _state: state,
            owner,
            alice,
        }
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Body,
        content_type: &str,
        cookie: Option<&str>,
    ) -> Response {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("origin", ROOT);
        if !content_type.is_empty() {
            req = req.header("content-type", content_type);
        }
        if let Some(cookie) = cookie {
            req = req.header("cookie", format!("wispnet_session={cookie}"));
        }
        self.hub
            .router()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap()
    }
    async fn post(&self, path: &str, body: Value, cookie: Option<&str>) -> Response {
        self.request(
            Method::POST,
            path,
            Body::from(body.to_string()),
            "application/json",
            cookie,
        )
        .await
    }
    async fn form(&self, path: &str, pairs: &[(&str, &str)]) -> Response {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs.iter().copied())
            .finish();
        self.request(
            Method::POST,
            path,
            Body::from(body),
            "application/x-www-form-urlencoded",
            None,
        )
        .await
    }
    async fn register(&self) -> String {
        let response=self.post("/api/auth/oauth2/register",json!({"client_name":"Protocol fixture","redirect_uris":[REDIRECT],"token_endpoint_auth_method":"none","grant_types":["authorization_code","refresh_token"],"response_types":["code"],"scope":"mcp:use offline_access"}),None).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        value(response).await["client_id"].as_str().unwrap().into()
    }
    fn private_client(&self) -> String {
        let id = "https://client.example/private-client.json";
        let metadata = json!({"client_id":id,"client_name":"Signed client fixture","redirect_uris":[REDIRECT],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"private_key_jwt","scope":"mcp:use offline_access","application_type":"native","jwks_uri":"https://client.example/jwks.json","token_endpoint_auth_signing_alg":"RS256"});
        let keys = json!({"keys":[{"kty":"RSA","kid":"fixture-key","use":"sig","key_ops":["verify"],"alg":"RS256","n":JWT_FIXTURE_N,"e":"AQAB"}]});
        let db = self.hub.db.lock().unwrap();
        db.execute(
            "INSERT INTO oauth_clients(client_id,metadata,source,expires) VALUES(?,?,'cimd',?)",
            params![id, metadata.to_string(), now() + 300],
        )
        .unwrap();
        db.execute("INSERT INTO oauth_jwks(client_id,uri,document,expires) VALUES(?,'https://client.example/jwks.json',?,?)",params![id,keys.to_string(),now()+300]).unwrap();
        id.into()
    }
    fn authorization(&self, client: &str, resource: &str, verifier: &str, scope: &str) -> String {
        let mut url = Url::parse(&format!("{ROOT}/api/auth/oauth2/authorize")).unwrap();
        url.query_pairs_mut()
            .append_pair("client_id", client)
            .append_pair("redirect_uri", REDIRECT)
            .append_pair("response_type", "code")
            .append_pair("resource", resource)
            .append_pair("scope", scope)
            .append_pair("code_challenge_method", "S256")
            .append_pair(
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            )
            .append_pair("state", "caller-state");
        format!("{}?{}", url.path(), url.query().unwrap())
    }
    async fn begin(
        &self,
        client: &str,
        resource: &str,
        scope: &str,
        cookie: &str,
    ) -> (String, String) {
        let verifier = token("");
        let path = self.authorization(client, resource, &verifier, scope);
        let response = self
            .request(Method::GET, &path, Body::empty(), "", Some(cookie))
            .await;
        assert_eq!(
            response.status(),
            StatusCode::FOUND,
            "{}",
            String::from_utf8_lossy(&to_bytes(response.into_body(), 65536).await.unwrap())
        );
        let location = Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
        assert_eq!(location.path(), "/consent");
        (verifier, location.query().unwrap().into())
    }
    async fn accept(&self, query: &str, cookie: &str) -> String {
        let response = self
            .post(
                "/api/auth/oauth2/consent",
                json!({"accept":true,"oauth_query":query}),
                Some(cookie),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let result = value(response).await;
        let callback = Url::parse(result["url"].as_str().unwrap()).unwrap();
        let query: std::collections::HashMap<_, _> = callback.query_pairs().into_owned().collect();
        assert_eq!(query["state"], "caller-state");
        assert_eq!(query["iss"], ROOT);
        query["code"].clone()
    }
    async fn exchange(&self, client: &str, code: &str, verifier: &str, resource: &str) -> Response {
        self.form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "authorization_code"),
                ("client_id", client),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", REDIRECT),
                ("resource", resource),
            ],
        )
        .await
    }
    async fn flow(&self, client: &str, cookie: &str) -> Value {
        let (verifier, query) = self
            .begin(client, RESOURCE, "mcp:use offline_access", cookie)
            .await;
        let code = self.accept(&query, cookie).await;
        let response = self.exchange(client, &code, &verifier, RESOURCE).await;
        assert_eq!(response.status(), StatusCode::OK);
        value(response).await
    }
    fn allowed(&self, access: &str, resource: &str) -> bool {
        let db = self.hub.db.lock().unwrap();
        let service = db::resource(&db, resource).unwrap().unwrap();
        oauth::authorize_token(&db, access, &service).is_ok()
    }
}
async fn value(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap()
}

#[tokio::test]
async fn discovery_and_public_client_registration_describe_the_supported_profile() {
    let f = Fixture::new();
    let response = f
        .request(
            Method::GET,
            "/.well-known/oauth-authorization-server",
            Body::empty(),
            "",
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let discovery = value(response).await;
    assert_eq!(discovery["issuer"], ROOT);
    assert_eq!(
        discovery["code_challenge_methods_supported"],
        json!(["S256"])
    );
    assert_eq!(
        discovery["token_endpoint_auth_methods_supported"],
        json!(["none", "private_key_jwt"])
    );
    assert_eq!(discovery["client_id_metadata_document_supported"], true);
    for metadata in [
        json!({"redirect_uris":["http://public.example/callback"]}),
        json!({"redirect_uris":["https://example.test/callback#fragment"]}),
        json!({"redirect_uris":[REDIRECT],"token_endpoint_auth_method":"client_secret_basic"}),
        json!({"redirect_uris":[REDIRECT],"grant_types":["client_credentials"]}),
    ] {
        assert_eq!(
            f.post("/api/auth/oauth2/register", metadata, None)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let client = f.register().await;
    let path = f.authorization(&client, RESOURCE, &token(""), "mcp:use");
    let response = f.request(Method::GET, &path, Body::empty(), "", None).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let location = Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
    assert_eq!(location.path(), "/sign-in");
    assert!(
        location
            .query_pairs()
            .any(|(k, v)| k == "returnTo" && v.starts_with("/api/auth/oauth2/authorize?"))
    );
    for id in [
        "https://127.0.0.1/client.json",
        "https://169.254.169.254/client.json",
        "https://example.test/",
        "http://example.test/client.json",
    ] {
        let path = f.authorization(id, RESOURCE, &token(""), "mcp:use");
        assert_eq!(
            f.request(Method::GET, &path, Body::empty(), "", Some(&f.owner))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn opaque_oauth_flow_binds_resource_and_rotates_refresh_with_replay_revocation() {
    let f = Fixture::new();
    let client = f.register().await;
    let first = f.flow(&client, &f.owner).await;
    let access = first["access_token"].as_str().unwrap();
    let refresh = first["refresh_token"].as_str().unwrap();
    assert!(access.starts_with("woa_"));
    assert!(f.allowed(access, RESOURCE));
    assert!(!f.allowed(access, OTHER));
    let rotated = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("refresh_token", refresh),
                ("resource", RESOURCE),
            ],
        )
        .await;
    assert_eq!(rotated.status(), StatusCode::OK);
    let rotated = value(rotated).await;
    assert_ne!(rotated["refresh_token"], first["refresh_token"]);
    let access2 = rotated["access_token"].as_str().unwrap();
    assert!(f.allowed(access2, RESOURCE));
    let replay = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("refresh_token", refresh),
                ("resource", RESOURCE),
            ],
        )
        .await;
    assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
    assert!(!f.allowed(access, RESOURCE));
    assert!(!f.allowed(access2, RESOURCE));
    let fresh = f.flow(&client, &f.owner).await;
    assert!(f.allowed(fresh["access_token"].as_str().unwrap(), RESOURCE));
    let replay = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("refresh_token", refresh),
                ("resource", RESOURCE),
            ],
        )
        .await;
    assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
    assert!(
        f.allowed(fresh["access_token"].as_str().unwrap(), RESOURCE),
        "old replay must not revoke a newer explicitly approved connection"
    );
    let db = f.hub.db.lock().unwrap();
    for table in ["oauth_access_tokens", "oauth_refresh_tokens", "oauth_codes"] {
        let mut statement = db
            .prepare(&format!("SELECT token_hash FROM {table}"))
            .unwrap();
        for hash in statement.query_map([], |r| r.get::<_, String>(0)).unwrap() {
            let hash = hash.unwrap();
            assert_eq!(hash.len(), 64);
            assert!(!hash.contains(access));
            assert!(!hash.contains(refresh));
        }
    }
}

#[tokio::test]
async fn consent_is_user_session_and_request_bound_and_codes_are_single_use() {
    let f = Fixture::new();
    let client = f.register().await;
    let (verifier, query) = f
        .begin(&client, RESOURCE, "mcp:use offline_access", &f.owner)
        .await;
    assert_eq!(
        f.post(
            "/api/auth/oauth2/consent",
            json!({"accept":true,"oauth_query":query}),
            Some(&f.alice)
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let mut changed: std::collections::BTreeMap<String, String> =
        url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
    changed.insert("resource".into(), OTHER.into());
    let changed = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&changed)
        .finish();
    assert_eq!(
        f.post(
            "/api/auth/oauth2/consent",
            json!({"accept":true,"oauth_query":changed}),
            Some(&f.owner)
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let other_session = token("");
    f.hub
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO sessions VALUES(?,'owner',?)",
            params![digest(&other_session), now() + 1000],
        )
        .unwrap();
    assert_eq!(
        f.post(
            "/api/auth/oauth2/consent",
            json!({"accept":true,"oauth_query":query}),
            Some(&other_session)
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let code = f.accept(&query, &f.owner).await;
    assert_eq!(
        f.post(
            "/api/auth/oauth2/consent",
            json!({"accept":true,"oauth_query":query}),
            Some(&f.owner)
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.exchange(&client, &code, &token(""), RESOURCE)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.exchange(&client, &code, &verifier, OTHER).await.status(),
        StatusCode::BAD_REQUEST
    );
    let tokens = f.exchange(&client, &code, &verifier, RESOURCE).await;
    assert_eq!(tokens.status(), StatusCode::OK);
    let tokens = value(tokens).await;
    assert!(f.allowed(tokens["access_token"].as_str().unwrap(), RESOURCE));
    assert_eq!(
        f.exchange(&client, &code, &verifier, RESOURCE)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(
        !f.allowed(tokens["access_token"].as_str().unwrap(), RESOURCE),
        "code replay revokes credentials from the original grant"
    );
}

#[tokio::test]
async fn live_grants_revocation_regrant_and_expiry_never_resurrect_old_credentials() {
    let f = Fixture::new();
    let client = f.register().await;
    let path = f.authorization(&client, RESOURCE, &token(""), "mcp:use");
    assert_eq!(
        f.request(Method::GET, &path, Body::empty(), "", Some(&f.alice))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    {
        let mut db = f.hub.db.lock().unwrap();
        let service = db::resource(&db, RESOURCE).unwrap().unwrap();
        db::set_grant(&mut db, &service, "alice", true).unwrap();
    }
    let flow = f.flow(&client, &f.alice).await;
    let access = flow["access_token"].as_str().unwrap();
    assert!(f.allowed(access, RESOURCE));
    {
        let mut db = f.hub.db.lock().unwrap();
        let service = db::resource(&db, RESOURCE).unwrap().unwrap();
        db::set_grant(&mut db, &service, "alice", false).unwrap();
        db::set_grant(&mut db, &service, "alice", true).unwrap();
    }
    assert!(!f.allowed(access, RESOURCE));
    let refresh = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("refresh_token", flow["refresh_token"].as_str().unwrap()),
                ("resource", RESOURCE),
            ],
        )
        .await;
    assert_eq!(refresh.status(), StatusCode::BAD_REQUEST);
    let fresh = f.flow(&client, &f.alice).await;
    f.hub
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE oauth_access_tokens SET expires=? WHERE token_hash=?",
            params![now() - 1, digest(fresh["access_token"].as_str().unwrap())],
        )
        .unwrap();
    assert!(!f.allowed(fresh["access_token"].as_str().unwrap(), RESOURCE));
    let (verifier, query) = f.begin(&client, RESOURCE, "mcp:use", &f.alice).await;
    let code = f.accept(&query, &f.alice).await;
    f.hub
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE oauth_codes SET expires=? WHERE token_hash=?",
            params![now() - 1, digest(&code)],
        )
        .unwrap();
    assert_eq!(
        f.exchange(&client, &code, &verifier, RESOURCE)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn standard_revocation_is_client_bound_and_stays_effective_after_reconsent() {
    let f = Fixture::new();
    let first = f.register().await;
    let second = f.register().await;
    let flow = f.flow(&first, &f.owner).await;
    let access = flow["access_token"].as_str().unwrap();
    let wrong = f
        .form(
            "/api/auth/oauth2/revoke",
            &[("client_id", &second), ("token", access)],
        )
        .await;
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    assert!(f.allowed(access, RESOURCE));
    let revoked = f
        .form(
            "/api/auth/oauth2/revoke",
            &[("client_id", &first), ("token", access)],
        )
        .await;
    assert_eq!(revoked.status(), StatusCode::OK);
    assert!(!f.allowed(access, RESOURCE));
    let fresh = f.flow(&first, &f.owner).await;
    assert!(f.allowed(fresh["access_token"].as_str().unwrap(), RESOURCE));
    let old_refresh = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &first),
                ("refresh_token", flow["refresh_token"].as_str().unwrap()),
                ("resource", RESOURCE),
            ],
        )
        .await;
    assert_eq!(old_refresh.status(), StatusCode::BAD_REQUEST);
    assert!(f.allowed(fresh["access_token"].as_str().unwrap(), RESOURCE));
}

#[tokio::test]
async fn refresh_scope_can_only_narrow_and_omitting_offline_access_omits_refresh() {
    let f = Fixture::new();
    let client = f.register().await;
    let (verifier, query) = f.begin(&client, RESOURCE, "mcp:use", &f.owner).await;
    let code = f.accept(&query, &f.owner).await;
    let response = f.exchange(&client, &code, &verifier, RESOURCE).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(value(response).await.get("refresh_token").is_none());
    let flow = f.flow(&client, &f.owner).await;
    let response = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("refresh_token", flow["refresh_token"].as_str().unwrap()),
                ("resource", RESOURCE),
                ("scope", "mcp:use"),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = value(response).await;
    assert_eq!(response["scope"], "mcp:use");
    assert!(response.get("refresh_token").is_none());
    assert!(f.allowed(response["access_token"].as_str().unwrap(), RESOURCE));
}

#[tokio::test]
async fn cimd_client_completes_the_same_oauth_flow_without_dynamic_registration() {
    let f = Fixture::new();
    let id = "https://client.example/metadata.json";
    // The network document reader is covered with real chunked HTTP in oauth's
    // unit tests; here exercise the complete protocol against its cached result.
    let metadata = json!({"client_id":id,"client_name":"CIMD fixture","redirect_uris":[REDIRECT],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none","scope":"mcp:use offline_access","application_type":"native"});
    f.hub
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO oauth_clients(client_id,metadata,source,expires) VALUES(?,?,'cimd',?)",
            params![id, metadata.to_string(), now() + 300],
        )
        .unwrap();
    let flow = f.flow(id, &f.owner).await;
    assert!(f.allowed(flow["access_token"].as_str().unwrap(), RESOURCE));
    let response = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", id),
                ("refresh_token", flow["refresh_token"].as_str().unwrap()),
                ("resource", RESOURCE),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(f.allowed(
        value(response).await["access_token"].as_str().unwrap(),
        RESOURCE
    ));
}

#[tokio::test]
async fn abandoned_registration_cleanup_preserves_active_clients_and_the_cap_bounds_storage() {
    let f = Fixture::new();
    {
        let db = f.hub.db.lock().unwrap();
        for id in ["abandoned", "active", "pending"] {
            db.execute("INSERT INTO oauth_clients(client_id,metadata,source,expires) VALUES(?,'{}','dcr',?)",params![id,now()-1]).unwrap();
        }
        db.execute("INSERT INTO command_oauth_connections(id,user_id,service_id,client_id) VALUES('active-connection','owner','gike','active')",[]).unwrap();
        db.execute("INSERT INTO oauth_pending(id,user_id,session_hash,request,expires) VALUES('pending-request','owner','hash',?,?)",params![json!({"client_id":"pending"}).to_string(),now()+100]).unwrap();
    }
    let _ = f.register().await;
    {
        let db = f.hub.db.lock().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM oauth_clients WHERE client_id='abandoned'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM oauth_clients WHERE client_id IN('active','pending')",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM oauth_clients WHERE source='dcr'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        db.execute("WITH RECURSIVE numbers(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM numbers WHERE x<?) INSERT INTO oauth_clients(client_id,metadata,source,expires) SELECT 'bulk-'||x,'{}','dcr',? FROM numbers",params![10000-count,now()+86400]).unwrap();
    }
    let response = f
        .post(
            "/api/auth/oauth2/register",
            json!({"redirect_uris":[REDIRECT],"token_endpoint_auth_method":"none"}),
            None,
        )
        .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        f.hub
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM oauth_clients WHERE source='dcr'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        10000
    );
}

#[tokio::test]
async fn private_key_jwt_client_completes_exchange_refresh_and_revocation_with_fresh_assertions() {
    let f = Fixture::new();
    let client = f.private_client();
    let (verifier, query) = f
        .begin(&client, RESOURCE, "mcp:use offline_access", &f.owner)
        .await;
    let code = f.accept(&query, &f.owner).await;
    assert_eq!(
        f.exchange(&client, &code, &verifier, RESOURCE)
            .await
            .status(),
        StatusCode::UNAUTHORIZED,
        "private clients cannot downgrade to none"
    );
    let assertion = signed_assertion(&client, json!({}));
    let response = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "authorization_code"),
                ("client_id", &client),
                ("code", &code),
                ("code_verifier", &verifier),
                ("redirect_uri", REDIRECT),
                ("resource", RESOURCE),
                ("client_assertion_type", ASSERTION_TYPE),
                ("client_assertion", &assertion),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let flow = value(response).await;
    assert!(f.allowed(flow["access_token"].as_str().unwrap(), RESOURCE));
    let replay = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("resource", RESOURCE),
                ("refresh_token", flow["refresh_token"].as_str().unwrap()),
                ("client_assertion_type", ASSERTION_TYPE),
                ("client_assertion", &assertion),
            ],
        )
        .await;
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    let renewed = signed_assertion(&client, json!({}));
    let response = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("resource", RESOURCE),
                ("refresh_token", flow["refresh_token"].as_str().unwrap()),
                ("client_assertion_type", ASSERTION_TYPE),
                ("client_assertion", &renewed),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let rotated = value(response).await;
    assert!(f.allowed(rotated["access_token"].as_str().unwrap(), RESOURCE));
    let revoke = signed_assertion(
        &client,
        json!({"aud":format!("{ROOT}/api/auth/oauth2/revoke")}),
    );
    let response = f
        .form(
            "/api/auth/oauth2/revoke",
            &[
                ("client_id", &client),
                ("token", rotated["refresh_token"].as_str().unwrap()),
                ("client_assertion_type", ASSERTION_TYPE),
                ("client_assertion", &revoke),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!f.allowed(rotated["access_token"].as_str().unwrap(), RESOURCE));
    let db = f.hub.db.lock().unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM oauth_assertions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn private_key_jwt_denies_bad_signatures_claims_and_replayed_jti_after_restart() {
    let f = Fixture::new();
    let client = f.private_client();
    for changes in [
        json!({"aud":"https://wrong.example/token"}),
        json!({"iss":"https://other.example/client.json"}),
        json!({"sub":"other-client"}),
        json!({"exp":now()-1}),
        json!({"exp":now()+1000}),
        json!({"iat":now()+100}),
        json!({"nbf":now()+100}),
        json!({"jti":""}),
    ] {
        let assertion = signed_assertion(&client, changes);
        let response = f
            .form(
                "/api/auth/oauth2/token",
                &[
                    ("grant_type", "authorization_code"),
                    ("client_id", &client),
                    ("resource", RESOURCE),
                    ("code", "not-a-code"),
                    ("code_verifier", "not-a-verifier"),
                    ("redirect_uri", REDIRECT),
                    ("client_assertion_type", ASSERTION_TYPE),
                    ("client_assertion", &assertion),
                ],
            )
            .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let mut forged = signed_assertion(&client, json!({})).into_bytes();
    let position = forged.iter().rposition(|b| *b == b'.').unwrap() + 1;
    forged[position] = if forged[position] == b'A' { b'B' } else { b'A' };
    let forged = String::from_utf8(forged).unwrap();
    let response = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("resource", RESOURCE),
                ("refresh_token", "no-refresh"),
                ("client_assertion_type", ASSERTION_TYPE),
                ("client_assertion", &forged),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let assertion = signed_assertion(&client, json!({"jti":"durable-replay-id"}));
    let response = f
        .form(
            "/api/auth/oauth2/revoke",
            &[
                ("client_id", &client),
                ("token", "unknown-token"),
                ("client_assertion_type", ASSERTION_TYPE),
                ("client_assertion", &assertion),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let restarted = Hub::open(f.hub.config.clone()).unwrap();
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("client_id", client.as_str()),
            ("token", "unknown-token"),
            ("client_assertion_type", ASSERTION_TYPE),
            ("client_assertion", assertion.as_str()),
        ])
        .finish();
    let response = restarted
        .router()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/oauth2/revoke")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn native_loopback_port_is_flexible_but_the_authorization_code_keeps_the_exact_redirect() {
    let f = Fixture::new();
    let client = f.register().await;
    let verifier = token("");
    let actual = "http://127.0.0.1:53321/callback";
    let path = f.authorization(&client, RESOURCE, &verifier, "mcp:use");
    let uri = Url::parse(&format!("{ROOT}{path}")).unwrap();
    let mut query: std::collections::BTreeMap<String, String> =
        uri.query_pairs().into_owned().collect();
    query.insert("redirect_uri".into(), actual.into());
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&query)
        .finish();
    let response = f
        .request(
            Method::GET,
            &format!("/api/auth/oauth2/authorize?{query}"),
            Body::empty(),
            "",
            Some(&f.owner),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let consent = Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
    let code = f.accept(consent.query().unwrap(), &f.owner).await;
    assert_eq!(
        f.exchange(&client, &code, &verifier, RESOURCE)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let response = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "authorization_code"),
                ("client_id", &client),
                ("code", &code),
                ("code_verifier", &verifier),
                ("resource", RESOURCE),
                ("redirect_uri", actual),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(f.allowed(
        value(response).await["access_token"].as_str().unwrap(),
        RESOURCE
    ));
}

#[tokio::test]
async fn expired_replay_records_cannot_revoke_a_current_connection_before_cleanup() {
    let f = Fixture::new();
    let client = f.register().await;
    let (verifier, query) = f
        .begin(&client, RESOURCE, "mcp:use offline_access", &f.owner)
        .await;
    let code = f.accept(&query, &f.owner).await;
    let first = value(f.exchange(&client, &code, &verifier, RESOURCE).await).await;
    let response = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("resource", RESOURCE),
                ("refresh_token", first["refresh_token"].as_str().unwrap()),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let current = value(response).await;
    {
        let db = f.hub.db.lock().unwrap();
        db.execute(
            "UPDATE oauth_codes SET expires=? WHERE token_hash=?",
            params![now() - 1, digest(&code)],
        )
        .unwrap();
        db.execute(
            "UPDATE oauth_refresh_tokens SET expires=? WHERE token_hash=?",
            params![now() - 1, digest(first["refresh_token"].as_str().unwrap())],
        )
        .unwrap();
        db.execute(
            "UPDATE oauth_access_tokens SET expires=? WHERE token_hash=?",
            params![now() - 1, digest(first["access_token"].as_str().unwrap())],
        )
        .unwrap();
    }
    assert_eq!(
        f.exchange(&client, &code, &verifier, RESOURCE)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(f.allowed(current["access_token"].as_str().unwrap(), RESOURCE));
    let response = f
        .form(
            "/api/auth/oauth2/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("resource", RESOURCE),
                ("refresh_token", first["refresh_token"].as_str().unwrap()),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(f.allowed(current["access_token"].as_str().unwrap(), RESOURCE));
    let response = f
        .form(
            "/api/auth/oauth2/revoke",
            &[
                ("client_id", &client),
                ("token", first["access_token"].as_str().unwrap()),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(f.allowed(current["access_token"].as_str().unwrap(), RESOURCE));
}

#[tokio::test]
#[ignore = "explicit network probe of public upstream CIMD documents; no user sign-in"]
async fn live_upstream_cimd_documents_reach_the_github_login_boundary() {
    let f = Fixture::new();
    for (client, redirect) in [
        (
            "https://claude.ai/oauth/mcp-oauth-client-metadata",
            "https://claude.ai/api/mcp/auth_callback",
        ),
        (
            "https://chatgpt.com/oauth/ZoZTx6U8hji2/client.json",
            "https://chatgpt.com/connector/oauth/ZoZTx6U8hji2",
        ),
    ] {
        let path = f.authorization(client, RESOURCE, &token(""), "mcp:use offline_access");
        let uri = Url::parse(&format!("{ROOT}{path}")).unwrap();
        let mut query: std::collections::BTreeMap<String, String> =
            uri.query_pairs().into_owned().collect();
        query.insert("redirect_uri".into(), redirect.into());
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&query)
            .finish();
        let response = f
            .request(
                Method::GET,
                &format!("/api/auth/oauth2/authorize?{query}"),
                Body::empty(),
                "",
                None,
            )
            .await;
        let status = response.status();
        let location = response
            .headers()
            .get("location")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        assert_eq!(
            status,
            StatusCode::FOUND,
            "{client}: {}",
            String::from_utf8_lossy(&body)
        );
        assert_eq!(Url::parse(&location.unwrap()).unwrap().path(), "/sign-in");
    }
}
