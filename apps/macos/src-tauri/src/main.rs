#![cfg(target_os = "macos")]
mod ipc;
use serde_json::{Value, json};
use std::{
    io::Read,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

struct Node {
    socket: PathBuf,
}
fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("Missing home directory.".into())
}
fn socket_path() -> Result<PathBuf, String> {
    let config = std::fs::read_to_string(home()?.join(".config/wispnet-node/host.toml"))
        .map_err(|e| format!("Cannot read node configuration: {e}"))?;
    let config: toml::Value = toml::from_str(&config).map_err(|e| e.to_string())?;
    let state = PathBuf::from(
        config
            .get("state_dir")
            .and_then(toml::Value::as_str)
            .ok_or("Missing state directory.")?,
    );
    if !state.is_absolute() {
        return Err("Node state directory must be absolute.".into());
    }
    Ok(state.join("node.sock"))
}
fn launchctl(args: &[&str]) -> Result<(), String> {
    let mut child = Command::new("/bin/launchctl")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err("Cannot change node state. Check its launch agent and logs.".into())
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Node control timed out. Refresh before retrying.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn lifecycle(online: bool, socket: &std::path::Path) -> Result<(), String> {
    let domain = format!("gui/{}", unsafe { libc::geteuid() });
    let service = format!("{domain}/app.wispnet.node");
    let loaded = launchctl(&["print", &service]).is_ok();
    if online {
        if loaded {
            launchctl(&["kickstart", &service])?;
        } else {
            launchctl(&[
                "bootstrap",
                &domain,
                home()?
                    .join("Library/LaunchAgents/app.wispnet.node.plist")
                    .to_str()
                    .ok_or("Invalid launch agent path.")?,
            ])?;
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while ipc::request(socket, &json!({"op":"status"})).is_err() {
            if Instant::now() >= deadline {
                return Err("Node started but its API is not ready.".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    } else if loaded {
        launchctl(&["bootout", &service])?;
    }
    Ok(())
}
#[tauri::command]
async fn node_request(node: tauri::State<'_, Node>, payload: Value) -> Result<Value, String> {
    let socket = node.socket.clone();
    tauri::async_runtime::spawn_blocking(move || ipc::request(&socket, &payload))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn set_online(node: tauri::State<'_, Node>, online: bool) -> Result<(), String> {
    let socket = node.socket.clone();
    tauri::async_runtime::spawn_blocking(move || lifecycle(online, &socket))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn open_service(
    node: tauri::State<'_, Node>,
    host: String,
    service: String,
    incarnation: String,
    host_name: Option<String>,
    reduce_motion: bool,
) -> Result<(), String> {
    let socket = node.socket.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let start = Instant::now();
        ipc::request(&socket, &json!({"op":"enable"}))?;
        let result = ipc::request(&socket, &json!({"op":"open", "host":host, "service":service, "incarnation":incarnation, "host_name":host_name}))?;
        let url = ipc::browser_url(result["url"].as_str().ok_or("Missing browser handoff.")?)?;
        if !reduce_motion { std::thread::sleep(Duration::from_millis(900).saturating_sub(start.elapsed())); }
        let status = Command::new("/usr/bin/open").arg(url.as_str()).status().map_err(|e| e.to_string())?;
        if status.success() { Ok(()) } else { Err("Could not open your browser.".into()) }
    }).await.map_err(|e| e.to_string())?
}
#[tauri::command]
async fn copy_pairing_code(code: String) -> Result<(), String> {
    if !code.starts_with("wispnet1:") || code.len() > 8192 {
        return Err("Invalid pairing code.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Write;
        let mut child = Command::new("/usr/bin/pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        child
            .stdin
            .take()
            .ok_or("Cannot open clipboard.")?
            .write_all(code.as_bytes())
            .map_err(|e| e.to_string())?;
        if child.wait().map_err(|e| e.to_string())?.success() {
            Ok(())
        } else {
            Err("Could not copy pairing code.".into())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
// Preserve private labels from the previous Swift app on first Tauri launch.
#[tauri::command]
fn legacy_peer_names() -> Value {
    let Ok(home) = home() else {
        return json!({});
    };
    let Ok(output) = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-", "--"])
        .arg(home.join("Library/Preferences/app.wispnet.desktop.plist"))
        .output()
    else {
        return json!({});
    };
    let preferences: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    preferences
        .get("peerNames")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}))
}
#[tauri::command]
fn build_identity() -> &'static str {
    env!("WISPNET_BUILD_ID")
}
#[tauri::command]
fn tray_status(app: tauri::AppHandle, text: String, online: bool) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id("wispnet") {
        let status = MenuItem::with_id(&app, "status", &text, false, None::<&str>)
            .map_err(|e| e.to_string())?;
        let show = MenuItem::with_id(&app, "show", "Show Wispnet", true, None::<&str>)
            .map_err(|e| e.to_string())?;
        let toggle = MenuItem::with_id(
            &app,
            if online { "offline" } else { "online" },
            if online { "Go offline" } else { "Go online" },
            true,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        let quit = MenuItem::with_id(
            &app,
            "quit",
            "Quit Wispnet (node keeps running)",
            true,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        let menu =
            Menu::with_items(&app, &[&status, &show, &toggle, &quit]).map_err(|e| e.to_string())?;
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
        tray.set_tooltip(Some(text.chars().take(256).collect::<String>()))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn reveal(app: &tauri::AppHandle) {
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Exercise exactly the GUI transport without creating a webview or a second instance.
    if args.get(1).map(String::as_str) == Some("--request") {
        let result = (|| {
            let path = args.get(2).ok_or("Missing socket path.")?;
            let mut input = Vec::new();
            std::io::stdin()
                .take(16385)
                .read_to_end(&mut input)
                .map_err(|e| e.to_string())?;
            if input.len() > 16384 {
                return Err("Request exceeds 16 KiB.".into());
            }
            ipc::request(
                std::path::Path::new(path),
                &serde_json::from_slice(&input).map_err(|e| e.to_string())?,
            )
        })();
        match result {
            Ok(value) => println!("{value}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    let socket = match socket_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| reveal(app)))
        .manage(Node { socket })
        .invoke_handler(tauri::generate_handler![
            node_request,
            set_online,
            open_service,
            build_identity,
            copy_pairing_code,
            legacy_peer_names,
            tray_status
        ])
        .setup(move |app| {
            let show = MenuItem::with_id(app, "show", "Show Wispnet", true, None::<&str>)?;
            let quit = MenuItem::with_id(
                app,
                "quit",
                "Quit Wispnet (node keeps running)",
                true,
                None::<&str>,
            )?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::with_id("wispnet")
                .title("W")
                .tooltip("Wispnet")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => reveal(app),
                    "quit" => app.exit(0),
                    "online" | "offline" => {
                        let socket = app.state::<Node>().socket.clone();
                        let online = event.id.as_ref() == "online";
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let result = tauri::async_runtime::spawn_blocking(move || {
                                lifecycle(online, &socket)
                            })
                            .await;
                            if !matches!(result, Ok(Ok(()))) {
                                reveal(&handle);
                            }
                        });
                    }
                    _ => {}
                })
                .build(app)?;
            if args.iter().any(|arg| arg == "--background") {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            } else {
                reveal(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                let _ = window
                    .app_handle()
                    .set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
        })
        .build(tauri::generate_context!())
        .expect("Cannot start Wispnet")
        .run(|app, event| {
            if let tauri::RunEvent::Reopen { .. } = event {
                reveal(app);
            }
        });
}
