//! Background services — today only UPower, for the battery.
//!
//! `upower/` is written against the iced-shaped service traits that
//! `compat.rs` provides, and `throttle.rs` paces its updates.

// The shim is general and allock uses one service, so most of it is
// unreferenced. Allowed at the module boundary rather than item by item.
#[allow(dead_code)]
pub mod compat;
mod throttle;
pub mod upower;

pub use compat::{ReadOnlyService, Service, ServiceEvent};
