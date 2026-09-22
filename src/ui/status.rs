//! The bottom row: session picker in the middle, battery on the right.

use std::cell::Cell;

use guido::prelude::*;

use super::Screen;
use crate::Mode;
use crate::components::icons::{StaticIcon, icon};
use crate::services::upower::BatteryStatus;

/// Exact, for the reason the field's width is: the window the names slide
/// through has to stay put while they do, or the arrows either side of it
/// would jump with every name. A name too long for it goes onto more lines.
const SESSION_LABEL_WIDTH: f32 = 200.0;

/// The padding around a session name inside its window.
const LABEL_PADDING: [f32; 2] = [8.0, 14.0];
const LABEL_FONT_SIZE: f32 = 15.0;

/// A generous guess at how wide one character of a name is drawn, per pixel
/// of font size.
///
/// The names are broken into lines here rather than by the text widget,
/// because guido wraps without aligning: the lines of a wrapped name would all
/// hang from the left of a label that centres everything else. Breaking them
/// here needs a width to break at, and nothing measures text ahead of layout,
/// so this errs wide — a line that breaks early costs a line, one that breaks
/// late is cut by the card.
const CHAR_WIDTH_PER_PX: f32 = 0.6;

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
        // The direction first: the label rebuilds on the index, and reads
        // the direction as it does.
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
/// A fixed width, and as tall as the name with the most lines: guido cannot
/// measure text, so that name is drawn invisibly behind every label and the
/// visible one fills the box it makes. Every label is then the same size, and
/// nothing around the picker moves while the names change.
///
/// Rebuilt on every change of the index, drawing the name it left beside the
/// one it arrived at: the old one travels out from the middle, the new one in
/// from the far side, both clipped by the card. On the first build there is
/// nothing to leave, and the name simply sits there.
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
    let shown: Cell<Option<usize>> = Cell::new(None);

    let room = SESSION_LABEL_WIDTH - LABEL_PADDING[1] * 2.0;
    let per_line = (room / (LABEL_FONT_SIZE * CHAR_WIDTH_PER_PX)).floor() as usize;
    let lines: Vec<Vec<String>> = names.iter().map(|n| break_lines(n, per_line)).collect();
    let tallest = lines
        .iter()
        .max_by_key(|l| l.len())
        .cloned()
        .unwrap_or_default();

    let column = |lines: &[String], color: Color| {
        lines.iter().fold(
            container().layout(
                Flex::column()
                    .main_alignment(MainAlignment::Center)
                    .cross_alignment(CrossAlignment::Center),
            ),
            |column, line| {
                column.child(
                    text(line.clone())
                        .font_size(LABEL_FONT_SIZE)
                        .color(color)
                        .nowrap(),
                )
            },
        )
    };

    let name = move |i: usize, at: Translate, from: Option<Translate>| {
        let slide = Transition::new(SLIDE_MS, TimingFunction::EaseInOut);
        let translate = match from {
            Some(from) => at.transition(slide).entering_from(from),
            None => at.transition(slide),
        };
        container()
            .width(SESSION_LABEL_WIDTH)
            .padding(LABEL_PADDING)
            .layout(ZStack::new())
            .translate(translate)
            // Leads the height; the visible name follows it.
            .child(column(&tallest, Color::TRANSPARENT))
            .child(column(&lines[i], theme.text).width(fill()).height(fill()))
    };

    super::card_surface(screen)
        .corners(Corners::squircle(16.0))
        .child(move || {
            let current = index.get() % count;
            let previous = shown.replace(Some(current)).filter(|&p| p != current);
            let stack = container().layout(ZStack::new());
            let Some(previous) = previous else {
                return stack.child(name(current, Translate::NONE, None));
            };
            let by = if step.get_untracked() < 0 { -1.0 } else { 1.0 };
            let away = Translate::new(-by * SESSION_LABEL_WIDTH, 0.0);
            let beyond = Translate::new(by * SESSION_LABEL_WIDTH, 0.0);
            stack
                .child(name(previous, away, Some(Translate::NONE)))
                .child(name(current, Translate::NONE, Some(beyond)))
        })
}

/// Greedy word wrap at `per_line` characters. A single word longer than that
/// keeps its own line rather than being split mid-word.
fn break_lines(name: &str, per_line: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in name.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= per_line => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
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
                .layout(
                    Flex::column()
                        .main_alignment(MainAlignment::Center)
                        .cross_alignment(CrossAlignment::Center),
                )
                .child(icon().kind(glyph).font_size(18.0).color(theme.text)),
        )
        .when_hovered(|s| s.lighter(0.08))
        .when_pressed(|s| s.ripple())
        .on_click(on_click)
}

#[cfg(test)]
mod tests {
    use super::break_lines;

    #[test]
    fn a_name_that_fits_stays_on_one_line() {
        assert_eq!(break_lines("CustomHyprland", 19), ["CustomHyprland"]);
    }

    #[test]
    fn a_long_name_breaks_between_words() {
        assert_eq!(
            break_lines("Hyprland (uwsm-managed)", 19),
            ["Hyprland", "(uwsm-managed)"]
        );
    }

    #[test]
    fn a_word_longer_than_a_line_is_not_split() {
        assert_eq!(
            break_lines("Supercalifragilisticexpialidocious", 19),
            ["Supercalifragilisticexpialidocious"]
        );
    }
}
