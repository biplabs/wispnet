use std::os::unix::fs::PermissionsExt;
use wispnet_core::{
    identity::Identity,
    pairing::{Bootstrap, PairingState},
};
use wispnet_registry::host::{self, Mutation};

#[test]
fn authenticated_status_and_session_guard_protect_new_pairings() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    let stranger = Identity::generate().endpoint_id();
    host::initialize(directory.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(directory.path(), host).unwrap();
    let bootstrap = Bootstrap {
        version: 1,
        host: peer,
        relay_url: "https://relay.example/".into(),
        direct_hint: None,
    };
    let old = "a".repeat(64);
    let new = "b".repeat(64);
    assert_eq!(
        store.pairing_status(host, peer, &old).unwrap().state,
        PairingState::Unknown
    );
    store
        .pairing_accept_session(host, &owner, &bootstrap, "Peer", Some(&old))
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Service {
                expected_revision: 1,
                id: "private".into(),
                backend: "127.0.0.1:8201".parse().unwrap(),
                allow: vec![peer],
                display_name: None,
                kind: None,
            },
        )
        .unwrap();
    assert_eq!(
        store.pairing_status(host, peer, &old).unwrap().state,
        PairingState::Paired
    );
    assert_eq!(
        store.pairing_status(host, stranger, &old).unwrap().state,
        PairingState::Unknown
    );
    assert!(store.pairing_status(host, peer, "bad").is_err());
    assert!(
        store
            .delete_pairing(host, "wrong", peer, Some(&bootstrap), None)
            .is_err()
    );
    assert!(
        store
            .delete_pairing(host, &owner, peer, Some(&bootstrap), Some(1))
            .is_err()
    );
    store
        .delete_pairing(host, &owner, peer, Some(&bootstrap), Some(2))
        .unwrap();
    assert!(store.host_snapshot(host).unwrap().clients.is_empty());
    assert!(
        store.host_snapshot(host).unwrap().services[0]
            .allow
            .is_empty()
    );
    assert_eq!(
        store.pairing_status(host, peer, &old).unwrap().state,
        PairingState::Removed
    );
    assert_eq!(
        store.pairing_status(host, peer, &new).unwrap().state,
        PairingState::Unknown
    );
    let revision = store.host_snapshot(host).unwrap().revision;
    // Repeated status checks are read-only, not notification retries.
    store.pairing_status(host, peer, &old).unwrap();
    assert!(
        !store
            .apply_remote_removal(host, &owner, peer, &old)
            .unwrap()
    );
    assert_eq!(store.host_snapshot(host).unwrap().revision, revision);
    drop(store);
    let (mut store, _) = host::owner_store(directory.path(), host).unwrap();
    assert_eq!(
        store.pairing_status(host, peer, &old).unwrap().state,
        PairingState::Removed
    );
    store
        .pairing_accept_session(host, &owner, &bootstrap, "Peer again", Some(&new))
        .unwrap();
    let revision = store.host_snapshot(host).unwrap().revision;
    // A delayed removal response for the previous session cannot erase this one.
    assert!(
        !store
            .apply_remote_removal(host, &owner, peer, &old)
            .unwrap()
    );
    assert_eq!(store.host_snapshot(host).unwrap().revision, revision);
    assert_eq!(
        store.pairing_status(host, peer, &new).unwrap().state,
        PairingState::Paired
    );
    assert!(
        store
            .apply_remote_removal(host, &owner, peer, &new)
            .unwrap()
    );
    assert!(store.paired_hosts(host).unwrap().is_empty());
    let db = rusqlite::Connection::open(directory.path().join("host.sqlite3")).unwrap();
    let queues: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='host_deletion_outbox'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(queues, 0);
}

#[test]
fn schema_nine_upgrade_preserves_legacy_pairs_and_policy_without_a_queue() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = Identity::generate().endpoint_id();
    let peer = Identity::generate().endpoint_id();
    host::initialize(directory.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(directory.path(), host).unwrap();
    let bootstrap = Bootstrap {
        version: 1,
        host: peer,
        relay_url: "https://relay.example/".into(),
        direct_hint: None,
    };
    store
        .pairing_accept_host(host, &owner, &bootstrap, "Existing peer")
        .unwrap();
    let before = serde_json::to_value(store.host_snapshot(host).unwrap()).unwrap();
    drop(store);
    let db = rusqlite::Connection::open(directory.path().join("host.sqlite3")).unwrap();
    db.execute_batch("DROP TABLE host_pairing_blocks; DROP TABLE host_legacy_pins; ALTER TABLE host_pairings DROP COLUMN pairing; ALTER TABLE host_pairing_offers DROP COLUMN pairing; ALTER TABLE host_services ADD COLUMN display_name TEXT NOT NULL DEFAULT ''; UPDATE host_services SET display_name=id; PRAGMA user_version=9;").unwrap();
    drop(db);
    let (mut store, _) = host::owner_store(directory.path(), host).unwrap();
    assert_eq!(
        serde_json::to_value(store.host_snapshot(host).unwrap()).unwrap(),
        before
    );
    assert_eq!(store.paired_hosts(host).unwrap()[0].host, peer);
    let session = store.pairing_session(host, peer).unwrap().unwrap();
    store
        .delete_pairing(host, &owner, peer, Some(&bootstrap), None)
        .unwrap();
    assert_eq!(
        store.pairing_status(host, peer, &session).unwrap().state,
        PairingState::Removed
    );
    store.index_legacy_pin(host, &owner, peer).unwrap();
    assert!(store.pairing_session(host, peer).unwrap().is_none());
}

#[test]
fn expired_fresh_request_is_not_confused_with_an_older_pairing() {
    use wispnet_core::pairing::{Claim, Proof};
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = Identity::generate().endpoint_id();
    let device = Identity::generate();
    let peer = device.endpoint_id();
    host::initialize(directory.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(directory.path(), host).unwrap();
    let bootstrap = Bootstrap {
        version: 1,
        host: peer,
        relay_url: "https://relay.example/".into(),
        direct_hint: None,
    };
    store
        .pairing_accept_session(host, &owner, &bootstrap, "Old peer", Some(&"a".repeat(64)))
        .unwrap();
    let past = wispnet_registry::enrollment::now() - 120;
    let invitation = store
        .pairing_invite(host, &owner, "https://relay.example/", past, 30)
        .unwrap();
    let challenge = store
        .pairing_challenge(
            host,
            peer,
            &Claim {
                invitation: invitation.invitation.clone(),
                code: invitation.code.clone(),
                name: "Fresh request".into(),
            },
            past,
        )
        .unwrap();
    let proof = Proof {
        signature: device.secret_key().sign(&challenge.signing_bytes()),
        challenge,
    };
    store
        .pairing_prove_session(host, peer, &proof, Some(&bootstrap), past, true)
        .unwrap();
    assert_eq!(
        store
            .pairing_status(host, peer, &invitation.invitation)
            .unwrap()
            .state,
        PairingState::Unknown
    );
    assert_eq!(store.host_snapshot(host).unwrap().clients.len(), 1);
}
