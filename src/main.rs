//! allock: a session lock over `ext-session-lock-v1` and PAM, and a greetd
//! greeter (`--greeter`), drawn as an island that starts and ends as the seed
//! a bar folds its own island into.

mod config;
mod services;
mod system;
mod ui;

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::Parser;
use guido::prelude::*;

use crate::config::{Config, SessionDef, SessionOutput};
use crate::services::auth::{self, Auth, Backend, FingerprintConfig};
use crate::services::handoff::Handoff;
use crate::services::{battery, clock, idle, power::Power};
use crate::system::{last_login, user};
use crate::ui::Screen;
use crate::ui::flow::{Flow, Phase};
use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Lock,
    Greeter,
    /// The lock screen with the real PAM conversation but without taking the
    /// lock, so a theme can be tried without risking a locked-out session.
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

    /// Draw the lock screen without locking; authenticating exits instead.
    #[arg(long)]
    preview: bool,
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
        // greetd runs PAM on its own side.
        Mode::Greeter => String::new(),
    };

    if config.background.path.is_some() && config.background.resolved_path().is_none() {
        log::warn!("background.path is set but unreadable; using the background colour");
    }

    let mut app = App::new();
    if let Some(family) = &config.appearance.font_name {
        app = app.default_font_family(FontFamily::name(family));
    }

    let config = Rc::new(config);
    let reason = app.run(|_app| match mode {
        Mode::Lock | Mode::Preview => run_lock(config.clone(), pam_service.clone(), mode),
        Mode::Greeter => run_greeter(config.clone()),
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

/// The parts of a [`Screen`] every mode builds the same way.
fn screen(config: Rc<Config>, mode: Mode, auth: Auth, flow: Flow) -> Screen {
    Screen {
        theme: Theme::new(&config.appearance),
        mode,
        flow,
        auth,
        battery: if config.indicators.battery {
            battery::watch()
        } else {
            create_signal(None).read_only()
        },
        power: Power::start(),
        now: clock::now(),
        host: system::hostname().into(),
        user: create_signal(None),
        users: Rc::new(Vec::new()),
        user_choice: create_signal(0),
        sessions: Rc::new(Vec::new()),
        session: create_signal(0),
        config,
    }
}

/// The PAM-backed screen: `Mode::Lock` takes the session lock once the bar
/// has folded into the seed, `Mode::Preview` draws on overlay surfaces and
/// exits once the password is accepted.
fn run_lock(config: Rc<Config>, pam_service: String, mode: Mode) {
    let me = user::current();
    log::info!("authenticating as {}", me.name);

    // The reader listens only while the card is up: a sensor left scanning
    // warms up, and the island at rest does not ask for a finger.
    let (reader_rests, reader_rest_rx) = tokio::sync::watch::channel(true);
    let fingerprint = config.auth.fingerprint.then(|| FingerprintConfig {
        user: me.name.clone(),
        retries: config.auth.fingerprint_retries,
        idle: Some(reader_rest_rx),
    });
    let auth = auth::start(
        Backend::Pam {
            service: pam_service,
            user: me.name.clone(),
        },
        fingerprint,
    );

    let handoff = (mode == Mode::Lock).then(|| Handoff::start(config.handoff));
    let flow = Flow::start(auth, move || match mode {
        Mode::Lock => {
            log::info!("authenticated, unlocking");
            unlock_session();
        }
        _ => {
            log::info!("authenticated; preview has nothing to unlock, exiting");
            quit_app();
        }
    });

    create_effect(move || {
        let awake = matches!(flow.phase.get(), Phase::Awake);
        reader_rests.send_replace(!awake);
    });

    // Away from the keyboard: back to rest.
    let away = (config.auth.idle_timeout > 0)
        .then(|| Duration::from_secs(config.auth.idle_timeout))
        .and_then(idle::watch);
    let away = idle::resting(away);
    create_effect(move || {
        if away.get() {
            flow.rest();
        }
    });

    let screen = screen(config, mode, auth, flow);
    screen.user.set(Some(me));

    match handoff {
        None => {
            log::warn!("preview mode: the session is NOT locked");
            // Never Exclusive: a preview must not swallow the session's keyboard.
            spawn_fullscreen_surfaces(screen, KeyboardInteractivity::OnDemand);
            flow.grow();
        }
        Some(handoff) => {
            handoff.prepare();
            let locked = std::cell::Cell::new(false);
            create_effect(move || {
                // Once: the bar is the seed, or will not answer.
                if handoff.ready.get() && !locked.replace(true) {
                    let screen = screen.clone();
                    lock_session(move |output| ui::view(screen.clone(), output));
                }
            });
            watch_lock_lifecycle(handoff, flow);
        }
    }
}

/// Exit on the `Locked → Unlocked` edge only, after telling the bar it may
/// grow back: `unlock_session()` is a request, and exiting before the
/// compositor processes it can leave the session locked.
fn watch_lock_lifecycle(handoff: Handoff, flow: Flow) {
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
            (Progress::Held, LockState::Locked) => {}
            (_, LockState::Locked) => {
                progress.set(Progress::Held);
                handoff.locked();
                // The compositor shows the seed now, where the bar left its own.
                flow.grow();
            }
            (Progress::Held, LockState::Unlocked) => {
                log::info!("session unlocked");
                handoff.resume();
            }
            (Progress::Requested, LockState::Unlocked) => {
                // Another client holds the lock, or there is no ext-session-lock-v1.
                log::error!("the compositor refused the session lock");
                LOCK_REFUSED.store(true, Ordering::SeqCst);
                handoff.resume();
            }
            (Progress::Pending, LockState::Unlocked) => {}
        }
    });

    create_effect(move || {
        if handoff.resumed.get() {
            quit_app();
        }
    });
}

fn run_greeter(config: Rc<Config>) {
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
    let sessions: Rc<Vec<SessionDef>> = Rc::new(if config.greeter.sessions.is_empty() {
        config::scan_sessions()
    } else {
        config.greeter.sessions.clone()
    });
    if sessions.is_empty() {
        log::error!(
            "no sessions found: add [[greeter.session]] entries to the config or install \
             desktop files under /usr/share/wayland-sessions"
        );
    }

    // "Other…" sits at `users.len()` and asks for the name. The preselection
    // is only a suggestion, so remembering never locks anyone out.
    let users = Rc::new(user::login_users());
    let preselected = [configured_user.as_deref(), remembered.user.as_deref()]
        .into_iter()
        .flatten()
        .find_map(|name| users.iter().position(|user| user.name == name))
        .unwrap_or(0);
    let chosen = |index: usize| users.get(index).cloned();

    // No fingerprint: greetd runs PAM itself and would not accept a local match.
    let auth = auth::start(
        Backend::Greetd {
            user: chosen(preselected).map(|user| user.name),
        },
        None,
    );

    let session_index = create_signal(last_login::preselect(
        &sessions,
        config.greeter.default_session.as_deref(),
        &remembered,
    ));
    let user_index = create_signal(preselected);

    let start_session = {
        let sessions = sessions.clone();
        let users = users.clone();
        let remember = config.greeter.remember;
        let systemd_cat = match config.greeter.session_output {
            SessionOutput::Journal => find_in_path("systemd-cat"),
            SessionOutput::Console => None,
        };
        move || {
            if sessions.is_empty() {
                log::error!("authenticated but there is no session to start");
                return;
            }
            // The modulo guards a list that shrank.
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
        }
    };
    // The session starts once the island has folded and the wallpaper is sharp,
    // the picture the session opens on.
    let flow = Flow::start(auth, start_session);

    let mut screen = screen(config, Mode::Greeter, auth, flow);
    screen.users = users.clone();
    screen.user_choice = user_index;
    screen.sessions = sessions;
    screen.session = session_index;
    screen.user.set(chosen(preselected));

    // A step in the user picker restarts the conversation; the first run is
    // the preselection greetd was started with.
    let user_signal = screen.user;
    let asked_for = std::cell::Cell::new(preselected);
    create_effect(move || {
        let index = user_index.get();
        if asked_for.replace(index) == index {
            return;
        }
        let user = users.get(index).cloned();
        auth.set_user(user.as_ref().map(|user| user.name.clone()));
        user_signal.set(user);
    });

    // Under "Other…" the name is typed; once it is, show who is signing in.
    create_effect(move || {
        if let Some(name) = auth.state.visible_answer.get() {
            user_signal.set(user::by_name(&name));
        }
    });

    // greetd starts the session only once the greeter has exited (`greetd-ipc(7)`).
    create_effect(move || {
        if auth.state.session_accepted.get() {
            log::info!("greetd accepted the session; exiting so it can start");
            quit_app();
        }
    });

    spawn_fullscreen_surfaces(screen, KeyboardInteractivity::Exclusive);
    flow.grow();
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
                move || ui::view(screen.clone(), info.clone()),
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
