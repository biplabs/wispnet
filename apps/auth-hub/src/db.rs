use crate::{Error, Result, now, token};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Serialize;
use std::{
    fs::{DirBuilder, OpenOptions},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};

#[derive(Clone, Serialize)]
pub struct Service {
    pub id: String,
    pub host: String,
    pub owner_id: String,
    pub service: String,
    pub incarnation: String,
    pub name: String,
    pub resource: String,
    pub generation: i64,
    pub auth_mode: String,
    pub enabled: i64,
}
#[derive(Clone, Serialize)]
pub struct Host {
    pub host: String,
    pub owner_id: String,
    pub name: String,
    pub publisher_hash: Option<String>,
    pub revoked: i64,
}
pub fn open(state: &Path) -> Result<Connection> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(state)
        .map_err(|_| Error::new(500, "Cannot create command center state"))?;
    let dir = std::fs::symlink_metadata(state)
        .map_err(|_| Error::new(500, "Cannot inspect command center state"))?;
    if !dir.is_dir() || dir.uid() != unsafe { libc::geteuid() } || dir.mode() & 0o077 != 0 {
        return Err(Error::new(
            400,
            "Command center state must be an owned directory with mode 700",
        ));
    }
    let path = state.join("command.sqlite");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
        .map_err(|_| Error::new(400, "Unsafe command center database"))?;
    let info = file
        .metadata()
        .map_err(|_| Error::new(400, "Unsafe command center database"))?;
    if !info.is_file()
        || info.nlink() != 1
        || info.uid() != unsafe { libc::geteuid() }
        || info.mode() & 0o777 != 0o600
    {
        return Err(Error::new(
            400,
            "Command center database must be an owned regular file with mode 600",
        ));
    }
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    db.busy_timeout(std::time::Duration::from_secs(3))?;
    let legacy:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name IN ('account','session','oauthApplication'))",[],|r|r.get(0))?;
    let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if !matches!(version, 0 | 1) {
        return Err(Error::new(
            400,
            "Unsupported command center database version; preserve the state and use a compatible binary",
        ));
    }
    let existing:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')",[],|r|r.get(0))?;
    if legacy || version == 0 && existing {
        return Err(Error::new(
            400,
            "Legacy Better Auth database detected; preserve its backup and use an explicitly migrated or new state directory",
        ));
    }
    if version==1 && db.prepare("SELECT u.github_id,s.token_hash,k.token_hash FROM users u,sessions s,command_keys k LIMIT 0").is_err(){return Err(Error::new(400,"Incompatible command center database schema; preserve the state before migration"));}
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
      BEGIN IMMEDIATE;
      CREATE TABLE IF NOT EXISTS users(id TEXT PRIMARY KEY,github_id TEXT NOT NULL UNIQUE,name TEXT NOT NULL,email TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS sessions(token_hash TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),expires INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS command_links(id TEXT PRIMARY KEY,host TEXT NOT NULL,name TEXT NOT NULL,nonce TEXT NOT NULL UNIQUE,device_hash TEXT NOT NULL UNIQUE,user_code TEXT NOT NULL UNIQUE,expires INTEGER NOT NULL,owner_id TEXT,consumed INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS command_hosts(host TEXT PRIMARY KEY,owner_id TEXT NOT NULL,name TEXT NOT NULL,publisher_hash TEXT UNIQUE,revoked INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS command_services(id TEXT PRIMARY KEY,host TEXT NOT NULL REFERENCES command_hosts(host),owner_id TEXT NOT NULL,service TEXT NOT NULL,incarnation TEXT NOT NULL,name TEXT NOT NULL,resource TEXT NOT NULL UNIQUE,generation INTEGER NOT NULL DEFAULT 1,auth_mode TEXT NOT NULL DEFAULT 'both',enabled INTEGER NOT NULL DEFAULT 1,UNIQUE(host,service));
      CREATE TABLE IF NOT EXISTS command_grants(service_id TEXT NOT NULL REFERENCES command_services(id),user_id TEXT NOT NULL REFERENCES users(id),generation INTEGER NOT NULL DEFAULT 1,active INTEGER NOT NULL DEFAULT 1,PRIMARY KEY(service_id,user_id));
      CREATE TABLE IF NOT EXISTS command_keys(id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),service_id TEXT NOT NULL REFERENCES command_services(id),service_generation INTEGER NOT NULL,grant_generation INTEGER NOT NULL,name TEXT NOT NULL,prefix TEXT NOT NULL,token_hash TEXT NOT NULL UNIQUE,expires INTEGER NOT NULL,revoked INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS command_oauth_connections(id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),service_id TEXT NOT NULL REFERENCES command_services(id),client_id TEXT NOT NULL,generation INTEGER NOT NULL DEFAULT 1,active INTEGER NOT NULL DEFAULT 1,UNIQUE(user_id,service_id,client_id));
      ")?;
    crate::identity::initialize(&db)?;
    crate::oauth::initialize(&db)?;
    db.execute_batch("PRAGMA user_version=1;COMMIT;")?;
    Ok(db)
}
fn read_service(r: &rusqlite::Row<'_>) -> rusqlite::Result<Service> {
    Ok(Service {
        id: r.get("id")?,
        host: r.get("host")?,
        owner_id: r.get("owner_id")?,
        service: r.get("service")?,
        incarnation: r.get("incarnation")?,
        name: r.get("name")?,
        resource: r.get("resource")?,
        generation: r.get("generation")?,
        auth_mode: r.get("auth_mode")?,
        enabled: r.get("enabled")?,
    })
}
pub fn service(db: &Connection, id: &str) -> Result<Option<Service>> {
    Ok(db
        .query_row(
            "SELECT * FROM command_services WHERE id=?",
            [id],
            read_service,
        )
        .optional()?)
}
pub fn resource(db: &Connection, uri: &str) -> Result<Option<Service>> {
    Ok(db
        .query_row(
            "SELECT * FROM command_services WHERE resource=?",
            [uri],
            read_service,
        )
        .optional()?)
}
pub fn host(db: &Connection, id: &str) -> Result<Option<Host>> {
    Ok(db
        .query_row("SELECT * FROM command_hosts WHERE host=?", [id], |r| {
            Ok(Host {
                host: r.get("host")?,
                owner_id: r.get("owner_id")?,
                name: r.get("name")?,
                publisher_hash: r.get("publisher_hash")?,
                revoked: r.get("revoked")?,
            })
        })
        .optional()?)
}
pub fn grant(db: &Connection, service: &Service, user: &str) -> Result<Option<i64>> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?)",
        [user],
        |r| r.get(0),
    )?;
    if !exists
        || service.enabled != 1
        || !host(db, &service.host)?
            .is_some_and(|h| h.revoked == 0 && h.owner_id == service.owner_id)
    {
        return Ok(None);
    }
    if service.owner_id == user {
        return Ok(Some(0));
    }
    Ok(db
        .query_row(
            "SELECT generation FROM command_grants WHERE service_id=? AND user_id=? AND active=1",
            params![service.id, user],
            |r| r.get(0),
        )
        .optional()?)
}
pub fn set_grant(db: &mut Connection, service: &Service, user: &str, active: bool) -> Result<()> {
    let tx = db.transaction()?;
    tx.execute("INSERT INTO command_grants(service_id,user_id,active) VALUES(?,?,?) ON CONFLICT(service_id,user_id) DO UPDATE SET active=excluded.active,generation=generation+1",params![service.id,user,active])?;
    tx.execute(
        "UPDATE command_keys SET revoked=1 WHERE service_id=? AND user_id=?",
        params![service.id, user],
    )?;
    tx.execute("UPDATE command_oauth_connections SET active=0,generation=generation+1 WHERE service_id=? AND user_id=?",params![service.id,user])?;
    tx.commit()?;
    Ok(())
}
pub fn revoke_connection(db: &Connection, id: &str) -> Result<()> {
    db.execute(
        "UPDATE command_oauth_connections SET active=0,generation=generation+1 WHERE id=?",
        [id],
    )?;
    Ok(())
}
pub fn register(
    db: &mut Connection,
    host: &Host,
    id: &str,
    incarnation: &str,
    name: &str,
    uri: &str,
) -> Result<Service> {
    let tx = db.transaction()?;
    let previous = tx
        .query_row(
            "SELECT * FROM command_services WHERE host=? AND service=?",
            params![host.host, id],
            read_service,
        )
        .optional()?;
    let sid = if let Some(previous) = previous {
        let replaced = previous.incarnation != incarnation
            || previous.resource != uri
            || previous.owner_id != host.owner_id;
        tx.execute("UPDATE command_services SET owner_id=?,incarnation=?,name=?,resource=?,enabled=1,generation=generation+? WHERE id=?",params![host.owner_id,incarnation,name,uri,i64::from(replaced),previous.id])?;
        if replaced {
            tx.execute(
                "UPDATE command_grants SET active=0,generation=generation+1 WHERE service_id=?",
                [&previous.id],
            )?;
            tx.execute(
                "UPDATE command_keys SET revoked=1 WHERE service_id=?",
                [&previous.id],
            )?;
            tx.execute("UPDATE command_oauth_connections SET active=0,generation=generation+1 WHERE service_id=?",[&previous.id])?;
        }
        previous.id
    } else {
        let sid = token("");
        tx.execute("INSERT INTO command_services(id,host,owner_id,service,incarnation,name,resource) VALUES(?,?,?,?,?,?,?)",params![sid,host.host,host.owner_id,id,incarnation,name,uri])?;
        sid
    };
    tx.commit()?;
    service(db, &sid)?.ok_or_else(|| Error::new(500, "Cannot register service"))
}
pub fn unlink(db: &mut Connection, host: &str) -> Result<()> {
    let tx = db.transaction()?;
    tx.execute("UPDATE command_links SET consumed=1 WHERE host=?", [host])?;
    tx.execute(
        "UPDATE command_hosts SET revoked=1,publisher_hash=NULL WHERE host=?",
        [host],
    )?;
    tx.execute(
        "UPDATE command_services SET enabled=0,generation=generation+1 WHERE host=?",
        [host],
    )?;
    tx.execute("UPDATE command_keys SET revoked=1 WHERE service_id IN(SELECT id FROM command_services WHERE host=?)",[host])?;
    tx.execute("UPDATE command_oauth_connections SET active=0,generation=generation+1 WHERE service_id IN(SELECT id FROM command_services WHERE host=?)",[host])?;
    tx.commit()?;
    Ok(())
}
pub fn authorize_key(db: &Connection, key: &str, service: &Service) -> Result<String> {
    let row:Option<(String,i64,i64)>=db.query_row("SELECT user_id,service_generation,grant_generation FROM command_keys WHERE token_hash=? AND service_id=? AND expires>? AND revoked=0",params![crate::digest(key),service.id,now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    if let Some((user, generation, grant_generation)) = row
        && generation == service.generation
        && grant(db, service, &user)? == Some(grant_generation)
    {
        return Ok(user);
    }
    Err(Error::new(403, "Service access denied"))
}
pub fn rows_json(
    db: &Connection,
    sql: &str,
    values: &[&dyn rusqlite::ToSql],
) -> Result<serde_json::Value> {
    let mut stmt = db.prepare(sql)?;
    let names: Vec<_> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let rows = stmt.query_map(values, |row| {
        let mut map = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                rusqlite::types::ValueRef::Integer(n) => serde_json::json!(n),
                rusqlite::types::ValueRef::Real(n) => serde_json::json!(n),
                rusqlite::types::ValueRef::Text(s) => serde_json::json!(String::from_utf8_lossy(s)),
                _ => serde_json::Value::Null,
            };
            map.insert(name.clone(), value);
        }
        Ok(serde_json::Value::Object(map))
    })?;
    Ok(serde_json::Value::Array(
        rows.collect::<std::result::Result<Vec<_>, _>>()?,
    ))
}
