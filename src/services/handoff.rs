//! The handoff with the bar: before the lock takes the screen, ashell folds
//! its island into the seed, so the compositor's cut happens under a dot both
//! sides draw in the same place; after the unlock, it grows back.
//!
//! Optional on purpose. A bar that is missing, slow or crashed costs at most
//! `timeout_ms` before the session is locked anyway, and the protocol carries
//! nothing but "fold" and "resume": never a credential or a verdict.
//!
//! logind's `LockedHint` is kept as well, so a bar that restarts mid-lock
//! knows to come back as the seed.

use std::time::Duration;

use guido::prelude::*;
use zbus::proxy;

use crate::config::HandoffConfig;

const ISLAND: &str = "org.ashell.Island";

#[proxy(
    interface = "org.ashell.Island1",
    default_service = "org.ashell.Island",
    default_path = "/org/ashell/Island"
)]
trait Island {
    fn prepare_handoff(&self, reason: &str) -> zbus::Result<bool>;
    fn resume_after_handoff(&self) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1/session/auto"
)]
trait Session {
    fn set_locked_hint(&self, locked: bool) -> zbus::Result<()>;
}

#[derive(Debug)]
enum Command {
    /// Fold the bar, then say `ready`.
    Prepare,
    /// The lock is held: tell logind.
    Locked,
    /// Unlocked: tell logind and the bar, then say `resumed`.
    Resume,
}

#[derive(Clone, Copy)]
pub struct Handoff {
    service: Service<Command>,
    /// The bar is the seed, or will never answer: the lock may be drawn.
    pub ready: Signal<bool>,
    /// The bar has been told it may grow back: the process may end.
    pub resumed: Signal<bool>,
}

impl Handoff {
    pub fn start(config: HandoffConfig) -> Self {
        let ready = create_signal(false);
        let resumed = create_signal(false);
        let (ready_w, resumed_w) = (ready.writer(), resumed.writer());
        let timeout = Duration::from_millis(config.timeout_ms);

        let service = create_service::<Command, _, _>(move |mut rx, _ctx| async move {
            let bus = if config.enabled {
                zbus::Connection::session()
                    .await
                    .inspect_err(|e| log::info!("no session bus for the handoff: {e}"))
                    .ok()
            } else {
                None
            };
            while let Some(command) = rx.recv().await {
                match command {
                    Command::Prepare => {
                        if let Some(bus) = &bus {
                            within(timeout, "fold", prepare(bus)).await;
                        }
                        ready_w.set(true);
                    }
                    Command::Locked => within(timeout, "LockedHint", locked_hint(true)).await,
                    Command::Resume => {
                        within(timeout, "LockedHint", locked_hint(false)).await;
                        if let Some(bus) = &bus {
                            within(timeout, "resume", resume(bus)).await;
                        }
                        resumed_w.set(true);
                    }
                }
            }
        });

        Self {
            service,
            ready: ready.read_only(),
            resumed: resumed.read_only(),
        }
    }

    pub fn prepare(&self) {
        self.service.send(Command::Prepare);
    }

    pub fn locked(&self) {
        self.service.send(Command::Locked);
    }

    pub fn resume(&self) {
        self.service.send(Command::Resume);
    }
}

async fn within(timeout: Duration, what: &str, call: impl Future<Output = zbus::Result<()>>) {
    match tokio::time::timeout(timeout, call).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::info!("handoff {what}: {e}"),
        Err(_) => log::info!("handoff {what}: no answer in {timeout:?}"),
    }
}

/// Asks the bus first, so a bar that is not running costs a round trip and
/// not the whole timeout.
async fn prepare(bus: &zbus::Connection) -> zbus::Result<()> {
    if !zbus::fdo::DBusProxy::new(bus)
        .await?
        .name_has_owner(ISLAND.try_into()?)
        .await?
    {
        return Ok(());
    }
    let folded = IslandProxy::new(bus).await?.prepare_handoff("lock").await?;
    if !folded {
        log::info!("the bar declined to fold; locking anyway");
    }
    Ok(())
}

async fn resume(bus: &zbus::Connection) -> zbus::Result<()> {
    IslandProxy::new(bus).await?.resume_after_handoff().await
}

async fn locked_hint(locked: bool) -> zbus::Result<()> {
    let system = zbus::Connection::system().await?;
    SessionProxy::new(&system)
        .await?
        .set_locked_hint(locked)
        .await
}
