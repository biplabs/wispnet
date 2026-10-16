//! Agent lifecycle boundary. Networking is not implemented.
use std::future::Future;
use wipsnet_core::config::Config;
pub mod identity;

/// Own and drain component tasks here as features are added. No detached tasks.
pub async fn run(
    config: &Config,
    shutdown: impl Future<Output = ()>,
) -> Result<(), wipsnet_core::identity::IdentityError> {
    use wipsnet_core::identity::IdentityStore;
    let store = identity::FileIdentityStore::new(&config.state_dir)?;
    let _lease = store.lease()?;
    let _identity = store.load()?;
    tracing::info!(component = "agent", event = "started", networking = false);
    shutdown.await;
    tracing::info!(component = "agent", event = "stopped");
    Ok(())
}
