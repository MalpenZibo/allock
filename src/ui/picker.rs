//! A value chosen by stepping through a short list: a name between a previous
//! and a next arrow, sliding as it changes.

use guido::prelude::*;

use super::Screen;
use super::tokens::{self, Type, Typography, icon_size, radius, space};
use crate::components::icons::{StaticIcon, icon};

/// Exact, so the arrows do not jump with every name.
const LABEL_WIDTH: f32 = 200.0;

const LABEL_PADDING: [f32; 2] = [space::S8, space::S16];

const ARROW_SIZE: f32 = space::S40;

const SLIDE_MS: f32 = 260.0;

/// Which entry is chosen, and which way the last step went.
#[derive(Clone, Copy)]
pub struct Choice {
    pub index: RwSignal<usize>,
    /// `1` next, `-1` previous, `0` before the first step. With two entries both
    /// arrows land on the same index, so the slide needs this.
    pub step: RwSignal<isize>,
}

/// How the chosen name is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// On a frosted card of its own.
    Card,
    /// The text alone, for a picker already inside a card.
    Plain,
}

impl Look {
    fn typography(self) -> Type {
        match self {
            Look::Card => tokens::LABEL_L,
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

/// The chosen name between two arrows, which do nothing while `held`.
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
        // The direction first: the label rebuilds on the index and reads it.
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

/// The window the names slide through. Every name is drawn invisibly behind the
/// visible one, so the label is as tall as the longest and never resizes.
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
                    // Read on removal, so it leaves the way the replacing step went.
                    .exiting_to(move || Translate::relative(-step.get() as f32, 0.0)),
            )
    };

    let window = match look {
        Look::Card => super::card_surface(screen).corners(Corners::squircle(radius::R16)),
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
