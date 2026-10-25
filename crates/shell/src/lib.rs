// SPDX-License-Identifier: AGPL-3.0-only
//! WispShell-derived terminal/session engine. Transport identity and service
//! authorization belong to Wispnet; this crate operates on accepted streams.
//! See NOTICE.md for source attribution.
mod protocol;
pub use protocol::*;
#[cfg(feature = "host")]
mod host;
#[cfg(feature = "host")]
pub use host::SessionManager;
