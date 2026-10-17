//! The big clock. It sits a third of the way down at rest, and moves up and
//! shrinks out of the card's way when the island comes down.

use guido::prelude::*;

use super::Screen;
use super::flow::Phase;
use ashell_ui::motion;
use ashell_ui::roll;

const TIME: roll::Look = roll::Look {
    size: 118.0,
    line: 118.0,
    weight: FontWeight::EXTRA_BOLD,
    spacing: -4.0,
};
const AWAKE_SCALE: f32 = 0.56;
/// The clock's centre, as a share of the output's height.
const REST_AT: f32 = 0.30;
const AWAKE_AT: f32 = 0.17;

pub fn view(screen: &Screen, height: Signal<f32>) -> Container {
    let theme = screen.theme;
    let phase = screen.flow.phase;
    let now = screen.now;
    let time_format = screen.config.clock.time_format.clone();
    let date_format = screen.config.clock.date_format.clone();

    let time = create_memo(move || now.get().format(&time_format).to_string());
    let date = create_memo(move || {
        // Collapse the padding `%e` leaves on single-digit days.
        now.get()
            .format(&date_format)
            .to_string()
            .replace("  ", " ")
    });
    let awake = create_memo(move || matches!(phase.get(), Phase::Awake | Phase::Ok));
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
                .spacing(10.0)
                .cross_alignment(CrossAlignment::Center),
        )
        .translate(place.transition(springy()))
        .scale(
            (move || Scale::uniform(if awake.get() { AWAKE_SCALE } else { 1.0 }))
                .transition(springy()),
        )
        .opacity((move || if shown.get() { 1.0 } else { 0.0 }).transition(400.0))
        .child(roll::rolling(time, TIME, move || {
            Color::WHITE.with_alpha(if awake.get() { 0.92 } else { 0.6 })
        }))
        .child(move || {
            let date = date.get();
            (!date.is_empty()).then(|| {
                text(date)
                    .font_size(20.0)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .color(theme.colors.text.with_alpha(0.7))
                    .nowrap()
            })
        })
}
