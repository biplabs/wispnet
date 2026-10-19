//! Independent single-writer authority. Remote catalogs never enter these tables.
use crate::{
    control::{self, ControlError},
    store::Store,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};
use tokio::sync::Mutex;
use wispnet_core::{
    config::ServiceConfig,
    enrollment::valid_name,
    host::{Catalog, Service, ServiceKind},
    identity::EndpointId,
    protocol::valid_service_id,
};

#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Mutation {
    Approve {
        expected_revision: u64,
        peer: EndpointId,
        name: String,
    },
    Revoke {
        expected_revision: u64,
        peer: EndpointId,
    },
    ApproveInvitation {
        expected_revision: u64,
        invitation: String,
        /// Additional existing services to grant in the same approval transaction.
        #[serde(default)]
        services: Vec<String>,
    },
    RejectInvitation {
        expected_revision: u64,
        invitation: String,
    },
    Service {
        expected_revision: u64,
        id: String,
        backend: std::net::SocketAddr,
        allow: Vec<EndpointId>,
        #[serde(default)]
        display_name: Option<String>,
        #[serde(default)]
        kind: Option<ServiceKind>,
    },
    Remove {
        expected_revision: u64,
        id: String,
    },
}
impl Mutation {
    fn revision(&self) -> u64 {
        match self {
            Self::Approve {
                expected_revision, ..
            }
            | Self::Revoke {
                expected_revision, ..
            }
            | Self::ApproveInvitation {
                expected_revision, ..
            }
            | Self::RejectInvitation {
                expected_revision, ..
            }
            | Self::Service {
                expected_revision, ..
            }
            | Self::Remove {
                expected_revision, ..
            } => *expected_revision,
        }
    }
}
#[derive(Serialize)]
pub struct Snapshot {
    pub host: EndpointId,
    pub generation: String,
    pub revision: u64,
    pub clients: Vec<Client>,
    pub services: Vec<LocalService>,
}
#[derive(Serialize)]
pub struct Client {
    pub peer: EndpointId,
    pub name: String,
}
#[derive(Clone, Serialize)]
pub struct LocalService {
    pub id: String,
    pub incarnation: String,
    pub display_name: String,
    pub kind: ServiceKind,
    pub backend: std::net::SocketAddr,
    pub allow: Vec<EndpointId>,
}
impl Store {
    pub fn host_initialize(&mut self, host: EndpointId, secret: &str) -> Result<(), ControlError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool =
            tx.query_row("SELECT EXISTS(SELECT 1 FROM host_authority)", [], |r| {
                r.get(0)
            })?;
        if exists {
            return Err(ControlError::Conflict);
        }
        let legacy:i64=tx.query_row("SELECT (SELECT count(*) FROM controller)+(SELECT count(*) FROM devices)+(SELECT count(*) FROM services)",[],|r|r.get(0))?;
        if legacy != 0 || !control::valid_token(secret) {
            return Err(ControlError::Invalid);
        }
        tx.execute(
            "INSERT INTO host_authority VALUES(1,?1,?2,?3)",
            params![host.to_string(), &*control::token(), control::hash(secret)],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn host_check(&self, host: EndpointId, secret: Option<&str>) -> Result<(), ControlError> {
        let authority: Option<(String, String)> = self
            .0
            .query_row(
                "SELECT endpoint,owner_hash FROM host_authority WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((id, digest)) = authority else {
            return Err(ControlError::NotReady);
        };
        if id != host.to_string() {
            return Err(ControlError::Forbidden);
        }
        if secret.is_some_and(|s| !control::valid_token(s) || control::hash(s) != digest) {
            return Err(ControlError::Forbidden);
        }
        Ok(())
    }
    pub fn host_snapshot(&mut self, host: EndpointId) -> Result<Snapshot, ControlError> {
        let tx = self.0.transaction()?;
        let (stored, generation): (String, String) = tx.query_row(
            "SELECT endpoint,generation FROM host_authority WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if stored != host.to_string() || !wispnet_core::enrollment::valid_code(&generation) {
            return Err(ControlError::Forbidden);
        }
        let revision: i64 =
            tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
        let mut clients = Vec::new();
        let mut services = Vec::new();
        {
            let mut stmt = tx.prepare("SELECT id,name FROM host_clients ORDER BY id")?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))? {
                let (peer, name): (String, String) = row?;
                if !valid_name(&name) {
                    return Err(ControlError::Storage);
                }
                clients.push(Client {
                    peer: peer.parse().map_err(|_| ControlError::Storage)?,
                    name,
                });
            }
            let mut stmt = tx.prepare(
                "SELECT id,backend,incarnation,display_name,kind FROM host_services ORDER BY id",
            )?;
            for row in stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })? {
                let (id, backend, incarnation, display_name, kind) = row?;
                if !valid_name(&display_name) {
                    return Err(ControlError::Storage);
                }
                let kind = match kind.as_str() {
                    "http" => ServiceKind::Http,
                    "tcp" => ServiceKind::Tcp,
                    _ => return Err(ControlError::Storage),
                };
                if !wispnet_core::enrollment::valid_code(&incarnation) {
                    return Err(ControlError::Storage);
                }
                let mut allow = Vec::new();
                let mut grants =
                    tx.prepare("SELECT client FROM host_grants WHERE service=?1 ORDER BY client")?;
                for row in grants.query_map([&id], |r| r.get::<_, String>(0))? {
                    allow.push(row?.parse().map_err(|_| ControlError::Storage)?);
                }
                if allow.iter().any(|p| !clients.iter().any(|c| c.peer == *p)) {
                    return Err(ControlError::Storage);
                }
                let backend = backend.parse().map_err(|_| ControlError::Storage)?;
                let service = ServiceConfig {
                    id: id.clone(),
                    backend,
                    allow: allow.clone(),
                };
                wispnet_core::config::validate_services(&[service])
                    .map_err(|_| ControlError::Storage)?;
                services.push(LocalService {
                    id,
                    incarnation,
                    display_name,
                    kind,
                    backend,
                    allow,
                });
            }
        }
        tx.commit()?;
        Ok(Snapshot {
            host,
            generation,
            revision: revision.try_into().map_err(|_| ControlError::Storage)?,
            clients,
            services,
        })
    }
    pub fn host_mutate(
        &mut self,
        host: EndpointId,
        secret: &str,
        operation: Mutation,
    ) -> Result<u64, ControlError> {
        // Host identity and owner hash are immutable after initialization.
        self.host_check(host, Some(secret))?;
        let expected = operation.revision();
        if expected >= i64::MAX as u64 {
            return Err(ControlError::Invalid);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: i64 =
            tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
        if revision as u64 != expected {
            return Err(ControlError::Conflict);
        }
        apply_mutation(&tx, operation)?;
        tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
        tx.commit()?;
        Ok(expected + 1)
    }
}
pub(crate) fn apply_mutation(
    tx: &rusqlite::Transaction<'_>,
    operation: Mutation,
) -> Result<(), ControlError> {
    match operation {
        Mutation::ApproveInvitation {
            invitation,
            services,
            ..
        } => {
            let unique: std::collections::BTreeSet<_> = services.iter().collect();
            if services.len() > 128 || unique.len() != services.len() {
                return Err(ControlError::Invalid);
            }
            for service in &services {
                if !valid_service_id(service)
                    || !tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM host_services WHERE id=?1)",
                        [service],
                        |r| r.get::<_, bool>(0),
                    )?
                {
                    return Err(ControlError::Invalid);
                }
            }
            let peer = crate::pairing::approve(tx, &invitation, crate::enrollment::now())?;
            // Preserve other clients' grants and any access this peer already has.
            for service in services {
                tx.execute(
                    "INSERT OR IGNORE INTO host_grants VALUES(?1,?2)",
                    params![service, peer],
                )?;
            }
        }
        Mutation::RejectInvitation { invitation, .. } => {
            crate::pairing::reject(tx, &invitation)?;
        }
        Mutation::Approve { peer, name, .. } => {
            if !valid_name(&name) {
                return Err(ControlError::Invalid);
            }
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM host_clients WHERE id!=?1",
                [peer.to_string()],
                |r| r.get(0),
            )?;
            if count >= 1024 {
                return Err(ControlError::Invalid);
            }
            tx.execute("INSERT INTO host_clients VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET name=excluded.name",params![peer.to_string(),name])?;
        }
        Mutation::Revoke { peer, .. } => {
            if tx.execute("DELETE FROM host_clients WHERE id=?1", [peer.to_string()])? == 0 {
                return Err(ControlError::Invalid);
            }
            tx.execute(
                "UPDATE host_invitations SET state='revoked',challenge=NULL
                    WHERE peer=?1 AND state IN ('issued','challenged','pending')",
                [peer.to_string()],
            )?;
        }
        Mutation::Service {
            id,
            backend,
            allow,
            display_name,
            kind,
            ..
        } => {
            wispnet_core::config::validate_services(&[ServiceConfig {
                id: id.clone(),
                backend,
                allow: allow.clone(),
            }])
            .map_err(|_| ControlError::Invalid)?;
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM host_services WHERE id!=?1",
                [&id],
                |r| r.get(0),
            )?;
            if count >= 128 {
                return Err(ControlError::Invalid);
            }
            for peer in &allow {
                let approved: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM host_clients WHERE id=?1)",
                    [peer.to_string()],
                    |r| r.get(0),
                )?;
                if !approved {
                    return Err(ControlError::Forbidden);
                }
            }
            if display_name
                .as_deref()
                .is_some_and(|name| !valid_name(name))
            {
                return Err(ControlError::Invalid);
            }
            let kind = kind.map(|k| match k {
                ServiceKind::Http => "http",
                ServiceKind::Tcp => "tcp",
            });
            tx.execute("INSERT INTO host_services(id,backend,incarnation,display_name,kind)
                VALUES(?1,?2,lower(hex(randomblob(32))),coalesce(?3,?1),coalesce(?4,'http'))
                ON CONFLICT(id) DO UPDATE SET backend=excluded.backend,
                incarnation=CASE WHEN ?4 IS NOT NULL AND ?4!=host_services.kind THEN lower(hex(randomblob(32))) ELSE host_services.incarnation END,
                display_name=coalesce(?3,host_services.display_name),kind=coalesce(?4,host_services.kind)",
                params![id,backend.to_string(),display_name,kind])?;
            tx.execute("DELETE FROM host_grants WHERE service=?1", [&id])?;
            for peer in allow {
                tx.execute(
                    "INSERT INTO host_grants VALUES(?1,?2)",
                    params![id, peer.to_string()],
                )?;
            }
        }
        Mutation::Remove { id, .. } => {
            if !valid_service_id(&id)
                || tx.execute("DELETE FROM host_services WHERE id=?1", [id])? == 0
            {
                return Err(ControlError::Invalid);
            }
        }
    }
    Ok(())
}
#[derive(Clone)]
pub struct Authority {
    pub(crate) host: EndpointId,
    pub(crate) store: Arc<Mutex<Store>>,
}
impl Authority {
    pub fn open(directory: &Path, host: EndpointId) -> Result<Self, ControlError> {
        let path = directory.join("host.sqlite3");
        // Agent startup must never create/reconstruct missing authoritative state.
        if !path.try_exists().map_err(|_| ControlError::Storage)? {
            return Err(ControlError::NotReady);
        }
        let mut store = Store::open(&path)?;
        store.host_check(host, None)?;
        if !store.ready()? {
            return Err(ControlError::Storage);
        }
        store.host_snapshot(host)?;
        Ok(Self {
            host,
            store: Arc::new(Mutex::new(store)),
        })
    }
    pub fn host(&self) -> EndpointId {
        self.host
    }
    pub async fn snapshot(&self) -> Result<Snapshot, ControlError> {
        let host = self.host;
        let mut store = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            self.store.clone().lock_owned(),
        )
        .await
        .map_err(|_| ControlError::RateLimited)?;
        tokio::task::spawn_blocking(move || store.host_snapshot(host))
            .await
            .map_err(|_| ControlError::Storage)?
    }
    /// The transport-authenticated peer may update only its existing display label.
    pub async fn set_client_name(
        &self,
        peer: EndpointId,
        name: String,
    ) -> Result<(), ControlError> {
        if !valid_name(&name) {
            return Err(ControlError::Invalid);
        }
        let host = self.host;
        let mut store = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            self.store.clone().lock_owned(),
        )
        .await
        .map_err(|_| ControlError::RateLimited)?;
        tokio::task::spawn_blocking(move || {
            store.host_check(host, None)?;
            let tx = store
                .0
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let previous: Option<String> = tx
                .query_row(
                    "SELECT name FROM host_clients WHERE id=?1",
                    [peer.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            let previous = previous.ok_or(ControlError::Forbidden)?;
            if previous != name {
                let revision: i64 =
                    tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
                if revision == i64::MAX {
                    return Err(ControlError::Invalid);
                }
                tx.execute(
                    "UPDATE host_clients SET name=?1 WHERE id=?2",
                    params![name, peer.to_string()],
                )?;
                tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .map_err(|_| ControlError::Storage)?
    }
    pub async fn approved(&self, peer: EndpointId) -> bool {
        self.snapshot()
            .await
            .is_ok_and(|s| s.clients.iter().any(|c| c.peer == peer))
    }
    pub async fn services(&self) -> Result<Vec<LocalService>, ControlError> {
        Ok(self.snapshot().await?.services)
    }
    pub async fn catalog(&self, peer: EndpointId) -> Result<Catalog, ControlError> {
        let snapshot = self.snapshot().await?;
        if !snapshot.clients.iter().any(|c| c.peer == peer) {
            return Err(ControlError::Forbidden);
        }
        let services: Vec<Service> = snapshot
            .services
            .into_iter()
            .filter(|s| s.allow.contains(&peer))
            .map(|s| Service {
                id: s.id,
                incarnation: s.incarnation,
                display_name: s.display_name,
                kind: s.kind,
            })
            .collect();
        Ok(Catalog {
            version: 1,
            complete: true,
            service_count: services.len() as u16,
            host: self.host,
            generation: snapshot.generation,
            revision: snapshot.revision,
            services,
        })
    }
}
impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Storage => "host state unavailable or corrupt",
            Self::Invalid => "invalid host operation",
            Self::Conflict => "host revision conflict",
            Self::Forbidden => "host operation denied",
            Self::NotReady => {
                "host authority missing; initialize explicitly or restore protected state"
            }
            Self::RateLimited => "host operation rate limited",
        })
    }
}
impl std::error::Error for ControlError {}
/// Explicit initialization only; a surviving credential blocks lost-database reset.
pub fn initialize(directory: &Path, host: EndpointId) -> Result<(), ControlError> {
    use std::io::Write;
    let path = directory.join("host.sqlite3");
    let credential = directory.join("host-owner.token");
    if credential.try_exists().map_err(|_| ControlError::Storage)? {
        if !path.try_exists().map_err(|_| ControlError::Storage)? {
            return Err(ControlError::NotReady);
        }
        let store = Store::open(&path)?;
        store.host_check(host, Some(&control::read_token(&credential)?))?;
        return Ok(());
    }
    if path.try_exists().map_err(|_| ControlError::Storage)? {
        return Err(ControlError::Conflict);
    }
    let secret = control::token();
    let mut temp = control::credential_temp(&credential)?;
    temp.write_all(secret.as_bytes())
        .map_err(|_| ControlError::Storage)?;
    temp.as_file()
        .sync_all()
        .map_err(|_| ControlError::Storage)?;
    temp.persist_noclobber(&credential)
        .map_err(|_| ControlError::Storage)?;
    std::fs::File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|_| ControlError::Storage)?;
    let mut store = Store::open(&path)?;
    store.host_initialize(host, &secret)
}
/// Open a verified existing store for protected local owner commands.
pub fn owner_store(
    directory: &Path,
    host: EndpointId,
) -> Result<(Store, zeroize::Zeroizing<String>), ControlError> {
    let secret = control::read_token(&directory.join("host-owner.token"))?;
    let path = directory.join("host.sqlite3");
    if !path.try_exists().map_err(|_| ControlError::Storage)? {
        return Err(ControlError::NotReady);
    }
    let store = Store::open(&path)?;
    store.host_check(host, Some(&secret))?;
    Ok((store, secret))
}
