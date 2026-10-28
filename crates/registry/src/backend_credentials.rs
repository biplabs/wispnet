//! Host-only HTTP backend credentials. Never include this state in catalogs or snapshots.
//!
//! Callers must establish owner authority before mutations. Reading a credential is
//! an internal hosting operation, not an owner API response. This file is protected
//! by the same OS-user boundary as the owning node; it is not encrypted at rest.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::SocketAddr,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::Mutex,
};
use wispnet_core::{enrollment::valid_code, identity::EndpointId, protocol::valid_service_id};
use zeroize::{Zeroize, Zeroizing};

const FILE: &str = "backend-credentials.json";
const LIMIT: u64 = 256 * 1024;
static WRITES: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    PassThrough,
    None,
    Bearer,
}

/// The complete safe-to-display result. It deliberately contains no token bytes.
#[derive(Clone, Serialize, Debug)]
pub struct Status {
    pub mode: Mode,
    pub configured: bool,
    pub generation: String,
}

/// Internal runtime value. Deliberately neither Debug nor Serialize.
pub struct Credential {
    pub mode: Mode,
    pub generation: String,
    secret: Zeroizing<String>,
}
impl Credential {
    pub fn bearer(&self) -> Option<&str> {
        (self.mode == Mode::Bearer).then_some(self.secret.as_str())
    }
    fn status(&self) -> Status {
        Status {
            mode: self.mode,
            configured: self.mode == Mode::Bearer,
            generation: self.generation.clone(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    incarnation: String,
    backend: SocketAddr,
    mode: Mode,
    generation: String,
    secret: String,
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u8,
    host: EndpointId,
    services: BTreeMap<String, Entry>,
}

fn protected_directory(state: &Path) -> Result<(), String> {
    let metadata =
        std::fs::symlink_metadata(state).map_err(|_| "Backend credential state unavailable")?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err("Backend credential directory must be owner-only".into());
    }
    Ok(())
}
fn read(state: &Path, host: EndpointId) -> Result<Saved, String> {
    protected_directory(state)?;
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(state.join(FILE))
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Saved {
                version: 1,
                host,
                services: BTreeMap::new(),
            });
        }
        Err(_) => return Err("Backend credential state unavailable".into()),
    };
    let metadata = file
        .metadata()
        .map_err(|_| "Backend credential state unavailable")?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
        || metadata.len() > LIMIT
    {
        return Err("Unsafe backend credential state".into());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Backend credential state unavailable")?;
    let saved: Saved =
        serde_json::from_slice(&bytes).map_err(|_| "Backend credential state invalid")?;
    if saved.version != 1 || saved.host != host || saved.services.len() > 128 {
        return Err("Backend credential identity or version mismatch".into());
    }
    for (service, entry) in &saved.services {
        validate(service, &entry.incarnation, entry.backend)?;
        if !valid_code(&entry.generation) || !valid_secret(entry.mode, &entry.secret) {
            return Err("Backend credential state invalid".into());
        }
    }
    Ok(saved)
}
fn write(state: &Path, saved: &Saved) -> Result<(), String> {
    let bytes =
        Zeroizing::new(serde_json::to_vec(saved).map_err(|_| "Cannot save backend credentials")?);
    if bytes.len() as u64 > LIMIT {
        return Err("Backend credential state is full".into());
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(state).map_err(|_| "Cannot reserve backend credentials")?;
    temporary
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|_| "Cannot protect backend credentials")?;
    temporary
        .write_all(&bytes)
        .map_err(|_| "Cannot save backend credentials")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| "Cannot sync backend credentials")?;
    temporary
        .persist(state.join(FILE))
        .map_err(|_| "Cannot install backend credentials")?;
    std::fs::File::open(state)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "Cannot sync backend credential directory")?;
    Ok(())
}
fn validate(service: &str, incarnation: &str, backend: SocketAddr) -> Result<(), String> {
    if !valid_service_id(service)
        || !valid_code(incarnation)
        || !backend.ip().is_loopback()
        || backend.port() == 0
    {
        return Err("Invalid backend credential binding".into());
    }
    Ok(())
}
fn valid_secret(mode: Mode, secret: &str) -> bool {
    if mode != Mode::Bearer {
        return secret.is_empty();
    }
    !secret.is_empty() && secret.len() <= 8192 && secret.bytes().all(|b| (0x21..=0x7e).contains(&b))
}
fn generation() -> String {
    wispnet_core::identity::SecretKey::generate()
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn load(
    state: &Path,
    host: EndpointId,
    service: &str,
    incarnation: &str,
    backend: SocketAddr,
) -> Result<Credential, String> {
    validate(service, incarnation, backend)?;
    let saved = read(state, host)?;
    // Host-authorized renames preserve the incarnation and binding. A recreated
    // service under the old name cannot inherit the old incarnation's secret.
    let entry = saved
        .services
        .values()
        .find(|entry| entry.incarnation == incarnation && entry.backend == backend)
        .or_else(|| saved.services.get(service));
    let Some(entry) = entry else {
        return Ok(Credential {
            mode: Mode::PassThrough,
            generation: "unmanaged".into(),
            secret: Zeroizing::new(String::new()),
        });
    };
    if entry.incarnation != incarnation || entry.backend != backend {
        return Err("Backend credential binding changed; configure it again".into());
    }
    Ok(Credential {
        mode: entry.mode,
        generation: entry.generation.clone(),
        secret: Zeroizing::new(entry.secret.clone()),
    })
}

pub fn status(
    state: &Path,
    host: EndpointId,
    service: &str,
    incarnation: &str,
    backend: SocketAddr,
) -> Result<Status, String> {
    Ok(load(state, host, service, incarnation, backend)?.status())
}

#[allow(clippy::too_many_arguments)]
pub fn set(
    state: &Path,
    host: EndpointId,
    service: &str,
    incarnation: &str,
    backend: SocketAddr,
    mode: Mode,
    secret: Option<&str>,
) -> Result<Status, String> {
    validate(service, incarnation, backend)?;
    let secret = secret.unwrap_or("");
    if !valid_secret(mode, secret) {
        return Err("Invalid backend bearer credential".into());
    }
    let _lock = WRITES
        .lock()
        .map_err(|_| "Backend credential state unavailable")?;
    let mut saved = read(state, host)?;
    saved
        .services
        .retain(|_, entry| entry.incarnation != incarnation);
    if saved.services.len() >= 128 && !saved.services.contains_key(service) {
        return Err("Too many backend credential bindings".into());
    }
    let generation = generation();
    saved.services.insert(
        service.into(),
        Entry {
            incarnation: incarnation.into(),
            backend,
            mode,
            generation: generation.clone(),
            secret: secret.into(),
        },
    );
    write(state, &saved)?;
    Ok(Status {
        mode,
        configured: mode == Mode::Bearer,
        generation,
    })
}

/// Remove the saved secret while retaining an explicit managed, no-credential
/// mode. Returning to caller-supplied credentials requires set(PassThrough).
pub fn remove(
    state: &Path,
    host: EndpointId,
    service: &str,
    incarnation: &str,
    backend: SocketAddr,
) -> Result<Status, String> {
    set(state, host, service, incarnation, backend, Mode::None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, EndpointId, String, SocketAddr) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        (
            directory,
            wispnet_core::identity::SecretKey::generate().public(),
            generation(),
            "127.0.0.1:8300".parse().unwrap(),
        )
    }
    #[test]
    fn credentials_are_bound_protected_and_status_is_redacted() {
        let (directory, host, incarnation, backend) = fixture();
        let before = status(directory.path(), host, "gike", &incarnation, backend).unwrap();
        assert_eq!(before.mode, Mode::PassThrough);
        let saved = set(
            directory.path(),
            host,
            "gike",
            &incarnation,
            backend,
            Mode::Bearer,
            Some("fixture-secret"),
        )
        .unwrap();
        assert!(
            !serde_json::to_string(&saved)
                .unwrap()
                .contains("fixture-secret")
        );
        assert_eq!(
            load(directory.path(), host, "gike", &incarnation, backend)
                .unwrap()
                .bearer(),
            Some("fixture-secret")
        );
        assert!(load(directory.path(), host, "gike", &generation(), backend).is_err());
        assert!(
            load(
                directory.path(),
                host,
                "gike",
                &incarnation,
                "127.0.0.1:8301".parse().unwrap()
            )
            .is_err()
        );
        assert!(
            load(
                directory.path(),
                wispnet_core::identity::SecretKey::generate().public(),
                "gike",
                &incarnation,
                backend
            )
            .is_err()
        );
        let renamed = load(directory.path(), host, "renamed", &incarnation, backend).unwrap();
        assert_eq!(renamed.bearer(), Some("fixture-secret"));
        let removed = remove(directory.path(), host, "renamed", &incarnation, backend).unwrap();
        assert_eq!(removed.mode, Mode::None);
        assert!(!removed.configured);
        assert_ne!(removed.generation, saved.generation);
    }
    #[test]
    fn unsafe_state_and_header_injection_are_rejected() {
        let (directory, host, incarnation, backend) = fixture();
        assert!(
            set(
                directory.path(),
                host,
                "gike",
                &incarnation,
                backend,
                Mode::Bearer,
                Some("bad\r\nheader")
            )
            .is_err()
        );
        set(
            directory.path(),
            host,
            "gike",
            &incarnation,
            backend,
            Mode::None,
            None,
        )
        .unwrap();
        std::fs::set_permissions(
            directory.path().join(FILE),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(load(directory.path(), host, "gike", &incarnation, backend).is_err());
        std::fs::remove_file(directory.path().join(FILE)).unwrap();
        let target = directory.path().join("target");
        std::fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, directory.path().join(FILE)).unwrap();
        assert!(load(directory.path(), host, "gike", &incarnation, backend).is_err());
    }
}
