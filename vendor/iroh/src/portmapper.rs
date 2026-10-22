//! Portmapper integration.
//!
//! Wraps the real [`portmapper`] crate when the `portmapper` feature is enabled,
//! or provides a no-op stub otherwise.

use std::net::SocketAddrV4;

use tokio::sync::watch;

/// Router port-mapping observations. A granted mapping is not proof of WAN reachability.
#[derive(Debug, Clone, Default)]
pub struct PortMappingStatus {
    /// Whether this endpoint has an active port-mapping service.
    pub enabled: bool,
    /// The currently granted external UDP address, if any.
    pub mapping: Option<SocketAddrV4>,
    /// Protocol support observed by a bounded gateway probe; absent if unavailable.
    pub protocols: Option<PortMappingProtocols>,
}

/// Protocol support reported by the gateway probe, independently of mapping creation.
#[derive(Debug, Clone)]
pub struct PortMappingProtocols {
    /// Whether UPnP responded.
    pub upnp: bool,
    /// Whether PCP responded.
    pub pcp: bool,
    /// Whether NAT-PMP responded.
    pub nat_pmp: bool,
}

/// Configuration for the portmapper service (UPnP, PCP, NAT-PMP).
///
/// Port mapping asks the local router to open an external port so peers can
/// reach this endpoint directly, improving connectivity behind NATs. The
/// discovery step (UPnP uses SSDP multicast) can, however, trigger firewall
/// prompts on some networks — see [`PortmapperConfig::Disabled`].
///
/// Used with [`crate::endpoint::Builder::portmapper_config`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PortmapperConfig {
    /// Enable portmapping with default settings.
    ///
    /// This is the default.
    #[non_exhaustive]
    Enabled {},
    /// Disable portmapping.
    ///
    /// Skips the UPnP/PCP/NAT-PMP gateway probing. Use this to avoid the
    /// SSDP multicast discovery that can raise firewall dialogs (notably on
    /// macOS), at the cost of potentially worse direct connectivity behind
    /// some NATs.
    Disabled,
}

impl Default for PortmapperConfig {
    fn default() -> Self {
        PortmapperConfig::Enabled {}
    }
}

pub(crate) fn create_client(config: &PortmapperConfig) -> Client {
    match config {
        #[cfg(all(not(wasm_browser), feature = "portmapper"))]
        PortmapperConfig::Enabled {} => Client::Enabled(::portmapper::Client::default()),
        _ => {
            let (tx, rx) = watch::channel(None);
            Client::Disabled { _tx: tx, rx }
        }
    }
}

/// Portmapper client: either the real implementation or a no-op.
///
/// The disabled variant is used when the `portmapper` feature is off, on wasm,
/// or when portmapping is disabled via [`PortmapperConfig::Disabled`].
#[derive(Debug, Clone)]
pub(crate) enum Client {
    /// The real portmapper client (requires the `portmapper` feature).
    #[cfg(all(not(wasm_browser), feature = "portmapper"))]
    Enabled(::portmapper::Client),
    /// No-op: keeps the sender alive so the receiver never closes.
    Disabled {
        _tx: watch::Sender<Option<SocketAddrV4>>,
        rx: watch::Receiver<Option<SocketAddrV4>>,
    },
}

impl Client {
    /// Forget a previous gateway and its lease when the default network changes.
    pub(crate) fn reset(&mut self) {
        #[cfg(all(not(wasm_browser), feature = "portmapper"))]
        if let Client::Enabled(client) = self {
            client.deactivate();
            *client = ::portmapper::Client::default();
        }
    }

    pub(crate) async fn status(&self) -> PortMappingStatus {
        match self {
            #[cfg(all(not(wasm_browser), feature = "portmapper"))]
            Client::Enabled(client) => {
                let protocols =
                    match tokio::time::timeout(std::time::Duration::from_secs(3), client.probe())
                        .await
                    {
                        Ok(Ok(Ok(probe))) => Some(PortMappingProtocols {
                            upnp: probe.upnp,
                            pcp: probe.pcp,
                            nat_pmp: probe.nat_pmp,
                        }),
                        _ => None,
                    };
                PortMappingStatus {
                    enabled: true,
                    mapping: *client.watch_external_address().borrow(),
                    protocols,
                }
            }
            Client::Disabled { .. } => PortMappingStatus::default(),
        }
    }

    /// Discover supported protocols before retrying a mapping. Without this,
    /// upstream's initial UPnP attempt can mask an available PCP/NAT-PMP service.
    pub(crate) async fn probe_and_procure(&self) {
        let _ = self.status().await;
        self.procure_mapping();
    }

    pub(crate) fn procure_mapping(&self) {
        match self {
            #[cfg(all(not(wasm_browser), feature = "portmapper"))]
            Client::Enabled(c) => c.procure_mapping(),
            Client::Disabled { .. } => {}
        }
    }

    pub(crate) fn update_local_port(&self, _port: std::num::NonZeroU16) {
        match self {
            #[cfg(all(not(wasm_browser), feature = "portmapper"))]
            Client::Enabled(c) => c.update_local_port(_port),
            Client::Disabled { .. } => {}
        }
    }

    pub(crate) fn deactivate(&self) {
        match self {
            #[cfg(all(not(wasm_browser), feature = "portmapper"))]
            Client::Enabled(c) => c.deactivate(),
            Client::Disabled { .. } => {}
        }
    }

    pub(crate) fn watch_external_address(&self) -> watch::Receiver<Option<SocketAddrV4>> {
        match self {
            #[cfg(all(not(wasm_browser), feature = "portmapper"))]
            Client::Enabled(c) => c.watch_external_address(),
            Client::Disabled { rx, .. } => rx.clone(),
        }
    }
}
