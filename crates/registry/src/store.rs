//! Local persistence boundary. Callers must authenticate before invoking mutations.
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;
use wispnet_core::{identity::EndpointId, protocol::valid_service_id};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Storage,
    UnsupportedSchema,
    InvalidInput,
    Conflict,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Storage => "registry storage failed",
            Self::UnsupportedSchema => "unsupported registry schema",
            Self::InvalidInput => "invalid registry input",
            Self::Conflict => "registry revision conflict",
        })
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Service {
    pub id: String,
    pub host: EndpointId,
    pub grants: Vec<EndpointId>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub revision: u64,
    pub devices: Vec<EndpointId>,
    pub services: Vec<Service>,
}
/// SQLite handle for a database in a private, caller-managed state directory.
/// This is not an authenticated network API.
pub struct Store(pub(crate) Connection);
impl Store {
    pub fn open(path: &Path) -> Result<Self, Error> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let parent = path.parent().ok_or(Error::InvalidInput)?;
        let parent_metadata = std::fs::symlink_metadata(parent).map_err(|_| Error::Storage)?;
        if !parent_metadata.is_dir()
            || parent_metadata.mode() & 0o777 != 0o700
            || parent_metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(Error::Storage);
        }
        // Reserve with private permissions before SQLite opens the file. Refuse
        // links/shared files; callers keep the parent directory private as well.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| Error::Storage)?;
        let metadata = file.metadata().map_err(|_| Error::Storage)?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.mode() & 0o777 != 0o600
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(Error::Storage);
        }
        let mut db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(3))?;
        db.pragma_update(None, "foreign_keys", true)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        match version {
            0 => tx.execute_batch("CREATE TABLE revision (id INTEGER PRIMARY KEY CHECK(id=1), value INTEGER NOT NULL CHECK(value>=0));
                INSERT INTO revision VALUES(1,0);
                CREATE TABLE devices (id TEXT PRIMARY KEY);
                CREATE TABLE services (id TEXT PRIMARY KEY, host TEXT NOT NULL REFERENCES devices(id));
                CREATE TABLE grants (service TEXT NOT NULL REFERENCES services(id) ON DELETE CASCADE, device TEXT NOT NULL REFERENCES devices(id), PRIMARY KEY(service,device));
                PRAGMA user_version=1;")?,
            1..=15 => {},
            _ => return Err(Error::UnsupportedSchema),
        }
        if version < 2 {
            tx.execute_batch("CREATE TABLE controller (id INTEGER PRIMARY KEY CHECK(id=1), mesh TEXT NOT NULL, owner_hash TEXT NOT NULL);
                CREATE TABLE device_tokens (device TEXT PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE, hash TEXT NOT NULL UNIQUE);
                CREATE TABLE routes (service TEXT PRIMARY KEY REFERENCES services(id) ON DELETE CASCADE, address TEXT NOT NULL);
                PRAGMA user_version=2;")?;
        }
        if version < 3 {
            tx.execute_batch("CREATE TABLE device_records(device TEXT PRIMARY KEY, name TEXT NOT NULL, state TEXT NOT NULL, enrolled_at INTEGER NOT NULL, revoked_at INTEGER);
                INSERT INTO device_records SELECT id,'legacy','enrolled',0,NULL FROM devices;
                CREATE TABLE invitations(id TEXT PRIMARY KEY, hash TEXT NOT NULL UNIQUE, origin TEXT NOT NULL, expires INTEGER NOT NULL, state TEXT NOT NULL, device TEXT, name TEXT, nonce TEXT, challenge_expires INTEGER, purpose TEXT, window INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0);
                PRAGMA user_version=3;")?;
        }
        if version < 4 {
            tx.execute_batch("CREATE TABLE host_authority(id INTEGER PRIMARY KEY CHECK(id=1), endpoint TEXT NOT NULL, generation TEXT NOT NULL, owner_hash TEXT NOT NULL);
                CREATE TABLE host_clients(id TEXT PRIMARY KEY,name TEXT NOT NULL);
                CREATE TABLE host_services(id TEXT PRIMARY KEY,backend TEXT NOT NULL);
                CREATE TABLE host_grants(service TEXT NOT NULL REFERENCES host_services(id) ON DELETE CASCADE,client TEXT NOT NULL REFERENCES host_clients(id) ON DELETE CASCADE,PRIMARY KEY(service,client));
                PRAGMA user_version=4;")?;
        }
        if version < 5 {
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS host_invitations (
            id TEXT PRIMARY KEY, hash TEXT NOT NULL, expires INTEGER NOT NULL,
            state TEXT NOT NULL, peer TEXT, name TEXT, challenge TEXT,
            window INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0);
            PRAGMA user_version=5;",
            )?;
        }
        if version < 6 {
            tx.execute_batch(
                "CREATE TABLE host_operation_receipts (
                id TEXT PRIMARY KEY CHECK(length(id)=64), payload TEXT NOT NULL CHECK(length(payload)=64),
                state TEXT NOT NULL CHECK(state IN ('applied','rejected')),
                revision INTEGER CHECK(revision>=0), reason TEXT);
                PRAGMA user_version=6;",
            )?;
        }
        if version < 7 {
            tx.execute_batch(
                "ALTER TABLE host_services ADD COLUMN incarnation TEXT NOT NULL DEFAULT '';
                UPDATE host_services SET incarnation=lower(hex(randomblob(32)));
                UPDATE revision SET value=value+1 WHERE EXISTS(SELECT 1 FROM host_services);
                PRAGMA user_version=7;",
            )?;
        }
        if version < 8 {
            tx.execute_batch(
                "ALTER TABLE host_services ADD COLUMN display_name TEXT NOT NULL DEFAULT '';
                ALTER TABLE host_services ADD COLUMN kind TEXT NOT NULL DEFAULT 'http';
                UPDATE host_services SET display_name=id;
                UPDATE revision SET value=value+1 WHERE EXISTS(SELECT 1 FROM host_services);
                PRAGMA user_version=8;",
            )?;
        }
        if version < 9 {
            tx.execute_batch(
                "CREATE TABLE host_pairing_offers (
                invitation TEXT PRIMARY KEY REFERENCES host_invitations(id) ON DELETE CASCADE,
                bootstrap TEXT NOT NULL);
                CREATE TABLE host_pairings (
                peer TEXT PRIMARY KEY REFERENCES host_clients(id) ON DELETE CASCADE,
                bootstrap TEXT NOT NULL);
                PRAGMA user_version=9;",
            )?;
        }
        if version < 10 {
            tx.execute_batch(
                "ALTER TABLE host_pairings ADD COLUMN pairing TEXT NOT NULL DEFAULT '';
                ALTER TABLE host_pairing_offers ADD COLUMN pairing TEXT NOT NULL DEFAULT '';
                CREATE TABLE host_pairing_blocks(peer TEXT PRIMARY KEY, pairing TEXT NOT NULL);
                CREATE TABLE host_legacy_pins(peer TEXT PRIMARY KEY);
                PRAGMA user_version=10;",
            )?;
        }
        if version < 11 {
            // Retire presentation-only names without changing service keys,
            // incarnations, host identity, pairings, or grants. Advance catalog
            // revisions so older caches accept the derived compatibility name.
            tx.execute_batch(
                "UPDATE revision SET value=value+1 WHERE EXISTS(SELECT 1 FROM host_services WHERE display_name!=id);
                ALTER TABLE host_services DROP COLUMN display_name;
                PRAGMA user_version=11;",
            )?;
        }
        if version < 12 {
            if crate::host::ensure_builtin_shell(&tx)? {
                tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
            }
            tx.pragma_update(None, "user_version", 12)?;
        }
        if version < 13 {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS host_file_exports(name TEXT PRIMARY KEY,root TEXT NOT NULL);")?;
            if crate::host::ensure_builtin_files(&tx)? {
                tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
            }
            tx.pragma_update(None, "user_version", 13)?;
        }
        if version < 14 {
            // Older nodes could record Files grants before any folder was
            // configured. Revoke these ineffective grants so choosing a folder
            // later cannot silently activate access that predates that choice.
            let removed = tx.execute(
                "DELETE FROM host_grants WHERE service='files' AND NOT EXISTS(SELECT 1 FROM host_file_exports)",
                [],
            )?;
            if removed > 0 {
                tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
            }
            tx.pragma_update(None, "user_version", 14)?;
        }
        if version < 15 {
            tx.execute_batch("ALTER TABLE host_services ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)); PRAGMA user_version=15;")?;
        }
        tx.commit()?;
        Ok(Self(db))
    }
    /// Atomically replace the directory and policy against a known revision.
    /// Enrollment and owner authentication belong to the API layer, not this method.
    pub fn replace(
        &mut self,
        expected: u64,
        devices: &[EndpointId],
        services: &[Service],
    ) -> Result<u64, Error> {
        if services.iter().map(|s| s.grants.len()).sum::<usize>() > 16384
            || devices.len() > 1024
            || services.len() > 1024
            || expected >= i64::MAX as u64
            || services
                .iter()
                .any(|s| !valid_service_id(&s.id) || s.grants.len() > 1024)
        {
            return Err(Error::InvalidInput);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: i64 =
            tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
        if revision != expected as i64 {
            return Err(Error::Conflict);
        }
        tx.execute_batch("DELETE FROM grants; DELETE FROM services; DELETE FROM devices;")?;
        for device in devices {
            tx.execute("INSERT INTO devices VALUES(?1)", [device.to_string()])?;
        }
        for service in services {
            tx.execute(
                "INSERT INTO services VALUES(?1,?2)",
                params![service.id, service.host.to_string()],
            )?;
            for grant in &service.grants {
                tx.execute(
                    "INSERT INTO grants VALUES(?1,?2)",
                    params![service.id, grant.to_string()],
                )?;
            }
        }
        tx.execute(
            "UPDATE revision SET value=?1 WHERE id=1",
            [(expected + 1) as i64],
        )?;
        tx.commit()?;
        Ok(expected + 1)
    }
    /// One read transaction guarantees policy and routing share a revision.
    pub fn snapshot(&mut self) -> Result<Snapshot, Error> {
        let tx = self.0.transaction()?;
        let revision: i64 =
            tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
        let devices = ids(&tx, "SELECT id FROM devices ORDER BY id", [])?;
        let mut services = Vec::new();
        {
            let mut stmt = tx.prepare("SELECT id,host FROM services ORDER BY id")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (id, host) = row?;
                let grants = ids(
                    &tx,
                    "SELECT device FROM grants WHERE service=?1 ORDER BY device",
                    [&id],
                )?;
                services.push(Service {
                    id,
                    host: host.parse().map_err(|_| Error::Storage)?,
                    grants,
                });
            }
        }
        tx.commit()?;
        Ok(Snapshot {
            revision: revision.try_into().map_err(|_| Error::Storage)?,
            devices,
            services,
        })
    }
    /// SQLite's online backup yields a consistent standalone database.
    /// Destination must be a newly reserved private file, never an existing backup.
    pub fn backup(&self, destination: &Path) -> Result<(), Error> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(destination)
            .map_err(|_| Error::Storage)?;
        self.0.backup("main", destination, None)?;
        file.sync_all().map_err(|_| Error::Storage)?;
        std::fs::File::open(destination.parent().ok_or(Error::InvalidInput)?)
            .and_then(|parent| parent.sync_all())
            .map_err(|_| Error::Storage)?;
        Ok(())
    }
    pub fn ready(&self) -> Result<bool, Error> {
        let result: Option<String> = self
            .0
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .optional()?;
        Ok(result.as_deref() == Some("ok"))
    }
}
fn ids<P: rusqlite::Params>(
    db: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<EndpointId>, Error> {
    let mut stmt = db.prepare(sql)?;
    stmt.query_map(params, |r| r.get::<_, String>(0))?
        .map(|r| r?.parse().map_err(|_| Error::Storage))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wispnet_core::identity::Identity;
    fn private_dir() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }
    #[test]
    fn restart_backup_restore_and_atomic_conflicts() {
        let dir = private_dir();
        let path = dir.path().join("registry.db");
        let backup = dir.path().join("backup.db");
        let host = Identity::generate().endpoint_id();
        let client = Identity::generate().endpoint_id();
        let mut store = Store::open(&path).unwrap();
        assert!(store.ready().unwrap());
        assert_eq!(
            store.replace(
                0,
                &[host, client],
                &[Service {
                    id: "website".into(),
                    host,
                    grants: vec![client]
                }]
            ),
            Ok(1)
        );
        let original = store.snapshot().unwrap();
        assert_eq!(store.replace(0, &[], &[]), Err(Error::Conflict));
        // Invalid foreign key after deletion must roll back the entire update.
        assert_eq!(
            store.replace(
                1,
                &[host],
                &[Service {
                    id: "website".into(),
                    host,
                    grants: vec![client]
                }]
            ),
            Err(Error::Storage)
        );
        assert_eq!(store.snapshot().unwrap(), original);
        store.backup(&backup).unwrap();
        assert_eq!(store.backup(&backup), Err(Error::Storage));
        drop(store);
        assert_eq!(Store::open(&path).unwrap().snapshot().unwrap(), original);
        // Restore offline into a new location; no live database is overwritten.
        let restored = dir.path().join("restored.db");
        std::fs::copy(backup, &restored).unwrap();
        assert_eq!(
            Store::open(&restored).unwrap().snapshot().unwrap(),
            original
        );
    }
    #[test]
    fn independent_writers_cannot_overwrite_newer_policy() {
        let dir = private_dir();
        let path = dir.path().join("db");
        let mut first = Store::open(&path).unwrap();
        let mut second = Store::open(&path).unwrap();
        let stale = second.snapshot().unwrap().revision;
        let host = Identity::generate().endpoint_id();
        first.replace(0, &[host], &[]).unwrap();
        assert_eq!(second.replace(stale, &[], &[]), Err(Error::Conflict));
        assert_eq!(second.snapshot().unwrap().devices, vec![host]);
    }
    #[test]
    fn shared_files_and_symlinks_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = private_dir();
        let path = dir.path().join("db");
        drop(Store::open(&path).unwrap());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(Store::open(&path), Err(Error::Storage)));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(matches!(Store::open(&link), Err(Error::Storage)));
        let hardlink = dir.path().join("hardlink");
        std::fs::hard_link(&path, &hardlink).unwrap();
        assert!(matches!(Store::open(&path), Err(Error::Storage)));
    }
    #[test]
    fn future_schema_and_input_limits_are_rejected() {
        let dir = private_dir();
        let path = dir.path().join("db");
        let mut store = Store::open(&path).unwrap();
        let host = Identity::generate().endpoint_id();
        assert_eq!(
            store.replace(
                0,
                &[host],
                &[Service {
                    id: "INVALID".into(),
                    host,
                    grants: vec![]
                }]
            ),
            Err(Error::InvalidInput)
        );
        store.0.pragma_update(None, "user_version", 999).unwrap();
        drop(store);
        assert!(matches!(Store::open(&path), Err(Error::UnsupportedSchema)));
    }
}
