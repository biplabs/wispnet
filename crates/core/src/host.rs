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
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub host: EndpointId,
    pub generation: String,
    pub revision: u64,
    pub services: Vec<Service>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub id: String,
}
