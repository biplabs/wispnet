//! Interactive terminal over the running node's private, owner-only API.
use crate::tui::api::Api;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    io::{self, IsTerminal, Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

pub const HELP: &str = "Shell: wispnet shell DEVICE_ID [--config /absolute/path/host.toml] [--service shell] [--session main]\n       wispnet shell --host PUBLIC_ID [--incarnation SERVICE_INCARNATION] [--config PATH]\nCtrl+] detaches; exit ends the remote shell. Sessions survive disconnects.";

pub fn run_args(args: &[OsString]) -> Result<u32, String> {
    let mut target = None;
    let mut config = None;
    let mut service = "shell".to_owned();
    let mut session = "main".to_owned();
    let mut incarnation = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let arg = arg.to_str().ok_or(HELP)?;
        if matches!(arg, "--help" | "-h") {
            println!("{HELP}");
            return Ok(0);
        }
        if !arg.starts_with('-') && target.is_none() {
            target = Some(arg.to_owned());
            continue;
        }
        let value = args.next().and_then(|s| s.to_str()).ok_or(HELP)?;
        match arg {
            "--host" if target.is_none() => target = Some(value.to_owned()),
            "--config" if config.is_none() => config = Some(PathBuf::from(value)),
            "--service" => service = value.into(),
            "--session" => session = value.into(),
            "--incarnation" => incarnation = Some(value.to_owned()),
            _ => return Err(HELP.into()),
        }
    }
    let target = target.ok_or(HELP)?;
    let config = config.unwrap_or(crate::tui::api::home()?.join(".config/wispnet-node/host.toml"));
    let api = Api::new(config)?;
    let status = api.request(json!({"op":"status"}))?;
    let host = resolve_host(&target, &status, &api.names())?;
    let catalog = if status["host"]["host"] == host {
        status["host"].clone()
    } else {
        api.request(json!({"op":"catalog","host":host}))?
    };
    let selected=catalog["services"].as_array().and_then(|ss|ss.iter().find(|s|s["id"]==service&&s["kind"]=="shell")).ok_or("Shell service unavailable or not permitted; enable Shell on the target and grant this device access")?;
    if incarnation
        .as_ref()
        .is_some_and(|i| selected["incarnation"] != *i)
    {
        return Err("Shell service changed; refresh before opening".into());
    }
    run_selected(
        &api,
        json!({"host":host,"service":service,"incarnation":selected["incarnation"],"session":session}),
    )
}
fn resolve_host(target: &str, status: &Value, names: &Value) -> Result<String, String> {
    if target.parse::<wispnet_core::identity::EndpointId>().is_ok() {
        return Ok(target.into());
    }
    let mut matches = std::collections::BTreeSet::new();
    if (status["host_name"] == target || target == "localhost")
        && let Some(host) = status["host"]["host"].as_str()
    {
        matches.insert(host.to_owned());
    }
    if let Some(names) = names.as_object() {
        for (id, name) in names {
            if name == target {
                matches.insert(id.clone());
            }
        }
    }
    if let Some(clients) = status["host"]["clients"].as_array() {
        for client in clients {
            if client["name"] == target
                && let Some(id) = client["peer"].as_str()
            {
                matches.insert(id.to_owned());
            }
        }
    }
    if matches.len() != 1 {
        return Err("Device ID unknown or ambiguous; use its full public ID".into());
    }
    Ok(matches.into_iter().next().unwrap())
}
struct Restore {
    raw: bool,
    api: Api,
    handle: String,
    signals: Vec<signal_hook::SigId>,
}
impl Drop for Restore {
    fn drop(&mut self) {
        if self.raw {
            let _ = crossterm::terminal::disable_raw_mode();
        }
        for signal in self.signals.drain(..) {
            signal_hook::low_level::unregister(signal);
        }
        let _ = self
            .api
            .request(json!({"op":"shell_close","session":self.handle}));
    }
}
fn terminal_size() -> Option<(u16, u16)> {
    // A new terminal can report 0x0 before its window is mapped. Keep using a
    // valid size until the terminal has dimensions the remote PTY can accept.
    crossterm::terminal::size()
        .ok()
        .filter(|(cols, rows)| (2..=1000).contains(cols) && (2..=1000).contains(rows))
}
pub(crate) fn run_selected(api: &Api, mut request: Value) -> Result<u32, String> {
    let raw = io::stdin().is_terminal() && io::stdout().is_terminal();
    let (cols, rows) = terminal_size().unwrap_or((80, 24));
    request["op"] = json!("shell_open");
    request["cols"] = json!(cols);
    request["rows"] = json!(rows);
    let opened = api.request(request)?;
    let handle = opened["session"]
        .as_str()
        .ok_or("Missing terminal session")?
        .to_owned();
    let mut restore = Restore {
        raw: false,
        api: api.clone(),
        handle: handle.clone(),
        signals: vec![],
    };
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        restore
            .signals
            .push(signal_hook::flag::register(signal, stop.clone()).map_err(|e| e.to_string())?);
    }
    if raw {
        crossterm::terminal::enable_raw_mode().map_err(|e| e.to_string())?;
        restore.raw = true;
        eprintln!("Connected to remote shell. Ctrl+] detaches.\r");
    }
    let (tx, rx) = mpsc::sync_channel::<io::Result<Vec<u8>>>(16);
    // poll(2) permits cleanup while stdin is idle, including returning to the TUI.
    let input_stop = Arc::new(AtomicBool::new(false));
    let input_done = input_stop.clone();
    let input = std::thread::spawn(move || {
        let mut stdin = io::stdin();
        while !input_done.load(Ordering::Relaxed) {
            let mut fd = libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut fd, 1, 100) };
            if ready < 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                let _ = tx.send(Err(io::Error::last_os_error()));
                break;
            }
            if ready == 0 {
                continue;
            }
            let mut bytes = vec![0; 8192];
            let outcome = stdin.read(&mut bytes).map(|n| {
                bytes.truncate(n);
                bytes
            });
            let end = outcome.as_ref().map_or(true, Vec::is_empty);
            // Avoid a permanently blocked input worker after terminal detach.
            let mut outcome = outcome;
            loop {
                match tx.try_send(outcome) {
                    Ok(()) => break,
                    Err(mpsc::TrySendError::Full(v)) => {
                        if input_done.load(Ordering::Relaxed) {
                            return;
                        }
                        outcome = v;
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return,
                }
            }
            if end {
                break;
            }
        }
    });
    let result = (|| {
        let mut last_size = (cols, rows);
        let mut stdout = io::stdout();
        let mut eof = false;
        let mut input_error: Option<(String, std::time::Instant)> = None;
        while !stop.load(Ordering::Relaxed) {
            // Poll output between stdin chunks. A paste or pipe must not fill
            // both directions' queues while the remote PTY waits for a reader.
            for _ in 0..1 {
                if input_error.is_some() {
                    break;
                }
                let Ok(bytes) = rx.try_recv() else {
                    break;
                };
                let mut bytes = bytes.map_err(|e| e.to_string())?;
                if raw && let Some(end) = bytes.iter().position(|b| *b == 29) {
                    bytes.truncate(end);
                    if !bytes.is_empty() {
                        api.request(json!({"op":"shell_input","session":handle,"data_b64":STANDARD.encode(bytes)}))?;
                    }
                    return Ok(0);
                }
                if bytes.is_empty() {
                    if eof {
                        continue;
                    }
                    eof = true;
                    bytes.push(4);
                }
                if let Err(error) = api.request(
                    json!({"op":"shell_input","session":handle,"data_b64":STANDARD.encode(bytes)}),
                ) {
                    input_error = Some((error, std::time::Instant::now()));
                }
            }
            if raw
                && let Some((cols, rows)) = terminal_size()
                && last_size != (cols, rows)
            {
                api.request(json!({"op":"shell_resize","session":handle,"cols":cols,"rows":rows}))?;
                last_size = (cols, rows);
            }
            let polled = api.request(json!({"op":"shell_poll","session":handle}))?;
            for event in polled["events"].as_array().into_iter().flatten() {
                match event["type"].as_str() {
                    Some("output" | "scrollback") => {
                        let bytes = STANDARD
                            .decode(
                                event["data_b64"]
                                    .as_str()
                                    .ok_or("Invalid terminal output")?,
                            )
                            .map_err(|_| "Invalid terminal output")?;
                        stdout.write_all(&bytes).map_err(|e| e.to_string())?;
                    }
                    Some("exit") => {
                        stdout.flush().map_err(|e| e.to_string())?;
                        return Ok(event["code"].as_u64().unwrap_or(1) as u32);
                    }
                    Some("error") => {
                        return Err(event["message"]
                            .as_str()
                            .unwrap_or("Terminal disconnected")
                            .to_owned());
                    }
                    _ => {}
                }
            }
            stdout.flush().map_err(|e| e.to_string())?;
            if polled["state"] == "disconnected" {
                return Err("Terminal disconnected; run the same command to resume".into());
            }
            if let Some((error, when)) = &input_error
                && when.elapsed() > Duration::from_secs(2)
            {
                return Err(error.clone());
            }
            if polled["events"].as_array().is_none_or(Vec::is_empty) {
                std::thread::sleep(Duration::from_millis(16));
            }
        }
        Ok(0)
    })();
    input_stop.store(true, Ordering::Relaxed);
    drop(rx);
    let _ = input.join();
    if raw {
        let _ = io::stdout().write_all(b"\r\n");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_unambiguous_and_public_ids_remain_authority() {
        let own = "a".repeat(64);
        let remote = "b".repeat(64);
        let status = json!({"host_name":"desktop","host":{"host":own,"clients":[]}});
        assert_eq!(resolve_host("desktop", &status, &json!({})).unwrap(), own);
        assert_eq!(
            resolve_host("phone", &status, &json!({remote.clone():"phone"})).unwrap(),
            remote
        );
        assert!(resolve_host("desktop", &status, &json!({remote:"desktop"})).is_err());
        assert!(resolve_host("missing", &status, &json!({})).is_err());
    }
}
