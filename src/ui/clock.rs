//! The big clock. It sits a third of the way down at rest, and moves up and
//! shrinks out of the card's way when the island comes down.

use guido::prelude::*;

use super::Screen;
use super::flow::Phase;
use ashell_ui::text::typed;
use ashell_ui::tokens::size;
use ashell_ui::typography::{Resolved, TITLE_MEDIUM};
use ashell_ui::ui;
use ashell_ui::{motion, roll};

/// The time's size and line, and how close its figures sit: past the type
/// scale, which stops at the island's numbers.
const TIME_SIZE: f32 = 118.0;
const TIME_TRACKING: f32 = -4.0;
const AWAKE_SCALE: f32 = 0.56;
/// The clock's centre, as a share of the output's height.
const REST_AT: f32 = 0.30;
const AWAKE_AT: f32 = 0.17;

pub fn view(screen: &Screen, height: Signal<f32>) -> Container {
    let flow = screen.flow;
    let phase = flow.phase;
    let now = screen.now;
    let time = screen.time;
    let date_format = screen.config.clock.date_format.clone();

    let date = create_memo(move || {
        // Collapse the padding `%e` leaves on single-digit days.
        now.get()
            .format(&date_format)
            .to_string()
            .replace("  ", " ")
    });
    let awake = create_memo(move || flow.is_down());
    let shown = create_memo(move || matches!(phase.get(), Phase::Rest | Phase::Awake | Phase::Ok));
    let clock_ref = create_widget_ref();

    let place = move || {
        let at = if awake.get() { AWAKE_AT } else { REST_AT };
        let own = clock_ref.layout_rect().get().height;
        Translate::new(0.0, height.get() * at - own / 2.0)
    };
    let springy = motion::spring;

    container()
        .widget_ref(clock_ref)
        .align_self(Alignment::TOP_CENTER)
        .layout(
            Flex::column()
                .spacing(size::S2.px())
                .cross_alignment(CrossAlignment::Center),
        )
        .translate(place.transition(springy()))
        .scale(
            (move || Scale::uniform(if awake.get() { AWAKE_SCALE } else { 1.0 }))
                .transition(springy()),
        )
        .opacity((move || if shown.get() { 1.0 } else { 0.0 }).transition(motion::easing(400.0)))
        .child(
            roll::rolling(time)
                .look(Resolved {
                    size: TIME_SIZE,
                    line: TIME_SIZE,
                    weight: FontWeight::EXTRA_BOLD,
                    tracking: TIME_TRACKING,
                    family: ui().fonts.sans,
                    tabular: true,
                })
                .color(move || Color::WHITE.with_alpha(if awake.get() { 0.92 } else { 0.6 })),
        )
        .child(move || {
            let date = date.get();
            (!date.is_empty()).then(|| {
                // On the wallpaper, as the time is: white whatever the theme.
                typed(date, TITLE_MEDIUM.em())
                    .color(Color::WHITE.with_alpha(0.7))
                    .nowrap()
            })
        })
}
