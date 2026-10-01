//! The login card: who is logging in, the field, and why the last attempt
//! did not work.

use guido::prelude::*;

use super::Screen;
use crate::Mode;
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
/// nf-md-lock_outline
const LOCK_GLYPH: &str = "\u{f0341}";
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
pub(super) const FIELD_WIDTH: f32 = 360.0;
const AVATAR_SIZE: f32 = 76.0;

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

/// The spinner's turn: once round every 1.2 seconds, at an even pace, for as
/// long as it is on screen. Plainly moving, and not in a hurry about a wait it
/// has no way to shorten.
fn spin() -> Keyframes<f32> {
    Keyframes::new(1200.0)
        .at(0.0, 0.0)
        .at(1.0, 360.0)
        .repeat(Repeat::Forever)
}

/// What an affordance shows: `resting` normally, a turning ring while
/// `turning` says its check is in flight.
///
/// The ring turns on an endless timeline, which plays for as long as the
/// widget carrying it exists — and the ring exists only while the check does,
/// since the caller rebuilds this when the wait starts and when it ends. So
/// the wait needs no end it can name, and a screen at rest has nothing turning
/// and nothing waking it.
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
        .padding([28.0, 32.0])
        .layout(
            Flex::column()
                .spacing(14.0)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(avatar(&screen))
        .child(who(&screen))
        .child(credentials(&screen));

    card(&screen, content)
}

/// How long the field takes to give way to the invitation, and to come back.
const REST_MS: f32 = 350.0;

/// Whether the field turns keys away: while the screen rests, and until the
/// form is back. The key that wakes the screen is only that — it is not
/// typed into a field nobody can see.
///
/// Through the fade as well as the rest, because the compositor says the
/// user is back over a connection of its own, in no order against the key
/// that brought them back: the key can arrive after the form has started to
/// return. It cannot arrive after the fade is over.
fn held(screen: &Screen) -> Memo<bool> {
    let resting = screen.resting;
    let fading = create_signal(false);
    let writer = fading.writer();
    let mut was_resting = false;
    create_effect(move || {
        let now = resting.get();
        if was_resting && !now {
            writer.set(true);
            create_task(move |_ctx| async move {
                tokio::time::sleep(std::time::Duration::from_millis(REST_MS as u64)).await;
                writer.set(false);
            });
        }
        was_resting = now;
    });
    create_memo(move || resting.get() || fading.get())
}

/// The field, what is said about it, and the fingerprint line — and, while
/// the screen rests, the invitation to press a key that takes their place.
///
/// Who is signing in stays: the card is the same card, it only stops asking.
/// That is the reader going to sleep made visible — the field and the line
/// beneath it are the screen saying it is listening, and once the reader is
/// not, they should not be saying so.
///
/// The field is never taken out of the tree, only faded: it keeps the focus,
/// so the key that wakes the screen is not spent on waking — it lands in the
/// field, which comes back with it already there.
///
/// Nothing loops while resting: the screen is likely dark, and a breathing
/// animation would keep the GPU awake for nobody.
fn credentials(screen: &Screen) -> Container {
    let inputs = container()
        .layout(
            Flex::column()
                // Tight: the notice keeps its line even when it has nothing
                // to say, so the space between the field and the fingerprint
                // line is this twice over plus that empty line.
                .spacing(6.0)
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

    // A cross-fade in one box, not a fold: the card keeps its size, so
    // nothing in it moves. A card that changed height would move everything
    // in it by half the change, since it is centred on the screen — and a
    // line of text landing between two rows of pixels is drawn soft.
    let inputs = container()
        .opacity((move || if resting.get() { 0.0 } else { 1.0 }).transition(ease()))
        .child(inputs);
    // Always in the tree and only faded, not built when it is needed: a
    // child that comes and goes is a dynamic child, and a dynamic child in a
    // `ZStack` does not follow the stack it fills — it takes all the room
    // there is, and the card with it.
    let invitation = container()
        .width(fill())
        .height(fill())
        .layout(Flex::column().center())
        .opacity((move || if resting.get() { 1.0 } else { 0.0 }).transition(ease()))
        .child(invitation(theme));

    container()
        .layout(ZStack::new())
        .child(inputs)
        .child(invitation)
}

/// The lock and the words, on one baseline the way the fingerprint line has
/// them: centred boxes would line up glyphs of different sizes by their line
/// boxes, and leave them at unrelated heights.
fn invitation(theme: ThemeColors) -> Container {
    container()
        .layout(
            Flex::row()
                .spacing(12.0)
                .cross_alignment(CrossAlignment::Baseline),
        )
        .child(turning_glyph(LOCK_GLYPH, 24.0, theme.text_weak, false))
        .child(
            text("Press any key to unlock")
                .font_size(17.0)
                .color(theme.text)
                .nowrap(),
        )
}

/// The card the form sits on, lifted off the backdrop and shaken by a refusal.
/// The polkit dialog is one too.
pub(super) fn card(screen: &Screen, content: Container) -> Container {
    super::card_surface(screen)
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
        .layout(Flex::column().center())
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

/// Who is signing in: the user picker in the greeter, the name otherwise.
///
/// The picker lists the accounts people log in to and then, unless
/// `[greeter] other_user` is off, "Other…", which has the form ask for the
/// name. With nobody listed there is nothing to pick and the form asks
/// anyway, so the name line stands in.
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
    // Held while an answer is checked: the attempt belongs to the account
    // it was typed for.
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
pub(super) fn field(screen: &Screen) -> Container {
    let theme = screen.theme;
    let auth = screen.auth;
    let state = auth.state;
    let held = held(screen);
    // The lock screen hides the pointer (see `super::lock_screen`), and the
    // field would bring it back as an I-beam: a caret-less masked field has
    // nothing a click could place. The greeter keeps it, for the user name.
    let pointer = match screen.mode {
        Mode::Greeter | Mode::Polkit => CursorIcon::Text,
        Mode::Lock | Mode::Preview => CursorIcon::Hidden,
    };

    // The field follows the prompt: greetd asks for a user name in the clear,
    // PAM for a password, and they are two different widgets — a password is
    // typed into guido's `Password`, a name into a `String`. A memo, so the
    // field is rebuilt when the answer flips rather than on every prompt.
    let secret = create_memo(move || state.asks_secret());
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
                        // The two field types share their builder but no trait
                        // allock can name, so the common dress is spelled once
                        // here.
                        macro_rules! dressed {
                            ($field:expr) => {
                                $field
                                    .placeholder(move || prompt_label(&state.prompt.get()))
                                    .font_size(16.0)
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
        // Reserved even when empty so the card does not jump on the first
        // failed attempt.
        .height(20.0)
        .layout(Flex::column().main_alignment(MainAlignment::Center))
        .child(move || {
            let notice = state.notice.get().filter(|n| !n.text.is_empty());
            let line = |words: String, color: Color| {
                Some(text(words).font_size(13.0).color(color).nowrap())
            };
            match notice {
                Some(n) if n.level == NoticeLevel::Error => line(n.text, theme.danger),
                // The line the answer will land on, saying that it is coming.
                // PAM takes its time over a wrong password on purpose, and an
                // empty line for those two seconds reads as an Enter that went
                // nowhere — then the refusal appears out of nothing.
                _ if state.busy.get() => line("Checking…".to_owned(), theme.text_weak),
                // A lock is said before the password is typed, not after: PAM
                // would only say so once it had been. A forecast, so the field
                // stays open — see `crate::faillock`.
                _ if let Some(locked) = state.locked.get() => line(locked, theme.warning),
                Some(n) => line(n.text, theme.text_weak),
                None => None,
            }
        })
}

/// How long the fingerprint line takes to open or close.
const HINT_REVEAL_MS: f32 = 250.0;

/// The fingerprint line, under the field.
///
/// It appears a moment after the screen does — fprintd answers over D-Bus, and
/// until then there is nothing to invite — and it goes if the reader does. So
/// its box follows the line with an eased height rather than jumping to it:
/// the card grows and the block around it glides instead of shifting, with no
/// height written down anywhere. The box clips what it has not yet opened to,
/// and the line fades in as it is uncovered.
fn fingerprint_hint(screen: &Screen) -> Container {
    let theme = screen.theme;
    let state = screen.auth.state;
    let reveal = || Transition::new(HINT_REVEAL_MS, TimingFunction::EaseInOut);

    // Whether there is a line at all, apart from what it says: the line is
    // built, and fades in, once per appearance, and a change of message only
    // redraws what is inside it.
    // Resting has no line either: the form it sits in has given way to an
    // invitation of its own, and coming back the line opens again once the
    // reader is listening rather than speaking for a reader still waking.
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
                            .spacing(7.0)
                            .cross_alignment(CrossAlignment::Baseline),
                    )
                    .opacity(1.0.transition(reveal()).entering_from(0.0))
                    .children(move || hint_line(theme, state.fingerprint.get()))
            })
        })
}

/// The glyph and the words for one fingerprint state; nothing when there is
/// no reader to invite.
fn hint_line(theme: ThemeColors, fingerprint: FingerprintState) -> Vec<Container> {
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
        FingerprintState::Unavailable | FingerprintState::Resting => return Vec::new(),
    };
    vec![
        // The same swap the submit arrow makes, for the same reason: the
        // touch is answered where the touch was invited.
        turning_glyph(FINGERPRINT_GLYPH, HINT_GLYPH_SIZE, color, reading),
        container().child(text(message).font_size(13.0).color(color).nowrap()),
    ]
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
