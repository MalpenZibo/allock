//! The island: one black shape at the top centre that springs to the size of
//! whatever it shows, and cross-fades between views as they change.

use guido::prelude::*;

use super::flow::{ARM_MS, Phase};
use super::icons::{Icon, weighted};
use super::{Overhang, Screen, card, rest, spring};
use crate::services::auth::NoticeLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Seed,
    Rest,
    Card,
    Ok,
}

/// Where the card sits once someone is at it, from the middle of the output:
/// the island comes down to meet them.
const AWAKE_ABOVE_MIDDLE: f32 = 140.0;
/// Up to this height the island is a pill; past it, a card with these corners.
const PILL_HEIGHT: f32 = 44.0;
const CARD_CORNERS: f32 = 30.0;
/// Width grows first and height follows; shrinking, height goes first.
const HEIGHT_LAG_MS: f32 = 45.0;
const WIDTH_LAG_MS: f32 = 55.0;

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

    // The view on show measures itself; the shape follows. A view not laid
    // out yet has no size, and is not a target.
    let measured = std::rc::Rc::new(std::cell::Cell::new(None::<WidgetRef>));
    let built = create_signal(0u64);
    let size = create_signal((seed.size, seed.size));
    create_effect({
        let measured = measured.clone();
        move || {
            built.get();
            let target = if shown.get() == View::Seed {
                (seed.size, seed.size)
            } else {
                let Some(view) = measured.get() else { return };
                let rect = view.layout_rect().get();
                if rect.width <= 0.0 {
                    return;
                }
                (rect.width, rect.height)
            };
            size.set(target);
        }
    });

    let glow = create_memo(move || {
        if flow.phase.get() == Phase::Ok {
            Glow::Ok
        } else if !controls.get() || flow.phase.get() != Phase::Awake {
            Glow::None
        } else if state.locked.get().is_some() {
            Glow::Warning
        } else if state.field_is_empty()
            && matches!(state.notice.get(), Some(n) if n.level == NoticeLevel::Error)
        {
            Glow::Error
        } else {
            Glow::None
        }
    });

    let theme = screen.theme;
    let ring = move || match glow.get() {
        Glow::None => Color::TRANSPARENT,
        Glow::Error => theme.danger,
        Glow::Ok => theme.success,
        Glow::Warning => theme.warning,
    };
    let shadows = move || {
        let ring = ring();
        Shadows::new([
            Shadow::new((0.0, 18.0), 50.0, 0.0, Color::BLACK.with_alpha(0.5)),
            Shadow::new((0.0, 0.0), 0.0, 1.5, ring),
            Shadow::new((0.0, 0.0), 40.0, -6.0, ring.with_alpha(ring.a * 0.8)),
            Shadow::new((0.0, 0.0), 0.0, 1.0, Color::WHITE.with_alpha(0.05)).inset(),
        ])
    };

    let springy = || Transition::spring(spring());
    let shape = container()
        .width((move || size.get().0).transition(springy().reverse(springy().delay(WIDTH_LAG_MS))))
        .height(
            (move || size.get().1).transition(springy().delay(HEIGHT_LAG_MS).reverse(springy())),
        )
        .corners(
            (move || {
                let (w, h) = size.get();
                if h <= PILL_HEIGHT {
                    w.min(h) / 2.0
                } else {
                    CARD_CORNERS
                }
            })
            .transition(springy()),
        )
        .background(theme.island)
        .shadow(shadows.transition(400.0))
        .overflow(Overflow::Hidden)
        .layout(Overhang)
        // Played by the refusal count, so every refusal shakes it again.
        .translate(Translate::NONE.timeline(shake().played_by(state.rejections)))
        .child({
            let screen = screen.clone();
            move || {
                let view = create_widget_ref();
                measured.set(Some(view));
                built.update(|n| *n += 1);
                let content = match shown.get() {
                    View::Seed => container().width(seed.size).height(seed.size),
                    View::Rest => rest::view(&screen, controls),
                    View::Card => card::view(&screen),
                    View::Ok => ok(&screen),
                };
                arriving(content).widget_ref(view)
            }
        });

    // Down to meet whoever is typing; back to the seed's place otherwise.
    let top = move || {
        let down = controls.get() && flow.is_down();
        let y = if down {
            (height.get() / 2.0 - AWAKE_ABOVE_MIDDLE).max(seed.top)
        } else {
            seed.top
        };
        Translate::new(0.0, y)
    };

    container()
        .width(fill())
        .layout(Flex::column().cross_alignment(CrossAlignment::Center))
        .translate(top.transition(springy()))
        .child(shape)
}

/// A view arrives out of focus and slightly small, and leaves quickly the same
/// way, slightly large: the island's content changes under one shape.
fn arriving(content: Container) -> Container {
    let enter = || Transition::new(300.0, TimingFunction::EaseOut).delay(70.0);
    let leave = || Transition::new(160.0, TimingFunction::EaseOut);
    container()
        .opacity(
            1.0.transition(enter().reverse(leave()))
                .entering_from(0.0)
                .exiting_to(0.0),
        )
        .blur(
            0.0.transition(leave().reverse(enter()))
                .entering_from(8.0)
                .exiting_to(8.0),
        )
        .scale(
            Scale::NONE
                .transition(Transition::spring(spring()))
                .entering_from(Scale::uniform(0.92))
                .exiting_to(Scale::uniform(1.05)),
        )
        .pivot(Pivot::TOP)
        .child(content)
}

fn ok(screen: &Screen) -> Container {
    container()
        .width(76.0)
        .height(76.0)
        .layout(Flex::column().center())
        .child(
            weighted(Icon::Check, 34.0, 3.0, screen.theme.success).scale(
                Scale::NONE
                    .transition(Transition::spring(SpringConfig::BOUNCY))
                    .entering_from(Scale::uniform(0.4)),
            ),
        )
}

fn shake() -> Keyframes<Translate> {
    Keyframes::new(420.0)
        .at(0.0, Translate::NONE)
        .at(0.15, Translate::new(-9.0, 0.0))
        .at(0.35, Translate::new(8.0, 0.0))
        .at(0.55, Translate::new(-6.0, 0.0))
        .at(0.75, Translate::new(3.0, 0.0))
        .at(1.0, Translate::NONE)
}

/// Whether a card built `ARM_MS` ago may be pressed yet.
pub fn armed() -> Signal<bool> {
    let armed = create_signal(false);
    set_timeout(std::time::Duration::from_millis(ARM_MS), move || {
        armed.set(true)
    });
    armed.into()
}
