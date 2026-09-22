//! The lock screen, one instance per output.
//!
//! Every output gets a surface — the session-lock protocol requires it — but
//! only the outputs selected by `main_output` draw the interface on top of it;
//! the rest show the background alone. All of them read the same signals, so
//! with more than one selected the form mirrors itself across screens:
//! whichever surface has keyboard focus drives the shared input buffer.
//!
//! The selection is a memo over the output list rather than a decision taken
//! when the surface is built, so unplugging the monitor named in the config
//! moves the controls to the one standing in for it instead of leaving them
//! on a screen that is no longer there.

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
    /// Which way the picker last stepped: `1` for next, `-1` for previous.
    /// Only the slide reads it — with two sessions both arrows land on the
    /// same index, so the index alone cannot say which way to move.
    pub session_step: RwSignal<isize>,
    pub now: Signal<DateTime<Local>>,
}

impl Screen {
    fn wallpaper(&self) -> Option<std::path::PathBuf> {
        self.config.background.resolved_path()
    }
}

pub fn lock_screen(screen: Screen, output: OutputInfo) -> Container {
    let theme = screen.theme;
    let shows_controls = shows_controls(&screen, output.id);

    let centre = container()
        .width(fill())
        .height(fill())
        .layout(
            Flex::column()
                .main_alignment(MainAlignment::Center)
                .cross_alignment(CrossAlignment::Center),
        )
        .child({
            let screen = screen.clone();
            move || {
                shows_controls.get().then(|| {
                    container()
                        .layout(
                            Flex::column()
                                .spacing(48.0)
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
        .padding(28.0)
        .layout(Flex::column().main_alignment(MainAlignment::End))
        .child({
            let screen = screen.clone();
            move || shows_controls.get().then(|| status::view(screen.clone()))
        });

    let root = container()
        .width(fill())
        .height(fill())
        // Painted under the wallpaper so a missing or half-loaded image never
        // shows through to a transparent surface.
        .background(theme.background)
        .layout(ZStack::new())
        .child(background_image(&screen))
        .child(scrim(&screen))
        .child(centre)
        .child(bottom);

    let root = if screen.mode == Mode::Greeter {
        show_pointer(root)
    } else {
        root
    };
    attach_keys(root, screen)
}

/// Give the pointer a shape when it arrives, so the greeter has a cursor.
///
/// A Wayland client that sets no shape on enter gets no cursor at all, and
/// guido only sends one when the shape *changes* — never on enter. That is
/// what keeps the lock screen cursorless, and it stays that way there. The
/// greeter wants one, and asking for `Default` alone would be dropped as no
/// change, so it goes through another shape first: both land in the same
/// pass, and only the last one reaches the compositor.
fn show_pointer(root: Container) -> Container {
    root.on_hover(|inside| {
        if inside {
            set_cursor(CursorIcon::Text);
            set_cursor(CursorIcon::Default);
        }
    })
}

/// Whether this output is one of the ones drawing the interface, as a tracked
/// read of the output list.
///
/// The list is what the rule is written against — "the first output stands in
/// when nothing named is connected" is a statement about all of them, not
/// about this one — so each surface finds its own place in the current list
/// and asks from there. Every surface asks the same question of the same list,
/// which is what makes exactly one of them the stand-in.
fn shows_controls(screen: &Screen, output: OutputId) -> Memo<bool> {
    let selector = screen.config.main_output.clone();
    create_memo(move || {
        let connected = outputs().get();
        let Some(index) = connected.iter().position(|info| info.id == output) else {
            // This surface's own output is gone; the surface is going with it.
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

/// The two keys the field does not want.
///
/// Everything about editing — characters, backspace, selection, Enter — belongs
/// to the `text_input`, which has the focus and answers first; what reaches here
/// is what it declined. These two are screen-level verbs rather than editing:
///
/// Escape clears what was typed, and pressing it again on an already empty field
/// restarts the conversation — the only way out of a PAM stage waiting on
/// something other than the keyboard. Ctrl+U is the readline habit for "clear the
/// line", which the input has no binding for.
fn attach_keys(root: Container, screen: Screen) -> Container {
    let auth = screen.auth;
    root.on_key_down(move |key, modifiers| {
        if auth.state.authenticated.get_untracked() {
            return;
        }
        match key {
            Key::Escape => {
                if auth.state.input.get_untracked().is_empty() {
                    auth.restart();
                } else {
                    auth.state.input.set(String::new());
                }
            }
            Key::Char('u') if modifiers.ctrl => auth.state.input.set(String::new()),
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
