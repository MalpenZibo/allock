//! The bottom row: session picker in the middle, battery on the right.

use guido::prelude::*;

use super::Screen;
use crate::Mode;
use crate::components::icons::{StaticIcon, icon};
use crate::services::upower::BatteryStatus;

/// Exact, for the reason the field's width is: the window the names slide
/// through has to stay put while they do, or the arrows either side of it
/// would jump with every name. A name too long for it wraps.
const SESSION_LABEL_WIDTH: f32 = 200.0;

/// The padding around a session name inside its window.
const LABEL_PADDING: [f32; 2] = [8.0, 14.0];
const LABEL_FONT_SIZE: f32 = 15.0;

/// The round buttons either side of the name.
const ARROW_SIZE: f32 = 36.0;

/// How long a name takes to cross the window.
const SLIDE_MS: f32 = 260.0;

/// Only built on the outputs carrying the interface, so the session picker is
/// always on a screen the user is typing on.
pub fn view(screen: Screen) -> Container {
    container()
        .width(fill())
        .layout(
            Flex::row()
                .spacing(10.0)
                .cross_alignment(CrossAlignment::Center),
        )
        // Two fill-width sides around the picker share what is left equally,
        // which is what puts it in the middle of the screen rather than in the
        // middle of whatever the battery leaves over. With the picker absent —
        // every lock-mode screen — they still push the battery to the right.
        .child(container().width(fill()))
        .child((screen.mode == Mode::Greeter).then(|| session_picker(&screen)))
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

/// The chosen session's name between a previous and a next arrow.
///
/// A popup menu would be the richer answer, but the greeter runs before
/// anything else exists and the list is normally two or three entries — a
/// name you step through keeps the whole thing legible.
///
/// Stepping slides the old name out and the new one in from the side the
/// arrow points to, so the list reads as a strip seen through a window.
fn session_picker(screen: &Screen) -> Container {
    let sessions = screen.sessions.clone();
    let index = screen.session_index;
    let step = screen.session_step;

    if sessions.is_empty() {
        return container();
    }

    let count = sessions.len();
    let names: Vec<String> = sessions.iter().map(|s| s.name.clone()).collect();

    let label = session_label(screen, names, index, step);
    if count == 1 {
        // Nothing to step to: keep the name, drop the arrows.
        return label;
    }

    let go = move |by: isize| {
        // The direction first: the label rebuilds on the index, and both the
        // name arriving and the one leaving read the direction as it does.
        step.set(by);
        index.update(|i| *i = (*i as isize + by).rem_euclid(count as isize) as usize);
    };

    container()
        .layout(
            Flex::row()
                .spacing(12.0)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(arrow(screen, StaticIcon::LeftChevron, move || go(-1)))
        .child(label)
        .child(arrow(screen, StaticIcon::RightChevron, move || go(1)))
}

/// The window the names slide through.
///
/// A fixed width, and as tall as the name that wraps onto the most lines:
/// every name is drawn invisibly behind the visible one, which fills the box
/// they make. Every label is then the same size, and nothing around the picker
/// moves while the names change.
///
/// Rebuilt on every change of the index. The new name enters from the side
/// the arrow points to and the old one leaves by the other, both by their own
/// width and both clipped by the card; a name still leaving when the next
/// step comes finishes from where it is. `step` is `0` until the first step,
/// so the first name enters from where it rests: it simply sits there.
///
/// The direction comes from `step` rather than from comparing indices,
/// because with two sessions next and previous land on the same one.
fn session_label(
    screen: &Screen,
    names: Vec<String>,
    index: RwSignal<usize>,
    step: RwSignal<isize>,
) -> Container {
    let theme = screen.theme;
    let count = names.len();

    let line = |name: &str, color: Color| {
        text(name.to_owned())
            .font_size(LABEL_FONT_SIZE)
            .color(color)
            .align(TextAlign::Center)
    };

    let name = move |i: usize| {
        let slide = Transition::new(SLIDE_MS, TimingFunction::EaseInOut);
        let arriving = step.get_untracked() as f32;
        names
            .iter()
            .fold(
                container()
                    .width(SESSION_LABEL_WIDTH)
                    .padding(LABEL_PADDING)
                    .layout(ZStack::new()),
                |label, name| label.child(line(name, Color::TRANSPARENT)),
            )
            .child(
                container()
                    .width(fill())
                    .height(fill())
                    .layout(Flex::column().center())
                    .child(line(&names[i], theme.text)),
            )
            .translate(
                Translate::NONE
                    .transition(slide)
                    .entering_from(Translate::relative(arriving, 0.0))
                    // Read when the name is removed, so it leaves the way the
                    // step that replaced it went.
                    .exiting_to(move || Translate::relative(-step.get() as f32, 0.0)),
            )
    };

    super::card_surface(screen)
        .corners(Corners::squircle(16.0))
        .child(move || name(index.get() % count))
}

fn arrow(screen: &Screen, glyph: StaticIcon, on_click: impl Fn() + 'static) -> Container {
    let theme = screen.theme;
    super::card_surface(screen)
        .width(ARROW_SIZE)
        .height(ARROW_SIZE)
        .corners(ARROW_SIZE / 2.0)
        .child(
            container()
                .width(fill())
                .height(fill())
                .layout(Flex::column().center())
                .child(icon().kind(glyph).font_size(18.0).color(theme.text)),
        )
        .when_hovered(|s| s.lighter(0.08))
        .when_pressed(|s| s.ripple())
        .cursor(CursorIcon::Pointer)
        .on_click(on_click)
}
