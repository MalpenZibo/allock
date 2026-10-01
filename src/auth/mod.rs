//! Authentication, modelled as a conversation.
//!
//! PAM, greetd and polkit's helper all work the same way: the backend asks
//! something ("Password:", "Enter TOTP"), the user answers, and the backend
//! either accepts, asks again, or fails. Modelling them as a stream of
//! [`AuthEvent`] driven by [`AuthCommand`] means the lock screen, the greeter
//! and the polkit dialog share one UI and one state machine, and the
//! fingerprint reader plugs into the same event stream as a second, parallel
//! producer.
//!
//! **Fail closed.** [`AuthState::authenticated`] is set in exactly one place:
//! on [`AuthEvent::Success`]. A backend that crashes, a channel that closes,
//! a D-Bus daemon that disappears — every one of those leaves the flag false
//! and the screen locked.

pub mod fingerprint;
pub mod greetd;
pub mod pam;
pub mod polkit;

use guido::prelude::*;
use tokio::sync::mpsc;
use tokio::time::Instant;

use futures::StreamExt;
use futures::stream::FuturesUnordered;

// ---------------------------------------------------------------------------
// Conversation protocol
// ---------------------------------------------------------------------------

/// One step of an authentication conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthEvent {
    /// The backend wants an answer. `secret` decides whether the field masks.
    Prompt { secret: bool, message: String },
    /// A question asked before anything was answered, where the user may
    /// already be typing: polkit's helper asks for the password only once
    /// the fingerprint reader gives up. Shown like a prompt, but the field
    /// keeps what is in it — that is the answer being written.
    Asked { secret: bool, message: String },
    /// Informational text from the auth stack.
    Info(String),
    /// Non-fatal error text from the auth stack.
    Error(String),
    /// Authentication succeeded. The only event that opens the screen.
    Success,
    /// This attempt failed; the backend has restarted the conversation.
    Failed(String),
    /// An attempt was read and refused, with nothing to say about it that the
    /// screen is not already saying. The fingerprint reader's no-match: the
    /// hint beside the sensor covers the words, this covers the fact.
    Rejected,
    /// The backend is gone and will ask nothing more.
    Ended(String),
    /// greetd accepted the session. It starts once the greeter has exited,
    /// so this is the greeter's cue to leave.
    SessionAccepted,
}

#[derive(Debug)]
pub enum AuthCommand {
    /// Answer the outstanding prompt. A `Secret` whatever the prompt, so a
    /// password is never a `String` on its way to the backend.
    Respond(Secret),
    /// Abandon this attempt; the backend starts a fresh conversation.
    Restart,
    /// Greeter only: launch this session now that we are authenticated.
    StartSession {
        command: Vec<String>,
        env: Vec<String>,
    },
    /// Greeter only: authenticate this account instead, abandoning the attempt
    /// in flight. `None` is "Other…": ask for the name in the form.
    SetUser(Option<String>),
}

/// Which conversation drives this run.
#[derive(Clone, Debug)]
pub enum Backend {
    /// Lock screen: PAM, against the given service name and user.
    Pam { service: String, user: String },
    /// Greeter: greetd over `$GREETD_SOCK`.
    Greetd { user: Option<String> },
    /// Polkit dialog: polkit's helper, for the request `cookie` names.
    Polkit { user: String, cookie: String },
}

// ---------------------------------------------------------------------------
// Reactive state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub secret: bool,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: NoticeLevel,
    pub text: String,
}

/// What the fingerprint indicator shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FingerprintState {
    /// No reader, no enrolled finger, or fingerprint disabled in config.
    #[default]
    Unavailable,
    /// Waiting for a finger.
    Waiting,
    /// A finger is on the sensor and the reader is deciding.
    Reading,
    /// The last scan was unusable — `String` is fprintd's reason, already
    /// turned into something a human can act on.
    Retry(String),
    /// The reader gave up (too many bad scans, or it went away).
    Exhausted,
    /// Released while nobody is at the machine; the next key wakes it.
    Resting,
}

/// Signals the lock screen renders from.
#[derive(Clone, Copy)]
pub struct AuthState {
    /// What the backend is asking. `None` before the first prompt arrives.
    pub prompt: RwSignal<Option<Prompt>>,
    /// A response is in flight: the field is disabled until the backend
    /// answers, so a second Enter cannot queue a second attempt.
    pub busy: RwSignal<bool>,
    pub notice: RwSignal<Option<Notice>>,
    /// How many attempts have been refused. A count rather than a flag,
    /// because two identical refusals are two events and a signal that stays
    /// equal notifies nobody — the second wrong password has to be as loud as
    /// the first.
    pub rejections: RwSignal<u32>,
    /// Set only by [`AuthEvent::Success`].
    pub authenticated: RwSignal<bool>,
    /// Set only by [`AuthEvent::SessionAccepted`], in greeter mode.
    pub session_accepted: RwSignal<bool>,
    pub fingerprint: RwSignal<FingerprintState>,
    /// What is typed at a visible prompt — a user name. Owned here so the auth
    /// layer can clear it, and so every output's field mirrors one buffer.
    pub input: RwSignal<String>,
    /// What is typed at a secret prompt: guido's `Password`, which keeps it in
    /// locked, undumpable pages and wipes them, and never hands out a copy.
    pub password: Password,
    /// Bumped by the auth task to have the field emptied — after a refusal, on
    /// a new prompt, on success. A `Password` lives on the UI thread, so the
    /// task asks rather than clears.
    wipes: RwSignal<u32>,
    /// What to say about a `pam_faillock` lock on the account, as far as
    /// [`crate::faillock`] can tell ahead of PAM: a warning, never a gate.
    pub locked: RwSignal<Option<String>>,
    /// The last answer given to a *visible* prompt. In both flows we drive,
    /// the only thing ever asked in the clear is a user name — the greeter
    /// uses this to show who is signing in.
    pub visible_answer: RwSignal<Option<String>>,
}

impl AuthState {
    fn new() -> Self {
        Self {
            prompt: create_signal(None),
            busy: create_signal(false),
            notice: create_signal(None),
            rejections: create_signal(0),
            authenticated: create_signal(false),
            session_accepted: create_signal(false),
            fingerprint: create_signal(FingerprintState::default()),
            input: create_signal(String::new()),
            password: create_password(),
            wipes: create_signal(0),
            locked: create_signal(None),
            visible_answer: create_signal(None),
        }
    }

    /// Whether the outstanding prompt is a secret one. Tracked. Before the
    /// first prompt the field masks: a name shown as dots is a nuisance, a
    /// password shown in the clear is not recoverable.
    pub fn asks_secret(&self) -> bool {
        self.prompt
            .get()
            .as_ref()
            .map(|prompt| prompt.secret)
            .unwrap_or(true)
    }

    /// Whether the field the prompt is using is empty. Tracked.
    pub fn field_is_empty(&self) -> bool {
        if self.asks_secret() {
            self.password.is_empty()
        } else {
            self.input.get().is_empty()
        }
    }

    /// [`field_is_empty`](Self::field_is_empty), for an event handler.
    pub fn field_is_empty_untracked(&self) -> bool {
        let secret = self
            .prompt
            .get_untracked()
            .as_ref()
            .map(|prompt| prompt.secret)
            .unwrap_or(true);
        if secret {
            self.password.with_untracked(Secret::is_empty)
        } else {
            self.input.get_untracked().is_empty()
        }
    }

    /// Empty both fields; the password is wiped in place.
    pub fn clear_field(&self) {
        self.input.set(String::new());
        self.password.clear();
    }
}

/// Main-thread handle to the running conversation.
#[derive(Clone, Copy)]
pub struct Auth {
    pub state: AuthState,
    service: Service<AuthCommand>,
}

impl Auth {
    /// Answer the outstanding prompt with whatever is in the field.
    ///
    /// Enter and the submit arrow both come here. A password is duplicated
    /// into protected pages of its own rather than taken, so the field keeps
    /// its dots while the check runs; it is wiped when the answer comes back.
    pub fn submit(&self) {
        let state = self.state;
        if state.busy.get_untracked() || state.authenticated.get_untracked() {
            return;
        }
        let secret = state
            .prompt
            .get_untracked()
            .map(|prompt| prompt.secret)
            .unwrap_or(true);
        // An empty answer is never a real attempt, and attempts are counted:
        // pam_faillock locks the account after a handful. Enter on an empty field
        // used to spend one.
        let answer = if secret {
            if state.password.with_untracked(Secret::is_empty) {
                return;
            }
            state.password.with_untracked(Secret::duplicate)
        } else {
            let name = state.input.get_untracked();
            if name.is_empty() {
                return;
            }
            state.visible_answer.set(Some(name.clone()));
            Secret::from(name)
        };
        state.busy.set(true);
        state.notice.set(None);
        self.service.send(AuthCommand::Respond(answer));
    }

    /// Throw away this attempt and start a fresh conversation.
    ///
    /// The way out of a stuck prompt: a PAM module can sit on the
    /// conversation (a fingerprint stage waiting for a finger that never
    /// comes), and without this the field would take input that PAM is not
    /// listening for.
    pub fn restart(&self) {
        self.state.clear_field();
        self.state.busy.set(false);
        self.state.notice.set(None);
        self.service.send(AuthCommand::Restart);
    }

    /// Greeter only: sign in as `user` instead, or ask for the name (`None`).
    ///
    /// Whatever was typed belongs to the attempt being abandoned — a password
    /// for the previous account, most likely — so it goes with it.
    pub fn set_user(&self, user: Option<String>) {
        self.state.clear_field();
        self.state.busy.set(false);
        self.state.notice.set(None);
        self.state.visible_answer.set(None);
        self.service.send(AuthCommand::SetUser(user));
    }

    pub fn start_session(&self, command: Vec<String>, env: Vec<String>) {
        self.service
            .send(AuthCommand::StartSession { command, env });
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Fingerprint verification settings, when it should run at all.
#[derive(Clone, Debug)]
pub struct FingerprintConfig {
    pub user: String,
    pub retries: u32,
    /// Whether the user is idle, from [`crate::idle`]. The reader is released
    /// while they are; `None` keeps it listening for as long as the screen is
    /// up.
    pub idle: Option<tokio::sync::watch::Receiver<bool>>,
}

/// Start the conversation and expose it reactively.
///
/// `fingerprint` is only ever `Some` in lock mode: a fprintd match proves the
/// user's identity, which is all the lock screen needs, but it does not give
/// greetd a session to start, so the greeter always requires the password.
pub fn start(backend: Backend, fingerprint: Option<FingerprintConfig>) -> Auth {
    let state = AuthState::new();

    let prompt_w = state.prompt.writer();
    let busy_w = state.busy.writer();
    let notice_w = state.notice.writer();
    let rejections_w = state.rejections.writer();
    let authenticated_w = state.authenticated.writer();
    let session_accepted_w = state.session_accepted.writer();
    let fingerprint_w = state.fingerprint.writer();
    let wipes_w = state.wipes.writer();
    let locked_w = state.locked.writer();
    // The account whose tally to read. The greeter's changes with the picker
    // and greetd runs its stack for whoever is typed, so it has none.
    let tally_user = match &backend {
        Backend::Pam { user, .. } | Backend::Polkit { user, .. } => Some(user.clone()),
        Backend::Greetd { .. } => None,
    };

    // The task's requests to empty the field, carried out on the UI thread,
    // where the `Password` lives. The first run is the count at zero.
    create_effect(move || {
        if state.wipes.get() > 0 {
            state.clear_field();
        }
    });

    let service = create_service::<AuthCommand, _, _>(move |mut ui_rx, _ctx| async move {
        let (event_tx, mut event_rx) = mpsc::channel::<AuthEvent>(32);
        let (cmd_tx, cmd_rx) = mpsc::channel::<AuthCommand>(8);

        let mut conversation = Box::pin(backend.run(cmd_rx, event_tx.clone()));
        let mut conversation_done = false;

        // Held in a FuturesUnordered rather than spawned: dropping this task
        // drops the reader loop with it, so nothing keeps the fprintd device
        // claimed after the screen is gone.
        let mut readers = FuturesUnordered::new();
        match fingerprint {
            Some(config) => readers.push(fingerprint::run(config, event_tx.clone(), fingerprint_w)),
            None => fingerprint_w.set(FingerprintState::Unavailable),
        }

        // The last sender we hold ourselves: dropped so `event_rx` closes once
        // the producers are done, instead of parking the loop forever.
        drop(event_tx);

        // The only writer, so it can be counted here rather than read back
        // from a signal a background task has no business reading.
        let mut rejections = 0u32;
        // Likewise the only writer of the wipe count.
        let mut wipes = 0u32;

        // When to read the tally again: at once, then when what the screen
        // says about a lock changes, and after every refusal. Never without
        // an account to read it for.
        let mut lock_check = tally_user.as_ref().map(|_| Instant::now());

        loop {
            tokio::select! {
                Some(command) = ui_rx.recv() => {
                    if cmd_tx.send(command).await.is_err() {
                        log::error!("auth backend is not accepting commands");
                        busy_w.set(false);
                        notice_w.set(Some(Notice {
                            level: NoticeLevel::Error,
                            text: "Authentication is unavailable".to_owned(),
                        }));
                    }
                }
                Some(event) = event_rx.recv() => {
                    log::debug!("auth event: {event:?}");
                    match event {
                        AuthEvent::Prompt { secret, message } => {
                            wipes += 1;
                            wipes_w.set(wipes);
                            busy_w.set(false);
                            prompt_w.set(Some(Prompt { secret, message }));
                        }
                        AuthEvent::Asked { secret, message } => {
                            prompt_w.set(Some(Prompt { secret, message }));
                        }
                        AuthEvent::Info(text) => {
                            notice_w.set(Some(Notice { level: NoticeLevel::Info, text }));
                        }
                        AuthEvent::Error(text) => {
                            notice_w.set(Some(Notice { level: NoticeLevel::Error, text }));
                        }
                        AuthEvent::Failed(text) => {
                            wipes += 1;
                            wipes_w.set(wipes);
                            busy_w.set(false);
                            notice_w.set(Some(Notice { level: NoticeLevel::Error, text }));
                            rejections += 1;
                            rejections_w.set(rejections);
                            if tally_user.is_some() {
                                lock_check = Some(Instant::now());
                            }
                        }
                        AuthEvent::Rejected => {
                            rejections += 1;
                            rejections_w.set(rejections);
                        }
                        // The one place the screen is allowed to open.
                        AuthEvent::Success => {
                            wipes += 1;
                            wipes_w.set(wipes);
                            busy_w.set(false);
                            notice_w.set(None);
                            authenticated_w.set(true);
                        }
                        AuthEvent::Ended(text) => {
                            busy_w.set(false);
                            notice_w.set(Some(Notice { level: NoticeLevel::Error, text }));
                        }
                        AuthEvent::SessionAccepted => session_accepted_w.set(true),
                    }
                }
                _ = tokio::time::sleep_until(lock_check.unwrap_or_else(Instant::now)),
                    if lock_check.is_some() =>
                {
                    let now = std::time::SystemTime::now();
                    let lock = tally_user.as_deref().and_then(crate::faillock::status);
                    locked_w.set(lock.map(|lock| lock.describe(now)));
                    lock_check = lock
                        .and_then(|lock| lock.next_change(now))
                        .map(|wait| Instant::now() + wait);
                }
                // Polled here, where it is owned; a finished reader has
                // nothing left to say.
                Some(()) = readers.next() => {}
                _ = &mut conversation, if !conversation_done => {
                    conversation_done = true;
                    // Fail closed: the conversation is over and it was not a
                    // success, so no further answer can ever be accepted.
                    busy_w.set(false);
                    notice_w.set(Some(Notice {
                        level: NoticeLevel::Error,
                        text: "Authentication backend stopped".to_owned(),
                    }));
                }
                else => break,
            }
        }
    });

    Auth { state, service }
}

impl Backend {
    async fn run(self, cmd_rx: mpsc::Receiver<AuthCommand>, event_tx: mpsc::Sender<AuthEvent>) {
        match self {
            Backend::Pam { service, user } => pam::run(service, user, cmd_rx, event_tx).await,
            Backend::Greetd { user } => greetd::run(user, cmd_rx, event_tx).await,
            Backend::Polkit { user, cookie } => polkit::run(user, cookie, cmd_rx, event_tx).await,
        }
    }
}
