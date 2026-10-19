//! Single-owner controller data and authorization. Never log credentials.
use crate::store::{Error, Store};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::SocketAddr,
    path::Path,
};
use wispnet_core::{
    identity::{EndpointId, SecretKey},
    protocol::valid_service_id,
};
use zeroize::Zeroizing;

pub fn token() -> Zeroizing<String> {
    let bytes = Zeroizing::new(SecretKey::generate().to_bytes());
    Zeroizing::new(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn valid_token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub fn read_token(path: &Path) -> Result<Zeroizing<String>, Error> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| Error::Storage)?;
    let meta = file.metadata().map_err(|_| Error::Storage)?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.mode() & 0o777 != 0o600
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.len() != 64
    {
        return Err(Error::Storage);
    }
    let mut secret = Zeroizing::new(String::new());
    file.take(65)
        .read_to_string(&mut secret)
        .map_err(|_| Error::Storage)?;
    if !valid_token(&secret) {
        return Err(Error::Storage);
    }
    Ok(secret)
}
/// Crash-retry-safe bootstrap: reserve the credential first, then initialize once.
/// An existing owner can never be replaced by rerunning this command.
pub fn initialize(store: &mut Store, path: &Path, mesh: &str) -> Result<(), Error> {
    if !valid_service_id(mesh) {
        return Err(Error::InvalidInput);
    }
    if !path.try_exists().map_err(|_| Error::Storage)? {
        let secret = token();
        let parent = path.parent().ok_or(Error::InvalidInput)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|_| Error::Storage)?;
        temp.write_all(secret.as_bytes())
            .map_err(|_| Error::Storage)?;
        temp.as_file().sync_all().map_err(|_| Error::Storage)?;
        temp.persist_noclobber(path).map_err(|_| Error::Storage)?;
        std::fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|_| Error::Storage)?;
    }
    let secret = read_token(path)?;
    let tx = store
        .0
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<(String, String)> = tx
        .query_row(
            "SELECT mesh,owner_hash FROM controller WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((name, digest)) = existing {
        if name != mesh || digest != hash(&secret) {
            return Err(Error::Conflict);
        }
    } else {
        tx.execute(
            "INSERT INTO controller VALUES(1,?1,?2)",
            params![mesh, hash(&secret)],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[derive(Clone, Copy)]
pub enum Principal {
    Owner,
    Device(EndpointId),
}
impl Store {
    pub fn initialized(&self) -> Result<bool, Error> {
        Ok(self
            .0
            .query_row("SELECT EXISTS(SELECT 1 FROM controller)", [], |r| r.get(0))?)
    }
    pub fn authenticate(&self, secret: &str) -> Result<Option<Principal>, Error> {
        if !valid_token(secret) {
            return Ok(None);
        }
        let digest = hash(secret);
        let owner: bool = self.0.query_row(
            "SELECT EXISTS(SELECT 1 FROM controller WHERE owner_hash=?1)",
            [&digest],
            |r| r.get(0),
        )?;
        if owner {
            return Ok(Some(Principal::Owner));
        }
        let id: Option<String> = self
            .0
            .query_row(
                "SELECT device FROM device_tokens WHERE hash=?1",
                [digest],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|s| s.parse().map(Principal::Device).map_err(|_| Error::Storage))
            .transpose()
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Directory {
    pub mesh: String,
    pub revision: u64,
    pub device: Option<EndpointId>,
    pub devices: Vec<EndpointId>,
    pub services: Vec<DirectoryService>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryService {
    pub id: String,
    pub host: EndpointId,
    pub address: Option<SocketAddr>,
    pub grants: Vec<EndpointId>,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Mutation {
    Revoke {
        expected_revision: u64,
        device: EndpointId,
    },
    Service {
        expected_revision: u64,
        id: String,
        host: EndpointId,
        grants: Vec<EndpointId>,
    },
    Publish {
        expected_revision: u64,
        id: String,
        address: SocketAddr,
    },
}
impl Mutation {
    fn revision(&self) -> u64 {
        match self {
            Self::Revoke {
                expected_revision, ..
            }
            | Self::Service {
                expected_revision, ..
            }
            | Self::Publish {
                expected_revision, ..
            } => *expected_revision,
        }
    }
}
/// Deliberately lacks Debug; enrollment responses carry credentials.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Updated {
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}
impl Drop for Updated {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.token.zeroize();
    }
}
#[derive(Debug, Clone, Copy)]
pub enum ControlError {
    Storage,
    Invalid,
    Conflict,
    Forbidden,
    NotReady,
    RateLimited,
}
impl From<Error> for ControlError {
    fn from(e: Error) -> Self {
        match e {
            Error::InvalidInput => Self::Invalid,
            Error::Conflict => Self::Conflict,
            _ => Self::Storage,
        }
    }
}
impl Store {
    pub fn directory(&mut self, principal: Principal) -> Result<Directory, ControlError> {
        let tx = self.0.transaction()?;
        let mesh: String =
            tx.query_row("SELECT mesh FROM controller WHERE id=1", [], |r| r.get(0))?;
        let revision: i64 =
            tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
        let device = match principal {
            Principal::Owner => None,
            Principal::Device(id) => Some(id),
        };
        let mut devices = Vec::new();
        if device.is_none() {
            let mut stmt = tx.prepare("SELECT id FROM devices ORDER BY id")?;
            for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
                devices.push(row?.parse().map_err(|_| ControlError::Storage)?);
            }
        } else if let Some(id) = device {
            devices.push(id);
        }
        let mut services = Vec::new();
        {
            let mut stmt=tx.prepare("SELECT s.id,s.host,r.address FROM services s LEFT JOIN routes r ON r.service=s.id WHERE ?1 IS NULL OR s.host=?1 OR EXISTS(SELECT 1 FROM grants g WHERE g.service=s.id AND g.device=?1) ORDER BY s.id")?;
            for row in stmt.query_map([device.map(|d| d.to_string())], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })? {
                let (id, host, address) = row?;
                let host: EndpointId = host.parse().map_err(|_| ControlError::Storage)?;
                let mut grants = Vec::new();
                if device.is_none() || device == Some(host) {
                    let mut stmt =
                        tx.prepare("SELECT device FROM grants WHERE service=?1 ORDER BY device")?;
                    for row in stmt.query_map([&id], |r| r.get::<_, String>(0))? {
                        grants.push(row?.parse().map_err(|_| ControlError::Storage)?);
                    }
                }
                services.push(DirectoryService {
                    id,
                    host,
                    address: address
                        .map(|s| s.parse().map_err(|_| ControlError::Storage))
                        .transpose()?,
                    grants,
                });
            }
        }
        tx.commit()?;
        Ok(Directory {
            mesh,
            revision: revision.try_into().map_err(|_| ControlError::Storage)?,
            device,
            devices,
            services,
        })
    }
    pub fn mutate(
        &mut self,
        principal: Principal,
        mutation: Mutation,
    ) -> Result<Updated, ControlError> {
        let expected = mutation.revision();
        if expected >= i64::MAX as u64 {
            return Err(ControlError::Invalid);
        }
        if !matches!(principal, Principal::Owner) && !matches!(&mutation, Mutation::Publish { .. })
        {
            return Err(ControlError::Forbidden);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: i64 =
            tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
        if expected != revision as u64 {
            return Err(ControlError::Conflict);
        }
        match mutation {
            Mutation::Revoke { device, .. } => {
                tx.execute(
                    "UPDATE device_records SET state='revoked', revoked_at=?2 WHERE device=?1",
                    params![device.to_string(), crate::enrollment::now() as i64],
                )?;
                tx.execute("UPDATE invitations SET state='revoked',nonce=NULL WHERE device=?1 AND state='enrolled'",[device.to_string()])?;
                tx.execute("DELETE FROM grants WHERE device=?1", [device.to_string()])?;
                tx.execute("DELETE FROM services WHERE host=?1", [device.to_string()])?;
                if tx.execute("DELETE FROM devices WHERE id=?1", [device.to_string()])? == 0 {
                    return Err(ControlError::Invalid);
                }
            }
            Mutation::Service {
                id, host, grants, ..
            } => {
                if !valid_service_id(&id)
                    || grants.len() > 1024
                    || grants
                        .iter()
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        != grants.len()
                {
                    return Err(ControlError::Invalid);
                }
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1)",
                    [host.to_string()],
                    |r| r.get(0),
                )?;
                if !exists {
                    return Err(ControlError::Invalid);
                }
                let count: u32 =
                    tx.query_row("SELECT count(*) FROM services WHERE id!=?1", [&id], |r| {
                        r.get(0)
                    })?;
                let grants_count: u32 = tx.query_row(
                    "SELECT count(*) FROM grants WHERE service!=?1",
                    [&id],
                    |r| r.get(0),
                )?;
                if count >= 1024 || grants_count as usize + grants.len() > 16384 {
                    return Err(ControlError::Invalid);
                }
                for grant in &grants {
                    let exists: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1)",
                        [grant.to_string()],
                        |r| r.get(0),
                    )?;
                    if !exists {
                        return Err(ControlError::Invalid);
                    }
                }
                // A hosting-owner change invalidates the previously published route.
                tx.execute("DELETE FROM routes WHERE service=?1 AND EXISTS(SELECT 1 FROM services WHERE id=?1 AND host!=?2)",params![id,host.to_string()])?;
                tx.execute("INSERT INTO services VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET host=excluded.host",params![id,host.to_string()])?;
                tx.execute("DELETE FROM grants WHERE service=?1", [&id])?;
                for grant in grants {
                    tx.execute(
                        "INSERT INTO grants VALUES(?1,?2)",
                        params![id, grant.to_string()],
                    )?;
                }
            }
            Mutation::Publish { id, address, .. } => {
                if !valid_service_id(&id)
                    || address.port() == 0
                    || address.ip().is_unspecified()
                    || address.ip().is_multicast()
                {
                    return Err(ControlError::Invalid);
                }
                let owner: Option<String> = tx
                    .query_row("SELECT host FROM services WHERE id=?1", [&id], |r| r.get(0))
                    .optional()?;
                let Some(owner) = owner else {
                    return Err(ControlError::Forbidden);
                };
                if let Principal::Device(device) = principal
                    && owner != device.to_string()
                {
                    return Err(ControlError::Forbidden);
                }
                tx.execute("INSERT INTO routes VALUES(?1,?2) ON CONFLICT(service) DO UPDATE SET address=excluded.address",params![id,address.to_string()])?;
            }
        }
        tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
        tx.commit()?;
        Ok(Updated {
            revision: expected + 1,
            token: None,
        })
    }
}
impl From<rusqlite::Error> for ControlError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

/// Reserve private credential storage before committing remote enrollment.
pub fn credential_temp(path: &Path) -> Result<tempfile::NamedTempFile, Error> {
    use std::os::unix::fs::MetadataExt;
    let parent = path.parent().ok_or(Error::InvalidInput)?;
    let meta = std::fs::symlink_metadata(parent).map_err(|_| Error::Storage)?;
    if !meta.is_dir() || meta.mode() & 0o777 != 0o700 || meta.uid() != unsafe { libc::geteuid() } {
        return Err(Error::Storage);
    }
    tempfile::NamedTempFile::new_in(parent).map_err(|_| Error::Storage)
}
pub fn read_invitation(path: &Path) -> Result<wispnet_core::enrollment::Invitation, Error> {
    if !path.is_absolute() {
        return Err(Error::InvalidInput);
    }
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| Error::Storage)?;
    let meta = file.metadata().map_err(|_| Error::Storage)?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.mode() & 0o777 != 0o600
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.len() > 4096
    {
        return Err(Error::Storage);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Storage)?;
    let invitation: wispnet_core::enrollment::Invitation =
        serde_json::from_slice(&bytes).map_err(|_| Error::InvalidInput)?;
    if !invitation.valid() {
        return Err(Error::InvalidInput);
    }
    Ok(invitation)
}
