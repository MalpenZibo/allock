//! The system battery, read through ashell's UPower service.

use ashell_services::upower::UPower;
use futures::StreamExt;
use guido::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    /// On the charger, as UPower's `OnBattery` says: unlike the battery's
    /// state, it holds while the firmware pauses charging near full.
    pub plugged: bool,
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
            let plugged = !data.on_battery;
            writer.set(data.system_battery.map(|battery| Battery {
                percent: battery.capacity.clamp(0, 100) as u8,
                plugged,
            }));
        }
    });
    battery.read_only()
}
