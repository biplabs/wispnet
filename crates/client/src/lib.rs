//! Shared peer browser client. One identity/endpoint shared by pairing, catalogs and gateway.
pub mod gateway;
use iroh::Endpoint;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::runtime::Runtime;
use wispnet_core::{
    config::TransportConfig,
    identity::{EndpointId, SecretKey},
};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static CLIENT: Mutex<Option<Arc<Client>>> = Mutex::new(None);
fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("native runtime")
    })
}
pub struct Client {
    pub endpoint: Endpoint,
    key: SecretKey,
    config: TransportConfig,
    state: PathBuf,
    pub gateway: gateway::Gateways,
    pub(crate) local_authority: Option<wispnet_registry::host::Authority>,
    telemetry: Mutex<wispnet_agent::tunnel::PeerTelemetry>,
}
fn token() -> String {
    SecretKey::generate()
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn argument<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing app argument".into())
}
fn peer(value: &Value) -> Result<EndpointId, String> {
    argument(value, "host")?
        .parse()
        .map_err(|_| "Invalid host pin".into())
}
impl Client {
    pub fn from_endpoint(
        endpoint: Endpoint,
        key: SecretKey,
        state: PathBuf,
        config: TransportConfig,
        authority: wispnet_registry::host::Authority,
    ) -> Arc<Self> {
        Arc::new(Self {
            endpoint,
            key,
            state,
            config,
            gateway: gateway::Gateways::default(),
            local_authority: Some(authority),
            telemetry: Mutex::new(std::collections::BTreeMap::new()),
        })
    }
    pub async fn new(
        key: SecretKey,
        state: PathBuf,
        config: TransportConfig,
    ) -> Result<Arc<Self>, String> {
        config
            .validate()
            .map_err(|_| "Invalid relay configuration")?;
        let endpoint = wispnet_agent::tunnel::bind_configured_client(&key, Some(&config))
            .await
            .map_err(|e| e.to_string())?;
        Ok(Arc::new(Self {
            endpoint,
            key,
            config,
            state,
            gateway: gateway::Gateways::default(),
            local_authority: None,
            telemetry: Mutex::new(std::collections::BTreeMap::new()),
        }))
    }
    pub fn record_telemetry(&self, peer: EndpointId, connection: &iroh::endpoint::Connection) {
        let path = wispnet_agent::diagnostics::selected_path(connection);
        let rtt = wispnet_agent::diagnostics::selected_rtt_ms(connection);
        let mut map = self.telemetry.lock().unwrap_or_else(|e| e.into_inner());
        let key = peer.to_string();
        if map.len() >= 1024
            && !map.contains_key(&key)
            && let Some(first) = map.keys().next().cloned()
        {
            map.remove(&first);
        }
        map.insert(key, (path, rtt));
    }
    pub fn telemetry_map(&self) -> wispnet_agent::tunnel::PeerTelemetry {
        self.telemetry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub async fn catalog(&self, host: EndpointId) -> Result<Value, String> {
        self.check_pairing(host).await?;
        let state = self.state.clone();
        let epoch =
            tokio::task::spawn_blocking(move || wispnet_agent::catalogs::begin(&state, host))
                .await
                .map_err(|_| "Catalog baseline task failed")??;
        let target = self.target(host).await?;
        let connection = wispnet_agent::control::connect_target(&self.endpoint, target)
            .await
            .map_err(|e| e.to_string())?;
        let result = wispnet_agent::control::catalog_on(&connection, host)
            .await
            .map_err(|e| e.to_string());
        if result.is_ok() {
            wispnet_agent::diagnostics::settle_path(&connection).await;
        }
        self.record_telemetry(host, &connection);
        connection.close(0u32.into(), b"catalog complete");
        let catalog = result?;
        let state = self.state.clone();
        let accepted = catalog.clone();
        tokio::task::spawn_blocking(move || {
            wispnet_agent::catalogs::accept(&state, host, &epoch, &accepted)
        })
        .await
        .map_err(|_| "Catalog baseline task failed")??;
        serde_json::to_value(catalog).map_err(|_| "Invalid catalog".into())
    }
    pub async fn saved_host(
        &self,
        host: EndpointId,
    ) -> Result<Option<wispnet_core::pairing::Bootstrap>, String> {
        if let Some(authority) = &self.local_authority
            && let Some(record) = authority
                .paired_hosts()
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|b| b.host == host)
        {
            return Ok(Some(record));
        }
        if let Some(authority) = &self.local_authority
            && authority
                .pairing_blocked(host)
                .await
                .map_err(|e| e.to_string())?
        {
            return Ok(None);
        }
        wispnet_agent::peers::load(&self.state, host).map_err(|_| "Invalid saved host".into())
    }
    pub async fn target(&self, host: EndpointId) -> Result<iroh::EndpointAddr, String> {
        wispnet_agent::tunnel::peer_target(
            host,
            Some(&self.config),
            self.saved_host(host).await?.and_then(|p| p.direct_hint),
        )
        .map_err(|e| e.to_string())
    }
    async fn check_pairing(&self, host: EndpointId) -> Result<(), String> {
        let Some(authority) = &self.local_authority else {
            return Ok(());
        };
        let Some(pairing) = authority
            .pairing_session(host)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(());
        };
        let target = self.target(host).await?;
        // Older peers and transport failures do not prove removal. Normal catalog
        // authorization still applies, including when no status can be obtained.
        let Ok(status) =
            wispnet_agent::pairing::pairing_status(&self.endpoint, target, &pairing).await
        else {
            return Ok(());
        };
        if status.state != wispnet_core::pairing::PairingState::Removed {
            return Ok(());
        }
        let state = self.state.clone();
        let own = self.endpoint.id();
        let removed = tokio::task::spawn_blocking(move || {
            let (mut store, owner) =
                wispnet_registry::host::owner_store(&state, own).map_err(|e| e.to_string())?;
            store
                .apply_remote_removal(own, &owner, host, &pairing)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|_| "Cannot apply remote removal")??;
        if removed {
            self.gateway.forget(self, host).await?;
            return Err("Pairing removed by remote machine".into());
        }
        Err("Pairing changed during refresh; try again".into())
    }
    pub async fn command(self: &Arc<Self>, value: &Value) -> Result<Value, String> {
        match argument(value, "op")? {
            "identity" => Ok(json!({"peer":self.endpoint.id().to_string()})),
            "preview_invitation" => {
                let invitation = wispnet_agent::pairing::decode_invitation(
                    argument(value, "invitation")?.as_bytes(),
                )
                .map_err(|_| "Invalid invitation")?;
                if !invitation.valid(wispnet_registry::enrollment::now()) {
                    return Err("Expired or invalid invitation".into());
                }
                Ok(
                    json!({"host":invitation.host.to_string(),"host_name":invitation.host_name.as_deref().unwrap_or(""),"relay_url":invitation.relay_url,"expires_at":invitation.expires_at}),
                )
            }
            "pair" => {
                let host = peer(value)?;
                let text = argument(value, "invitation")?;
                if text.len() > 8192 {
                    return Err("Invitation too large".into());
                }
                let invitation: wispnet_core::pairing::Invitation =
                    wispnet_agent::pairing::decode_invitation(text.as_bytes())
                        .map_err(|_| "Invalid invitation")?;
                let bootstrap = invitation
                    .bootstrap(host, wispnet_registry::enrollment::now())
                    .ok_or("Expired invitation or wrong host pin")?;
                let reverse =
                    self.local_authority
                        .as_ref()
                        .map(|_| wispnet_core::pairing::Bootstrap {
                            version: wispnet_core::pairing::VERSION,
                            host: self.endpoint.id(),
                            relay_url: bootstrap.relay_url.clone(),
                            direct_hint: self
                                .endpoint
                                .bound_sockets()
                                .into_iter()
                                .find(|a| !a.ip().is_unspecified()),
                        });
                let session = invitation.invitation.clone();
                let pending = wispnet_agent::pairing::request_enrollment_with_bootstrap(
                    &self.endpoint,
                    host,
                    &invitation,
                    &self.config,
                    &self.key,
                    value
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Wispnet Android"),
                    reverse,
                )
                .await
                .map_err(|e| e.to_string())?;
                if self.local_authority.is_some() {
                    let state = self.state.clone();
                    let own = self.endpoint.id();
                    let name = invitation
                        .host_name
                        .clone()
                        .unwrap_or_else(|| host.to_string());
                    tokio::task::spawn_blocking(move || {
                        let (mut store, owner) = wispnet_registry::host::owner_store(&state, own)
                            .map_err(|e| e.to_string())?;
                        store
                            .pairing_accept_session(own, &owner, &bootstrap, &name, Some(&session))
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .map_err(|_| "Cannot save mutual pairing")??;
                } else {
                    wispnet_agent::peers::save(&self.state, &bootstrap)
                        .map_err(|_| "Cannot save host pin")?;
                }
                let state = self.state.clone();
                tokio::task::spawn_blocking(move || wispnet_agent::catalogs::reset(&state, host))
                    .await
                    .map_err(|_| "Cannot reset catalog baseline")??;
                Ok(json!({"state":"pending_owner_approval","host":pending.host.to_string()}))
            }
            "unpair" => {
                let host = peer(value)?;
                if host == self.endpoint.id() {
                    return Err("Cannot unpair this machine".into());
                }
                let saved = self.saved_host(host).await?;
                if let Some(authority) = &self.local_authority {
                    let known = authority
                        .snapshot()
                        .await
                        .map_err(|e| e.to_string())?
                        .clients
                        .iter()
                        .any(|c| c.peer == host);
                    let bootstrap = saved.or_else(|| {
                        if !known {
                            return None;
                        }
                        self.config.relay_url.clone().map(|relay_url| {
                            wispnet_core::pairing::Bootstrap {
                                version: wispnet_core::pairing::VERSION,
                                host,
                                relay_url,
                                direct_hint: None,
                            }
                        })
                    });
                    let state = self.state.clone();
                    let own = self.endpoint.id();
                    let expected = value
                        .get("expected_revision")
                        .map(|v| v.as_u64().ok_or("Invalid policy revision"))
                        .transpose()?;
                    tokio::task::spawn_blocking(move || {
                        let (mut store, owner) = wispnet_registry::host::owner_store(&state, own)
                            .map_err(|e| e.to_string())?;
                        store
                            .delete_pairing(own, &owner, host, bootstrap.as_ref(), expected)
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .map_err(|_| "Cannot remove pairing")??;
                }
                self.gateway.forget(self, host).await?;
                wispnet_agent::peers::remove(&self.state, host).map_err(|e| e.to_string())?;
                Ok(json!({"unpaired":host.to_string()}))
            }
            "set_name" => {
                let host = peer(value)?;
                let name = argument(value, "name")?;
                if !wispnet_core::enrollment::valid_name(name) {
                    return Err("Invalid device name".into());
                }
                if self.saved_host(host).await?.is_none() {
                    return Err("Pair with this host first".into());
                }
                let connection = wispnet_agent::control::connect_target(
                    &self.endpoint,
                    self.target(host).await?,
                )
                .await
                .map_err(|e| e.to_string())?;
                let result = wispnet_agent::control::set_name_on(&connection, host, name).await;
                connection.close(0u32.into(), b"name update complete");
                result.map_err(|e| e.to_string())?;
                Ok(json!({"name":name}))
            }
            "catalog" => {
                let host = peer(value)?;
                if self.saved_host(host).await?.is_none() {
                    return Err("Pair with this host first".into());
                }
                self.catalog(host).await
            }
            "relay_status" => Ok(
                json!({"relay":wispnet_agent::diagnostics::relay_state(&self.endpoint,self.config.relay_url.is_some())}),
            ),
            "telemetry" => {
                let peers: serde_json::Map<String, Value> = self
                    .telemetry_map()
                    .into_iter()
                    .map(|(k, (path, rtt_ms))| (k, json!({"path": path, "rtt_ms": rtt_ms})))
                    .collect();
                Ok(json!({"peers": peers}))
            }
            "access_status" => Ok(json!({"enabled":self.gateway.is_enabled()})),
            "enable" => {
                self.gateway.enable();
                if let Err(error) = self.gateway.restore(self.clone()).await {
                    self.gateway.stop().await;
                    return Err(error);
                }
                Ok(json!({"enabled":true}))
            }
            "stop" => {
                self.gateway.stop().await;
                Ok(json!({"enabled":false}))
            }
            "network" => {
                let interrupted = self.gateway.interrupt().await;
                self.endpoint.network_change().await;
                if self.gateway.is_enabled() {
                    self.gateway.restore(self.clone()).await?;
                }
                Ok(json!({"network_changed":true,"interrupted_requests":interrupted}))
            }
            "open" | "app_open" => {
                let host = peer(value)?;
                let id = argument(value, "service")?;
                let incarnation = argument(value, "incarnation")?;
                if !wispnet_core::enrollment::valid_code(incarnation) {
                    return Err("Refresh this service before opening".into());
                }
                if host == self.endpoint.id()
                    && let Some(authority) = &self.local_authority
                {
                    // Only the owner-only node API constructs a local authority client.
                    // Local owner access needs neither a self-pin nor a self-grant.
                    let services = authority.services().await.map_err(|e| e.to_string())?;
                    if !services.iter().any(|s| {
                        s.id == id
                            && s.incarnation == incarnation
                            && s.kind == wispnet_core::host::ServiceKind::Http
                    }) {
                        return Err("Service no longer permitted".into());
                    }
                } else {
                    if self.saved_host(host).await?.is_none() {
                        return Err("Pair with this host first".into());
                    }
                    let catalog = self.catalog(host).await?;
                    if !catalog["services"].as_array().is_some_and(|s| {
                        s.iter().any(|s| {
                            s["id"] == id && s["incarnation"] == incarnation && s["kind"] == "http"
                        })
                    }) {
                        return Err("Service no longer permitted".into());
                    }
                }
                let url = self
                    .gateway
                    .open_named(
                        self.clone(),
                        host,
                        id.to_owned(),
                        incarnation.to_owned(),
                        value.get("host_name").and_then(Value::as_str),
                    )
                    .await?;
                if value["op"] == "app_open" {
                    let access = self
                        .gateway
                        .app_access(host, id, argument(value, "grant")?)
                        .await?;
                    return Ok(json!({"url":url,"access_token":access,"expires_in_seconds":900}));
                }
                let launch_url = self.gateway.launch_url(host, id).await?;
                Ok(json!({"url":url,"launch_url":launch_url}))
            }
            "app_revoke" => {
                let grant = argument(value, "grant")?;
                if !wispnet_core::enrollment::valid_code(grant) {
                    return Err("Invalid app grant".into());
                }
                self.gateway.revoke_app(grant).await;
                if self.gateway.is_enabled() {
                    self.gateway.restore(self.clone()).await?;
                }
                Ok(json!({"revoked":true}))
            }
            _ => Err("Unknown app operation".into()),
        }
    }
}
pub fn call(text: &str) -> String {
    let outcome = (|| -> Result<Value, String> {
        if text.len() > 16384 {
            return Err("App request too large".into());
        }
        let value: Value = serde_json::from_str(text).map_err(|_| "Invalid app request")?;
        if value["op"] == "new_key" {
            return Ok(json!({"key":token()}));
        }
        if value["op"] == "init" {
            let key: SecretKey = argument(&value, "key")?
                .parse()
                .map_err(|_| "Invalid device key")?;
            let state = PathBuf::from(argument(&value, "state")?);
            let config:TransportConfig=serde_json::from_value(json!({"bind_addr":"0.0.0.0:0","relay_url":argument(&value,"relay")?,"relay_only":false})).map_err(|_|"Invalid relay")?;
            let mut client = CLIENT.lock().map_err(|_| "Native state unavailable")?;
            if client.is_none() {
                *client = Some(runtime().block_on(Client::new(key, state, config))?);
            }
            return Ok(
                json!({"ready":true,"peer":client.as_ref().expect("initialized").endpoint.id().to_string()}),
            );
        }
        let client = CLIENT
            .lock()
            .map_err(|_| "Native state unavailable")?
            .clone()
            .ok_or("Client not initialized")?;
        runtime().block_on(client.command(&value))
    })();
    match outcome {
        Ok(value) => json!({"ok":true,"result":value}).to_string(),
        Err(error) => {
            let code = match error.as_str() {
                "access denied" => "host_denied",
                "catalog host or schema mismatch"
                | "invalid catalog"
                | "control frame truncated"
                | "control trailing data"
                | "control frame too large"
                | "Catalog rollback, changed generation or inconsistent snapshot"
                | "Catalog baseline unavailable or corrupt"
                | "Pairing changed during catalog refresh" => "catalog_rejected",
                "Service no longer permitted" => "service_denied",
                "pairing request denied" => "invitation_denied",
                "Expired invitation or wrong host pin" => "invitation_invalid",
                _ => "operation_failed",
            };
            json!({"ok":false,"error":error,"code":code}).to_string()
        }
    }
}
