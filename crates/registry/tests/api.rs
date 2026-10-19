use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use tower::ServiceExt;
use wispnet_core::{
    enrollment::{Challenge, Proof},
    identity::Identity,
};
use wispnet_registry::{api, control, store::Store};

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
async fn enroll(app: &Router, owner: &str, revision: u64, key: &Identity) -> String {
    let (status, body) = request(
        app,
        "POST",
        "/v1/enrollment/admin",
        Some(owner),
        json!({"op":"invite","controller":"https://controller.example","ttl_seconds":600}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let invitation = &body["invitation"];
    let id = invitation["invitation"].as_str().unwrap();
    let code = invitation["code"].as_str().unwrap();
    for completing in [false, true] {
        let (status, body) = request(
            app,
            "POST",
            "/v1/enrollment/challenge",
            Some(code),
            json!({"invitation":id,"device":key.endpoint_id(),"name":"test-device"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let challenge: Challenge = serde_json::from_value(body["challenge"].clone()).unwrap();
        let proof = Proof {
            invitation: id.into(),
            device: key.endpoint_id(),
            name: "test-device".into(),
            nonce: challenge.nonce.clone(),
            signature: key.secret_key().sign(&challenge.signing_bytes()),
        };
        let (status, body) = request(
            app,
            "POST",
            "/v1/enrollment/proof",
            Some(code),
            serde_json::to_value(proof).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if completing {
            assert_eq!(body["revision"], revision + 1);
            return body["token"].as_str().unwrap().into();
        }
        assert_eq!(body["state"], "pending");
        assert_eq!(request(app,"POST","/v1/enrollment/admin",Some(owner),json!({"op":"approve","invitation":id,"device":key.endpoint_id(),"name":"test-device","expected_revision":revision})).await.0,StatusCode::OK);
    }
    unreachable!()
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
    let host_key = Identity::generate();
    let guest_key = Identity::generate();
    let outsider_key = Identity::generate();
    let host = host_key.endpoint_id();
    let guest = guest_key.endpoint_id();
    let outsider = outsider_key.endpoint_id();
    let host_token = enroll(&app, &owner, 0, &host_key).await;
    let guest_token = enroll(&app, &owner, 1, &guest_key).await;
    let outsider_token = enroll(&app, &owner, 2, &outsider_key).await;
    for mutation in [
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
    assert_eq!(
        request(
            &app,
            "POST",
            "/v1/enrollment/admin",
            Some(&guest_token),
            json!({"op":"invite","controller":"https://controller.example","ttl_seconds":600})
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
            Some(&owner),
            json!({"op":"enroll","expected_revision":3,"device":Identity::generate().endpoint_id()})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
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
#[tokio::test]
async fn migration_bootstrap_restart_and_backup_preserve_authority() {
    let (dir, app, owner) = fixture();
    let host_key = Identity::generate();
    let device_token = enroll(&app, &owner, 0, &host_key).await;
    drop(app);
    let path = dir.path().join("db");
    let credential = dir.path().join("owner.token");
    let mut store = Store::open(&path).unwrap();
    control::initialize(&mut store, &credential, "test-mesh").unwrap();
    assert!(control::initialize(&mut store, &credential, "different").is_err());
    let host = host_key.endpoint_id();
    assert!(store.authenticate(&device_token).unwrap().is_some());
    let backup = dir.path().join("backup");
    store.backup(&backup).unwrap();
    drop(store);
    let mut restored = Store::open(&backup).unwrap();
    assert!(matches!(
        restored.authenticate(&owner).unwrap(),
        Some(control::Principal::Owner)
    ));
    assert!(
        matches!(restored.authenticate(&device_token).unwrap(),Some(control::Principal::Device(id)) if id==host)
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
    db.execute_batch("DROP TABLE host_operation_receipts; DROP TABLE host_grants; DROP TABLE host_services; DROP TABLE host_clients; DROP TABLE host_authority; DROP TABLE invitations; DROP TABLE device_records; DROP TABLE routes; DROP TABLE device_tokens; DROP TABLE controller; PRAGMA user_version=1;").unwrap();
    drop(db);
    let mut migrated = Store::open(&path).unwrap();
    assert_eq!(migrated.snapshot().unwrap().devices, vec![host]);
    assert!(!migrated.initialized().unwrap());
}

#[tokio::test]
async fn public_enrollment_budget_does_not_block_owner_review() {
    let (_dir, app, owner) = fixture();
    let code = control::token();
    let id = control::token();
    let key = Identity::generate();
    for _ in 0..60 {
        assert_eq!(
            request(
                &app,
                "POST",
                "/v1/enrollment/challenge",
                Some(&code),
                json!({"invitation":&*id,"device":key.endpoint_id(),"name":"test"})
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        request(
            &app,
            "POST",
            "/v1/enrollment/challenge",
            Some(&code),
            json!({"invitation":&*id,"device":key.endpoint_id(),"name":"test"})
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        request(&app, "GET", "/v1/enrollment", Some(&owner), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
}
