//! allock: a session lock over `ext-session-lock-v1` and PAM, a greetd greeter
//! (`--greeter`) and a polkit agent (`--polkit`) in one binary.
//!
//! The layout is fixed; the config only chooses colours, wallpaper, clock
//! format and indicators.

mod agent;
mod auth;
mod components;
mod config;
mod faillock;
mod idle;
mod last_login;
mod services;
mod theme;
mod ui;
mod user;
mod utils;

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::Parser;
use guido::prelude::*;

use crate::auth::{Backend, FingerprintConfig};
use crate::config::{Config, SessionDef, SessionOutput};
use crate::services::compat::run_readonly_service;
use crate::services::upower::UPowerService;
use crate::ui::Screen;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Lock,
    Greeter,
    /// The lock screen with the real PAM conversation but without taking the
    /// lock, so a theme can be tried without risking a locked-out session.
    Preview,
    Polkit,
}

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Lock screen, greetd greeter and polkit agent for Wayland"
)]
struct Args {
    /// Path to the config file (default `~/.config/allock/config.toml`)
    #[arg(short, long)]
    config_path: Option<String>,

    /// Run as a greetd greeter instead of a session lock.
    #[arg(long, conflicts_with_all = ["preview", "polkit"])]
    greeter: bool,

    /// Draw the lock screen without locking; authenticating exits instead.
    #[arg(long, conflicts_with = "polkit")]
    preview: bool,

    /// Run as the session's polkit authentication agent, until killed.
    #[arg(long)]
    polkit: bool,
}

/// Set when the compositor refuses the lock, so a failed lock does not exit like an unlock.
static LOCK_REFUSED: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // No core dump and no non-root ptrace: copies of the password outside
    // guido's locked pages (PAM's `CString`, key strings) are plain memory.
    // Failure is only reported; it is no reason to leave the screen unlocked.
    let undumpable = match unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    };
    let mode = match (args.greeter, args.preview, args.polkit) {
        (true, _, _) => Mode::Greeter,
        (_, true, _) => Mode::Preview,
        (_, _, true) => Mode::Polkit,
        _ => Mode::Lock,
    };

    let config_path = args
        .config_path
        .map(|path| config::expand_tilde(&path))
        .unwrap_or_else(config::default_config_path);
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(config::log_level(&config_path)),
    )
    .init();
    log::info!("starting allock in {mode:?} mode");
    if let Err(e) = undumpable {
        log::warn!("could not mark the process undumpable: {e}");
    }

    let config = config::load(&config_path);

    // Resolved before locking: without a usable policy the lock could never be undone.
    let pam_service = match mode {
        Mode::Lock | Mode::Preview => match auth::pam::resolve_service(&config.auth.pam_service) {
            Some(service) => service,
            None => {
                eprintln!(
                    "allock: no PAM policy for service \"{}\", and no stock stack to fall back \
                     on.\nInstall pam/allock as /etc/pam.d/{} before locking — without a policy \
                     PAM uses `other`, which rejects every password.",
                    config.auth.pam_service, config.auth.pam_service
                );
                std::process::exit(1);
            }
        },
        // greetd and polkit's helper run PAM on their own side.
        Mode::Greeter | Mode::Polkit => String::new(),
    };

    let theme = theme::init(&config.appearance);

    let sessions: Rc<Vec<SessionDef>> = Rc::new(match mode {
        Mode::Greeter if config.greeter.sessions.is_empty() => config::scan_sessions(),
        Mode::Greeter => config.greeter.sessions.clone(),
        Mode::Lock | Mode::Preview | Mode::Polkit => Vec::new(),
    });
    if mode == Mode::Greeter && sessions.is_empty() {
        log::error!(
            "no sessions found: add [[greeter.session]] entries to {config_path:?} or install \
             desktop files under /usr/share/wayland-sessions"
        );
    }

    if config.background.path.is_some() && config.background.resolved_path().is_none() {
        log::warn!(
            "background.path is set but unreadable; falling back to the appearance background \
             colour"
        );
    }

    let mut app = App::new();
    if let Some(family) = &config.appearance.font_name {
        app = app.default_font_family(FontFamily::name(family));
    }
    // The agent has no surface between dialogs.
    if mode == Mode::Polkit {
        app = app.quit_on_last_surface(false);
    }

    let config = Rc::new(config);
    let reason = app.run(|_app| match mode {
        Mode::Lock | Mode::Preview => {
            run_pam_screen(config.clone(), theme, sessions.clone(), pam_service, mode)
        }
        Mode::Greeter => run_greeter(config.clone(), theme, sessions.clone()),
        Mode::Polkit => agent::run(config.clone(), theme),
    });

    match reason {
        ExitReason::Error(e) => {
            log::error!("platform error: {e}");
            std::process::exit(1);
        }
        _ if LOCK_REFUSED.load(Ordering::SeqCst) => std::process::exit(1),
        _ if agent::REGISTRATION_FAILED.load(Ordering::SeqCst) => std::process::exit(1),
        _ => {}
    }
}

/// The PAM-backed screen: `Mode::Lock` takes the session lock, `Mode::Preview`
/// draws it on an overlay surface and exits once the password is accepted.
fn run_pam_screen(
    config: Rc<Config>,
    theme: theme::ThemeColors,
    sessions: Rc<Vec<SessionDef>>,
    // Resolved against what is installed, unlike config.auth.pam_service.
    pam_service: String,
    mode: Mode,
) {
    let user = user::current();
    log::info!("authenticating as {}", user.name);

    // Form and reader rest together, so the screen never invites a finger the
    // reader is not listening for.
    let idle = (config.auth.idle_timeout > 0)
        .then(|| Duration::from_secs(config.auth.idle_timeout))
        .and_then(idle::watch);

    let fingerprint = config.auth.fingerprint.then(|| FingerprintConfig {
        user: user.name.clone(),
        retries: config.auth.fingerprint_retries,
        idle: idle.clone(),
    });

    let auth = auth::start(
        Backend::Pam {
            service: pam_service,
            user: user.name.clone(),
        },
        fingerprint,
    );

    let screen = Screen {
        theme,
        auth,
        mode,
        user: create_signal(Some(user)),
        battery: battery_service(&config),
        resting: idle::resting(idle),
        sessions,
        session: ui::picker::Choice::new(0),
        users: Rc::new(Vec::new()),
        user_choice: ui::picker::Choice::new(0),
        now: ui::clock::create(),
        config,
    };

    // Half a password is not left in a field nobody is watching.
    let resting = screen.resting;
    create_effect(move || {
        if resting.get() {
            auth.state.clear_field();
        }
    });

    if mode == Mode::Preview {
        log::warn!("preview mode: the session is NOT locked");
        // Never Exclusive: a preview must not swallow the session's keyboard.
        spawn_fullscreen_surfaces(screen, KeyboardInteractivity::OnDemand);
        create_effect(move || {
            if auth.state.authenticated.get() {
                log::info!("authenticated; preview has nothing to unlock, exiting");
                quit_app();
            }
        });
        return;
    }

    // guido covers every output, including ones plugged in while locked.
    lock_session(move |output| ui::lock_screen(screen.clone(), output));

    create_effect(move || {
        if auth.state.authenticated.get() {
            log::info!("authenticated, unlocking");
            unlock_session();
        }
    });

    watch_lock_lifecycle();
}

/// Exit on the `Locked → Unlocked` edge only: `unlock_session()` is a request,
/// and exiting before the compositor processes it can leave the session locked.
fn watch_lock_lifecycle() {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Progress {
        Pending,
        Requested,
        Held,
    }

    let progress = create_signal(Progress::Pending);

    create_effect(move || {
        let state = lock_state().get();
        match (progress.get_untracked(), state) {
            (_, LockState::Locking) => progress.set(Progress::Requested),
            (_, LockState::Locked) => progress.set(Progress::Held),
            (Progress::Held, LockState::Unlocked) => {
                log::info!("session unlocked");
                quit_app();
            }
            (Progress::Requested, LockState::Unlocked) => {
                // Another client holds the lock, or there is no ext-session-lock-v1.
                log::error!("the compositor refused the session lock");
                LOCK_REFUSED.store(true, Ordering::SeqCst);
                quit_app();
            }
            (Progress::Pending, LockState::Unlocked) => {}
        }
    });
}

fn run_greeter(config: Rc<Config>, theme: theme::ThemeColors, sessions: Rc<Vec<SessionDef>>) {
    let configured_user = config
        .greeter
        .user
        .clone()
        .filter(|name| !name.trim().is_empty());

    let remembered = if config.greeter.remember {
        last_login::load()
    } else {
        last_login::LastLogin::default()
    };

    // "Other…" sits at `users.len()` and asks for the name in the form. The
    // preselection is only a suggestion, so remembering never locks anyone out.
    let users = Rc::new(user::login_users());
    let preselected_user = [configured_user.as_deref(), remembered.user.as_deref()]
        .into_iter()
        .flatten()
        .find_map(|name| users.iter().position(|user| user.name == name))
        .unwrap_or(0);
    let chosen = |index: usize| users.get(index).cloned();

    // No fingerprint: greetd runs PAM itself and would not accept a local match.
    let auth = auth::start(
        Backend::Greetd {
            user: chosen(preselected_user).map(|user| user.name),
        },
        None,
    );

    let screen = Screen {
        theme,
        auth,
        mode: Mode::Greeter,
        user: create_signal(chosen(preselected_user)),
        battery: battery_service(&config),
        // Nothing to rest: the greeter has no reader to switch off.
        resting: idle::resting(None),
        sessions: sessions.clone(),
        session: ui::picker::Choice::new(last_login::preselect(
            &sessions,
            config.greeter.default_session.as_deref(),
            &remembered,
        )),
        users: users.clone(),
        user_choice: ui::picker::Choice::new(preselected_user),
        now: ui::clock::create(),
        config: config.clone(),
    };

    // A step in the user picker restarts the conversation; the first run is
    // the preselection greetd was started with.
    let user_signal = screen.user;
    let user_index = screen.user_choice.index;
    let asked_for = std::cell::Cell::new(preselected_user);
    create_effect({
        let users = users.clone();
        move || {
            let index = user_index.get();
            if asked_for.replace(index) == index {
                return;
            }
            let user = users.get(index).cloned();
            auth.set_user(user.as_ref().map(|user| user.name.clone()));
            user_signal.set(user);
        }
    });

    // Under "Other…" the name is typed; once it is, show who is signing in.
    create_effect(move || {
        if let Some(name) = auth.state.visible_answer.get() {
            user_signal.set(user::by_name(&name));
        }
    });

    let session_index = screen.session.index;
    let remember = config.greeter.remember;
    let systemd_cat = match config.greeter.session_output {
        SessionOutput::Journal => find_in_path("systemd-cat"),
        SessionOutput::Console => None,
    };
    create_effect(move || {
        if !auth.state.authenticated.get() {
            return;
        }
        if sessions.is_empty() {
            log::error!("authenticated but there is no session to start");
            return;
        }
        // The picker wraps already; the modulo guards a list that shrank.
        let session = &sessions[session_index.get_untracked() % sessions.len()];
        log::info!("starting session {}", session.name);
        if remember {
            // Before the request: once greetd accepts it this process exits.
            let user = users
                .get(user_index.get_untracked())
                .map(|user| user.name.clone())
                .or_else(|| auth.state.visible_answer.get_untracked());
            last_login::save(user.as_deref(), &session.name);
        }
        auth.start_session(session.argv(systemd_cat.as_deref()), session.env.clone());
    });

    // greetd starts the session only once the greeter has exited (`greetd-ipc(7)`).
    create_effect(move || {
        if auth.state.session_accepted.get() {
            log::info!("greetd accepted the session; exiting so it can start");
            quit_app();
        }
    });

    spawn_fullscreen_surfaces(screen, KeyboardInteractivity::Exclusive);
}

/// One fullscreen overlay per output, following hotplug.
fn spawn_fullscreen_surfaces(screen: Screen, keyboard: KeyboardInteractivity) {
    let handles: Rc<std::cell::RefCell<std::collections::HashMap<OutputId, SurfaceHandle>>> =
        Default::default();

    create_effect(move || {
        let outputs = outputs().get();
        let mut handles = handles.borrow_mut();

        for output in &outputs {
            if handles.contains_key(&output.id) {
                continue;
            }
            let screen = screen.clone();
            let info = output.clone();
            let handle = spawn_surface(
                SurfaceConfig::new()
                    // Zero size with every edge anchored is the whole output.
                    .width(0)
                    .height(0)
                    .anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT)
                    .layer(Layer::Overlay)
                    .exclusive_zone(ExclusiveZone::Ignore)
                    .keyboard_interactivity(keyboard)
                    .namespace("allock")
                    .output(output.id)
                    .background_color(screen.theme.background),
                move || ui::lock_screen(screen.clone(), info.clone()),
            );
            handles.insert(output.id, handle);
        }

        handles.retain(|id, handle| {
            let connected = outputs.iter().any(|output| output.id == *id);
            if !connected {
                handle.close();
            }
            connected
        });
    });
}

fn battery_service(config: &Config) -> Option<RwSignal<Option<UPowerService>>> {
    config
        .indicators
        .battery
        .then(run_readonly_service::<UPowerService>)
}

/// The first executable called `name` on `$PATH`.
fn find_in_path(name: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join(name))
        .find(|candidate| {
            candidate
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}
