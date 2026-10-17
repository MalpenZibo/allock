//! Whether the user is at the machine, from `ext-idle-notify-v1` on a Wayland
//! connection of its own, since guido does not speak the protocol. Version 2's
//! input idle notification is preferred: it ignores idle inhibitors.

use std::time::Duration;

use guido::prelude::*;
use tokio::sync::watch;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

/// `true` after `timeout` without input, `false` at the next input. `None`
/// when the compositor cannot say.
pub fn watch(timeout: Duration) -> Option<watch::Receiver<bool>> {
    match start(timeout) {
        Ok(idle) => Some(idle),
        Err(e) => {
            log::info!("no idle notifications: {e}");
            None
        }
    }
}

/// The same answer as a signal; always `false` without a watcher.
pub fn resting(idle: Option<watch::Receiver<bool>>) -> Signal<bool> {
    let resting = create_signal(false);
    let writer = resting.writer();
    if let Some(mut idle) = idle {
        create_task(move |_ctx| async move {
            while idle.changed().await.is_ok() {
                let now = *idle.borrow_and_update();
                writer.set(now);
            }
        });
    }
    resting.read_only()
}

fn start(timeout: Duration) -> anyhow::Result<watch::Receiver<bool>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Idle>(&connection)?;
    let qh = queue.handle();

    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ())?;
    let notifier: ExtIdleNotifierV1 = globals.bind(&qh, 1..=2, ())?;
    let timeout = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    let notification = if notifier.version() >= 2 {
        notifier.get_input_idle_notification(timeout, &seat, &qh, ())
    } else {
        notifier.get_idle_notification(timeout, &seat, &qh, ())
    };

    log::debug!(
        "idle notifications from ext_idle_notifier_v1 v{}",
        notifier.version()
    );

    let (tx, rx) = watch::channel(false);
    let mut idle = Idle { tx };
    std::thread::Builder::new()
        .name("allock-idle".to_owned())
        .spawn(move || {
            // Owned here so the objects live as long as the thread does.
            let _objects = (seat, notifier, notification);
            // Ends at the first event after nobody is listening any more.
            while !idle.tx.is_closed() {
                if let Err(e) = queue.blocking_dispatch(&mut idle) {
                    log::info!("idle notifications ended: {e}");
                    break;
                }
            }
        })?;
    Ok(rx)
}

struct Idle {
    tx: watch::Sender<bool>,
}

impl Dispatch<ExtIdleNotificationV1, ()> for Idle {
    fn event(
        idle: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => idle.tx.send_replace(true),
            ext_idle_notification_v1::Event::Resumed => idle.tx.send_replace(false),
            _ => return,
        };
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Idle {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Idle {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for Idle {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotifierV1,
        _: <ExtIdleNotifierV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
