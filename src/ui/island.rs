//! The island: one black shape at the top centre that springs to the size of
//! whatever it shows, and cross-fades between views as they change.

use guido::prelude::*;

use super::flow::Phase;
use super::{Screen, card, rest};
use crate::services::auth::NoticeLevel;
use ashell_ui::motion::{self, arriving, shake};

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
const CARD_CORNERS: f32 = 44.0;
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
        Glow::Error => theme.colors.danger,
        Glow::Ok => theme.colors.success,
        Glow::Warning => theme.colors.warning,
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

    let springy = || motion::spring();
    let shape = container()
        .width((move || size.get().0).transition(springy().reverse(springy().delay(WIDTH_LAG_MS))))
        .height(
            (move || size.get().1).transition(springy().delay(HEIGHT_LAG_MS).reverse(springy())),
        )
        .corners(
            (move || {
                let (w, h) = size.get();
                // A pill and the seed are capsules, and the seed has to be
                // the circle the bar draws; the card has continuous corners.
                // The curvature eases between the two with the radius.
                if h <= PILL_HEIGHT {
                    Corners::rounded(w.min(h) / 2.0)
                } else {
                    Corners::squircle(CARD_CORNERS)
                }
            })
            .transition(springy()),
        )
        .background(theme.colors.island)
        .shadow(shadows.transition(400.0))
        .overflow(Overflow::Hidden)
        .layout(ZStack::new().align(Alignment::TOP_CENTER))
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
                // At its own size whatever the shape's is: the shape springs
                // around it and clips it, and nothing inside re-wraps.
                arriving(content)
                    .width(natural())
                    .height(natural())
                    .widget_ref(view)
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

fn ok(screen: &Screen) -> Container {
    container()
        .width(76.0)
        .height(76.0)
        .layout(Flex::column().center())
        .child(super::tick::tick(
            34.0,
            3.0 * 34.0 / 24.0,
            screen.theme.colors.success,
        ))
}
