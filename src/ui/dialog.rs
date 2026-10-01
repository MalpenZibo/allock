//! The polkit dialog: the lock screen's field, asked on a program's behalf.

use std::rc::Rc;

use chrono::Local;
use guido::prelude::*;

use super::tokens::{self, Typography, icon_size, space};
use super::{Screen, form};
use crate::Mode;
use crate::agent::{Outcome, Request};
use crate::auth::{self, Backend, NoticeLevel};
use crate::components::icons::{IconKind, icon};
use crate::config::Config;
use crate::theme::ThemeColors;

/// nf-md-shield_lock, checked against SymbolsNerdFont-Regular's cmap.
const SHIELD_GLYPH: &str = "\u{f099d}";
const SHIELD_SIZE: f32 = icon_size::I32;

const DIM: f32 = 0.45;

/// Build the dialog for `request`, which ends by calling `finish` once. The
/// helper conversation lives in the surface's scope and dies with it.
pub fn view(
    config: Rc<Config>,
    theme: ThemeColors,
    request: Request,
    finish: impl Fn(Outcome) + Clone + 'static,
) -> Container {
    let auth = auth::start(
        Backend::Polkit {
            user: request.user.clone(),
            cookie: request.cookie.clone(),
        },
        // pam_fprintd, if any, runs inside polkit's helper.
        None,
    );

    let screen = Screen {
        theme,
        auth,
        mode: Mode::Polkit,
        user: create_signal(crate::user::by_name(&request.user)),
        battery: None,
        resting: create_signal(false).into(),
        sessions: Rc::new(Vec::new()),
        session: super::picker::Choice::new(0),
        users: Rc::new(Vec::new()),
        user_choice: super::picker::Choice::new(0),
        now: create_signal(Local::now()).into(),
        config,
    };

    create_effect({
        let finish = finish.clone();
        move || {
            if auth.state.authenticated.get() {
                finish(Outcome::Authenticated);
            }
        }
    });

    let content = container()
        .padding([space::S24, space::S32, space::S16, space::S32])
        .layout(Flex::column().spacing(space::S16))
        .child(header(&screen, &request.message))
        .child(form::field(&screen))
        .child(notice(&screen));

    let root = container()
        .width(fill())
        .height(fill())
        .background(Color::rgba(0.0, 0.0, 0.0, DIM))
        .layout(Flex::column().center())
        .child(form::card(&screen, content));

    attach_keys(root, &screen, move || finish(Outcome::Dismissed))
}

fn header(screen: &Screen, message: &str) -> Container {
    let theme = screen.theme;
    container()
        .width(form::FIELD_WIDTH)
        .layout(
            Flex::row()
                .spacing(space::S16)
                .cross_alignment(CrossAlignment::Start),
        )
        .child(
            icon()
                .kind(IconKind::from(SHIELD_GLYPH))
                .font_size(SHIELD_SIZE)
                .color(theme.primary),
        )
        .child(
            container()
                .width(fill())
                .layout(Flex::column().spacing(space::S4))
                .child(
                    text("Authentication required")
                        .typography(tokens::TITLE_M)
                        .color(theme.text)
                        .nowrap(),
                )
                .child(
                    text(message.to_owned())
                        .typography(tokens::BODY_M)
                        .color(theme.text_weak),
                ),
        )
}

/// The lines under the field. Unlike the lock screen's they wrap: pam_faillock
/// takes two lines to say an account is locked. The lock forecast goes first,
/// since polkit's stack asks for a finger before it reaches the lock.
fn notice(screen: &Screen) -> Container {
    let theme = screen.theme;
    let state = screen.auth.state;
    let line =
        move |words: String, color: Color| text(words).typography(tokens::BODY_S).color(color);

    container()
        .width(form::FIELD_WIDTH)
        .height(at_least(space::S16))
        .layout(Flex::column().spacing(space::S4))
        .children(move || {
            let notice = state.notice.get().filter(|n| !n.text.is_empty());
            // PAM's own refusal says it better than the forecast would.
            if let Some(n) = notice.as_ref().filter(|n| n.level == NoticeLevel::Error) {
                return vec![line(n.text.clone(), theme.danger)];
            }
            let mut lines: Vec<Text> = state
                .locked
                .get()
                .map(|locked| line(locked, theme.warning))
                .into_iter()
                .collect();
            if state.busy.get() {
                lines.push(line("Checking…".to_owned(), theme.text_weak));
            } else if let Some(n) = notice {
                lines.push(line(n.text, theme.text_weak));
            }
            lines
        })
}

/// Escape on an empty field cancels the dialog.
fn attach_keys(root: Container, screen: &Screen, dismiss: impl Fn() + 'static) -> Container {
    let state = screen.auth.state;
    root.on_key_down(move |key, modifiers| {
        if state.authenticated.get_untracked() {
            return;
        }
        match key {
            Key::Escape if state.field_is_empty_untracked() => dismiss(),
            Key::Escape => state.clear_field(),
            Key::Char('u') if modifiers.ctrl => state.clear_field(),
            _ => {}
        }
    })
}
