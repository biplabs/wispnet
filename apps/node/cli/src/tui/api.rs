//! Owner-local transport. The TUI never opens a second Iroh endpoint.
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
            net::UnixStream,
        },
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use wispnet_core::config::Config;

#[derive(Clone)]
pub struct Api {
    pub config: PathBuf,
    pub state: PathBuf,
}
impl Api {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        let config = fs::canonicalize(path).map_err(|_| "Cannot find host configuration")?;
        let data = Config::load(&config).map_err(|e| e.to_string())?;
        if data.schema_version != 2 {
            return Err("The terminal app requires an independent host configuration".into());
        }
        Ok(Self {
            config,
            state: data.state_dir,
        })
    }
    pub fn request(&self, value: Value) -> Result<Value, String> {
        let path = self.state.join("node.sock");
        let meta = fs::symlink_metadata(&path)
            .map_err(|_| "Node offline — select Go online to start it")?;
        let uid = unsafe { libc::geteuid() };
        if !meta.file_type().is_socket() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
            return Err("Node socket must be an owner-only socket".into());
        }
        let mut stream =
            UnixStream::connect(path).map_err(|_| "Cannot connect to the local node")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(27)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        let owner_matches = {
            let mut peer_uid: libc::uid_t = 0;
            let mut peer_gid: libc::gid_t = 0;
            (unsafe { libc::getpeereid(stream.as_raw_fd(), &mut peer_uid, &mut peer_gid) }) == 0
                && peer_uid == uid
        };
        #[cfg(not(target_os = "macos"))]
        let owner_matches = {
            let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
            let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
            (unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut cred as *mut libc::ucred).cast(),
                    &mut size,
                )
            }) == 0
                && cred.uid == uid
        };
        if !owner_matches {
            return Err("Unexpected local node owner".into());
        }
        let mut data = serde_json::to_vec(&value).map_err(|_| "Invalid node request")?;
        if data.len() > 16384 {
            return Err("Local request exceeds 16 KiB".into());
        }
        data.push(b'\n');
        stream
            .write_all(&data)
            .map_err(|_| "Node request unconfirmed; refresh before retrying")?;
        let mut response = Vec::new();
        BufReader::new(stream)
            .take(2 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut response)
            .map_err(|_| "Node reply unconfirmed; refresh before retrying")?;
        if response.len() > 2 * 1024 * 1024 || response.last() != Some(&b'\n') {
            return Err("Invalid or oversized node response".into());
        }
        let response: Value =
            serde_json::from_slice(&response).map_err(|_| "Invalid node response")?;
        if response["ok"] != true {
            return Err(response["error"]
                .as_str()
                .unwrap_or("Node rejected the operation")
                .to_owned());
        }
        let result = response.get("result").ok_or("Missing node result")?.clone();
        if value["op"] == "status" && result["api_version"] != 1 {
            return Err("Unsupported local node API version".into());
        }
        Ok(result)
    }
    pub fn online(&self, online: bool) -> Result<Value, String> {
        let expected = home()?.join(".config/wispnet-node/host.toml");
        if self.config
            != fs::canonicalize(expected).map_err(|_| "Installed host configuration unavailable")?
        {
            return Err(
                "Online control is available only for the installed host configuration".into(),
            );
        }
        #[cfg(target_os = "macos")]
        {
            let domain = format!("gui/{}", unsafe { libc::geteuid() });
            let service = format!("{domain}/app.wispnet.node");
            let loaded = command(
                Command::new("launchctl").args(["print", &service]),
                None,
                10,
            )
            .is_ok();
            if online && loaded {
                command(
                    Command::new("launchctl").args(["kickstart", &service]),
                    None,
                    35,
                )?;
            } else if online {
                let plist = home()?.join("Library/LaunchAgents/app.wispnet.node.plist");
                command(
                    Command::new("launchctl")
                        .args(["bootstrap", &domain])
                        .arg(plist),
                    None,
                    35,
                )?;
            } else if loaded {
                command(
                    Command::new("launchctl").args(["bootout", &service]),
                    None,
                    35,
                )?;
            }
        }
        #[cfg(not(target_os = "macos"))]
        command(
            Command::new("systemctl").args([
                "--user",
                if online { "start" } else { "stop" },
                "wispnet-host.service",
            ]),
            None,
            35,
        )?;
        if !online {
            return Ok(json!({"online":false}));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(status) = self.request(json!({"op":"status"})) {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err("Node started but its API is not ready".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    pub fn check_rename(&self, name: &str) -> Result<(), String> {
        if !wispnet_core::protocol::valid_service_id(name) {
            return Err("Invalid device ID".into());
        }
        let current = Config::load(&self.config).map_err(|e| e.to_string())?;
        if current.device_name == name {
            return Ok(());
        }
        let expected = fs::canonicalize(home()?.join(".config/wispnet-node/host.toml"))
            .map_err(|_| "Installed host configuration unavailable")?;
        if self.config != expected {
            return Err("Rename requires the installed node configuration".into());
        }
        Ok(())
    }
    pub fn rename_device(&self, name: &str) -> Result<(), String> {
        self.check_rename(name)?;
        if Config::load(&self.config)
            .map_err(|e| e.to_string())?
            .device_name
            == name
        {
            return Ok(());
        }
        let source = fs::read_to_string(&self.config).map_err(|e| e.to_string())?;
        let updated = renamed_config(&source, name)?;
        let mut file = tempfile::NamedTempFile::new_in(
            self.config.parent().ok_or("Missing config directory")?,
        )
        .map_err(|e| e.to_string())?;
        file.write_all(updated.as_bytes())
            .map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(&self.config).map_err(|e| e.to_string())?;
        self.online(false)?;
        self.online(true)?;
        self.request(json!({"op":"enable"}))?;
        Ok(())
    }
    // Reuse the desktop's tested durable cross-host journal, locking and receipts.
    pub fn owner(&self, args: &[String]) -> Result<Value, String> {
        let helper = home()?.join(".local/bin/wispnet-node");
        let mut cmd = Command::new(helper);
        cmd.arg("--config").arg(&self.config).args(args);
        let output = command(&mut cmd, None, 90)?;
        serde_json::from_slice(&output).map_err(|_| "Invalid owner-operation reply".into())
    }
    pub fn owner_start(&self, plan: &Value) -> Result<Value, String> {
        let mut file = tempfile::Builder::new()
            .prefix("owner-plan-")
            .tempfile_in(&self.state)
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut file, plan).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        self.owner(&["--batch".into(), file.path().to_string_lossy().into_owned()])
    }
    pub fn names(&self) -> Value {
        let mut names = read_private(&self.state.join("desktop-peer-names.json"), 65536)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if let Some(ids) = read_private(&self.state.join("desktop-flutter.json"), 2 * 1024 * 1024)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v["preferences"]["device_ids"].as_object().cloned())
        {
            for (peer, name) in ids {
                if name
                    .as_str()
                    .is_some_and(wispnet_core::protocol::valid_service_id)
                {
                    names[peer] = name;
                }
            }
        }
        names
    }
    pub fn save_pair(&self, host: &str, name: &str) -> Result<(), String> {
        if host.len() != 64 || !host.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid host ID".into());
        }
        let mut names = self.names();
        names[host] = json!(name);
        let mut file = tempfile::NamedTempFile::new_in(&self.state).map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut file, &names).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(self.state.join("desktop-peer-names.json"))
            .map_err(|_| "Pairing sent; cannot save display name")?;
        File::open(&self.state)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        match fs::remove_file(
            self.state
                .join("desktop-catalogs")
                .join(format!("{host}.json")),
        ) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Pairing sent; cannot reset display cache".into()),
        }
    }
}
fn renamed_config(source: &str, name: &str) -> Result<String, String> {
    if !wispnet_core::protocol::valid_service_id(name) {
        return Err("Invalid device ID".into());
    }
    let mut value: toml::Value =
        toml::from_str(source).map_err(|_| "Invalid host configuration")?;
    let table = value.as_table_mut().ok_or("Invalid host configuration")?;
    table.remove("device_name");
    table.insert("device_id".into(), toml::Value::String(name.into()));
    toml::to_string(&value).map_err(|_| "Cannot serialize host configuration".into())
}
pub fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is unset; pass --config".into())
}
pub fn read_private(path: &Path, limit: u64) -> Result<String, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "Cannot open private file")?;
    let meta = file.metadata().map_err(|_| "Cannot inspect private file")?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.len() > limit
    {
        return Err("Use a bounded, owner-only regular file (chmod 600)".into());
    }
    let mut text = String::new();
    file.take(limit + 1)
        .read_to_string(&mut text)
        .map_err(|_| "Invalid private text file")?;
    if text.len() as u64 > limit {
        return Err("Private file too large".into());
    }
    Ok(text)
}
// Bounded child lifetime/output; nothing is interpreted by a shell.
pub fn command(cmd: &mut Command, input: Option<&[u8]>, timeout: u64) -> Result<Vec<u8>, String> {
    let mut out = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut err = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut child = cmd
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(out.try_clone().map_err(|e| e.to_string())?)
        .stderr(err.try_clone().map_err(|e| e.to_string())?)
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(bytes) = input
        && child
            .stdin
            .take()
            .ok_or("Missing child input")?
            .write_all(bytes)
            .is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Child input failed".into());
    }
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline
            || out.metadata().map_err(|e| e.to_string())?.len() > 2 * 1024 * 1024
            || err.metadata().map_err(|e| e.to_string())?.len() > 65536
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(
                "Operation timed out or exceeded its output limit; acknowledgement unconfirmed"
                    .into(),
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    use std::io::{Seek, SeekFrom};
    out.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    err.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    out.take(2 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if !status.success() {
        let mut text = String::new();
        err.take(65536)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        return Err(if text.trim().is_empty() {
            "Command failed".into()
        } else {
            text.trim().to_owned()
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    #[test]
    fn rename_preserves_transport_and_removes_legacy_alias() {
        let source = "schema_version=2\ndevice_name='old'\nstate_dir='/tmp/state'\nlog_level='info'\n[transport]\nbind_addr='127.0.0.1:0'\n";
        let updated = renamed_config(source, "new-device").unwrap();
        let value: toml::Value = toml::from_str(&updated).unwrap();
        assert_eq!(value["device_id"].as_str(), Some("new-device"));
        assert!(value.get("device_name").is_none());
        assert_eq!(
            value["transport"]["bind_addr"].as_str(),
            Some("127.0.0.1:0")
        );
        assert!(renamed_config(source, "Invalid name").is_err());
    }
    #[test]
    fn owner_socket_and_api_version_are_verified_before_trusting_a_response() {
        let root = tempfile::tempdir().unwrap();
        let api = Api {
            config: root.path().join("host.toml"),
            state: root.path().to_owned(),
        };
        let path = root.path().join("node.sock");
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(
            api.request(json!({"op":"status"}))
                .unwrap_err()
                .contains("owner-only")
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&request).unwrap()["op"],
                "status"
            );
            stream
                .write_all(b"{\"ok\":true,\"result\":{\"api_version\":99}}\n")
                .unwrap();
        });
        assert!(
            api.request(json!({"op":"status"}))
                .unwrap_err()
                .contains("Unsupported")
        );
        server.join().unwrap();
    }
}
