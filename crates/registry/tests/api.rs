use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use tower::ServiceExt;
use wipsnet_core::identity::{EndpointId, Identity};
use wipsnet_registry::{api, control, store::Store};

fn fixture() -> (tempfile::TempDir, Router, String) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    let credential = dir.path().join("owner.token");
    control::initialize(&mut store, &credential, "test-mesh").unwrap();
    let owner = control::read_token(&credential).unwrap().to_string();
    (dir, api::router(store), owner)
}
async fn request(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn enroll(app: &Router, owner: &str, revision: u64, id: EndpointId) -> String {
    let (status, body) = request(
        app,
        "POST",
        "/v1/mutations",
        Some(owner),
        json!({"op":"enroll","expected_revision":revision,"device":id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revision"], revision + 1);
    body["token"].as_str().unwrap().to_owned()
}
#[tokio::test]
async fn roles_directory_ownership_revision_and_revocation() {
    let (_dir, app, owner) = fixture();
    assert_eq!(
        request(&app, "GET", "/healthz", None, Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "GET", "/readyz", None, Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "GET", "/v1/directory", None, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let host = Identity::generate().endpoint_id();
    let guest = Identity::generate().endpoint_id();
    let outsider = Identity::generate().endpoint_id();
    let host_token = enroll(&app, &owner, 0, host).await;
    let guest_token = enroll(&app, &owner, 1, guest).await;
    let outsider_token = enroll(&app, &owner, 2, outsider).await;
    for mutation in [
        json!({"op":"enroll","expected_revision":3,"device":Identity::generate().endpoint_id()}),
        json!({"op":"service","expected_revision":3,"id":"stolen","host":guest,"grants":[guest]}),
        json!({"op":"revoke","expected_revision":3,"device":host}),
    ] {
        assert_eq!(
            request(&app, "POST", "/v1/mutations", Some(&guest_token), mutation)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    let policy =
        json!({"op":"service","expected_revision":3,"id":"website","host":host,"grants":[guest]});
    assert_eq!(
        request(&app, "POST", "/v1/mutations", Some(&owner), policy.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "POST", "/v1/mutations", Some(&owner), policy)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let publication =
        json!({"op":"publish","expected_revision":4,"id":"website","address":"127.0.0.1:7779"});
    assert_eq!(
        request(
            &app,
            "POST",
            "/v1/mutations",
            Some(&guest_token),
            publication.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/v1/mutations",
            Some(&host_token),
            publication
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, directory) = request(
        &app,
        "GET",
        "/v1/directory",
        Some(&guest_token),
        Value::Null,
    )
    .await;
    assert_eq!(directory["revision"], 5);
    assert_eq!(directory["devices"], json!([guest]));
    assert_eq!(directory["services"][0]["address"], "127.0.0.1:7779");
    assert_eq!(directory["services"][0]["grants"], json!([]));
    let (_, hidden) = request(
        &app,
        "GET",
        "/v1/directory",
        Some(&outsider_token),
        Value::Null,
    )
    .await;
    assert_eq!(hidden["services"], json!([]));
    let (_, hosting) = request(&app, "GET", "/v1/directory", Some(&host_token), Value::Null).await;
    assert_eq!(hosting["services"][0]["grants"], json!([guest]));
    // Reassigning a host atomically clears its stale route.
    assert_eq!(request(&app,"POST","/v1/mutations",Some(&owner),json!({"op":"service","expected_revision":5,"id":"website","host":outsider,"grants":[guest]})).await.0,StatusCode::OK);
    let (_, directory) = request(
        &app,
        "GET",
        "/v1/directory",
        Some(&guest_token),
        Value::Null,
    )
    .await;
    assert_eq!(directory["revision"], 6);
    assert!(directory["services"][0]["address"].is_null());
    assert_eq!(
        request(
            &app,
            "POST",
            "/v1/mutations",
            Some(&owner),
            json!({"op":"revoke","expected_revision":6,"device":guest})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "GET",
            "/v1/directory",
            Some(&guest_token),
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // No owner-replacement operation exists, even for a device with a valid token.
    let (_, error) = request(
        &app,
        "POST",
        "/v1/mutations",
        Some(&host_token),
        json!({"op":"owner","token":"SECRET-SENTINEL"}),
    )
    .await;
    assert_eq!(error, json!({"error":"invalid_json"}));
}
#[tokio::test]
async fn malformed_oversized_browser_requests_and_unknown_versions_fail_closed() {
    let (_dir, app, owner) = fixture();
    let req = Request::builder()
        .method("POST")
        .uri("/v1/mutations")
        .header("authorization", format!("Bearer {owner}"))
        .header("content-type", "application/json")
        .body(Body::from("x".repeat(128 * 1024 + 1)))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let req = Request::builder()
        .uri("/v1/directory")
        .header("origin", "https://evil.example")
        .header("authorization", format!("Bearer {owner}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(request_unknown(&app, &owner).await, StatusCode::NOT_FOUND);
    let (_,error)=request(&app,"POST","/v1/mutations",Some(&owner),json!({"op":"service","expected_revision":0,"id":"SECRET-SENTINEL","host":"SECRET-SENTINEL","grants":[]})).await;
    assert_eq!(error, json!({"error":"invalid_json"}));
}
async fn request_unknown(app: &Router, owner: &str) -> StatusCode {
    request(app, "GET", "/v2/directory", Some(owner), Value::Null)
        .await
        .0
}
#[test]
fn migration_bootstrap_restart_and_backup_preserve_authority() {
    let (dir, app, owner) = fixture();
    drop(app);
    let path = dir.path().join("db");
    let credential = dir.path().join("owner.token");
    let mut store = Store::open(&path).unwrap();
    control::initialize(&mut store, &credential, "test-mesh").unwrap();
    assert!(control::initialize(&mut store, &credential, "different").is_err());
    let host = Identity::generate().endpoint_id();
    let update = store
        .mutate(
            control::Principal::Owner,
            control::Mutation::Enroll {
                expected_revision: 0,
                device: host,
            },
        )
        .unwrap();
    let device_token = update.token.as_ref().unwrap();
    assert!(store.authenticate(device_token).unwrap().is_some());
    let backup = dir.path().join("backup");
    store.backup(&backup).unwrap();
    drop(store);
    let mut restored = Store::open(&backup).unwrap();
    assert!(matches!(
        restored.authenticate(&owner).unwrap(),
        Some(control::Principal::Owner)
    ));
    assert!(
        matches!(restored.authenticate(device_token).unwrap(),Some(control::Principal::Device(id)) if id==host)
    );
    assert_eq!(
        restored
            .directory(control::Principal::Owner)
            .unwrap()
            .revision,
        1
    );
    // A different credential file can never silently replace the owner.
    assert!(
        control::initialize(
            &mut restored,
            &dir.path().join("replacement.token"),
            "test-mesh"
        )
        .is_err()
    );
}
#[test]
fn version_one_migrates_without_losing_existing_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join("db");
    let host = Identity::generate().endpoint_id();
    {
        let mut store = Store::open(&path).unwrap();
        store.replace(0, &[host], &[]).unwrap();
    }
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE routes; DROP TABLE device_tokens; DROP TABLE controller; PRAGMA user_version=1;").unwrap();
    drop(db);
    let mut migrated = Store::open(&path).unwrap();
    assert_eq!(migrated.snapshot().unwrap().devices, vec![host]);
    assert!(!migrated.initialized().unwrap());
}
