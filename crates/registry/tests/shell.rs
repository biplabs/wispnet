use wispnet_core::{host::ServiceKind, identity::Identity};
use wispnet_registry::{
    control::ControlError,
    host::{self, Mutation},
};
fn directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}
#[test]
fn builtin_shell_is_permanent_reserved_and_remote_grants_are_explicit() {
    let dir = directory();
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    host::initialize(dir.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(dir.path(), host).unwrap();
    let snapshot = store.host_snapshot(host).unwrap();
    assert_eq!(snapshot.revision, 0);
    assert_eq!(snapshot.services.len(), 2);
    let builtin = snapshot.services.iter().find(|s| s.id == "shell").unwrap();
    assert_eq!(builtin.id, "shell");
    assert_eq!(builtin.kind, ServiceKind::Shell);
    assert_eq!(builtin.backend.to_string(), "127.0.0.1:0");
    assert!(builtin.allow.is_empty());
    let incarnation = builtin.incarnation.clone();
    for request in [
        serde_json::json!({"op":"service","expected_revision":0,"id":"shell","kind":"http","backend":"127.0.0.1:22","allow":[]}),
        serde_json::json!({"op":"service","expected_revision":0,"id":"other","kind":"shell","backend":"127.0.0.1:0","allow":[]}),
        serde_json::json!({"op":"edit_service","expected_revision":0,"id":"shell","new_id":"other","backend":"127.0.0.1:22"}),
        serde_json::json!({"op":"remove","expected_revision":0,"id":"shell"}),
    ] {
        assert!(matches!(
            store.host_mutate(host, &owner, serde_json::from_value(request).unwrap()),
            Err(ControlError::Invalid)
        ));
    }
    assert!(matches!(
        store.host_mutate(
            host,
            &owner,
            Mutation::Shell {
                expected_revision: 0,
                allow: vec![peer]
            }
        ),
        Err(ControlError::Forbidden)
    ));
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer,
                name: "client".into(),
            },
        )
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Shell {
                expected_revision: 1,
                allow: vec![peer],
            },
        )
        .unwrap();
    assert_eq!(
        store
            .host_snapshot(host)
            .unwrap()
            .services
            .into_iter()
            .find(|s| s.id == "shell")
            .unwrap()
            .allow,
        vec![peer]
    );
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Shell {
                expected_revision: 2,
                allow: vec![],
            },
        )
        .unwrap();
    drop(store);
    let (mut store, _) = host::owner_store(dir.path(), host).unwrap();
    let after = store.host_snapshot(host).unwrap();
    assert_eq!(after.revision, 3);
    assert_eq!(
        after
            .services
            .iter()
            .find(|s| s.id == "shell")
            .unwrap()
            .incarnation,
        incarnation
    );
    assert!(
        after
            .services
            .iter()
            .find(|s| s.id == "shell")
            .unwrap()
            .allow
            .is_empty()
    );
}
#[test]
fn schema_twelve_preserves_legacy_shell_service_and_adds_builtin_once() {
    let dir = directory();
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    host::initialize(dir.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(dir.path(), host).unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer,
                name: "client".into(),
            },
        )
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Shell {
                expected_revision: 1,
                allow: vec![peer],
            },
        )
        .unwrap();
    let old = store.host_snapshot(host).unwrap();
    let old_inc = old
        .services
        .iter()
        .find(|s| s.id == "shell")
        .unwrap()
        .incarnation
        .clone();
    drop(store);
    let db = rusqlite::Connection::open(dir.path().join("host.sqlite3")).unwrap();
    db.execute_batch("UPDATE host_services SET kind='http',backend='127.0.0.1:88' WHERE id='shell'; INSERT INTO host_services(id,backend,incarnation,kind) VALUES('shell-legacy','127.0.0.1:99',lower(hex(randomblob(32))),'tcp'); PRAGMA user_version=11;").unwrap();
    drop(db);
    let (mut store, _) = host::owner_store(dir.path(), host).unwrap();
    let migrated = store.host_snapshot(host).unwrap();
    assert_eq!(migrated.revision, old.revision + 1);
    assert_eq!(migrated.services.len(), 4);
    let builtin = migrated.services.iter().find(|s| s.id == "shell").unwrap();
    assert_eq!(builtin.kind, ServiceKind::Shell);
    assert!(builtin.allow.is_empty());
    assert_ne!(builtin.incarnation, old_inc);
    let preserved = migrated
        .services
        .iter()
        .find(|s| s.id == "shell-legacy-1")
        .unwrap();
    assert_eq!(preserved.kind, ServiceKind::Http);
    assert_eq!(preserved.incarnation, old_inc);
    assert_eq!(preserved.allow, vec![peer]);
    assert_eq!(preserved.backend.to_string(), "127.0.0.1:88");
    let inc = builtin.incarnation.clone();
    drop(store);
    let (mut store, _) = host::owner_store(dir.path(), host).unwrap();
    let again = store.host_snapshot(host).unwrap();
    assert_eq!(again.revision, migrated.revision);
    assert_eq!(
        again
            .services
            .iter()
            .find(|s| s.id == "shell")
            .unwrap()
            .incarnation,
        inc
    );
}

#[tokio::test]
async fn builtins_reserve_their_own_slots_beside_128_custom_services() {
    let dir = directory();
    let workspace = directory();
    let own = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    host::initialize(dir.path(), own).unwrap();
    let (mut store, owner) = host::owner_store(dir.path(), own).unwrap();
    store
        .host_mutate(
            own,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer,
                name: "client".into(),
            },
        )
        .unwrap();
    for n in 0..128 {
        store
            .host_mutate(
                own,
                &owner,
                Mutation::Service {
                    expected_revision: n + 1,
                    id: format!("service-{n:03}"),
                    backend: "127.0.0.1:99".parse().unwrap(),
                    allow: vec![peer],
                    display_name: None,
                    kind: None,
                },
            )
            .unwrap();
    }
    store
        .host_mutate(
            own,
            &owner,
            Mutation::Shell {
                expected_revision: 129,
                allow: vec![peer],
            },
        )
        .unwrap();
    store
        .host_mutate(
            own,
            &owner,
            Mutation::Files {
                expected_revision: 130,
                allow: vec![peer],
                exports: std::collections::BTreeMap::from([(
                    "shared".into(),
                    workspace.path().canonicalize().unwrap(),
                )]),
            },
        )
        .unwrap();
    drop(store);
    let authority = host::Authority::open(dir.path(), own).unwrap();
    let catalog = authority.catalog(peer).await.unwrap();
    assert_eq!(catalog.services.len(), 130);
    assert!(catalog.valid(own));
}
