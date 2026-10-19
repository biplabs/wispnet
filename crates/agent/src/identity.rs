//! Native Unix identity storage (Linux and Android shell harness). Parent directories must be controlled by this UID.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use wispnet_core::identity::{EndpointId, Identity, IdentityError, IdentityStore, SecretKey};
use zeroize::Zeroizing;

const KEY_FILE: &str = "identity.key";
const MAGIC: &[u8; 8] = b"WIPSKEY1";
const RECORD_LEN: usize = 72;

pub struct FileIdentityStore {
    directory: PathBuf,
}

impl FileIdentityStore {
    pub fn new(directory: &Path) -> Result<Self, IdentityError> {
        let metadata = fs::symlink_metadata(directory)
            .map_err(|_| IdentityError("identity state directory is unavailable"))?;
        // SAFETY: geteuid has no arguments and no memory safety preconditions.
        let uid = unsafe { libc::geteuid() };
        if !directory.is_absolute()
            || !metadata.is_dir()
            || metadata.permissions().mode() & 0o777 != 0o700
            || metadata.uid() != uid
        {
            return Err(IdentityError(
                "identity state directory must be owned by this user, mode 700, and not a symlink",
            ));
        }
        Ok(Self {
            directory: directory.into(),
        })
    }

    fn private_file(&self, name: &str, create: bool) -> Result<File, IdentityError> {
        let file = OpenOptions::new()
            .read(true)
            .write(create)
            .create(create)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.directory.join(name))
            .map_err(|_| {
                IdentityError("identity file unavailable; check ownership/permissions and symlinks")
            })?;
        let metadata = file
            .metadata()
            .map_err(|_| IdentityError("cannot inspect identity file"))?;
        // SAFETY: geteuid has no memory safety preconditions.
        let uid = unsafe { libc::geteuid() };
        if !metadata.is_file()
            || metadata.uid() != uid
            || metadata.permissions().mode() & 0o777 != 0o600
            || metadata.nlink() != 1
        {
            return Err(IdentityError(
                "identity files must be regular, singly linked, owned by this user, and mode 600",
            ));
        }
        Ok(file)
    }

    fn lock(&self, name: &str) -> Result<File, IdentityError> {
        let file = self.private_file(name, true)?;
        #[cfg(target_os = "android")]
        let locked = {
            use std::os::fd::AsRawFd;
            // SAFETY: fd is live for this call; flock has no pointer arguments.
            // Rust std file locking is unsupported on Android. The lease remains
            // exclusive until File closes, including on cancellation/process exit.
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
        };
        #[cfg(not(target_os = "android"))]
        let locked = file.try_lock().is_ok();
        if !locked {
            return Err(IdentityError(
                "identity is in use or locking is unavailable; stop the agent or retry the operation",
            ));
        }
        Ok(file)
    }

    /// Hold this lease for the entire agent lifetime to block reset while in use.
    pub fn lease(&self) -> Result<File, IdentityError> {
        self.lock(".identity-use.lock")
    }

    fn read_key(&self) -> Result<Identity, IdentityError> {
        let file = self.private_file(KEY_FILE, false)?;
        let mut record = Zeroizing::new(Vec::new());
        file.take((RECORD_LEN + 1) as u64)
            .read_to_end(&mut record)
            .map_err(|_| {
                IdentityError("cannot read identity; restore the original state, do not regenerate")
            })?;
        if record.len() != RECORD_LEN || &record[..8] != MAGIC {
            return Err(IdentityError(
                "identity is corrupt or unsupported; restore the original state, do not regenerate",
            ));
        }
        let mut seed = Zeroizing::new([0u8; 32]);
        seed.copy_from_slice(&record[8..40]);
        let identity = Identity::from_secret_key(SecretKey::from_bytes(&seed));
        if identity.endpoint_id().as_bytes() != &record[40..] {
            return Err(IdentityError(
                "identity integrity check failed; restore the original state, do not regenerate",
            ));
        }
        Ok(identity)
    }

    fn sync_directory(&self) -> Result<(), IdentityError> {
        File::open(&self.directory)
            .and_then(|f| f.sync_all())
            .map_err(|_| {
                IdentityError(
                    "cannot sync identity directory; verify stored identity before retrying",
                )
            })
    }
}

impl IdentityStore for FileIdentityStore {
    fn load(&self) -> Result<Identity, IdentityError> {
        let _lock = self.lock(".identity.lock")?;
        if fs::symlink_metadata(self.directory.join(KEY_FILE))
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            return Err(IdentityError(
                "identity missing; run identity-init explicitly for a new installation, or restore the original key",
            ));
        }
        self.read_key()
    }

    fn initialize(&self) -> Result<Identity, IdentityError> {
        let _lock = self.lock(".identity.lock")?;
        match fs::symlink_metadata(self.directory.join(KEY_FILE)) {
            Ok(_) => return self.read_key(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(IdentityError(
                    "cannot inspect existing identity; refusing replacement",
                ));
            }
        }
        let identity = Identity::generate();
        let seed = Zeroizing::new(identity.secret_key().to_bytes());
        let mut record = Zeroizing::new(Vec::with_capacity(RECORD_LEN));
        record.extend_from_slice(MAGIC);
        record.extend_from_slice(seed.as_ref());
        record.extend_from_slice(identity.endpoint_id().as_bytes());
        let mut temporary = tempfile::Builder::new()
            .prefix(".identity-write-")
            .tempfile_in(&self.directory)
            .map_err(|_| IdentityError("cannot create private identity temporary file"))?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| IdentityError("cannot protect identity temporary file"))?;
        temporary
            .write_all(&record)
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|_| {
                IdentityError("cannot write/sync identity; no new identity was committed")
            })?;
        temporary
            .persist_noclobber(self.directory.join(KEY_FILE))
            .map_err(|_| {
                IdentityError(
                    "cannot atomically commit identity; verify stored identity before retrying",
                )
            })?;
        self.sync_directory()?;
        Ok(identity)
    }

    fn reset(&self, expected: EndpointId) -> Result<(), IdentityError> {
        let _lease = self.lease()?;
        let _lock = self.lock(".identity.lock")?;
        if self.read_key()?.endpoint_id() != expected {
            return Err(IdentityError(
                "reset endpoint ID does not match; identity was not changed",
            ));
        }
        fs::remove_file(self.directory.join(KEY_FILE))
            .map_err(|_| IdentityError("cannot remove identity; no replacement generated"))?;
        self.sync_directory()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> (tempfile::TempDir, FileIdentityStore) {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = FileIdentityStore::new(directory.path()).unwrap();
        (directory, store)
    }
    #[test]
    fn persists_independent_identities_and_private_permissions() {
        let (dir, store) = store();
        assert!(store.load().is_err());
        let id = store.initialize().unwrap().endpoint_id();
        assert_eq!(store.initialize().unwrap().endpoint_id(), id);
        assert_eq!(
            FileIdentityStore::new(dir.path())
                .unwrap()
                .load()
                .unwrap()
                .endpoint_id(),
            id
        );
        let (_other_dir, other) = self::store();
        assert_ne!(other.initialize().unwrap().endpoint_id(), id);
        assert_eq!(
            fs::metadata(dir.path().join(KEY_FILE)).unwrap().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn corruption_and_insecure_files_never_rotate() {
        let (dir, store) = store();
        store.initialize().unwrap();
        let path = dir.path().join(KEY_FILE);
        let original = fs::read(&path).unwrap();
        for damaged in [b"PRIVATE-SENTINEL".to_vec(), {
            let mut bytes = original.clone();
            bytes[9] ^= 1;
            bytes
        }] {
            fs::write(&path, &damaged).unwrap();
            for result in [store.load(), store.initialize()] {
                let error = result.err().unwrap().to_string();
                assert!(!error.contains("PRIVATE-SENTINEL"));
            }
            assert!(
                fs::read(&path).unwrap() == damaged,
                "damaged identity was changed"
            );
        }
        fs::write(&path, &original).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(store.load().is_err());
        assert!(store.initialize().is_err());
        assert!(
            fs::read(&path).unwrap() == original,
            "insecure identity was changed"
        );
    }
    #[test]
    fn interrupted_write_is_not_loaded_or_allowed_to_replace_committed_identity() {
        let (dir, store) = store();
        let pending = dir.path().join(".identity-write-interrupted");
        fs::write(&pending, b"partial").unwrap();
        assert!(store.load().is_err());
        let id = store.initialize().unwrap().endpoint_id();
        fs::write(&pending, b"other partial").unwrap();
        assert_eq!(store.initialize().unwrap().endpoint_id(), id);
        assert_eq!(store.load().unwrap().endpoint_id(), id);
        fs::write(dir.path().join(KEY_FILE), b"partial committed record").unwrap();
        assert!(store.initialize().is_err());
    }
    #[test]
    fn reset_is_explicit_guarded_and_cannot_run_while_agent_uses_key() {
        let (_dir, store) = store();
        let id = store.initialize().unwrap().endpoint_id();
        assert!(store.reset(Identity::generate().endpoint_id()).is_err());
        assert_eq!(store.load().unwrap().endpoint_id(), id);
        let lease = store.lease().unwrap();
        assert!(store.reset(id).is_err());
        drop(lease);
        store.reset(id).unwrap();
        assert!(store.load().is_err());
        assert_ne!(store.initialize().unwrap().endpoint_id(), id);
    }
    #[test]
    fn symlinks_and_concurrent_writers_fail_closed() {
        let (dir, store) = store();
        let lock = store.lock(".identity.lock").unwrap();
        assert!(store.initialize().is_err());
        drop(lock);
        let other = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(other.path(), dir.path().join(KEY_FILE)).unwrap();
        assert!(store.initialize().is_err());
        assert!(store.load().is_err());
        assert_eq!(fs::metadata(other.path()).unwrap().len(), 0);
    }

    #[test]
    fn interrupted_link_commit_fails_closed_until_orphan_is_removed() {
        let (dir, store) = store();
        let id = store.initialize().unwrap().endpoint_id();
        // persist_noclobber may use link/unlink. Model a crash between them.
        let orphan = dir.path().join(".identity-write-linked");
        fs::hard_link(dir.path().join(KEY_FILE), &orphan).unwrap();
        assert!(store.load().is_err());
        assert!(store.initialize().is_err());
        fs::remove_file(orphan).unwrap();
        assert_eq!(store.load().unwrap().endpoint_id(), id);
    }
}
