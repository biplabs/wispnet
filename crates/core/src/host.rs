//! Host-scoped control contract. Transport identity, never a caller field, is authority.
use crate::identity::EndpointId;
use serde::{Deserialize, Serialize};
// Legacy wire spelling is frozen for compatibility with installed Wispnet clients.
pub const ALPN: &[u8] = b"wipsnet/control/1";
pub const MAX_REQUEST: usize = 4096;
pub const MAX_RESPONSE: usize = 128 * 1024;
#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Catalog {},
    Diagnostic {},
    SetName { name: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub version: u16,
    pub complete: bool,
    pub service_count: u16,
    pub host: EndpointId,
    pub generation: String,
    pub revision: u64,
    pub services: Vec<Service>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub id: String,
    pub incarnation: String,
    pub display_name: String,
    pub kind: ServiceKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    #[default]
    Http,
    Tcp,
}
impl Catalog {
    /// Only complete, canonical owner-filtered snapshots can advance a baseline.
    pub fn valid(&self, expected: EndpointId) -> bool {
        self.version == 1
            && self.complete
            && self.host == expected
            && crate::enrollment::valid_code(&self.generation)
            && self.revision <= i64::MAX as u64
            && self.services.len() <= 128
            && usize::from(self.service_count) == self.services.len()
            && self.services.windows(2).all(|pair| pair[0].id < pair[1].id)
            && self.services.iter().all(|s| {
                crate::protocol::valid_service_id(&s.id)
                    && crate::enrollment::valid_code(&s.incarnation)
                    && crate::enrollment::valid_name(&s.display_name)
            })
    }
    pub fn follows(&self, previous: &Self) -> bool {
        self.host == previous.host
            && self.generation == previous.generation
            && (self.revision > previous.revision || self == previous)
    }
}

/// Address-free observations, disclosed only to an authenticated approved peer.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticSnapshot {
    pub version: u16,
    pub host: EndpointId,
    pub relay_only: bool,
    pub relay_status: String,
    pub network: NetworkObservation,
    pub paths: PathObservation,
    /// Legacy wire name: count of IP entries in Iroh's remote address cache.
    /// This is not the QUIC NAT traversal candidate list.
    pub peer_ip_candidates: Option<usize>,
    pub elapsed_ms: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkObservation {
    pub fresh: bool,
    pub report: Option<NetworkReport>,
    pub wait_ms: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkReport {
    pub udp_v4: bool,
    pub udp_v6: bool,
    pub mapping_varies: Option<bool>,
    pub public_ipv4_discovered: bool,
    pub public_ipv6_discovered: bool,
    pub captive_portal: Option<bool>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PathObservation {
    pub initial_path: String,
    pub selected_path: String,
    pub rtt_ms: Option<u64>,
    pub direct_paths_seen: usize,
    pub direct_paths_closed: usize,
    pub direct_datagrams_sent: u64,
    pub direct_datagrams_received: u64,
    pub direct_challenges_sent: u64,
    pub direct_responses_received: u64,
    pub connection_challenges_sent: u64,
    pub connection_responses_received: u64,
    pub events_lost: u64,
}
impl DiagnosticSnapshot {
    pub fn valid(&self, expected: EndpointId) -> bool {
        self.version == 1
            && self.host == expected
            && matches!(
                self.relay_status.as_str(),
                "disabled" | "connected" | "denied" | "unavailable"
            )
            && [&self.paths.initial_path, &self.paths.selected_path]
                .iter()
                .all(|p| matches!(p.as_str(), "direct" | "relay" | "unknown" | "closed"))
            && (!self.network.fresh || self.network.report.is_some())
            && self.elapsed_ms <= 40_000
            && self.network.wait_ms <= 31_000
            && self.peer_ip_candidates.is_none_or(|count| count <= 1024)
    }
}
