use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{FileTypeExt, MetadataExt},
            net::UnixStream,
        },
    },
    path::Path,
    time::Duration,
};

pub fn request(path: &Path, payload: &Value) -> Result<Value, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "Node offline. Choose Go online to start it.")?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err("Node socket must be an owner-only socket.".into());
    }
    if !payload.is_object() {
        return Err("Expected a JSON object.".into());
    }
    let mut bytes = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if bytes.len() > 16384 {
        return Err("Request exceeds 16 KiB.".into());
    }
    let mut stream = UnixStream::connect(path).map_err(|_| "Cannot connect to the local node.")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(27)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(27)))
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let owner = {
        let mut uid = 0;
        let mut gid = 0;
        unsafe {
            libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) == 0 && uid == libc::geteuid()
        }
    };
    #[cfg(target_os = "linux")]
    let owner = {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of_val(&cred) as libc::socklen_t;
        unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            ) == 0
                && cred.uid == libc::geteuid()
        }
    };
    if !owner {
        return Err("Unexpected local node owner.".into());
    }
    stream
        .write_all(&bytes)
        .map_err(|_| "Request unconfirmed. Refresh before retrying.")?;
    let mut reply = Vec::new();
    BufReader::new(stream)
        .take(2 * 1024 * 1024 + 1)
        .read_until(b'\n', &mut reply)
        .map_err(|_| "Node reply unconfirmed. Refresh before retrying.")?;
    if reply.len() > 2 * 1024 * 1024 || reply.last() != Some(&b'\n') {
        return Err("Invalid or oversized node reply.".into());
    }
    let envelope: Value = serde_json::from_slice(&reply).map_err(|_| "Invalid node reply.")?;
    if envelope["ok"] != true {
        return Err(envelope["error"]
            .as_str()
            .unwrap_or("Node rejected the operation.")
            .into());
    }
    let result = envelope
        .get("result")
        .ok_or("Missing node result.")?
        .clone();
    if payload["op"] == "status" && result["api_version"] != 1 {
        return Err("Unsupported node API version.".into());
    }
    Ok(result)
}

pub fn browser_url(value: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(value).map_err(|_| "Invalid local browser handoff.")?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "http"
        || !host.ends_with(".localhost")
        || host.len() <= 10
        || url.port().is_none()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Invalid local browser handoff.".into());
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    #[test]
    fn browser_handoff_is_only_token_free_localhost() {
        assert!(browser_url("http://service.localhost:8080/").is_ok());
        for url in [
            "https://service.localhost:8080/",
            "http://localhost:8080/",
            "http://service.localhost:8080/?token=x",
            "http://user@service.localhost:8080/",
            "http://service.localhost:8080/path",
            "http://service.localhost:8080/#x",
            "http://service.localhost/",
            "http://service.localhost.evil:8080/",
        ] {
            assert!(browser_url(url).is_err(), "{url}");
        }
    }
    #[test]
    fn refuses_files_symlinks_and_public_sockets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.sock");
        fs::write(&path, "").unwrap();
        assert!(request(&path, &json!({"op":"status"})).is_err());
        fs::remove_file(&path).unwrap();
        let _listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(request(&path, &json!({"op":"status"})).is_err());
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(request(&link, &json!({"op":"status"})).is_err());
    }
    #[test]
    fn protocol_limits_and_errors() {
        for (reply, success) in [
            (
                "{\"ok\":true,\"result\":{\"api_version\":1}}\n".to_owned(),
                true,
            ),
            (
                "{\"ok\":true,\"result\":{\"api_version\":2}}\n".to_owned(),
                false,
            ),
            ("{\"ok\":false,\"error\":\"denied\"}\n".to_owned(), false),
            ("{}".to_owned(), false),
            ("x".repeat(2 * 1024 * 1024 + 1), false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("s");
            let listener = UnixListener::bind(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(&line).unwrap()["op"],
                    "status"
                );
                let _ = stream.write_all(reply.as_bytes());
            });
            assert_eq!(request(&path, &json!({"op":"status"})).is_ok(), success);
            server.join().unwrap();
        }
    }
}
