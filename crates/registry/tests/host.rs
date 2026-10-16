use std::os::unix::fs::PermissionsExt;
use wipsnet_core::identity::Identity;
use wipsnet_registry::{
    control,
    host::{self, Mutation},
    store::Store,
};
fn directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
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
    assert_eq!(store.host_snapshot(a).unwrap().clients[0].peer, client);
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
                    expected_revision: 0,
                    id: "web".into(),
                    backend: "127.0.0.1:8080".parse().unwrap(),
                    allow: vec![client]
                }
            )
            .is_err()
    );
    assert_eq!(store.host_snapshot(host).unwrap().revision, 0);
    let legacy_dir = directory();
    let mut legacy = Store::open(&legacy_dir.path().join("registry.sqlite3")).unwrap();
    legacy.replace(0, &[client], &[]).unwrap();
    assert!(legacy.host_initialize(host, &secret).is_err());
    assert_eq!(legacy.snapshot().unwrap().devices, vec![client]);
}
