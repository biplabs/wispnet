use serde::Deserialize;
use std::{
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use wispnet_core::{config::TransportConfig, identity::IdentityStore};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    state_dir: PathBuf,
    listen: std::net::SocketAddr,
    domain: String,
    publishers: Vec<wispnet_core::identity::EndpointId>,
    transport: TransportConfig,
}
fn read_config(path: &Path) -> Result<Config, Box<dyn std::error::Error>> {
    if !path.is_absolute() {
        return Err("Public gateway configuration path must be absolute".into());
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
        || metadata.len() > 65536
    {
        return Err("Public gateway configuration must be protected, regular and mode 600".into());
    }
    let mut text = String::new();
    file.take(65537).read_to_string(&mut text)?;
    Ok(toml::from_str(&text)?)
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: wispnet-public-gateway --init /absolute/state OR --config /absolute/config.toml".into());
    }
    if args[0] == "--init" {
        let state = PathBuf::from(&args[1]);
        if !state.is_absolute() {
            return Err("State directory must be absolute".into());
        }
        if !state.exists() {
            std::fs::create_dir_all(&state)?;
            std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700))?;
        }
        let identity = wispnet_agent::identity::FileIdentityStore::new(&state)?.initialize()?;
        println!("{}", identity.endpoint_id());
        return Ok(());
    }
    if args[0] != "--config" {
        return Err("Unsupported operation".into());
    }
    let config = read_config(Path::new(&args[1]))?;
    if !config.listen.ip().is_loopback() || config.listen.port() == 0 {
        return Err("Public HTTP gateway must bind loopback behind HTTPS".into());
    }
    let store = wispnet_agent::identity::FileIdentityStore::new(&config.state_dir)?;
    let _lease = store.lease()?;
    let identity = store.load()?;
    let endpoint = wispnet_agent::tunnel::bind_configured_client(
        identity.secret_key(),
        Some(&config.transport),
    )
    .await?;
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    let ingress =
        wispnet_client::public_access::ingress::Ingress::new(config.domain, config.publishers)?;
    println!("Public ingress ready; endpoint {}", identity.endpoint_id());
    let task = tokio::spawn(ingress.run(endpoint.clone(), listener));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {_=term.recv()=>{},_=tokio::signal::ctrl_c()=>{}}
    task.abort();
    let _ = task.await;
    endpoint.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsafe_gateway_configuration_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("gateway.toml");
        std::fs::write(&path, "not a config").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(
            read_config(&path)
                .err()
                .unwrap()
                .to_string()
                .contains("protected")
        );
    }
}
