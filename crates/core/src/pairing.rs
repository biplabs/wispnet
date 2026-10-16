//! Public pre-approval bootstrap. A hello is not an invitation or authorization.
use crate::identity::EndpointId;
use serde::{Deserialize, Serialize};
mod invitation;
pub use invitation::{Bootstrap, Challenge, Invitation, Purpose, Scope};

pub const ALPN: &[u8] = b"wipsnet/pairing/1";
pub const VERSION: u16 = 1;
pub const MAX_REQUEST: usize = 256;
pub const MAX_RESPONSE: usize = 512;

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Hello {},
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub version: u16,
    pub host: EndpointId,
}
