//! Agent identity and authenticated service-host lifecycle.
use std::future::Future;
use wipsnet_core::config::Config;
pub mod identity;
pub mod tunnel;

/// Own and drain component tasks here as features are added. No detached tasks.
pub async fn run(
    config: &Config,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Box<dyn std::error::Error>> {
    use wipsnet_core::identity::IdentityStore;
    let store = identity::FileIdentityStore::new(&config.state_dir)?;
    let _lease = store.lease()?;
    let identity = store.load()?;
    let host = if let Some(transport) = &config.transport {
        Some(tunnel::TunnelHost::bind(identity.secret_key(), transport, &config.services).await?)
    } else {
        None
    };
    if let Some(host) = &host {
        tracing::info!(component = "agent", event = "started", networking = true,
            endpoint_id = %host.endpoint().id(), bind_addr = %host.endpoint().bound_sockets()[0]);
    } else {
        tracing::info!(component = "agent", event = "started", networking = false);
    }
    shutdown.await;
    if let Some(host) = host {
        host.shutdown().await;
    }
    tracing::info!(component = "agent", event = "stopped");
    Ok(())
}
