//! Everything allock talks to outside its own window, each behind a handle
//! the interface only reads signals from and sends commands to.

pub mod auth;
pub mod battery;
pub mod clock;
pub mod handoff;
pub mod idle;
pub mod power;
