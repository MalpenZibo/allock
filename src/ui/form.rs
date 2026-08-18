//! The login card: who is logging in, the field, and why the last attempt
//! did not work.

use guido::animation::Keyframes;
use guido::prelude::*;

use super::Screen;
use crate::auth::{FingerprintState, NoticeLevel};
use crate::components::icons::{IconKind, icon};
use crate::theme::with_alpha;

// Codepoints verified against SymbolsNerdFont-Regular's cmap, not guessed:
// the neighbourhood is dense and wrong-but-present glyphs render silently.
/// nf-md-fingerprint
const FINGERPRINT_GLYPH: &str = "\u{f0237}";
/// nf-md-arrow_right_bold_circle_outline — the "submit" affordance.
const SUBMIT_GLYPH: &str = "\u{f0057}";
/// nf-md-account
const ACCOUNT_GLYPH: &str = "\u{f0004}";
/// nf-md-apple_keyboard_caps — the ⇪ every other password field uses.
const CAPS_LOCK_GLYPH: &str = "\u{f0632}";

/// Exact, not a minimum: under the centring column a minimum width grows to
/// fill the output, and the card grows with it into a full-width band.
const FIELD_WIDTH: f32 = 360.0;
const AVATAR_SIZE: f32 = 76.0;

/// The wobble a refused attempt gets.
///
/// Small angles and a decaying amplitude — the shake of a head saying no, not a
/// machine falling over. A third of a second: long enough to be seen, short
/// enough that a second wrong password can start its own before the first is
/// forgotten. Started again mid-run it restarts, which is what a second refusal
/// means.
fn shake() -> Keyframes<Transform> {
    Keyframes::new(320.0)
        .at(0.0, Transform::IDENTITY)
        .at(0.15, Transform::rotate_degrees(1.2))
        .at(0.40, Transform::rotate_degrees(-1.2))
        .at(0.65, Transform::rotate_degrees(0.9))
        .at(0.85, Transform::rotate_degrees(-0.45))
        .at(1.0, Transform::IDENTITY)
}

pub fn view(screen: Screen) -> Container {
    let content = container()
        .padding([28.0, 32.0])
        .layout(
            Flex::column()
                .spacing(14.0)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(avatar(&screen))
        .child(name(&screen))
        .child(field(&screen))
        .child(notice(&screen))
        .child(fingerprint_hint(&screen));

    super::card_surface(&screen)
        .corner_radius(22.0)
        .squircle()
        .elevation(3.0)
        // Turned about its own centre, which is the default origin. Played by
        // the count of refused attempts, so the second wrong password shakes as
        // loudly as the first.
        .keyframes_transform(shake(), screen.auth.state.rejections)
        .layout(ZStack::new())
        .child(content)
}

fn avatar(screen: &Screen) -> Container {
    let theme = screen.theme;
    let user = screen.user;

    container()
        .width(AVATAR_SIZE)
        .height(AVATAR_SIZE)
        .corner_radius(AVATAR_SIZE / 2.0)
        .background(with_alpha(theme.primary, 0.15))
        .overflow(Overflow::Hidden)
        .layout(
            Flex::column()
                .main_alignment(MainAlignment::Center)
                .cross_alignment(CrossAlignment::Center),
        )
        // The avatar takes its box from this container — `Cover` crops the
        // picture into whatever room it is offered.
        .child(move || match user.get().and_then(|user| user.avatar) {
            Some(path) => image(ImageSource::Path(path))
                .content_fit(ContentFit::Cover)
                .into_any(),
            None => icon()
                .kind(IconKind::from(ACCOUNT_GLYPH))
                .color(theme.primary)
                .font_size(38.0)
                .into_any(),
        })
}

fn name(screen: &Screen) -> Text {
    let user = screen.user;

    text(move || {
        user.get()
            .map(|user| user.display_name)
            // The greeter has nobody yet: the field itself asks.
            .unwrap_or_else(|| "Sign in".to_owned())
    })
    .font_size(21.0)
    .nowrap()
}

/// The input field.
///
/// A real `text_input`, which is what gives it click-to-position, selection and
/// the whole editing vocabulary. What it deliberately does not get is a caret:
/// on a masked field every character looks the same and the position is always
/// the end, so the caret says nothing and costs the only repaint a still lock
/// screen would otherwise make, twice a second, all night.
///
/// It takes the keyboard by itself (`autofocus`), which is why there is nothing
/// to click before typing — see [`super::attach_keys`] for the keys the field
/// does not want. With no caret, the border is the only thing that says where
/// the keys are going, which matters on a second monitor: it draws the same
/// form, and does not hold the focus.
fn field(screen: &Screen) -> Container {
    let theme = screen.theme;
    let auth = screen.auth;
    let state = auth.state;

    // The mask follows the prompt: greetd asks for a user name in the clear, PAM
    // for a password. A memo, so the field is rebuilt when the answer flips
    // rather than on every prompt — `password()` takes a bool, not a signal.
    let secret = create_memo(move || is_secret(&state.prompt.get()));
    let failed = create_memo(move || {
        matches!(
            state.notice.get().map(|notice| notice.level),
            Some(NoticeLevel::Error)
        )
    });

    container()
        .width(FIELD_WIDTH)
        .corner_radius(14.0)
        .squircle()
        // Inset into the card: a step *down* from it, back towards the
        // backdrop colour, so the field reads as a well rather than another
        // panel.
        .background(with_alpha(theme.background, 0.55))
        .border(1.5, with_alpha(theme.text, 0.15))
        // Three layers over that base, and the order is the ranking: the last
        // one that applies and speaks about the border wins. The ring is
        // declared first because it is the weakest of the three — this field
        // holds the focus essentially all the time, so a ring that outranked
        // the others would be a ring that hid them.
        .when_focused(|s| s.border(1.5, with_alpha(theme.primary, 0.6)))
        .state(failed, |s| s.border(1.5, theme.danger))
        .state(state.busy, |s| s.border(1.5, theme.primary))
        .child(
            container()
                .padding([12.0, 16.0])
                .layout(
                    Flex::row()
                        .spacing(8.0)
                        .cross_alignment(CrossAlignment::Center),
                )
                .child(
                    // Only the width is the container's business now; the input
                    // dresses itself. Rebuilt when the mask flips: the old input
                    // leaving the tree drops the focus, which the new one's
                    // `autofocus` then takes — so the greeter's username step
                    // becomes the password step without a click.
                    container().width(fill()).child(move || {
                        text_input(state.input)
                            .password(secret.get())
                            .placeholder(move || prompt_label(&state.prompt.get()))
                            .font_size(16.0)
                            .color(theme.text)
                            .placeholder_color(with_alpha(theme.text, 0.4))
                            .no_caret()
                            .autofocus()
                            .on_submit(move |_| auth.submit())
                    }),
                )
                // Inside the field, where every other password field puts it:
                // the warning is about what this box is receiving. Read from the
                // latched state rather than a key event, so it is already there
                // when the screen appears with caps lock on.
                .child(move || {
                    keyboard_modifiers().get().caps_lock.then(|| {
                        icon()
                            .kind(IconKind::from(CAPS_LOCK_GLYPH))
                            .font_size(16.0)
                            .color(theme.warning)
                    })
                })
                .child(submit(screen)),
        )
}

/// The submit affordance.
///
/// The click lives here and not on the whole field, which is what it used to do:
/// clicking a text field means "put the cursor here", and on the old hand-rolled
/// field it meant "send what you have to PAM" — an empty password on a stray
/// click, which `pam_faillock` counts like any other failed attempt.
fn submit(screen: &Screen) -> Container {
    let theme = screen.theme;
    let auth = screen.auth;
    let state = auth.state;

    container()
        // The glyph is 18px; the padding is what makes it worth aiming at.
        .padding(4.0)
        .corner_radius(10.0)
        .on_click(move || auth.submit())
        .child(
            icon()
                .kind(IconKind::from(SUBMIT_GLYPH))
                .font_size(18.0)
                .color(move || {
                    if state.busy.get() || state.input.get().is_empty() {
                        with_alpha(theme.text, 0.25)
                    } else {
                        theme.primary
                    }
                }),
        )
}

fn notice(screen: &Screen) -> Container {
    let theme = screen.theme;
    let state = screen.auth.state;

    container()
        // Reserved even when empty so the card does not jump on the first
        // failed attempt.
        .height(20.0)
        .layout(Flex::column().main_alignment(MainAlignment::Center))
        .child(move || {
            state.notice.get().filter(|n| !n.text.is_empty()).map(|n| {
                text(n.text)
                    .font_size(13.0)
                    .color(match n.level {
                        NoticeLevel::Error => theme.danger,
                        NoticeLevel::Info => theme.text_weak,
                    })
                    .nowrap()
            })
        })
}

fn fingerprint_hint(screen: &Screen) -> Container {
    let theme = screen.theme;
    let state = screen.auth.state;

    container().child(move || {
        let fingerprint = state.fingerprint.get();
        if fingerprint == FingerprintState::Unavailable {
            return None;
        }

        let (message, color) = match &fingerprint {
            FingerprintState::Waiting => (
                "Or touch the fingerprint sensor".to_owned(),
                theme.text_weak,
            ),
            FingerprintState::Retry(hint) => (hint.clone(), theme.warning),
            // The reader has stopped answering, and the screen has to say so —
            // silence reads as a broken sensor.
            FingerprintState::Exhausted => (
                "Too many fingerprint attempts — use your password".to_owned(),
                theme.text_weak,
            ),
            FingerprintState::Unavailable => unreachable!("filtered above"),
        };

        Some(
            container()
                .layout(
                    Flex::row()
                        .spacing(7.0)
                        .cross_alignment(CrossAlignment::Baseline),
                )
                .child(
                    icon()
                        .kind(IconKind::from(FINGERPRINT_GLYPH))
                        .font_size(15.0)
                        .color(color),
                )
                .child(text(message).font_size(13.0).color(color).nowrap()),
        )
    })
}

fn is_secret(prompt: &Option<crate::auth::Prompt>) -> bool {
    // Defaults to masked: before the first prompt arrives we must not echo
    // something that turns out to be a password.
    prompt.as_ref().map(|prompt| prompt.secret).unwrap_or(true)
}

fn prompt_label(prompt: &Option<crate::auth::Prompt>) -> String {
    match prompt {
        // PAM sends "Password: " with the colon and padding; the field has a
        // label's job here, not a terminal prompt's.
        Some(prompt) => prompt
            .message
            .trim()
            .trim_end_matches(':')
            .trim()
            .to_owned(),
        None => "Password".to_owned(),
    }
}
