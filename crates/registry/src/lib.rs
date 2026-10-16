//! Persistent registry lifecycle. Network API authentication is not implemented.
pub mod store;
use std::future::Future;
use wipsnet_core::config::Config;

/// Open and check persistent storage before announcing readiness.
/// No network listener is opened until the authenticated API is implemented.
pub async fn run(config: &Config, shutdown: impl Future<Output = ()>) -> Result<(), store::Error> {
    let mut store = store::Store::open(&config.state_dir.join("registry.sqlite3"))?;
    if !store.ready()? {
        return Err(store::Error::Storage);
    }
    let revision = store.snapshot()?.revision;
    tracing::info!(
        component = "registry",
        event = "started",
        networking = false,
        revision
    );
    shutdown.await;
    drop(store);
    tracing::info!(component = "registry", event = "stopped");
    Ok(())
}
