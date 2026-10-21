//! Wall clock: one signal, changing on the minute, or on the second when a
//! format shows seconds.

use std::time::Duration;

use chrono::{DateTime, Local, Timelike};
use guido::prelude::*;

/// The local time, to the second when `seconds`, else to the minute. Format
/// it through a memo, or everything that reads it repaints at each change.
///
/// It wakes every second either way: tokio's timers stand still while the
/// machine sleeps, so one set for the next minute could leave the clock a
/// minute behind when the lid opens.
pub fn now(seconds: bool) -> Signal<DateTime<Local>> {
    let now = create_signal(Local::now());
    let writer = now.writer();

    create_task(move |ctx| async move {
        let mut shown = Local::now();
        while ctx.is_running() {
            let into_second = Local::now().nanosecond() % 1_000_000_000;
            let until_next = 1_000_000_000u32.saturating_sub(into_second);
            tokio::time::sleep(Duration::from_nanos(u64::from(until_next))).await;
            let time = Local::now();
            if seconds || time.timestamp() / 60 != shown.timestamp() / 60 {
                shown = time;
                writer.set(time);
            }
        }
    });

    now.read_only()
}

/// Whether a `strftime` format shows seconds.
pub fn shows_seconds(format: &str) -> bool {
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            continue;
        }
        // Flags and widths, then the conversion.
        let conversion = chars
            .by_ref()
            .find(|c| !matches!(c, '-' | '_' | '0'..='9' | '^' | '#' | ':' | '.'));
        if matches!(
            conversion,
            Some('S' | 'T' | 'X' | 'r' | 's' | 'c' | '+' | 'f')
        ) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::shows_seconds;

    #[test]
    fn seconds_are_seen_in_a_format() {
        assert!(!shows_seconds("%H:%M"));
        assert!(!shows_seconds("%A %e %B"));
        assert!(!shows_seconds("%%S"));
        assert!(shows_seconds("%H:%M:%S"));
        assert!(shows_seconds("%-S"));
        assert!(shows_seconds("%T"));
    }
}
