use std::{collections::BTreeMap, path::PathBuf};
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
async fn inbox_folders_have_independent_roots_permissions_and_lifetimes() {
    let state = directory();
    let first = directory();
    let second = directory();
    let host = Identity::generate().endpoint_id();
    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();
    host::initialize(state.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    let mut revision = 0;
    for peer in [alice, bob] {
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
    for (id, root, peer) in [
        ("datasets", first.path(), alice),
        ("checkpoints", second.path(), bob),
    ] {
        revision = store
            .host_mutate(
                host,
                &owner,
                Mutation::FileInbox {
                    expected_revision: revision,
                    id: id.into(),
                    root: root.canonicalize().unwrap(),
                    allow: vec![peer],
                },
            )
            .unwrap();
    }
    let snapshot = store.host_snapshot(host).unwrap();
    assert!(snapshot.file_exports.is_empty());
    assert_eq!(snapshot.file_inboxes.len(), 2);
    let incarnation = snapshot
        .services
        .iter()
        .find(|s| s.id == "datasets")
        .unwrap()
        .incarnation
        .clone();
    assert!(
        snapshot
            .services
            .iter()
            .filter(|s| s.kind == ServiceKind::FileInbox)
            .all(|s| s.backend.to_string() == "127.0.0.1:0")
    );
    let authority = host::Authority::open(state.path(), host).unwrap();
    assert_eq!(
        authority.catalog(alice).await.unwrap().services[0].id,
        "datasets"
    );
    assert_eq!(
        authority.catalog(bob).await.unwrap().services[0].id,
        "checkpoints"
    );
    let serialized = serde_json::to_string(&authority.catalog(alice).await.unwrap()).unwrap();
    assert!(!serialized.contains(first.path().to_str().unwrap()));
    assert!(!serialized.contains("file_inboxes"));
    assert!(
        !authority
            .services()
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == "files")
    );
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::FileInbox {
                expected_revision: revision,
                id: "datasets".into(),
                root: first.path().canonicalize().unwrap(),
                allow: vec![alice, bob],
            },
        )
        .unwrap();
    assert_eq!(
        store
            .host_snapshot(host)
            .unwrap()
            .services
            .iter()
            .find(|s| s.id == "datasets")
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
                service: "datasets".into(),
                peer: alice,
            },
        )
        .unwrap();
    assert!(authority.catalog(alice).await.unwrap().services.is_empty());
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::FileInbox {
                expected_revision: revision,
                id: "datasets".into(),
                root: second.path().canonicalize().unwrap(),
                allow: vec![bob],
            },
        )
        .unwrap();
    let snapshot = store.host_snapshot(host).unwrap();
    let service = snapshot
        .services
        .iter()
        .find(|s| s.id == "datasets")
        .unwrap();
    assert_ne!(service.incarnation, incarnation);
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::SetServiceEnabled {
                expected_revision: revision,
                id: "datasets".into(),
                incarnation: service.incarnation.clone(),
                enabled: false,
            },
        )
        .unwrap();
    assert_eq!(authority.catalog(bob).await.unwrap().services.len(), 1);
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Remove {
                expected_revision: revision,
                id: "datasets".into(),
            },
        )
        .unwrap();
    assert!(
        !store
            .host_snapshot(host)
            .unwrap()
            .file_inboxes
            .contains_key("datasets")
    );
}
#[test]
fn inbox_configuration_cannot_convert_services_or_export_unsafe_roots() {
    let state = directory();
    let folder = directory();
    let host = Identity::generate().endpoint_id();
    host::initialize(state.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    let linked = folder.path().join("linked");
    std::os::unix::fs::symlink(state.path(), &linked).unwrap();
    let file = folder.path().join("file");
    std::fs::write(&file, b"bytes").unwrap();
    for root in [
        PathBuf::from("relative"),
        linked,
        file,
        state.path().into(),
        state.path().parent().unwrap().into(),
        folder.path().join("missing"),
    ] {
        assert!(matches!(
            store.host_mutate(
                host,
                &owner,
                Mutation::FileInbox {
                    expected_revision: 0,
                    id: "incoming".into(),
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
            Mutation::FileInbox {
                expected_revision: 0,
                id: "incoming".into(),
                root: folder.path().canonicalize().unwrap(),
                allow: vec![],
            },
        )
        .unwrap();
    for request in [
        serde_json::json!({"op":"service","expected_revision":1,"id":"incoming","backend":"127.0.0.1:22","allow":[]}),
        serde_json::json!({"op":"service","expected_revision":1,"id":"other","backend":"127.0.0.1:0","kind":"file_inbox","allow":[]}),
        serde_json::json!({"op":"edit_service","expected_revision":1,"id":"incoming","new_id":"other","backend":"127.0.0.1:22"}),
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
            Mutation::FileShare {
                expected_revision: 1,
                id: "incoming".into(),
                root: folder.path().canonicalize().unwrap(),
                allow: vec![]
            }
        ),
        Err(ControlError::Conflict)
    ));
    assert_eq!(store.host_snapshot(host).unwrap().revision, 1);
}
#[tokio::test]
async fn schema_seventeen_migrates_exports_without_global_grant_bypass_or_identity_changes() {
    let state = directory();
    let first = directory();
    let second = directory();
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
                name: "laptop".into(),
            },
        )
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Service {
                expected_revision: 1,
                id: "incoming".into(),
                backend: "127.0.0.1:80".parse().unwrap(),
                allow: vec![peer],
                display_name: None,
                kind: None,
            },
        )
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Files {
                expected_revision: 2,
                allow: vec![peer],
                exports: BTreeMap::from([
                    ("incoming".into(), first.path().canonicalize().unwrap()),
                    ("models".into(), second.path().canonicalize().unwrap()),
                ]),
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
    db.execute_batch("DROP TABLE host_file_inboxes; PRAGMA user_version=16;")
        .unwrap();
    drop(db);
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    let migrated = store.host_snapshot(host).unwrap();
    assert_eq!(migrated.host, before.host);
    assert_eq!(migrated.generation, before.generation);
    assert_eq!(migrated.revision, before.revision + 1);
    assert_eq!(migrated.clients[0].peer, peer);
    assert!(migrated.file_exports.is_empty());
    assert_eq!(
        migrated.file_inboxes["inbox-incoming"],
        first.path().canonicalize().unwrap()
    );
    assert_eq!(
        migrated.file_inboxes["models"],
        second.path().canonicalize().unwrap()
    );
    let legacy = migrated.services.iter().find(|s| s.id == "files").unwrap();
    assert!(legacy.allow.is_empty());
    assert_ne!(legacy.incarnation, old_inc);
    assert_eq!(
        migrated
            .services
            .iter()
            .find(|s| s.id == "incoming")
            .unwrap()
            .kind,
        ServiceKind::Http
    );
    assert!(
        migrated
            .services
            .iter()
            .filter(|s| s.kind == ServiceKind::FileInbox)
            .all(|s| s.allow == vec![peer] && s.enabled && s.incarnation != old_inc)
    );
    let revision = migrated.revision;
    store
        .host_mutate(
            host,
            &owner,
            Mutation::RevokeService {
                expected_revision: revision,
                service: "models".into(),
                peer,
            },
        )
        .unwrap();
    drop(store);
    let authority = host::Authority::open(state.path(), host).unwrap();
    let catalog = authority.catalog(peer).await.unwrap();
    assert!(catalog.services.iter().any(|s| s.id == "inbox-incoming"));
    assert!(
        !catalog
            .services
            .iter()
            .any(|s| s.id == "models" || s.id == "files")
    );
    assert!(catalog.valid(host));
    let (mut store, _) = host::owner_store(state.path(), host).unwrap();
    assert_eq!(store.host_snapshot(host).unwrap().revision, revision + 1);
}

#[tokio::test]
async fn migration_preserves_full_capacity_and_existing_edits_without_allowing_more_services() {
    let state = directory();
    let folder = directory();
    let other = directory();
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    host::initialize(state.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    let mut revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer,
                name: "laptop".into(),
            },
        )
        .unwrap();
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::FileShare {
                expected_revision: revision,
                id: "source".into(),
                root: folder.path().canonicalize().unwrap(),
                allow: vec![peer],
            },
        )
        .unwrap();
    for index in 1..128 {
        revision = store
            .host_mutate(
                host,
                &owner,
                Mutation::Service {
                    expected_revision: revision,
                    id: format!("svc-{index:03}"),
                    backend: "127.0.0.1:80".parse().unwrap(),
                    allow: vec![peer],
                    display_name: None,
                    kind: None,
                },
            )
            .unwrap();
    }
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::Files {
                expected_revision: revision,
                allow: vec![peer],
                exports: (0..32)
                    .map(|index| {
                        (
                            format!("inbox-{index:02}"),
                            folder.path().canonicalize().unwrap(),
                        )
                    })
                    .collect(),
            },
        )
        .unwrap();
    drop(store);
    let db = rusqlite::Connection::open(state.path().join("host.sqlite3")).unwrap();
    db.execute_batch("DROP TABLE host_file_inboxes;PRAGMA user_version=16;")
        .unwrap();
    drop(db);
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    let migrated = store.host_snapshot(host).unwrap();
    assert_eq!(migrated.services.len(), 162);
    assert_eq!(migrated.file_inboxes.len(), 32);
    assert_eq!(migrated.revision, revision + 1);
    revision = migrated.revision;
    let authority = host::Authority::open(state.path(), host).unwrap();
    let catalog = authority.catalog(peer).await.unwrap();
    assert_eq!(catalog.services.len(), 160);
    assert!(catalog.valid(host));
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::FileInbox {
                expected_revision: revision,
                id: "inbox-00".into(),
                root: other.path().canonicalize().unwrap(),
                allow: vec![peer],
            },
        )
        .unwrap();
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::FileShare {
                expected_revision: revision,
                id: "source".into(),
                root: other.path().canonicalize().unwrap(),
                allow: vec![peer],
            },
        )
        .unwrap();
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::Service {
                expected_revision: revision,
                id: "svc-001".into(),
                backend: "127.0.0.1:8080".parse().unwrap(),
                allow: vec![peer],
                display_name: None,
                kind: None,
            },
        )
        .unwrap();
    assert!(matches!(
        store.host_mutate(
            host,
            &owner,
            Mutation::FileInbox {
                expected_revision: revision,
                id: "extra-inbox".into(),
                root: other.path().canonicalize().unwrap(),
                allow: vec![]
            }
        ),
        Err(ControlError::Invalid)
    ));
    assert!(matches!(
        store.host_mutate(
            host,
            &owner,
            Mutation::FileShare {
                expected_revision: revision,
                id: "extra-share".into(),
                root: other.path().canonicalize().unwrap(),
                allow: vec![]
            }
        ),
        Err(ControlError::Invalid)
    ));
    assert!(matches!(
        store.host_mutate(
            host,
            &owner,
            Mutation::Service {
                expected_revision: revision,
                id: "extra-http".into(),
                backend: "127.0.0.1:80".parse().unwrap(),
                allow: vec![],
                display_name: None,
                kind: None
            }
        ),
        Err(ControlError::Invalid)
    ));
}

#[test]
fn migration_keeps_a_paused_legacy_inbox_paused() {
    let state = directory();
    let folder = directory();
    let host = Identity::generate().endpoint_id();
    host::initialize(state.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Files {
                expected_revision: 0,
                allow: vec![],
                exports: BTreeMap::from([(
                    "incoming".into(),
                    folder.path().canonicalize().unwrap(),
                )]),
            },
        )
        .unwrap();
    drop(store);
    let db = rusqlite::Connection::open(state.path().join("host.sqlite3")).unwrap();
    db.execute_batch("UPDATE host_services SET enabled=0 WHERE id='files';DROP TABLE host_file_inboxes;PRAGMA user_version=16;").unwrap();
    drop(db);
    let (mut store, _) = host::owner_store(state.path(), host).unwrap();
    let snapshot = store.host_snapshot(host).unwrap();
    assert!(
        !snapshot
            .services
            .iter()
            .find(|s| s.id == "incoming")
            .unwrap()
            .enabled
    );
    assert!(snapshot.file_exports.is_empty());
}

#[test]
fn migration_bounds_reserved_and_colliding_aliases_and_keeps_unavailable_folder_references() {
    let state = directory();
    let folder = directory();
    let removed = directory();
    let missing = removed.path().canonicalize().unwrap();
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    let long_alias = "a".repeat(64);
    let fallback = format!("inbox-{}", "a".repeat(48));
    std::fs::write(folder.path().join("existing-checkpoint"), b"retained").unwrap();
    host::initialize(state.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    let mut revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer,
                name: "laptop".into(),
            },
        )
        .unwrap();
    for id in [long_alias.clone(), fallback.clone(), "inbox-files".into()] {
        revision = store
            .host_mutate(
                host,
                &owner,
                Mutation::Service {
                    expected_revision: revision,
                    id,
                    backend: "127.0.0.1:80".parse().unwrap(),
                    allow: vec![],
                    display_name: None,
                    kind: None,
                },
            )
            .unwrap();
    }
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Files {
                expected_revision: revision,
                allow: vec![peer],
                exports: BTreeMap::from([
                    ("files".into(), folder.path().canonicalize().unwrap()),
                    ("shell".into(), folder.path().canonicalize().unwrap()),
                    (long_alias, folder.path().canonicalize().unwrap()),
                    ("removable".into(), missing.clone()),
                ]),
            },
        )
        .unwrap();
    drop(store);
    drop(removed);
    let db = rusqlite::Connection::open(state.path().join("host.sqlite3")).unwrap();
    db.execute_batch("DROP TABLE host_file_inboxes;PRAGMA user_version=16;")
        .unwrap();
    drop(db);
    let (mut store, _) = host::owner_store(state.path(), host).unwrap();
    let snapshot = store.host_snapshot(host).unwrap();
    for id in [
        "inbox-files-1".to_owned(),
        "inbox-shell".into(),
        format!("{fallback}-1"),
        "removable".into(),
    ] {
        assert!(wispnet_core::protocol::valid_service_id(&id));
        assert!(snapshot.file_inboxes.contains_key(&id));
        assert_eq!(
            snapshot.services.iter().find(|s| s.id == id).unwrap().allow,
            vec![peer]
        );
    }
    assert_eq!(snapshot.file_inboxes["removable"], missing);
    assert!(!missing.exists());
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
    assert_eq!(
        std::fs::read(folder.path().join("existing-checkpoint")).unwrap(),
        b"retained"
    );
}

#[tokio::test]
async fn folder_rename_preserves_mode_grants_and_pause_and_rolls_back_invalid_edits() {
    for inbox in [true, false] {
        let state = directory();
        let folder = directory();
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
                    name: "client".into(),
                },
            )
            .unwrap();
        let root = folder.path().canonicalize().unwrap();
        let create = if inbox {
            Mutation::FileInbox {
                expected_revision: 1,
                id: "original".into(),
                root: root.clone(),
                allow: vec![peer],
            }
        } else {
            Mutation::FileShare {
                expected_revision: 1,
                id: "original".into(),
                root: root.clone(),
                allow: vec![peer],
            }
        };
        store.host_mutate(host, &owner, create).unwrap();
        let before = store.host_snapshot(host).unwrap();
        let original = before.services.iter().find(|s| s.id == "original").unwrap();
        store
            .host_mutate(
                host,
                &owner,
                Mutation::SetServiceEnabled {
                    expected_revision: 2,
                    id: "original".into(),
                    incarnation: original.incarnation.clone(),
                    enabled: false,
                },
            )
            .unwrap();
        store
            .host_mutate(
                host,
                &owner,
                Mutation::EditFolder {
                    expected_revision: 3,
                    id: "original".into(),
                    new_id: "renamed".into(),
                    root: root.clone(),
                    allow: vec![peer],
                },
            )
            .unwrap();
        let after = store.host_snapshot(host).unwrap();
        let renamed = after.services.iter().find(|s| s.id == "renamed").unwrap();
        assert_eq!(renamed.kind, original.kind);
        assert_eq!(renamed.allow, vec![peer]);
        assert!(!renamed.enabled);
        assert_ne!(renamed.incarnation, original.incarnation);
        assert!(!after.services.iter().any(|s| s.id == "original"));
        let roots = if inbox {
            &after.file_inboxes
        } else {
            &after.file_shares
        };
        assert_eq!(roots.get("renamed"), Some(&root));
        assert!(!roots.contains_key("original"));
        for (new_id, allow) in [
            ("shell", vec![peer]),
            ("invalid-edit", vec![Identity::generate().endpoint_id()]),
        ] {
            assert!(
                store
                    .host_mutate(
                        host,
                        &owner,
                        Mutation::EditFolder {
                            expected_revision: 4,
                            id: "renamed".into(),
                            new_id: new_id.into(),
                            root: root.clone(),
                            allow
                        }
                    )
                    .is_err()
            );
            let unchanged = store.host_snapshot(host).unwrap();
            assert_eq!(unchanged.revision, 4);
            assert!(unchanged.services.iter().any(|s| s.id == "renamed"
                && s.incarnation == renamed.incarnation
                && !s.enabled
                && s.allow == vec![peer]));
            assert!(!unchanged.services.iter().any(|s| s.id == "invalid-edit"));
        }
    }
}
