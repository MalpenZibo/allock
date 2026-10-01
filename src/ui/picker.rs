//! A value chosen by stepping through a short list: a name between a previous
//! and a next arrow, sliding as it changes.
//!
//! A popup menu would be the richer answer, but a greeter runs before anything
//! else exists and its lists — sessions, users — are normally two or three
//! entries. A name you step through keeps the whole thing legible.

use guido::prelude::*;

use super::Screen;
use super::tokens::{self, Type, Typography, icon_size, radius, space};
use crate::components::icons::{StaticIcon, icon};

/// Exact, for the reason the field's width is: the window the names slide
/// through has to stay put while they do, or the arrows either side of it
/// would jump with every name. A name too long for it wraps.
const LABEL_WIDTH: f32 = 200.0;

/// The padding around a name inside its window.
const LABEL_PADDING: [f32; 2] = [space::S8, space::S16];

/// The round buttons either side of the name.
const ARROW_SIZE: f32 = space::S40;

/// How long a name takes to cross the window.
const SLIDE_MS: f32 = 260.0;

/// Which entry is chosen, and which way the last step went.
#[derive(Clone, Copy)]
pub struct Choice {
    pub index: RwSignal<usize>,
    /// `1` for next, `-1` for previous, `0` before the first step. Only the
    /// slide reads it — with two entries both arrows land on the same index,
    /// so the index alone cannot say which way to move.
    pub step: RwSignal<isize>,
}

/// How the chosen name is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// On a frosted card of its own, for a picker standing on the wallpaper.
    Card,
    /// The text alone, at a heading's size, for a picker inside a card that is
    /// already there: a card on a card only thickens the frame.
    Plain,
}

impl Look {
    fn typography(self) -> Type {
        match self {
            Look::Card => tokens::LABEL_L,
            // The account name's own style.
            Look::Plain => tokens::TITLE_L,
        }
    }
}

impl Choice {
    pub fn new(index: usize) -> Self {
        Self {
            index: create_signal(index),
            step: create_signal(0),
        }
    }
}

/// The chosen name between a previous and a next arrow; the name alone when
/// there is only one.
///
/// Stepping slides the old name out and the new one in from the side the
/// arrow points to, so the list reads as a strip seen through a window.
///
/// While `held` is true the arrows do nothing: a choice that the attempt in
/// flight was made for must not change under it.
pub fn view(
    screen: &Screen,
    names: Vec<String>,
    choice: Choice,
    look: Look,
    held: Signal<bool>,
) -> Container {
    let count = names.len();
    if count == 0 {
        return container();
    }

    let label = label(screen, names, choice, look);
    if count == 1 {
        return label;
    }

    let Choice { index, step } = choice;
    let go = move |by: isize| {
        if held.get_untracked() {
            return;
        }
        // The direction first: the label rebuilds on the index, and both the
        // name arriving and the one leaving read the direction as it does.
        step.set(by);
        index.update(|i| *i = (*i as isize + by).rem_euclid(count as isize) as usize);
    };

    container()
        .layout(
            Flex::row()
                .spacing(space::S12)
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
/// width and both clipped by the window; a name still leaving when the next
/// step comes finishes from where it is. `step` is `0` until the first step,
/// so the first name enters from where it rests: it simply sits there.
///
/// The direction comes from `step` rather than from comparing indices,
/// because with two entries next and previous land on the same one.
fn label(screen: &Screen, names: Vec<String>, choice: Choice, look: Look) -> Container {
    let Choice { index, step } = choice;
    let theme = screen.theme;
    let count = names.len();
    let style = look.typography();

    let line = move |name: &str, color: Color| {
        text(name.to_owned())
            .typography(style)
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
                    .width(LABEL_WIDTH)
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

    let window = match look {
        Look::Card => super::card_surface(screen).corners(Corners::squircle(radius::R16)),
        // Still clipped: the names slide in from outside it.
        Look::Plain => container().overflow(Overflow::Hidden),
    };
    window.child(move || name(index.get() % count))
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
                .child(
                    icon()
                        .kind(glyph)
                        .font_size(icon_size::I16)
                        .color(theme.text),
                ),
        )
        .when_hovered(|s| s.lighter(0.08))
        .when_pressed(|s| s.ripple())
        .cursor(CursorIcon::Pointer)
        .on_click(on_click)
}
