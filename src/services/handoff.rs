//! The handoff with ashell, over `org.ashell.Island1` on the session bus.
//!
//! Before the lock takes the screen, ashell's island goes up past the top
//! edge, so the session the lock covers (and the snapshot it takes of it)
//! has no island in it; the lock's own island then drops in from the same
//! edge. After the unlock, ashell's drops back in. See `docs/HANDOFF.md`.
//!
//! Optional on purpose. Without ashell on the bus nothing is waited for; a
//! bar that is slow or stuck costs at most `timeout_ms` before the session
//! is locked anyway. The protocol carries "go" and "come back", never a
//! credential or a verdict.
//!
//! logind's `LockedHint` is kept as well, so a bar that starts or restarts
//! mid-lock knows to stay away until the unlock.

use std::time::Duration;

use guido::prelude::*;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::proxy;

use super::logind::{self, SessionProxy};
use crate::config::HandoffConfig;

const ISLAND: &str = "org.ashell.Island";

#[proxy(
    interface = "org.ashell.Island1",
    default_service = "org.ashell.Island",
    default_path = "/org/ashell/Island"
)]
trait Island {
    /// Sends the island up past the top edge; answers once it is gone.
    fn prepare_handoff(&self, reason: &str) -> zbus::Result<bool>;
    /// The lock is gone: the island may grow back.
    fn resume_after_handoff(&self) -> zbus::Result<()>;
}

#[derive(Debug)]
enum Command {
    /// Send the bar away, then say `ready`.
    Prepare,
    /// The lock is held: tell logind.
    Locked,
    /// Unlocked: tell logind and the bar, then say `resumed`.
    Resume,
}

#[derive(Clone, Copy)]
pub struct Handoff {
    service: Service<Command>,
    /// The bar is gone, or will never answer: the lock may be drawn.
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
            // Connected now, not in the middle of an unlock.
            if let Err(e) = logind::system().await {
                log::info!("no system bus for LockedHint: {e}");
            }
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
                            within(timeout, "send away", prepare(bus)).await;
                        }
                        ready_w.set(true);
                    }
                    Command::Locked => within(timeout, "LockedHint", locked_hint(true)).await,
                    // Independent: the exit waits on the slower, not on both.
                    Command::Resume => {
                        let bar = async {
                            if let Some(bus) = &bus {
                                within(timeout, "resume", resume(bus)).await;
                            }
                        };
                        tokio::join!(within(timeout, "LockedHint", locked_hint(false)), bar);
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

async fn owned(bus: &zbus::Connection) -> zbus::Result<bool> {
    DBusProxy::new(bus)
        .await?
        .name_has_owner(BusName::try_from(ISLAND)?)
        .await
        .map_err(Into::into)
}

/// Asks the bus first, so a bar that is not running costs a round trip and
/// not the whole timeout.
async fn prepare(bus: &zbus::Connection) -> zbus::Result<()> {
    if !owned(bus).await? {
        return Ok(());
    }
    if !IslandProxy::new(bus).await?.prepare_handoff("lock").await? {
        log::info!("the bar declined to go; locking anyway");
    }
    Ok(())
}

async fn resume(bus: &zbus::Connection) -> zbus::Result<()> {
    if !owned(bus).await? {
        return Ok(());
    }
    IslandProxy::new(bus).await?.resume_after_handoff().await
}

async fn locked_hint(locked: bool) -> zbus::Result<()> {
    SessionProxy::new(&logind::system().await?)
        .await?
        .set_locked_hint(locked)
        .await
}
