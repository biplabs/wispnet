//! Agent lifecycle boundary. Networking is not implemented.
use std::future::Future;
use wipsnet_core::config::Config;

/// Own and drain component tasks here as features are added. No detached tasks.
pub async fn run(_config: &Config, shutdown: impl Future<Output = ()>) {
    tracing::info!(component = "agent", event = "started", networking = false);
    shutdown.await;
    tracing::info!(component = "agent", event = "stopped");
}
