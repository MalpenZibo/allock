//! ashell's services, reused as-is.
//!
//! `compat.rs`, `throttle.rs` and `upower/` are copied verbatim from the
//! ashell/guido port. They are written against the iced-shaped service traits
//! that `compat.rs` mimics, so when ashell publishes them as a dedicated
//! crate this module becomes a `use` line and the copies go away. Do not
//! refactor them here — divergence is what would make that swap expensive.

// allock uses one service where ashell uses a dozen, so most of the shim is
// unreferenced here. Allowed at the module boundary rather than inside the
// files, so the copies stay byte-identical to upstream.
#[allow(dead_code)]
pub mod compat;
mod throttle;
pub mod upower;

pub use compat::{ReadOnlyService, Service, ServiceEvent};
