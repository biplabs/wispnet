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

// Pairing fixtures below assert user-service grants; the builtin starts with
// no grants and is checked independently in tests/shell.rs.
fn custom_snapshot(
    store: &mut wispnet_registry::store::Store,
    host: wispnet_core::identity::EndpointId,
) -> Result<wispnet_registry::host::Snapshot, wispnet_registry::control::ControlError> {
    let mut snapshot = store.host_snapshot(host)?;
    snapshot
        .services
        .retain(|s| s.kind != wispnet_core::host::ServiceKind::Shell);
    Ok(snapshot)
}
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
    assert!(
        custom_snapshot(&mut store, host)
            .unwrap()
            .clients
            .is_empty()
    );
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
                    services: vec![],
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
                    services: vec![],
                    expected_revision: 0,
                    invitation: invitation.invitation.clone()
                }
            )
            .unwrap(),
        1
    );
    let snapshot = custom_snapshot(&mut store, host).unwrap();
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
    assert!(
        custom_snapshot(&mut store, host)
            .unwrap()
            .clients
            .is_empty()
    );
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
                    services: vec![],
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
                    services: vec![],
                    expected_revision: 2,
                    invitation: invitation.invitation.clone()
                }
            )
            .is_err()
    );
    assert!(
        custom_snapshot(&mut store, host)
            .unwrap()
            .clients
            .is_empty()
    );
    assert_eq!(custom_snapshot(&mut store, host).unwrap().revision, 2);
}

#[test]
fn approval_and_selected_grants_commit_together_and_preserve_other_access() {
    use wispnet_core::identity::EndpointId;
    fn pending(store: &mut Store, host: EndpointId, owner: &str, device: &Identity) -> String {
        let now = wispnet_registry::enrollment::now();
        let invitation = store
            .pairing_invite(host, owner, "https://relay.example", now, 300)
            .unwrap();
        let claim = Claim {
            invitation: invitation.invitation.clone(),
            code: invitation.code.clone(),
            name: "new phone".into(),
        };
        let challenge = store
            .pairing_challenge(host, device.endpoint_id(), &claim, now)
            .unwrap();
        let proof = Proof {
            signature: device.secret_key().sign(&challenge.signing_bytes()),
            challenge,
        };
        store
            .pairing_prove(host, device.endpoint_id(), &proof, now)
            .unwrap();
        invitation.invitation.clone()
    }
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = Identity::generate().endpoint_id();
    let device = Identity::generate();
    let other = Identity::generate();
    host::initialize(directory.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(directory.path(), host).unwrap();
    let mut revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer: other.endpoint_id(),
                name: "existing phone".into(),
            },
        )
        .unwrap();
    for id in ["alpha", "beta", "gamma"] {
        revision = store
            .host_mutate(
                host,
                &owner,
                Mutation::Service {
                    expected_revision: revision,
                    id: id.into(),
                    backend: "127.0.0.1:8201".parse().unwrap(),
                    allow: vec![other.endpoint_id()],
                    display_name: None,
                    kind: None,
                },
            )
            .unwrap();
    }
    let invitation = pending(&mut store, host, &owner, &device);
    let before = serde_json::to_value(custom_snapshot(&mut store, host).unwrap()).unwrap();
    for services in [
        vec!["alpha".into(), "missing".into()],
        vec!["alpha".into(), "alpha".into()],
    ] {
        assert!(
            store
                .host_mutate(
                    host,
                    &owner,
                    Mutation::ApproveInvitation {
                        expected_revision: revision,
                        invitation: invitation.clone(),
                        services
                    }
                )
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(custom_snapshot(&mut store, host).unwrap()).unwrap(),
            before
        );
        assert_eq!(
            store
                .pairing_list(host, &owner, wispnet_registry::enrollment::now())
                .unwrap()
                .iter()
                .find(|v| v.invitation == invitation)
                .unwrap()
                .state,
            "pending"
        );
    }
    assert!(matches!(
        store.host_mutate(
            host,
            &owner,
            Mutation::ApproveInvitation {
                expected_revision: revision - 1,
                invitation: invitation.clone(),
                services: vec!["alpha".into()]
            }
        ),
        Err(ControlError::Conflict)
    ));
    let approved = store
        .host_mutate(
            host,
            &owner,
            Mutation::ApproveInvitation {
                expected_revision: revision,
                invitation: invitation.clone(),
                services: vec!["alpha".into(), "gamma".into()],
            },
        )
        .unwrap();
    assert_eq!(approved, revision + 1);
    let snapshot = custom_snapshot(&mut store, host).unwrap();
    assert_eq!(snapshot.clients.len(), 2);
    for (index, service) in snapshot.services.iter().enumerate() {
        assert!(service.allow.contains(&other.endpoint_id()));
        assert_eq!(
            service.allow.contains(&device.endpoint_id()),
            service.id != "beta"
        );
        assert_eq!(
            service.incarnation,
            before["services"][index]["incarnation"]
        );
        assert_eq!(
            service.backend.to_string(),
            before["services"][index]["backend"]
        );
    }
    assert!(
        store
            .host_mutate(
                host,
                &owner,
                Mutation::ApproveInvitation {
                    expected_revision: approved,
                    invitation,
                    services: vec!["beta".into()]
                }
            )
            .is_err()
    );
    assert_eq!(
        custom_snapshot(&mut store, host).unwrap().revision,
        approved
    );
    // Old callers omit services; re-pairing never withdraws or restores any other grant.
    let re_pair = pending(&mut store, host, &owner, &device);
    let old_request: Mutation = serde_json::from_value(serde_json::json!({"op":"approve_invitation","expected_revision":approved,"invitation":re_pair})).unwrap();
    revision = store.host_mutate(host, &owner, old_request).unwrap();
    let next = pending(&mut store, host, &owner, &Identity::generate());
    // Remove/recreate a reviewed ID: a stale approval cannot grant the new service.
    let reviewed = revision;
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::Remove {
                expected_revision: revision,
                id: "gamma".into(),
            },
        )
        .unwrap();
    revision = store
        .host_mutate(
            host,
            &owner,
            Mutation::Service {
                expected_revision: revision,
                id: "gamma".into(),
                backend: "127.0.0.1:8201".parse().unwrap(),
                allow: vec![other.endpoint_id()],
                display_name: None,
                kind: None,
            },
        )
        .unwrap();
    assert!(matches!(
        store.host_mutate(
            host,
            &owner,
            Mutation::ApproveInvitation {
                expected_revision: reviewed,
                invitation: next.clone(),
                services: vec!["gamma".into()]
            }
        ),
        Err(ControlError::Conflict)
    ));
    store
        .pairing_list(host, &owner, wispnet_registry::enrollment::now() + 301)
        .unwrap();
    assert!(
        store
            .host_mutate(
                host,
                &owner,
                Mutation::ApproveInvitation {
                    expected_revision: revision,
                    invitation: next,
                    services: vec!["alpha".into()]
                }
            )
            .is_err()
    );
    drop(store);
    let (mut store, _) = host::owner_store(directory.path(), host).unwrap();
    let snapshot = custom_snapshot(&mut store, host).unwrap();
    assert_eq!(snapshot.revision, revision);
    assert_eq!(snapshot.clients.len(), 2);
    assert!(
        snapshot
            .services
            .iter()
            .find(|s| s.id == "alpha")
            .unwrap()
            .allow
            .contains(&device.endpoint_id())
    );
    assert!(
        !snapshot
            .services
            .iter()
            .find(|s| s.id == "beta")
            .unwrap()
            .allow
            .contains(&device.endpoint_id())
    );
}

#[test]
fn mutual_bootstrap_requires_proof_and_owner_consent_and_never_grants_services() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = Identity::generate().endpoint_id();
    let requester = Identity::generate();
    let peer = requester.endpoint_id();
    host::initialize(directory.path(), host).unwrap();
    let (mut store, owner) = host::owner_store(directory.path(), host).unwrap();
    let now = wispnet_registry::enrollment::now();
    let invitation = store
        .pairing_invite(host, &owner, "https://relay.example", now, 300)
        .unwrap();
    let challenge = store
        .pairing_challenge(
            host,
            peer,
            &Claim {
                invitation: invitation.invitation.clone(),
                code: invitation.code.clone(),
                name: "Requester".into(),
            },
            now,
        )
        .unwrap();
    let proof = Proof {
        signature: requester.secret_key().sign(&challenge.signing_bytes()),
        challenge,
    };
    let mut bootstrap = wispnet_core::pairing::Bootstrap {
        version: 1,
        host,
        relay_url: "https://relay.example/".into(),
        direct_hint: None,
    };
    // A proved peer cannot substitute a third machine's identity for the reverse pin.
    assert!(
        store
            .pairing_prove_with_bootstrap(host, peer, &proof, Some(&bootstrap), now)
            .is_err()
    );
    bootstrap.host = peer;
    store
        .pairing_prove_with_bootstrap(host, peer, &proof, Some(&bootstrap), now)
        .unwrap();
    assert!(store.paired_hosts(host).unwrap().is_empty());
    assert!(
        custom_snapshot(&mut store, host)
            .unwrap()
            .clients
            .is_empty()
    );
    assert!(store.pairing_list(host, &owner, now).unwrap()[0].mutual);
    // A failed owner transaction cannot expose the reverse pin either.
    assert!(
        store
            .host_mutate(
                host,
                &owner,
                Mutation::ApproveInvitation {
                    expected_revision: 0,
                    invitation: invitation.invitation.clone(),
                    services: vec!["missing".into()],
                }
            )
            .is_err()
    );
    assert!(store.paired_hosts(host).unwrap().is_empty());
    store
        .host_mutate(
            host,
            &owner,
            Mutation::ApproveInvitation {
                expected_revision: 0,
                invitation: invitation.invitation.clone(),
                services: vec![],
            },
        )
        .unwrap();
    assert_eq!(store.paired_hosts(host).unwrap()[0].host, peer);
    drop(store);
    let (mut store, _) = host::owner_store(directory.path(), host).unwrap();
    assert_eq!(store.paired_hosts(host).unwrap()[0].host, peer);
    assert!(
        store
            .pairing_accept_host(host, "wrong owner", &bootstrap, "Requester")
            .is_err()
    );
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Service {
                expected_revision: 1,
                id: "private".into(),
                display_name: Some("Private".into()),
                kind: Some(wispnet_core::host::ServiceKind::Http),
                backend: "127.0.0.1:8201".parse().unwrap(),
                allow: vec![],
            },
        )
        .unwrap();
    assert!(
        custom_snapshot(&mut store, host).unwrap().services[0]
            .allow
            .is_empty()
    );
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Service {
                expected_revision: 2,
                id: "private".into(),
                display_name: None,
                kind: None,
                backend: "127.0.0.1:8201".parse().unwrap(),
                allow: vec![peer],
            },
        )
        .unwrap();
    // Re-pairing preserves an existing owner's choices rather than clearing grants.
    store
        .pairing_accept_host(host, &owner, &bootstrap, "Requester")
        .unwrap();
    assert_eq!(
        custom_snapshot(&mut store, host).unwrap().services[0].allow,
        vec![peer]
    );
    store
        .host_mutate(
            host,
            &owner,
            Mutation::Revoke {
                expected_revision: 4,
                peer,
            },
        )
        .unwrap();
    assert!(store.paired_hosts(host).unwrap().is_empty());
}
