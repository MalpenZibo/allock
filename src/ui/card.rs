//! The card the island opens into: who is signing in, the field, what the
//! auth stack has to say, and a drawer for the session and the power menu.

use std::time::SystemTime;

use guido::prelude::*;

use super::Screen;
use super::field;
use super::flow::Zone;
use crate::Mode;
use crate::services::auth::{FingerprintState, Lock, NoticeLevel};
use crate::services::power::PowerAction;
use crate::system::user::UserInfo;
use ashell_ui::Theme;
use ashell_ui::button::{Content, chip_button};
use ashell_ui::list;
use ashell_ui::motion::{self, armed};
use ashell_ui::progress::ring;
use ashell_ui::roll;
use ashell_ui::{Icon, icon};

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

/// "Locked", or the machine in the greeter, and the battery. No time: the
/// big clock above says it.
fn top(screen: &Screen) -> Container {
    let theme = screen.theme;
    let what = match screen.mode {
        Mode::Greeter => screen.host.to_string(),
        Mode::Lock | Mode::Preview => "Locked".to_owned(),
    };
    let small = move |words: String| {
        text(words)
            .font_size(11.5)
            .line_height(LineHeight::Absolute(16.0))
            .font_weight(FontWeight::SEMI_BOLD)
            .color(theme.colors.dim)
            .nowrap()
    };
    container()
        .width(fill())
        .layout(
            Flex::row()
                .main_alignment(MainAlignment::SpaceBetween)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(container().child(small(what)))
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
                .layout(ZStack::new().align(Alignment::CENTER))
                .child(fingerprint_ring(screen))
                .child(move || avatar(theme, user.get().as_ref(), 76.0)),
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
                        .color(theme.colors.text)
                        .nowrap(),
                )
                .child(
                    text(account)
                        .font_size(11.5)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.colors.dim)
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
            theme.colors.accent,
            theme.colors.danger,
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
            .color(theme.colors.accent_ink)
            .nowrap(),
        ),
        None => disc.child(icon(Icon::User, size * 0.4, theme.colors.accent_ink)),
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
                Reader::Off => (0.0, theme.colors.accent, None),
                Reader::Scanning => (0.2, theme.colors.accent, Some(1600.0)),
                Reader::Reading => (0.6, theme.colors.accent, Some(800.0)),
                Reader::Refused => (1.0, theme.colors.danger, None),
            };
            let ring = arc()
                .size(96.0)
                .stroke(3.0)
                .caps(Cap::Round)
                .color(color.transition(300.0))
                .progress(share.transition(motion::spring()).entering_from(0.0));
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
                    .width(size.transition(motion::spring()))
                    .height(size.transition(motion::spring()))
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
                    .color(theme.colors.text)
                    .nowrap(),
            )
    }))
}

/// One line under the field: checking, a lockout, a refusal, or the reader.
#[derive(Debug, Clone, PartialEq)]
enum Line {
    Checking,
    Locked,
    /// Just refused: why, and how many tries are used.
    Refused {
        text: String,
        caps: bool,
    },
    /// The next refusal locks the account.
    LastTry(Option<std::time::Duration>),
    Notice(String),
    Reader(FingerprintState),
}

fn message(screen: &Screen) -> Container {
    let theme = screen.theme;
    let state = screen.auth.state;
    let flow = screen.flow;
    let screen = screen.clone();

    // Rebuilt when what is said changes, not on every key.
    let said = create_memo(move || {
        if state.busy.get() {
            return Some(Line::Checking);
        }
        if state.locked.get().is_some() {
            return Some(Line::Locked);
        }
        let tally = state.tally.get();
        match state.notice.get().filter(|n| !n.text.is_empty()) {
            // A refusal is said until the next answer is being typed.
            Some(notice) if notice.level == NoticeLevel::Error && state.field_is_empty() => {
                return Some(Line::Refused {
                    text: notice.text,
                    caps: flow.caps_at_refusal.get(),
                });
            }
            Some(notice) if notice.level == NoticeLevel::Info => {
                return Some(Line::Notice(notice.text));
            }
            _ => {}
        }
        if let Some(tally) = tally.filter(|t| t.failed > 0 && t.left() == 1) {
            return Some(Line::LastTry(tally.lockout));
        }
        match state.fingerprint.get() {
            FingerprintState::Unavailable | FingerprintState::Resting => None,
            reader => Some(Line::Reader(reader)),
        }
    });

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
            Some(match said.get()? {
                Line::Checking => line(None, "Checking…".to_owned(), theme.colors.dim),
                Line::Locked => lockout_chip(&screen, true),
                Line::Refused { text, caps } => {
                    let words = if caps {
                        format!("{text} — Caps Lock is on")
                    } else {
                        text
                    };
                    line(None, words, theme.colors.danger)
                        .layout(
                            Flex::row()
                                .spacing(8.0)
                                .cross_alignment(CrossAlignment::Center),
                        )
                        .child(attempts(&screen))
                }
                Line::LastTry(lockout) => {
                    let then = match lockout {
                        Some(lockout) => {
                            format!("then a {}-minute lockout", lockout.as_secs().div_ceil(60))
                        }
                        None => "then the account locks".to_owned(),
                    };
                    line(
                        None,
                        format!("1 attempt left, {then}"),
                        theme.colors.warning,
                    )
                }
                Line::Notice(text) => line(None, text, theme.colors.dim),
                Line::Reader(reader) => {
                    let (words, color) = match reader {
                        FingerprintState::Waiting => (
                            "Or touch the fingerprint sensor".to_owned(),
                            theme.colors.dim,
                        ),
                        FingerprintState::Reading => {
                            ("Checking your fingerprint…".to_owned(), theme.colors.accent)
                        }
                        FingerprintState::Retry(hint) => (hint, theme.colors.warning),
                        _ => (
                            "Too many fingerprint attempts — use your password".to_owned(),
                            theme.colors.dim,
                        ),
                    };
                    line(Some(Icon::Fingerprint), words, color)
                }
            })
        })
}

/// A dot per attempt `pam_faillock` allows, the used ones red.
fn attempts(screen: &Screen) -> Container {
    let theme = screen.theme;
    let tally = screen.auth.state.tally;
    container()
        .layout(Flex::row().spacing(4.0))
        .children(move || {
            let Some(tally) = tally.get() else {
                return Vec::new();
            };
            (0..tally.deny)
                .map(|n| {
                    let used = n < tally.failed;
                    container()
                        .width(6.0)
                        .height(6.0)
                        .corners(3.0)
                        .background(if used {
                            theme.colors.danger
                        } else {
                            Color::WHITE.with_alpha(0.18)
                        })
                        // The one just used fills in.
                        .scale(Scale::NONE.transition(motion::spring()).entering_from(
                            Scale::uniform(if used && n + 1 == tally.failed {
                                0.2
                            } else {
                                1.0
                            }),
                        ))
                })
                .collect::<Vec<_>>()
        })
}

/// The chips that open the drawer.
fn zones(screen: &Screen, armed: Signal<bool>) -> Container {
    let flow = screen.flow;
    let zone_chip = move |content: Content, zone: Zone| {
        chip_button(content, move || flow.zone.get() == Some(zone)).on_click(move || {
            if armed.get_untracked() {
                flow.zone.update(|open| {
                    *open = if *open == Some(zone) {
                        None
                    } else {
                        Some(zone)
                    }
                });
            }
        })
    };

    let sessions = screen.sessions.clone();
    let session = screen.session;
    let session_zone = (screen.mode == Mode::Greeter && sessions.len() > 1).then(|| {
        let name = create_memo(move || {
            sessions
                .get(session.get())
                .map(|s| s.name.clone())
                .unwrap_or_default()
        });
        zone_chip(
            Content::IconLabel(Icon::Monitor, name.into_signal()),
            Zone::Session,
        )
    });

    container()
        .layout(
            Flex::row()
                .spacing(6.0)
                .main_alignment(MainAlignment::Center),
        )
        .child(session_zone)
        .child(zone_chip(Icon::Power.into(), Zone::Power))
}

/// What the open zone offers.
fn drawer(screen: &Screen, armed: Signal<bool>) -> Container {
    let flow = screen.flow;
    let sessions = screen.sessions.clone();
    let session = screen.session;
    let power = screen.power;
    let theme = screen.theme;

    container()
        .width(fill())
        .visible(move || flow.zone.get().is_some())
        .child(move || {
            let zone = flow.zone.get()?;
            let enter = |c: Container| {
                c.opacity(1.0.transition(350.0).entering_from(0.0))
                    .blur(0.0.transition(350.0).entering_from(3.0))
                    .translate(
                        Translate::NONE
                            .transition(350.0)
                            .entering_from(Translate::new(0.0, -6.0)),
                    )
            };
            let choose = move |action: Box<dyn Fn()>| {
                move || {
                    if armed.get_untracked() {
                        action();
                        flow.zone.set(None);
                    }
                }
            };
            Some(match zone {
                // A menu, one session a line: their names are too long, and
                // too many, for a row of chips.
                Zone::Session => {
                    let chosen = session.get_untracked();
                    enter(
                        container()
                            .width(fill())
                            .layout(Flex::column().spacing(2.0)),
                    )
                    .children(sessions.iter().enumerate().map(|(index, s)| {
                        let selected = index == chosen;
                        let mut line = list::row(s.name.clone())
                            .icon(Icon::Monitor)
                            .selected(selected);
                        if selected {
                            line = line.trailing(icon(Icon::Check, 16.0, theme.colors.accent));
                        }
                        line.build()
                            .on_click(choose(Box::new(move || session.set(index))))
                    }))
                }
                Zone::Power => enter(
                    container().layout(
                        Flex::row()
                            .spacing(4.0)
                            .main_alignment(MainAlignment::Center),
                    ),
                )
                .children(PowerAction::ALL.into_iter().map(|action| {
                    let kind = match action {
                        PowerAction::Suspend => Icon::Moon,
                        PowerAction::Reboot => Icon::Refresh,
                        PowerAction::PowerOff => Icon::Power,
                    };
                    chip_button((kind, action.label()), false)
                        .on_click(choose(Box::new(move || power.request(action))))
                })),
            })
        })
}

/// A line that runs out while the card's buttons are not armed yet.
fn guard(theme: Theme) -> Container {
    container()
        .width(fill())
        .padding(Padding {
            top: 0.0,
            right: 24.0,
            bottom: 6.0,
            left: 24.0,
        })
        .align_self(Alignment::BOTTOM_CENTER)
        .child(
            container()
                .width(fill())
                .height(2.0)
                .corners(1.0)
                .background(theme.colors.accent)
                .pivot(Pivot::LEFT)
                .scale(
                    Scale::new(0.0, 1.0).timeline(
                        Keyframes::new(motion::ARM_MS as f32)
                            .at(0.0, Scale::NONE)
                            .at(1.0, Scale::new(0.0, 1.0)),
                    ),
                )
                .opacity(
                    0.0.timeline(
                        Keyframes::new(motion::ARM_MS as f32)
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
            theme.colors.success
        } else if battery.percent <= 15 {
            theme.colors.danger
        } else if battery.percent <= 30 {
            theme.colors.warning
        } else {
            theme.colors.text
        };
        Some(ring(
            f32::from(battery.percent) / 100.0,
            color,
            size,
            size * 0.145,
        ))
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
                chip(Icon::CapsLock, theme.colors.warning)
                    .child(chip_text(words.to_owned(), theme.colors.warning))
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
                    .child(icon(Icon::Lock, 14.0, theme.colors.warning))
            } else {
                chip(Icon::Lock, theme.colors.warning)
            };
            let (size, weight) = if long {
                (12.0, FontWeight::SEMI_BOLD)
            } else {
                (11.5, FontWeight::SEMI_BOLD)
            };
            Some(match lock {
                Lock::UntilReset => {
                    row.child(chip_text("Account locked".to_owned(), theme.colors.warning))
                }
                Lock::Until(_) => row
                    .child(long.then(|| chip_text("Locked for".to_owned(), theme.colors.warning)))
                    .child(roll::rolling(
                        left,
                        roll::Look {
                            size,
                            line: 16.0,
                            weight,
                            spacing: 0.0,
                        },
                        move || theme.colors.warning,
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
