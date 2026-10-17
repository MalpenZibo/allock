//! The password field.
//!
//! The keyboard goes to one input that never leaves the screen, so a key
//! pressed at rest lands in it and wakes the island, and the card that then
//! appears shows what it holds: the first key typed is the first character
//! of the password. The field on the card only draws.

use guido::prelude::*;

use super::Screen;
use super::flow::Phase;
use super::icons::{Icon, icon};
use crate::Mode;

/// The input the keyboard types into, kept out of sight.
pub fn keyboard(screen: &Screen) -> Container {
    let auth = screen.auth;
    let state = auth.state;
    let flow = screen.flow;

    // Asked about the account in the greeter's "Other…", then the password.
    let secret = create_memo(move || state.asks_secret());
    // Nothing is typed while an answer is checked or after it was accepted.
    let held = move || state.busy.get() || !matches!(flow.phase.get(), Phase::Rest | Phase::Awake);

    // A key typed at rest wakes the island.
    create_effect(move || {
        if !state.field_is_empty() && flow.phase.get_untracked() == Phase::Rest {
            flow.wake();
        }
    });

    // The input is the screen's only focusable widget: a stray click or a
    // keyboard that left and came back would otherwise leave keys going nowhere.
    let input_ref = create_widget_ref();
    create_effect(move || {
        if guido::reactive::focus::focus_path().widget().is_none() {
            input_ref.focus();
        }
    });

    let submit = move || {
        if flow.phase.get_untracked() == Phase::Rest {
            flow.wake();
        } else {
            auth.submit();
        }
    };

    container()
        .width(1.0)
        .height(1.0)
        .overflow(Overflow::Hidden)
        .opacity(0.0)
        .child(move || {
            if secret.get() {
                container().child(
                    password_input(state.password)
                        .readonly(held)
                        .no_caret()
                        .widget_ref(input_ref)
                        .autofocus()
                        .on_submit(submit),
                )
            } else {
                container().child(
                    text_input(state.input)
                        .readonly(held)
                        .no_caret()
                        .widget_ref(input_ref)
                        .autofocus()
                        .on_submit(move |_| submit()),
                )
            }
        })
}

/// What the keyboard typed, drawn: dots, the password itself when revealed,
/// or the account name.
pub fn view(screen: &Screen) -> Container {
    let theme = screen.theme;
    let auth = screen.auth;
    let state = auth.state;
    let flow = screen.flow;
    let user = screen.user;
    let greeter = screen.mode == Mode::Greeter;

    let secret = create_memo(move || state.asks_secret());
    let shown = create_memo(move || {
        if !secret.get() {
            return state.input.get();
        }
        if flow.reveal.get() {
            state.password.with(|password| password.expose().to_owned())
        } else {
            let typed = state
                .password
                .with(|password| password.expose().chars().count());
            "•".repeat(typed)
        }
    });
    let placeholder = create_memo(move || match state.prompt.get() {
        // PAM sends "Password: " with the colon and padding.
        Some(prompt) if !prompt.secret || !greeter => prompt
            .message
            .trim()
            .trim_end_matches(':')
            .trim()
            .to_owned(),
        _ if greeter => match user.get() {
            Some(user) => format!("Password for {}", user.name),
            None => "Password".to_owned(),
        },
        _ => "Password".to_owned(),
    });
    let failed = create_memo(move || {
        state.field_is_empty()
            && matches!(state.notice.get(), Some(n) if n.level == crate::services::auth::NoticeLevel::Error)
    });

    let typed = move || {
        let shown = shown.get();
        let empty = shown.is_empty();
        let masked = secret.get() && !flow.reveal.get();
        text(if empty { placeholder.get() } else { shown })
            .font_size(15.0)
            .line_height(LineHeight::Absolute(20.0))
            .letter_spacing(if masked && !empty { 1.8 } else { 0.0 })
            .color(if empty { theme.dim } else { theme.text })
            .nowrap()
            .overflow(TextOverflow::Ellipsis)
    };

    container()
        .width(fill())
        .height(46.0)
        .corners(23.0)
        .background(theme.surface)
        .shadow(
            (move || {
                let ring = if failed.get() {
                    theme.danger
                } else {
                    theme.accent.with_alpha(0.7)
                };
                Shadow::new((0.0, 0.0), 0.0, 1.5, ring).inset()
            })
            .transition(250.0),
        )
        .opacity((move || if state.busy.get() { 0.45 } else { 1.0 }).transition(200.0))
        .padding(Padding {
            top: 0.0,
            right: 6.0,
            bottom: 0.0,
            left: 18.0,
        })
        .layout(
            Flex::row()
                .spacing(6.0)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(container().width(fill()).child(typed))
        .child(move || secret.get().then(|| reveal(theme, flow)))
        .child(go(screen))
}

fn reveal(theme: super::theme::Theme, flow: super::flow::Flow) -> Container {
    round_button(34.0)
        .when_hovered(|s| s.background(theme.surface2))
        .on_click(move || flow.reveal.update(|shown| *shown = !*shown))
        .child(move || {
            let kind = if flow.reveal.get() {
                Icon::EyeOff
            } else {
                Icon::Eye
            };
            icon(kind, 16.0, theme.sub)
        })
}

/// Submit. The only part of the field that takes a click: a stray click
/// submitting an empty password would count against `pam_faillock`.
fn go(screen: &Screen) -> Container {
    let theme = screen.theme;
    let auth = screen.auth;
    let state = auth.state;
    let ready = move || !state.field_is_empty();

    round_button(34.0)
        .background(
            (move || {
                if ready() || state.busy.get() {
                    theme.accent
                } else {
                    theme.surface2
                }
            })
            .transition(200.0),
        )
        .on_click(move || auth.submit())
        .child(move || {
            if state.busy.get() {
                icon(Icon::Refresh, 16.0, theme.accent_ink).rotate(0.0.timeline(spin()))
            } else {
                icon(Icon::Arrow, 16.0, move || {
                    if ready() { theme.accent_ink } else { theme.dim }
                })
            }
        })
}

fn round_button(size: f32) -> Container {
    container()
        .width(size)
        .height(size)
        .corners(size / 2.0)
        .background(Color::TRANSPARENT.transition(150.0))
        .layout(Flex::column().center())
}

pub fn spin() -> Keyframes<f32> {
    Keyframes::new(900.0)
        .at(0.0, 0.0)
        .at(1.0, 360.0)
        .repeat(Repeat::Forever)
}
