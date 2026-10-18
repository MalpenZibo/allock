//! The system battery, read through ashell's UPower service.

use ashell_services::upower::{BatteryStatus, UPower};
use futures::StreamExt;
use guido::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// `None` until UPower answers, and on a machine without a battery.
pub fn watch() -> Signal<Option<Battery>> {
    let battery = create_signal(None);
    let writer = battery.writer();
    create_task(move |ctx| async move {
        let upower = match UPower::connect().await {
            Ok(upower) => upower,
            Err(e) => {
                log::info!("no battery indicator: {e}");
                return;
            }
        };
        let mut updates = Box::pin(upower.updates());
        while ctx.is_running()
            && let Some(data) = updates.next().await
        {
            writer.set(data.system_battery.map(|data| Battery {
                percent: data.capacity.clamp(0, 100) as u8,
                // A full battery is not charging, as ashell's ring says:
                // only charge going in is green.
                charging: matches!(data.status, BatteryStatus::Charging(_)),
            }));
        }
    });
    battery.read_only()
}
