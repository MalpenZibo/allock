//! One screen per output: the wallpaper, the big clock and the island. Every
//! output is covered; only those `main_output` selects get the clock and the
//! card, the others keep the island at rest. All of them share the signals.

mod background;
mod card;
mod clock;
mod field;
pub mod flow;
mod icons;
mod island;
mod rest;
mod roll;
#[cfg(test)]
mod snapshots;
pub mod theme;

use std::rc::Rc;

use chrono::{DateTime, Local};
use guido::prelude::*;
use guido::widget_prelude::*;

use crate::Mode;
use crate::config::{Config, SessionDef};
use crate::services::auth::Auth;
use crate::services::battery::Battery;
use crate::services::power::Power;
use crate::system::user::UserInfo;
use flow::{Flow, Phase};
use theme::Theme;

/// Everything a screen reads. Cheap to clone.
#[derive(Clone)]
pub struct Screen {
    pub config: Rc<Config>,
    pub theme: Theme,
    pub mode: Mode,
    pub flow: Flow,
    pub auth: Auth,
    pub battery: Signal<Option<Battery>>,
    pub power: Power,
    pub now: Signal<DateTime<Local>>,
    pub host: Rc<str>,
    /// Who is signing in. Greeter: follows the picker.
    pub user: RwSignal<Option<UserInfo>>,
    /// Greeter only. "Other…" sits after these, at `users.len()`.
    pub users: Rc<Vec<UserInfo>>,
    pub user_choice: RwSignal<usize>,
    /// Greeter only.
    pub sessions: Rc<Vec<SessionDef>>,
    pub session: RwSignal<usize>,
}

/// The island's spring, the one every fold and growth uses: ashell's island
/// moves on the same curve, so the handoff reads as one motion.
pub fn spring() -> SpringConfig {
    SpringConfig {
        mass: 1.0,
        stiffness: 230.0,
        damping: 21.0,
    }
}

pub fn view(screen: Screen, output: OutputInfo) -> Container {
    let controls = shows_controls(&screen, output.id);
    let height = create_signal(0.0f32);
    let root_ref = create_widget_ref();
    create_effect(move || height.set(root_ref.layout_rect().get().height));

    let root = container()
        .width(fill())
        .height(fill())
        .widget_ref(root_ref)
        .background(screen.theme.background)
        .layout(ZStack::new())
        .child(background::view(&screen))
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

/// Lays its children out at their own size, whatever its own, centred
/// across and at the top: the island's shape springs to the size of what it
/// shows, and what it shows is never squeezed while it does.
struct Overhang;

impl Layout for Overhang {
    fn layout(
        &mut self,
        ctx: &mut LayoutCtx,
        children: &[WidgetId],
        constraints: Constraints,
        origin: (f32, f32),
    ) -> Size {
        let size = constraints.constrain(Size::new(0.0, 0.0));
        let free = Constraints::new(0.0, 0.0, f32::INFINITY, f32::INFINITY);
        for &child in children {
            let laid = ctx.layout_child(child, free).unwrap_or_default();
            let x = origin.0 + (size.width - laid.width) / 2.0;
            ctx.tree().set_origin(child, x, origin.1);
        }
        ctx.set_minimum(Size::new(0.0, 0.0));
        size
    }
}
