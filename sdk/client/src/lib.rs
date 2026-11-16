//! Source-distributed remote client facade: reexports only, no new authority.
//! The initial concrete transport is the private same-user Unix profile.
#![forbid(unsafe_code)]
pub use chronos_client as client;
pub use chronos_local as transport;
pub use chronos_protocol as protocol;
pub use chronos_wire as wire;
