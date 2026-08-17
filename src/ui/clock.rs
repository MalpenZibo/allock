//! Wall clock: one signal, ticking on the second boundary.

use std::time::Duration;

use chrono::{DateTime, Local, Timelike};
use guido::prelude::*;

use crate::config::ClockConfig;
use crate::theme::ThemeColors;

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

pub fn view(now: Signal<DateTime<Local>>, config: &ClockConfig, theme: ThemeColors) -> Container {
    let time_format = config.time_format.clone();
    let date_format = config.date_format.clone();

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

    container()
        .layout(
            Flex::column()
                .spacing(2.0)
                .cross_alignment(CrossAlignment::Center),
        )
        // The colour is inherited from the screen root; only the sizes and the
        // dimmer date differ from it.
        .child(
            container()
                .font_size(76.0)
                .child(text(move || time.get()).nowrap()),
        )
        .child(
            container()
                .font_size(18.0)
                .text_color(theme.text_weak)
                .child(move || {
                    let date = date.get();
                    (!date.is_empty()).then(|| text(date).nowrap())
                }),
        )
}
