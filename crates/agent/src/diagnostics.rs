//! Read-only reachability/path snapshots. Never opens a backend or retries a service.
use crate::{control, tunnel::TunnelError};
use iroh::{Endpoint, EndpointAddr, Watcher, endpoint::Connection};
use std::time::Duration;
use tokio::time::{Instant, sleep};
use wispnet_core::identity::EndpointId;

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

/// Keep a short successful request alive while Iroh negotiates a direct path.
/// Direct connections return immediately; relay-only peers add at most one second.
/// A closed connection cannot negotiate further and must not delay its caller.
pub async fn settle_path(connection: &Connection) {
    let until = Instant::now() + Duration::from_secs(1);
    while selected_path(connection) != "direct"
        && connection.close_reason().is_none()
        && Instant::now() < until
    {
        sleep(Duration::from_millis(20)).await;
    }
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
    settle_path(&connection.0).await;
    if connection.0.close_reason().is_some() {
        return Err(TunnelError::Transport(
            "peer closed during path observation",
        ));
    }
    Ok(serde_json::json!({
        "event":"probe", "ok":true, "host":expected, "client":endpoint.id(),
        "selected_path":selected_path(&connection.0), "relay_status":relay_state(endpoint, relay_configured),
        "generation":catalog.generation, "revision":catalog.revision, "service_count":catalog.services.len(),
        "elapsed_ms":started.elapsed().as_millis()
    }))
}

/// A longer, explicit user-requested observation. No service backend is opened.
pub async fn explain_path(
    endpoint: &Endpoint,
    expected: EndpointId,
    target: EndpointAddr,
    relay_only: bool,
    relay_configured: bool,
) -> Result<serde_json::Value, TunnelError> {
    if target.id != expected {
        return Err(TunnelError::Transport(
            "diagnostic target identity mismatch",
        ));
    }
    let connection = Close(control::connect_target(endpoint, target).await?);
    control::catalog_on(&connection.0, expected).await?;
    let initial = selected_path(&connection.0);
    let started = Instant::now();
    while !relay_only
        && selected_path(&connection.0) != "direct"
        && connection.0.close_reason().is_none()
        && started.elapsed() < Duration::from_secs(10)
    {
        sleep(Duration::from_millis(100)).await;
    }
    let closed = connection.0.close_reason().is_some();
    let selected = if closed {
        "closed"
    } else {
        selected_path(&connection.0)
    };
    let report = endpoint.net_report().get();
    let network = report.map(|r| {
        serde_json::json!({
            "udp_v4":r.udp_v4, "udp_v6":r.udp_v6,
            "mapping_varies":r.mapping_varies_by_dest(),
            "public_ipv4_discovered":r.global_v4.is_some(),
            "public_ipv6_discovered":r.global_v6.is_some(),
            "captive_portal":r.captive_portal
        })
    });
    Ok(serde_json::json!({
        "initial_path":initial, "selected_path":selected,
        "rtt_ms":selected_rtt_ms(&connection.0),
        "observation_ms":started.elapsed().as_millis(),
        "relay_only":relay_only, "relay_status":relay_state(endpoint,relay_configured),
        "network_report":network,
        "reason":match selected {
            "direct" => "Direct connectivity succeeded.",
            "closed" => "Peer closed the connection during observation.",
            _ if relay_only => "This device is configured to use relay only.",
            _ => "No direct path was established within 10 seconds. NAT or firewall restrictions at either end are possible; this test cannot identify which network is responsible."
        }
    }))
}
