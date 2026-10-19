use std::os::unix::fs::PermissionsExt;
use wispnet_core::identity::Identity;
use wispnet_registry::{combined::Action, control, host, store::Store};

#[test]
fn receipts_survive_reopen_and_cannot_replay_approval_after_revocation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    host::initialize(dir.path(), host).unwrap();
    let (mut store, secret) = host::owner_store(dir.path(), host).unwrap();
    let approve = control::token();
    let revoke = control::token();
    let first = store
        .host_apply_operation(
            host,
            &secret,
            &approve,
            Action::Approve {
                peer,
                name: "test".into(),
            },
        )
        .unwrap();
    assert_eq!(first.state, "applied");
    assert_eq!(first.revision, Some(1));
    assert!(
        store
            .host_apply_operation(host, &secret, &approve, Action::Revoke { peer })
            .is_err()
    );
    let removed = store
        .host_apply_operation(host, &secret, &revoke, Action::Revoke { peer })
        .unwrap();
    assert_eq!(removed.revision, Some(2));
    drop(store);
    let mut store = Store::open(&dir.path().join("host.sqlite3")).unwrap();
    assert_eq!(
        store
            .host_apply_operation(
                host,
                &secret,
                &approve,
                Action::Approve {
                    peer,
                    name: "test".into()
                }
            )
            .unwrap(),
        first
    );
    assert_eq!(
        store
            .host_apply_operation(host, &secret, &revoke, Action::Revoke { peer })
            .unwrap(),
        removed
    );
    assert!(store.host_snapshot(host).unwrap().clients.is_empty());
    assert_eq!(store.host_snapshot(host).unwrap().revision, 2);
    let rejected = control::token();
    let no_client = store
        .host_apply_operation(host, &secret, &rejected, Action::Revoke { peer })
        .unwrap();
    assert_eq!(no_client.state, "rejected");
    assert_eq!(no_client.revision, None);
    assert_eq!(store.host_snapshot(host).unwrap().revision, 2);
    store
        .host_apply_operation(
            host,
            &secret,
            &control::token(),
            Action::Approve {
                peer,
                name: "fresh".into(),
            },
        )
        .unwrap();
    assert_eq!(
        store
            .host_apply_operation(host, &secret, &rejected, Action::Revoke { peer })
            .unwrap(),
        no_client
    );
    assert_eq!(store.host_snapshot(host).unwrap().clients.len(), 1);
    assert!(
        store
            .host_apply_operation(
                host,
                &control::token(),
                &control::token(),
                Action::Revoke { peer }
            )
            .is_err()
    );
    assert!(
        store
            .host_apply_operation(
                Identity::generate().endpoint_id(),
                &secret,
                &control::token(),
                Action::Revoke { peer }
            )
            .is_err()
    );
}

#[test]
fn failed_receipt_commit_rolls_back_the_authority_change() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    host::initialize(dir.path(), host).unwrap();
    let (mut store, secret) = host::owner_store(dir.path(), host).unwrap();
    let db = rusqlite::Connection::open(dir.path().join("host.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER receipt_failure BEFORE INSERT ON host_operation_receipts BEGIN SELECT RAISE(ABORT,'test fault'); END;").unwrap();
    let id = control::token();
    assert!(
        store
            .host_apply_operation(
                host,
                &secret,
                &id,
                Action::Approve {
                    peer,
                    name: "fixture".into()
                }
            )
            .is_err()
    );
    assert!(store.host_snapshot(host).unwrap().clients.is_empty());
    assert_eq!(store.host_snapshot(host).unwrap().revision, 0);
    db.execute_batch("DROP TRIGGER receipt_failure;").unwrap();
    let receipt = store
        .host_apply_operation(
            host,
            &secret,
            &id,
            Action::Approve {
                peer,
                name: "fixture".into(),
            },
        )
        .unwrap();
    assert_eq!(receipt.revision, Some(1));
    assert_eq!(store.host_snapshot(host).unwrap().clients.len(), 1);
}
