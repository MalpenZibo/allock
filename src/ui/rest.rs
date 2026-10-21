//! The island at rest: a lock, whose session it is, what needs saying, the
//! battery, and the time where the big clock is not there to tell it.

use guido::prelude::*;

use super::Screen;
use super::card::{caps_chip, gauge, lockout_chip};
use crate::Mode;
use ashell_ds::text::typed;
use ashell_ds::tokens::{icon as icon_box, size};
use ashell_ds::typography::LABEL_SMALL;
use ashell_ds::{Icon, centered_row, icon, roll, ui};

/// The bar's compact island's, so the one becomes the other.
pub const HEIGHT: f32 = 34.0;

pub fn view(screen: &Screen, controls: Signal<bool>) -> Container {
    let t = ui();
    let flow = screen.flow;
    let time = screen.time;
    let hint = match screen.mode {
        Mode::Greeter => screen.host.to_string(),
        Mode::Lock | Mode::Preview => "type to unlock".to_owned(),
    };

    container()
        .height(HEIGHT)
        .padding([0.0, 10.0, 0.0, 12.0])
        .layout(centered_row(size::S2.px()).main_alignment(MainAlignment::Center))
        .on_click(move || flow.wake())
        .child(icon(
            match screen.mode {
                Mode::Greeter => Icon::Power,
                Mode::Lock | Mode::Preview => Icon::Lock,
            },
            icon_box::XSMALL.px(),
            t.colors.sub,
        ))
        .child(
            container()
                .visible(move || !controls.get())
                .child(roll::rolling(time).color(t.colors.text)),
        )
        .child(lockout_chip(screen.auth.state.locked))
        .child(caps_chip("Caps"))
        .child({
            let user = screen.user;
            let greeter = screen.mode == Mode::Greeter;
            // The greeter has nobody yet: it names the machine instead.
            move || {
                let name = user.get().filter(|_| !greeter)?.display_name;
                Some(typed(name, LABEL_SMALL.em()).color(t.colors.sub).nowrap())
            }
        })
        .child(typed(hint, LABEL_SMALL).color(t.colors.dim).nowrap())
        .child(gauge(screen.battery, icon_box::SMALL))
}
