//! Authentication as a conversation: PAM and greetd both
//! become a stream of [`AuthEvent`] driven by [`AuthCommand`], with the
//! fingerprint reader as a second producer on the same stream.
//!
//! Fail closed: [`AuthState::authenticated`] is set only on
//! [`AuthEvent::Success`]; any crash or closed channel leaves the screen locked.

pub mod fingerprint;
pub mod greetd;
pub mod pam;

use guido::prelude::*;

pub use crate::system::faillock::Lock;
use tokio::sync::mpsc;
use tokio::time::Instant;

use futures::StreamExt;
use futures::stream::FuturesUnordered;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthEvent {
    /// A question; the field is cleared for the answer.
    Prompt { secret: bool, message: String },
    /// Informational text from the auth stack.
    Info(String),
    /// Non-fatal error text from the auth stack.
    Error(String),
    /// The only event that opens the screen.
    Success,
    /// This attempt was refused; another may follow.
    Failed(String),
    /// A refusal with no message of its own (a fingerprint no-match).
    Rejected,
    /// The backend is gone and will ask nothing more.
    Ended(String),
    /// greetd starts the session only once the greeter has exited.
    SessionAccepted,
}

#[derive(Debug)]
pub enum AuthCommand {
    /// Always a `Secret`, so a password is never a `String` on its way.
    Respond(Secret),
    Restart,
    /// Greeter only.
    StartSession {
        command: Vec<String>,
        env: Vec<String>,
    },
    /// Greeter only. `None` asks for the name in the form.
    SetUser(Option<String>),
}

#[derive(Clone, Debug)]
pub enum Backend {
    /// Lock screen.
    Pam { service: String, user: String },
    /// Greeter, over `$GREETD_SOCK`.
    Greetd { user: Option<String> },
    /// Asks nothing and answers nothing: a screen drawn in a test.
    #[cfg(test)]
    Silent,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FingerprintState {
    #[default]
    Unavailable,
    Waiting,
    Reading,
    Retry(String),
    Exhausted,
    /// Released while the user is idle; the next key wakes it.
    Resting,
}

#[derive(Clone, Copy)]
pub struct AuthState {
    pub prompt: RwSignal<Option<Prompt>>,
    /// Disables the field so a second Enter cannot queue a second attempt.
    pub busy: RwSignal<bool>,
    pub notice: RwSignal<Option<Notice>>,
    /// A count, not a flag: an unchanged signal notifies nobody, and every
    /// refusal must be seen.
    pub rejections: RwSignal<u32>,
    /// Set only by [`AuthEvent::Success`].
    pub authenticated: RwSignal<bool>,
    /// Set only by [`AuthEvent::SessionAccepted`], in greeter mode.
    pub session_accepted: RwSignal<bool>,
    pub fingerprint: RwSignal<FingerprintState>,
    /// What is typed at a visible prompt; shared by every output's field.
    pub input: RwSignal<String>,
    /// Kept in locked, undumpable pages and wiped; never copied out.
    pub password: Password,
    /// Bumped by the auth task to have the UI thread, which owns the
    /// `Password`, empty the field.
    wipes: RwSignal<u32>,
    /// A `pam_faillock` lock as [`crate::system::faillock`] sees it: a warning, never
    /// a gate.
    pub locked: RwSignal<Option<Lock>>,
    /// The last answer to a visible prompt, i.e. the user name.
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

    /// Tracked. Masks before the first prompt: a password shown in the clear
    /// is worse than a name shown as dots.
    pub fn asks_secret(&self) -> bool {
        self.prompt
            .get()
            .as_ref()
            .map(|prompt| prompt.secret)
            .unwrap_or(true)
    }

    /// Tracked.
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

    pub fn clear_field(&self) {
        self.input.set(String::new());
        self.password.clear();
    }
}

#[derive(Clone, Copy)]
pub struct Auth {
    pub state: AuthState,
    service: Service<AuthCommand>,
}

impl Auth {
    /// The password is duplicated into protected pages, not taken, so the
    /// field keeps its dots until the verdict.
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
        // An empty answer would still count towards pam_faillock.
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

    /// The way out of a PAM module sitting on the conversation.
    pub fn restart(&self) {
        self.state.clear_field();
        self.state.busy.set(false);
        self.state.notice.set(None);
        self.service.send(AuthCommand::Restart);
    }

    /// Greeter only. Clears the field: it likely holds the previous account's
    /// password.
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

#[derive(Clone, Debug)]
pub struct FingerprintConfig {
    pub user: String,
    pub retries: u32,
    /// The reader is released while idle; `None` keeps it always listening.
    pub idle: Option<tokio::sync::watch::Receiver<bool>>,
}

/// `fingerprint` is only `Some` in lock mode: a fprintd match gives greetd no
/// session to start.
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
    // The greeter's account changes with the picker, so it has no tally.
    let tally_user = match &backend {
        Backend::Pam { user, .. } => Some(user.clone()),
        Backend::Greetd { .. } => None,
        #[cfg(test)]
        Backend::Silent => None,
    };

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

        // Not spawned: dropping this task must drop the reader and its claim.
        let mut readers = FuturesUnordered::new();
        match fingerprint {
            Some(config) => readers.push(fingerprint::run(config, event_tx.clone(), fingerprint_w)),
            None => fingerprint_w.set(FingerprintState::Unavailable),
        }

        // So `event_rx` closes once the producers are done.
        drop(event_tx);

        // Counted here, as the only writer, rather than read back from signals.
        let mut rejections = 0u32;
        let mut wipes = 0u32;

        // When to read the faillock tally again: now, when the lock's
        // description changes, and after every refusal.
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
                    let lock = tally_user.as_deref().and_then(crate::system::faillock::status);
                    locked_w.set(lock);
                    lock_check = lock
                        .and_then(|lock| lock.next_change(now))
                        .map(|wait| Instant::now() + wait);
                }
                Some(()) = readers.next() => {}
                _ = &mut conversation, if !conversation_done => {
                    conversation_done = true;
                    // Fail closed: no further answer can be accepted.
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
            #[cfg(test)]
            Backend::Silent => {
                let _ = (cmd_rx, event_tx);
                std::future::pending().await
            }
        }
    }
}
