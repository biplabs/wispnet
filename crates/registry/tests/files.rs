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
    let workspace = directory();
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
            Mutation::Files {
                expected_revision: 1,
                allow: vec![peer],
                exports: BTreeMap::from([(
                    "training".into(),
                    workspace.path().canonicalize().unwrap(),
                )]),
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
    db.execute_batch("UPDATE host_services SET kind='http',backend='127.0.0.1:88' WHERE id='files'; INSERT INTO host_services(id,backend,incarnation,kind) VALUES('files-legacy','127.0.0.1:99',lower(hex(randomblob(32))),'tcp'); DROP TABLE host_file_exports; ALTER TABLE host_services DROP COLUMN enabled; PRAGMA user_version=12;").unwrap();
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

#[test]
fn folder_is_required_for_every_grant_path_and_clearing_it_revokes_access() {
    use wispnet_core::pairing::{Claim, Proof};
    let state = directory();
    let workspace = directory();
    let host_key = Identity::generate();
    let host = host_key.endpoint_id();
    let peer = Identity::generate().endpoint_id();
    let incoming = Identity::generate();
    host::initialize(state.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), host).unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer,
                name: "paired".into(),
            },
        )
        .unwrap();
    let now = wispnet_registry::enrollment::now();
    let invitation = store
        .pairing_invite(host, &owner, "https://relay.example", now, 300)
        .unwrap();
    let claim = Claim {
        invitation: invitation.invitation.clone(),
        code: invitation.code.clone(),
        name: "incoming".into(),
    };
    let challenge = store
        .pairing_challenge(host, incoming.endpoint_id(), &claim, now)
        .unwrap();
    let proof = Proof {
        signature: incoming.secret_key().sign(&challenge.signing_bytes()),
        challenge,
    };
    store
        .pairing_prove(host, incoming.endpoint_id(), &proof, now)
        .unwrap();
    for operation in [
        Mutation::Files {
            expected_revision: 1,
            allow: vec![peer],
            exports: BTreeMap::new(),
        },
        Mutation::GrantService {
            expected_revision: 1,
            service: "files".into(),
            peer,
        },
        Mutation::ApproveInvitation {
            expected_revision: 1,
            invitation: invitation.invitation.clone(),
            services: vec!["files".into()],
        },
    ] {
        assert!(matches!(
            store.host_mutate(host, &owner, operation),
            Err(ControlError::Invalid)
        ));
        let snapshot = store.host_snapshot(host).unwrap();
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.clients.len(), 1);
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
            store.pairing_list(host, &owner, now).unwrap()[0].state,
            "pending"
        );
    }
    let exports = BTreeMap::from([("shared".into(), workspace.path().canonicalize().unwrap())]);
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Files {
                expected_revision: 1,
                allow: vec![],
                exports: exports.clone(),
            },
        )
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::GrantService {
                expected_revision: 2,
                service: "files".into(),
                peer,
            },
        )
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::ApproveInvitation {
                expected_revision: 3,
                invitation: invitation.invitation.clone(),
                services: vec!["files".into()],
            },
        )
        .unwrap();
    let configured = store.host_snapshot(host).unwrap();
    let configured_files = configured
        .services
        .iter()
        .find(|s| s.id == "files")
        .unwrap();
    assert_eq!(configured_files.allow.len(), 2);
    let incarnation = configured_files.incarnation.clone();
    assert!(matches!(
        store.host_mutate(
            host,
            &owner,
            Mutation::Files {
                expected_revision: 4,
                allow: vec![peer],
                exports: BTreeMap::new()
            }
        ),
        Err(ControlError::Invalid)
    ));
    assert_eq!(store.host_snapshot(host).unwrap().revision, 4);
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Files {
                expected_revision: 4,
                allow: vec![],
                exports: BTreeMap::new(),
            },
        )
        .unwrap();
    let cleared = store.host_snapshot(host).unwrap();
    assert!(cleared.file_exports.is_empty());
    let cleared_files = cleared.services.iter().find(|s| s.id == "files").unwrap();
    assert!(cleared_files.allow.is_empty());
    assert_ne!(cleared_files.incarnation, incarnation);
    assert_eq!(cleared.clients.len(), 2);
}

#[test]
fn upgrade_revokes_only_legacy_files_grants_without_a_folder() {
    for configured in [false, true] {
        let state = directory();
        let workspace = directory();
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
                    name: "paired".into(),
                },
            )
            .unwrap();
        if configured {
            store
                .host_mutate(
                    host,
                    &owner,
                    Mutation::Files {
                        expected_revision: 1,
                        allow: vec![peer],
                        exports: BTreeMap::from([(
                            "shared".into(),
                            workspace.path().canonicalize().unwrap(),
                        )]),
                    },
                )
                .unwrap();
        }
        let before = store.host_snapshot(host).unwrap();
        drop(store);
        let db = rusqlite::Connection::open(state.path().join("host.sqlite3")).unwrap();
        if !configured {
            db.execute(
                "INSERT INTO host_grants VALUES('files',?1)",
                [peer.to_string()],
            )
            .unwrap();
        }
        db.execute_batch("ALTER TABLE host_services DROP COLUMN enabled;")
            .unwrap();
        db.pragma_update(None, "user_version", 13).unwrap();
        drop(db);
        let (mut store, _) = host::owner_store(state.path(), host).unwrap();
        let after = store.host_snapshot(host).unwrap();
        assert_eq!(after.revision, before.revision + u64::from(!configured));
        let files = after.services.iter().find(|s| s.id == "files").unwrap();
        assert_eq!(files.allow, if configured { vec![peer] } else { vec![] });
        assert_eq!(after.file_exports, before.file_exports);
        assert_eq!(
            files.incarnation,
            before
                .services
                .iter()
                .find(|s| s.id == "files")
                .unwrap()
                .incarnation
        );
        let revision = after.revision;
        drop(store);
        let (mut store, _) = host::owner_store(state.path(), host).unwrap();
        assert_eq!(store.host_snapshot(host).unwrap().revision, revision);
    }
}
