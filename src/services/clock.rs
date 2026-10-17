//! Wall clock: one signal, ticking on the second boundary.

use std::time::Duration;

use chrono::{DateTime, Local, Timelike};
use guido::prelude::*;

/// The local time. Format it through a memo, or everything that reads it
/// repaints every second.
pub fn now() -> Signal<DateTime<Local>> {
    let now = create_signal(Local::now());
    let writer = now.writer();

    create_task(move |ctx| async move {
        while ctx.is_running() {
            let into_second = Local::now().nanosecond() % 1_000_000_000;
            let until_next = 1_000_000_000u32.saturating_sub(into_second);
            tokio::time::sleep(Duration::from_nanos(u64::from(until_next))).await;
            writer.set(Local::now());
        }
    });

    now.read_only()
}
