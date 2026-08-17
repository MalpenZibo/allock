//! The login card: who is logging in, the field, and why the last attempt
//! did not work.

use guido::prelude::*;

use super::Screen;
use crate::auth::{FingerprintState, NoticeLevel};
use crate::components::icons::{IconKind, icon};
use crate::theme::{ThemeColors, with_alpha};

// Codepoints verified against SymbolsNerdFont-Regular's cmap, not guessed:
// the neighbourhood is dense and wrong-but-present glyphs render silently.
/// nf-md-fingerprint
const FINGERPRINT_GLYPH: &str = "\u{f0237}";
/// nf-md-arrow_right_bold_circle_outline — the "submit" affordance.
const SUBMIT_GLYPH: &str = "\u{f0057}";
/// nf-md-account
const ACCOUNT_GLYPH: &str = "\u{f0004}";

/// Exact, not a minimum: under the centring column a minimum width grows to
/// fill the output, and the card grows with it into a full-width band.
const FIELD_WIDTH: f32 = 360.0;
const AVATAR_SIZE: f32 = 76.0;

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

fn name(screen: &Screen) -> Container {
    let user = screen.user;

    container().font_size(21.0).child(
        text(move || {
            user.get()
                .map(|user| user.display_name)
                // The greeter has nobody yet: the field itself asks.
                .unwrap_or_else(|| "Sign in".to_owned())
        })
        .nowrap(),
    )
}

/// The input field.
///
/// There is no `text_input` here on purpose — see [`super::attach_keys`]. The
/// buffer is rendered as dots for a secret prompt and verbatim for a visible
/// one (the greeter's username step), with a static caret at the end.
fn field(screen: &Screen) -> Container {
    let theme = screen.theme;
    let auth = screen.auth;
    let state = auth.state;

    container()
        .width(FIELD_WIDTH)
        .corner_radius(14.0)
        .squircle()
        // Inset into the card: a step *down* from it, back towards the
        // backdrop colour, so the field reads as a well rather than another
        // panel.
        .background(with_alpha(theme.background, 0.55))
        .border(1.5, move || {
            if state.busy.get() {
                with_alpha(theme.primary, 0.7)
            } else if matches!(
                state.notice.get().map(|notice| notice.level),
                Some(NoticeLevel::Error)
            ) {
                theme.danger
            } else {
                with_alpha(theme.text, 0.15)
            }
        })
        .child(
            container()
                .padding([12.0, 16.0])
                .layout(
                    Flex::row()
                        .spacing(8.0)
                        .cross_alignment(CrossAlignment::Center),
                )
                .child(
                    container()
                        .width(fill())
                        .layout(
                            Flex::row()
                                .spacing(2.0)
                                .cross_alignment(CrossAlignment::Center),
                        )
                        .child(
                            container()
                                .font_size(16.0)
                                // Dimmed while the placeholder shows.
                                .text_color(move || {
                                    if state.input.get().is_empty() {
                                        with_alpha(theme.text, 0.4)
                                    } else {
                                        theme.text
                                    }
                                })
                                .child(
                                    text(move || {
                                        let buffer = state.input.get();
                                        if buffer.is_empty() {
                                            return placeholder(&state.prompt.get());
                                        }
                                        if is_secret(&state.prompt.get()) {
                                            // Count graphemes, not bytes: one
                                            // accented character must not show
                                            // as two dots.
                                            "\u{2022}".repeat(buffer.chars().count())
                                        } else {
                                            buffer
                                        }
                                    })
                                    .nowrap(),
                                ),
                        )
                        .child(caret(theme, state.input)),
                )
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
                ),
        )
        .on_click(move || auth.submit())
}

fn caret(theme: ThemeColors, input: RwSignal<String>) -> Container {
    container()
        .width(2.0)
        .height(20.0)
        .corner_radius(1.0)
        .background(move || {
            // Hidden while the placeholder shows, so the two do not collide.
            if input.get().is_empty() {
                Color::TRANSPARENT
            } else {
                theme.primary
            }
        })
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
                container()
                    .font_size(13.0)
                    .text_color(match n.level {
                        NoticeLevel::Error => theme.danger,
                        NoticeLevel::Info => theme.text_weak,
                    })
                    .child(text(n.text).nowrap())
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
            FingerprintState::Exhausted => ("Fingerprint unavailable".to_owned(), theme.text_weak),
            FingerprintState::Unavailable => unreachable!("filtered above"),
        };

        Some(
            container()
                .font_size(13.0)
                .text_color(color)
                .layout(
                    Flex::row()
                        .spacing(7.0)
                        .cross_alignment(CrossAlignment::Center),
                )
                .child(
                    icon()
                        .kind(IconKind::from(FINGERPRINT_GLYPH))
                        .font_size(15.0)
                        .color(color),
                )
                .child(text(message).nowrap()),
        )
    })
}

fn is_secret(prompt: &Option<crate::auth::Prompt>) -> bool {
    // Defaults to masked: before the first prompt arrives we must not echo
    // something that turns out to be a password.
    prompt.as_ref().map(|prompt| prompt.secret).unwrap_or(true)
}

fn placeholder(prompt: &Option<crate::auth::Prompt>) -> String {
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
