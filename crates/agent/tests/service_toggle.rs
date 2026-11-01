#![cfg(unix)]
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wispnet_agent::tunnel::{self, TunnelHost};
use wispnet_core::{config::TransportConfig, identity::Identity};
use wispnet_registry::host::{self, Authority, Mutation};

#[tokio::test]
async fn toggles_close_streams_hide_catalog_and_preserve_grants_for_every_service_kind() {
    let state = tempfile::tempdir().unwrap();
    std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let export = tempfile::tempdir().unwrap();
    let identity = Identity::generate();
    let own = identity.endpoint_id();
    let client = Identity::generate();
    let peer = client.endpoint_id();
    host::initialize(state.path(), own).unwrap();
    let (mut store, owner) = host::owner_store(state.path(), own).unwrap();
    store
        .host_mutate(
            own,
            &owner,
            Mutation::Approve {
                expected_revision: 0,
                peer,
                name: "client".into(),
            },
        )
        .unwrap();
    let backend = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = backend.local_addr().unwrap();
    let echo = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = backend.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let (mut r, mut w) = stream.into_split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    let exports = BTreeMap::from([("shared".into(), export.path().canonicalize().unwrap())]);
    for op in [
        Mutation::Service {
            expected_revision: 1,
            id: "web".into(),
            backend: addr,
            allow: vec![peer],
            display_name: None,
            kind: None,
        },
        Mutation::Shell {
            expected_revision: 2,
            allow: vec![peer],
        },
        Mutation::Files {
            expected_revision: 3,
            allow: vec![peer],
            exports: exports.clone(),
        },
    ] {
        store.host_mutate(own, &owner, op).unwrap();
    }
    let authority = Authority::open(state.path(), own).unwrap();
    let config: TransportConfig =
        serde_json::from_value(serde_json::json!({"bind_addr":"127.0.0.1:0"})).unwrap();
    let host = TunnelHost::bind_host(identity.secret_key(), &config, authority.clone())
        .await
        .unwrap();
    let endpoint = tunnel::bind_client(client.secret_key()).await.unwrap();
    let connection = tunnel::connect(&endpoint, own, host.endpoint().bound_sockets()[0])
        .await
        .unwrap();
    for id in ["web", "shell", "files"] {
        let before = store.host_snapshot(own).unwrap();
        let service = before.services.iter().find(|s| s.id == id).unwrap();
        assert!(service.enabled);
        let inc = service.incarnation.clone();
        let mut stream = tunnel::open_service_bound(&connection, id, &inc)
            .await
            .unwrap();
        // Open the built-in protocol handlers, not only the outer tunnel.
        match id {
            "shell" => {
                wispnet_shell::write_client(
                    &mut stream,
                    &wispnet_shell::ClientFrame::Attach {
                        session: "toggle".into(),
                        cols: 80,
                        rows: 24,
                    },
                )
                .await
                .unwrap();
                assert!(matches!(
                    wispnet_shell::read_server(&mut stream).await.unwrap(),
                    Some(wispnet_shell::ServerFrame::Attached { .. })
                ));
            }
            "files" => {
                wispnet_files::write_request(&mut stream, &wispnet_files::Request::Workspaces)
                    .await
                    .unwrap();
                assert!(matches!(
                    wispnet_files::read_response(&mut stream).await.unwrap(),
                    Some(wispnet_files::Response::Workspaces { .. })
                ));
            }
            _ => {
                stream.write_all(b"ping").await.unwrap();
                let mut data = [0; 4];
                stream.read_exact(&mut data).await.unwrap();
                assert_eq!(&data, b"ping");
            }
        }
        store
            .host_mutate(
                own,
                &owner,
                Mutation::SetServiceEnabled {
                    expected_revision: before.revision,
                    id: id.into(),
                    incarnation: inc.clone(),
                    enabled: false,
                },
            )
            .unwrap();
        assert!(
            !authority
                .catalog(peer)
                .await
                .unwrap()
                .services
                .iter()
                .any(|s| s.id == id)
        );
        assert!(
            tunnel::open_service_bound(&connection, id, &inc)
                .await
                .is_err()
        );
        // Active HTTP, PTY, and Files streams must all stop after the policy tick.
        let mut remaining = vec![];
        let _ = tokio::time::timeout(Duration::from_secs(4), stream.read_to_end(&mut remaining))
            .await
            .expect("disabled stream remained open");
        drop(store);
        (store, _) = host::owner_store(state.path(), own).unwrap();
        let paused = store.host_snapshot(own).unwrap();
        let paused_service = paused.services.iter().find(|s| s.id == id).unwrap();
        assert!(!paused_service.enabled);
        assert_eq!(paused_service.allow, vec![peer]);
        assert_eq!(paused_service.incarnation, inc);
        assert_eq!(paused.file_exports, exports);
        store
            .host_mutate(
                own,
                &owner,
                Mutation::SetServiceEnabled {
                    expected_revision: paused.revision,
                    id: id.into(),
                    incarnation: inc.clone(),
                    enabled: true,
                },
            )
            .unwrap();
        assert!(
            authority
                .catalog(peer)
                .await
                .unwrap()
                .services
                .iter()
                .any(|s| s.id == id)
        );
        assert!(
            tunnel::open_service_bound(&connection, id, &inc)
                .await
                .is_ok()
        );
    }
    endpoint.close().await;
    host.shutdown().await;
    echo.abort();
}
