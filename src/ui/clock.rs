//! Wall clock: one signal, ticking on the second boundary.

use std::time::Duration;

use chrono::{DateTime, Local, Timelike};
use guido::prelude::*;

use super::Screen;

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
/// The hour keeps none: with a contour around it there is nothing else the
/// fill has to do, and a hole is the whole point. The date keeps a little,
/// being a third of the size, where a pure hole reads as a smudge.
const HOUR_TINT: f32 = 0.0;
const DATE_TINT: f32 = 0.35;

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

    let _ = create_service::<(), _, _>(move |_rx, ctx| async move {
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

    let clock = container()
        .layout(
            Flex::column()
                .spacing(2.0)
                .cross_alignment(CrossAlignment::Center),
        )
        // The colour is inherited from the screen root; only the sizes, the
        // glass and the dimmer date differ from it.
        .child(frosted(
            text(move || time.get()).font_size(96.0).nowrap(),
            theme.text,
            HOUR_TINT,
            glass,
        ))
        .child(move || {
            let date = date.get();
            (!date.is_empty()).then(|| {
                frosted(
                    text(date).font_size(42.0).nowrap(),
                    theme.text_weak,
                    DATE_TINT,
                    glass,
                )
            })
        });

    if glass {
        clock.text_stroke(STROKE)
    } else {
        clock
    }
}

/// A text made of glass, or left alone.
///
/// `tint` is what survives of `color` over the blur: the fill is drawn *over*
/// the frost, so an opaque one would hide the very thing being blurred.
fn frosted(text: Text, color: Color, tint: f32, glass: bool) -> Text {
    if glass {
        text.color(color.with_alpha(tint)).backdrop_blur(FROST)
    } else {
        // Nothing behind it to blur — a blur of a flat colour is that same flat
        // colour — so it goes back to being ordinary text.
        text.color(color)
    }
}
