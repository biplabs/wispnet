//! Read-only reachability/path snapshots. Never opens a backend or retries a service.
use crate::{control, tunnel::TunnelError};
use iroh::{Endpoint, EndpointAddr, Watcher, endpoint::Connection};
use std::time::Duration;
use tokio::time::{Instant, sleep};
use wipsnet_core::identity::EndpointId;

struct Close(Connection);
impl Drop for Close {
    fn drop(&mut self) {
        self.0.close(0u32.into(), b"diagnostic complete");
    }
}

/// Observed selected path, not a prediction of future connectivity.
pub fn selected_path(connection: &Connection) -> &'static str {
    for path in connection.paths().iter() {
        if path.is_selected() {
            return if path.is_ip() {
                "direct"
            } else if path.is_relay() {
                "relay"
            } else {
                "unknown"
            };
        }
    }
    "unknown"
}

/// Passive QUIC round-trip time estimate (in milliseconds) on the selected path.
pub fn selected_rtt_ms(connection: &Connection) -> Option<u64> {
    for path in connection.paths().iter() {
        if path.is_selected() {
            let rtt = path.rtt();
            if !rtt.is_zero() {
                return Some((rtt.as_millis() as u64).max(1));
            }
        }
    }
    None
}

/// Fixed labels only: relay-provided errors and reasons are untrusted text.
pub fn relay_state(endpoint: &Endpoint, configured: bool) -> &'static str {
    if !configured {
        return "disabled";
    }
    let states = endpoint.home_relay_status().get();
    if states.iter().any(|s| s.is_connected()) {
        "connected"
    } else if states.iter().any(|s| s.auth_denied_reason().is_some()) {
        "denied"
    } else {
        "unavailable"
    }
}

/// Authenticates the expected host and fetches its permitted catalog first. Gives
/// an available direct path up to one second to replace an initial relay path.
/// Retains no records and exposes neither addresses, grants nor service payloads.
pub async fn probe(
    endpoint: &Endpoint,
    expected: EndpointId,
    target: EndpointAddr,
    relay_configured: bool,
) -> Result<serde_json::Value, TunnelError> {
    if target.id != expected {
        return Err(TunnelError::Transport(
            "diagnostic target identity mismatch",
        ));
    }
    let started = Instant::now();
    let connection = Close(control::connect_target(endpoint, target).await?);
    let catalog = control::catalog_on(&connection.0, expected).await?;
    let until = Instant::now() + Duration::from_secs(1);
    while selected_path(&connection.0) != "direct" && Instant::now() < until {
        if connection.0.close_reason().is_some() {
            return Err(TunnelError::Transport(
                "peer closed during path observation",
            ));
        }
        sleep(Duration::from_millis(20)).await;
    }
    Ok(serde_json::json!({
        "event":"probe", "ok":true, "host":expected, "client":endpoint.id(),
        "selected_path":selected_path(&connection.0), "relay_status":relay_state(endpoint, relay_configured),
        "generation":catalog.generation, "revision":catalog.revision, "service_count":catalog.services.len(),
        "elapsed_ms":started.elapsed().as_millis()
    }))
}
