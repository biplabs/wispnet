use crate::{
    Error, Result, Shared, body_json, db, digest, identity, json, now, required, same_origin, token,
};
use axum::{
    body::Body,
    http::{HeaderMap, Method, Request, header},
    response::{IntoResponse, Response},
};
use ed25519_dalek::{Signature, VerifyingKey};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json as value};

fn hex<'a>(body: &'a Value, key: &str) -> Result<&'a str> {
    let text = required(body, key, 64)?;
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::new(400, "Invalid device or service identity"));
    }
    Ok(text)
}
fn service_id(body: &Value) -> Result<&str> {
    let text = required(body, "service", 128)?;
    if !text.as_bytes()[0].is_ascii_alphanumeric()
        || !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(Error::new(400, "Invalid service"));
    }
    Ok(text)
}
fn bytes_hex<const N: usize>(text: &str) -> Result<[u8; N]> {
    if text.len() != N * 2
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::new(400, "Invalid host proof"));
    }
    let mut result = [0; N];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| Error::new(400, "Invalid host proof"))?;
    }
    Ok(result)
}
fn publisher(db: &Connection, headers: &HeaderMap, id: &str) -> Result<db::Host> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, t)| t)
        .ok_or_else(|| Error::new(403, "Publisher not authorized"))?;
    if raw.len() != 47
        || !raw.starts_with("wph_")
        || !raw[4..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(Error::new(403, "Publisher not authorized"));
    }
    let host = db::host(db, id)?
        .filter(|h| h.revoked == 0 && h.publisher_hash.as_deref() == Some(digest(raw).as_str()))
        .ok_or_else(|| Error::new(403, "Publisher not authorized"))?;
    Ok(host)
}
fn owned_service(db: &Connection, id: &str, user: &str) -> Result<db::Service> {
    db::service(db, id)?
        .filter(|s| {
            s.owner_id == user
                && db::host(db, &s.host).is_ok_and(|h| h.is_some_and(|h| h.revoked == 0))
        })
        .ok_or_else(|| Error::new(403, "Service access denied"))
}
pub fn published_hostname(name: &str, host: &str, service: &str, domain: &str) -> String {
    fn slug(text: &str, limit: usize) -> String {
        let mut result = String::new();
        for byte in text.bytes() {
            if result.len() == limit {
                break;
            }
            if byte.is_ascii_alphanumeric() {
                result.push(byte.to_ascii_lowercase() as char);
            } else if !result.is_empty() && !result.ends_with('-') {
                result.push('-');
            }
        }
        result.trim_matches('-').into()
    }
    let mut part = slug(service, 24);
    if part != service {
        use sha1::Digest as _;
        let hash: String = sha1::Sha1::digest(service.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        part = format!("{}-{}", slug(service, 15), &hash[..8]);
    }
    let name = slug(name, 20);
    format!(
        "{}-{}-{part}.{domain}",
        if name.is_empty() { "node" } else { &name },
        &host[..12]
    )
}
pub async fn handle(hub: Shared, request: Request<Body>) -> Result<Response> {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or("").to_owned();
    if method == Method::GET && path == "/healthz" {
        return Ok(json(
            value!({"ok":true,"service":"wispnet-command-center","login":"github"}),
        ));
    }
    if method == Method::GET && matches!(path.as_str(), "/" | "/sign-in" | "/connect" | "/consent")
    {
        return Ok((
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            crate::ui::page(&path),
        )
            .into_response());
    }
    if method == Method::GET && path == "/assets/command.js" {
        return Ok((
            [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
            crate::ui::SCRIPT,
        )
            .into_response());
    }
    if method == Method::GET && path == "/assets/command.css" {
        return Ok((
            [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
            crate::ui::CSS,
        )
            .into_response());
    }
    if method == Method::POST && path == "/api/command/links/start" {
        let (_, body) = body_json(request).await?;
        return start_link(&hub, &body);
    }
    if method == Method::POST && path == "/api/command/links/poll" {
        let (_, body) = body_json(request).await?;
        return poll_link(&hub, &body);
    }
    if method == Method::POST
        && matches!(
            path.as_str(),
            "/api/command/host/register" | "/api/command/host/unlink" | "/api/command/authorize"
        )
    {
        let (headers, body) = body_json(request).await?;
        let host_id = hex(&body, "host")?;
        let mut db = hub.db.lock().unwrap();
        let host = publisher(&db, &headers, host_id)?;
        if path == "/api/command/host/unlink" {
            db::unlink(&mut db, host_id)?;
            return Ok(json(value!({"ok":true})));
        }
        if path == "/api/command/host/register" {
            let id = service_id(&body)?;
            let incarnation = hex(&body, "incarnation")?;
            let name = required(&body, "name", 80)?;
            let uri = required(&body, "resource", 2048)?;
            let parsed =
                url::Url::parse(uri).map_err(|_| Error::new(400, "Invalid public resource URL"))?;
            if parsed.scheme() != "https"
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.port().is_some()
                || parsed.fragment().is_some()
                || parsed.query().is_some()
                || parsed.host_str()
                    != Some(
                        published_hostname(name, host_id, id, &hub.config.public_domain).as_str(),
                    )
                || parsed.as_str() != uri
            {
                return Err(Error::new(400, "Invalid public resource URL"));
            }
            let resource = db::register(&mut db, &host, id, incarnation, name, uri)?;
            return Ok(json(
                value!({"ok":true,"resource":uri,"resource_id":resource.id,"auth_mode":resource.auth_mode}),
            ));
        }
        hub.throttle(&format!("authorize:{host_id}"), 1200)?;
        let service = db::resource(&db, required(&body, "resource", 2048)?)?
            .filter(|s| s.host == host_id)
            .ok_or_else(|| Error::new(403, "Service access denied"))?;
        if service.service != service_id(&body)?
            || service.incarnation != hex(&body, "incarnation")?
        {
            return Err(Error::new(403, "Service access denied"));
        }
        let credential = required(&body, "token", 16384)?;
        let subject = if credential.starts_with("wpk_") {
            if service.auth_mode == "oauth" {
                return Err(Error::new(403, "Service access denied"));
            }
            db::authorize_key(&db, credential, &service)?
        } else {
            if service.auth_mode == "bearer" {
                return Err(Error::new(403, "Service access denied"));
            }
            crate::oauth::authorize_token(&db, credential, &service)?
        };
        return Ok(json(
            value!({"allowed":true,"subject":subject,"lease_seconds":5}),
        ));
    }
    if !path.starts_with("/api/command/") {
        return Err(Error::new(404, "Not found"));
    }
    let user = identity::session(&hub, request.headers())?;
    if method == Method::GET && path == "/api/command/dashboard" {
        let db = hub.db.lock().unwrap();
        let services = db::rows_json(
            &db,
            "SELECT s.*,h.name AS host_name FROM command_services s JOIN command_hosts h ON h.host=s.host WHERE h.revoked=0 AND s.enabled=1 AND (s.owner_id=? OR EXISTS(SELECT 1 FROM command_grants g WHERE g.service_id=s.id AND g.user_id=? AND g.active=1)) ORDER BY s.name,s.service",
            &[&user.id, &user.id],
        )?;
        let hosts = db::rows_json(
            &db,
            "SELECT host,name,revoked FROM command_hosts WHERE owner_id=?",
            &[&user.id],
        )?;
        let keys = db::rows_json(
            &db,
            "SELECT id,service_id,name,prefix,expires,revoked FROM command_keys WHERE user_id=? ORDER BY expires DESC",
            &[&user.id],
        )?;
        let connections = db::rows_json(
            &db,
            "SELECT id,service_id,client_id,generation,active FROM command_oauth_connections WHERE user_id=?",
            &[&user.id],
        )?;
        let grants = db::rows_json(
            &db,
            "SELECT g.service_id,g.user_id,g.active,u.name,u.email FROM command_grants g JOIN users u ON u.id=g.user_id JOIN command_services s ON s.id=g.service_id WHERE s.owner_id=?",
            &[&user.id],
        )?;
        return Ok(json(
            value!({"user":user,"services":services,"hosts":hosts,"keys":keys,"connections":connections,"grants":grants}),
        ));
    }
    if method == Method::GET && path == "/api/command/link" {
        let codes: Vec<_> = url::form_urlencoded::parse(query.as_bytes())
            .filter(|(key, _)| key == "code")
            .map(|(_, s)| s.into_owned())
            .collect();
        if codes.len() != 1 || codes[0].len() > 32 {
            return Err(Error::new(400, "Invalid link code"));
        }
        let code = codes[0].replace('-', "").to_ascii_uppercase();
        let db = hub.db.lock().unwrap();
        let rows = db::rows_json(
            &db,
            "SELECT host,name,expires,owner_id FROM command_links WHERE user_code=? AND consumed=0 AND expires>?",
            &[&code, &now()],
        )?;
        return rows[0]
            .is_object()
            .then(|| json(rows[0].clone()))
            .ok_or_else(|| Error::new(404, "Link code expired or invalid"));
    }
    if method != Method::POST {
        return Err(Error::new(405, "Method not allowed"));
    }
    same_origin(&hub, request.headers())?;
    hub.throttle(&format!("owner:{}", user.id), 30)?;
    let (_, body) = body_json(request).await?;
    let mut db = hub.db.lock().unwrap();
    match path.as_str() {
        "/api/command/links/approve" => {
            let code = required(&body, "user_code", 32)?
                .replace('-', "")
                .to_ascii_uppercase();
            let row:Option<(String,Option<String>)>=db.query_row("SELECT host,owner_id FROM command_links WHERE user_code=? AND expires>? AND consumed=0",params![code,now()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            let (host, owner) =
                row.ok_or_else(|| Error::new(404, "Link code expired or invalid"))?;
            if owner.is_some_and(|id| id != user.id)
                || db::host(&db, &host)?.is_some_and(|h| h.revoked == 0 && h.owner_id != user.id)
            {
                return Err(Error::new(403, "Host already linked"));
            }
            db.execute(
                "UPDATE command_links SET owner_id=? WHERE user_code=?",
                params![user.id, code],
            )?;
            Ok(json(value!({"ok":true,"host":host})))
        }
        "/api/command/hosts/unlink" => {
            let host = hex(&body, "host")?;
            if db::host(&db, host)?.is_none_or(|h| h.owner_id != user.id) {
                return Err(Error::new(403, "Host access denied"));
            }
            db::unlink(&mut db, host)?;
            Ok(json(value!({"ok":true})))
        }
        "/api/command/grants" => {
            let service = owned_service(&db, required(&body, "service_id", 64)?, &user.id)?;
            let uid = required(&body, "user_id", 128)?;
            let exists: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM users WHERE id=?)",
                [uid],
                |r| r.get(0),
            )?;
            let active = body["active"]
                .as_bool()
                .ok_or_else(|| Error::new(400, "Select a registered user's account ID"))?;
            if !exists || uid == service.owner_id {
                return Err(Error::new(400, "Select a registered user's account ID"));
            }
            db::set_grant(&mut db, &service, uid, active)?;
            Ok(json(value!({"ok":true})))
        }
        "/api/command/services/mode" => {
            let service = owned_service(&db, required(&body, "service_id", 64)?, &user.id)?;
            let mode = required(&body, "auth_mode", 16)?;
            if !matches!(mode, "oauth" | "bearer" | "both") {
                return Err(Error::new(400, "Invalid public authentication mode"));
            }
            if mode != service.auth_mode {
                let tx = db.transaction()?;
                tx.execute(
                    "UPDATE command_services SET auth_mode=?,generation=generation+1 WHERE id=?",
                    params![mode, service.id],
                )?;
                tx.execute(
                    "UPDATE command_keys SET revoked=1 WHERE service_id=?",
                    [&service.id],
                )?;
                tx.execute("UPDATE command_oauth_connections SET active=0,generation=generation+1 WHERE service_id=?",[&service.id])?;
                tx.commit()?;
            }
            Ok(json(value!({"ok":true})))
        }
        "/api/command/keys/create" => {
            let service = db::service(&db, required(&body, "service_id", 64)?)?
                .ok_or_else(|| Error::new(403, "Service access denied"))?;
            let generation = db::grant(&db, &service, &user.id)?
                .filter(|_| service.auth_mode != "oauth")
                .ok_or_else(|| Error::new(403, "Service access denied"))?;
            let name = required(&body, "name", 80)?;
            let ttl = body
                .get("expires_in")
                .map_or(Some(2_592_000), Value::as_i64)
                .filter(|n| (3600..=31_536_000).contains(n))
                .ok_or_else(|| Error::new(400, "Choose an expiry between one hour and one year"))?;
            let key = token("wpk_");
            let id = token("");
            let expires = now() + ttl;
            db.execute("INSERT INTO command_keys(id,user_id,service_id,service_generation,grant_generation,name,prefix,token_hash,expires) VALUES(?,?,?,?,?,?,?,?,?)",params![id,user.id,service.id,service.generation,generation,name,&key[..10],digest(&key),expires])?;
            Ok(json(
                value!({"id":id,"key":key,"service_id":service.id,"expires":expires,"note":"Save this key now. It is shown only once."}),
            ))
        }
        "/api/command/keys/revoke" => {
            let id = required(&body, "key_id", 128)?;
            let changed = db.execute(
                "UPDATE command_keys SET revoked=1 WHERE id=? AND user_id=?",
                params![id, user.id],
            )?;
            if changed == 0 {
                return Err(Error::new(404, "Key not found"));
            }
            Ok(json(value!({"ok":true})))
        }
        "/api/command/connections/revoke" => {
            let id = required(&body, "connection_id", 128)?;
            let exists: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM command_oauth_connections WHERE id=? AND user_id=?)",
                params![id, user.id],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(Error::new(404, "Connection not found"));
            }
            db::revoke_connection(&db, id)?;
            Ok(json(value!({"ok":true})))
        }
        _ => Err(Error::new(404, "Not found")),
    }
}
fn start_link(hub: &crate::Hub, body: &Value) -> Result<Response> {
    hub.throttle("links:start", 60)?;
    let host = hex(body, "host")?;
    let nonce = hex(body, "nonce")?;
    let name = required(body, "name", 80)?;
    let key = VerifyingKey::from_bytes(&bytes_hex(host)?)
        .map_err(|_| Error::new(400, "Invalid host proof"))?;
    let signature = Signature::from_bytes(&bytes_hex(required(body, "signature", 128)?)?);
    let proof = format!(
        "wispnet-command-link-v1\n{}\n{host}\n{name}\n{nonce}",
        hub.config.base_url
    );
    key.verify_strict(proof.as_bytes(), &signature)
        .map_err(|_| Error::new(403, "Invalid host proof"))?;
    let db = hub.db.lock().unwrap();
    let replay: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_links WHERE nonce=?)",
        [nonce],
        |r| r.get(0),
    )?;
    if replay {
        return Err(Error::new(409, "Link challenge already used"));
    }
    let active: i64 = db.query_row(
        "SELECT COUNT(*) FROM command_links WHERE host=? AND expires>? AND consumed=0",
        params![host, now()],
        |r| r.get(0),
    )?;
    if active >= 5 {
        return Err(Error::new(429, "Finish an existing link request first"));
    }
    let device = token("wpd_");
    let code = digest(&token(""))[..12].to_ascii_uppercase();
    let shown = format!("{}-{}-{}", &code[..4], &code[4..8], &code[8..]);
    db.execute("INSERT INTO command_links(id,host,name,nonce,device_hash,user_code,expires) VALUES(?,?,?,?,?,?,?)",params![token(""),host,name,nonce,digest(&device),code,now()+600])?;
    Ok(json(
        value!({"device_code":device,"user_code":shown,"verification_url":format!("{}/connect?code={shown}",hub.config.base_url),"expires_in":600,"interval":5}),
    ))
}
fn poll_link(hub: &crate::Hub, body: &Value) -> Result<Response> {
    let device = required(body, "device_code", 128)?;
    let mut db = hub.db.lock().unwrap();
    let tx = db.transaction()?;
    let link:Option<(String,String,String,Option<String>)>=tx.query_row("SELECT id,host,name,owner_id FROM command_links WHERE device_hash=? AND expires>? AND consumed=0",params![digest(device),now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let (id, host, name, owner) =
        link.ok_or_else(|| Error::new(404, "Link request expired or completed"))?;
    let Some(owner) = owner else {
        return Ok(json(value!({"state":"pending"})));
    };
    if db::host(&tx, &host)?.is_some_and(|h| h.revoked == 0 && h.owner_id != owner) {
        return Err(Error::new(403, "Host already linked"));
    }
    let account: Option<String> = tx
        .query_row("SELECT name FROM users WHERE id=?", [&owner], |r| r.get(0))
        .optional()?;
    let account = account.ok_or_else(|| Error::new(403, "Account no longer available"))?;
    let publisher = token("wph_");
    tx.execute("INSERT INTO command_hosts(host,owner_id,name,publisher_hash,revoked) VALUES(?,?,?,?,0) ON CONFLICT(host) DO UPDATE SET owner_id=excluded.owner_id,name=excluded.name,publisher_hash=excluded.publisher_hash,revoked=0",params![host,owner,name,digest(&publisher)])?;
    tx.execute("UPDATE command_links SET consumed=1 WHERE id=?", [id])?;
    tx.commit()?;
    Ok(json(
        value!({"state":"linked","publisher_token":publisher,"account":{"id":owner,"name":account}}),
    ))
}
