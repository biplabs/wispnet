use wispnet_core::{
    host::ServiceKind,
    identity::{EndpointId, Identity},
};
use wispnet_registry::{
    control::ControlError,
    host::{self, Mutation, Snapshot},
    store::Store,
};

struct Host {
    _directory: tempfile::TempDir,
    id: EndpointId,
    store: Store,
    owner: zeroize::Zeroizing<String>,
    revision: u64,
}

impl Host {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let id = Identity::generate().endpoint_id();
        host::initialize(directory.path(), id).unwrap();
        let (store, owner) = host::owner_store(directory.path(), id).unwrap();
        Self {
            _directory: directory,
            id,
            store,
            owner,
            revision: 0,
        }
    }

    fn mutate(&mut self, operation: Mutation) {
        self.revision = self
            .store
            .host_mutate(self.id, &self.owner, operation)
            .unwrap();
    }

    fn approve(&mut self) -> EndpointId {
        let peer = Identity::generate().endpoint_id();
        self.mutate(Mutation::Approve {
            expected_revision: self.revision,
            peer,
            name: "client".into(),
        });
        peer
    }

    fn service(&mut self, id: &str, kind: ServiceKind, allow: Vec<EndpointId>) {
        self.mutate(Mutation::Service {
            expected_revision: self.revision,
            id: id.into(),
            backend: "127.0.0.1:8080".parse().unwrap(),
            allow,
            display_name: None,
            kind: Some(kind),
        });
    }

    fn grant(&mut self, service: &str, peer: EndpointId) {
        self.mutate(Mutation::GrantService {
            expected_revision: self.revision,
            service: service.into(),
            peer,
        });
    }

    fn revoke(&mut self, service: &str, peer: EndpointId) {
        self.mutate(Mutation::RevokeService {
            expected_revision: self.revision,
            service: service.into(),
            peer,
        });
    }

    fn snapshot(&mut self) -> Snapshot {
        self.store.host_snapshot(self.id).unwrap()
    }
}

fn after_grant(before: Snapshot, service: &str, peer: EndpointId) -> serde_json::Value {
    let mut expected = serde_json::to_value(before).unwrap();
    expected["revision"] = (expected["revision"].as_u64().unwrap() + 1).into();
    let service = expected["services"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["id"] == service)
        .unwrap();
    let allow = service["allow"].as_array_mut().unwrap();
    allow.push(peer.to_string().into());
    allow.sort_by(|a, b| a.as_str().unwrap().cmp(b.as_str().unwrap()));
    allow.dedup();
    expected
}

fn after_revoke(before: Snapshot, service: &str, peer: EndpointId) -> serde_json::Value {
    let mut expected = serde_json::to_value(before).unwrap();
    expected["revision"] = (expected["revision"].as_u64().unwrap() + 1).into();
    let service = expected["services"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["id"] == service)
        .unwrap();
    service["allow"]
        .as_array_mut()
        .unwrap()
        .retain(|entry| *entry != peer.to_string());
    expected
}

#[test]
fn generic_grant_preserves_everything_else_for_http_tcp_and_builtin_shell() {
    let mut host = Host::new();
    let existing = host.approve();
    let added = host.approve();
    host.service("web", ServiceKind::Http, vec![existing]);
    host.service("database", ServiceKind::Tcp, vec![existing]);
    host.mutate(Mutation::Shell {
        expected_revision: host.revision,
        allow: vec![existing],
    });

    for service in ["web", "database", "shell"] {
        let expected = after_grant(host.snapshot(), service, added);
        host.grant(service, added);
        assert_eq!(serde_json::to_value(host.snapshot()).unwrap(), expected);

        // Repeated grants change only the revision, never duplicate the grant.
        let expected = after_grant(host.snapshot(), service, added);
        host.grant(service, added);
        assert_eq!(serde_json::to_value(host.snapshot()).unwrap(), expected);
    }
}

#[test]
fn generic_grant_rejects_invalid_unknown_unapproved_and_stale_without_mutation() {
    let mut host = Host::new();
    let approved = host.approve();
    let unapproved = Identity::generate().endpoint_id();
    host.service("web", ServiceKind::Http, vec![approved]);
    let before = serde_json::to_value(host.snapshot()).unwrap();

    for (service, peer, revision, expected_error) in [
        ("bad/name", approved, host.revision, "invalid"),
        ("missing", approved, host.revision, "invalid"),
        ("web", unapproved, host.revision, "forbidden"),
        ("shell", unapproved, host.revision, "forbidden"),
        ("web", approved, host.revision - 1, "conflict"),
    ] {
        let error = host
            .store
            .host_mutate(
                host.id,
                &host.owner,
                Mutation::GrantService {
                    expected_revision: revision,
                    service: service.into(),
                    peer,
                },
            )
            .unwrap_err();
        assert!(matches!(
            (expected_error, error),
            ("invalid", ControlError::Invalid)
                | ("forbidden", ControlError::Forbidden)
                | ("conflict", ControlError::Conflict)
        ));
        assert_eq!(serde_json::to_value(host.snapshot()).unwrap(), before);
    }
}

#[test]
fn generic_grant_limits_each_service_to_128_distinct_peers_and_allows_regrant() {
    let mut host = Host::new();
    let peers = (0..129).map(|_| host.approve()).collect::<Vec<_>>();
    host.service("web", ServiceKind::Http, vec![]);
    for peer in &peers[..128] {
        host.grant("shell", *peer);
    }
    let before = serde_json::to_value(host.snapshot()).unwrap();
    assert!(matches!(
        host.store.host_mutate(
            host.id,
            &host.owner,
            Mutation::GrantService {
                expected_revision: host.revision,
                service: "shell".into(),
                peer: peers[128],
            }
        ),
        Err(ControlError::Invalid)
    ));
    assert_eq!(serde_json::to_value(host.snapshot()).unwrap(), before);

    let expected = after_grant(host.snapshot(), "shell", peers[0]);
    host.grant("shell", peers[0]);
    assert_eq!(serde_json::to_value(host.snapshot()).unwrap(), expected);

    // The full shell service does not consume another service's grant slots.
    let expected = after_grant(host.snapshot(), "web", peers[128]);
    host.grant("web", peers[128]);
    assert_eq!(serde_json::to_value(host.snapshot()).unwrap(), expected);
}

#[test]
fn generic_grant_uses_the_owner_mutation_wire_contract() {
    let peer = Identity::generate().endpoint_id();
    let request = serde_json::json!({
        "op": "grant_service",
        "expected_revision": 7,
        "service": "shell",
        "peer": peer,
    });
    let mutation: Mutation = serde_json::from_value(request.clone()).unwrap();
    assert_eq!(serde_json::to_value(mutation).unwrap(), request);
}

#[tokio::test]
async fn generic_revoke_preserves_everything_else_for_http_tcp_and_builtin_shell() {
    let mut host = Host::new();
    let preserved = host.approve();
    let removed = host.approve();
    host.service("web", ServiceKind::Http, vec![preserved, removed]);
    host.service("database", ServiceKind::Tcp, vec![preserved, removed]);
    host.mutate(Mutation::Shell {
        expected_revision: host.revision,
        allow: vec![preserved, removed],
    });
    let authority = host::Authority::open(host._directory.path(), host.id).unwrap();
    let services = ["web", "database", "shell"];

    for (index, service) in services.into_iter().enumerate() {
        let expected = after_revoke(host.snapshot(), service, removed);
        host.revoke(service, removed);
        assert_eq!(serde_json::to_value(host.snapshot()).unwrap(), expected);

        assert!(authority.approved(removed).await);
        let catalog = authority.catalog(removed).await.unwrap();
        assert_eq!(catalog.services.len(), services.len() - index - 1);
        for allowed in &services[index + 1..] {
            assert!(catalog.services.iter().any(|entry| entry.id == *allowed));
        }
        assert!(!catalog.services.iter().any(|entry| entry.id == service));
        assert!(authority.approved(preserved).await);
        let preserved_catalog = authority.catalog(preserved).await.unwrap();
        assert_eq!(preserved_catalog.services.len(), services.len());
        for allowed in services {
            assert!(
                preserved_catalog
                    .services
                    .iter()
                    .any(|entry| entry.id == allowed)
            );
        }

        // The paired device stays approved, so an already absent grant is valid.
        let expected = after_revoke(host.snapshot(), service, removed);
        host.revoke(service, removed);
        assert_eq!(serde_json::to_value(host.snapshot()).unwrap(), expected);
    }

    // Revoking service access leaves the pairing available for a later grant.
    let expected = after_grant(host.snapshot(), "shell", removed);
    host.grant("shell", removed);
    assert_eq!(serde_json::to_value(host.snapshot()).unwrap(), expected);
    let catalog = authority.catalog(removed).await.unwrap();
    assert_eq!(catalog.services.len(), 1);
    assert_eq!(catalog.services[0].id, "shell");
}

#[test]
fn generic_revoke_rejects_invalid_unknown_unapproved_stale_and_wrong_owner_without_mutation() {
    let mut host = Host::new();
    let approved = host.approve();
    let unapproved = Identity::generate().endpoint_id();
    host.service("web", ServiceKind::Http, vec![approved]);
    host.grant("shell", approved);
    let before = serde_json::to_value(host.snapshot()).unwrap();

    for (service, peer, revision, expected_error) in [
        ("bad/name", approved, host.revision, "invalid"),
        ("missing", approved, host.revision, "invalid"),
        ("web", unapproved, host.revision, "forbidden"),
        ("shell", unapproved, host.revision, "forbidden"),
        ("web", approved, host.revision - 1, "conflict"),
    ] {
        let error = host
            .store
            .host_mutate(
                host.id,
                &host.owner,
                Mutation::RevokeService {
                    expected_revision: revision,
                    service: service.into(),
                    peer,
                },
            )
            .unwrap_err();
        assert!(matches!(
            (expected_error, error),
            ("invalid", ControlError::Invalid)
                | ("forbidden", ControlError::Forbidden)
                | ("conflict", ControlError::Conflict)
        ));
        assert_eq!(serde_json::to_value(host.snapshot()).unwrap(), before);
    }

    assert!(matches!(
        host.store.host_mutate(
            host.id,
            "wrong-owner-secret",
            Mutation::RevokeService {
                expected_revision: host.revision,
                service: "shell".into(),
                peer: approved,
            }
        ),
        Err(ControlError::Forbidden)
    ));
    assert_eq!(serde_json::to_value(host.snapshot()).unwrap(), before);
}

#[test]
fn generic_revoke_uses_the_owner_mutation_wire_contract() {
    let peer = Identity::generate().endpoint_id();
    let request = serde_json::json!({
        "op": "revoke_service",
        "expected_revision": 7,
        "service": "shell",
        "peer": peer,
    });
    let mutation: Mutation = serde_json::from_value(request.clone()).unwrap();
    assert_eq!(serde_json::to_value(mutation).unwrap(), request);
}
