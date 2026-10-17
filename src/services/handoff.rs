//! The handoff with ashell, over `org.ashell.Island1` on the session bus.
//!
//! Before the lock takes the screen, ashell folds its island into the seed,
//! so the compositor's cut falls under a dot both processes draw in the same
//! place; after the unlock, it grows back. See `docs/HANDOFF.md`.
//!
//! Optional on purpose. Without ashell on the bus nothing is waited for and
//! the island arrives and leaves on its own (see [`Handoff::bar`]); a bar
//! that is slow or stuck costs at most `timeout_ms` before the session is
//! locked anyway. The protocol carries "fold" and "resume", never a
//! credential or a verdict.
//!
//! logind's `LockedHint` is kept as well, so a bar that starts or restarts
//! mid-lock knows to come up as the seed.

use std::time::Duration;

use futures::StreamExt;
use guido::prelude::*;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::proxy;

use crate::config::HandoffConfig;

const ISLAND: &str = "org.ashell.Island";

#[proxy(
    interface = "org.ashell.Island1",
    default_service = "org.ashell.Island",
    default_path = "/org/ashell/Island"
)]
trait Island {
    /// Folds the island into the seed; answers once it is the seed.
    fn prepare_handoff(&self, reason: &str) -> zbus::Result<bool>;
    /// The lock is gone: the island may grow back.
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
    /// A bar is on the bus to take the seed from the lock and give it back.
    /// Without one the island has nobody to hand over to, so it arrives and
    /// leaves by itself instead of as a dot.
    pub bar: Signal<bool>,
}

impl Handoff {
    pub fn start(config: HandoffConfig) -> Self {
        let ready = create_signal(false);
        let resumed = create_signal(false);
        let bar = create_signal(false);
        let (ready_w, resumed_w, bar_w) = (ready.writer(), resumed.writer(), bar.writer());
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
            // Who holds the name, now and on every change: ashell may come or
            // go while the lock is up.
            if let Some(bus) = bus.clone() {
                tokio::spawn(async move {
                    if let Err(e) = watch_bar(&bus, |present| bar_w.set(present)).await {
                        log::info!("not watching for the bar: {e}");
                    }
                });
            }
            while let Some(command) = rx.recv().await {
                match command {
                    Command::Prepare => {
                        if let Some(bus) = &bus {
                            // Before the lock draws its first frame, whatever
                            // the watch has heard by then.
                            if let Ok(Ok(present)) = tokio::time::timeout(timeout, owned(bus)).await
                            {
                                bar_w.set(present);
                            }
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
            bar: bar.read_only(),
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

/// Subscribes before asking, so a bar that appears in between is not missed.
async fn watch_bar(bus: &zbus::Connection, mut seen: impl FnMut(bool)) -> zbus::Result<()> {
    let dbus = DBusProxy::new(bus).await?;
    let mut changes = dbus
        .receive_name_owner_changed_with_args(&[(0, ISLAND)])
        .await?;
    seen(dbus.name_has_owner(BusName::try_from(ISLAND)?).await?);
    while let Some(change) = changes.next().await {
        if let Ok(args) = change.args() {
            seen(args.new_owner().is_some());
        }
    }
    Ok(())
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
        log::info!("the bar declined to fold; locking anyway");
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
    let system = zbus::Connection::system().await?;
    SessionProxy::new(&system)
        .await?
        .set_locked_hint(locked)
        .await
}
