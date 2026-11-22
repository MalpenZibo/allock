//! The island: styletto's shape, at the top centre, showing the seed, the
//! rest, the card or the tick, and coming down to meet whoever types.

use guido::prelude::*;

use super::flow::Phase;
use super::{Screen, card, rest};
use styletto::foundations::control::Size;
use styletto::foundations::motion::{self, shake};
use styletto::foundations::tokens::{icon as icon_box, radius, size, stroke};
use styletto::primitives::island::{self, above_edge, island};
use styletto::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum View {
    Seed,
    Rest,
    Card,
    Ok,
}

/// Where the card sits once someone is at it, from the middle of the output:
/// the island comes down to meet them.
const AWAKE_ABOVE_MIDDLE: f32 = 140.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Glow {
    None,
    Error,
    Ok,
    Warning,
}

pub fn view(screen: &Screen, controls: Signal<bool>, height: Signal<f32>) -> Container {
    let flow = screen.flow;
    let state = screen.auth.state;
    let seed = screen.config.island.seed;

    let shown = create_memo(move || match (flow.phase.get(), controls.get()) {
        (Phase::Seed | Phase::Gone, _) => View::Seed,
        (Phase::Rest, _) | (_, false) => View::Rest,
        (Phase::Awake, true) => View::Card,
        (Phase::Ok, true) => View::Ok,
    });

    let glow = create_memo(move || {
        if flow.phase.get() == Phase::Ok {
            Glow::Ok
        } else if !controls.get() || flow.phase.get() != Phase::Awake {
            Glow::None
        } else if state.locked.get().is_some() {
            Glow::Warning
        } else if state.refused() {
            Glow::Error
        } else {
            Glow::None
        }
    });

    let t = ui();
    let ring = move || match glow.get() {
        Glow::None => Color::TRANSPARENT,
        Glow::Error => t.colors.danger.fill,
        Glow::Ok => t.colors.success.fill,
        Glow::Warning => t.colors.warning.fill,
    };
    // The card's corners are the field's, its padding out, so the curves
    // stay concentric.
    let card_corners = radius::around(Size::Lg.height() / 2.0, size::S6.px());
    let corners = move || match shown.get() {
        // The seed a circle, the rest a pill.
        View::Seed => Corners::rounded(seed.size / 2.0),
        View::Rest => Corners::rounded(rest::HEIGHT / 2.0),
        View::Card | View::Ok => radius::shape(card_corners),
    };

    let shape = island(
        shown,
        {
            let screen = screen.clone();
            move |view| match view {
                View::Seed => container().width(seed.size).height(seed.size),
                View::Rest => rest::view(&screen, controls),
                View::Card => card::view(&screen),
                View::Ok => ok(),
            }
        },
        corners,
    )
    .shadow((move || island::shadow(ring())).transition(motion::easing(400.0)))
    // Played by the refusal count, so every refusal shakes it again.
    .translate(Translate::NONE.timeline(shake().played_by(state.rejections)));

    // Down to meet whoever is typing, back to its place at the top
    // otherwise. Before it arrives and once it has gone it is up past the
    // output's top edge: it drops in from there and leaves into it, as the
    // bar's island does, so nothing is left on screen across the cut.
    let top = move || match flow.phase.get() {
        Phase::Seed | Phase::Gone => {
            let up = above_edge(seed.top, seed.size);
            Translate::new(0.0, seed.top + up.y)
        }
        _ if controls.get() && flow.is_down() => {
            Translate::new(0.0, (height.get() / 2.0 - AWAKE_ABOVE_MIDDLE).max(seed.top))
        }
        _ => Translate::new(0.0, seed.top),
    };
    container()
        .align_self(Alignment::TOP_CENTER)
        .translate(top.transition(motion::spring()))
        .child(shape)
}

fn ok() -> Container {
    let side = icon_box::XLARGE.px();
    container()
        .width(size::S20.px())
        .height(size::S20.px())
        .layout(Flex::column().center())
        .child(super::tick::tick(
            side,
            stroke::HEAVY * side / 24.0,
            ui().colors.success.text,
        ))
}
