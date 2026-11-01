use std::path::PathBuf;
use wispnet_core::{host::ServiceKind, identity::Identity};
use wispnet_registry::{
    control::ControlError,
    host::{self, Mutation},
};

fn directory() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
#[tokio::test]
async fn shares_have_independent_roots_grants_and_incarnations_without_inbox_access() {
    let state = directory();
    let first = directory();
    let second = directory();
    let host = Identity::generate().endpoint_id();
    let laptop = Identity::generate().endpoint_id();
    let other = Identity::generate().endpoint_id();
    host::initialize(state.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    let mut revision = 0;
    for peer in [laptop, other] {
        revision = store
            .host_mutate(
                host,
                &owner,
                Mutation::Approve {
                    expected_revision: revision,
                    peer,
                    name: "client".into(),
                },
            )
            .unwrap();
    }
    let root = first.path().canonicalize().unwrap();
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::FileShare {
                expected_revision: revision,
                id: "checkpoints".into(),
                root: root.clone(),
                allow: vec![laptop],
            },
        )
        .unwrap();
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::FileShare {
                expected_revision: revision,
                id: "other-data".into(),
                root: second.path().canonicalize().unwrap(),
                allow: vec![other],
            },
        )
        .unwrap();
    let snapshot = store.host_snapshot(host).unwrap();
    assert!(snapshot.file_exports.is_empty());
    assert!(
        snapshot
            .services
            .iter()
            .find(|s| s.id == "files")
            .unwrap()
            .allow
            .is_empty()
    );
    let share = snapshot
        .services
        .iter()
        .find(|s| s.id == "checkpoints")
        .unwrap();
    assert_eq!(share.kind, ServiceKind::FileShare);
    assert_eq!(share.backend.to_string(), "127.0.0.1:0");
    let incarnation = share.incarnation.clone();
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::FileShare {
                expected_revision: revision,
                id: "checkpoints".into(),
                root: root.clone(),
                allow: vec![laptop, other],
            },
        )
        .unwrap();
    assert_eq!(
        store
            .host_snapshot(host)
            .unwrap()
            .services
            .iter()
            .find(|s| s.id == "checkpoints")
            .unwrap()
            .incarnation,
        incarnation
    );
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::RevokeService {
                expected_revision: revision,
                service: "checkpoints".into(),
                peer: other,
            },
        )
        .unwrap();
    drop(store);
    let authority = host::Authority::open(state.path(), host).unwrap();
    let catalog = authority.catalog(laptop).await.unwrap();
    assert_eq!(catalog.services.len(), 1);
    assert_eq!(catalog.services[0].id, "checkpoints");
    assert_eq!(catalog.services[0].kind, ServiceKind::FileShare);
    let serialized = serde_json::to_string(&catalog).unwrap();
    assert!(!serialized.contains(root.to_str().unwrap()));
    assert!(!serialized.contains("file_shares"));
    assert_eq!(
        authority.catalog(other).await.unwrap().services[0].id,
        "other-data"
    );
    let (mut store, _) = host::owner_store(state.path(), host).unwrap();
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::FileShare {
                expected_revision: revision,
                id: "checkpoints".into(),
                root: second.path().canonicalize().unwrap(),
                allow: vec![laptop],
            },
        )
        .unwrap();
    let snapshot = store.host_snapshot(host).unwrap();
    let share = snapshot
        .services
        .iter()
        .find(|s| s.id == "checkpoints")
        .unwrap();
    assert_ne!(share.incarnation, incarnation);
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::SetServiceEnabled {
                expected_revision: revision,
                id: share.id.clone(),
                incarnation: share.incarnation.clone(),
                enabled: false,
            },
        )
        .unwrap();
    assert!(authority.catalog(laptop).await.unwrap().services.is_empty());
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Remove {
                expected_revision: revision,
                id: "checkpoints".into(),
            },
        )
        .unwrap();
    assert!(
        !store
            .host_snapshot(host)
            .unwrap()
            .file_shares
            .contains_key("checkpoints")
    );
}
#[test]
fn share_roots_and_generic_service_edits_fail_closed() {
    let state = directory();
    let workspace = directory();
    let host = Identity::generate().endpoint_id();
    host::initialize(state.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    let link = workspace.path().join("link");
    std::os::unix::fs::symlink(state.path(), &link).unwrap();
    for root in [
        PathBuf::from("relative"),
        workspace.path().join("missing"),
        link,
        state.path().to_path_buf(),
        state.path().parent().unwrap().to_path_buf(),
    ] {
        assert!(matches!(
            store.host_mutate(
                host,
                &owner,
                Mutation::FileShare {
                    expected_revision: 0,
                    id: "checkpoints".into(),
                    root,
                    allow: vec![]
                }
            ),
            Err(ControlError::Invalid)
        ));
    }
    store
        .host_mutate(
            host,
            &owner,
            Mutation::FileShare {
                expected_revision: 0,
                id: "checkpoints".into(),
                root: workspace.path().canonicalize().unwrap(),
                allow: vec![],
            },
        )
        .unwrap();
    for request in [
        serde_json::json!({"op":"service","expected_revision":1,"id":"checkpoints","backend":"127.0.0.1:22","allow":[]}),
        serde_json::json!({"op":"service","expected_revision":1,"id":"other","kind":"file_share","backend":"127.0.0.1:0","allow":[]}),
        serde_json::json!({"op":"edit_service","expected_revision":1,"id":"checkpoints","new_id":"other","backend":"127.0.0.1:22"}),
    ] {
        assert!(matches!(
            store.host_mutate(host, &owner, serde_json::from_value(request).unwrap()),
            Err(ControlError::Invalid)
        ));
    }
    assert_eq!(store.host_snapshot(host).unwrap().revision, 1);
    let selected = workspace.path().join("selected.pt");
    std::fs::write(&selected, b"weights").unwrap();
    assert!(matches!(
        store.host_mutate(
            host,
            &owner,
            Mutation::FileShare {
                expected_revision: 1,
                id: "selected".into(),
                root: selected,
                allow: vec![]
            }
        ),
        Err(ControlError::Invalid)
    ));
    assert_eq!(store.host_snapshot(host).unwrap().revision, 1);
}
