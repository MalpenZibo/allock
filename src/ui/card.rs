//! The card the island opens into: who is signing in, the field, what the
//! auth stack has to say, and a drawer for the session and the power menu.

use std::time::{Duration, SystemTime};

use guido::prelude::*;

use super::Screen;
use super::field;
use super::flow::{Flow, Zone};
use crate::Mode;
use crate::services::auth::{FingerprintState, Lock, NoticeLevel, Tally};
use crate::services::battery::Battery;
use crate::services::power::PowerAction;
use crate::system::user::UserInfo;
use styletto::components::badge::dot;
use styletto::components::button::chip_button;
use styletto::components::chip::{Chip, chip};
use styletto::components::list;
use styletto::components::progress::{battery_ink, ring};
use styletto::components::roll;
use styletto::foundations::control::Size;
use styletto::foundations::motion::{self, armed};
use styletto::foundations::tokens::{IconSize, icon as icon_box, radius, size, stroke};
use styletto::foundations::typography as ty;
use styletto::primitives::text::{caption, heading, typed};
use styletto::{Icon, centered_row, icon, ui};

const WIDTH: f32 = 400.0;
/// The avatar, and how far the fingerprint ring stands off it.
const AVATAR: f32 = 80.0;
const RING_GAP: f32 = 8.0;

pub fn view(screen: &Screen) -> Container {
    let armed = armed();

    let body = container()
        .width(WIDTH)
        .padding([size::S6.px(), size::S6.px(), size::S4.px(), size::S6.px()])
        .layout(
            Flex::column()
                .spacing(size::S3.px())
                .cross_alignment(CrossAlignment::Center),
        )
        .child(top(screen))
        .child(match screen.mode {
            Mode::Greeter if !screen.users.is_empty() => users(screen, armed),
            _ => identity(screen),
        })
        .child(field::view(screen))
        .child(caps_chip("Caps Lock is on"))
        .child(message(screen))
        .child(zones(screen, armed))
        .child(drawer(screen, armed));

    container().layout(ZStack::new()).child(body).child(guard())
}

/// "Locked", or the machine in the greeter, and the battery. No time: the
/// big clock above says it.
fn top(screen: &Screen) -> Container {
    let t = ui();
    let what = match screen.mode {
        Mode::Greeter => screen.host.to_string(),
        Mode::Lock | Mode::Preview => "Locked".to_owned(),
    };
    let small = move |words: String| {
        typed(words, ty::LABEL_SMALL.em())
            .color(t.colors.dim)
            .nowrap()
    };
    container()
        .width(fill())
        .layout(centered_row(0.0).main_alignment(MainAlignment::SpaceBetween))
        .child(container().child(small(what)))
        .child({
            let battery = screen.battery;
            container()
                .layout(centered_row(size::S2.px()))
                .child(gauge(battery, icon_box::SMALL))
                .child(move || {
                    battery
                        .get()
                        .map(|battery| container().child(small(format!("{}%", battery.percent))))
                })
        })
}

/// The avatar in the fingerprint ring, the name, the account.
fn identity(screen: &Screen) -> Container {
    let t = ui();
    let user = screen.user;
    let host = screen.host.clone();

    container()
        .layout(
            Flex::column()
                .spacing(size::S2.px())
                .cross_alignment(CrossAlignment::Center),
        )
        .child(
            container()
                .layout(ZStack::new().align(Alignment::CENTER))
                .child(fingerprint_ring(screen))
                .child(avatar(user)),
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
                        .spacing(size::S0_5.px())
                        .cross_alignment(CrossAlignment::Center),
                )
                .child(heading(name))
                .child(caption(account).color(t.colors.dim))
        })
}

/// The picture, or the initial on the accent.
#[component]
fn avatar(
    #[prop(required)] user: Option<UserInfo>,
    #[prop(default = AVATAR)] size: f32,
) -> impl Widget {
    let t = ui();
    container()
        .width(size)
        .height(size)
        .corners(move || radius::pill(size.get()))
        .overflow(Overflow::Hidden)
        .gradient(LinearGradient::new(
            t.colors.accent,
            t.colors.danger,
            GradientDirection::Diagonal,
        ))
        .layout(Flex::column().center())
        // Tracks the size only where it is drawn at it, so a picture is not
        // rebuilt (and decoded again) as the greeter's picker resizes it.
        .child(move || {
            let mark = move || size.get() * 0.4;
            match user.get() {
                Some(UserInfo {
                    avatar: Some(path), ..
                }) => image(ImageSource::Path(path))
                    .content_fit(ContentFit::Cover)
                    .into_any(),
                Some(user) => text(
                    user.display_name
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default(),
                )
                .font_family(t.fonts.sans)
                .font_size(mark)
                .font_weight(FontWeight::BOLD)
                .color(t.colors.on_accent)
                .nowrap()
                .into_any(),
                None => icon(Icon::User, mark(), t.colors.on_accent).into_any(),
            }
        })
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
    let t = ui();
    let side = AVATAR + 2.0 * RING_GAP;
    let state = screen.auth.state;
    let reader = create_memo(move || match state.fingerprint.get() {
        FingerprintState::Waiting => Reader::Scanning,
        FingerprintState::Reading => Reader::Reading,
        FingerprintState::Retry(_) => Reader::Refused,
        _ => Reader::Off,
    });

    container()
        .width(side)
        .height(side)
        .layout(ZStack::new())
        .child(
            arc()
                .size(side)
                .stroke(stroke::HEAVY)
                .track(t.colors.surface),
        )
        // Rebuilt when the speed changes, so the endless orbit stops with it.
        .child(move || {
            let (share, color, lap_ms) = match reader.get() {
                Reader::Off => (0.0, t.colors.accent, None),
                Reader::Scanning => (0.2, t.colors.accent, Some(1600.0)),
                Reader::Reading => (0.6, t.colors.accent, Some(800.0)),
                Reader::Refused => (1.0, t.colors.danger, None),
            };
            let ring = arc()
                .size(side)
                .stroke(stroke::HEAVY)
                .caps(Cap::Round)
                .color(color.transition(motion::color()))
                .progress(share.transition(motion::spring()).entering_from(0.0));
            let ring = container().width(side).height(side).child(ring);
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
    let t = ui();
    let choice = screen.user_choice;
    let busy = screen.auth.state.busy;
    let mut entries: Vec<Option<UserInfo>> = screen.users.iter().cloned().map(Some).collect();
    if screen.config.greeter.other_user {
        entries.push(None);
    }

    let row = container().layout(
        Flex::row()
            .spacing(size::S4.px())
            .cross_alignment(CrossAlignment::End),
    );
    row.children(entries.into_iter().enumerate().map(move |(index, user)| {
        let chosen = move || choice.get() == index;
        let name = user
            .as_ref()
            .map(|user| user.display_name.clone())
            .unwrap_or_else(|| "Other…".to_owned());
        let side = move || if chosen() { AVATAR } else { size::S10.px() };
        container()
            .layout(
                Flex::column()
                    .spacing(size::S2.px())
                    .cross_alignment(CrossAlignment::Center),
            )
            .opacity((move || if chosen() { 1.0 } else { 0.5 }).transition(motion::color()))
            .on_click(move || {
                if armed.get_untracked() && !busy.get_untracked() {
                    choice.set(index);
                }
            })
            .child(
                container()
                    .width(side.transition(motion::spring()))
                    .height(side.transition(motion::spring()))
                    .layout(Flex::column().center())
                    .child(avatar(user.clone()).size(side)),
            )
            .child(
                typed(name, ty::LABEL_SMALL.em())
                    .color(t.colors.text)
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
    let t = ui();
    let state = screen.auth.state;
    let flow = screen.flow;

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
            Some(notice) if state.refused() => {
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
        .height(icon_box::SMALL.px())
        .layout(Flex::row().center())
        .child(move || {
            let line = |icon_kind: Option<Icon>, words: String, color: Color| {
                container()
                    .layout(centered_row(size::S2.px()))
                    .child(icon_kind.map(|kind| icon(kind, icon_box::SMALL.px(), color)))
                    .child(typed(words, ty::BODY_SMALL).color(color).nowrap())
            };
            Some(match said.get()? {
                Line::Checking => line(None, "Checking…".to_owned(), t.colors.dim),
                Line::Locked => container().child(lockout_chip(state.locked).long(true)),
                Line::Refused { text, caps } => {
                    let words = if caps {
                        format!("{text} — Caps Lock is on")
                    } else {
                        text
                    };
                    line(None, words, t.colors.danger).child(attempts(state.tally))
                }
                Line::LastTry(lockout) => {
                    let then = match lockout {
                        Some(lockout) => {
                            format!("then a {}-minute lockout", lockout.as_secs().div_ceil(60))
                        }
                        None => "then the account locks".to_owned(),
                    };
                    line(None, format!("1 attempt left, {then}"), t.colors.warning)
                }
                Line::Notice(text) => line(None, text, t.colors.dim),
                Line::Reader(reader) => {
                    let (words, color) = match reader {
                        FingerprintState::Waiting => {
                            ("Or touch the fingerprint sensor".to_owned(), t.colors.dim)
                        }
                        FingerprintState::Reading => {
                            ("Checking your fingerprint…".to_owned(), t.colors.accent)
                        }
                        FingerprintState::Retry(hint) => (hint, t.colors.warning),
                        _ => (
                            "Too many fingerprint attempts — use your password".to_owned(),
                            t.colors.dim,
                        ),
                    };
                    line(Some(Icon::Fingerprint), words, color)
                }
            })
        })
}

/// A dot per attempt `pam_faillock` allows, the used ones red.
fn attempts(tally: RwSignal<Option<Tally>>) -> Container {
    let t = ui();
    container()
        .layout(Flex::row().spacing(size::S1.px()))
        .children(move || {
            let Some(tally) = tally.get() else {
                return Vec::new();
            };
            (0..tally.deny)
                .map(|n| {
                    let used = n < tally.failed;
                    container()
                        .child(dot(if used {
                            t.colors.danger
                        } else {
                            t.colors.surface2
                        }))
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
    let zone_chip = move |label: Signal<String>, kind: Icon, zone: Zone| {
        chip_button(label)
            .icon(Some(kind))
            .active(move || flow.zone.get() == Some(zone))
            .on_click(move || {
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
        zone_chip(name.into_signal(), Icon::Monitor, Zone::Session)
    });

    container()
        .layout(centered_row(size::S2.px()).main_alignment(MainAlignment::Center))
        .child(session_zone)
        // No label: a circle.
        .child(zone_chip(
            String::new().into_signal(),
            Icon::Power,
            Zone::Power,
        ))
}

/// What the open zone offers.
fn drawer(screen: &Screen, armed: Signal<bool>) -> Container {
    let flow = screen.flow;
    let sessions = screen.sessions.clone();
    let session = screen.session;
    let power = screen.power;
    let t = ui();

    container()
        .width(fill())
        .visible(move || flow.zone.get().is_some())
        .child(move || {
            let zone = flow.zone.get()?;
            Some(motion::entering(match zone {
                // A menu, one session a line: their names are too long, and
                // too many, for a row of chips.
                Zone::Session => {
                    let chosen = session.get_untracked();
                    container()
                        .width(fill())
                        .layout(Flex::column().spacing(size::S0_5.px()))
                        .children(sessions.iter().enumerate().map(|(index, s)| {
                            let selected = index == chosen;
                            let mut line = list::row(s.name.clone())
                                .icon(Some(Icon::Monitor))
                                .selected(selected)
                                .on_click(choose(armed, flow, move || session.set(index)));
                            if selected {
                                line = line.trailing(icon(
                                    Icon::Check,
                                    icon_box::SMALL.px(),
                                    t.colors.accent,
                                ));
                            }
                            line
                        }))
                }
                Zone::Power => container()
                    .layout(centered_row(size::S1.px()).main_alignment(MainAlignment::Center))
                    .children(PowerAction::ALL.into_iter().map(|action| {
                        let kind = match action {
                            PowerAction::Suspend => Icon::Moon,
                            PowerAction::Reboot => Icon::Refresh,
                            PowerAction::PowerOff => Icon::Power,
                        };
                        chip_button(action.label().to_owned())
                            .icon(Some(kind))
                            .on_click(choose(armed, flow, move || power.request(action)))
                    })),
            }))
        })
}

/// A drawer entry's press, once the card is armed: `action`, then the
/// drawer closes.
fn choose(armed: Signal<bool>, flow: Flow, action: impl Fn() + 'static) -> impl Fn() + 'static {
    move || {
        if armed.get_untracked() {
            action();
            flow.zone.set(None);
        }
    }
}

/// A line that runs out while the card's buttons are not armed yet.
fn guard() -> Container {
    let t = ui();
    container()
        .width(fill())
        .padding([0.0, size::S6.px(), size::S1.px(), size::S6.px()])
        .align_self(Alignment::BOTTOM_CENTER)
        .child(
            container()
                .width(fill())
                .height(stroke::BOLD)
                .corners(radius::pill(stroke::BOLD))
                .background(t.colors.accent)
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

/// The battery as a ring, while there is one, coloured by how much is
/// left: green on the charger, whatever the battery reports doing.
pub fn gauge(battery: Signal<Option<Battery>>, size: IconSize) -> Container {
    let t = ui();
    container().child(move || {
        let battery = battery.get()?;
        let percent = f32::from(battery.percent);
        Some(
            ring(percent / 100.0)
                .color(battery_ink(&t.colors, percent, battery.plugged, false))
                .size(size),
        )
    })
}

/// A small warning pill, shown while caps lock is on.
#[component]
pub fn caps_chip(#[prop(required)] words: String) -> impl Widget {
    let t = ui();
    // Hidden rather than empty, so it takes no spacing in a row or a column.
    container()
        .visible(move || keyboard_modifiers().get().caps_lock)
        .child(warning(Icon::CapsLock).child(chip_text(words, t.colors.warning)))
}

/// The `pam_faillock` lock, counting down. A warning, never a gate: the field
/// stays open and PAM decides. `long` says what it
/// is, where there is room.
#[component]
pub fn lockout_chip(#[prop(required)] locked: Option<Lock>, long: bool) -> impl Widget {
    let t = ui();
    let long = long.get_untracked();
    // Bumped each second while a lock counts down, and only then.
    let tick = create_signal(0u32);
    let left = create_memo(move || match locked.get() {
        Some(Lock::Until(until)) => {
            let _tick = tick.get();
            let secs = until
                .duration_since(SystemTime::now())
                .unwrap_or_default()
                .as_secs();
            format!("{}:{:02}", secs / 60, secs % 60)
        }
        _ => String::new(),
    });
    // The card's line under the field, or the chip's.
    let kind = if long { ty::BODY_SMALL } else { CHIP.label() };
    let words = move |words: &str| {
        typed(words.to_owned(), kind.em())
            .color(t.colors.warning)
            .nowrap()
    };
    container()
        .visible(move || locked.get().is_some())
        .child(move || {
            let lock = locked.get()?;
            if let Lock::Until(_) = lock {
                let timer = set_interval(Duration::from_secs(1), move || tick.update(|n| *n += 1));
                on_cleanup(move || timer.cancel());
            }
            let what = match lock {
                Lock::UntilReset => words("Account locked").into_any(),
                Lock::Until(_) => container()
                    .layout(centered_row(size::S1.px()))
                    .child(long.then(|| words("Locked for")))
                    .child(roll::rolling(left).look(kind.em()).color(t.colors.warning))
                    .into_any(),
            };
            Some(if long {
                container()
                    .layout(centered_row(size::S2.px()))
                    .child(icon(Icon::Lock, icon_box::SMALL.px(), t.colors.warning))
                    .child(what)
                    .into_any()
            } else {
                warning(Icon::Lock).child(what).into_any()
            })
        })
}

/// The warning pills' size: the island's at rest.
const CHIP: Size = Size::Xxs;

/// A pill tinted with the warning colour, led by `kind`.
fn warning(kind: Icon) -> Chip {
    let t = ui();
    chip()
        .size(CHIP)
        .fill(t.colors.warning.with_alpha(0.2))
        .child(icon(kind, CHIP.icon(), t.colors.warning))
}

fn chip_text<M>(words: impl IntoSignal<String, M>, color: Color) -> Text {
    typed(words, CHIP.label().em()).color(color).nowrap()
}
