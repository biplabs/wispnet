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
    assert!(report["observation_ms"].as_u64().unwrap() <= 40_000);
    assert_eq!(report["remote"]["host"], target.endpoint.id().to_string());
    assert_eq!(report["remote"]["version"], 1);
    assert!(report["remote_error"].is_null());
    for (side, expected) in [
        ("local", phone.endpoint.id()),
        ("remote", target.endpoint.id()),
    ] {
        let mapping: wispnet_core::host::RouterMappingReport =
            serde_json::from_value(report["router_mapping"][side].clone()).unwrap();
        assert!(mapping.valid(expected));
        assert!(mapping.enabled);
    }

    let mut remote: wispnet_core::host::DiagnosticSnapshot =
        serde_json::from_value(report["remote"].clone()).unwrap();
    assert!(remote.valid(target.endpoint.id()));
    assert!(!remote.valid(phone.endpoint.id()));
    remote.network.fresh = true;
    remote.network.report = None;
    assert!(!remote.valid(target.endpoint.id()));

    assert_eq!(report["local"]["paths"]["selected_path"], "direct");
    assert!(
        report["local"]["paths"]["direct_paths_seen"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        report["remote"]["paths"]["direct_datagrams_received"]
            .as_u64()
            .unwrap()
            > 0
    );
    phone.endpoint.close().await;
    target.endpoint.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn android_receiver_uses_existing_pairing_owner_grants_and_private_platform_storage() {
    use std::{
        io::Read,
        sync::{Mutex, atomic::AtomicBool},
    };
    use wispnet_client::files::{FileTarget, TransferOptions};
    #[derive(Default)]
    struct Storage(Mutex<Vec<u8>>);
    impl wispnet_files::Publisher for Storage {
        fn publish(
            &self,
            publication: wispnet_files::Publication<'_>,
            mut source: std::fs::File,
            _cancel: Arc<AtomicBool>,
        ) -> std::io::Result<()> {
            let mut bytes = Vec::new();
            source.read_to_end(&mut bytes)?;
            assert_eq!(wispnet_files::sha256(&bytes), publication.sha256);
            *self.0.lock().unwrap() = bytes;
            Ok(())
        }
    }
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let phone_state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(phone_state.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let staging = tempfile::tempdir().unwrap();
        let source = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(source.path(), b"desktop to Android verified upload").unwrap();
        let key = SecretKey::generate();
        let config: TransportConfig = serde_json::from_value(
            json!({"bind_addr":"127.0.0.1:0","relay_url":"https://relay.example"}),
        )
        .unwrap();
        let storage = Arc::new(Storage::default());
        let phone = Client::new_peer_with_publisher(
            key.clone(),
            phone_state.path().into(),
            config.clone(),
            Some(storage.clone()),
        )
        .await
        .unwrap();
        let (desktop_state, desktop) = peer().await;
        pair(&phone, &desktop, desktop_state.path()).await;
        phone.catalog(desktop.endpoint.id()).await.unwrap();
        let status = phone
            .command(&json!({"op":"local_files_status"}))
            .await
            .unwrap();
        assert_eq!(status["allow"], json!([]));
        let operation = json!({"op":"local_files_configure","expected_revision":status["revision"],
            "exports":{"incoming":staging.path()},"allow":[desktop.endpoint.id()]});
        phone.command(&operation).await.unwrap();
        assert!(
            phone.command(&operation).await.is_err(),
            "stale owner revision must fail"
        );
        assert!(desktop.catalog(phone.endpoint.id()).await.is_err());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        phone
            .command(&json!({"op":"configure_access","settings":{"registry_port":port}}))
            .await
            .unwrap();
        phone.command(&json!({"op":"enable"})).await.unwrap();
        let catalog = desktop.catalog(phone.endpoint.id()).await.unwrap();
        let service = catalog["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "files")
            .unwrap();
        assert_eq!(
            service.as_object().unwrap().len(),
            4,
            "sender sees no storage configuration"
        );
        let target = FileTarget {
            host: phone.endpoint.id(),
            service: "files".into(),
            incarnation: service["incarnation"].as_str().unwrap().into(),
        };
        phone.command(&json!({"op":"stop"})).await.unwrap();
        assert!(
            desktop
                .put_file(
                    target.clone(),
                    "",
                    source.path(),
                    "output.bin",
                    TransferOptions::default()
                )
                .await
                .is_err(),
            "receiving requires foreground access to be online"
        );
        phone.command(&json!({"op":"enable"})).await.unwrap();
        let uploaded = desktop
            .put_file(
                target.clone(),
                "",
                source.path(),
                "output.bin",
                TransferOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            uploaded.sha256,
            wispnet_files::hash_file(source.path()).unwrap()
        );
        assert_eq!(
            *storage.0.lock().unwrap(),
            b"desktop to Android verified upload"
        );
        assert!(
            !staging.path().join("output.bin").exists(),
            "platform publication does not retain a second local file"
        );
        phone.command(&json!({"op":"stop"})).await.unwrap();
        let status = phone
            .command(&json!({"op":"local_files_status"}))
            .await
            .unwrap();
        phone
            .command(
                &json!({"op":"local_files_configure","expected_revision":status["revision"],
            "exports":{"incoming":staging.path()},"allow":[]}),
            )
            .await
            .unwrap();
        assert!(desktop.catalog(phone.endpoint.id()).await.is_err());
        phone.command(&json!({"op":"enable"})).await.unwrap();
        assert!(
            desktop.catalog(phone.endpoint.id()).await.unwrap()["services"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        phone.command(&json!({"op":"stop"})).await.unwrap();
        let own = phone.endpoint.id();
        phone.endpoint.close().await;
        drop(phone);
        let restarted =
            Client::new_peer_with_publisher(key, phone_state.path().into(), config, Some(storage))
                .await
                .unwrap();
        assert_eq!(restarted.endpoint.id(), own);
        let restored = restarted
            .command(&json!({"op":"local_files_status"}))
            .await
            .unwrap();
        assert_eq!(
            restored["exports"]["incoming"],
            staging.path().to_str().unwrap()
        );
        assert_eq!(restored["allow"], json!([]));
        assert_eq!(restored["clients"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}
