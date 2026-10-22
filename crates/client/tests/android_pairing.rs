//! Android's peer-capable client, exercised through real Iroh endpoints.
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, sync::Arc};
use wispnet_client::Client;
use wispnet_core::{config::TransportConfig, identity::SecretKey};
use wispnet_registry::host;
async fn peer() -> (tempfile::TempDir, Arc<Client>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let config: TransportConfig = serde_json::from_value(
        json!({"bind_addr":"127.0.0.1:0","relay_url":"https://relay.example"}),
    )
    .unwrap();
    let client = Client::new_peer(SecretKey::generate(), dir.path().into(), config)
        .await
        .unwrap();
    (dir, client)
}
async fn pair(phone: &Arc<Client>, target: &Arc<Client>, state: &std::path::Path) {
    let (mut store, owner) = host::owner_store(state, target.endpoint.id()).unwrap();
    let mut invitation = store
        .pairing_invite(
            target.endpoint.id(),
            &owner,
            "https://relay.example",
            wispnet_registry::enrollment::now(),
            300,
        )
        .unwrap();
    invitation.direct_hint = Some(target.endpoint.bound_sockets()[0]);
    phone.command(&json!({"op":"pair","host":target.endpoint.id(),"invitation":serde_json::to_string(&invitation).unwrap(),"name":"Phone"})).await.unwrap();
    // Waiting for approval must never erase Android's native saved pairing.
    assert!(
        phone
            .command(&json!({"op":"catalog","host":target.endpoint.id()}))
            .await
            .is_err()
    );
    assert_eq!(
        phone
            .command(&json!({"op":"pairing_state","host":target.endpoint.id()}))
            .await
            .unwrap()["paired"],
        true
    );
    store
        .host_mutate(
            target.endpoint.id(),
            &owner,
            host::Mutation::ApproveInvitation {
                expected_revision: 0,
                invitation: invitation.invitation.clone(),
                services: vec![],
            },
        )
        .unwrap();
}
#[tokio::test]
async fn android_learns_host_removal_and_host_learns_phone_removal() {
    let (_phone_state, phone) = peer().await;
    let (target_state, target) = peer().await;
    pair(&phone, &target, target_state.path()).await;
    assert_eq!(
        phone.catalog(target.endpoint.id()).await.unwrap()["services"],
        json!([])
    );
    target
        .command(&json!({"op":"unpair","host":phone.endpoint.id()}))
        .await
        .unwrap();
    assert_eq!(
        phone.catalog(target.endpoint.id()).await.unwrap_err(),
        "Pairing removed by remote machine"
    );
    assert_eq!(
        phone
            .command(&json!({"op":"pairing_state","host":target.endpoint.id()}))
            .await
            .unwrap()["paired"],
        false
    );
    // Re-pairing establishes a fresh session and does not inherit the old removal.
    let (mut store, _) = host::owner_store(target_state.path(), target.endpoint.id()).unwrap();
    let revision = store.host_snapshot(target.endpoint.id()).unwrap().revision;
    drop(store);
    let (mut store, owner) = host::owner_store(target_state.path(), target.endpoint.id()).unwrap();
    let mut invitation = store
        .pairing_invite(
            target.endpoint.id(),
            &owner,
            "https://relay.example",
            wispnet_registry::enrollment::now(),
            300,
        )
        .unwrap();
    invitation.direct_hint = Some(target.endpoint.bound_sockets()[0]);
    phone.command(&json!({"op":"pair","host":target.endpoint.id(),"invitation":serde_json::to_string(&invitation).unwrap(),"name":"Phone"})).await.unwrap();
    store
        .host_mutate(
            target.endpoint.id(),
            &owner,
            host::Mutation::ApproveInvitation {
                expected_revision: revision,
                invitation: invitation.invitation.clone(),
                services: vec![],
            },
        )
        .unwrap();
    phone
        .command(&json!({"op":"unpair","host":target.endpoint.id()}))
        .await
        .unwrap();
    assert_eq!(
        target.catalog(phone.endpoint.id()).await.unwrap_err(),
        "Pairing removed by remote machine"
    );
    assert_eq!(
        target
            .command(&json!({"op":"pairing_state","host":phone.endpoint.id()}))
            .await
            .unwrap()["paired"],
        false
    );
    phone.endpoint.close().await;
    target.endpoint.close().await;
}

#[tokio::test]
async fn connection_diagnostic_requires_pairing_and_observes_direct_path() {
    let (_phone_state, phone) = peer().await;
    let (target_state, target) = peer().await;
    let request = json!({"op":"diagnose_connection", "host":target.endpoint.id()});
    assert!(phone.command(&request).await.is_err());
    pair(&phone, &target, target_state.path()).await;
    let report = phone.command(&request).await.unwrap();
    assert_eq!(report["selected_path"], "direct");
    assert_eq!(report["relay_only"], false);
    assert_eq!(report["reason"], "Direct connectivity succeeded.");
    assert!(report["observation_ms"].as_u64().unwrap() < 1000);
}
