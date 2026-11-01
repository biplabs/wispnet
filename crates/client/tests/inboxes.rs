#![cfg(unix)]
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};
use wispnet_client::{
    Client,
    files::{FileTarget, TransferOptions},
};
use wispnet_core::{config::TransportConfig, identity::SecretKey};
use wispnet_registry::host::{self, Authority, Mutation};

fn config() -> TransportConfig {
    serde_json::from_value(json!({"bind_addr":"127.0.0.1:0"})).unwrap()
}
fn private(path: &Path) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
async fn target(authority: &Authority, id: &str) -> FileTarget {
    let snapshot = authority.snapshot().await.unwrap();
    let s = snapshot.services.iter().find(|s| s.id == id).unwrap();
    FileTarget {
        host: snapshot.host,
        service: id.into(),
        incarnation: s.incarnation.clone(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_inbox_grants_select_one_root_and_are_independent_from_reads() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let state = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        let sources = tempfile::tempdir().unwrap();
        private(state.path());
        private(local.path());
        let hk = SecretKey::generate();
        let ck = SecretKey::generate();
        let own = hk.public();
        let peer = ck.public();
        host::initialize(state.path(), own).unwrap();
        let authority = Authority::open(state.path(), own).unwrap();
        let (mut store, owner) = host::owner_store(state.path(), own).unwrap();
        let revision = store.host_snapshot(own).unwrap().revision;
        let mut revision = store
            .host_mutate(
                own,
                &owner,
                Mutation::Approve {
                    expected_revision: revision,
                    peer,
                    name: "reader".into(),
                },
            )
            .unwrap();
        for (id, root, allow) in [
            ("inbox-a", a.path(), vec![peer]),
            ("inbox-b", b.path(), vec![]),
        ] {
            revision = store
                .host_mutate(
                    own,
                    &owner,
                    Mutation::FileInbox {
                        expected_revision: revision,
                        id: id.into(),
                        root: root.to_owned(),
                        allow,
                    },
                )
                .unwrap();
        }
        store
            .host_mutate(
                own,
                &owner,
                Mutation::FileShare {
                    expected_revision: revision,
                    id: "checkpoints".into(),
                    root: shared.path().to_owned(),
                    allow: vec![peer],
                },
            )
            .unwrap();
        let ta = target(&authority, "inbox-a").await;
        let tb = target(&authority, "inbox-b").await;
        let ts = target(&authority, "checkpoints").await;
        let host = wispnet_agent::tunnel::TunnelHost::bind_host(&hk, &config(), authority.clone())
            .await
            .unwrap();
        wispnet_agent::peers::save(
            local.path(),
            &wispnet_core::pairing::Bootstrap {
                version: 1,
                host: own,
                relay_url: "https://fixture.invalid".into(),
                direct_hint: Some(host.endpoint().bound_sockets()[0]),
            },
        )
        .unwrap();
        let client = Client::new(ck, local.path().to_owned(), config())
            .await
            .unwrap();
        let source = sources.path().join("source");
        std::fs::write(&source, b"first inbox bytes\0\xff").unwrap();
        client
            .put_file(
                ta.clone(),
                "",
                &source,
                "same.bin",
                TransferOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(a.path().join("same.bin")).unwrap(),
            b"first inbox bytes\0\xff"
        );
        assert!(!b.path().join("same.bin").exists());
        assert_eq!(
            client.list_file_workspaces(ta.clone()).await.unwrap(),
            vec!["incoming"]
        );
        assert_eq!(
            client
                .put_file(
                    ta.clone(),
                    "inbox-b",
                    &source,
                    "escape.bin",
                    TransferOptions::default()
                )
                .await
                .unwrap_err()
                .code,
            "PATH_DENIED"
        );
        assert_eq!(
            client
                .put_file(
                    tb.clone(),
                    "",
                    &source,
                    "same.bin",
                    TransferOptions::default()
                )
                .await
                .unwrap_err()
                .code,
            "PERMISSION_DENIED"
        );
        assert_eq!(
            client
                .put_file(
                    ts,
                    "",
                    &source,
                    "must-not-upload",
                    TransferOptions::default()
                )
                .await
                .unwrap_err()
                .code,
            "PERMISSION_DENIED"
        );
        assert_eq!(
            client
                .get_file(
                    ta.clone(),
                    "same.bin",
                    &sources.path().join("must-not-read"),
                    TransferOptions::default()
                )
                .await
                .unwrap_err()
                .code,
            "PERMISSION_DENIED"
        );
        let revision = store.host_snapshot(own).unwrap().revision;
        store
            .host_mutate(
                own,
                &owner,
                Mutation::GrantService {
                    expected_revision: revision,
                    service: "inbox-b".into(),
                    peer,
                },
            )
            .unwrap();
        std::fs::write(&source, b"second inbox bytes").unwrap();
        client
            .put_file(
                tb.clone(),
                "",
                &source,
                "same.bin",
                TransferOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(b.path().join("same.bin")).unwrap(),
            b"second inbox bytes"
        );
        assert_eq!(
            std::fs::read(a.path().join("same.bin")).unwrap(),
            b"first inbox bytes\0\xff"
        );
        let revision = store.host_snapshot(own).unwrap().revision;
        store
            .host_mutate(
                own,
                &owner,
                Mutation::RevokeService {
                    expected_revision: revision,
                    service: "inbox-a".into(),
                    peer,
                },
            )
            .unwrap();
        assert_eq!(
            client
                .put_file(ta, "", &source, "revoked.bin", TransferOptions::default())
                .await
                .unwrap_err()
                .code,
            "PERMISSION_DENIED"
        );
        client
            .put_file(
                tb,
                "incoming",
                &source,
                "still.bin",
                TransferOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(b.path().join("still.bin")).unwrap(),
            b"second inbox bytes"
        );
        assert!(std::fs::read_dir(shared.path()).unwrap().next().is_none());
        client.endpoint.close().await;
        host.endpoint().close().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_owner_inbox_commands_and_uploads_preserve_legacy_and_source_policies() {
    let state = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    private(state.path());
    let key = SecretKey::generate();
    let client = Client::new_peer(key, state.path().to_owned(), config())
        .await
        .unwrap();
    let before = client
        .command(&json!({"op":"local_files_status"}))
        .await
        .unwrap();
    let mut status = client
        .command(&json!({"op":"local_inboxes_status"}))
        .await
        .unwrap();
    for (id, root) in [("inbox-a", a.path()), ("inbox-b", b.path())] {
        client.command(&json!({"op":"local_inbox_configure","id":id,"root":root,"allow":[],"expected_revision":status["revision"]})).await.unwrap();
        status = client
            .command(&json!({"op":"local_inboxes_status"}))
            .await
            .unwrap();
    }
    assert_eq!(status["inboxes"].as_array().unwrap().len(), 2);
    let authority = Authority::open(state.path(), client.endpoint.id()).unwrap();
    let ta = target(&authority, "inbox-a").await;
    let source = local.path().join("source");
    std::fs::write(&source, b"local upload").unwrap();
    client
        .put_file(ta, "", &source, "chosen.bin", TransferOptions::default())
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(a.path().join("chosen.bin")).unwrap(),
        b"local upload"
    );
    assert!(!b.path().join("chosen.bin").exists());
    let after = client
        .command(&json!({"op":"local_files_status"}))
        .await
        .unwrap();
    assert_eq!(before["exports"], after["exports"]);
    assert_eq!(before["allow"], after["allow"]);
    assert!(client.command(&json!({"op":"local_share_remove","id":"inbox-a","expected_revision":status["revision"]})).await.is_err());
    client.command(&json!({"op":"local_inbox_remove","id":"inbox-a","expected_revision":status["revision"]})).await.unwrap();
    assert!(a.path().join("chosen.bin").exists());
    client.endpoint.close().await;
}
