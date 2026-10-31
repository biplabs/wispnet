use std::{collections::BTreeMap, path::PathBuf};
use wispnet_core::{host::ServiceKind, identity::Identity};
use wispnet_registry::{
    control::ControlError,
    host::{self, Mutation},
};

fn directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

#[tokio::test]
async fn files_exports_grants_and_incarnations_are_durable_and_owner_local() {
    let state = directory();
    let workspace = directory();
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    host::initialize(state.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    let initial = store.host_snapshot(host).unwrap();
    assert_eq!(initial.revision, 0);
    let builtin = initial.services.iter().find(|s| s.id == "files").unwrap();
    assert_eq!(builtin.kind, ServiceKind::Files);
    assert_eq!(builtin.backend.to_string(), "127.0.0.1:0");
    assert!(builtin.allow.is_empty());
    assert!(initial.file_exports.is_empty());
    let initial_inc = builtin.incarnation.clone();
    let exports = BTreeMap::from([("training".into(), workspace.path().canonicalize().unwrap())]);
    assert!(matches!(
        store.host_mutate(
            host,
            &owner,
            Mutation::Files {
                expected_revision: 0,
                allow: vec![peer],
                exports: exports.clone()
            }
        ),
        Err(ControlError::Forbidden)
    ));
    assert!(store.host_snapshot(host).unwrap().file_exports.is_empty());
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer,
                name: "worker".into(),
            },
        )
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Files {
                expected_revision: 1,
                allow: vec![peer],
                exports: exports.clone(),
            },
        )
        .unwrap();
    let configured = store.host_snapshot(host).unwrap();
    let configured_inc = configured
        .services
        .iter()
        .find(|s| s.id == "files")
        .unwrap()
        .incarnation
        .clone();
    assert_ne!(initial_inc, configured_inc);
    assert_eq!(configured.file_exports, exports);
    // Grant-only edits keep the content/target incarnation stable.
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Files {
                expected_revision: 2,
                allow: vec![],
                exports: exports.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        store
            .host_snapshot(host)
            .unwrap()
            .services
            .iter()
            .find(|s| s.id == "files")
            .unwrap()
            .incarnation,
        configured_inc
    );
    store
        .host_mutate(
            host,
            &owner,
            Mutation::GrantService {
                expected_revision: 3,
                service: "files".into(),
                peer,
            },
        )
        .unwrap();
    drop(store);
    let authority = host::Authority::open(state.path(), host).unwrap();
    assert_eq!(authority.state_dir(), state.path());
    assert_eq!(authority.file_exports().await.unwrap(), exports);
    let catalog = authority.catalog(peer).await.unwrap();
    assert_eq!(catalog.services.len(), 1);
    assert_eq!(catalog.services[0].kind, ServiceKind::Files);
    let remote = serde_json::to_string(&catalog).unwrap();
    assert!(!remote.contains(workspace.path().to_str().unwrap()));
    assert!(!remote.contains("file_exports"));
    assert_eq!(catalog.services[0].incarnation, configured_inc);
}

#[test]
fn files_name_and_exports_fail_closed_without_mutating_authority() {
    let state = directory();
    let workspace = directory();
    let host = Identity::generate().endpoint_id();
    host::initialize(state.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    for request in [
        serde_json::json!({"op":"service","expected_revision":0,"id":"files","kind":"http","backend":"127.0.0.1:22","allow":[]}),
        serde_json::json!({"op":"service","expected_revision":0,"id":"other","kind":"files","backend":"127.0.0.1:0","allow":[]}),
        serde_json::json!({"op":"edit_service","expected_revision":0,"id":"files","new_id":"other","backend":"127.0.0.1:22"}),
        serde_json::json!({"op":"remove","expected_revision":0,"id":"files"}),
    ] {
        assert!(matches!(
            store.host_mutate(host, &owner, serde_json::from_value(request).unwrap()),
            Err(ControlError::Invalid)
        ));
    }
    let link = workspace.path().join("linked");
    std::os::unix::fs::symlink(state.path(), &link).unwrap();
    for (name, root) in [
        ("training", PathBuf::from("relative")),
        ("../training", workspace.path().canonicalize().unwrap()),
        ("training", workspace.path().join("missing")),
        ("training", link),
        ("training", state.path().to_path_buf()),
        ("training", state.path().parent().unwrap().to_path_buf()),
    ] {
        assert!(matches!(
            store.host_mutate(
                host,
                &owner,
                Mutation::Files {
                    expected_revision: 0,
                    allow: vec![],
                    exports: BTreeMap::from([(name.into(), root)])
                }
            ),
            Err(ControlError::Invalid)
        ));
    }
    let after = store.host_snapshot(host).unwrap();
    assert_eq!(after.revision, 0);
    assert!(after.file_exports.is_empty());
    assert!(after.services.iter().all(|s| s.allow.is_empty()));
}

#[test]
fn schema_thirteen_preserves_legacy_files_service_and_grants_once() {
    let state = directory();
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    host::initialize(state.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer,
                name: "worker".into(),
            },
        )
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::GrantService {
                expected_revision: 1,
                service: "files".into(),
                peer,
            },
        )
        .unwrap();
    let before = store.host_snapshot(host).unwrap();
    let old_inc = before
        .services
        .iter()
        .find(|s| s.id == "files")
        .unwrap()
        .incarnation
        .clone();
    drop(store);
    let db = rusqlite::Connection::open(state.path().join("host.sqlite3")).unwrap();
    db.execute_batch("UPDATE host_services SET kind='http',backend='127.0.0.1:88' WHERE id='files'; INSERT INTO host_services(id,backend,incarnation,kind) VALUES('files-legacy','127.0.0.1:99',lower(hex(randomblob(32))),'tcp'); DROP TABLE host_file_exports; PRAGMA user_version=12;").unwrap();
    drop(db);
    let (mut store, _) = host::owner_store(state.path(), host).unwrap();
    let after = store.host_snapshot(host).unwrap();
    assert_eq!(after.revision, before.revision + 1);
    let builtin = after.services.iter().find(|s| s.id == "files").unwrap();
    assert_eq!(builtin.kind, ServiceKind::Files);
    assert!(builtin.allow.is_empty());
    assert_ne!(builtin.incarnation, old_inc);
    let legacy = after
        .services
        .iter()
        .find(|s| s.id == "files-legacy-1")
        .unwrap();
    assert_eq!(legacy.kind, ServiceKind::Http);
    assert_eq!(legacy.incarnation, old_inc);
    assert_eq!(legacy.allow, vec![peer]);
    assert_eq!(legacy.backend.to_string(), "127.0.0.1:88");
    assert!(after.file_exports.is_empty());
    let incarnation = builtin.incarnation.clone();
    drop(store);
    let (mut store, _) = host::owner_store(state.path(), host).unwrap();
    let reopened = store.host_snapshot(host).unwrap();
    assert_eq!(reopened.revision, after.revision);
    assert_eq!(
        reopened
            .services
            .iter()
            .find(|s| s.id == "files")
            .unwrap()
            .incarnation,
        incarnation
    );
}
