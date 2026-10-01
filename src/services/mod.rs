//! Background services; today only UPower, on the iced-shaped traits in `compat`.

// The shim is general and allock uses only part of it.
#[allow(dead_code)]
pub mod compat;
mod throttle;
pub mod upower;

pub use compat::{ReadOnlyService, Service, ServiceEvent};
