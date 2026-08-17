//! Authentication, modelled as a conversation.
//!
//! PAM and greetd both work the same way: the backend asks something
//! ("Password:", "Enter TOTP"), the user answers, and the backend either
//! accepts, asks again, or fails. Modelling both as a stream of [`AuthEvent`]
//! driven by [`AuthCommand`] means the lock screen and the greeter share one
//! UI and one state machine, and the fingerprint reader plugs into the same
//! event stream as a second, parallel producer.
//!
//! **Fail closed.** [`AuthState::authenticated`] is set in exactly one place:
//! on [`AuthEvent::Success`]. A backend that crashes, a channel that closes,
//! a D-Bus daemon that disappears — every one of those leaves the flag false
//! and the screen locked.

pub mod fingerprint;
pub mod greetd;
pub mod pam;

use guido::prelude::*;
use tokio::sync::mpsc;

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
    /// Informational text from the auth stack.
    Info(String),
    /// Non-fatal error text from the auth stack.
    Error(String),
    /// Authentication succeeded. The only event that opens the screen.
    Success,
    /// This attempt failed; the backend has restarted the conversation.
    Failed(String),
    /// The backend is gone and will ask nothing more.
    Ended(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCommand {
    /// Answer the outstanding prompt.
    Respond(String),
    /// Abandon this attempt; the backend starts a fresh conversation.
    Restart,
    /// Greeter only: launch this session now that we are authenticated.
    StartSession {
        command: Vec<String>,
        env: Vec<String>,
    },
}

/// Which conversation drives this run.
#[derive(Clone, Debug)]
pub enum Backend {
    /// Lock screen: PAM, against the given service name and user.
    Pam { service: String, user: String },
    /// Greeter: greetd over `$GREETD_SOCK`.
    Greetd { user: Option<String> },
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
    /// The last scan was unusable — `String` is fprintd's reason, already
    /// turned into something a human can act on.
    Retry(String),
    /// The reader gave up (too many bad scans, or it went away).
    Exhausted,
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
    /// Set only by [`AuthEvent::Success`].
    pub authenticated: RwSignal<bool>,
    pub fingerprint: RwSignal<FingerprintState>,
    /// The text being typed. Owned here so the auth layer can clear it on a
    /// failed attempt, and so every output's field mirrors the same buffer.
    pub input: RwSignal<String>,
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
            authenticated: create_signal(false),
            fingerprint: create_signal(FingerprintState::default()),
            input: create_signal(String::new()),
            visible_answer: create_signal(None),
        }
    }
}

/// Main-thread handle to the running conversation.
#[derive(Clone, Copy)]
pub struct Auth {
    pub state: AuthState,
    service: Service<AuthCommand>,
}

impl Auth {
    /// Answer the outstanding prompt with whatever is in the input buffer.
    pub fn submit(&self) {
        if self.state.busy.get_untracked() || self.state.authenticated.get_untracked() {
            return;
        }
        let answer = self.state.input.get_untracked();
        // An empty answer is never a real attempt, and attempts are counted:
        // pam_faillock locks the account after a handful. Enter on an empty field
        // used to spend one.
        if answer.is_empty() {
            return;
        }
        let visible = self
            .state
            .prompt
            .get_untracked()
            .is_some_and(|prompt| !prompt.secret);
        if visible {
            self.state.visible_answer.set(Some(answer.clone()));
        }
        self.state.busy.set(true);
        self.state.notice.set(None);
        self.service.send(AuthCommand::Respond(answer));
    }

    /// Throw away this attempt and start a fresh conversation.
    ///
    /// The way out of a stuck prompt: a PAM module can sit on the
    /// conversation (a fingerprint stage waiting for a finger that never
    /// comes), and without this the field would take input that PAM is not
    /// listening for.
    pub fn restart(&self) {
        self.state.input.set(String::new());
        self.state.busy.set(false);
        self.state.notice.set(None);
        self.service.send(AuthCommand::Restart);
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
    let authenticated_w = state.authenticated.writer();
    let fingerprint_w = state.fingerprint.writer();
    let input_w = state.input.writer();

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
                            input_w.set(String::new());
                            busy_w.set(false);
                            prompt_w.set(Some(Prompt { secret, message }));
                        }
                        AuthEvent::Info(text) => {
                            notice_w.set(Some(Notice { level: NoticeLevel::Info, text }));
                        }
                        AuthEvent::Error(text) => {
                            notice_w.set(Some(Notice { level: NoticeLevel::Error, text }));
                        }
                        AuthEvent::Failed(text) => {
                            input_w.set(String::new());
                            busy_w.set(false);
                            notice_w.set(Some(Notice { level: NoticeLevel::Error, text }));
                        }
                        // The one place the screen is allowed to open.
                        AuthEvent::Success => {
                            input_w.set(String::new());
                            busy_w.set(false);
                            notice_w.set(None);
                            authenticated_w.set(true);
                        }
                        AuthEvent::Ended(text) => {
                            busy_w.set(false);
                            notice_w.set(Some(Notice { level: NoticeLevel::Error, text }));
                        }
                    }
                }
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
        }
    }
}
