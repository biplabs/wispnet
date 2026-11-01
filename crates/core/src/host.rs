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
    RouterMapping {},
    SetName { name: String },
    DeviceId {},
    PublicLinks {},
}

/// Optional display metadata, separate from the revisioned permission catalog.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicLink {
    pub service: String,
    pub incarnation: String,
    pub url: String,
    pub state: String,
}
impl PublicLink {
    pub fn valid(&self) -> bool {
        crate::protocol::valid_service_id(&self.service)
            && crate::enrollment::valid_code(&self.incarnation)
            && matches!(self.state.as_str(), "online" | "connecting" | "offline")
            && self.url.len() <= 2048
            && url::Url::parse(&self.url).is_ok_and(|u| {
                u.scheme() == "https"
                    && matches!(u.host(), Some(url::Host::Domain(_)))
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.port().is_none()
                    && u.path() == "/"
                    && u.query().is_none()
                    && u.fragment().is_none()
                    && u.as_str() == self.url
            })
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicLinks {
    pub version: u16,
    pub host: EndpointId,
    pub services: Vec<PublicLink>,
}
impl PublicLinks {
    pub fn valid(&self, expected: EndpointId) -> bool {
        self.version == 1
            && self.host == expected
            && self.services.len() <= 16
            && self.services.iter().all(PublicLink::valid)
            && self
                .services
                .windows(2)
                .all(|s| s[0].service < s[1].service)
    }
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
    /// Built-in PTY service; no TCP backend listener.
    Shell,
    /// Built-in resumable binary file service; no TCP backend listener.
    Files,
    /// Owner-selected read-only file share with an independent grant list.
    FileShare,
    /// Owner-selected upload-only folder with independent device grants.
    FileInbox,
}
impl Catalog {
    /// Only complete, canonical owner-filtered snapshots can advance a baseline.
    pub fn valid(&self, expected: EndpointId) -> bool {
        self.version == 1
            && self.complete
            && self.host == expected
            && crate::enrollment::valid_code(&self.generation)
            && self.revision <= i64::MAX as u64
            && self.services.len() <= 162
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

/// Address-free automatic mapping status, available only to approved peers.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouterMappingReport {
    pub version: u16,
    pub host: EndpointId,
    pub enabled: bool,
    pub active: bool,
    pub external_port: Option<u16>,
    pub public_address: Option<bool>,
    pub upnp: Option<bool>,
    pub pcp: Option<bool>,
    pub nat_pmp: Option<bool>,
}
impl RouterMappingReport {
    pub fn valid(&self, expected: EndpointId) -> bool {
        self.version == 1
            && self.host == expected
            && self.active == self.external_port.is_some()
            && self.active == self.public_address.is_some()
            && self.external_port != Some(0)
            && (self.enabled || !self.active)
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

#[cfg(test)]
mod mapping_tests {
    use super::*;
    #[test]
    fn public_links_reject_unsafe_urls_wrong_hosts_and_duplicate_services() {
        let host = crate::identity::Identity::generate().endpoint_id();
        let link = PublicLink {
            service: "gike".into(),
            incarnation: "a".repeat(64),
            url: "https://desktop-gike.wispnet.test/".into(),
            state: "online".into(),
        };
        let mut links = PublicLinks {
            version: 1,
            host,
            services: vec![link.clone()],
        };
        assert!(links.valid(host));
        assert!(!links.valid(crate::identity::Identity::generate().endpoint_id()));
        for url in [
            "javascript:alert(1)",
            "http://desktop.test/",
            "https://user@desktop.test/",
            "https://desktop.test:444/",
            "https://desktop.test/path",
            "https://desktop.test/?token=secret",
            "https://127.0.0.1/",
        ] {
            links.services[0].url = url.into();
            assert!(!links.valid(host), "{url}");
        }
        links.services = vec![link.clone(), link];
        assert!(!links.valid(host));
    }
    #[test]
    fn mapping_report_requires_consistent_lease_and_pinned_host() {
        let host = crate::identity::Identity::generate().endpoint_id();
        let other = crate::identity::Identity::generate().endpoint_id();
        let mut report = RouterMappingReport {
            version: 1,
            host,
            enabled: true,
            active: false,
            external_port: None,
            public_address: None,
            upnp: None,
            pcp: None,
            nat_pmp: None,
        };
        assert!(report.valid(host));
        assert!(!report.valid(other));
        report.external_port = Some(43210);
        assert!(!report.valid(host));
        report.active = true;
        report.public_address = Some(false);
        assert!(report.valid(host)); // A private upstream mapping is still a granted lease.
        report.enabled = false;
        assert!(!report.valid(host));
        report.enabled = true;
        report.external_port = Some(0);
        assert!(!report.valid(host));
    }
}
