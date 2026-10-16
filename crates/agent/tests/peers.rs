use std::os::unix::fs::{PermissionsExt, symlink};
use wipsnet_agent::peers;
use wipsnet_core::{identity::Identity, pairing::Bootstrap};
fn record() -> Bootstrap {
    Bootstrap {
        version: 1,
        host: Identity::generate().endpoint_id(),
        relay_url: "https://relay.example/".into(),
        direct_hint: None,
    }
}
#[test]
fn persisted_records_are_independent_bounded_and_fail_closed() {
    let state = tempfile::tempdir().unwrap();
    std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let a = record();
    let b = record();
    assert!(peers::load(state.path(), a.host).unwrap().is_none());
    peers::save(state.path(), &a).unwrap();
    peers::save(state.path(), &b).unwrap();
    let path = state
        .path()
        .join("peer-pins")
        .join(format!("{}.json", a.host));
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        peers::load(state.path(), a.host).unwrap().unwrap().host,
        a.host
    );
    assert_eq!(
        peers::load(state.path(), b.host).unwrap().unwrap().host,
        b.host
    );
    let mut changed = record();
    changed.host = a.host;
    changed.relay_url = "https://replacement.example/".into();
    changed.direct_hint = Some("127.0.0.1:1234".parse().unwrap());
    peers::save(state.path(), &changed).unwrap();
    let loaded = peers::load(state.path(), a.host).unwrap().unwrap();
    assert_eq!(loaded.relay_url, changed.relay_url);
    let fallback = peers::transport(None, Some(&loaded)).unwrap();
    assert_eq!(fallback.relay_url, Some(changed.relay_url));
    let mut explicit = fallback.clone();
    explicit.relay_url = None;
    assert!(
        peers::transport(Some(&explicit), Some(&loaded))
            .unwrap()
            .relay_url
            .is_none()
    );
    std::fs::write(&path, serde_json::to_vec(&b).unwrap()).unwrap();
    assert!(peers::load(state.path(), a.host).is_err());
    assert!(peers::save(state.path(), &a).is_err());
    std::fs::write(&path, [b'x'; 4097]).unwrap();
    assert!(peers::load(state.path(), a.host).is_err());
    std::fs::write(&path, serde_json::to_vec(&a).unwrap()).unwrap();
    let link = state.path().join("hardlink");
    std::fs::hard_link(&path, &link).unwrap();
    assert!(peers::load(state.path(), a.host).is_err());
    std::fs::remove_file(&link).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert!(peers::load(state.path(), a.host).is_err());
    std::fs::remove_file(&path).unwrap();
    symlink(&link, &path).unwrap();
    assert!(peers::load(state.path(), a.host).is_err());
    assert!(peers::save(state.path(), &a).is_err());
}
#[test]
fn private_directory_and_lock_are_required() {
    use std::os::fd::AsRawFd;
    let state = tempfile::tempdir().unwrap();
    std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let record = record();
    let dir = state.path().join("peer-pins");
    symlink(state.path(), &dir).unwrap();
    assert!(peers::save(state.path(), &record).is_err());
    std::fs::remove_file(&dir).unwrap();
    peers::save(state.path(), &record).unwrap();
    let lock = std::fs::File::open(dir.join(".lock")).unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    assert!(peers::save(state.path(), &record).is_err());
    drop(lock);
    peers::save(state.path(), &record).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(peers::load(state.path(), record.host).is_err());
}
