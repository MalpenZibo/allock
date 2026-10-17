//! What allock reads off the machine itself.

pub mod faillock;
pub mod last_login;
pub mod user;

/// The machine's name, for the greeter and the card's account line.
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|name| name.trim().to_owned())
        .unwrap_or_default()
}
