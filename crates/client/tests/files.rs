#![cfg(unix)]
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::BTreeMap,
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use wispnet_client::{
    Client,
    files::{FileCancellation, FileTarget, TransferOptions},
};
use wispnet_core::{config::TransportConfig, identity::SecretKey};
use wispnet_registry::host::{self, Authority, Mutation};

fn config() -> TransportConfig {
    serde_json::from_value(json!({"bind_addr":"127.0.0.1:0"})).unwrap()
}
async fn create(state: &Path, export: &Path, key: SecretKey) -> (Arc<Client>, FileTarget) {
    std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let own = key.public();
    let client = Client::new_peer(key, state.to_owned(), config())
        .await
        .unwrap();
    let authority = Authority::open(state, own).unwrap();
    if authority.file_exports().await.unwrap().is_empty() {
        let revision = authority.snapshot().await.unwrap().revision;
        let (mut store, owner) = host::owner_store(state, own).unwrap();
        store
            .host_mutate(
                own,
                &owner,
                Mutation::Files {
                    expected_revision: revision,
                    allow: vec![],
                    exports: BTreeMap::from([("training".into(), export.canonicalize().unwrap())]),
                },
            )
            .unwrap();
    }
    let incarnation = authority
        .services()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == "files")
        .unwrap()
        .incarnation;
    (
        client,
        FileTarget {
            host: own,
            service: "files".into(),
            incarnation,
        },
    )
}
async fn remote(
    host_state: &Path,
    client_state: &Path,
    export: &Path,
    host_key: SecretKey,
    client_key: SecretKey,
) -> (wispnet_agent::tunnel::TunnelHost, Arc<Client>, FileTarget) {
    std::fs::set_permissions(host_state, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(client_state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let own = host_key.public();
    host::initialize(host_state, own).unwrap();
    let authority = Authority::open(host_state, own).unwrap();
    if authority.snapshot().await.unwrap().clients.is_empty() {
        let (mut store, owner) = host::owner_store(host_state, own).unwrap();
        store
            .host_mutate(
                own,
                &owner,
                Mutation::Approve {
                    expected_revision: 0,
                    peer: client_key.public(),
                    name: "file-client".into(),
                },
            )
            .unwrap();
        store
            .host_mutate(
                own,
                &owner,
                Mutation::Files {
                    expected_revision: 1,
                    allow: vec![client_key.public()],
                    exports: BTreeMap::from([("training".into(), export.canonicalize().unwrap())]),
                },
            )
            .unwrap();
    }
    let incarnation = authority
        .services()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == "files")
        .unwrap()
        .incarnation;
    let host = wispnet_agent::tunnel::TunnelHost::bind_host(&host_key, &config(), authority)
        .await
        .unwrap();
    wispnet_agent::peers::save(
        client_state,
        &wispnet_core::pairing::Bootstrap {
            version: 1,
            host: own,
            relay_url: "https://fixture.invalid".into(),
            direct_hint: Some(host.endpoint().bound_sockets()[0]),
        },
    )
    .unwrap();
    let client = Client::new(client_key, client_state.to_owned(), config())
        .await
        .unwrap();
    assert_ne!(client.endpoint.id(), own);
    (
        host,
        client,
        FileTarget {
            host: own,
            service: "files".into(),
            incarnation,
        },
    )
}

fn write_source(path: &Path, chunks: usize) {
    let mut source = std::fs::File::create(path).unwrap();
    for n in 0..chunks {
        source
            .write_all(&vec![n as u8; wispnet_files::MAX_CHUNK])
            .unwrap();
    }
    source.write_all(b"binary\0\xfftail").unwrap();
    source.sync_all().unwrap();
}
fn pause_after(bytes: u64) -> TransferOptions {
    let cancellation = FileCancellation::default();
    let pause = cancellation.clone();
    TransferOptions {
        cancellation,
        progress: Some(Arc::new(move |progress| {
            if progress.phase == "transferring" && progress.verified_bytes >= bytes {
                pause.cancel();
            }
        })),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sdk_upload_resumes_across_client_restart_without_restarting_prefix() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let state = tempfile::tempdir().unwrap();
        let export = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let key = SecretKey::generate();
        let source = local.path().join("source.bin");
        write_source(&source, 10); // 40 MiB crosses the 32 MiB pipeline checkpoint.
        let (client, target) = create(state.path(), export.path(), key.clone()).await;
        let error = client
            .put_file(
                target.clone(),
                "training",
                &source,
                "job/input.bin",
                pause_after(wispnet_files::MAX_CHUNK as u64),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "CANCELLED");
        assert!(!export.path().join("job/input.bin").exists());
        client.endpoint.close().await;
        drop(client);
        let (client, target) = create(state.path(), export.path(), key.clone()).await;
        let resumed_upload = Arc::new(Mutex::new(Vec::new()));
        let observed = resumed_upload.clone();
        let uploaded = client
            .put_file(
                target.clone(),
                "training",
                &source,
                "job/input.bin",
                TransferOptions {
                    progress: Some(Arc::new(move |p| {
                        if p.phase == "transferring" {
                            observed.lock().unwrap().push(p.verified_bytes);
                        }
                    })),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(resumed_upload.lock().unwrap()[0] > 0);
        assert_eq!(wispnet_files::hash_file(&source).unwrap(), uploaded.sha256);
        assert_eq!(
            wispnet_files::hash_file(&export.path().join("job/input.bin")).unwrap(),
            uploaded.sha256
        );
        client.endpoint.close().await;
        drop(client);
        let (client, target) = create(state.path(), export.path(), key).await;
        // A completed upload retains its peer-bound idempotency key and can be
        // retried after restart without remote stat or read access.
        assert_eq!(
            client
                .put_file(
                    target,
                    "training",
                    &source,
                    "job/input.bin",
                    TransferOptions::default()
                )
                .await
                .unwrap(),
            uploaded
        );
        client.endpoint.close().await;
    })
    .await
    .expect("SDK transfer test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uploads_never_replace_existing_destinations_or_claim_other_uploads() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let state = tempfile::tempdir().unwrap();
        let export = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let source = local.path().join("source.bin");
        std::fs::write(&source, b"original").unwrap();
        std::fs::write(export.path().join("existing.bin"), b"original").unwrap();
        let (client, target) = create(state.path(), export.path(), SecretKey::generate()).await;
        // Even identical pre-existing files cannot be probed or claimed by a new upload.
        assert_eq!(
            client
                .put_file(
                    target.clone(),
                    "training",
                    &source,
                    "existing.bin",
                    TransferOptions::default()
                )
                .await
                .unwrap_err()
                .code,
            "CONFLICT"
        );
        client
            .put_file(
                target.clone(),
                "training",
                &source,
                "created.bin",
                TransferOptions::default(),
            )
            .await
            .unwrap();
        // Discarding the caller's upload identity does not confer access to the existing artifact.
        assert_eq!(
            client
                .put_file(
                    target.clone(),
                    "training",
                    &source,
                    "created.bin",
                    TransferOptions {
                        resume: false,
                        ..Default::default()
                    }
                )
                .await
                .unwrap_err()
                .code,
            "CONFLICT"
        );
        std::fs::write(&source, b"changed").unwrap();
        assert_eq!(
            client
                .put_file(
                    target,
                    "training",
                    &source,
                    "created.bin",
                    TransferOptions::default()
                )
                .await
                .unwrap_err()
                .code,
            "CONFLICT"
        );
        assert_eq!(
            std::fs::read(export.path().join("existing.bin")).unwrap(),
            b"original"
        );
        assert_eq!(
            std::fs::read(export.path().join("created.bin")).unwrap(),
            b"original"
        );
        client.endpoint.close().await;
    })
    .await
    .expect("SDK upload conflict test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn owner_jobs_report_only_metadata_and_transfer_binary_without_shell_output() {
    tokio::time::timeout(Duration::from_secs(60), async {
    let state = tempfile::tempdir().unwrap();
    let export = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let source = local.path().join("source.bin");
    std::fs::write(&source, [0, 255, 1, 128]).unwrap();
    let (client, target) = create(state.path(), export.path(), SecretKey::generate()).await;
    let destination = local.path().join("must-not-download.bin");
    assert_eq!(
        client.command(&json!({"op":"file_get","host":target.host,"incarnation":target.incarnation,"workspace":"training","remote_path":"private.bin","local_path":destination}))
            .await.unwrap_err(),
        "Unknown app operation"
    );
    assert!(!destination.exists());
    let job = client.command(&json!({"op":"file_put","host":target.host,"incarnation":target.incarnation,"workspace":"training","local_path":source,"remote_path":"binary"})).await.unwrap();
    let id = job["transfer"].as_str().unwrap();
    let complete = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let status = client
                .command(&json!({"op":"file_status","transfer":id}))
                .await
                .unwrap();
            if status["state"] != "running" {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(complete["state"], "complete", "{complete}");
    assert_eq!(complete["file"]["size_bytes"], 4);
    assert!(!complete.to_string().contains("data_b64"));
    assert_eq!(
        std::fs::read(export.path().join("binary")).unwrap(),
        [0, 255, 1, 128]
    );
    client.endpoint.close().await;

    }).await.expect("SDK transfer test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_upload_never_exposes_an_incomplete_destination() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let state = tempfile::tempdir().unwrap();
        let export = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let source = local.path().join("source.bin");
        write_source(&source, 2);
        let (client, target) = create(state.path(), export.path(), SecretKey::generate()).await;
        let error = client
            .put_file(
                target.clone(),
                "training",
                &source,
                "output.bin",
                pause_after(wispnet_files::MAX_CHUNK as u64),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "CANCELLED");
        assert!(!export.path().join("output.bin").exists());
        let file = client
            .put_file(
                target,
                "training",
                &source,
                "output.bin",
                TransferOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            wispnet_files::hash_file(&export.path().join("output.bin")).unwrap(),
            file.sha256
        );
        client.endpoint.close().await;
    })
    .await
    .expect("SDK cancelled upload test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sdk_uses_authenticated_remote_quic_for_binary_upload() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let host_state = tempfile::tempdir().unwrap();
        let client_state = tempfile::tempdir().unwrap();
        let export = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let source = local.path().join("source.bin");
        write_source(&source, 2);
        let (host, client, target) = remote(
            host_state.path(),
            client_state.path(),
            export.path(),
            SecretKey::generate(),
            SecretKey::generate(),
        )
        .await;
        let uploaded = client
            .put_file(
                target.clone(),
                "training",
                &source,
                "job/native.bin",
                TransferOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(uploaded.sha256, wispnet_files::hash_file(&source).unwrap());
        assert_eq!(
            wispnet_files::hash_file(&source).unwrap(),
            wispnet_files::hash_file(&export.path().join("job/native.bin")).unwrap()
        );
        let authority = Authority::open(host_state.path(), target.host).unwrap();
        let snapshot = authority.snapshot().await.unwrap();
        let (mut store, owner) = host::owner_store(host_state.path(), target.host).unwrap();
        store
            .host_mutate(
                target.host,
                &owner,
                Mutation::Files {
                    expected_revision: snapshot.revision,
                    allow: vec![],
                    exports: snapshot.file_exports,
                },
            )
            .unwrap();
        assert_eq!(
            client
                .put_file(
                    target,
                    "training",
                    &source,
                    "denied",
                    TransferOptions::default()
                )
                .await
                .unwrap_err()
                .code,
            "PERMISSION_DENIED"
        );
        client.endpoint.close().await;
        host.shutdown().await;
    })
    .await
    .expect("Native SDK transfer timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "1 GiB SDK helper acceptance including disk hashes and resumed upload"]
async fn sdk_large_file() {
    let state = tempfile::tempdir().unwrap();
    let client_state = tempfile::tempdir().unwrap();
    let export = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let key = SecretKey::generate();
    let client_key = SecretKey::generate();
    let source = local.path().join("source.bin");
    write_source(&source, 256);
    let (host, client, target) = remote(
        state.path(),
        client_state.path(),
        export.path(),
        key.clone(),
        client_key.clone(),
    )
    .await;
    assert_eq!(
        client
            .put_file(
                target,
                "training",
                &source,
                "artifact.bin",
                pause_after(32 * 1024 * 1024)
            )
            .await
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert!(!export.path().join("artifact.bin").exists());
    client.endpoint.close().await;
    drop(client);
    host.shutdown().await;
    let (host, client, target) = remote(
        state.path(),
        client_state.path(),
        export.path(),
        key,
        client_key,
    )
    .await;
    let resumed_offset = Arc::new(Mutex::new(None));
    let observed_offset = resumed_offset.clone();
    let started = std::time::Instant::now();
    let uploaded = client
        .put_file(
            target,
            "training",
            &source,
            "artifact.bin",
            TransferOptions {
                progress: Some(Arc::new(move |progress| {
                    if progress.phase == "transferring" {
                        observed_offset
                            .lock()
                            .unwrap()
                            .get_or_insert(progress.verified_bytes);
                    }
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let resumed_offset = resumed_offset.lock().unwrap().unwrap();
    assert!(resumed_offset > 0);
    eprintln!(
        "SDK 1 GiB resumed upload total {:?}, {:.3} Gbps",
        started.elapsed(),
        (uploaded.size_bytes - resumed_offset) as f64 * 8.0 / started.elapsed().as_secs_f64() / 1e9
    );
    assert_eq!(
        wispnet_files::hash_file(&export.path().join("artifact.bin")).unwrap(),
        uploaded.sha256
    );
    client.endpoint.close().await;
    host.shutdown().await;
}
