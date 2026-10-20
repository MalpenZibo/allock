//! The system battery, read through ashell's UPower service.

use ashell_services::upower::{BatteryStatus, UPower};
use futures::StreamExt;
use guido::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    /// On the charger, as UPower's `OnBattery` says: unlike the battery's
    /// state, it holds while the firmware pauses charging near full. When
    /// UPower cannot tell, the battery's state is all there is, as ashell's.
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
            let on_battery = data.on_battery;
            writer.set(data.system_battery.map(|battery| Battery {
                percent: battery.capacity.clamp(0, 100) as u8,
                plugged: on_battery.map_or_else(
                    || {
                        matches!(
                            battery.status,
                            BatteryStatus::Charging(_)
                                | BatteryStatus::Full
                                | BatteryStatus::NotCharging
                        )
                    },
                    |on_battery| !on_battery,
                ),
            }));
        }
    });
    battery.read_only()
}
