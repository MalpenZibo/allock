//! logind, over one system-bus connection shared by everything that asks it:
//! power actions, the session's `LockedHint` and the sleep signal.

use tokio::sync::OnceCell;
use zbus::{Connection, proxy};

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub trait Manager {
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1/session/auto"
)]
pub trait Session {
    fn set_locked_hint(&self, locked: bool) -> zbus::Result<()>;
}

/// The system bus, connected on first use. A failure is not kept: the next
/// caller tries again.
pub async fn system() -> zbus::Result<Connection> {
    static SYSTEM: OnceCell<Connection> = OnceCell::const_new();
    SYSTEM.get_or_try_init(Connection::system).await.cloned()
}
