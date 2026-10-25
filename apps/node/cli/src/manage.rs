//! Scriptable management of the running node, sharing the terminal/desktop API.
use crate::tui::{api::Api, service_links};
use serde_json::{Value, json};
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

pub const HELP: &str = "Management (uses the running node; --config is optional):
  wispnet node-status
  wispnet online | offline
  wispnet settings [--port 7842] [--device-id my-computer]
  wispnet shell-access [--allow PUBLIC_ID,PUBLIC_ID | --allow none]
  wispnet service-links --host HOST_ID --service SERVICE_ID
  wispnet resolve-link --link wispnet://HOST_ID/SERVICE_ID
  wispnet node-request --request /absolute/path/request.json
All management commands return JSON. node-request accepts the same owner-local
operations as the desktop: catalogs, pairing, revision-checked service/grant
mutations, access settings and public access. Request files must be owner-only.
Service links grant no permissions; pair and obtain access first.";

pub fn handles(mode: &str) -> bool {
    matches!(
        mode,
        "node-status"
            | "online"
            | "offline"
            | "settings"
            | "service-links"
            | "resolve-link"
            | "node-request"
            | "shell-access"
    )
}
pub fn run(args: &[OsString]) -> Result<Value, String> {
    let mode = args[0].to_str().ok_or("Invalid command")?;
    let mut options = BTreeMap::new();
    if !(args.len() - 1).is_multiple_of(2) {
        return Err(HELP.into());
    }
    for pair in args[1..].chunks_exact(2) {
        let key = pair[0].to_str().ok_or("Invalid option")?;
        let value = pair[1].to_str().ok_or("Invalid option value")?;
        let valid = key == "--config"
            || match mode {
                "settings" => matches!(key, "--port" | "--device-id"),
                "service-links" => matches!(key, "--host" | "--service"),
                "resolve-link" => key == "--link",
                "node-request" => key == "--request",
                "shell-access" => key == "--allow",
                _ => false,
            };
        if !valid || options.insert(key, value).is_some() {
            return Err(HELP.into());
        }
    }
    let config = options
        .get("--config")
        .map(PathBuf::from)
        .unwrap_or(crate::tui::api::home()?.join(".config/wispnet-node/host.toml"));
    let api = Api::new(config)?;
    let required = |key| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| format!("Missing {key}\n{HELP}"))
    };
    match mode {
        "shell-access" => {
            let status = api.request(json!({"op":"status"}))?;
            let service = status["host"]["services"]
                .as_array()
                .and_then(|ss| {
                    ss.iter()
                        .find(|s| s["id"] == "shell" && s["kind"] == "shell")
                })
                .ok_or("Built-in Shell unavailable; update and restart the node")?;
            let Some(allow) = options.get("--allow") else {
                return Ok(service.clone());
            };
            let allow = if *allow == "none" {
                vec![]
            } else {
                allow
                    .split(',')
                    .map(|id| {
                        id.parse::<wispnet_core::identity::EndpointId>()
                            .map(|p| json!(p))
                            .map_err(|_| "Invalid allowed device ID")
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };
            let result=api.request(json!({"op":"mutate","operation":{"op":"shell","allow":allow,"expected_revision":status["host"]["revision"]}}))?;
            Ok(json!({"service":"shell","revision":result["revision"],"allow":allow}))
        }
        "node-status" => api.request(json!({"op":"status","refresh_clients":true})),
        "online" => {
            api.online(true)?;
            api.request(json!({"op":"enable"}))
        }
        "offline" => api.online(false),
        "node-request" => {
            let data =
                crate::tui::api::read_private(&PathBuf::from(required("--request")?), 16384)?;
            let request: Value = serde_json::from_str(&data).map_err(|_| "Invalid request JSON")?;
            if !request.is_object() || !request["op"].is_string() {
                return Err("Request must be an object with an op".into());
            }
            api.request(request)
        }
        "settings" => {
            let port = options
                .get("--port")
                .map(|s| {
                    s.parse::<u16>()
                        .map_err(|_| "Port must be between 1024 and 65535")
                })
                .transpose()?;
            if port.is_some_and(|p| p < 1024) {
                return Err("Port must be between 1024 and 65535".into());
            }
            if let Some(name) = options.get("--device-id")
                && !wispnet_core::protocol::valid_service_id(name)
            {
                return Err("Invalid device ID".into());
            }
            if let Some(name) = options.get("--device-id") {
                api.check_rename(name)?;
            }
            if let Some(port) = port {
                let mut settings =
                    api.request(json!({"op":"gateway_settings"}))?["settings"].clone();
                settings["registry_port"] = json!(port);
                settings["service_ports"] = json!([]);
                api.request(json!({"op":"configure_access","settings":settings}))?;
            }
            if let Some(name) = options.get("--device-id") {
                api.rename_device(name)?;
            }
            let mut result = api.request(json!({"op":"gateway_settings"}))?;
            result["device_id"] = api.request(json!({"op":"status"}))?["host_name"].clone();
            Ok(result)
        }
        "resolve-link" => {
            let link = required("--link")?;
            wispnet_core::service_link::ServiceLink::parse(link)?;
            api.request(json!({"op":"enable"}))?;
            api.request(json!({"op":"resolve_link","link":link}))
        }
        "service-links" => {
            let host = required("--host")?;
            let service = required("--service")?;
            wispnet_core::service_link::ServiceLink::parse(&format!("wispnet://{host}/{service}"))?;
            let status = api.request(json!({"op":"status"}))?;
            let catalog = if status["host"]["host"] == host {
                status["host"].clone()
            } else {
                api.request(json!({"op":"catalog","host":host}))?
            };
            let selected = catalog["services"]
                .as_array()
                .and_then(|a| a.iter().find(|s| s["id"] == service))
                .ok_or("Service unavailable or not permitted")?;
            if selected["kind"] != "http" {
                return Err("TCP services use wispnet tunnel".into());
            }
            service_links(&api, host, selected)
        }
        _ => Err(HELP.into()),
    }
}
