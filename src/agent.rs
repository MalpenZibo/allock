//! The polkit authentication agent: `allock --polkit`.
//!
//! Each `BeginAuthentication` opens a modal dialog that talks to polkit's
//! helper itself ([`crate::auth::polkit`]), so the password never passes
//! through here. A second request while a dialog is open is refused. The agent
//! is not re-registered if polkitd restarts, so its service should restart too.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use guido::prelude::*;
use tokio::sync::oneshot;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::config::Config;
use crate::theme::ThemeColors;

/// Any path does; polkitd is told it at registration.
const OBJECT_PATH: &str = "/org/allock/PolicyKit1/AuthenticationAgent";

/// Set when registration fails, so the non-zero exit gets the service restarted.
pub static REGISTRATION_FAILED: AtomicBool = AtomicBool::new(false);

/// One authentication polkitd asked for, as the dialog shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// Tells a request from the next one with the same text.
    pub id: u64,
    pub action_id: String,
    /// What polkit says the program is trying to do, already localized.
    pub message: String,
    pub user: String,
    pub cookie: String,
}

/// How a dialog ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The helper accepted, and has told polkitd so itself.
    Authenticated,
    /// Cancelled, or the dialog went away without an answer.
    Dismissed,
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Failed(String),
    /// The name polkit's own agents answer a dismissed dialog with.
    Cancelled(String),
}

struct Pending {
    id: u64,
    cookie: String,
    reply: oneshot::Sender<Outcome>,
}

/// The request being shown. Ending it and hiding its dialog happen under one
/// lock, so a request ending as the next begins cannot close the new dialog.
#[derive(Clone)]
pub struct Requests {
    state: Arc<Mutex<State>>,
    show: WriteSignal<Option<Request>>,
}

#[derive(Default)]
struct State {
    next_id: u64,
    pending: Option<Pending>,
}

impl Requests {
    fn begin(&self, request: impl FnOnce(u64) -> Request) -> Option<oneshot::Receiver<Outcome>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.pending.is_some() {
            return None;
        }
        state.next_id += 1;
        let request = request(state.next_id);
        let (reply, outcome) = oneshot::channel();
        state.pending = Some(Pending {
            id: request.id,
            cookie: request.cookie.clone(),
            reply,
        });
        self.show.set(Some(request));
        Some(outcome)
    }

    /// End the request `id` with `outcome`, if it is still the one shown.
    pub fn resolve(&self, id: u64, outcome: Outcome) {
        self.end(|pending| pending.id == id, outcome);
    }

    /// polkitd withdrew the request: the caller went away, or gave up.
    fn cancel(&self, cookie: &str) {
        self.end(|pending| pending.cookie == cookie, Outcome::Dismissed);
    }

    fn end(&self, matches: impl FnOnce(&Pending) -> bool, outcome: Outcome) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.pending.as_ref().is_some_and(matches) {
            return;
        }
        let Some(pending) = state.pending.take() else {
            return;
        };
        self.show.set(None);
        // The request's caller may be gone already; nothing to tell then.
        let _ = pending.reply.send(outcome);
    }
}

struct Agent {
    requests: Requests,
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    /// Returns once the dialog is over: success is the helper's to report,
    /// so all that is left to say here is whether the user walked away.
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        _icon_name: String,
        _details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> Result<(), AgentError> {
        let Some(user) = choose_identity(&identities) else {
            log::warn!("{action_id}: no user among the identities polkit offered");
            return Err(AgentError::Failed("No user to authenticate as".to_owned()));
        };
        log::info!("{action_id}: authenticating as {user}");

        let begun = self.requests.begin(|id| Request {
            id,
            action_id: action_id.clone(),
            message,
            user,
            cookie,
        });
        let Some(outcome) = begun else {
            log::info!("{action_id}: refused, another authentication is in progress");
            return Err(AgentError::Failed(
                "Another authentication is in progress".to_owned(),
            ));
        };

        match outcome.await.unwrap_or(Outcome::Dismissed) {
            Outcome::Authenticated => {
                log::info!("{action_id}: authenticated");
                Ok(())
            }
            Outcome::Dismissed => {
                log::info!("{action_id}: dismissed");
                Err(AgentError::Cancelled("Dismissed".to_owned()))
            }
        }
    }

    async fn cancel_authentication(&self, cookie: String) {
        log::info!("polkit withdrew a request");
        self.requests.cancel(&cookie);
    }
}

/// Our own account when polkit offers it, else the first user it offers.
/// Groups are skipped: polkit also lists their members as users.
fn choose_identity(identities: &[(String, HashMap<String, OwnedValue>)]) -> Option<String> {
    let uids: Vec<u32> = identities
        .iter()
        .filter(|(kind, _)| kind == "unix-user")
        .filter_map(|(_, details)| u32::try_from(details.get("uid")?).ok())
        .collect();
    let own = uzers::get_current_uid();
    let uid = uids
        .iter()
        .find(|&&uid| uid == own)
        .or_else(|| uids.first())?;
    let user = uzers::get_user_by_uid(*uid)?;
    Some(user.name().to_string_lossy().into_owned())
}

/// Register as the agent for this user's graphical session, found the way
/// polkitd finds it: the process's own session, else the user's display one.
/// A service under `user@.service` has no session of its own.
async fn register(requests: Requests) -> zbus::Result<zbus::Connection> {
    let connection = zbus::Connection::system().await?;
    connection
        .object_server()
        .at(OBJECT_PATH, Agent { requests })
        .await?;

    let session = session_id(&connection).await?;
    let subject = (
        "unix-session",
        HashMap::from([("session-id", Value::from(session.as_str()))]),
    );
    connection
        .call_method(
            Some("org.freedesktop.PolicyKit1"),
            "/org/freedesktop/PolicyKit1/Authority",
            Some("org.freedesktop.PolicyKit1.Authority"),
            "RegisterAuthenticationAgent",
            &(subject, locale(), OBJECT_PATH),
        )
        .await?;
    log::info!("registered as the polkit agent for session {session}");
    Ok(connection)
}

async fn session_id(connection: &zbus::Connection) -> zbus::Result<String> {
    const LOGIND: &str = "org.freedesktop.login1";
    let manager = zbus::Proxy::new(
        connection,
        LOGIND,
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;

    let own: zbus::Result<OwnedObjectPath> =
        manager.call("GetSessionByPID", &(std::process::id())).await;
    if let Ok(path) = own {
        let session =
            zbus::Proxy::new(connection, LOGIND, path, "org.freedesktop.login1.Session").await?;
        return session.get_property("Id").await;
    }

    let path: OwnedObjectPath = manager.call("GetUser", &(uzers::get_current_uid())).await?;
    let user = zbus::Proxy::new(connection, LOGIND, path, "org.freedesktop.login1.User").await?;
    let (id, _): (String, OwnedObjectPath) = user.get_property("Display").await?;
    if id.is_empty() {
        return Err(zbus::Error::Failure(
            "no graphical session to be the agent for".to_owned(),
        ));
    }
    Ok(id)
}

/// The language polkit localizes its messages in.
fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| "C".to_owned())
}

/// Register, and open a dialog for each request until the process ends.
pub fn run(config: Rc<Config>, theme: ThemeColors) {
    let shown = create_signal::<Option<Request>>(None);
    let requests = Requests {
        state: Default::default(),
        show: shown.writer(),
    };

    let failed = create_signal(false);
    let failed_w = failed.writer();
    create_service::<(), _, _>({
        let requests = requests.clone();
        move |_rx, _ctx| async move {
            match register(requests).await {
                // Dropping the connection leaves the bus, and polkitd forgets the agent.
                Ok(_connection) => std::future::pending::<()>().await,
                Err(e) => {
                    log::error!("cannot register as a polkit agent: {e}");
                    failed_w.set(true);
                }
            }
        }
    });
    create_effect(move || {
        if failed.get() {
            REGISTRATION_FAILED.store(true, Ordering::SeqCst);
            quit_app();
        }
    });

    let open: Rc<Cell<Option<(u64, SurfaceHandle)>>> = Default::default();
    create_effect(move || {
        let request = shown.get();
        match (open.take(), request) {
            (Some((id, handle)), Some(request)) if request.id == id => {
                open.set(Some((id, handle)));
            }
            (previous, request) => {
                if let Some((_, handle)) = previous {
                    handle.close();
                }
                if let Some(request) = request {
                    let id = request.id;
                    let handle = spawn_dialog(config.clone(), theme, request, requests.clone());
                    open.set(Some((id, handle)));
                }
            }
        }
    });
}

fn spawn_dialog(
    config: Rc<Config>,
    theme: ThemeColors,
    request: Request,
    requests: Requests,
) -> SurfaceHandle {
    spawn_surface(
        SurfaceConfig::new()
            // No output named: the compositor puts the dialog where the user is.
            .width(0)
            .height(0)
            .anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT)
            .layer(Layer::Overlay)
            .exclusive_zone(ExclusiveZone::Ignore)
            .keyboard_interactivity(KeyboardInteractivity::Exclusive)
            .namespace("allock-polkit")
            .background_color(Color::TRANSPARENT),
        move || {
            let id = request.id;
            // The request ends with the dialog, even one the compositor closes.
            on_cleanup({
                let requests = requests.clone();
                move || requests.resolve(id, Outcome::Dismissed)
            });
            crate::ui::dialog::view(config, theme, request, move |outcome| {
                requests.resolve(id, outcome)
            })
        },
    )
}
