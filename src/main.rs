//! allock — a lock screen and greetd greeter.
//!
//! One binary, two modes:
//!
//! - **lock** (default): takes an `ext-session-lock-v1` lock, puts a surface
//!   on every output, authenticates through PAM, and exits once the
//!   compositor confirms the session is unlocked.
//! - **greeter** (`--greeter`): a fullscreen overlay layer surface per output
//!   that authenticates through greetd and starts the chosen session.
//!
//! The look is not configurable the way hyprlock's is — the layout is decided
//! here and the config only chooses colours, wallpaper, clock format and
//! which indicators appear.

mod auth;
mod components;
mod config;
mod last_login;
mod services;
mod theme;
mod ui;
mod user;
mod utils;

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// The lock screen, drawn without taking the lock.
    ///
    /// Developing a lock screen otherwise means locking yourself out of the
    /// session you are developing in, and a lock client that dies while
    /// holding the lock leaves the compositor locked with nobody to unlock
    /// it — the protocol guarantees that on purpose. Preview keeps the real
    /// PAM conversation so prompts and failures are the real ones, and takes
    /// keyboard focus on demand rather than exclusively, so it can never trap
    /// the keyboard. It exits on a successful password instead of unlocking.
    Preview,
}

#[derive(Parser, Debug)]
#[command(version, about = "Lock screen and greetd greeter for Wayland")]
struct Args {
    /// Path to the config file (default `~/.config/allock/config.toml`)
    #[arg(short, long)]
    config_path: Option<String>,

    /// Run as a greetd greeter instead of a session lock.
    #[arg(long, conflicts_with = "preview")]
    greeter: bool,

    /// Draw the lock screen without locking the session, for trying out a
    /// theme. Authenticating exits instead of unlocking; kill it any time.
    #[arg(long)]
    preview: bool,
}

/// Set when the compositor refuses the lock, so the exit code says so. A
/// process that failed to lock must not look like one that unlocked.
static LOCK_REFUSED: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Before anything is typed: no core dump, and no ptrace attach short of
    // root. guido keeps a password in pages that are locked and excluded from
    // dumps, but copies outside it — pam-client2's `CString`, the keyboard's
    // one-character strings, the clipboard — are plain memory, and a crash
    // under systemd-coredump would write them to disk. Failing to set it is
    // not a reason to leave the screen unlocked, so it is only reported.
    let undumpable = match unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    };
    let mode = match (args.greeter, args.preview) {
        (true, _) => Mode::Greeter,
        (_, true) => Mode::Preview,
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

    // Resolved before anything is drawn: locking first and discovering there
    // is no usable policy means a screen that can never be unlocked.
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
        // greetd runs the PAM stack on its side of the socket.
        Mode::Greeter => String::new(),
    };

    let theme = theme::init(&config.appearance);

    // Sessions are only meaningful in greeter mode, and scanning the desktop
    // entries is disk work the lock screen has no use for.
    let sessions: Rc<Vec<SessionDef>> = Rc::new(match mode {
        Mode::Greeter if config.greeter.sessions.is_empty() => config::scan_sessions(),
        Mode::Greeter => config.greeter.sessions.clone(),
        Mode::Lock | Mode::Preview => Vec::new(),
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

    // The nerd font the icons ask for by name is a system font, and guido's
    // font system already has those; only a family that overrides the default
    // has to be declared. Icons render as tofu without the font installed,
    // which is ugly but never blocks logging in.
    let mut app = App::new();
    if let Some(family) = &config.appearance.font_name {
        app = app.default_font_family(FontFamily::name(family));
    }

    let config = Rc::new(config);
    // `app` goes unused inside the closure: every mode spawns its surfaces from
    // an `outputs()` effect (or from the lock factory) rather than declaring
    // them up front.
    let reason = app.run(|_app| match mode {
        Mode::Lock | Mode::Preview => {
            run_pam_screen(config.clone(), theme, sessions.clone(), pam_service, mode)
        }
        Mode::Greeter => run_greeter(config.clone(), theme, sessions.clone()),
    });

    match reason {
        ExitReason::Error(e) => {
            log::error!("platform error: {e}");
            std::process::exit(1);
        }
        _ if LOCK_REFUSED.load(Ordering::SeqCst) => std::process::exit(1),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Lock and preview modes
// ---------------------------------------------------------------------------

/// The PAM-backed screen. `Mode::Lock` takes the session lock; `Mode::Preview`
/// draws the same thing on an ordinary overlay surface and exits when the
/// password is accepted, which is the only way to iterate on the look without
/// locking the session you are working in.
fn run_pam_screen(
    config: Rc<Config>,
    theme: theme::ThemeColors,
    sessions: Rc<Vec<SessionDef>>,
    // Already resolved against what is installed — not config.auth.pam_service,
    // which may name a policy that does not exist.
    pam_service: String,
    mode: Mode,
) {
    let user = user::current();
    log::info!("authenticating as {}", user.name);

    let fingerprint = config.auth.fingerprint.then(|| FingerprintConfig {
        user: user.name.clone(),
        retries: config.auth.fingerprint_retries,
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
        sessions,
        session: ui::picker::Choice::new(0),
        users: Rc::new(Vec::new()),
        user_choice: ui::picker::Choice::new(0),
        now: ui::clock::create(),
        config,
    };

    if mode == Mode::Preview {
        log::warn!("preview mode: the session is NOT locked");
        // OnDemand, never Exclusive: a preview must not be able to swallow
        // the keyboard of the session it is being previewed in.
        spawn_fullscreen_surfaces(screen, KeyboardInteractivity::OnDemand);
        create_effect(move || {
            if auth.state.authenticated.get() {
                log::info!("authenticated; preview has nothing to unlock, exiting");
                quit_app();
            }
        });
        return;
    }

    // Ask the compositor for the lock. One surface per output is guido's job
    // from here, including monitors plugged in while locked.
    lock_session(move |output| ui::lock_screen(screen.clone(), output));

    create_effect(move || {
        if auth.state.authenticated.get() {
            log::info!("authenticated, unlocking");
            unlock_session();
        }
    });

    watch_lock_lifecycle();
}

/// Exit when the lock is over, and only then.
///
/// `unlock_session()` is a request: the process has to stay alive until the
/// compositor has processed it, or the session can be left locked with no
/// client to unlock it. Quitting on the `Locked → Unlocked` edge is what
/// guarantees the round trip completed.
fn watch_lock_lifecycle() {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Progress {
        /// Before the compositor has answered at all.
        Pending,
        /// The lock was requested.
        Requested,
        /// The lock was granted — this is the state we must leave from.
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
                // Another lock client already holds it, or the compositor has
                // no ext-session-lock-v1. Either way nothing was locked.
                log::error!("the compositor refused the session lock");
                LOCK_REFUSED.store(true, Ordering::SeqCst);
                quit_app();
            }
            (Progress::Pending, LockState::Unlocked) => {}
        }
    });
}

// ---------------------------------------------------------------------------
// Greeter mode
// ---------------------------------------------------------------------------

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

    // The user picker: the accounts people log in to, then — unless turned
    // off — "Other…" at `users.len()`, which asks for the name in the form. It
    // starts on the configured account, else the remembered one, else the
    // first: a suggestion the arrows undo, so remembering can never keep
    // somebody else from signing in. With nobody listed the form asks.
    let users = Rc::new(user::login_users());
    let preselected_user = [configured_user.as_deref(), remembered.user.as_deref()]
        .into_iter()
        .flatten()
        .find_map(|name| users.iter().position(|user| user.name == name))
        .unwrap_or(0);
    let chosen = |index: usize| users.get(index).cloned();

    // greetd runs the PAM stack itself, so there is no second credential to
    // check locally — fingerprint verification would prove identity without
    // producing a session greetd would accept.
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

    // A step in the user picker starts the conversation over for that
    // account. The first run is the preselection greetd was started with.
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

    // Authenticated: hand greetd the session to launch.
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
            // Before the request, not after: once greetd accepts it this
            // process exits, and a write racing that loses.
            let user = users
                .get(user_index.get_untracked())
                .map(|user| user.name.clone())
                .or_else(|| auth.state.visible_answer.get_untracked());
            last_login::save(user.as_deref(), &session.name);
        }
        auth.start_session(session.argv(systemd_cat.as_deref()), session.env.clone());
    });

    // greetd starts the session only once the greeter has exited
    // (`greetd-ipc(7)`), so accepted means leave. The compositor hosting the
    // greeter has to quit with it — that is its config's job.
    create_effect(move || {
        if auth.state.session_accepted.get() {
            log::info!("greetd accepted the session; exiting so it can start");
            quit_app();
        }
    });

    // Exclusive: the greeter owns the machine, nothing else may take the
    // keyboard from it.
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
                    // Zero size with every edge anchored means "the whole
                    // output" in layer-shell terms.
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

// ---------------------------------------------------------------------------
// Shared setup
// ---------------------------------------------------------------------------

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
