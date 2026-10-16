//! Platform-neutral configuration and protocol foundations.
pub mod config;
pub mod identity;

/// Reserved for the versioned service tunnel. No wire protocol is implemented yet.
pub const PROTOCOL_VERSION: u16 = 1;
