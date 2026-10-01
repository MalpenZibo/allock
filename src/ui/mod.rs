//! The lock screen, one instance per output. Every output gets a surface (the
//! session-lock protocol requires it); only those selected by `main_output`
//! draw the interface, and all of them share the same signals.

pub mod clock;
pub mod dialog;
pub mod form;
pub mod picker;
pub mod status;
pub mod tokens;

use std::rc::Rc;

use chrono::{DateTime, Local};
use guido::prelude::*;

use crate::Mode;
use crate::auth::Auth;
use crate::config::{BackgroundFit, Config, SessionDef};
use crate::services::upower::UPowerService;
use crate::theme::{ThemeColors, with_alpha};
use crate::user::UserInfo;

/// Everything a lock surface needs. Cheap to clone.
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
    /// Idle for `[auth] idle_timeout`; never set in the greeter.
    pub resting: Signal<bool>,
    pub sessions: Rc<Vec<SessionDef>>,
    pub session: picker::Choice,
    /// Greeter only. The picker appends an "Other…" entry after these.
    pub users: Rc<Vec<UserInfo>>,
    pub user_choice: picker::Choice,
    pub now: Signal<DateTime<Local>>,
}

impl Screen {
    /// The polkit dialog sits over the session and has no wallpaper.
    fn wallpaper(&self) -> Option<std::path::PathBuf> {
        match self.mode {
            Mode::Polkit => None,
            _ => self.config.background.resolved_path(),
        }
    }
}

pub fn lock_screen(screen: Screen, output: OutputInfo) -> Container {
    let theme = screen.theme;
    let shows_controls = shows_controls(&screen, output.id);

    let centre = container()
        .width(fill())
        .height(fill())
        .layout(Flex::column().center())
        .child({
            let screen = screen.clone();
            move || {
                shows_controls.get().then(|| {
                    container()
                        .layout(
                            Flex::column()
                                .spacing(tokens::space::S40)
                                .cross_alignment(CrossAlignment::Center),
                        )
                        .child(clock::view(&screen))
                        .child(form::view(screen.clone()))
                })
            }
        });

    let bottom = container()
        .width(fill())
        .height(fill())
        .padding(tokens::space::S24)
        .layout(Flex::column().main_alignment(MainAlignment::End))
        .child({
            let screen = screen.clone();
            move || shows_controls.get().then(|| status::view(screen.clone()))
        });

    let root = container()
        .width(fill())
        .height(fill())
        // Under the wallpaper, so a missing or loading image never shows a
        // transparent surface.
        .background(theme.background)
        // The greeter's pickers are clicked; the lock screen is only typed at.
        .cursor(match screen.mode {
            Mode::Greeter | Mode::Polkit => CursorIcon::Default,
            Mode::Lock | Mode::Preview => CursorIcon::Hidden,
        })
        .layout(ZStack::new())
        .child(background_image(&screen))
        .child(scrim(&screen))
        .child(centre)
        .child(bottom);

    attach_keys(root, screen)
}

/// Whether this output draws the interface. A memo over the whole output list,
/// so unplugging the configured monitor moves the controls to its stand-in.
fn shows_controls(screen: &Screen, output: OutputId) -> Memo<bool> {
    let selector = screen.config.main_output.clone();
    create_memo(move || {
        let connected = outputs().get();
        let Some(index) = connected.iter().position(|info| info.id == output) else {
            return false;
        };
        let described: Vec<(&str, &str, &str)> = connected
            .iter()
            .map(|info| {
                (
                    info.name.as_deref().unwrap_or(""),
                    info.make.as_str(),
                    info.model.as_str(),
                )
            })
            .collect();
        selector.selects(index, &described)
    })
}

/// Keys the `text_input` declines. Escape on an empty field restarts the
/// conversation: the only way out of a PAM stage not waiting on the keyboard.
fn attach_keys(root: Container, screen: Screen) -> Container {
    let auth = screen.auth;
    root.on_key_down(move |key, modifiers| {
        if auth.state.authenticated.get_untracked() {
            return;
        }
        match key {
            Key::Escape => {
                if auth.state.field_is_empty_untracked() {
                    auth.restart();
                } else {
                    auth.state.clear_field();
                }
            }
            Key::Char('u') if modifiers.ctrl => auth.state.clear_field(),
            _ => {}
        }
    })
}

const WALLPAPER_FADE_MS: f32 = 400.0;

fn background_image(screen: &Screen) -> Option<Container> {
    let source = ImageSource::Path(screen.wallpaper()?);
    let fit = match screen.config.background.fit {
        BackgroundFit::Cover => ContentFit::Cover,
        BackgroundFit::Contain => ContentFit::Contain,
        BackgroundFit::Stretch => ContentFit::Fill,
    };

    // Decoded off the frame, so the first frame goes out without it instead of
    // leaving the compositor's "locker has not drawn" colour up (~140 ms for a 3K PNG).
    let wallpaper = image(source).content_fit(fit);
    let ready = wallpaper.ready();

    // `Contain` is letterboxed by this container's alignment.
    Some(
        container()
            .width(fill())
            .height(fill())
            .layout(Flex::column().center())
            .opacity(
                (move || if ready.get() { 1.0 } else { 0.0 })
                    .transition(Transition::new(WALLPAPER_FADE_MS, TimingFunction::EaseOut)),
            )
            .child(wallpaper),
    )
}

fn scrim(screen: &Screen) -> Option<Container> {
    let dim = screen.config.background.dim.clamp(0.0, 1.0);
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

/// A translucent tint over a blurred cut-out of what is behind the card. A lock
/// surface has nothing composited underneath, so it blurs its own surface.
pub fn card_surface(screen: &Screen) -> Container {
    let theme = screen.theme;

    // The polkit dialog floats over the session, so the compositor does the blur.
    if screen.mode == Mode::Polkit {
        let card = container()
            .overflow(Overflow::Hidden)
            .layout(ZStack::new())
            .background(with_alpha(
                theme.background_weak,
                theme.opacity * screen.config.background.card_opacity.clamp(0.0, 1.0),
            ));
        return if screen.config.background.blur > 0.0 {
            card.backdrop_blur(BackdropBlur::new(0.0).sources(BackdropSources::COMPOSITOR))
        } else {
            card
        };
    }

    let has_wallpaper = screen.wallpaper().is_some();

    // With no wallpaper there is nothing to reveal, so the card stays opaque.
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
