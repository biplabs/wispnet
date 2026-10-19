//! Host-scoped control contract. Transport identity, never a caller field, is authority.
use crate::identity::EndpointId;
use serde::{Deserialize, Serialize};
pub const ALPN: &[u8] = b"wipsnet/control/1";
pub const MAX_REQUEST: usize = 4096;
pub const MAX_RESPONSE: usize = 128 * 1024;
#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Catalog {},
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
