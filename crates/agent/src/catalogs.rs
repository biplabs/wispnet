//! Durable catalog baselines, separate from rebuildable UI snapshots and authority.
use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};
use wipsnet_core::{
    host::Catalog,
    identity::{EndpointId, SecretKey},
};

const LIMIT: usize = wipsnet_core::host::MAX_RESPONSE + 4096;
const FAILURE: &str = "Catalog baseline unavailable or corrupt";

fn protected(file: &fs::File) -> Result<(), String> {
    let m = file.metadata().map_err(|_| FAILURE)?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || m.nlink() != 1
    {
        return Err(FAILURE.into());
    }
    Ok(())
}
fn locked(state: &Path, host: EndpointId) -> Result<(PathBuf, fs::File), String> {
    if !state.is_absolute()
        || state
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(FAILURE.into());
    }
    let directory = state.join("catalog-baselines");
    for path in [state, &directory] {
        if path == directory && !path.exists() {
            match fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(FAILURE.into()),
            }
        }
        let m = fs::symlink_metadata(path).map_err(|_| FAILURE)?;
        if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
            return Err(FAILURE.into());
        }
    }
    fs::File::open(state)
        .and_then(|f| f.sync_all())
        .map_err(|_| FAILURE)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(format!("{host}.lock")))
        .map_err(|_| FAILURE)?;
    protected(&lock)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("Catalog refresh busy; retry this host".into());
    }
    Ok((directory.join(format!("{host}.json")), lock))
}
fn read(path: &Path, host: EndpointId) -> Result<Option<(String, Option<Catalog>)>, String> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(FAILURE.into()),
    };
    protected(&file)?;
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| FAILURE)?;
    if bytes.len() > LIMIT {
        return Err(FAILURE.into());
    }
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| FAILURE)?;
    if v.as_object().is_none_or(|o| {
        o.len() != 3
            || !o.contains_key("catalog")
            || !o.contains_key("epoch")
            || !o.contains_key("format")
    }) || v["format"] != 1
    {
        return Err(FAILURE.into());
    }
    let epoch = v["epoch"]
        .as_str()
        .filter(|e| wipsnet_core::enrollment::valid_code(e))
        .ok_or(FAILURE)?
        .to_owned();
    let catalog = if v["catalog"].is_null() {
        None
    } else {
        let c: Catalog = serde_json::from_value(v["catalog"].clone()).map_err(|_| FAILURE)?;
        if !c.valid(host) {
            return Err(FAILURE.into());
        }
        Some(c)
    };
    Ok(Some((epoch, catalog)))
}
fn write(path: &Path, epoch: &str, catalog: Option<&Catalog>) -> Result<(), String> {
    let bytes =
        serde_json::to_vec(&serde_json::json!({"format":1,"epoch":epoch,"catalog":catalog}))
            .map_err(|_| FAILURE)?;
    if bytes.len() > LIMIT {
        return Err(FAILURE.into());
    }
    let directory = path.parent().ok_or(FAILURE)?;
    let mut temp = tempfile::NamedTempFile::new_in(directory).map_err(|_| FAILURE)?;
    temp.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| FAILURE)?;
    temp.write_all(&bytes).map_err(|_| FAILURE)?;
    temp.as_file().sync_all().map_err(|_| FAILURE)?;
    temp.persist(path).map_err(|_| FAILURE)?;
    fs::File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|_| FAILURE)?;
    Ok(())
}
fn epoch() -> String {
    SecretKey::generate()
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
/// Capture pairing epoch before requesting a catalog; late replies cannot undo re-pairing.
pub fn begin(state: &Path, host: EndpointId) -> Result<String, String> {
    let (path, _guard) = locked(state, host)?;
    if let Some((epoch, _)) = read(&path, host)? {
        return Ok(epoch);
    }
    let epoch = epoch();
    write(&path, &epoch, None)?;
    Ok(epoch)
}
pub fn accept(
    state: &Path,
    host: EndpointId,
    epoch: &str,
    catalog: &Catalog,
) -> Result<(), String> {
    if !catalog.valid(host) {
        return Err("Invalid or incomplete catalog".into());
    }
    let (path, _guard) = locked(state, host)?;
    let (current, previous) = read(&path, host)?.ok_or(FAILURE)?;
    if current != epoch {
        return Err("Pairing changed during catalog refresh".into());
    }
    if previous.as_ref().is_some_and(|p| !catalog.follows(p)) {
        return Err("Catalog rollback, changed generation or inconsistent snapshot".into());
    }
    if previous.as_ref() != Some(catalog) {
        write(&path, epoch, Some(catalog))?;
    }
    Ok(())
}
/// Only after explicit authenticated target re-pairing. Display-cache deletion never calls this.
pub fn reset(state: &Path, host: EndpointId) -> Result<(), String> {
    let (path, _guard) = locked(state, host)?;
    read(&path, host)?;
    write(&path, &epoch(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsafe_baselines_and_lock_files_fail_closed_without_touching_their_targets() {
        let state = tempfile::tempdir().unwrap();
        fs::set_permissions(state.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let host = SecretKey::generate().public();
        let epoch = begin(state.path(), host).unwrap();
        let path = state
            .path()
            .join("catalog-baselines")
            .join(format!("{host}.json"));
        let original = fs::read(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(begin(state.path(), host).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let outside = state.path().join("outside");
        fs::hard_link(&path, &outside).unwrap();
        assert!(begin(state.path(), host).is_err());
        fs::remove_file(&outside).unwrap();
        fs::rename(&path, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(begin(state.path(), host).is_err());
        assert!(reset(state.path(), host).is_err());
        assert_eq!(fs::read(&outside).unwrap(), original);
        fs::remove_file(&path).unwrap();
        fs::rename(&outside, &path).unwrap();
        let lock = path.with_extension("lock");
        fs::remove_file(&lock).unwrap();
        std::os::unix::fs::symlink(&path, &lock).unwrap();
        assert!(begin(state.path(), host).is_err());
        fs::remove_file(&lock).unwrap();
        assert_eq!(begin(state.path(), host).unwrap(), epoch);
        let wrong = SecretKey::generate().public();
        let record = serde_json::json!({"format":1,"epoch":epoch,"extra":null});
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(begin(state.path(), host).is_err());
        assert!(begin(state.path(), wrong).is_ok());
    }
}
