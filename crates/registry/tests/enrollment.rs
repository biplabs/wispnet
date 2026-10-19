use std::os::unix::fs::PermissionsExt;
use wispnet_core::{
    enrollment::{Challenge, Claim, Invitation, Proof},
    identity::Identity,
};
use wispnet_registry::{
    control::{self, ControlError, Mutation, Principal},
    enrollment::Admin,
    store::Store,
};
fn setup() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    control::initialize(&mut store, &dir.path().join("owner.token"), "test").unwrap();
    (dir, store)
}
fn invite(store: &mut Store, now: u64) -> Invitation {
    store
        .enrollment_admin(
            Admin::Invite {
                controller: "https://controller.example".into(),
                ttl_seconds: 600,
            },
            now,
        )
        .unwrap()
        .invitation
        .unwrap()
}
fn claim(invite: &Invitation, key: &Identity) -> Claim {
    Claim {
        invitation: invite.invitation.clone(),
        device: key.endpoint_id(),
        name: "Linux laptop".into(),
    }
}
fn proof(challenge: &Challenge, key: &Identity) -> Proof {
    Proof {
        invitation: challenge.invitation.clone(),
        device: key.endpoint_id(),
        name: challenge.name.clone(),
        nonce: challenge.nonce.clone(),
        signature: key.secret_key().sign(&challenge.signing_bytes()),
    }
}
fn pending(store: &mut Store, invite: &Invitation, key: &Identity, now: u64) -> Challenge {
    let challenge = store
        .enrollment_challenge(&invite.code, claim(invite, key), now)
        .unwrap()
        .challenge
        .take()
        .unwrap();
    assert!(challenge.matches(invite, key.endpoint_id(), "Linux laptop", now));
    assert_eq!(
        store
            .enrollment_proof(&invite.code, proof(&challenge, key), now)
            .unwrap()
            .state,
        "pending"
    );
    challenge
}
fn approve(store: &mut Store, invite: &Invitation, key: &Identity, now: u64) {
    store
        .enrollment_admin(
            Admin::Approve {
                invitation: invite.invitation.clone(),
                device: key.endpoint_id(),
                name: "Linux laptop".into(),
                expected_revision: 0,
            },
            now,
        )
        .unwrap();
}
#[test]
fn approval_is_bound_to_proven_key_name_and_fresh_action_challenge() {
    let (_dir, mut store) = setup();
    let key = Identity::generate();
    let attacker = Identity::generate();
    let invite = invite(&mut store, 1000);
    let first = store
        .enrollment_challenge(&invite.code, claim(&invite, &key), 1000)
        .unwrap()
        .challenge
        .take()
        .unwrap();
    assert!(
        store
            .enrollment_proof(&invite.code, proof(&first, &attacker), 1000)
            .is_err()
    );
    let forged = Proof {
        invitation: first.invitation.clone(),
        device: key.endpoint_id(),
        name: first.name.clone(),
        nonce: first.nonce.clone(),
        signature: attacker.secret_key().sign(&first.signing_bytes()),
    };
    assert!(store.enrollment_proof(&invite.code, forged, 1000).is_err());
    assert!(
        store
            .enrollment_admin(
                Admin::Approve {
                    invitation: invite.invitation.clone(),
                    device: key.endpoint_id(),
                    name: "Linux laptop".into(),
                    expected_revision: 0
                },
                1000
            )
            .is_err()
    );
    assert_eq!(
        store
            .enrollment_proof(&invite.code, proof(&first, &key), 1000)
            .unwrap()
            .state,
        "pending"
    );
    assert!(
        store
            .enrollment_proof(&invite.code, proof(&first, &key), 1000)
            .is_err()
    );
    assert!(
        store
            .enrollment_admin(
                Admin::Approve {
                    invitation: invite.invitation.clone(),
                    device: attacker.endpoint_id(),
                    name: "Linux laptop".into(),
                    expected_revision: 0
                },
                1000
            )
            .is_err()
    );
    assert!(
        store
            .enrollment_admin(
                Admin::Approve {
                    invitation: invite.invitation.clone(),
                    device: key.endpoint_id(),
                    name: "Wrong name".into(),
                    expected_revision: 0
                },
                1000
            )
            .is_err()
    );
    approve(&mut store, &invite, &key, 1000);
    // Advance the rate window. A capability holder cannot substitute another key.
    assert!(
        store
            .enrollment_challenge(&invite.code, claim(&invite, &attacker), 1061)
            .is_err()
    );
    assert!(
        store
            .enrollment_proof(&invite.code, proof(&first, &key), 1061)
            .is_err()
    );
    let second = store
        .enrollment_challenge(&invite.code, claim(&invite, &key), 1061)
        .unwrap()
        .challenge
        .take()
        .unwrap();
    // Old request signature is not a complete-action signature, even with new nonce.
    let replay = Proof {
        invitation: second.invitation.clone(),
        device: key.endpoint_id(),
        name: second.name.clone(),
        nonce: second.nonce.clone(),
        signature: key.secret_key().sign(&first.signing_bytes()),
    };
    assert!(store.enrollment_proof(&invite.code, replay, 1061).is_err());
    let completed = store
        .enrollment_proof(&invite.code, proof(&second, &key), 1061)
        .unwrap();
    assert_eq!(completed.revision, Some(1));
    assert!(
        matches!(store.authenticate(completed.token.as_ref().unwrap()).unwrap(),Some(Principal::Device(id)) if id==key.endpoint_id())
    );
    assert!(
        store
            .enrollment_proof(&invite.code, proof(&second, &key), 1061)
            .is_err()
    );
    assert!(
        store
            .enrollment_challenge(&invite.code, claim(&invite, &key), 1061)
            .is_err()
    );
    let listing = store.enrollments(1061).unwrap();
    assert_eq!(listing.invitations[0].state, "enrolled");
    assert_eq!(listing.devices[0].name, "Linux laptop");
    store
        .mutate(
            Principal::Owner,
            Mutation::Revoke {
                expected_revision: 1,
                device: key.endpoint_id(),
            },
        )
        .unwrap();
    let listing = store.enrollments(1061).unwrap();
    assert_eq!(listing.revision, 2);
    assert_eq!(listing.devices[0].state, "revoked");
    assert_eq!(listing.invitations[0].state, "revoked");
    assert!(
        store
            .authenticate(completed.token.as_ref().unwrap())
            .unwrap()
            .is_none()
    );
}
#[test]
fn expiry_rejection_challenge_refresh_and_controller_binding() {
    let (_dir, mut store) = setup();
    let key = Identity::generate();
    let expired = invite(&mut store, 1000);
    let challenge = store
        .enrollment_challenge(&expired.code, claim(&expired, &key), 1000)
        .unwrap()
        .challenge
        .take()
        .unwrap();
    assert!(
        store
            .enrollment_proof(&expired.code, proof(&challenge, &key), 1120)
            .is_err()
    );
    let fresh = store
        .enrollment_challenge(&expired.code, claim(&expired, &key), 1120)
        .unwrap()
        .challenge
        .take()
        .unwrap();
    assert!(
        store
            .enrollment_proof(&expired.code, proof(&challenge, &key), 1120)
            .is_err()
    );
    let mut tampered = fresh.clone();
    tampered.controller = "https://evil.example/".into();
    assert!(
        store
            .enrollment_proof(&expired.code, proof(&tampered, &key), 1120)
            .is_err()
    );
    assert!(
        store
            .enrollment_challenge(&expired.code, claim(&expired, &key), 1600)
            .is_err()
    );
    assert_eq!(
        store.enrollments(1600).unwrap().invitations[0].state,
        "expired"
    );
    let rejected = invite(&mut store, 1600);
    pending(&mut store, &rejected, &key, 1600);
    store
        .enrollment_admin(
            Admin::Reject {
                invitation: rejected.invitation.clone(),
            },
            1600,
        )
        .unwrap();
    assert!(
        store
            .enrollment_challenge(&rejected.code, claim(&rejected, &key), 1600)
            .is_err()
    );
    assert_eq!(
        store.enrollments(1600).unwrap().invitations[0].state,
        "rejected"
    );
}
#[test]
fn pending_approval_and_rate_budgets_survive_restart() {
    let (dir, mut store) = setup();
    let key = Identity::generate();
    let invite = invite(&mut store, 1000);
    pending(&mut store, &invite, &key, 1000);
    for _ in 0..10 {
        assert_eq!(
            store
                .enrollment_challenge(&invite.code, claim(&invite, &key), 1000)
                .unwrap()
                .state,
            "pending"
        );
    }
    drop(store);
    let mut store = Store::open(&dir.path().join("db")).unwrap();
    assert!(matches!(
        store.enrollment_challenge(&invite.code, claim(&invite, &key), 1000),
        Err(ControlError::RateLimited)
    ));
    assert_eq!(
        store.enrollments(1000).unwrap().invitations[0].state,
        "pending"
    );
    approve(&mut store, &invite, &key, 1000);
    let challenge = store
        .enrollment_challenge(&invite.code, claim(&invite, &key), 1060)
        .unwrap()
        .challenge
        .take()
        .unwrap();
    let enrolled = store
        .enrollment_proof(&invite.code, proof(&challenge, &key), 1060)
        .unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("db")).unwrap();
    assert!(
        store
            .authenticate(enrolled.token.as_ref().unwrap())
            .unwrap()
            .is_some()
    );
}
#[test]
fn concurrent_completion_mints_exactly_one_credential_and_revision() {
    let (dir, mut store) = setup();
    let key = Identity::generate();
    let invite = invite(&mut store, 1000);
    pending(&mut store, &invite, &key, 1000);
    approve(&mut store, &invite, &key, 1000);
    let challenge = store
        .enrollment_challenge(&invite.code, claim(&invite, &key), 1000)
        .unwrap()
        .challenge
        .take()
        .unwrap();
    let first_proof = proof(&challenge, &key);
    let second_proof = proof(&challenge, &key);
    let path = dir.path().join("db");
    let code = invite.code.clone();
    let other_code = code.clone();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other_barrier = barrier.clone();
    let other = std::thread::spawn(move || {
        let mut db = Store::open(&path).unwrap();
        other_barrier.wait();
        db.enrollment_proof(&other_code, second_proof, 1000).is_ok()
    });
    barrier.wait();
    let first = store.enrollment_proof(&code, first_proof, 1000).is_ok();
    assert_ne!(first, other.join().unwrap());
    assert_eq!(store.enrollments(1000).unwrap().revision, 1);
}

#[test]
fn schema_two_migration_preserves_existing_credentials_and_marks_legacy_devices() {
    let (dir, mut store) = setup();
    let key = Identity::generate();
    let invitation = invite(&mut store, 1000);
    pending(&mut store, &invitation, &key, 1000);
    approve(&mut store, &invitation, &key, 1000);
    let challenge = store
        .enrollment_challenge(&invitation.code, claim(&invitation, &key), 1000)
        .unwrap()
        .challenge
        .take()
        .unwrap();
    let result = store
        .enrollment_proof(&invitation.code, proof(&challenge, &key), 1000)
        .unwrap();
    drop(store);
    let path = dir.path().join("db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE host_pairing_blocks; DROP TABLE host_legacy_pins; DROP TABLE host_pairing_offers; DROP TABLE host_pairings; DROP TABLE host_operation_receipts; DROP TABLE host_grants; DROP TABLE host_services; DROP TABLE host_clients; DROP TABLE host_authority; DROP TABLE invitations; DROP TABLE device_records; PRAGMA user_version=2;")
        .unwrap();
    drop(db);
    let mut migrated = Store::open(&path).unwrap();
    assert!(
        migrated
            .authenticate(result.token.as_ref().unwrap())
            .unwrap()
            .is_some()
    );
    let records = migrated.enrollments(1000).unwrap();
    assert_eq!(records.revision, 1);
    assert_eq!(records.devices[0].name, "legacy");
    assert_eq!(records.devices[0].state, "enrolled");
    assert!(records.invitations.is_empty());
}
