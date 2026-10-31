//! Noninteractive file helpers; bytes stay inside the node's native QUIC stream.
use crate::tui::api::Api;
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const HELP: &str = "Files: wispnet files <put|get> DEVICE WORKSPACE LOCAL_PATH REMOTE_PATH [--config PATH] [--no-resume]\n       wispnet files status TRANSFER [--config PATH]\n       wispnet files cancel TRANSFER [--config PATH]\nTransfers resume by default. Ctrl+C pauses the transfer and retains resumable data.\nProgress is JSON on stderr; completion or a typed transfer error is JSON on stdout.";

pub fn error_result(error: &str) -> Value {
    let (code, message) = error
        .split_once(": ")
        .filter(|(code, _)| {
            !code.is_empty() && code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
        })
        .unwrap_or(("UNAVAILABLE", error));
    json!({"state":"failed","error":{"code":code,"message":message}})
}

struct Options {
    action: String,
    positional: Vec<String>,
    config: PathBuf,
    resume: bool,
}
fn options(args: &[OsString]) -> Result<Option<Options>, String> {
    let mut positional = Vec::new();
    let mut config = None;
    let mut resume = true;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let arg = arg.to_str().ok_or("Files arguments must be valid UTF-8")?;
        match arg {
            "--help" | "-h" => return Ok(None),
            "--config" if config.is_none() => {
                config = Some(PathBuf::from(args.next().ok_or(HELP)?))
            }
            "--no-resume" => resume = false,
            _ if !arg.starts_with('-') => positional.push(arg.to_owned()),
            _ => return Err(HELP.into()),
        }
    }
    let action = positional.first().cloned().ok_or(HELP)?;
    positional.remove(0);
    if !match action.as_str() {
        "put" | "get" => positional.len() == 4,
        "status" | "cancel" => positional.len() == 1,
        _ => false,
    } {
        return Err(HELP.into());
    }
    let config = config.unwrap_or(crate::tui::api::home()?.join(".config/wispnet-node/host.toml"));
    Ok(Some(Options {
        action,
        positional,
        config,
        resume,
    }))
}

pub fn run_args(args: &[OsString]) -> Result<u32, String> {
    let Some(options) = options(args).map_err(|error| format!("INVALID_ARGUMENT: {error}"))? else {
        println!("{HELP}");
        return Ok(0);
    };
    let api = Api::new(options.config)?;
    if matches!(options.action.as_str(), "status" | "cancel") {
        let value = api.request(
            json!({"op":format!("file_{}", options.action),"transfer":options.positional[0]}),
        )?;
        println!(
            "{}",
            serde_json::to_string(&value).map_err(|_| "Invalid Files result")?
        );
        return Ok(if value["state"] == "failed" { 1 } else { 0 });
    }
    let status = api.request(json!({"op":"status"}))?;
    let host = resolve_host(&options.positional[0], &status, &api.names())?;
    let catalog = if status["host"]["host"] == host {
        status["host"].clone()
    } else {
        api.request(json!({"op":"catalog","host":host}))?
    };
    let service = catalog["services"]
        .as_array()
        .and_then(|services| {
            services
                .iter()
                .find(|s| s["id"] == "files" && s["kind"] == "files")
        })
        .ok_or("PERMISSION_DENIED: Files service unavailable or not granted; configure files-access on the target")?;
    let path = absolute_path(Path::new(&options.positional[2]))?;
    let started = api.request(json!({"op":format!("file_{}", options.action),"host":host,"service":"files","incarnation":service["incarnation"],"workspace":options.positional[1],"local_path":path,"remote_path":options.positional[3],"resume":options.resume}))?;
    let transfer = started["transfer"]
        .as_str()
        .ok_or("Missing Files transfer handle")?
        .to_owned();
    let cancelled = Arc::new(AtomicBool::new(false));
    let signals = vec![
        signal_hook::flag::register(signal_hook::consts::SIGINT, cancelled.clone())
            .map_err(|_| "Cannot register cancellation")?,
        signal_hook::flag::register(signal_hook::consts::SIGTERM, cancelled.clone())
            .map_err(|_| "Cannot register cancellation")?,
    ];
    struct Signals(Vec<signal_hook::SigId>);
    impl Drop for Signals {
        fn drop(&mut self) {
            for id in self.0.drain(..) {
                signal_hook::low_level::unregister(id);
            }
        }
    }
    let _signals = Signals(signals);
    let mut requested_cancel = false;
    let mut previous = Value::Null;
    loop {
        let op = if cancelled.load(Ordering::Relaxed) && !requested_cancel {
            requested_cancel = true;
            "file_cancel"
        } else {
            "file_status"
        };
        let status = api.request(json!({"op":op,"transfer":transfer}))?;
        if status != previous {
            eprintln!(
                "{}",
                serde_json::to_string(&status).map_err(|_| "Invalid Files progress")?
            );
            previous = status.clone();
        }
        match status["state"].as_str() {
            Some("complete") => {
                println!(
                    "{}",
                    serde_json::to_string(&status).map_err(|_| "Invalid Files result")?
                );
                return Ok(0);
            }
            Some("failed") => {
                println!(
                    "{}",
                    serde_json::to_string(&status).map_err(|_| "Invalid Files result")?
                );
                return Ok(1);
            }
            Some("cancelled") => {
                println!(
                    "{}",
                    serde_json::to_string(&status).map_err(|_| "Invalid Files result")?
                );
                return Ok(130);
            }
            Some("running") => {}
            _ => return Err("Invalid Files transfer state".into()),
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|_| "Cannot inspect working directory")?
            .join(path)
    };
    let name = path
        .file_name()
        .ok_or("INVALID_ARGUMENT: Local path must name a file")?;
    let parent = std::fs::canonicalize(
        path.parent()
            .ok_or("INVALID_ARGUMENT: Local path must name a file")?,
    )
    .map_err(|_| "NOT_FOUND: Local parent directory is unavailable")?;
    Ok(parent.join(name))
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
        return Err(
            "INVALID_ARGUMENT: Device ID unknown or ambiguous; use its full public ID".into(),
        );
    }
    Ok(matches.into_iter().next().unwrap())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }
    #[test]
    fn command_order_and_resume_default_are_explicit() {
        let result = options(&args(&[
            "put",
            "device",
            "training",
            "dataset.npz",
            "jobs/1/data",
            "--config",
            "/tmp/config",
        ]))
        .unwrap()
        .unwrap();
        assert!(result.resume);
        assert_eq!(result.positional[2], "dataset.npz");
        assert_eq!(result.positional[3], "jobs/1/data");
        let result = options(&args(&[
            "get",
            "device",
            "training",
            "output",
            "remote",
            "--config",
            "/tmp/config",
            "--no-resume",
        ]))
        .unwrap()
        .unwrap();
        assert!(!result.resume);
        assert!(
            options(&args(&[
                "get",
                "device",
                "training",
                "output",
                "--config",
                "/tmp/config"
            ]))
            .is_err()
        );
    }
}
