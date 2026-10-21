//! The password field.
//!
//! The keyboard goes to one input that never leaves the screen, so a key
//! pressed at rest lands in it and wakes the island, and the card that then
//! appears shows what it holds: the first key typed is the first character
//! of the password. The field on the card only draws.

use guido::prelude::*;

use super::Screen;
use super::flow::Phase;
use crate::Mode;
use ashell_ds::button::{Fill, icon_button};
use ashell_ds::control::Size;
use ashell_ds::input::password_field;
use ashell_ds::typography::{LABEL_LARGE, Typeset};
use ashell_ds::{Icon, ui};

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
pub fn view(screen: &Screen) -> impl Widget + use<> {
    let t = ui();
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
    let failed = create_memo(move || state.refused());

    // One text whose content, spacing and colour follow what is typed, not
    // a new one per key.
    let empty = create_memo(move || shown.with(String::is_empty));
    let typed = text(move || {
        if empty.get() {
            placeholder.get()
        } else {
            shown.get()
        }
    })
    .typeset(LABEL_LARGE)
    .letter_spacing(move || {
        let masked = secret.get() && !flow.reveal.get();
        if masked && !empty.get() { 1.8 } else { 0.0 }
    })
    .color(move || {
        if empty.get() {
            t.colors.dim
        } else {
            t.colors.text
        }
    })
    .nowrap()
    .overflow(TextOverflow::Ellipsis);

    // A stray click on submit sends nothing: `submit` declines an empty
    // answer, which would count against `pam_faillock`.
    password_field()
        .refused(failed)
        .busy(state.busy)
        .held(state.busy)
        .on_submit(move || auth.submit())
        .extra(container().child(move || secret.get().then(|| reveal(flow))))
        .child(container().width(fill()).child(typed))
}

fn reveal(flow: super::flow::Flow) -> impl Widget {
    let kind = move || {
        if flow.reveal.get() {
            Icon::EyeOff
        } else {
            Icon::Eye
        }
    };
    icon_button(kind)
        .size(Size::Sm)
        .fill(Fill::Ghost)
        .on_click(move || flow.reveal.update(|shown| *shown = !*shown))
}
