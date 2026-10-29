//! Optional public-account commands over the existing owner-local node API.
use crate::tui::api::{self, Api};
use serde_json::{Value, json};
use std::{ffi::OsString, io::Write, path::PathBuf, process::Command, time::Duration};
use tokio::time::{Instant, timeout_at};
use url::Url;

pub const HELP: &str = "Public account (uses the running node; private access needs no login):
  wispnet login [--config PATH] [--url HTTPS_ORIGIN] [--no-browser]
                [--timeout SECONDS] [--restart] [--json]
  wispnet account [status] [--config PATH] [--json]
  wispnet logout [--config PATH] [--json]
Login opens GitHub sign-in through Command Center and links this computer.
The default wait is 600 seconds; --timeout accepts 1..600. Retry resumes an
existing request; --restart creates a new code. --no-browser prints the URL/code.
--json writes one final JSON object to stdout, with sign-in progress on stderr.
Logout stops managed publications here and attempts remote account unlinking.
Use --config to select a different host configuration.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Login,
    Logout,
    Status,
}
#[derive(Debug)]
struct Options {
    operation: Operation,
    config: Option<PathBuf>,
    url: Option<String>,
    no_browser: bool,
    restart: bool,
    json: bool,
    timeout: Duration,
    help: bool,
}
pub fn handles(command: &str) -> bool {
    matches!(command, "login" | "logout" | "account")
}
impl Options {
    fn parse(args: &[OsString]) -> Result<Self, String> {
        let operation = match args.first().and_then(|s| s.to_str()) {
            Some("login") => Operation::Login,
            Some("logout") => Operation::Logout,
            Some("account") => Operation::Status,
            _ => return Err(HELP.into()),
        };
        let mut options = Self {
            operation,
            config: None,
            url: None,
            no_browser: false,
            restart: false,
            json: false,
            timeout: Duration::from_secs(600),
            help: false,
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut args = args[1..].iter().peekable();
        if operation == Operation::Status && args.peek().is_some_and(|s| *s == "status") {
            args.next();
        }
        while let Some(arg) = args.next() {
            let arg = arg.to_str().ok_or(HELP)?;
            if !seen.insert(arg) {
                return Err(format!("Duplicate option: {arg}\n{HELP}"));
            }
            match arg {
                "--help" | "-h" => options.help = true,
                "--json" => options.json = true,
                "--no-browser" if operation == Operation::Login => options.no_browser = true,
                "--restart" if operation == Operation::Login => options.restart = true,
                "--config" => options.config = Some(PathBuf::from(args.next().ok_or(HELP)?)),
                "--url" if operation == Operation::Login => {
                    let value = args.next().and_then(|s| s.to_str()).ok_or(HELP)?;
                    options.url = Some(origin(value)?);
                }
                "--timeout" if operation == Operation::Login => {
                    let seconds = args
                        .next()
                        .and_then(|s| s.to_str())
                        .and_then(|s| s.parse::<u64>().ok())
                        .filter(|s| (1..=600).contains(s))
                        .ok_or("Login timeout must be between 1 and 600 seconds")?;
                    options.timeout = Duration::from_secs(seconds);
                }
                _ => return Err(format!("Invalid account option: {}\n{HELP}", clean(arg))),
            }
        }
        Ok(options)
    }
}

fn origin(value: &str) -> Result<String, String> {
    let url = Url::parse(value).map_err(|_| "Invalid Command Center URL")?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Command Center requires an HTTPS origin (HTTP is local-development only)".into(),
        );
    }
    Ok(url.origin().ascii_serialization())
}

fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(512)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pending {
    code: String,
    url: String,
}
impl Pending {
    fn read(value: &Value, base: &str) -> Result<Self, String> {
        let code = value["user_code"]
            .as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
            .ok_or("Invalid account verification code")?;
        let raw = value["verification_url"]
            .as_str()
            .filter(|s| s.len() <= 4096)
            .ok_or("Invalid account verification URL")?;
        let url = Url::parse(raw).map_err(|_| "Invalid account verification URL")?;
        let base = Url::parse(&origin(base)?).map_err(|_| "Invalid Command Center URL")?;
        let query: Vec<_> = url.query_pairs().collect();
        if url.origin() != base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/connect"
            || url.fragment().is_some()
            || query.len() != 1
            || query[0].0 != "code"
            || query[0].1 != code
        {
            return Err("Invalid account verification destination".into());
        }
        Ok(Self {
            code: code.into(),
            url: url.to_string(),
        })
    }
}

// Never return raw account or node JSON: future fields may contain credentials.
fn account(value: &Value) -> Value {
    let mut result = serde_json::Map::new();
    for field in ["id", "name"] {
        if let Some(text) = value[field].as_str() {
            result.insert(field.into(), json!(clean(text)));
        }
    }
    if result.is_empty() {
        Value::Null
    } else {
        Value::Object(result)
    }
}

fn status(value: &Value) -> Result<Value, String> {
    let base = origin(
        value["base_url"]
            .as_str()
            .ok_or("Missing Command Center URL")?,
    )?;
    let linked = value["linked"]
        .as_bool()
        .ok_or("Missing account link status")?;
    let pending = if !linked && !value["pending"].is_null() {
        let pending = Pending::read(&value["pending"], &base)?;
        json!({"verification_url":pending.url,"user_code":pending.code})
    } else {
        Value::Null
    };
    let mut result = json!({"state":if linked {"linked"} else if !pending.is_null() {"pending"} else {"unlinked"},"base_url":base,"linked":linked,"account":account(&value["account"]),"pending":pending});
    if let Some(revocation) = value["remote_revocation"].as_str()
        && matches!(revocation, "confirmed" | "unconfirmed" | "not_requested")
    {
        result["remote_revocation"] = json!(revocation);
        if revocation == "unconfirmed" {
            result["warning"] = json!(
                "Publications stopped on this computer. Remote revocation could not be confirmed; check Command Center to disconnect this computer."
            );
        }
    }
    Ok(result)
}

pub(crate) fn lines(value: &Value) -> Result<Vec<String>, String> {
    let value = status(value)?;
    let mut lines = vec![if value["linked"] == true {
        format!(
            "Signed in as {}",
            value["account"]["name"]
                .as_str()
                .unwrap_or("Wispnet account")
        )
    } else if value["state"] == "pending" {
        "Account sign-in is pending. Run wispnet login to continue.".into()
    } else {
        "Not signed in. Private device access remains available.".into()
    }];
    if let Some(id) = value["account"]["id"].as_str() {
        lines.push(format!("Account ID: {id}"));
    }
    lines.push(format!(
        "Command Center: {}",
        value["base_url"].as_str().unwrap()
    ));
    if let Some(warning) = value["warning"].as_str() {
        lines.push(warning.into());
    }
    Ok(lines)
}

async fn request(api: &Api, value: Value) -> Result<Value, String> {
    let api = api.clone();
    tokio::task::spawn_blocking(move || api.request(value))
        .await
        .map_err(|_| "Local account request failed".to_owned())?
        .map_err(|error| clean(&error))
}

async fn poll(api: &Api, pending: &Pending, base: &str) -> Result<Option<Value>, String> {
    let current = request(api, json!({"op":"command_center_status"})).await?;
    if origin(current["base_url"].as_str().unwrap_or(""))? != base {
        return Err(
            "Command Center changed in another Wispnet window; run wispnet login again".into(),
        );
    }
    if current["linked"] == true {
        return status(&current).map(Some);
    }
    if Pending::read(&current["pending"], base)? != *pending {
        return Err(
            "Sign-in request changed in another Wispnet window; run wispnet login again".into(),
        );
    }
    let response = request(api, json!({"op":"command_center_poll","expected_user_code":pending.code,"expected_verification_url":pending.url})).await;
    match response {
        Ok(value) if value["state"] == "linked" => Ok(Some(
            json!({"state":"linked","linked":true,"base_url":base,"account":account(&value["account"]),"pending":null}),
        )),
        Ok(value) if value["state"] == "pending" => Ok(None),
        Ok(_) => Err("Invalid account sign-in response".into()),
        Err(error) => {
            // Another UI may have consumed the one-time device record, or the
            // node may have saved a link before the IPC reply was interrupted.
            if let Ok(current) = request(api, json!({"op":"command_center_status"})).await
                && current["linked"] == true
                && origin(current["base_url"].as_str().unwrap_or(""))? == base
            {
                return status(&current).map(Some);
            }
            Err(format!(
                "Sign-in could not be confirmed: {error}. Retry wispnet login; use --restart if the request expired"
            ))
        }
    }
}

async fn login(
    api: &Api,
    options: &Options,
    progress: &mut dyn Write,
    open: impl FnOnce(&str) -> Result<(), String> + Send + 'static,
) -> Result<Value, String> {
    let current = request(api, json!({"op":"command_center_status"})).await?;
    let current = status(&current)?;
    let base = options
        .url
        .as_deref()
        .unwrap_or(current["base_url"].as_str().unwrap());
    if current["linked"] == true {
        if base != current["base_url"].as_str().unwrap() {
            return Err("Log out before linking this computer to another Command Center".into());
        }
        return Ok(current);
    }
    let pending = if !options.restart
        && base == current["base_url"].as_str().unwrap()
        && !current["pending"].is_null()
    {
        Pending::read(&current["pending"], base)?
    } else {
        let started = request(api, json!({"op":"command_center_link","base_url":base})).await?;
        if started["state"] != "pending" {
            return Err("Invalid account sign-in response".into());
        }
        Pending::read(&started, base)?
    };
    writeln!(progress, "Open {}", pending.url).map_err(|e| e.to_string())?;
    writeln!(progress, "Verification code: {}", pending.code).map_err(|e| e.to_string())?;
    writeln!(
        progress,
        "Sign in with GitHub and approve this computer. Ctrl+C stops waiting."
    )
    .map_err(|e| e.to_string())?;
    progress.flush().map_err(|e| e.to_string())?;
    if !options.no_browser {
        let url = pending.url.clone();
        if let Err(error) = tokio::task::spawn_blocking(move || open(&url))
            .await
            .map_err(|_| "Browser handoff failed".to_owned())?
        {
            writeln!(progress, "{} Open the URL above manually.", clean(&error))
                .map_err(|e| e.to_string())?;
        }
    }
    loop {
        if let Some(linked) = poll(api, &pending, base).await? {
            return Ok(linked);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

fn open_browser(url: &str) -> Result<(), String> {
    if !cfg!(target_os = "macos")
        && std::env::var_os("DISPLAY").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
    {
        return Err("No desktop browser session detected.".into());
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    api::command(Command::new(opener).arg(url), None, 10)
        .map(|_| ())
        .map_err(|_| "Could not open the system browser.".into())
}

async fn bounded_login(
    api: &Api,
    options: &Options,
    progress: &mut dyn Write,
    open: impl FnOnce(&str) -> Result<(), String> + Send + 'static,
    cancelled: impl std::future::Future<Output = ()>,
) -> Result<Value, String> {
    tokio::select! {
        result = timeout_at(Instant::now() + options.timeout, login(api, options, progress, open)) => {
            result.map_err(|_| "Sign-in timed out. Check wispnet account, or run wispnet login to retry".to_owned())?
        }
        _ = cancelled => Err("Sign-in wait cancelled. Check wispnet account; any pending request will expire".into()),
    }
}

pub async fn run(args: &[OsString]) -> Result<(), String> {
    let options = Options::parse(args)?;
    if options.help {
        println!("{HELP}");
        return Ok(());
    }
    let config = match &options.config {
        Some(path) => path.clone(),
        None => api::home()?.join(".config/wispnet-node/host.toml"),
    };
    let api = Api::new(config)?;
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let result = match options.operation {
        Operation::Login => {
            let progress: &mut dyn Write = if options.json {
                &mut stderr
            } else {
                &mut stdout
            };
            bounded_login(&api, &options, progress, open_browser, async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?
        }
        Operation::Status => status(&request(&api, json!({"op":"command_center_status"})).await?)?,
        Operation::Logout => status(&request(&api, json!({"op":"command_center_unlink"})).await?)?,
    };
    if options.json {
        writeln!(
            stdout,
            "{}",
            serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
        )
        .map_err(|e| e.to_string())?;
    } else {
        if options.operation == Operation::Logout {
            writeln!(
                stdout,
                "Signed out on this computer. Managed public publications stopped."
            )
            .map_err(|e| e.to_string())?;
        }
        for line in lines(&result)? {
            writeln!(stdout, "{line}").map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The TUI first restores the ordinary terminal, then invokes this same CLI
/// command in a child. It never nests a Tokio runtime inside its running one.
pub(crate) fn terminal_login(api: &Api, restart: bool) -> Result<(), String> {
    let mut command =
        Command::new(std::env::current_exe().map_err(|_| "Cannot locate Wispnet CLI")?);
    command.arg("login").arg("--config").arg(&api.config);
    if restart {
        command.arg("--restart");
    }
    let status = command
        .status()
        .map_err(|_| "Cannot start account sign-in")?;
    if status.success() {
        Ok(())
    } else {
        Err("Sign-in did not finish; check the account status or try again".into())
    }
}

#[cfg(test)]
mod tests;
