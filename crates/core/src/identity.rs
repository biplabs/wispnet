//! Identity contract shared by Linux and future platform key stores.
pub use iroh_base::{EndpointId, SecretKey};
use std::fmt;

/// Deliberately neither Debug nor Serialize. Only expose the public ID in UI/logs.
pub struct Identity(SecretKey);

impl Identity {
    pub fn generate() -> Self {
        Self(SecretKey::generate())
    }
    pub fn from_secret_key(key: SecretKey) -> Self {
        Self(key)
    }
    pub fn endpoint_id(&self) -> EndpointId {
        self.0.public()
    }
    /// For storage/transport implementations only; never pass to diagnostics.
    pub fn secret_key(&self) -> &SecretKey {
        &self.0
    }
}

#[derive(Debug)]
pub struct IdentityError(pub &'static str);
impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for IdentityError {}

/// Implementations must serialize mutations, commit atomically, and fail closed
/// on corrupt/unreadable state. Missing state must never trigger implicit rotation.
pub trait IdentityStore {
    fn load(&self) -> Result<Identity, IdentityError>;
    /// Explicit first-use operation; existing valid identity is returned unchanged.
    fn initialize(&self) -> Result<Identity, IdentityError>;
    /// Explicit destructive operation guarded by the current public identity.
    fn reset(&self, expected: EndpointId) -> Result<(), IdentityError>;
}
