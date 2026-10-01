//! The bottom row: session picker in the middle, battery on the right.

use guido::prelude::*;

use super::tokens::{self, Typography, radius, space};
use super::{Screen, picker};
use crate::Mode;
use crate::components::icons::icon;
use crate::services::upower::BatteryStatus;

pub fn view(screen: Screen) -> Container {
    container()
        .width(fill())
        // Bottom-aligned: the battery arrives late over D-Bus and would lift a centred picker.
        .layout(
            Flex::row()
                .spacing(space::S8)
                .cross_alignment(CrossAlignment::End),
        )
        // Equal fill-width sides keep the picker centred on the screen.
        .child(container().width(fill()))
        .child(
            (screen.mode == Mode::Greeter)
                .then(|| session_picker(&screen))
                .flatten(),
        )
        .child(
            container()
                .width(fill())
                .layout(Flex::row().main_alignment(MainAlignment::End))
                .child(battery(&screen)),
        )
}

fn battery(screen: &Screen) -> Container {
    let theme = screen.theme;
    let Some(service) = screen.battery else {
        return container();
    };

    // The service republishes on every D-Bus change and has no equality.
    let battery = create_memo(move || {
        let data = service.get()?.system_battery?;
        Some((
            data.capacity,
            data.get_icon(),
            data.get_indicator_state(),
            matches!(data.status, BatteryStatus::Charging(_)),
        ))
    });

    let screen = screen.clone();
    container().child(move || {
        let (capacity, battery_icon, indicator, charging) = battery.get()?;
        let color = indicator.color(&theme);

        Some(
            super::card_surface(&screen)
                .corners(Corners::squircle(radius::R16))
                .child(
                    container()
                        .padding([space::S12, space::S24])
                        // Baseline: the glyph and the reading are different sizes.
                        .layout(
                            Flex::row()
                                .spacing(space::S12)
                                .cross_alignment(CrossAlignment::Baseline),
                        )
                        .child(
                            icon()
                                .kind(battery_icon)
                                .font_size(tokens::DISPLAY_M.size)
                                .color(color),
                        )
                        .child(
                            text(format!("{capacity}%"))
                                .typography(tokens::DISPLAY_S)
                                .color(if charging { color } else { theme.text })
                                .nowrap(),
                        ),
                ),
        )
    })
}

fn session_picker(screen: &Screen) -> Option<Container> {
    let sessions = &screen.sessions;
    (sessions.len() > 1).then(|| {
        let names = sessions.iter().map(|s| s.name.clone()).collect();
        // Never held: the session is read only once the password is accepted.
        picker::view(
            screen,
            names,
            screen.session,
            picker::Look::Card,
            false.into_signal(),
        )
    })
}
