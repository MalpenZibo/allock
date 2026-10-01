//! Wall clock: one signal, ticking on the second boundary.

use std::time::Duration;

use chrono::{DateTime, Local, Timelike};
use guido::prelude::*;

use super::Screen;
use super::tokens::{self, Typography, space};

/// The clock's glyphs show the wallpaper behind them, blurred.
const FROST: f32 = 16.0;

/// On glass guido draws this outside the letter only, so it does not fill the glyph.
const STROKE: TextStroke = TextStroke {
    width: 2.0,
    color: Color::rgba(0.0, 0.0, 0.0, 0.75),
};

/// Text alpha over the glass: untinted glass is only as legible as the wallpaper.
const TINT: f32 = 0.35;

/// The local time, ticking on the second boundary. Format it through a memo, or
/// the whole screen repaints every second.
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

/// Glass text when there is a wallpaper; plain text otherwise.
fn frosted(text: Text, color: Color, glass: bool) -> Text {
    if glass {
        text.color(color.with_alpha(TINT))
            .backdrop_blur(FROST)
            .text_stroke(STROKE)
    } else {
        text.color(color)
    }
}
