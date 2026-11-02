#![cfg(unix)]

use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, sync::Arc};
use wispnet_client::Client;
use wispnet_core::identity::SecretKey;

async fn embedded() -> (tempfile::TempDir, Arc<Client>) {
    let state = tempfile::tempdir().unwrap();
    std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let client = Client::new_peer(
        SecretKey::generate(),
        state.path().into(),
        serde_json::from_value(
            json!({"bind_addr":"127.0.0.1:0","relay_url":"https://relay.example"}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    (state, client)
}

async fn mutate(client: &Arc<Client>, operation: Value) -> Result<Value, String> {
    client
        .command(&json!({"op":"local_owner_mutate","operation":operation}))
        .await
}

#[tokio::test]
async fn embedded_owner_supports_http_tcp_and_explicit_grants_with_stale_revision_rejection() {
    let (_state, client) = embedded().await;
    let peer = SecretKey::generate().public();
    let before = client
        .command(&json!({"op":"local_owner_status"}))
        .await
        .unwrap();
    assert_eq!(before["host"]["revision"], 0);
    mutate(
        &client,
        json!({"op":"approve","expected_revision":0,"peer":peer,"name":"laptop"}),
    )
    .await
    .unwrap();
    for (revision, id, kind) in [(1, "website", "http"), (2, "database", "tcp")] {
        mutate(
            &client,
            json!({"op":"service","expected_revision":revision,
            "id":id,"backend":"127.0.0.1:8080","kind":kind,"allow":[peer]}),
        )
        .await
        .unwrap();
    }
    let status = client
        .command(&json!({"op":"local_owner_status"}))
        .await
        .unwrap();
    for (id, kind) in [("website", "http"), ("database", "tcp")] {
        let service = status["host"]["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .unwrap();
        assert_eq!(service["kind"], kind);
        assert_eq!(service["allow"], json!([peer]));
    }
    assert!(
        mutate(
            &client,
            json!({"op":"remove","id":"website","expected_revision":1})
        )
        .await
        .is_err()
    );
    assert_eq!(
        client
            .command(&json!({"op":"local_owner_status"}))
            .await
            .unwrap()["host"],
        status["host"]
    );
    let web = status["host"]["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "website")
        .unwrap();
    mutate(
        &client,
        json!({"op":"set_service_enabled","expected_revision":3,"id":"website",
        "incarnation":web["incarnation"],"enabled":false}),
    )
    .await
    .unwrap();
    let after = client
        .command(&json!({"op":"local_owner_status"}))
        .await
        .unwrap();
    let web = after["host"]["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "website")
        .unwrap();
    assert_eq!(web["enabled"], false);
    assert_eq!(web["allow"], json!([peer]));
}

#[tokio::test]
async fn embedded_owner_cannot_bypass_provider_folder_checks_or_enable_host_shell() {
    let (_state, client) = embedded().await;
    let status = client
        .command(&json!({"op":"local_owner_status"}))
        .await
        .unwrap();
    let shell = status["host"]["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "shell")
        .unwrap();
    for operation in [
        json!({"op":"set_service_enabled","expected_revision":0,"id":"shell","incarnation":shell["incarnation"],"enabled":true}),
        json!({"op":"files","expected_revision":0,"exports":{"incoming":"/tmp"},"allow":[]}),
        json!({"op":"file_inbox","expected_revision":0,"id":"unsafe","root":"/tmp","allow":[]}),
        json!({"op":"file_share","expected_revision":0,"id":"unsafe","root":"/tmp","allow":[]}),
    ] {
        assert!(mutate(&client, operation).await.is_err());
    }
    assert_eq!(
        client
            .command(&json!({"op":"local_owner_status"}))
            .await
            .unwrap()["host"],
        status["host"]
    );
}

#[tokio::test]
async fn embedded_owner_invites_and_durable_owner_receipts_use_the_existing_store() {
    let (_state, client) = embedded().await;
    let invitation = client
        .command(&json!({"op":"local_owner_invite"}))
        .await
        .unwrap();
    let preview = client
        .command(&json!({"op":"preview_invitation","invitation":invitation["code"]}))
        .await
        .unwrap();
    assert_eq!(preview["host"], client.endpoint.id().to_string());
    assert!(invitation["matrix"].as_array().unwrap().len() > 21);
    let peer = SecretKey::generate().public();
    let request = json!({"op":"local_owner_apply","host":client.endpoint.id(),"id":"a".repeat(64),
        "action":{"op":"approve","peer":peer,"name":"workstation"}});
    let first = client.command(&request).await.unwrap();
    assert_eq!(first["state"], "applied");
    assert_eq!(client.command(&request).await.unwrap(), first);
    let mut conflict = request.clone();
    conflict["action"] = json!({"op":"revoke","peer":peer});
    assert!(client.command(&conflict).await.is_err());
    let status = client
        .command(&json!({"op":"local_owner_status"}))
        .await
        .unwrap();
    assert_eq!(status["host"]["revision"], first["revision"]);
    assert_eq!(status["host"]["clients"].as_array().unwrap().len(), 1);
    assert!(
        status["host"]["services"]
            .as_array()
            .unwrap()
            .iter()
            .all(|service| service["allow"].as_array().unwrap().is_empty())
    );
}
