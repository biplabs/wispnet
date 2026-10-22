//! Unprivileged runtime report. The root helper independently verifies every socket.
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
};

pub(crate) struct Reporter {
    path: PathBuf,
    last: Vec<std::net::SocketAddr>,
}
impl Reporter {
    pub(crate) fn new() -> Option<Self> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let uid = unsafe { libc::geteuid() };
        let unit = format!(
            "0::/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/wispnet-host.service"
        );
        if std::fs::read_to_string("/proc/self/cgroup").ok()?.trim() != unit {
            return None;
        }

        let dir = PathBuf::from(format!("/run/user/{uid}"));
        let m = std::fs::symlink_metadata(&dir).ok()?;
        if !m.is_dir() || m.uid() != uid || m.mode() & 0o077 != 0 {
            return None;
        }
        Some(Self {
            path: dir.join("wispnet-firewall.json"),
            last: Vec::new(),
        })
    }
    pub(crate) fn update(&mut self, endpoint: &iroh::Endpoint) -> std::io::Result<()> {
        let ports: Vec<_> = endpoint
            .bound_sockets()
            .into_iter()
            .filter(|a| !a.ip().is_loopback() && a.port() >= 1024)
            .collect();
        if self.last == ports && self.path.exists() {
            return Ok(());
        }
        let start = std::fs::read_to_string("/proc/self/stat")?;
        let start = start
            .rsplit_once(')')
            .and_then(|(_, s)| s.split_whitespace().nth(19))
            .ok_or_else(|| std::io::Error::other("Missing process start time"))?;
        let report = serde_json::json!({"version":1,"pid":std::process::id(),"start_time":start,
            "ports":ports.iter().map(|a|serde_json::json!({"family":if a.is_ipv4(){"ipv4"}else{"ipv6"},"port":a.port()})).collect::<Vec<_>>()});
        let tmp = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(report.to_string().as_bytes())?;
            std::fs::rename(&tmp, &self.path)
        })();
        let _ = std::fs::remove_file(&tmp);
        result?;
        self.last = ports;
        Ok(())
    }
}
impl Drop for Reporter {
    fn drop(&mut self) {
        if std::fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .is_some_and(|v| v["pid"] == std::process::id())
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
