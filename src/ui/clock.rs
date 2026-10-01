//! Wall clock: one signal, ticking on the second boundary.

use std::time::Duration;

use chrono::{DateTime, Local, Timelike};
use guido::prelude::*;

use super::Screen;
use super::tokens::{self, Typography, space};

/// The clock is glass: the glyphs show the wallpaper behind them, blurred.
///
/// Nothing else on the screen does this, and that is the point — the clock is
/// the one thing sitting directly on the picture, everything else is on a card
/// that already blurs its own box.
const FROST: f32 = 16.0;

/// The contour that draws the clock's edge.
///
/// On glass this is a real outline: guido dilates the coverage mask the frost
/// already needs and draws the band outside the letter. The cheap stroke —
/// copies of the glyphs under the fill — would ring the letter *and* fill it,
/// which under an opaque fill nobody notices and over glass is a black clock.
/// That was the first thing this screen showed me.
const STROKE: TextStroke = TextStroke {
    width: 2.0,
    color: Color::rgba(0.0, 0.0, 0.0, 0.75),
};

/// What is left of the text colour over the glass.
///
/// The same on both lines. A hole with no tint at all is only as legible as
/// whatever happens to be behind it, and the wallpaper is not something the
/// clock gets to choose; a third of the colour holds the numerals together and
/// still reads as glass. The contour above draws the edge, this fills it.
const TINT: f32 = 0.35;

/// A signal holding the current local time.
///
/// The tick is aligned to the next whole second rather than a fixed interval,
/// so a displayed minute never lags behind the system clock by an accumulated
/// drift. Consumers should format through a memo — the formatted string
/// usually only changes once a minute, and the memo's `PartialEq` is what
/// stops a per-second repaint of the whole screen.
pub fn create() -> Signal<DateTime<Local>> {
    let now = create_signal(Local::now());
    let writer = now.writer();

    create_task(move |ctx| async move {
        while ctx.is_running() {
            let nanos_into_second = Local::now().nanosecond() % 1_000_000_000;
            let until_next_second = 1_000_000_000u32.saturating_sub(nanos_into_second);
            tokio::time::sleep(Duration::from_nanos(u64::from(until_next_second))).await;
            writer.set(Local::now());
        }
    });

    now.read_only()
}

pub fn view(screen: &Screen) -> Container {
    let now = screen.now;
    let theme = screen.theme;
    let time_format = screen.config.clock.time_format.clone();
    let date_format = screen.config.clock.date_format.clone();

    let time = create_memo(move || now.get().format(&time_format).to_string());
    let date = create_memo(move || {
        if date_format.is_empty() {
            String::new()
        } else {
            // Collapse the padding `%e` leaves on single-digit days.
            now.get()
                .format(&date_format)
                .to_string()
                .replace("  ", " ")
        }
    });

    let glass = screen.wallpaper().is_some();

    container()
        .layout(
            Flex::column()
                .spacing(space::S4)
                .cross_alignment(CrossAlignment::Center),
        )
        // A container draws a box and says nothing about the glyphs inside it,
        // so every colour here is named where it is drawn.
        .child(frosted(
            text(move || time.get())
                .typography(tokens::DISPLAY_L)
                .nowrap(),
            theme.text,
            glass,
        ))
        .child(move || {
            let date = date.get();
            (!date.is_empty()).then(|| {
                frosted(
                    text(date).typography(tokens::HEADLINE_L).nowrap(),
                    theme.text_weak,
                    glass,
                )
            })
        })
}

/// A text made of glass, or left alone.
///
/// Over the frost the colour arrives at [`TINT`], because the fill is drawn
/// *over* what the blur put inside the letter and an opaque one would hide it.
/// Only the colour is the caller's: how much of it survives the glass, and the
/// contour that draws its edge, are the same everywhere on this clock.
fn frosted(text: Text, color: Color, glass: bool) -> Text {
    if glass {
        text.color(color.with_alpha(TINT))
            .backdrop_blur(FROST)
            .text_stroke(STROKE)
    } else {
        // Nothing behind it to blur — a blur of a flat colour is that same flat
        // colour — so it goes back to being ordinary text.
        text.color(color)
    }
}
