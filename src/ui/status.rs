//! The bottom row: session picker in the middle, battery on the right.

use guido::prelude::*;

use super::{Screen, picker};
use crate::Mode;
use crate::components::icons::icon;
use crate::services::upower::BatteryStatus;

/// Only built on the outputs carrying the interface, so the session picker is
/// always on a screen the user is typing on.
pub fn view(screen: Screen) -> Container {
    container()
        .width(fill())
        // Bottom-aligned, not centred: the battery arrives a moment after the
        // first frame (its reading comes over D-Bus) and is the tallest thing
        // here, so a centred row would lift the session picker by half the
        // card's height the moment it did.
        .layout(
            Flex::row()
                .spacing(10.0)
                .cross_alignment(CrossAlignment::End),
        )
        // Two fill-width sides around the picker share what is left equally,
        // which is what puts it in the middle of the screen rather than in the
        // middle of whatever the battery leaves over. With the picker absent —
        // every lock-mode screen — they still push the battery to the right.
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

    // Memo on the rendered values, not the service: the upower service
    // republishes on every D-Bus change and holds a connection, so it has no
    // meaningful equality — the cutoff belongs here, and it has to be over
    // something comparable, which `BatteryData` is not.
    let battery = create_memo(move || {
        let data = service.get()?.system_battery?;
        Some((
            data.capacity,
            data.get_icon(),
            data.get_indicator_state(),
            matches!(data.status, BatteryStatus::Charging(_)),
        ))
    });

    // The card is rebuilt whenever the reading changes, and each rebuild needs
    // its own frosted surface, so the closure owns a Screen rather than
    // borrowing the caller's.
    let screen = screen.clone();
    container().child(move || {
        let (capacity, battery_icon, indicator, charging) = battery.get()?;
        let color = indicator.color(&theme);

        Some(
            super::card_surface(&screen)
                .corners(Corners::squircle(32.0))
                .child(
                    container()
                        .padding([12.0, 24.0])
                        // Baseline, not Center: the glyph and the reading are
                        // different sizes, and lining their boxes up leaves
                        // them floating at unrelated heights.
                        .layout(
                            Flex::row()
                                .spacing(12.0)
                                .cross_alignment(CrossAlignment::Baseline),
                        )
                        .child(icon().kind(battery_icon).font_size(64.0).color(color))
                        .child(
                            text(format!("{capacity}%"))
                                .font_size(48.0)
                                // Charging tints the reading to match the icon.
                                .color(if charging { color } else { theme.text })
                                .nowrap(),
                        ),
                ),
        )
    })
}

/// The session to start, between a previous and a next arrow.
///
/// Not drawn at all with one session or none: there is nothing to choose, and
/// the one there is starts anyway.
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
