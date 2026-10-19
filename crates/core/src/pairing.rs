//! Public pre-approval bootstrap. A hello is not an invitation or authorization.
use crate::identity::EndpointId;
use serde::{Deserialize, Serialize};
mod invitation;
pub use crate::enrollment::Signature;
pub use invitation::{Bootstrap, Challenge, Invitation, Purpose, Scope};

// Legacy wire spelling is frozen for compatibility with installed Wispnet clients.
pub const ALPN: &[u8] = b"wipsnet/pairing/1";
pub const VERSION: u16 = 1;
pub const MAX_REQUEST: usize = 4096;
pub const MAX_RESPONSE: usize = 4096;

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Hello {},
    Challenge {
        claim: Claim,
    },
    Prove {
        proof: Proof,
    },
    /// Host-to-host pairing: authenticated reverse bootstrap, never service grants.
    ProveMutual {
        proof: Proof,
        bootstrap: Bootstrap,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub invitation: String,
    pub code: String,
    pub name: String,
}
impl Drop for Claim {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.code.zeroize();
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub challenge: Challenge,
    pub signature: Signature,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub host: EndpointId,
    pub peer: EndpointId,
    pub invitation: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub version: u16,
    pub host: EndpointId,
}
