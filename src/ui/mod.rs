//! The lock screen, one instance per output.
//!
//! Every output gets a surface — the session-lock protocol requires it — but
//! only the outputs selected by `form_outputs` draw the login form; the rest
//! show the clock over the same background. All of them read the same
//! signals, so the form mirrors itself across screens: whichever surface has
//! keyboard focus drives the shared input buffer.

pub mod clock;
pub mod form;
pub mod status;

use std::rc::Rc;

use chrono::{DateTime, Local};
use guido::prelude::*;

use crate::Mode;
use crate::auth::Auth;
use crate::config::{BackgroundFit, Config, SessionDef};
use crate::services::upower::UPowerService;
use crate::theme::{ThemeColors, with_alpha};
use crate::user::UserInfo;

/// Everything a lock surface needs. Cheap to clone — signals are arena ids
/// and the heavy pieces sit behind `Rc`.
#[derive(Clone)]
pub struct Screen {
    pub config: Rc<Config>,
    pub theme: ThemeColors,
    pub auth: Auth,
    pub mode: Mode,
    /// Mutable in greeter mode, where the form asks who is logging in.
    pub user: RwSignal<Option<UserInfo>>,
    /// `None` when the battery indicator is off or there is no battery.
    pub battery: Option<RwSignal<Option<UPowerService>>>,
    pub sessions: Rc<Vec<SessionDef>>,
    pub session_index: RwSignal<usize>,
    pub now: Signal<DateTime<Local>>,
}

impl Screen {
    fn wallpaper(&self) -> Option<std::path::PathBuf> {
        self.config.background.resolved_path()
    }
}

pub fn lock_screen(screen: Screen, output: OutputInfo) -> Container {
    let theme = screen.theme;
    let shows_form = screen.config.form_outputs.matches(
        output.name.as_deref().unwrap_or(""),
        &output.make,
        &output.model,
    );

    let centre = container()
        .width(fill())
        .height(fill())
        .layout(
            Flex::column()
                .spacing(48.0)
                .main_alignment(MainAlignment::Center)
                .cross_alignment(CrossAlignment::Center),
        )
        .child(clock::view(screen.now, &screen.config.clock, theme))
        .maybe_child(shows_form.then(|| form::view(screen.clone())));

    let bottom = container()
        .width(fill())
        .height(fill())
        .padding(28.0)
        .layout(Flex::column().main_alignment(MainAlignment::End))
        .child(status::view(screen.clone(), shows_form));

    let root = container()
        .width(fill())
        .height(fill())
        // Painted under the wallpaper so a missing or half-loaded image never
        // shows through to a transparent surface.
        .background(theme.background)
        // Declared once for the whole screen: text style is inherited per
        // property, so the parts below only name the colours that differ.
        .text_color(theme.text)
        .layout(ZStack::new())
        .maybe_child(background_image(&screen))
        .maybe_child(scrim(&screen))
        .child(centre)
        .child(bottom);

    attach_keys(root, screen)
}

/// The lock screen owns key handling at the root instead of using a focused
/// text field: typing has to work the instant the screen appears, with no
/// click to focus and no visible pointer. It also keeps the clipboard out of
/// the password path, which is the behaviour a lock screen wants anyway.
fn attach_keys(root: Container, screen: Screen) -> Container {
    let auth = screen.auth;
    root.on_key_down(move |key, modifiers| {
        if auth.state.authenticated.get_untracked() {
            return;
        }
        match key {
            Key::Enter => auth.submit(),
            // Escape clears what was typed; pressing it again on an already
            // empty field restarts the conversation, which is the only way
            // out of a PAM stage that is waiting on something other than the
            // keyboard.
            Key::Escape => {
                if auth.state.input.get_untracked().is_empty() {
                    auth.restart();
                } else {
                    auth.state.input.set(String::new());
                }
            }
            // Ctrl+U: the readline habit for "clear the line".
            Key::Char('u') if modifiers.ctrl => auth.state.input.set(String::new()),
            Key::Backspace => auth.state.input.update(|buffer| {
                buffer.pop();
            }),
            // Modified characters are shortcuts, not input.
            Key::Char(c)
                if !modifiers.ctrl
                    && !modifiers.alt
                    && !modifiers.logo
                    && !auth.state.busy.get_untracked() =>
            {
                auth.state.input.update(|buffer| buffer.push(c));
            }
            _ => {}
        }
    })
}

fn background_image(screen: &Screen) -> Option<Container> {
    let source = ImageSource::Path(screen.wallpaper()?);
    let fit = match screen.config.background.fit {
        BackgroundFit::Cover => ContentFit::Cover,
        BackgroundFit::Contain => ContentFit::Contain,
        BackgroundFit::Stretch => ContentFit::Fill,
    };

    // The box comes from this container, which is the whole surface: `Cover`
    // and `Stretch` take all of it, `Contain` the largest rect of the picture's
    // own aspect ratio that fits, letterboxed by the alignment here.
    Some(
        container()
            .width(fill())
            .height(fill())
            .layout(
                Flex::column()
                    .main_alignment(MainAlignment::Center)
                    .cross_alignment(CrossAlignment::Center),
            )
            .child(image(source).content_fit(fit)),
    )
}

fn scrim(screen: &Screen) -> Option<Container> {
    let dim = screen.config.background.dim.clamp(0.0, 1.0);
    // Without a wallpaper the background is already the theme colour; dimming
    // it would only mean "a slightly darker theme colour".
    if dim <= 0.0 || screen.wallpaper().is_none() {
        return None;
    }
    Some(
        container()
            .width(fill())
            .height(fill())
            .background(Color::rgba(0.0, 0.0, 0.0, dim)),
    )
}

/// The surface a card sits on: a translucent tint over a blurred cut-out of
/// whatever is behind it.
///
/// guido does the blur, and only the surface source is asked for: the wallpaper
/// is drawn by allock inside its own surface, while a lock surface has nothing
/// composited underneath for `ext-background-effect-v1` to reach.
pub fn card_surface(screen: &Screen) -> Container {
    let theme = screen.theme;
    let has_wallpaper = screen.wallpaper().is_some();

    // With no wallpaper there is nothing behind the card to reveal, so it
    // stays opaque rather than showing the flat background colour through
    // itself.
    let opacity = if has_wallpaper {
        theme.opacity * screen.config.background.card_opacity.clamp(0.0, 1.0)
    } else {
        theme.opacity
    };

    let card = container()
        .overflow(Overflow::Hidden)
        .layout(ZStack::new())
        .background(with_alpha(theme.background_weak, opacity));

    let blur = screen.config.background.blur;
    if has_wallpaper && blur > 0.0 {
        card.backdrop_blur(BackdropBlur::new(blur).sources(BackdropSources::SURFACE))
    } else {
        card
    }
}
