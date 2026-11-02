#![cfg(unix)]

use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use wispnet_client::Client;
use wispnet_core::identity::SecretKey;
use wispnet_registry::host::{Mutation, owner_store};

#[tokio::test]
async fn local_folder_rename_preserves_capability_grants_and_pause_atomically() {
    for inbox in [true, false] {
        let state = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let key = SecretKey::generate();
        let own = key.public();
        let peer = SecretKey::generate().public();
        let client = Client::new_peer(
            key,
            state.path().to_owned(),
            serde_json::from_value(json!({"bind_addr":"127.0.0.1:0"})).unwrap(),
        )
        .await
        .unwrap();
        let (mut store, owner) = owner_store(state.path(), own).unwrap();
        store
            .host_mutate(
                own,
                &owner,
                Mutation::Approve {
                    expected_revision: 0,
                    peer,
                    name: "laptop".into(),
                },
            )
            .unwrap();
        let (configure, status_op, services_key) = if inbox {
            ("local_inbox_configure", "local_inboxes_status", "inboxes")
        } else {
            ("local_share_configure", "local_shares_status", "shares")
        };
        let root = folder.path().canonicalize().unwrap();
        client
            .command(&json!({"op":configure,"expected_revision":1,
                "id":"original","root":root,"allow":[peer]}))
            .await
            .unwrap();
        let before = client.command(&json!({"op":status_op})).await.unwrap();
        let original = &before[services_key][0];
        client
            .command(&json!({"op":"local_service_enabled","expected_revision":2,
                "id":"original","incarnation":original["incarnation"],"enabled":false}))
            .await
            .unwrap();
        let renamed = client
            .command(&json!({"op":configure,"expected_revision":3,
                "id":"original","new_id":"renamed","root":root,"allow":[peer]}))
            .await
            .unwrap();
        assert_eq!(renamed["revision"], 4);
        let after = client.command(&json!({"op":status_op})).await.unwrap();
        assert_eq!(after[services_key].as_array().unwrap().len(), 1);
        let service = &after[services_key][0];
        assert_eq!(service["id"], "renamed");
        assert_eq!(service["kind"], original["kind"]);
        assert_eq!(service["allow"], json!([peer]));
        assert_eq!(service["enabled"], false);
        assert_ne!(service["incarnation"], original["incarnation"]);
        assert_eq!(after["roots"], json!({"renamed":root}));

        client
            .command(&json!({"op":configure,"expected_revision":4,
                "id":"existing","root":root,"allow":[]}))
            .await
            .unwrap();
        let baseline = client.command(&json!({"op":status_op})).await.unwrap();
        for rejected in [
            json!({"op":configure,"expected_revision":5,"id":"renamed", "new_id":"existing","root":root,"allow":[peer]}),
            json!({"op":configure,"expected_revision":4,"id":"renamed", "new_id":"stale","root":root,"allow":[peer]}),
            json!({"op":configure,"expected_revision":5,"id":"renamed", "new_id":"invalid-root","root":root.join("missing"),"allow":[peer]}),
            json!({"op":if inbox {"local_share_configure"} else {"local_inbox_configure"},"expected_revision":5,"id":"renamed", "new_id":"wrong-mode","root":root,"allow":[peer]}),
        ] {
            assert!(client.command(&rejected).await.is_err());
            assert_eq!(
                client.command(&json!({"op":status_op})).await.unwrap(),
                baseline,
            );
        }
        client.endpoint.close().await;
    }
}
