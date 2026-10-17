//! Suspend, reboot and power off, asked of logind. Without the rights (the
//! greeter's user usually has them, polkit decides) the call fails and is
//! logged; nothing on the screen depends on it.

use guido::prelude::*;
use zbus::proxy;

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Suspend,
    Reboot,
    PowerOff,
}

impl PowerAction {
    pub const ALL: [PowerAction; 3] = [Self::Suspend, Self::Reboot, Self::PowerOff];

    pub fn label(self) -> &'static str {
        match self {
            Self::Suspend => "Suspend",
            Self::Reboot => "Reboot",
            Self::PowerOff => "Shut down",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Power {
    service: Service<PowerAction>,
}

impl Power {
    pub fn start() -> Self {
        let service = create_service::<PowerAction, _, _>(|mut rx, _ctx| async move {
            while let Some(action) = rx.recv().await {
                if let Err(e) = request(action).await {
                    log::warn!("{action:?} refused: {e}");
                }
            }
        });
        Self { service }
    }

    pub fn request(&self, action: PowerAction) {
        self.service.send(action);
    }
}

async fn request(action: PowerAction) -> zbus::Result<()> {
    let connection = zbus::Connection::system().await?;
    let manager = ManagerProxy::new(&connection).await?;
    match action {
        PowerAction::Suspend => manager.suspend(false).await,
        PowerAction::Reboot => manager.reboot(false).await,
        PowerAction::PowerOff => manager.power_off(false).await,
    }
}
