//! Platform-neutral configuration and protocol foundations.
pub mod config;
pub mod enrollment;
pub mod host;
pub mod identity;
pub mod pairing;

pub mod protocol;
pub use protocol::VERSION as PROTOCOL_VERSION;
