//! Persistent single-owner registry with an authenticated loopback API.
pub mod api;
pub mod client;
pub mod control;
pub mod enrollment;
pub mod host;
pub mod pairing;
pub mod store;
use std::future::Future;
use wipsnet_core::config::Config;

/// Open and check persistent storage before announcing readiness.
/// A configured API listener requires explicit owner bootstrap first.
pub async fn run(
    config: &Config,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), store::Error> {
    let mut store = store::Store::open(&config.state_dir.join("registry.sqlite3"))?;
    if !store.ready()? {
        return Err(store::Error::Storage);
    }
    let revision = store.snapshot()?.revision;
    if let Some(network) = &config.registry {
        if !store.initialized()? {
            return Err(store::Error::InvalidInput);
        }
        let listener = tokio::net::TcpListener::bind(network.bind_addr)
            .await
            .map_err(|_| store::Error::Storage)?;
        tracing::info!(
            component = "registry",
            event = "started",
            networking = true,
            revision
        );
        axum::serve(listener, api::router(store))
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|_| store::Error::Storage)?;
        tracing::info!(component = "registry", event = "stopped");
        return Ok(());
    }
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
