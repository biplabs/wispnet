use std::os::unix::fs::PermissionsExt;
use wispnet_core::{
    identity::Identity,
    pairing::{Claim, Proof},
};
use wispnet_registry::{
    control::{self, ControlError},
    host::{self, Mutation},
    store::Store,
};

#[test]
fn target_local_proofs_are_single_use_and_owner_approval_is_separate() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let host_key = Identity::generate();
    let device = Identity::generate();
    let other = Identity::generate();
    let host = host_key.endpoint_id();
    host::initialize(directory.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(directory.path(), host).unwrap();
    let now = wispnet_registry::enrollment::now();
    assert!(
        store
            .pairing_invite(host, &control::token(), "https://relay.example", now, 300)
            .is_err()
    );
    let invitation = store
        .pairing_invite(host, &owner, "https://relay.example", now, 300)
        .unwrap();
    let claim = Claim {
        invitation: invitation.invitation.clone(),
        code: invitation.code.clone(),
        name: "tablet".into(),
    };
    let wrong = Claim {
        invitation: invitation.invitation.clone(),
        code: "0".repeat(64),
        name: "tablet".into(),
    };
    for _ in 0..20 {
        assert!(
            store
                .pairing_challenge(host, device.endpoint_id(), &wrong, now)
                .is_err()
        );
    }
    let challenge = store
        .pairing_challenge(host, device.endpoint_id(), &claim, now)
        .unwrap();
    assert!(challenge.matches(
        &invitation,
        device.endpoint_id(),
        "tablet",
        wispnet_core::pairing::Purpose::Request,
        now
    ));
    assert!(
        store
            .pairing_challenge(host, other.endpoint_id(), &claim, now)
            .is_err()
    );
    let signature = device.secret_key().sign(&challenge.signing_bytes());
    let proof = Proof {
        challenge,
        signature,
    };
    assert!(
        store
            .pairing_prove(host, other.endpoint_id(), &proof, now)
            .is_err()
    );
    assert!(
        store
            .pairing_prove(other.endpoint_id(), device.endpoint_id(), &proof, now)
            .is_err()
    );
    let fresh = store
        .pairing_challenge(host, device.endpoint_id(), &claim, now)
        .unwrap();
    assert!(
        store
            .pairing_prove(host, device.endpoint_id(), &proof, now)
            .is_err()
    );
    let proof = Proof {
        signature: device.secret_key().sign(&fresh.signing_bytes()),
        challenge: fresh,
    };
    let pending = store
        .pairing_prove(host, device.endpoint_id(), &proof, now)
        .unwrap();
    assert_eq!(pending.peer, device.endpoint_id());
    assert!(store.host_snapshot(host).unwrap().clients.is_empty());
    assert!(
        store
            .pairing_prove(host, device.endpoint_id(), &proof, now)
            .is_err()
    );
    let list = store.pairing_list(host, &owner, now).unwrap();
    assert_eq!(list[0].state, "pending");
    assert!(
        !serde_json::to_string(&list)
            .unwrap()
            .contains(&invitation.code)
    );
    assert!(
        store
            .host_mutate(
                host,
                &owner,
                Mutation::ApproveInvitation {
                    expected_revision: 1,
                    invitation: invitation.invitation.clone()
                }
            )
            .is_err()
    );
    assert_eq!(
        store
            .host_mutate(
                host,
                &owner,
                Mutation::ApproveInvitation {
                    expected_revision: 0,
                    invitation: invitation.invitation.clone()
                }
            )
            .unwrap(),
        1
    );
    let snapshot = store.host_snapshot(host).unwrap();
    assert_eq!(snapshot.clients[0].peer, device.endpoint_id());
    assert!(snapshot.services.is_empty());
    assert!(
        store
            .pairing_challenge(host, device.endpoint_id(), &claim, now)
            .is_err()
    );
    drop(store);
    let (mut store, owner) = host::owner_store(directory.path(), host).unwrap();
    assert_eq!(
        store.pairing_list(host, &owner, now).unwrap()[0].state,
        "spent"
    );
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Revoke {
                expected_revision: 1,
                peer: device.endpoint_id(),
            },
        )
        .unwrap();
    assert!(
        store
            .pairing_challenge(host, device.endpoint_id(), &claim, now)
            .is_err()
    );
    assert!(
        store
            .pairing_prove(host, device.endpoint_id(), &proof, now)
            .is_err()
    );
    assert!(store.host_snapshot(host).unwrap().clients.is_empty());
    let db = rusqlite::Connection::open(directory.path().join("host.sqlite3")).unwrap();
    let digest: String = db
        .query_row(
            "SELECT hash FROM host_invitations WHERE id=?1",
            [&invitation.invitation],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(digest, control::hash(&invitation.code));
}

#[test]
fn attempts_expiry_rejection_and_capacity_are_durable_and_bounded() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = Identity::generate().endpoint_id();
    let device = Identity::generate();
    host::initialize(directory.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(directory.path(), host).unwrap();
    let now = wispnet_registry::enrollment::now();
    let invitation = store
        .pairing_invite(host, &owner, "https://relay.example", now, 300)
        .unwrap();
    let claim = Claim {
        invitation: invitation.invitation.clone(),
        code: invitation.code.clone(),
        name: "client".into(),
    };
    let challenge = store
        .pairing_challenge(host, device.endpoint_id(), &claim, now)
        .unwrap();
    let bad = Proof {
        signature: Identity::generate()
            .secret_key()
            .sign(&challenge.signing_bytes()),
        challenge,
    };
    for _ in 0..11 {
        assert!(
            store
                .pairing_prove(host, device.endpoint_id(), &bad, now)
                .is_err()
        );
    }
    drop(store);
    let (mut store, owner) = host::owner_store(directory.path(), host).unwrap();
    assert!(matches!(
        store.pairing_challenge(host, device.endpoint_id(), &claim, now),
        Err(ControlError::RateLimited)
    ));
    let challenge = store
        .pairing_challenge(host, device.endpoint_id(), &claim, now + 61)
        .unwrap();
    let proof = Proof {
        signature: device.secret_key().sign(&challenge.signing_bytes()),
        challenge,
    };
    store
        .pairing_prove(host, device.endpoint_id(), &proof, now + 61)
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::RejectInvitation {
                expected_revision: 0,
                invitation: invitation.invitation.clone(),
            },
        )
        .unwrap();
    assert!(
        store
            .pairing_challenge(host, device.endpoint_id(), &claim, now + 61)
            .is_err()
    );
    assert!(
        store
            .host_mutate(
                host,
                &owner,
                Mutation::ApproveInvitation {
                    expected_revision: 1,
                    invitation: invitation.invitation.clone()
                }
            )
            .is_err()
    );
    assert!(
        store
            .pairing_challenge(host, device.endpoint_id(), &claim, now + 300)
            .is_err()
    );
    for _ in 0..127 {
        store
            .pairing_invite(host, &owner, "https://relay.example", now, 300)
            .unwrap();
    }
    assert!(matches!(
        store.pairing_invite(host, &owner, "https://relay.example", now, 300),
        Err(ControlError::RateLimited)
    ));
    // Expired rows can be discarded, but their old capabilities still fail closed.
    store
        .pairing_invite(host, &owner, "https://relay.example", now + 301, 300)
        .unwrap();
    assert!(
        store
            .pairing_challenge(host, device.endpoint_id(), &claim, now + 301)
            .is_err()
    );
    let _ = Store::open(&directory.path().join("host.sqlite3")).unwrap();
}

#[test]
fn revocation_cancels_unfinished_requests_and_expired_proofs_cannot_be_approved() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = Identity::generate().endpoint_id();
    let device = Identity::generate();
    host::initialize(directory.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(directory.path(), host).unwrap();
    let now = wispnet_registry::enrollment::now();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer: device.endpoint_id(),
                name: "client".into(),
            },
        )
        .unwrap();
    let invitation = store
        .pairing_invite(host, &owner, "https://relay.example", now, 30)
        .unwrap();
    let claim = Claim {
        invitation: invitation.invitation.clone(),
        code: invitation.code.clone(),
        name: "client".into(),
    };
    let challenge = store
        .pairing_challenge(host, device.endpoint_id(), &claim, now)
        .unwrap();
    let proof = Proof {
        signature: device.secret_key().sign(&challenge.signing_bytes()),
        challenge,
    };
    assert!(
        store
            .pairing_prove(host, device.endpoint_id(), &proof, now + 30)
            .is_err()
    );
    store
        .pairing_prove(host, device.endpoint_id(), &proof, now)
        .unwrap();
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Revoke {
                expected_revision: 1,
                peer: device.endpoint_id(),
            },
        )
        .unwrap();
    assert_eq!(
        store.pairing_list(host, &owner, now).unwrap()[0].state,
        "revoked"
    );
    assert!(
        store
            .pairing_challenge(host, device.endpoint_id(), &claim, now)
            .is_err()
    );
    assert!(
        store
            .host_mutate(
                host,
                &owner,
                Mutation::ApproveInvitation {
                    expected_revision: 2,
                    invitation: invitation.invitation.clone()
                }
            )
            .is_err()
    );
    assert!(store.host_snapshot(host).unwrap().clients.is_empty());
    assert_eq!(store.host_snapshot(host).unwrap().revision, 2);
}
