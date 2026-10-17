//! The card the island opens into: who is signing in, the field, what the
//! auth stack has to say, and a drawer for the session and the power menu.

use std::time::SystemTime;

use guido::prelude::*;

use super::field;
use super::flow::Zone;
use super::icons::{Icon, icon};
use super::island::armed;
use super::theme::Theme;
use super::{Screen, roll, spring};
use crate::Mode;
use crate::services::auth::{FingerprintState, Lock, NoticeLevel};
use crate::services::power::PowerAction;
use crate::system::user::UserInfo;

const WIDTH: f32 = 400.0;

pub fn view(screen: &Screen) -> Container {
    let armed = armed();

    let body = container()
        .width(WIDTH)
        .padding(Padding {
            top: 22.0,
            right: 22.0,
            bottom: 18.0,
            left: 22.0,
        })
        .layout(
            Flex::column()
                .spacing(12.0)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(top(screen))
        .child(match screen.mode {
            Mode::Greeter if !screen.users.is_empty() => users(screen, armed),
            _ => identity(screen),
        })
        .child(field::view(screen))
        .child(caps_chip(screen, "Caps Lock is on"))
        .child(message(screen))
        .child(zones(screen, armed))
        .child(drawer(screen, armed));

    container()
        .layout(ZStack::new())
        .child(body)
        .child(guard(screen.theme))
}

/// "Locked · 12:49" and the battery.
fn top(screen: &Screen) -> Container {
    let theme = screen.theme;
    let now = screen.now;
    let time_format = screen.config.clock.time_format.clone();
    let time = create_memo(move || now.get().format(&time_format).to_string());
    let what = match screen.mode {
        Mode::Greeter => screen.host.to_string(),
        Mode::Lock | Mode::Preview => "Locked".to_owned(),
    };
    let small = move |words: String| {
        text(words)
            .font_size(11.5)
            .line_height(LineHeight::Absolute(16.0))
            .font_weight(FontWeight::SEMI_BOLD)
            .color(theme.dim)
            .nowrap()
    };
    let look = roll::Look {
        size: 11.5,
        line: 16.0,
        weight: FontWeight::SEMI_BOLD,
        spacing: 0.0,
    };

    container()
        .width(fill())
        .layout(
            Flex::row()
                .main_alignment(MainAlignment::SpaceBetween)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(
            container()
                .layout(Flex::row().cross_alignment(CrossAlignment::Center))
                .child(container().child(small(format!("{what} · "))))
                .child(roll::view(time, look, move || theme.dim)),
        )
        .child({
            let screen = screen.clone();
            let battery = screen.battery;
            container()
                .layout(
                    Flex::row()
                        .spacing(8.0)
                        .cross_alignment(CrossAlignment::Center),
                )
                .child(battery_ring(&screen, 16.0))
                .child(move || {
                    battery
                        .get()
                        .map(|battery| container().child(small(format!("{}%", battery.percent))))
                })
        })
}

/// The avatar in the fingerprint ring, the name, the account.
fn identity(screen: &Screen) -> Container {
    let theme = screen.theme;
    let user = screen.user;
    let host = screen.host.clone();

    container()
        .layout(
            Flex::column()
                .spacing(10.0)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(
            container()
                .width(96.0)
                .height(96.0)
                .layout(ZStack::new())
                .child(fingerprint_ring(screen))
                .child(
                    container()
                        .width(96.0)
                        .height(96.0)
                        .layout(Flex::column().center())
                        .child(move || avatar(theme, user.get().as_ref(), 76.0)),
                ),
        )
        .child(move || {
            let user = user.get();
            let name = user
                .as_ref()
                .map(|u| u.display_name.clone())
                .unwrap_or_default();
            let account = user
                .map(|u| format!("{}@{host}", u.name))
                .unwrap_or_default();
            container()
                .layout(
                    Flex::column()
                        .spacing(2.0)
                        .cross_alignment(CrossAlignment::Center),
                )
                .child(
                    text(name)
                        .font_size(16.0)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .color(theme.text)
                        .nowrap(),
                )
                .child(
                    text(account)
                        .font_size(11.5)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.dim)
                        .nowrap(),
                )
        })
}

/// The picture, or the initial on the accent.
fn avatar(theme: Theme, user: Option<&UserInfo>, size: f32) -> Container {
    let disc = container()
        .width(size)
        .height(size)
        .corners(size / 2.0)
        .overflow(Overflow::Hidden)
        .gradient(LinearGradient::new(
            theme.accent,
            theme.danger,
            GradientDirection::Diagonal,
        ))
        .layout(Flex::column().center());
    match user {
        Some(UserInfo {
            avatar: Some(path), ..
        }) => disc.child(image(ImageSource::Path(path.clone())).content_fit(ContentFit::Cover)),
        Some(user) => disc.child(
            text(
                user.display_name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default(),
            )
            .font_size(size * 0.4)
            .font_weight(FontWeight::BOLD)
            .color(theme.accent_ink)
            .nowrap(),
        ),
        None => disc.child(icon(Icon::User, size * 0.4, theme.accent_ink)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Reader {
    Off,
    Scanning,
    Reading,
    Refused,
}

/// A ring around the avatar while the reader listens: it orbits waiting,
/// runs faster while a finger is read, and closes red on a mismatch.
fn fingerprint_ring(screen: &Screen) -> Container {
    let theme = screen.theme;
    let state = screen.auth.state;
    let reader = create_memo(move || match state.fingerprint.get() {
        FingerprintState::Waiting => Reader::Scanning,
        FingerprintState::Reading => Reader::Reading,
        FingerprintState::Retry(_) => Reader::Refused,
        _ => Reader::Off,
    });

    container()
        .width(96.0)
        .height(96.0)
        .layout(ZStack::new())
        .child(
            arc()
                .size(96.0)
                .stroke(3.0)
                .track(Color::WHITE.with_alpha(0.07)),
        )
        // Rebuilt when the speed changes, so the endless orbit stops with it.
        .child(move || {
            let (share, color, lap_ms) = match reader.get() {
                Reader::Off => (0.0, theme.accent, None),
                Reader::Scanning => (0.2, theme.accent, Some(1600.0)),
                Reader::Reading => (0.6, theme.accent, Some(800.0)),
                Reader::Refused => (1.0, theme.danger, None),
            };
            let ring = arc()
                .size(96.0)
                .stroke(3.0)
                .caps(Cap::Round)
                .color(color.transition(300.0))
                .progress(
                    share
                        .transition(Transition::spring(spring()))
                        .entering_from(0.0),
                );
            let ring = container().width(96.0).height(96.0).child(ring);
            match lap_ms {
                Some(ms) => ring.rotate(
                    0.0.timeline(
                        Keyframes::new(ms)
                            .at(0.0, 0.0)
                            .at(1.0, 360.0)
                            .repeat(Repeat::Forever),
                    ),
                ),
                None => ring,
            }
        })
}

/// The greeter's accounts, the chosen one larger.
fn users(screen: &Screen, armed: Signal<bool>) -> Container {
    let theme = screen.theme;
    let choice = screen.user_choice;
    let busy = screen.auth.state.busy;
    let mut entries: Vec<Option<UserInfo>> = screen.users.iter().cloned().map(Some).collect();
    if screen.config.greeter.other_user {
        entries.push(None);
    }

    let row = container().layout(
        Flex::row()
            .spacing(14.0)
            .cross_alignment(CrossAlignment::End),
    );
    row.children(entries.into_iter().enumerate().map(move |(index, user)| {
        let chosen = move || choice.get() == index;
        let name = user
            .as_ref()
            .map(|user| user.display_name.clone())
            .unwrap_or_else(|| "Other…".to_owned());
        let size = move || if chosen() { 72.0 } else { 44.0 };
        container()
            .layout(
                Flex::column()
                    .spacing(6.0)
                    .cross_alignment(CrossAlignment::Center),
            )
            .opacity((move || if chosen() { 1.0 } else { 0.5 }).transition(300.0))
            .on_click(move || {
                if armed.get_untracked() && !busy.get_untracked() {
                    choice.set(index);
                }
            })
            .child(
                container()
                    .width(size.transition(Transition::spring(spring())))
                    .height(size.transition(Transition::spring(spring())))
                    .layout(Flex::column().center())
                    .child({
                        let user = user.clone();
                        move || avatar(theme, user.as_ref(), size())
                    }),
            )
            .child(
                text(name)
                    .font_size(12.0)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .color(theme.text)
                    .nowrap(),
            )
    }))
}

/// One line under the field: checking, a lockout, a refusal, or the reader.
fn message(screen: &Screen) -> Container {
    let theme = screen.theme;
    let state = screen.auth.state;
    let screen = screen.clone();

    container()
        .height(18.0)
        .layout(Flex::row().center())
        .child(move || {
            let line = |icon_kind: Option<Icon>, words: String, color: Color| {
                container()
                    .layout(
                        Flex::row()
                            .spacing(6.0)
                            .cross_alignment(CrossAlignment::Center),
                    )
                    .child(icon_kind.map(|kind| icon(kind, 14.0, color)))
                    .child(text(words).font_size(12.0).color(color).nowrap())
            };
            let notice = state.notice.get().filter(|n| !n.text.is_empty());
            if state.busy.get() {
                return Some(line(None, "Checking…".to_owned(), theme.dim));
            }
            if state.locked.get().is_some() {
                return Some(lockout_chip(&screen, true));
            }
            if let Some(notice) = notice {
                let color = match notice.level {
                    NoticeLevel::Error => theme.danger,
                    NoticeLevel::Info => theme.dim,
                };
                return Some(line(None, notice.text, color));
            }
            let (words, color) = match state.fingerprint.get() {
                FingerprintState::Waiting => {
                    ("Or touch the fingerprint sensor".to_owned(), theme.dim)
                }
                FingerprintState::Reading => {
                    ("Checking your fingerprint…".to_owned(), theme.accent)
                }
                FingerprintState::Retry(hint) => (hint, theme.warning),
                FingerprintState::Exhausted => (
                    "Too many fingerprint attempts — use your password".to_owned(),
                    theme.dim,
                ),
                FingerprintState::Unavailable | FingerprintState::Resting => return None,
            };
            Some(line(Some(Icon::Finger), words, color))
        })
}

/// The buttons that open the drawer.
fn zones(screen: &Screen, armed: Signal<bool>) -> Container {
    let theme = screen.theme;
    let flow = screen.flow;
    let toggle = move |zone: Zone| {
        move || {
            if armed.get_untracked() {
                flow.zone.update(|open| {
                    *open = if *open == Some(zone) {
                        None
                    } else {
                        Some(zone)
                    }
                });
            }
        }
    };
    let pill = move |zone: Zone| {
        container()
            .height(30.0)
            .padding([0.0, 12.0])
            .corners(15.0)
            .background(
                (move || {
                    if flow.zone.get() == Some(zone) {
                        theme.surface2
                    } else {
                        theme.surface
                    }
                })
                .transition(150.0),
            )
            .when_hovered(|s| s.background(theme.surface2))
            .layout(
                Flex::row()
                    .spacing(7.0)
                    .cross_alignment(CrossAlignment::Center),
            )
            .on_click(toggle(zone))
    };

    let sessions = screen.sessions.clone();
    let session = screen.session;
    let session_zone = (screen.mode == Mode::Greeter && sessions.len() > 1).then(|| {
        pill(Zone::Session)
            .child(icon(Icon::Monitor, 13.0, theme.text))
            .child(move || {
                let name = sessions
                    .get(session.get())
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                text(name)
                    .font_size(12.0)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .color(theme.text)
                    .nowrap()
            })
            .child(icon(Icon::ChevronDown, 12.0, theme.sub))
    });

    container()
        .layout(
            Flex::row()
                .spacing(6.0)
                .main_alignment(MainAlignment::Center),
        )
        .child(session_zone)
        .child(pill(Zone::Power).child(icon(Icon::Power, 13.0, theme.text)))
}

/// What the open zone offers.
fn drawer(screen: &Screen, armed: Signal<bool>) -> Container {
    let theme = screen.theme;
    let flow = screen.flow;
    let sessions = screen.sessions.clone();
    let session = screen.session;
    let power = screen.power;

    let option = move |label: String, icon_kind: Option<Icon>, on: bool, danger: bool| {
        let ink = if on { theme.accent_ink } else { theme.sub };
        container()
            .height(30.0)
            .padding([0.0, 12.0])
            .corners(15.0)
            .background(if on { theme.accent } else { Color::TRANSPARENT })
            .when_hovered(move |s| {
                s.background(if on {
                    theme.accent
                } else if danger {
                    theme.danger.with_alpha(0.3)
                } else {
                    theme.surface2
                })
            })
            .layout(
                Flex::row()
                    .spacing(7.0)
                    .cross_alignment(CrossAlignment::Center),
            )
            .child(icon_kind.map(|kind| icon(kind, 13.0, ink)))
            .child(
                text(label)
                    .font_size(12.0)
                    .font_weight(if on {
                        FontWeight::SEMI_BOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .color(ink)
                    .nowrap(),
            )
    };

    container()
        .visible(move || flow.zone.get().is_some())
        .child(move || {
            let zone = flow.zone.get()?;
            let row = container()
                .layout(
                    Flex::row()
                        .spacing(4.0)
                        .main_alignment(MainAlignment::Center),
                )
                .opacity(1.0.transition(350.0).entering_from(0.0))
                .blur(0.0.transition(350.0).entering_from(3.0))
                .translate(
                    Translate::NONE
                        .transition(350.0)
                        .entering_from(Translate::new(0.0, -6.0)),
                );
            let choose = move |action: Box<dyn Fn()>| {
                move || {
                    if armed.get_untracked() {
                        action();
                        flow.zone.set(None);
                    }
                }
            };
            Some(match zone {
                Zone::Session => {
                    let chosen = session.get_untracked();
                    row.children(sessions.iter().enumerate().map(|(index, s)| {
                        option(s.name.clone(), None, index == chosen, false)
                            .on_click(choose(Box::new(move || session.set(index))))
                    }))
                }
                Zone::Power => row.children(PowerAction::ALL.into_iter().map(|action| {
                    let icon_kind = match action {
                        PowerAction::Suspend => Icon::Moon,
                        PowerAction::Reboot => Icon::Refresh,
                        PowerAction::PowerOff => Icon::Power,
                    };
                    option(
                        action.label().to_owned(),
                        Some(icon_kind),
                        false,
                        action != PowerAction::Suspend,
                    )
                    .on_click(choose(Box::new(move || power.request(action))))
                })),
            })
        })
}

/// A line that runs out while the card's buttons are not armed yet.
fn guard(theme: Theme) -> Container {
    container()
        .width(fill())
        .height(fill())
        .padding(Padding {
            top: 0.0,
            right: 24.0,
            bottom: 6.0,
            left: 24.0,
        })
        .layout(Flex::column().main_alignment(MainAlignment::End))
        .child(
            container()
                .width(fill())
                .height(2.0)
                .corners(1.0)
                .background(theme.accent)
                .pivot(Pivot::LEFT)
                .scale(
                    Scale::new(0.0, 1.0).timeline(
                        Keyframes::new(super::flow::ARM_MS as f32)
                            .at(0.0, Scale::NONE)
                            .at(1.0, Scale::new(0.0, 1.0)),
                    ),
                )
                .opacity(
                    0.0.timeline(
                        Keyframes::new(super::flow::ARM_MS as f32)
                            .at(0.0, 0.85)
                            .at(1.0, 0.0),
                    ),
                ),
        )
}

/// The battery as a ring, coloured by how much is left.
pub fn battery_ring(screen: &Screen, size: f32) -> Container {
    let theme = screen.theme;
    let battery = screen.battery;
    container().child(move || {
        let battery = battery.get()?;
        let color = if battery.charging {
            theme.success
        } else if battery.percent <= 15 {
            theme.danger
        } else if battery.percent <= 30 {
            theme.warning
        } else {
            theme.text
        };
        Some(
            arc()
                .size(size)
                .stroke(size * 0.145)
                .track(Color::WHITE.with_alpha(0.16))
                .caps(Cap::Round)
                .color(color.transition(300.0))
                .progress((f32::from(battery.percent) / 100.0).transition(1000.0)),
        )
    })
}

/// A small warning pill, shown while caps lock is on.
pub fn caps_chip(screen: &Screen, words: &'static str) -> Container {
    let theme = screen.theme;
    // Hidden rather than empty, so it takes no spacing in a row or a column.
    container()
        .visible(move || keyboard_modifiers().get().caps_lock)
        .child(move || {
            keyboard_modifiers().get().caps_lock.then(|| {
                chip(Icon::Caps, theme.warning).child(chip_text(words.to_owned(), theme.warning))
            })
        })
}

/// The `pam_faillock` lock, counting down. A warning, never a gate: the field
/// stays open and PAM decides.
pub fn lockout_chip(screen: &Screen, long: bool) -> Container {
    let theme = screen.theme;
    let state = screen.auth.state;
    let now = screen.now;
    let left = create_memo(move || match state.locked.get() {
        Some(Lock::Until(until)) => {
            let _tick = now.get();
            let secs = until
                .duration_since(SystemTime::now())
                .unwrap_or_default()
                .as_secs();
            format!("{}:{:02}", secs / 60, secs % 60)
        }
        _ => String::new(),
    });
    container()
        .visible(move || state.locked.get().is_some())
        .child(move || {
            let lock = state.locked.get()?;
            let row = if long {
                container()
                    .layout(
                        Flex::row()
                            .spacing(8.0)
                            .cross_alignment(CrossAlignment::Center),
                    )
                    .child(icon(Icon::Lock, 14.0, theme.warning))
            } else {
                chip(Icon::Lock, theme.warning)
            };
            let (size, weight) = if long {
                (12.0, FontWeight::SEMI_BOLD)
            } else {
                (11.5, FontWeight::SEMI_BOLD)
            };
            Some(match lock {
                Lock::UntilReset => {
                    row.child(chip_text("Account locked".to_owned(), theme.warning))
                }
                Lock::Until(_) => row
                    .child(long.then(|| chip_text("Locked for".to_owned(), theme.warning)))
                    .child(roll::view(
                        left,
                        roll::Look {
                            size,
                            line: 16.0,
                            weight,
                            spacing: 0.0,
                        },
                        move || theme.warning,
                    )),
            })
        })
}

fn chip(kind: Icon, color: Color) -> Container {
    container()
        .height(22.0)
        .padding([0.0, 8.0])
        .corners(11.0)
        .background(color.with_alpha(0.2))
        .layout(
            Flex::row()
                .spacing(5.0)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(icon(kind, 11.0, color))
}

fn chip_text(words: String, color: Color) -> Text {
    text(words)
        .font_size(11.5)
        .font_weight(FontWeight::SEMI_BOLD)
        .color(color)
        .nowrap()
}
