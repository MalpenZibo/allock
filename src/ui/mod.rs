//! One screen per output: the wallpaper, the big clock and the island. Every
//! output is covered; only those `main_output` selects get the clock and the
//! card, the others keep the island at rest. All of them share the signals.

mod background;
mod card;
mod clock;
mod field;
pub mod flow;
mod island;
mod rest;
#[cfg(test)]
mod snapshots;
pub mod theme;
mod tick;

use std::rc::Rc;

use chrono::{DateTime, Local};
use guido::prelude::*;

use crate::Mode;
use crate::config::{Config, SessionDef};
use crate::services::auth::Auth;
use crate::services::battery::Battery;
use crate::services::power::Power;
use crate::services::snapshot::Snapshot;
use crate::system::user::UserInfo;
use ashell_ui::Theme;
use flow::{Flow, Phase};

/// Whether the seed is handed over across the compositor's cut. When
/// nobody draws it on the other side, the island arrives and leaves by
/// itself instead of as a lone dot.
#[derive(Clone, Copy)]
pub struct Handover {
    /// A bar folded into the seed before the lock was drawn.
    pub arrives: Signal<bool>,
    /// Something will draw the seed once this screen is gone.
    pub leaves: Signal<bool>,
}

impl Handover {
    pub fn none() -> Self {
        let no = create_signal(false).read_only();
        Self {
            arrives: no,
            leaves: no,
        }
    }
}

pub type Snapshots = std::sync::Arc<std::collections::HashMap<String, Snapshot>>;

/// Everything a screen reads. Cheap to clone.
#[derive(Clone)]
pub struct Screen {
    pub config: Rc<Config>,
    pub theme: Theme,
    /// Behind the wallpaper, and instead of it.
    pub background: Color,
    pub mode: Mode,
    pub flow: Flow,
    pub auth: Auth,
    pub battery: Signal<Option<Battery>>,
    pub power: Power,
    pub now: Signal<DateTime<Local>>,
    pub host: Rc<str>,
    /// The session as it was when the lock was asked for, by output name.
    /// Empty unless the background is a snapshot.
    pub snapshots: Snapshots,
    /// Who is signing in. Greeter: follows the picker.
    pub user: RwSignal<Option<UserInfo>>,
    /// Greeter only. "Other…" sits after these, at `users.len()`.
    pub users: Rc<Vec<UserInfo>>,
    pub user_choice: RwSignal<usize>,
    pub handover: Handover,
    /// Greeter only.
    pub sessions: Rc<Vec<SessionDef>>,
    pub session: RwSignal<usize>,
}

pub fn view(screen: Screen, output: OutputInfo) -> Container {
    // ashell's components read it from here.
    ashell_ui::provide_theme(screen.theme);
    let controls = shows_controls(&screen, output.id);
    let height = create_signal(0.0f32);
    let root_ref = create_widget_ref();
    create_effect(move || height.set(root_ref.layout_rect().get().height));

    let root = container()
        .width(fill())
        .height(fill())
        .widget_ref(root_ref)
        .background(screen.background)
        .layout(ZStack::new())
        .child(background::view(&screen, &output))
        .child({
            let screen = screen.clone();
            move || controls.get().then(|| clock::view(&screen, height.into()))
        })
        .child(island::view(&screen, controls.into_signal(), height.into()))
        .child({
            let screen = screen.clone();
            move || controls.get().then(|| field::keyboard(&screen))
        });

    attach_keys(root, &screen)
}

/// Whether this output draws the clock and the card. A memo over the whole
/// output list, so unplugging the chosen monitor moves them to its stand-in.
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

/// The keys the field declines. Escape empties the field, then rests the
/// island; in the greeter it restarts the conversation instead, the only way
/// out of a PAM stage that is not waiting on the keyboard.
fn attach_keys(root: Container, screen: &Screen) -> Container {
    let (auth, flow, mode) = (screen.auth, screen.flow, screen.mode);
    root.on_key_down(move |key, modifiers, _repeat| {
        match flow.phase.get_untracked() {
            Phase::Rest => {
                flow.wake();
                return EventResponse::Handled;
            }
            Phase::Awake => {}
            _ => return EventResponse::Ignored,
        }
        match key {
            Key::Escape if !auth.state.field_is_empty_untracked() => auth.state.clear_field(),
            Key::Escape if flow.zone.get_untracked().is_some() => flow.zone.set(None),
            Key::Escape => match mode {
                Mode::Greeter => auth.restart(),
                Mode::Lock | Mode::Preview => flow.rest(),
            },
            Key::Char('u') if modifiers.ctrl => auth.state.clear_field(),
            _ => return EventResponse::Ignored,
        }
        EventResponse::Handled
    })
}
