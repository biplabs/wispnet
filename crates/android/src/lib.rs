//! Native Android client. One identity/endpoint shared by pairing, catalogs and gateway.
pub mod gateway;
use iroh::Endpoint;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::runtime::Runtime;
use wipsnet_core::{
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
    pub async fn new(
        key: SecretKey,
        state: PathBuf,
        config: TransportConfig,
    ) -> Result<Arc<Self>, String> {
        config
            .validate()
            .map_err(|_| "Invalid relay configuration")?;
        let endpoint = wipsnet_agent::tunnel::bind_configured_client(&key, Some(&config))
            .await
            .map_err(|e| e.to_string())?;
        Ok(Arc::new(Self {
            endpoint,
            key,
            config,
            state,
            gateway: gateway::Gateways::default(),
        }))
    }
    pub async fn catalog(&self, host: EndpointId) -> Result<Value, String> {
        let target = wipsnet_agent::tunnel::peer_target(host, Some(&self.config), None)
            .map_err(|e| e.to_string())?;
        let connection = wipsnet_agent::control::connect_target(&self.endpoint, target)
            .await
            .map_err(|e| e.to_string())?;
        let result = wipsnet_agent::control::catalog_on(&connection, host)
            .await
            .map_err(|e| e.to_string());
        connection.close(0u32.into(), b"catalog complete");
        serde_json::to_value(result?).map_err(|_| "Invalid catalog".into())
    }
    pub fn target(&self, host: EndpointId) -> Result<iroh::EndpointAddr, String> {
        wipsnet_agent::tunnel::peer_target(host, Some(&self.config), None)
            .map_err(|e| e.to_string())
    }
    async fn command(self: &Arc<Self>, value: &Value) -> Result<Value, String> {
        match argument(value, "op")? {
            "identity" => Ok(json!({"peer":self.endpoint.id().to_string()})),
            "pair" => {
                let host = peer(value)?;
                let text = argument(value, "invitation")?;
                if text.len() > 8192 {
                    return Err("Invitation too large".into());
                }
                let invitation: wipsnet_core::pairing::Invitation =
                    serde_json::from_str(text).map_err(|_| "Invalid invitation")?;
                let bootstrap = invitation
                    .bootstrap(host, wipsnet_registry::enrollment::now())
                    .ok_or("Expired invitation or wrong host pin")?;
                let pending = wipsnet_agent::pairing::request_enrollment(
                    &self.endpoint,
                    host,
                    &invitation,
                    &self.config,
                    &self.key,
                    "Wispnet Android",
                )
                .await
                .map_err(|e| e.to_string())?;
                wipsnet_agent::peers::save(&self.state, &bootstrap)
                    .map_err(|_| "Cannot save host pin")?;
                Ok(json!({"state":"pending_owner_approval","host":pending.host.to_string()}))
            }
            "catalog" => {
                let host = peer(value)?;
                if wipsnet_agent::peers::load(&self.state, host)
                    .map_err(|_| "Invalid saved host")?
                    .is_none()
                {
                    return Err("Pair with this host first".into());
                }
                self.catalog(host).await
            }
            "enable" => {
                self.gateway.enable();
                Ok(json!({"enabled":true}))
            }
            "stop" => {
                self.gateway.stop().await;
                Ok(json!({"enabled":false}))
            }
            "network" => {
                self.endpoint.network_change().await;
                Ok(json!({"network_changed":true}))
            }
            "open" => {
                let host = peer(value)?;
                if wipsnet_agent::peers::load(&self.state, host)
                    .map_err(|_| "Invalid saved host")?
                    .is_none()
                {
                    return Err("Pair with this host first".into());
                }
                let id = argument(value, "service")?;
                let catalog = self.catalog(host).await?;
                if !catalog["services"]
                    .as_array()
                    .is_some_and(|s| s.iter().any(|s| s["id"] == id))
                {
                    return Err("Service no longer permitted".into());
                }
                let url = self.gateway.open(self.clone(), host, id.to_owned()).await?;
                Ok(json!({"url":url}))
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
        Err(error) => json!({"ok":false,"error":error}).to_string(),
    }
}
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_biplabs_wispnet_Native_call(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    input: jni::objects::JString,
) -> jni::sys::jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let input: zeroize::Zeroizing<String> =
            zeroize::Zeroizing::new(env.get_string(&input).map(|s| s.into()).unwrap_or_default());
        call(&input)
    }))
    .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"Native operation failed\"}".into());
    env.new_string(result)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
