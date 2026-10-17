//! The island at rest: a lock, whose session it is, what needs saying, the
//! battery, and the time where the big clock is not there to tell it.

use guido::prelude::*;

use super::card::{battery_ring, caps_chip, lockout_chip};
use super::icons::{Icon, icon};
use super::{Screen, roll};
use crate::Mode;

pub fn view(screen: &Screen, controls: Signal<bool>) -> Container {
    let theme = screen.theme;
    let flow = screen.flow;
    let now = screen.now;
    let time_format = screen.config.clock.time_format.clone();
    let time = create_memo(move || now.get().format(&time_format).to_string());
    let hint = match screen.mode {
        Mode::Greeter => screen.host.to_string(),
        Mode::Lock | Mode::Preview => "type to unlock".to_owned(),
    };

    container()
        .height(34.0)
        .padding(Padding {
            top: 0.0,
            right: 10.0,
            bottom: 0.0,
            left: 13.0,
        })
        .layout(
            Flex::row()
                .spacing(9.0)
                .cross_alignment(CrossAlignment::Center)
                .main_alignment(MainAlignment::Center),
        )
        .on_click(move || flow.wake())
        .child(icon(
            match screen.mode {
                Mode::Greeter => Icon::Power,
                Mode::Lock | Mode::Preview => Icon::Lock,
            },
            14.0,
            theme.sub,
        ))
        .child(
            container()
                .visible(move || !controls.get())
                .child(roll::view(
                    time,
                    roll::Look {
                        size: 13.0,
                        line: 16.0,
                        weight: FontWeight::SEMI_BOLD,
                        spacing: 0.0,
                    },
                    move || theme.text,
                )),
        )
        .child(lockout_chip(screen, false))
        .child(caps_chip(screen, "Caps"))
        .child({
            let user = screen.user;
            let greeter = screen.mode == Mode::Greeter;
            // The greeter has nobody yet: it names the machine instead.
            move || {
                let name = user.get().filter(|_| !greeter)?.display_name;
                Some(
                    text(name)
                        .font_size(12.0)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .color(theme.sub)
                        .nowrap(),
                )
            }
        })
        .child(text(hint).font_size(12.0).color(theme.dim).nowrap())
        .child(battery_ring(screen, 18.0))
}
