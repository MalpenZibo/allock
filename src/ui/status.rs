//! The bottom row: session picker on the left, battery on the right.

use guido::prelude::*;

use super::Screen;
use crate::Mode;
use crate::components::icons::{IconKind, icon};
use crate::services::upower::BatteryStatus;

/// nf-md-monitor_dashboard — the session picker. Codepoint taken from the
/// font's cmap; `f0a3a` next door is `md-comment_eye`.
const SESSION_GLYPH: &str = "\u{f0a07}";

/// `shows_form` keeps the session picker on the screen the user is typing on;
/// a secondary monitor showing only the clock has nothing to pick with.
pub fn view(screen: Screen, shows_form: bool) -> Container {
    container()
        .width(fill())
        .layout(
            Flex::row()
                .spacing(10.0)
                .cross_alignment(CrossAlignment::Center),
        )
        .maybe_child((shows_form && screen.mode == Mode::Greeter).then(|| session_picker(&screen)))
        // An explicit spacer rather than SpaceBetween: with the picker absent
        // — which is every lock-mode screen — SpaceBetween has one child to
        // place and would leave the battery on the left.
        .child(container().width(fill()))
        .child(battery(&screen))
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
                .corner_radius(32.0)
                .squircle()
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

/// One button cycling through the available sessions.
///
/// A popup menu would be the richer answer, but the greeter runs before
/// anything else exists and the list is normally two or three entries — a
/// button that names the current choice keeps the whole thing legible.
fn session_picker(screen: &Screen) -> Container {
    let theme = screen.theme;
    let sessions = screen.sessions.clone();
    let index = screen.session_index;

    if sessions.is_empty() {
        return container();
    }

    let count = sessions.len();
    let names: Vec<String> = sessions.iter().map(|s| s.name.clone()).collect();
    let single = count == 1;

    let button = super::card_surface(screen)
        .corner_radius(16.0)
        .squircle()
        .child(
            container()
                .padding([8.0, 14.0])
                .layout(
                    Flex::row()
                        .spacing(8.0)
                        .cross_alignment(CrossAlignment::Baseline),
                )
                .child(
                    icon()
                        .kind(IconKind::from(SESSION_GLYPH))
                        .font_size(17.0)
                        .color(theme.text_weak),
                )
                .child(
                    text(move || names.get(index.get() % count).cloned().unwrap_or_default())
                        .font_size(15.0)
                        .nowrap(),
                ),
        );

    if single {
        // Nothing to cycle to: keep the label, drop the affordance.
        return button;
    }

    button
        .when_hovered(|s| s.lighter(0.08))
        .when_pressed(|s| s.ripple())
        .on_click(move || index.update(|i| *i = (*i + 1) % count))
}
