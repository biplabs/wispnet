//! Private local UI API sharing the host's existing identity lease and endpoint.
use serde_json::{Value, json};
use std::{
    future::Future,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    task::JoinSet,
};
use wipsnet_core::{config::Config, identity::IdentityStore};
struct State {
    state_dir: PathBuf,
    device_name: String,
    client: Arc<wipsnet_client::Client>,
    authority: wipsnet_registry::host::Authority,
    counts: wipsnet_agent::tunnel::ActiveCounts,
    relay_configured: bool,
}
impl State {
    async fn request(&self, mut value: Value) -> Result<Value, String> {
        let op = value["op"].as_str().ok_or("Missing operation")?.to_owned();
        match op.as_str() {
            "status" => {
                let snapshot = self.authority.snapshot().await.map_err(|e| e.to_string())?;
                let peers = self.counts.peers();
                let mut recent = self.counts.recent_peers();
                // A recent request is display metadata, never durable authority.
                recent
                    .retain(|peer, _| snapshot.clients.iter().any(|c| c.peer.to_string() == *peer));
                let mut pins = Vec::new();
                if let Ok(entries) = std::fs::read_dir(self.state_dir.join("peer-pins")) {
                    for entry in entries.take(129) {
                        let entry = entry.map_err(|_| "Cannot inspect peer pins")?;
                        let name = entry.file_name();
                        let Some(id) = name.to_str().and_then(|s| s.strip_suffix(".json")) else {
                            continue;
                        };
                        let Ok(id) = id.parse() else { continue };
                        if wipsnet_agent::peers::load(&self.state_dir, id)
                            .map_err(|e| e.to_string())?
                            .is_some()
                        {
                            pins.push(id.to_string());
                        }
                    }
                }
                let (invitations, host_name) = self.owner(None).await?;
                Ok(
                    json!({"api_version":1,"host":snapshot,"host_name":host_name,"connected_peers":peers,"recent_peers":recent,"presence_timeout_seconds":wipsnet_agent::tunnel::PRESENCE_TIMEOUT.as_secs(),"transport_streams":self.counts.streams(),"invitations":invitations,"peer_pins":pins,"relay":wipsnet_agent::diagnostics::relay_state(&self.client.endpoint,self.relay_configured),"access_enabled":self.client.gateway.is_enabled()}),
                )
            }
            "apply_operation" => {
                let host = value["host"].as_str().ok_or("Missing target host")?;
                if host != self.client.endpoint.id().to_string() {
                    return Err("Wrong target host".into());
                }
                let id = value["id"]
                    .as_str()
                    .ok_or("Missing operation ID")?
                    .to_owned();
                let action = serde_json::from_value(value["action"].clone())
                    .map_err(|_| "Invalid combined action")?;
                let directory = self.state_dir.clone();
                let host = self.client.endpoint.id();
                let operation_id = id.clone();
                let outcome = tokio::task::spawn_blocking(move || {
                    let (mut store, secret) =
                        wipsnet_registry::host::owner_store(&directory, host)?;
                    store.host_apply_operation(host, &secret, &operation_id, action)
                })
                .await;
                use wipsnet_registry::control::ControlError;
                let mut receipt = match outcome {
                    Ok(Ok(receipt)) => {
                        json!({"state":receipt.state,"revision":receipt.revision,"reason":receipt.reason})
                    }
                    Ok(Err(
                        error @ (ControlError::Invalid
                        | ControlError::Forbidden
                        | ControlError::Conflict
                        | ControlError::RateLimited),
                    )) => {
                        json!({"state":"rejected","revision":null,"reason":error.to_string()})
                    }
                    // A commit may have happened before an I/O/handler failure.
                    // Only a replayed durable receipt can resolve that uncertainty.
                    _ => {
                        json!({"state":"pending","revision":null,"reason":"Target storage unavailable or acknowledgement unconfirmed"})
                    }
                };
                receipt["host"] = json!(host.to_string());
                receipt["id"] = json!(id);
                Ok(receipt)
            }
            "mutate" => {
                let operation = value
                    .get("operation")
                    .ok_or("Missing owner operation")?
                    .clone();
                let revision = self.owner(Some(operation)).await?.0;
                Ok(json!({"revision":revision}))
            }
            "pair" => {
                value["name"] = json!(self.device_name);
                self.client.command(&value).await
            }
            "preview_invitation" | "catalog" | "open" | "enable" | "stop" | "access_status" => {
                self.client.command(&value).await
            }
            _ => Err("Unsupported local operation".into()),
        }
    }
    async fn owner(&self, operation: Option<Value>) -> Result<(Value, String), String> {
        let host = self.client.endpoint.id();
        let directory = self.state_dir.clone();
        let name = self.device_name.clone();
        tokio::task::spawn_blocking(move || {
            let (mut store, secret) =
                wipsnet_registry::host::owner_store(&directory, host).map_err(|e| e.to_string())?;
            let result = if let Some(operation) = operation {
                let op =
                    serde_json::from_value(operation).map_err(|_| "Invalid owner operation")?;
                json!(
                    store
                        .host_mutate(host, &secret, op)
                        .map_err(|e| e.to_string())?
                )
            } else {
                json!(
                    store
                        .pairing_list(host, &secret, wipsnet_registry::enrollment::now())
                        .map_err(|e| e.to_string())?
                )
            };
            Ok((result, name))
        })
        .await
        .map_err(|_| "Local owner operation failed".to_owned())?
    }
}
async fn serve(mut stream: UnixStream, state: Arc<State>) {
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(25), async {
        if stream.peer_cred()?.uid() != unsafe { libc::geteuid() } {
            return Err(std::io::Error::other("Wrong local user"));
        }
        let mut request = Vec::new();
        let mut byte = [0; 1];
        while request.len() <= 16384 {
            if stream.read(&mut byte).await? == 0 {
                break;
            }
            if byte[0] == b'\n' {
                break;
            }
            request.push(byte[0]);
        }
        let result = if request.len() > 16384 {
            Err("Local request too large".into())
        } else {
            match serde_json::from_slice(&request) {
                Ok(value) => state.request(value).await,
                Err(_) => Err("Invalid local request".into()),
            }
        };
        let response = match result {
            Ok(result) => json!({"ok":true,"result":result}),
            Err(error) => json!({"ok":false,"error":error}),
        };
        let mut bytes = serde_json::to_vec(&response)?;
        bytes.push(b'\n');
        stream.write_all(&bytes).await?;
        Ok::<_, std::io::Error>(())
    })
    .await;
    let _ = outcome;
}
struct SocketPath(PathBuf);
impl Drop for SocketPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub async fn run(
    config: &Config,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Box<dyn std::error::Error>> {
    if config.schema_version != 2 {
        return wipsnet_agent::run(config, shutdown).await;
    }
    let store = wipsnet_agent::identity::FileIdentityStore::new(&config.state_dir)?;
    let _lease = store.lease()?;
    let identity = store.load()?;
    let authority =
        wipsnet_registry::host::Authority::open(&config.state_dir, identity.endpoint_id())?;
    let transport = config.transport.as_ref().ok_or("Host transport required")?;
    let host = wipsnet_agent::tunnel::TunnelHost::bind_host(
        identity.secret_key(),
        transport,
        authority.clone(),
    )
    .await?;
    let client = wipsnet_client::Client::from_endpoint(
        host.endpoint().clone(),
        identity.secret_key().clone(),
        config.state_dir.clone(),
        transport.clone(),
        authority.clone(),
    );
    let path = config.state_dir.join("node.sock");
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if !metadata.file_type().is_socket() {
            return Err("Refusing to replace non-socket node API path".into());
        }
        if UnixStream::connect(&path).await.is_ok() {
            return Err("Node API already running".into());
        }
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let _socket = SocketPath(path);
    let state = Arc::new(State {
        state_dir: config.state_dir.clone(),
        device_name: config.device_name.clone(),
        client: client.clone(),
        authority,
        counts: host.counts(),
        relay_configured: transport.relay_url.is_some(),
    });
    let mut tasks = JoinSet::new();
    tokio::pin!(shutdown);
    tracing::info!(component="agent",event="started",networking=true,mode="independent_host",endpoint_id=%host.endpoint().id(),bind_addr=%host.endpoint().bound_sockets().first().map(ToString::to_string).unwrap_or_else(||"relay-only".into()));
    loop {
        tokio::select! {_= &mut shutdown=>break,Some(_)=tasks.join_next(),if !tasks.is_empty()=>{},accepted=listener.accept()=>{let(stream,_)=accepted?;if tasks.len()<32{tasks.spawn(serve(stream,state.clone()));}}}
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    client.gateway.stop().await;
    drop(state);
    drop(client);
    host.shutdown().await;
    tracing::info!(component = "agent", event = "stopped");
    Ok(())
}
