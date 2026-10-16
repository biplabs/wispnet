//! Real Iroh handshakes with persisted keys. No public discovery or relay services.
use iroh::{Endpoint, EndpointAddr, endpoint::presets};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use wipsnet_agent::identity::FileIdentityStore;
use wipsnet_core::identity::{Identity, IdentityStore};

const ALPN: &[u8] = b"wipsnet/identity-test/1";

async fn endpoint(identity: &Identity) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(identity.secret_key().clone())
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap()
}

#[tokio::test]
async fn persisted_keys_authenticate_peers_and_wrong_expected_identity_fails() {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    for dir in [&a_dir, &b_dir] {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let a_store = FileIdentityStore::new(a_dir.path()).unwrap();
    let b_store = FileIdentityStore::new(b_dir.path()).unwrap();
    let a_id = a_store.initialize().unwrap().endpoint_id();
    let b_id = b_store.initialize().unwrap().endpoint_id();
    // Reload from disk before constructing the actual transport endpoints.
    let a = endpoint(&a_store.load().unwrap()).await;
    let b = endpoint(&b_store.load().unwrap()).await;
    let b_socket = b.bound_sockets()[0];
    let address = EndpointAddr::new(b_id).with_ip_addr(b_socket);
    let exchange = async {
        let (outgoing, incoming) = tokio::join!(a.connect(address, ALPN), async {
            b.accept().await.unwrap().await
        });
        let outgoing = outgoing.unwrap();
        let incoming = incoming.unwrap();
        assert_eq!(outgoing.remote_id(), b_id);
        assert_eq!(incoming.remote_id(), a_id);
        outgoing.close(0u32.into(), b"done");
        incoming.close(0u32.into(), b"done");
    };
    let correct = tokio::time::timeout(Duration::from_secs(5), exchange).await;
    // An actual B socket must not authenticate as a different device. A bounded
    // timeout is also a denial: no application stream can be opened in that case.
    let wrong_address =
        EndpointAddr::new(Identity::generate().endpoint_id()).with_ip_addr(b_socket);
    let (wrong, _) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(2), a.connect(wrong_address, ALPN)),
        tokio::time::timeout(Duration::from_secs(2), async {
            b.accept().await.unwrap().await
        })
    );
    a.close().await;
    b.close().await;
    assert!(correct.is_ok(), "expected peer handshake timed out");
    assert!(
        !matches!(wrong, Ok(Ok(_))),
        "wrong endpoint identity authenticated"
    );
}
