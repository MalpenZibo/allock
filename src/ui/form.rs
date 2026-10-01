//! The login card: who is logging in, the field, and the last notice.

use std::time::Duration;

use guido::prelude::*;

use super::Screen;
use super::tokens::{self, Typography, icon_size, radius, space};
use crate::Mode;
use crate::auth::{FingerprintState, NoticeLevel};
use crate::components::icons::{IconKind, icon};
use crate::theme::{ThemeColors, with_alpha};

// Codepoints verified against SymbolsNerdFont-Regular's cmap, not guessed:
// the neighbourhood is dense and wrong-but-present glyphs render silently.
/// nf-md-fingerprint
const FINGERPRINT_GLYPH: &str = "\u{f0237}";
/// nf-md-arrow_right_bold_circle_outline
const SUBMIT_GLYPH: &str = "\u{f0057}";
/// nf-md-account
const ACCOUNT_GLYPH: &str = "\u{f0004}";
/// nf-md-lock_outline
const LOCK_GLYPH: &str = "\u{f0341}";
/// nf-md-apple_keyboard_caps
const CAPS_LOCK_GLYPH: &str = "\u{f0632}";
/// nf-fa-circle_o_notch. Not md-loading (`f0772`), which is drawn at half size;
/// its ink is centred on the line box, so it turns in place. `f1cd` is a lifebuoy.
const SPINNER_GLYPH: &str = "\u{f1ce}";

/// The fa- spinner fills 0.906 of its em box against the md- glyphs' 0.832.
const SPINNER_SCALE: f32 = 0.918;

/// Exact, not a minimum: under the centring column a minimum grows to fill the output.
pub(super) const FIELD_WIDTH: f32 = 360.0;
const AVATAR_SIZE: f32 = space::S80;

const HINT_GLYPH_SIZE: f32 = icon_size::I16;

const SUBMIT_SIZE: f32 = icon_size::I16;

/// Material elevation 3.
const CARD_SHADOW: Shadow = Shadow::new((0.0, 3.0), 6.0, 0.0, Color::rgba(0.0, 0.0, 0.0, 0.19));

const FIELD_BORDER: f32 = 1.5;

/// The wobble a refused attempt gets, in degrees.
fn shake() -> Keyframes<f32> {
    Keyframes::new(320.0)
        .at(0.0, 0.0)
        .at(0.15, 1.2)
        .at(0.40, -1.2)
        .at(0.65, 0.9)
        .at(0.85, -0.45)
        .at(1.0, 0.0)
}

fn spin() -> Keyframes<f32> {
    Keyframes::new(1200.0)
        .at(0.0, 0.0)
        .at(1.0, 360.0)
        .repeat(Repeat::Forever)
}

/// `resting`, or a spinner while `turning`. The caller rebuilds it when `turning`
/// flips, so the endless spin stops with it. The height is pinned because the two
/// glyphs have different line boxes and the row would change height.
fn turning_glyph<M>(
    resting: &str,
    size: f32,
    color: impl IntoSignal<Color, M>,
    turning: bool,
) -> Container {
    let (kind, font_size) = if turning {
        (SPINNER_GLYPH, size * SPINNER_SCALE)
    } else {
        (resting, size)
    };
    let glyph = container()
        .height(size)
        .layout(Flex::column().center())
        .child(
            icon()
                .kind(IconKind::from(kind))
                .font_size(font_size)
                .color(color),
        );
    if turning {
        glyph.rotate(0.0.timeline(spin()))
    } else {
        glyph
    }
}

pub fn view(screen: Screen) -> Container {
    let content = container()
        .padding(space::S32)
        .layout(
            Flex::column()
                .spacing(space::S16)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(avatar(&screen))
        .child(who(&screen))
        .child(credentials(&screen));

    card(&screen, content)
}

const REST_MS: f32 = 350.0;

/// Whether the field turns keys away: while resting and through the fade back,
/// since the compositor's "user is back" is unordered against the waking key.
fn held(screen: &Screen) -> Signal<bool> {
    let resting = screen.resting;
    let held = create_signal(false);
    // Each run cancels the timer the last one set.
    create_effect(move || {
        if resting.get() {
            held.set(true);
        } else {
            set_timeout(Duration::from_millis(REST_MS as u64), move || {
                held.set(false)
            });
        }
    });
    held.into()
}

/// The field, notice and fingerprint line, cross-faded with an invitation while
/// resting. The field is only faded, never removed, so it keeps the focus.
fn credentials(screen: &Screen) -> Container {
    let inputs = container()
        .layout(
            Flex::column()
                // Tight: the notice keeps an empty line between field and hint.
                .spacing(space::S4)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(field(screen))
        .child(notice(screen))
        .child(fingerprint_hint(screen));
    if !matches!(screen.mode, Mode::Lock | Mode::Preview) {
        return inputs;
    }

    let theme = screen.theme;
    let resting = screen.resting;
    let ease = || Transition::new(REST_MS, TimingFunction::EaseInOut);

    // A cross-fade, not a fold: a centred card changing height would land text
    // between pixel rows.
    let inputs = container()
        .opacity((move || if resting.get() { 0.0 } else { 1.0 }).transition(ease()))
        .child(inputs);
    let invitation = move || {
        resting.get().then(|| {
            container()
                .width(fill())
                .height(fill())
                .layout(Flex::column().center())
                .opacity(1.0.transition(ease()).entering_from(0.0).exiting_to(0.0))
                .child(invitation(theme))
        })
    };

    container()
        .layout(ZStack::new())
        .child(inputs)
        .child(invitation)
}

fn invitation(theme: ThemeColors) -> Container {
    container()
        .layout(
            Flex::row()
                .spacing(space::S12)
                .cross_alignment(CrossAlignment::Baseline),
        )
        .child(turning_glyph(
            LOCK_GLYPH,
            icon_size::I24,
            theme.text_weak,
            false,
        ))
        .child(
            text("Press any key to unlock")
                .typography(tokens::TITLE_M)
                .color(theme.text)
                .nowrap(),
        )
}

/// The card the form sits on, shaken by a refusal. The polkit dialog uses it too.
pub(super) fn card(screen: &Screen, content: Container) -> Container {
    super::card_surface(screen)
        .corners(Corners::squircle(radius::R16))
        .shadow(CARD_SHADOW)
        // Played by the rejection count, so every refusal restarts it.
        .rotate(0.0.timeline(shake().played_by(screen.auth.state.rejections)))
        .layout(ZStack::new())
        .child(content)
}

fn avatar(screen: &Screen) -> Container {
    let theme = screen.theme;
    let user = screen.user;

    container()
        .width(AVATAR_SIZE)
        .height(AVATAR_SIZE)
        .corners(AVATAR_SIZE / 2.0)
        .background(with_alpha(theme.primary, 0.15))
        .overflow(Overflow::Hidden)
        .layout(Flex::column().center())
        .child(move || match user.get().and_then(|user| user.avatar) {
            Some(path) => image(ImageSource::Path(path))
                .content_fit(ContentFit::Cover)
                .into_any(),
            None => icon()
                .kind(IconKind::from(ACCOUNT_GLYPH))
                .color(theme.primary)
                .font_size(icon_size::I40)
                .into_any(),
        })
}

/// Who is signing in: the user picker in the greeter, the name otherwise.
fn who(screen: &Screen) -> Container {
    if screen.mode != Mode::Greeter || screen.users.is_empty() {
        return container().child(name(screen));
    }
    let other = screen
        .config
        .greeter
        .other_user
        .then(|| "Other…".to_owned());
    let names = screen
        .users
        .iter()
        .map(|user| user.display_name.clone())
        .chain(other)
        .collect();
    // Held while an answer is checked: the attempt belongs to that account.
    let busy = screen.auth.state.busy;
    super::picker::view(
        screen,
        names,
        screen.user_choice,
        super::picker::Look::Plain,
        busy.into_signal(),
    )
}

fn name(screen: &Screen) -> Text {
    let user = screen.user;

    text(move || {
        user.get()
            .map(|user| user.display_name)
            .unwrap_or_else(|| "Sign in".to_owned())
    })
    .typography(tokens::TITLE_L)
    .color(screen.theme.text)
    .nowrap()
}

/// The input field. No caret: on a masked field it says nothing and its blink
/// would be the only repaint a still lock screen makes.
pub(super) fn field(screen: &Screen) -> Container {
    let theme = screen.theme;
    let auth = screen.auth;
    let state = auth.state;
    let held = held(screen);
    // Keep the lock screen's pointer hidden; the greeter needs the I-beam for names.
    let pointer = match screen.mode {
        Mode::Greeter | Mode::Polkit => CursorIcon::Text,
        Mode::Lock | Mode::Preview => CursorIcon::Hidden,
    };

    // A user name and a password are different widgets; the memo rebuilds the
    // field only when that flips, not on every prompt.
    let secret = create_memo(move || state.asks_secret());
    let failed = create_memo(move || {
        matches!(
            state.notice.get().map(|notice| notice.level),
            Some(NoticeLevel::Error)
        )
    });

    container()
        .width(FIELD_WIDTH)
        .corners(Corners::squircle(radius::R8))
        .background(with_alpha(theme.background, 0.55))
        .border(FIELD_BORDER, with_alpha(theme.text, 0.15))
        // The last applying layer wins; focus is first because it is almost always on.
        .when_focused(|s| s.border(FIELD_BORDER, with_alpha(theme.primary, 0.6)))
        .state(failed, |s| s.border(FIELD_BORDER, theme.danger))
        .state(state.busy, |s| s.border(FIELD_BORDER, theme.primary))
        .child(
            container()
                .padding([space::S12, space::S16])
                .layout(
                    Flex::row()
                        .spacing(space::S8)
                        .cross_alignment(CrossAlignment::Center),
                )
                .child(
                    // Rebuilt when the mask flips; the new input's `autofocus` takes
                    // the focus the old one dropped.
                    container().width(fill()).child(move || {
                        // The two input types share no nameable trait.
                        macro_rules! dressed {
                            ($field:expr) => {
                                $field
                                    .placeholder(move || prompt_label(&state.prompt.get()))
                                    .font_size(tokens::BODY_L.size)
                                    .line_height(LineHeight::Absolute(tokens::BODY_L.line))
                                    .color(theme.text)
                                    .placeholder_color(with_alpha(theme.text, 0.4))
                                    .no_caret()
                                    .cursor(pointer)
                                    .readonly(held)
                                    .autofocus()
                            };
                        }
                        if secret.get() {
                            container().child(
                                dressed!(password_input(state.password))
                                    .on_submit(move || auth.submit()),
                            )
                        } else {
                            container().child(
                                dressed!(text_input(state.input)).on_submit(move |_| auth.submit()),
                            )
                        }
                    }),
                )
                // Latched modifier state, so it shows when the screen appears with caps lock on.
                .child(move || {
                    keyboard_modifiers().get().caps_lock.then(|| {
                        icon()
                            .kind(IconKind::from(CAPS_LOCK_GLYPH))
                            .font_size(icon_size::I16)
                            .color(theme.warning)
                    })
                })
                .child(submit(screen)),
        )
}

/// The submit affordance. Only it takes clicks: a stray click submitting an empty
/// password would count as a `pam_faillock` failure.
fn submit(screen: &Screen) -> Container {
    let theme = screen.theme;
    let auth = screen.auth;
    let state = auth.state;

    container()
        .padding(space::S4)
        .corners(radius::R8)
        .on_click(move || auth.submit())
        .child(move || {
            let checking = state.busy.get();
            turning_glyph(
                SUBMIT_GLYPH,
                SUBMIT_SIZE,
                // Read inside, so typing re-tints the arrow instead of rebuilding it.
                move || {
                    if checking || !state.field_is_empty() {
                        theme.primary
                    } else {
                        with_alpha(theme.text, 0.25)
                    }
                },
                checking,
            )
        })
}

fn notice(screen: &Screen) -> Container {
    let theme = screen.theme;
    let state = screen.auth.state;

    container()
        // Reserved even when empty so the card does not jump.
        .height(tokens::BODY_S.line)
        .layout(Flex::column().main_alignment(MainAlignment::Center))
        .child(move || {
            let notice = state.notice.get().filter(|n| !n.text.is_empty());
            let line = |words: String, color: Color| {
                Some(text(words).typography(tokens::BODY_S).color(color).nowrap())
            };
            match notice {
                Some(n) if n.level == NoticeLevel::Error => line(n.text, theme.danger),
                // PAM delays a wrong password on purpose; an empty line meanwhile
                // reads as an Enter that went nowhere.
                _ if state.busy.get() => line("Checking…".to_owned(), theme.text_weak),
                // A faillock forecast, said before typing; the field stays open.
                _ if let Some(locked) = state.locked.get() => line(locked, theme.warning),
                Some(n) => line(n.text, theme.text_weak),
                None => None,
            }
        })
}

const HINT_REVEAL_MS: f32 = 250.0;

/// The fingerprint line. It appears once fprintd answers over D-Bus, so its
/// height eases open instead of jumping.
fn fingerprint_hint(screen: &Screen) -> Container {
    let theme = screen.theme;
    let state = screen.auth.state;
    let reveal = || Transition::new(HINT_REVEAL_MS, TimingFunction::EaseInOut);

    // Separate from the message, so a new message does not replay the fade-in.
    let shown = create_memo(move || {
        !matches!(
            state.fingerprint.get(),
            FingerprintState::Unavailable | FingerprintState::Resting
        )
    });

    container()
        .height(Length::default().transition(reveal()))
        .overflow(Overflow::Hidden)
        .child(move || {
            shown.get().then(|| {
                container()
                    .layout(
                        Flex::row()
                            .spacing(space::S8)
                            .cross_alignment(CrossAlignment::Baseline),
                    )
                    .opacity(1.0.transition(reveal()).entering_from(0.0))
                    .children(move || hint_line(theme, state.fingerprint.get()))
            })
        })
}

fn hint_line(theme: ThemeColors, fingerprint: FingerprintState) -> Vec<Container> {
    let reading = fingerprint == FingerprintState::Reading;
    let (message, color) = match &fingerprint {
        FingerprintState::Waiting => (
            "Or touch the fingerprint sensor".to_owned(),
            theme.text_weak,
        ),
        // From fprintd's `finger-present`, its only signal before the verdict.
        FingerprintState::Reading => ("Checking your fingerprint…".to_owned(), theme.primary),
        FingerprintState::Retry(hint) => (hint.clone(), theme.warning),
        FingerprintState::Exhausted => (
            "Too many fingerprint attempts — use your password".to_owned(),
            theme.text_weak,
        ),
        FingerprintState::Unavailable | FingerprintState::Resting => return Vec::new(),
    };
    vec![
        turning_glyph(FINGERPRINT_GLYPH, HINT_GLYPH_SIZE, color, reading),
        container().child(
            text(message)
                .typography(tokens::BODY_S)
                .color(color)
                .nowrap(),
        ),
    ]
}

fn prompt_label(prompt: &Option<crate::auth::Prompt>) -> String {
    match prompt {
        // PAM sends "Password: " with the colon and padding.
        Some(prompt) => prompt
            .message
            .trim()
            .trim_end_matches(':')
            .trim()
            .to_owned(),
        None => "Password".to_owned(),
    }
}
