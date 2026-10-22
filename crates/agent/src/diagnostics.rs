//! Read-only reachability/path snapshots. Never opens a backend or retries a service.
use crate::{control, tunnel::TunnelError};
use iroh::endpoint::{PathEvent, PathStats};
use iroh::{Endpoint, EndpointAddr, Watcher, endpoint::Connection};
use n0_future::StreamExt;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::time::{Instant, sleep};
use wispnet_core::host::{DiagnosticSnapshot, NetworkObservation, NetworkReport, PathObservation};
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

#[derive(Clone)]
pub(crate) struct Context {
    pub endpoint: Endpoint,
    pub relay_only: bool,
    pub relay_configured: bool,
}

pub fn network_report(endpoint: &Endpoint) -> Option<NetworkReport> {
    endpoint.net_report().get().map(|r| NetworkReport {
        udp_v4: r.udp_v4,
        udp_v6: r.udp_v6,
        mapping_varies: r.mapping_varies_by_dest(),
        public_ipv4_discovered: r.global_v4.is_some_and(|a| public_ip((*a.ip()).into())),
        public_ipv6_discovered: r.global_v6.is_some_and(|a| public_ip((*a.ip()).into())),
        captive_portal: r.captive_portal,
    })
}

/// Wait for an actual report publication after subscribing. Merely notifying
/// Iroh of a possible network change does not guarantee it reruns discovery.
async fn fresh_network(context: &Context) -> NetworkObservation {
    let started = Instant::now();
    if context.relay_only || !context.relay_configured {
        return NetworkObservation {
            fresh: false,
            report: network_report(&context.endpoint),
            wait_ms: 0,
        };
    }
    let mut updates = context.endpoint.net_report().stream();
    let _baseline = updates.next().await;
    context.endpoint.network_change().await;
    let update = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(report) = updates.next().await {
            if report.is_some() {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    NetworkObservation {
        fresh: update,
        report: network_report(&context.endpoint),
        wait_ms: started.elapsed().as_millis() as u64,
    }
}

fn record_event(
    event: PathEvent,
    direct: &mut BTreeMap<String, PathStats>,
    closed: &mut BTreeSet<String>,
    events_lost: &mut u64,
) {
    match event {
        PathEvent::Opened {
            id, remote_addr, ..
        } if remote_addr.is_ip() => {
            direct.entry(format!("{id:?}")).or_default();
        }
        PathEvent::Closed {
            id,
            remote_addr,
            last_stats,
            ..
        } if remote_addr.is_ip() => {
            let key = format!("{id:?}");
            closed.insert(key.clone());
            direct.insert(key, *last_stats);
        }
        PathEvent::Lagged { missed, .. } => *events_lost = events_lost.saturating_add(missed),
        _ => {}
    }
}

/// Reports only connection-scoped counters and candidate counts, never addresses.
/// Failed candidates that never became paths are not exposed by Iroh's path API.
pub(crate) async fn observe(context: &Context, connection: &Connection) -> DiagnosticSnapshot {
    observe_until(context, connection, None).await
}

async fn observe_until(
    context: &Context,
    connection: &Connection,
    remote_finished: Option<tokio::sync::watch::Receiver<bool>>,
) -> DiagnosticSnapshot {
    let started = Instant::now();
    let initial = selected_path(connection).to_owned();
    let mut events = connection.path_events();
    let mut events_open = true;
    let mut direct: BTreeMap<String, PathStats> = BTreeMap::new();
    let mut closed = BTreeSet::new();
    let mut events_lost = 0u64;
    let network_future = fresh_network(context);
    tokio::pin!(network_future);
    let mut network = None;
    loop {
        for path in connection.paths().iter().filter(|p| p.is_ip()) {
            direct.insert(format!("{:?}", path.id()), path.stats());
        }
        let path_done = context.relay_only
            || selected_path(connection) == "direct"
            || started.elapsed() >= Duration::from_secs(10);
        let remote_done = remote_finished
            .as_ref()
            .is_none_or(|finished| *finished.borrow());
        if (path_done && network.is_some() && remote_done) || connection.close_reason().is_some() {
            break;
        }
        tokio::select! {
            value = &mut network_future, if network.is_none() => network = Some(value),
            event = events.next(), if events_open => match event {
                Some(event) => record_event(event,&mut direct,&mut closed,&mut events_lost),
                None => events_open = false,
            },
            _ = sleep(Duration::from_millis(100)) => {},
        }
    }
    // Include events already queued when the observation deadline completed.
    for _ in 0..64 {
        let mut next = std::pin::pin!(events.next());
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let std::task::Poll::Ready(Some(event)) = std::future::Future::poll(next.as_mut(), &mut cx)
        else {
            break;
        };
        record_event(event, &mut direct, &mut closed, &mut events_lost);
    }
    let stats = connection.stats();
    let mut paths = PathObservation {
        initial_path: initial,
        selected_path: if connection.close_reason().is_some() {
            "closed".into()
        } else {
            selected_path(connection).into()
        },
        rtt_ms: selected_rtt_ms(connection),
        direct_paths_seen: direct.len(),
        direct_paths_closed: closed.len(),
        events_lost,
        connection_challenges_sent: stats.frame_tx.path_challenge,
        connection_responses_received: stats.frame_rx.path_response,
        ..Default::default()
    };
    for stats in direct.values() {
        paths.direct_datagrams_sent = paths
            .direct_datagrams_sent
            .saturating_add(stats.udp_tx.datagrams);
        paths.direct_datagrams_received = paths
            .direct_datagrams_received
            .saturating_add(stats.udp_rx.datagrams);
        paths.direct_challenges_sent = paths
            .direct_challenges_sent
            .saturating_add(stats.frame_tx.path_challenge);
        paths.direct_responses_received = paths
            .direct_responses_received
            .saturating_add(stats.frame_rx.path_response);
    }
    let candidates = context
        .endpoint
        .remote_info(connection.remote_id())
        .await
        .map(|info| info.addrs().filter(|a| a.addr().is_ip()).count());
    DiagnosticSnapshot {
        version: 1,
        host: context.endpoint.id(),
        relay_only: context.relay_only,
        relay_status: relay_state(&context.endpoint, context.relay_configured).into(),
        network: network.unwrap_or(NetworkObservation {
            fresh: false,
            report: network_report(&context.endpoint),
            wait_ms: started.elapsed().as_millis() as u64,
        }),
        paths,
        peer_ip_candidates: candidates,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

/// Observe both authenticated endpoints without opening a service backend.
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
    let context = Context {
        endpoint: endpoint.clone(),
        relay_only,
        relay_configured,
    };
    let (finished, finished_watch) = tokio::sync::watch::channel(false);
    let (local, remote) = tokio::join!(
        observe_until(&context, &connection.0, Some(finished_watch)),
        async {
            let result = control::diagnostic_on(&connection.0, expected).await;
            let _ = finished.send(true);
            result
        }
    );
    let reason = if local.paths.selected_path == "direct" {
        "Direct connectivity succeeded."
    } else if local.paths.selected_path == "closed" {
        "Peer closed the connection during observation."
    } else if relay_only {
        "This device is configured to use relay only."
    } else if remote.as_ref().is_ok_and(|r| r.relay_only) {
        "The target computer is configured to use relay only."
    } else if local.network.fresh
        && local
            .network
            .report
            .as_ref()
            .is_some_and(|r| r.udp_v4 || r.udp_v6)
    {
        "UDP discovery works; direct peer connectivity failed during this test. NAT or firewall restrictions remain possible at either end."
    } else {
        "Direct peer connectivity failed during this test. Discovery freshness and both endpoints' observations are needed to narrow the cause."
    };
    Ok(serde_json::json!({
        "initial_path":local.paths.initial_path, "selected_path":local.paths.selected_path,
        "rtt_ms":local.paths.rtt_ms, "observation_ms":local.elapsed_ms,
        "relay_only":relay_only, "relay_status":local.relay_status,
        "network_report":local.network.report, "reason":reason,
        "local":local, "remote":remote.as_ref().ok(),
        "remote_error":remote.as_ref().err().map(|_| "Host diagnostic unavailable: the host may need an update, or its diagnostic request failed."),
        "candidate_failure_reasons":"unavailable: transport does not expose reasons for failed candidate validation",
    }))
}

/// Address advertisement and coordinated hole-punch frame counters from the
/// authenticated connection. Counts include retransmissions, not unique addresses.
/// Exposed only through owner-local IPC, never raw addresses or application data.
pub(crate) fn connection_counters(connection: &Connection) -> serde_json::Value {
    let stats = connection.stats();
    serde_json::json!({
        "connection_id":connection.stable_id(),
        "path":selected_path(connection),
        "closed":connection.close_reason().is_some(),
        "address_advertisements_sent":stats.frame_tx.add_address,
        "address_advertisements_received":stats.frame_rx.add_address,
        "address_removals_sent":stats.frame_tx.remove_address,
        "address_removals_received":stats.frame_rx.remove_address,
        "holepunch_requests_sent":stats.frame_tx.reach_out,
        "holepunch_requests_received":stats.frame_rx.reach_out,
        "path_challenges_sent":stats.frame_tx.path_challenge,
        "path_challenges_received":stats.frame_rx.path_challenge,
        "path_responses_sent":stats.frame_tx.path_response,
        "path_responses_received":stats.frame_rx.path_response,
        "direct_paths":connection.paths().iter().filter(|p|p.is_ip()).count(),
    })
}

/// Exclude private and special-use addresses from the diagnostic's public-IP
/// check. A true result describes address scope, not verified reachability.
pub fn public_ip(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(ip) => {
            let [a, b, c, d] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_documentation()
                || ip.is_broadcast()
                || a == 0
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 198 && (b == 18 || b == 19))
                || (a == 192 && b == 0 && c == 0 && d != 9 && d != 10)
                || (a == 192 && b == 88 && c == 99))
        }
        std::net::IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_ip(v4.into());
            }
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && !(s[0] == 0x2001 && (s[1] == 0xdb8 || (s[1] == 2 && s[2] == 0)))
                && !(s[0] == 0x3fff && s[1] & 0xf000 == 0)
        }
    }
}

/// Owner-visible relay and discovery summaries, without IP addresses or payloads.
pub fn transport_observations(endpoint: &Endpoint) -> serde_json::Value {
    let homes = endpoint.home_relay_status().get();
    let relays: Vec<_> = endpoint
        .relay_connections()
        .iter()
        .map(|s| {
            serde_json::json!({
                "url":s.url(), "connected":s.is_connected(),
                "preferred":homes.iter().any(|h|h.url()==s.url()),
            })
        })
        .collect();
    let report = endpoint.net_report().get();
    let discovery: Vec<_> = report
        .iter()
        .flat_map(|r| r.discovery_addresses.iter())
        .map(|(url, addresses)| {
            serde_json::json!({
                "url":url, "ipv4":addresses.iter().any(|a|a.is_ipv4()),
                "ipv6":addresses.iter().any(|a|a.is_ipv6()),
                "public_ipv4":addresses.iter().any(|a|a.is_ipv4()&&public_ip(a.ip())),
                "public_ipv6":addresses.iter().any(|a|a.is_ipv6()&&public_ip(a.ip())),
            })
        })
        .collect();
    serde_json::json!({"relays":relays,"discovery":discovery,
        "direct_ip_candidates":endpoint.addr().ip_addrs().count()})
}

#[cfg(test)]
mod address_scope_tests {
    use super::public_ip;
    #[test]
    fn discovery_addresses_are_not_necessarily_public() {
        for ip in [
            "192.168.1.1",
            "10.1.2.3",
            "172.16.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "0.0.0.0",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "::ffff:192.168.1.1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["162.156.47.159", "1.1.1.1", "2606:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }
}
