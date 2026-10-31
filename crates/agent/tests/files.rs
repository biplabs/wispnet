#![cfg(unix)]
use std::{
    collections::BTreeMap,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};
use wispnet_agent::tunnel::{self, TunnelHost};
use wispnet_core::{
    config::TransportConfig,
    identity::{EndpointId, Identity},
    protocol::Status,
};
use wispnet_files::{ErrorCode, Request, Response, TransferState, read_response, write_request};
use wispnet_registry::host::{self, Authority, Mutation};

fn config() -> TransportConfig {
    TransportConfig {
        backup_relay_urls: vec![],
        discovery_relay_url: None,
        relay_url: None,
        relay_only: false,
        relay_ca_file: None,
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        max_connections: 8,
        max_streams: 16,
        max_streams_per_connection: 8,
    }
}
fn mutate(dir: &Path, host: EndpointId, op: Mutation) {
    let (mut store, owner) = host::owner_store(dir, host).unwrap();
    store.host_mutate(host, &owner, op).unwrap();
}
async fn rpc(stream: &mut tunnel::ServiceStream, request: Request) -> Response {
    write_request(stream, &request).await.unwrap();
    read_response(stream).await.unwrap().unwrap()
}
async fn incarnation(authority: &Authority) -> String {
    authority
        .services()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == "files")
        .unwrap()
        .incarnation
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_files_grants_restart_lost_ack_commit_and_revocation() {
    let state = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let export = tempfile::tempdir().unwrap();
    let identity = Identity::generate();
    let own = identity.endpoint_id();
    let allowed = Identity::generate();
    let denied = Identity::generate();
    host::initialize(state.path(), own).unwrap();
    for (revision, peer) in [allowed.endpoint_id(), denied.endpoint_id()]
        .into_iter()
        .enumerate()
    {
        mutate(
            state.path(),
            own,
            Mutation::Approve {
                expected_revision: revision as u64,
                peer,
                name: "client".into(),
            },
        );
    }
    let exports = BTreeMap::from([("jobs".into(), export.path().canonicalize().unwrap())]);
    mutate(
        state.path(),
        own,
        Mutation::Files {
            expected_revision: 2,
            allow: vec![allowed.endpoint_id()],
            exports: exports.clone(),
        },
    );
    // Shell authorization does not grant Files.
    mutate(
        state.path(),
        own,
        Mutation::Shell {
            expected_revision: 3,
            allow: vec![denied.endpoint_id()],
        },
    );
    let authority = Authority::open(state.path(), own).unwrap();
    let inc = incarnation(&authority).await;
    let host = TunnelHost::bind_host(identity.secret_key(), &config(), authority.clone())
        .await
        .unwrap();
    let endpoint = tunnel::bind_client(allowed.secret_key()).await.unwrap();
    let connection = tunnel::connect(&endpoint, own, host.endpoint().bound_sockets()[0])
        .await
        .unwrap();
    let other = tunnel::bind_client(denied.secret_key()).await.unwrap();
    let other_connection = tunnel::connect(&other, own, host.endpoint().bound_sockets()[0])
        .await
        .unwrap();
    assert!(matches!(
        tunnel::open_service_bound(&other_connection, "files", &inc).await,
        Err(tunnel::TunnelError::Status(Status::Denied))
    ));
    assert!(
        tunnel::open_service_bound(&connection, "files", &"ab".repeat(32))
            .await
            .is_err()
    );
    let mut stream = tunnel::open_service_bound(&connection, "files", &inc)
        .await
        .unwrap();
    let bytes = vec![73; 1024 * 1024];
    let begin = Request::UploadBegin {
        workspace: "jobs".into(),
        path: "run/input.bin".into(),
        size_bytes: bytes.len() as u64,
        sha256: wispnet_files::sha256(&bytes),
        idempotency_key: "ab".repeat(32),
    };
    let Response::Upload { transfer_id, .. } = rpc(&mut stream, begin.clone()).await else {
        panic!("begin failed")
    };
    let write = Request::UploadWrite {
        transfer_id: transfer_id.clone(),
        offset: 0,
        chunk_sha256: wispnet_files::sha256(&bytes),
        data: bytes.clone(),
    };
    assert_eq!(
        rpc(&mut stream, write.clone()).await,
        Response::Written {
            committed_offset: bytes.len() as u64
        }
    );
    // Lose a duplicate acknowledgement, then restart the entire native node.
    write_request(&mut stream, &write).await.unwrap();
    drop(stream);
    host.shutdown().await;
    let host = TunnelHost::bind_host(identity.secret_key(), &config(), authority.clone())
        .await
        .unwrap();
    let connection = tunnel::connect(&endpoint, own, host.endpoint().bound_sockets()[0])
        .await
        .unwrap();
    let mut stream = tunnel::open_service_bound(&connection, "files", &inc)
        .await
        .unwrap();
    assert!(
        matches!(rpc(&mut stream, begin).await, Response::Upload { transfer_id: ref id, committed_offset, .. } if id == &transfer_id && committed_offset == bytes.len() as u64)
    );
    assert_eq!(
        rpc(&mut stream, write).await,
        Response::Written {
            committed_offset: bytes.len() as u64
        }
    );
    let committed = rpc(
        &mut stream,
        Request::UploadCommit {
            transfer_id: transfer_id.clone(),
        },
    )
    .await;
    assert!(matches!(committed, Response::File { .. }), "{committed:?}");
    let Response::File { file: ref info } = committed else {
        panic!("commit did not return file info")
    };
    let denied_read = rpc(
        &mut stream,
        Request::FileReadRange {
            workspace: "jobs".into(),
            path: "run/input.bin".into(),
            expected_version: info.file_version.clone(),
            offset: 0,
            max_bytes: bytes.len() as u64,
        },
    )
    .await;
    assert!(matches!(
        denied_read,
        Response::Error {
            code: ErrorCode::PermissionDenied,
            ..
        }
    ));
    // A standing incoming grant cannot inspect files already in the receiving
    // folder, even when the sender knows the path and a real content version.
    std::fs::write(export.path().join("receiver-private.bin"), b"private").unwrap();
    for path in ["run/input.bin", "receiver-private.bin", "missing.bin"] {
        for request in [
            Request::FileStat {
                workspace: "jobs".into(),
                path: path.into(),
            },
            Request::FileReadRange {
                workspace: "jobs".into(),
                path: path.into(),
                expected_version: info.file_version.clone(),
                offset: 0,
                max_bytes: 1,
            },
        ] {
            assert!(matches!(
                rpc(&mut stream, request).await,
                Response::Error {
                    code: ErrorCode::PermissionDenied,
                    ..
                }
            ));
        }
    }
    assert_eq!(
        rpc(
            &mut stream,
            Request::UploadCommit {
                transfer_id: transfer_id.clone()
            }
        )
        .await,
        committed
    );
    assert!(matches!(
        rpc(&mut stream, Request::UploadStatus { transfer_id }).await,
        Response::Upload {
            state: TransferState::Committed,
            file: Some(_),
            ..
        }
    ));
    assert_eq!(
        std::fs::read(export.path().join("run/input.bin")).unwrap(),
        bytes
    );
    assert!(matches!(
        rpc(
            &mut stream,
            Request::FileStat {
                workspace: "jobs".into(),
                path: "../escape".into()
            }
        )
        .await,
        Response::Error { .. }
    ));
    mutate(
        state.path(),
        own,
        Mutation::Files {
            expected_revision: 4,
            allow: vec![],
            exports,
        },
    );
    tokio::time::timeout(Duration::from_secs(4), async {
        while matches!(read_response(&mut stream).await, Ok(Some(_))) {}
    })
    .await
    .unwrap();
    assert!(
        tunnel::open_service_bound(&connection, "files", &inc)
            .await
            .is_err()
    );
    other.close().await;
    endpoint.close().await;
    host.shutdown().await;
}

/// Actual filesystem-backed, hashed binary payload over authenticated Iroh/QUIC.
/// Run `cargo test -p wispnet-agent --release --test files native_files_bulk -- --ignored --nocapture`.
/// This measures loopback capability; a WAN/relay result must be measured separately.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "1 GiB native Files throughput and recovery acceptance benchmark"]
async fn native_files_bulk() {
    let state = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let export = tempfile::tempdir().unwrap();
    let identity = Identity::generate();
    let client = Identity::generate();
    let own = identity.endpoint_id();
    host::initialize(state.path(), own).unwrap();
    mutate(
        state.path(),
        own,
        Mutation::Approve {
            expected_revision: 0,
            peer: client.endpoint_id(),
            name: "benchmark".into(),
        },
    );
    let exports = BTreeMap::from([("bench".into(), export.path().canonicalize().unwrap())]);
    mutate(
        state.path(),
        own,
        Mutation::Files {
            expected_revision: 1,
            allow: vec![client.endpoint_id()],
            exports,
        },
    );
    let authority = Authority::open(state.path(), own).unwrap();
    let inc = incarnation(&authority).await;
    let chunk = vec![139; wispnet_files::MAX_CHUNK];
    let chunks = 256;
    let total = (chunks * chunk.len()) as u64;
    let source = state.path().join("source.bin");
    let mut file = std::fs::File::create(&source).unwrap();
    for _ in 0..chunks {
        file.write_all(&chunk).unwrap();
    }
    file.sync_all().unwrap();
    let hash = wispnet_files::hash_file(&source).unwrap();
    let host = TunnelHost::bind_host(identity.secret_key(), &config(), authority.clone())
        .await
        .unwrap();
    let endpoint = tunnel::bind_client(client.secret_key()).await.unwrap();
    let connection = tunnel::connect(&endpoint, own, host.endpoint().bound_sockets()[0])
        .await
        .unwrap();
    let mut stream = tunnel::open_service_bound(&connection, "files", &inc)
        .await
        .unwrap();
    let begin = Request::UploadBegin {
        workspace: "bench".into(),
        path: "artifact.bin".into(),
        size_bytes: total,
        sha256: hash.clone(),
        idempotency_key: "cd".repeat(32),
    };
    let Response::Upload { transfer_id, .. } = rpc(&mut stream, begin.clone()).await else {
        panic!("begin failed")
    };
    // Leave 4 MiB durable, restart and ensure the 1 GiB transfer resumes.
    assert!(matches!(
        rpc(
            &mut stream,
            Request::UploadWrite {
                transfer_id: transfer_id.clone(),
                offset: 0,
                chunk_sha256: wispnet_files::sha256(&chunk),
                data: chunk.clone()
            }
        )
        .await,
        Response::Written { .. }
    ));
    drop(stream);
    host.shutdown().await;
    let host = TunnelHost::bind_host(identity.secret_key(), &config(), authority)
        .await
        .unwrap();
    let connection = tunnel::connect(&endpoint, own, host.endpoint().bound_sockets()[0])
        .await
        .unwrap();
    let mut stream = tunnel::open_service_bound(&connection, "files", &inc)
        .await
        .unwrap();
    assert!(
        matches!(rpc(&mut stream, begin).await, Response::Upload { committed_offset, .. } if committed_offset == chunk.len() as u64)
    );
    let started = Instant::now();
    let (mut reader, mut writer) = tokio::io::split(stream);
    let chunk_hash = wispnet_files::sha256(&chunk);
    // Eight unacknowledged chunks bound application-level in-flight bytes to 32 MiB.
    let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let sender_permits = permits.clone();
    let sender = async {
        for n in 1..chunks {
            let permit = sender_permits.acquire().await.unwrap();
            write_request(
                &mut writer,
                &Request::UploadWrite {
                    transfer_id: transfer_id.clone(),
                    offset: (n * chunk.len()) as u64,
                    chunk_sha256: chunk_hash.clone(),
                    data: chunk.clone(),
                },
            )
            .await
            .unwrap();
            permit.forget();
        }
    };
    let receiver = async {
        for n in 1..chunks {
            let Response::Written { committed_offset } =
                read_response(&mut reader).await.unwrap().unwrap()
            else {
                panic!("upload chunk was not acknowledged")
            };
            assert!((((n + 1) * chunk.len()) as u64..=total).contains(&committed_offset));
            permits.add_permits(1);
        }
    };
    let control = async {
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut status = tunnel::open_service_bound(&connection, "files", &inc)
                .await
                .unwrap();
            assert!(matches!(
                rpc(
                    &mut status,
                    Request::UploadStatus {
                        transfer_id: transfer_id.clone()
                    }
                )
                .await,
                Response::Upload {
                    state: TransferState::Uploading,
                    ..
                }
            ));
        })
        .await
        .expect("Files status was blocked by bulk transfer");
    };
    tokio::join!(sender, receiver, control);
    let transfer_seconds = started.elapsed().as_secs_f64();
    let mut stream = reader.unsplit(writer);
    let committed = rpc(&mut stream, Request::UploadCommit { transfer_id }).await;
    assert!(
        matches!(&committed, Response::File { file } if file.sha256 == hash && file.size_bytes == total),
        "{committed:?}"
    );
    let completed_seconds = started.elapsed().as_secs_f64();
    assert_eq!(
        wispnet_files::hash_file(&export.path().join("artifact.bin")).unwrap(),
        hash
    );
    eprintln!(
        "Files native loopback: {} bytes resumed after restart; payload {:.3} Gbit/s in {:.3}s; verified/published {:.3}s",
        total,
        ((total - chunk.len() as u64) as f64 * 8.0 / transfer_seconds / 1e9),
        transfer_seconds,
        completed_seconds
    );
    endpoint.close().await;
    host.shutdown().await;
}
