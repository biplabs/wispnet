use std::os::unix::fs::PermissionsExt;
use wispnet_core::identity::Identity;
use wispnet_registry::{
    control,
    host::{self, Mutation},
    store::Store,
};
// These legacy tests compare user-configured services. The permanent builtin
// is asserted independently in shell.rs and is not a user service fixture.
fn custom_snapshot(
    store: &mut Store,
    host: wispnet_core::identity::EndpointId,
) -> Result<host::Snapshot, control::ControlError> {
    let mut snapshot = store.host_snapshot(host)?;
    snapshot.services.retain(|s| {
        !matches!(
            s.kind,
            wispnet_core::host::ServiceKind::Shell | wispnet_core::host::ServiceKind::Files
        )
    });
    Ok(snapshot)
}
fn directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

#[tokio::test]
async fn service_rename_preserves_grants_pairings_and_incarnation_after_restart() {
    use serde_json::json;
    let host = Identity::generate().endpoint_id();
    let peers = [
        Identity::generate().endpoint_id(),
        Identity::generate().endpoint_id(),
        Identity::generate().endpoint_id(),
    ];
    let dir = directory();
    host::initialize(dir.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(dir.path(), host).unwrap();
    for peer in peers {
        store
            .pairing_accept_host(
                host,
                &owner,
                &wispnet_core::pairing::Bootstrap {
                    version: 1,
                    host: peer,
                    relay_url: "https://relay.example/".into(),
                    direct_hint: None,
                },
                "computer",
            )
            .unwrap();
    }
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Service {
                expected_revision: 3,
                id: "gike".into(),
                backend: "127.0.0.1:8310".parse().unwrap(),
                allow: peers[..2].to_vec(),
                display_name: None,
                kind: None,
            },
        )
        .unwrap();
    let before = custom_snapshot(&mut store, host).unwrap();
    let pairings = serde_json::to_value(store.paired_hosts(host).unwrap()).unwrap();
    // Omitting allow in the owner API preserves the authoritative grants.
    let rename = serde_json::from_value(json!({
        "op": "edit_service", "expected_revision": 4,
        "id": "gike", "new_id": "research",
        "backend": "127.0.0.1:8311"
    }))
    .unwrap();
    assert_eq!(store.host_mutate(host, &owner, rename).unwrap(), 5);
    drop(store);
    let (mut store, _) = host::owner_store(dir.path(), host).unwrap();
    let after = custom_snapshot(&mut store, host).unwrap();
    assert_eq!(after.host, before.host);
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.revision, 5);
    assert_eq!(after.services.len(), 1);
    assert_eq!(after.services[0].id, "research");
    assert_eq!(after.services[0].display_name, "research");
    assert_eq!(after.services[0].backend, "127.0.0.1:8311".parse().unwrap());
    assert_eq!(after.services[0].allow, before.services[0].allow);
    assert_eq!(
        after.services[0].incarnation,
        before.services[0].incarnation
    );
    assert_eq!(
        serde_json::to_value(&after.clients).unwrap(),
        serde_json::to_value(&before.clients).unwrap()
    );
    assert_eq!(
        serde_json::to_value(store.paired_hosts(host).unwrap()).unwrap(),
        pairings
    );
    drop(store);
    let authority = host::Authority::open(dir.path(), host).unwrap();
    for peer in peers[..2].iter().copied() {
        let catalog = authority.catalog(peer).await.unwrap();
        assert_eq!(catalog.services.len(), 1);
        assert_eq!(catalog.services[0].id, "research");
        assert_eq!(
            catalog.services[0].incarnation,
            before.services[0].incarnation
        );
    }
    assert!(
        authority
            .catalog(peers[2])
            .await
            .unwrap()
            .services
            .is_empty()
    );
}

#[test]
fn service_rename_rejects_conflicts_and_rolls_back_invalid_edits() {
    use serde_json::json;
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    let dir = directory();
    host::initialize(dir.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(dir.path(), host).unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer,
                name: "computer".into(),
            },
        )
        .unwrap();
    for (revision, id) in [(1, "gike"), (2, "taken")] {
        store
            .host_mutate(
                host,
                &owner,
                Mutation::Service {
                    expected_revision: revision,
                    id: id.into(),
                    backend: "127.0.0.1:8310".parse().unwrap(),
                    allow: vec![peer],
                    display_name: None,
                    kind: None,
                },
            )
            .unwrap();
    }
    let before = serde_json::to_value(custom_snapshot(&mut store, host).unwrap()).unwrap();
    let edit = json!({"op":"edit_service", "expected_revision":3,
        "id":"gike", "new_id":"research", "backend":"127.0.0.1:8311"});
    for changes in [
        json!({"new_id":"taken"}),
        json!({"new_id":"UpperCase"}),
        json!({"id":"missing"}),
        json!({"expected_revision":2}),
        json!({"backend":"192.168.1.1:80"}),
        json!({"backend":"127.0.0.1:0"}),
        json!({"allow":[Identity::generate().endpoint_id()]}),
        json!({"allow":[peer,peer]}),
    ] {
        let mut operation = edit.clone();
        operation
            .as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        assert!(
            store
                .host_mutate(host, &owner, serde_json::from_value(operation).unwrap())
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(custom_snapshot(&mut store, host).unwrap()).unwrap(),
            before
        );
    }
    assert!(
        store
            .host_mutate(
                host,
                &control::token(),
                serde_json::from_value(edit.clone()).unwrap()
            )
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(custom_snapshot(&mut store, host).unwrap()).unwrap(),
        before
    );
    // Renaming and explicitly revoking a grant are one atomic owner action.
    let mut revoke = edit;
    revoke["allow"] = json!([]);
    store
        .host_mutate(host, &owner, serde_json::from_value(revoke).unwrap())
        .unwrap();
    let after = custom_snapshot(&mut store, host).unwrap();
    let renamed = after.services.iter().find(|s| s.id == "research").unwrap();
    assert!(renamed.allow.is_empty());
    assert_eq!(
        renamed.incarnation,
        before["services"][0]["incarnation"].as_str().unwrap()
    );
    assert_eq!(
        after
            .services
            .iter()
            .find(|s| s.id == "taken")
            .unwrap()
            .allow,
        vec![peer]
    );
    // Saving without a name change is still an edit, not a conflicting create.
    store
        .host_mutate(
            host,
            &owner,
            serde_json::from_value(json!({
                "op":"edit_service", "expected_revision":4, "id":"research",
                "new_id":"research", "backend":"127.0.0.1:8312"
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(custom_snapshot(&mut store, host).unwrap().revision, 5);
}

#[test]
fn removing_display_names_preserves_identity_pairings_grants_and_incarnations() {
    use wispnet_core::pairing::Bootstrap;
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    let dir = directory();
    host::initialize(dir.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(dir.path(), host).unwrap();
    let bootstrap = Bootstrap {
        version: 1,
        host: peer,
        relay_url: "https://relay.example/".into(),
        direct_hint: None,
    };
    store
        .pairing_accept_host(host, &owner, &bootstrap, "phone")
        .unwrap();
    let revision = custom_snapshot(&mut store, host).unwrap().revision;
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Service {
                expected_revision: revision,
                id: "website".into(),
                backend: "127.0.0.1:8080".parse().unwrap(),
                allow: vec![peer],
                display_name: None,
                kind: None,
            },
        )
        .unwrap();
    let before = custom_snapshot(&mut store, host).unwrap();
    let pairing = store.pairing_session(host, peer).unwrap();
    let credential = std::fs::read(dir.path().join("host-owner.token")).unwrap();
    drop(store);
    // Reconstruct a schema-10 installation with a separate presentation name.
    let db = rusqlite::Connection::open(dir.path().join("host.sqlite3")).unwrap();
    db.execute_batch("ALTER TABLE host_services ADD COLUMN display_name TEXT NOT NULL DEFAULT 'Old website label'; PRAGMA user_version=10;").unwrap();
    drop(db);
    let (mut store, _) = host::owner_store(dir.path(), host).unwrap();
    let after = custom_snapshot(&mut store, host).unwrap();
    assert_eq!(after.host, before.host);
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.revision, before.revision + 1);
    assert_eq!(after.clients[0].peer, peer);
    assert_eq!(after.services[0].id, "website");
    assert_eq!(after.services[0].display_name, "website");
    assert_eq!(
        after.services[0].incarnation,
        before.services[0].incarnation
    );
    assert_eq!(after.services[0].allow, vec![peer]);
    assert_eq!(store.paired_hosts(host).unwrap()[0].host, peer);
    assert_eq!(store.pairing_session(host, peer).unwrap(), pairing);
    assert_eq!(
        std::fs::read(dir.path().join("host-owner.token")).unwrap(),
        credential
    );
    drop(store);
    let (mut store, _) = host::owner_store(dir.path(), host).unwrap();
    assert_eq!(
        custom_snapshot(&mut store, host).unwrap().revision,
        after.revision
    );
}

#[tokio::test]
async fn complete_filtered_catalog_metadata_and_type_changes_are_host_local() {
    use wispnet_core::host::ServiceKind;
    let id = Identity::generate().endpoint_id();
    let client = Identity::generate().endpoint_id();
    let dir = directory();
    host::initialize(dir.path(), id).unwrap();
    let (mut store, secret) = host::owner_store(dir.path(), id).unwrap();
    store
        .host_mutate(
            id,
            &secret,
            Mutation::Approve {
                expected_revision: 0,
                peer: client,
                name: "Client".into(),
            },
        )
        .unwrap();
    let operation = |revision, name: Option<String>, kind| Mutation::Service {
        expected_revision: revision,
        id: "website".into(),
        backend: "127.0.0.1:8080".parse().unwrap(),
        allow: vec![client],
        display_name: name,
        kind,
    };
    store
        .host_mutate(
            id,
            &secret,
            operation(1, Some("My site".into()), Some(ServiceKind::Http)),
        )
        .unwrap();
    let first = custom_snapshot(&mut store, id).unwrap().services[0]
        .incarnation
        .clone();
    store
        .host_mutate(id, &secret, operation(2, None, None))
        .unwrap();
    assert_eq!(
        custom_snapshot(&mut store, id).unwrap().services[0].display_name,
        "website"
    );
    assert_eq!(
        custom_snapshot(&mut store, id).unwrap().services[0].incarnation,
        first
    );
    assert!(
        store
            .host_mutate(id, &secret, operation(3, Some("bad\nname".into()), None))
            .is_err()
    );
    assert_eq!(custom_snapshot(&mut store, id).unwrap().revision, 3);
    store
        .host_mutate(id, &secret, operation(3, None, Some(ServiceKind::Tcp)))
        .unwrap();
    assert_ne!(
        custom_snapshot(&mut store, id).unwrap().services[0].incarnation,
        first
    );
    drop(store);
    let authority = host::Authority::open(dir.path(), id).unwrap();
    let snapshot = authority.catalog(client).await.unwrap();
    assert!(snapshot.valid(id));
    assert_eq!(snapshot.services[0].kind, ServiceKind::Tcp);
    assert_eq!(snapshot.services[0].display_name, "website");
    let (mut store, secret) = host::owner_store(dir.path(), id).unwrap();
    store
        .host_mutate(
            id,
            &secret,
            Mutation::Remove {
                expected_revision: 4,
                id: "website".into(),
            },
        )
        .unwrap();
    let removed = authority.catalog(client).await.unwrap();
    assert!(removed.valid(id));
    assert_eq!(removed.service_count, 0);
    assert!(removed.services.is_empty());
    assert!(removed.follows(&snapshot));
}

#[test]
fn service_incarnation_persists_on_edit_and_restart_but_not_recreation() {
    let id = Identity::generate().endpoint_id();
    let dir = directory();
    host::initialize(dir.path(), id).unwrap();
    let (mut store, secret) = host::owner_store(dir.path(), id).unwrap();
    let edit = |revision| Mutation::Service {
        display_name: None,
        kind: None,
        expected_revision: revision,
        id: "website".into(),
        backend: "127.0.0.1:8080".parse().unwrap(),
        allow: vec![],
    };
    store.host_mutate(id, &secret, edit(0)).unwrap();
    let first = custom_snapshot(&mut store, id).unwrap().services[0]
        .incarnation
        .clone();
    store.host_mutate(id, &secret, edit(1)).unwrap();
    assert_eq!(
        custom_snapshot(&mut store, id).unwrap().services[0].incarnation,
        first
    );
    drop(store);
    let (mut store, secret) = host::owner_store(dir.path(), id).unwrap();
    assert_eq!(
        custom_snapshot(&mut store, id).unwrap().services[0].incarnation,
        first
    );
    store
        .host_mutate(
            id,
            &secret,
            Mutation::Remove {
                expected_revision: 2,
                id: "website".into(),
            },
        )
        .unwrap();
    store.host_mutate(id, &secret, edit(3)).unwrap();
    let new = custom_snapshot(&mut store, id).unwrap().services[0]
        .incarnation
        .clone();
    assert_ne!(first, new);
    // Simulate the real schema-6 table, preserving the authority and grants.
    drop(store);
    let db = rusqlite::Connection::open(dir.path().join("host.sqlite3")).unwrap();
    db.execute_batch("DELETE FROM host_services WHERE id IN ('shell','files'); DROP TABLE host_pairing_blocks; DROP TABLE host_legacy_pins; DROP TABLE host_pairing_offers; DROP TABLE host_pairings; ALTER TABLE host_services DROP COLUMN incarnation; ALTER TABLE host_services DROP COLUMN kind; PRAGMA user_version=6;")
        .unwrap();
    drop(db);
    let (mut store, _) = host::owner_store(dir.path(), id).unwrap();
    let migrated = custom_snapshot(&mut store, id).unwrap();
    assert_eq!(migrated.revision, 8);
    assert_eq!(migrated.host, id);
    assert!(wispnet_core::enrollment::valid_code(
        &migrated.services[0].incarnation
    ));
    let generation = migrated.services[0].incarnation.clone();
    drop(store);
    let (mut store, _) = host::owner_store(dir.path(), id).unwrap();
    assert_eq!(
        custom_snapshot(&mut store, id).unwrap().services[0].incarnation,
        generation
    );
}
#[test]
fn host_identity_and_admin_cannot_cross_authority_boundaries() {
    let a = Identity::generate().endpoint_id();
    let b = Identity::generate().endpoint_id();
    let client = Identity::generate().endpoint_id();
    let dir = directory();
    host::initialize(dir.path(), a).unwrap();
    let (mut store, secret) = host::owner_store(dir.path(), a).unwrap();
    assert!(store.host_check(b, None).is_err());
    assert!(
        store
            .host_mutate(
                a,
                &control::token(),
                Mutation::Approve {
                    expected_revision: 0,
                    peer: client,
                    name: "forged admin".into()
                }
            )
            .is_err()
    );
    store
        .host_mutate(
            a,
            &secret,
            Mutation::Approve {
                expected_revision: 0,
                peer: client,
                name: "local client".into(),
            },
        )
        .unwrap();
    assert_eq!(
        custom_snapshot(&mut store, a).unwrap().clients[0].peer,
        client
    );
    drop(store);
    host::initialize(dir.path(), a).unwrap();
    assert!(host::initialize(dir.path(), b).is_err());
    std::fs::remove_file(dir.path().join("host.sqlite3")).unwrap();
    assert!(host::initialize(dir.path(), a).is_err());
    assert!(
        !dir.path().join("host.sqlite3").exists(),
        "lost authority silently recreated"
    );
}
#[test]
fn wrong_backend_unapproved_grants_and_legacy_in_place_conversion_fail_closed() {
    let host = Identity::generate().endpoint_id();
    let client = Identity::generate().endpoint_id();
    let dir = directory();
    host::initialize(dir.path(), host).unwrap();
    let (mut store, secret) = host::owner_store(dir.path(), host).unwrap();
    for backend in ["192.168.1.1:80", "127.0.0.1:0"] {
        assert!(
            store
                .host_mutate(
                    host,
                    &secret,
                    Mutation::Service {
                        display_name: None,
                        kind: None,
                        expected_revision: 0,
                        id: "web".into(),
                        backend: backend.parse().unwrap(),
                        allow: vec![]
                    }
                )
                .is_err()
        );
    }
    assert!(
        store
            .host_mutate(
                host,
                &secret,
                Mutation::Service {
                    display_name: None,
                    kind: None,
                    expected_revision: 0,
                    id: "web".into(),
                    backend: "127.0.0.1:8080".parse().unwrap(),
                    allow: vec![client]
                }
            )
            .is_err()
    );
    assert_eq!(custom_snapshot(&mut store, host).unwrap().revision, 0);
    let legacy_dir = directory();
    let mut legacy = Store::open(&legacy_dir.path().join("registry.sqlite3")).unwrap();
    legacy.replace(0, &[client], &[]).unwrap();
    assert!(legacy.host_initialize(host, &secret).is_err());
    assert_eq!(legacy.snapshot().unwrap().devices, vec![client]);
}

#[test]
fn schema_eight_adds_mutual_pairing_without_changing_identity_revision_or_grants() {
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
                name: "Existing device".into(),
            },
        )
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Service {
                expected_revision: 1,
                id: "website".into(),
                backend: "127.0.0.1:8201".parse().unwrap(),
                allow: vec![peer],
                display_name: Some("Website".into()),
                kind: None,
            },
        )
        .unwrap();
    let before = serde_json::to_value(custom_snapshot(&mut store, host).unwrap()).unwrap();
    drop(store);
    let db = rusqlite::Connection::open(dir.path().join("host.sqlite3")).unwrap();
    db.execute_batch(
        "DROP TABLE host_pairing_blocks; DROP TABLE host_legacy_pins; DROP TABLE host_pairing_offers; DROP TABLE host_pairings; ALTER TABLE host_services ADD COLUMN display_name TEXT NOT NULL DEFAULT ''; UPDATE host_services SET display_name=id; PRAGMA user_version=8;",
    )
    .unwrap();
    drop(db);
    let (mut store, _) = host::owner_store(dir.path(), host).unwrap();
    assert_eq!(
        serde_json::to_value(custom_snapshot(&mut store, host).unwrap()).unwrap(),
        before
    );
    assert!(store.paired_hosts(host).unwrap().is_empty());
}
