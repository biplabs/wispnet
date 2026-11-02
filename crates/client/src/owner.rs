//! Owner-only operations for an embedded host. No owner capability leaves JNI.
use crate::{Client, argument};
use qrcode::{Color, QrCode};
use serde_json::{Value, json};
use std::sync::Arc;
use wispnet_registry::host::Mutation;

pub(crate) async fn command(client: &Arc<Client>, value: &Value) -> Result<Value, String> {
    let authority = client
        .local_authority
        .as_ref()
        .ok_or("Local host unavailable")?;
    let own = client.endpoint.id();
    let directory = client.state.clone();
    match argument(value, "op")? {
        "local_owner_status" => {
            // A gateway outage must not hide the local controls needed to turn
            // a publication off or edit unrelated services.
            let _ = client.publications.reconcile(client).await;
            let snapshot = authority.snapshot().await.map_err(|e| e.to_string())?;
            let pins: Vec<_> = authority
                .paired_hosts()
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|host| host.host)
                .collect();
            let name = authority.device_id();
            let invitations = tokio::task::spawn_blocking(move || {
                let (mut store, owner) = wispnet_registry::host::owner_store(&directory, own)
                    .map_err(|e| e.to_string())?;
                store
                    .pairing_list(own, &owner, wispnet_registry::enrollment::now())
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|_| "Cannot read owner requests")??;
            let counts = client._host.as_ref().map(|host| host.counts());
            Ok(json!({"api_version":1,"host":snapshot,"host_name":name,
                "invitations":invitations,"peer_pins":pins,
                "connected_peers":counts.as_ref().map(|c|c.peers()).unwrap_or_default(),
                "recent_peers":counts.as_ref().map(|c|c.recent_peers()).unwrap_or_default(),
                "public_access":client.publications.status(client).await.unwrap_or_else(|error| json!({"error":error,"services":[]})),
                "relay":wispnet_agent::diagnostics::relay_state(&client.endpoint,client.config.relay_url.is_some())}))
        }
        "local_owner_mutate" => {
            let operation: Mutation = serde_json::from_value(value["operation"].clone())
                .map_err(|_| "Invalid owner operation")?;
            // Provider-backed folders must pass Java's persisted URI-grant checks.
            // A mobile app cannot offer an operating-system account's host shell.
            match &operation {
                Mutation::Approve { .. }
                | Mutation::Revoke { .. }
                | Mutation::ApproveInvitation { .. }
                | Mutation::RejectInvitation { .. }
                | Mutation::GrantService { .. }
                | Mutation::RevokeService { .. } => {}
                Mutation::Service { id, .. }
                | Mutation::EditService { id, .. }
                | Mutation::Remove { id, .. }
                | Mutation::SetServiceEnabled { id, .. }
                    if id != "shell" && id != "files" =>
                {
                    if let Some(service) = authority
                        .snapshot()
                        .await
                        .map_err(|e| e.to_string())?
                        .services
                        .iter()
                        .find(|s| s.id == *id)
                        && !matches!(
                            service.kind,
                            wispnet_core::host::ServiceKind::Http
                                | wispnet_core::host::ServiceKind::Tcp
                        )
                    {
                        return Err("Use the folder permission editor for this service".into());
                    }
                }
                _ => {
                    return Err("Use the native folder permission editor for this operation".into());
                }
            }
            let revision = tokio::task::spawn_blocking(move || {
                let (mut store, owner) = wispnet_registry::host::owner_store(&directory, own)
                    .map_err(|e| e.to_string())?;
                store
                    .host_mutate(own, &owner, operation)
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|_| "Owner operation failed")??;
            Ok(json!({"revision":revision}))
        }
        "local_owner_invite" => {
            let relay = client
                .config
                .relay_url
                .clone()
                .ok_or("Pairing requires a configured relay")?;
            let name = authority.device_id();
            tokio::task::spawn_blocking(move || {
                let (mut store, owner) = wispnet_registry::host::owner_store(&directory, own)
                    .map_err(|e| e.to_string())?;
                let mut invitation = store
                    .pairing_invite(
                        own,
                        &owner,
                        &relay,
                        wispnet_registry::enrollment::now(),
                        300,
                    )
                    .map_err(|e| e.to_string())?;
                invitation.host_name = Some(name.clone());
                let code = wispnet_agent::pairing::encode_invitation(&invitation)
                    .map_err(|e| e.to_string())?;
                let qr = QrCode::new(code.as_bytes()).map_err(|_| "Pairing code is too large")?;
                let width = qr.width();
                let matrix: Vec<Vec<bool>> = (0..width + 8)
                    .map(|y| {
                        (0..width + 8)
                            .map(|x| {
                                x >= 4
                                    && y >= 4
                                    && x < width + 4
                                    && y < width + 4
                                    && qr[(x - 4, y - 4)] == Color::Dark
                            })
                            .collect()
                    })
                    .collect();
                Ok(
                    json!({"host":own,"host_name":name,"invitation":invitation.invitation,
                    "expires_at":invitation.expires_at,"code":code.as_str(),"matrix":matrix}),
                )
            })
            .await
            .map_err(|_| "Cannot generate pairing code")?
        }
        "local_owner_apply" => {
            let id = argument(value, "id")?.to_owned();
            if !wispnet_registry::control::valid_token(&id) {
                return Err("Invalid operation ID".into());
            }
            let target: wispnet_core::identity::EndpointId = argument(value, "host")?
                .parse()
                .map_err(|_| "Invalid target host")?;
            let action: wispnet_registry::combined::Action =
                serde_json::from_value(value["action"].clone())
                    .map_err(|_| "Invalid owner action")?;
            if target != own {
                if !client.services_enabled() {
                    return Err("Go online before applying remote owner operations".into());
                }
                // Existing explicit Shell permission is the execution capability.
                // The remote node still verifies its OS owner and durably records
                // the operation before replying. No admin key crosses the wire.
                let request = json!({"op":"apply_operation","id":id,"host":target,"action":action});
                let encoded =
                    serde_json::to_string(&request).map_err(|_| "Invalid owner action")?;
                let quoted = format!("'{}'", encoded.replace('\'', "'\\''"));
                let result = client.run_shell_command(&json!({"host":target.to_string(),
                    "command":format!("printf '%s' {quoted} | \"$HOME/.local/bin/wispnet-node\" --response"),
                    "timeout_seconds":8,"max_output_bytes":65536})).await?;
                if result["exit_code"] != 0
                    || result["timed_out"] == true
                    || result["output_truncated"] == true
                {
                    return Err("Owner command unavailable or acknowledgement unconfirmed".into());
                }
                let reply: Value = serde_json::from_str(
                    result["stdout"]
                        .as_str()
                        .ok_or("Owner acknowledgement unavailable")?,
                )
                .map_err(|_| "Invalid owner acknowledgement")?;
                if reply["ok"] != true {
                    return Err("Owner operation rejected".into());
                }
                let receipt = reply["result"].clone();
                if receipt["host"] != target.to_string()
                    || receipt["id"] != id
                    || !["applied", "pending", "rejected"]
                        .contains(&receipt["state"].as_str().unwrap_or(""))
                {
                    return Err("Mismatched owner acknowledgement".into());
                }
                if receipt["state"] == "applied" && receipt["revision"].as_u64().is_none() {
                    return Err("Missing durable owner revision".into());
                }
                return Ok(receipt);
            }
            tokio::task::spawn_blocking(move || {
                let (mut store, owner) = wispnet_registry::host::owner_store(&directory, own)
                    .map_err(|e|e.to_string())?;
                let receipt = store.host_apply_operation(own,&owner,&id,action).map_err(|e|e.to_string())?;
                Ok(json!({"host":own,"id":id,"state":receipt.state,"revision":receipt.revision,"reason":receipt.reason}))
            }).await.map_err(|_|"Owner operation unconfirmed")?
        }
        _ => Err("Unsupported owner operation".into()),
    }
}
