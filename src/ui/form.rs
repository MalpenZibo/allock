//! The login card: who is logging in, the field, and why the last attempt
//! did not work.

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
/// nf-fa-circle_o_notch — the open ring that stands in while a check runs.
///
/// From the fa- range and not md- like the rest, because the md- spinner
/// (`md-loading`, `f0772`) is drawn at half the size of every glyph around it
/// and would come out a third of the ring it replaces. `f1cd` next door is
/// `fa-support`, a lifebuoy: also a ring, and silently the wrong one.
///
/// Its ink is centred on the line box in both axes — 614 units above the
/// baseline, which is exactly the box's own centre — so it turns in place
/// rather than orbiting.
const SPINNER_GLYPH: &str = "\u{f1ce}";

/// How much smaller the spinner is drawn than the glyph it stands in for.
///
/// `fa-circle_o_notch` fills 0.906 of its em box against the md- glyphs' 0.832,
/// so at the same font size its ring would be the larger of the two. This is
/// that ratio, and it makes the two rings the same size on screen.
const SPINNER_SCALE: f32 = 0.918;

/// Exact, not a minimum: under the centring column a minimum width grows to
/// fill the output, and the card grows with it into a full-width band.
const FIELD_WIDTH: f32 = 360.0;
const AVATAR_SIZE: f32 = 76.0;

/// Degrees the spinner turns per step of `AuthState::spin`.
///
/// The auth task steps twenty-five times a second, so twelve degrees is a turn
/// every 1.2 seconds: plainly moving, and not in a hurry about a wait it has no
/// way to shorten.
const SPIN_STEP: f32 = 12.0;

/// The glyph beside the fingerprint hint, and the spinner that stands in for
/// it while the sensor is reading.
const HINT_GLYPH_SIZE: f32 = 15.0;

/// The submit affordance and the spinner that stands in for it.
const SUBMIT_SIZE: f32 = 18.0;

/// The lift the card is drawn with.
///
/// What `elevation(3.0)` used to resolve to in guido's Material table, written
/// out now that the library ships no ladder: the card is a panel a step above
/// the wallpaper, not a thing that floats.
const CARD_SHADOW: Shadow = Shadow::new((0.0, 3.0), 6.0, 0.0, Color::rgba(0.0, 0.0, 0.0, 0.19));

/// The field's frame, base and every state layer over it.
///
/// A border is a width and a colour together — a layer that only recolours
/// still restates the width — so the width lives here and the layers below
/// name only what they change.
const FIELD_BORDER: f32 = 1.5;

/// The wobble a refused attempt gets.
///
/// Small angles in degrees and a decaying amplitude — the shake of a head
/// saying no, not a machine falling over. A third of a second: long enough to
/// be seen, short enough that a second wrong password can start its own before
/// the first is forgotten. Started again mid-run it restarts, which is what a
/// second refusal means.
fn shake() -> Keyframes<f32> {
    Keyframes::new(320.0)
        .at(0.0, 0.0)
        .at(0.15, 1.2)
        .at(0.40, -1.2)
        .at(0.65, 0.9)
        .at(0.85, -0.45)
        .at(1.0, 0.0)
}

/// What an affordance shows: `resting` normally, a turning ring while `spin`
/// says its check is in flight.
///
/// Not a keyframe. guido's timelines are finite and play from a trigger, and a
/// wait has no end it can name. So the angle is stepped instead, by the auth
/// task, which is the one that knows when the wait starts and when it ends;
/// this only reads the count as degrees.
///
/// The box is pinned to `size` because the two glyphs do not have the same line
/// box, and the row this sits in takes its height from its tallest child: the
/// field would otherwise lose a pixel and a half of height every time a
/// password was checked. The box costs nothing in a baseline row — a container
/// republishes the baseline of the first child that has one, so the glyph still
/// sits where the text beside it sits.
fn turning_glyph<M>(
    resting: &str,
    size: f32,
    color: impl IntoSignal<Color, M>,
    spin: Option<RwSignal<u32>>,
) -> Container {
    let (kind, font_size) = match spin {
        Some(_) => (SPINNER_GLYPH, size * SPINNER_SCALE),
        None => (resting, size),
    };
    let glyph = container()
        .height(size)
        .layout(
            Flex::column()
                .main_alignment(MainAlignment::Center)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(
            icon()
                .kind(IconKind::from(kind))
                .font_size(font_size)
                .color(color),
        );
    match spin {
        Some(spin) => glyph.rotate(
            // The task steps twenty-five times a second and the screen draws
            // rather more often than that, so the steps are carried rather than
            // jumped: the angle eases over exactly one step's worth of time, and
            // arrives just as the next one is published.
            (move || spin.get() as f32 * SPIN_STEP).transition(Transition::new(
                crate::auth::SPIN_PERIOD.as_millis() as f32,
                TimingFunction::Linear,
            )),
        ),
        None => glyph,
    }
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
        .corners(Corners::squircle(22.0))
        .shadow(CARD_SHADOW)
        // Turned about its own centre, which is the default pivot. Played by
        // the count of refused attempts, so the second wrong password shakes as
        // loudly as the first, and resting at zero in between.
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
    .color(screen.theme.text)
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
        .corners(Corners::squircle(14.0))
        // Inset into the card: a step *down* from it, back towards the
        // backdrop colour, so the field reads as a well rather than another
        // panel.
        .background(with_alpha(theme.background, 0.55))
        .border(FIELD_BORDER, with_alpha(theme.text, 0.15))
        // Three layers over that base, and the order is the ranking: the last
        // one that applies and speaks about the border wins. The ring is
        // declared first because it is the weakest of the three — this field
        // holds the focus essentially all the time, so a ring that outranked
        // the others would be a ring that hid them.
        .when_focused(|s| s.border(FIELD_BORDER, with_alpha(theme.primary, 0.6)))
        .state(failed, |s| s.border(FIELD_BORDER, theme.danger))
        .state(state.busy, |s| s.border(FIELD_BORDER, theme.primary))
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
        .corners(10.0)
        .on_click(move || auth.submit())
        // The affordance that took the attempt is the one that answers for it:
        // press the arrow, or Enter, and the arrow is what starts turning. A
        // second Enter is refused while it does — see `Auth::submit`.
        .child(move || {
            let checking = state.busy.get();
            turning_glyph(
                SUBMIT_GLYPH,
                SUBMIT_SIZE,
                // Read inside, so typing the first character re-tints the arrow
                // instead of rebuilding it — and not read at all while the ring
                // turns, which is lit whatever the field holds.
                move || {
                    if checking || !state.input.get().is_empty() {
                        theme.primary
                    } else {
                        with_alpha(theme.text, 0.25)
                    }
                },
                checking.then_some(state.spin),
            )
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
            match state.notice.get().filter(|n| !n.text.is_empty()) {
                Some(n) => Some(
                    text(n.text)
                        .font_size(13.0)
                        .color(match n.level {
                            NoticeLevel::Error => theme.danger,
                            NoticeLevel::Info => theme.text_weak,
                        })
                        .nowrap(),
                ),
                // The line the answer will land on, saying that it is coming.
                // PAM takes its time over a wrong password on purpose, and an
                // empty line for those two seconds reads as an Enter that went
                // nowhere — then the refusal appears out of nothing.
                None if state.busy.get() => Some(
                    text("Checking…")
                        .font_size(13.0)
                        .color(theme.text_weak)
                        .nowrap(),
                ),
                None => None,
            }
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

        let reading = fingerprint == FingerprintState::Reading;
        let (message, color) = match &fingerprint {
            FingerprintState::Waiting => (
                "Or touch the fingerprint sensor".to_owned(),
                theme.text_weak,
            ),
            // fprintd says nothing between the touch and the verdict except
            // `finger-present`, and this is what allock does with it: the
            // sensor stops asking and starts answering.
            FingerprintState::Reading => ("Checking your fingerprint…".to_owned(), theme.primary),
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
                // The same swap the submit arrow makes, for the same reason:
                // the touch is answered where the touch was invited.
                .child(turning_glyph(
                    FINGERPRINT_GLYPH,
                    HINT_GLYPH_SIZE,
                    color,
                    reading.then_some(state.spin),
                ))
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
