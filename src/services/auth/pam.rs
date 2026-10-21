//! PAM conversation, used by the lock screen. `pam_authenticate()` blocks and
//! calls back into us, so it runs on its own thread.
//!
//! An attempt starts only once the user has submitted an answer, so no
//! conversation is left pending to fail (and count for pam_faillock) when the
//! screen is unlocked by fingerprint, Escape or exit.

use std::ffi::{CStr, CString};
use std::sync::mpsc as sync_mpsc;

use pam_client2::{Context, ConversationHandler, ErrorCode, Flag};
use tokio::sync::mpsc;

use guido::prelude::Secret;

use super::{AuthCommand, AuthEvent, Prompt};

/// Stock policies to borrow when allock has none, best first. `login` is a
/// last resort: its `pam_nologin` and `pam_shells` can make the screen
/// impossible to unlock.
const FALLBACK_SERVICES: [&str; 3] = ["system-auth", "common-auth", "login"];

fn service_exists(service: &str) -> bool {
    // libpam searches both; /etc/pam.d overrides vendor defaults.
    ["/etc/pam.d", "/usr/lib/pam.d"]
        .iter()
        .any(|dir| std::path::Path::new(dir).join(service).is_file())
}

/// Must run before locking: without a policy PAM uses `other`, usually
/// `pam_deny`, and the screen would refuse every password.
pub fn resolve_service(configured: &str) -> Option<String> {
    if service_exists(configured) {
        return Some(configured.to_owned());
    }

    let fallback = FALLBACK_SERVICES
        .iter()
        .find(|service| service_exists(service))?;

    log::warn!(
        "no PAM policy for service \"{configured}\"; falling back to \"{fallback}\". \
         Install pam/allock as /etc/pam.d/{configured} to pick the stack yourself."
    );
    Some((*fallback).to_owned())
}

enum Reply {
    Answer(Secret),
    /// Fails the callback, unwinding `authenticate()`.
    Abort,
}

pub async fn run(
    service: String,
    user: String,
    mut cmd_rx: mpsc::Receiver<AuthCommand>,
    event_tx: mpsc::Sender<AuthEvent>,
) {
    let (reply_tx, reply_rx) = sync_mpsc::channel::<Reply>();

    let thread = std::thread::Builder::new()
        .name("allock-pam".to_owned())
        .spawn({
            let event_tx = event_tx.clone();
            move || pam_thread(&service, &user, reply_rx, event_tx)
        });

    let thread = match thread {
        Ok(handle) => handle,
        Err(e) => {
            log::error!("failed to spawn the PAM thread: {e}");
            let _ = event_tx
                .send(AuthEvent::Ended("Cannot start authentication".to_owned()))
                .await;
            return;
        }
    };

    while let Some(command) = cmd_rx.recv().await {
        let reply = match command {
            AuthCommand::Respond(answer) => Reply::Answer(answer),
            AuthCommand::Restart => Reply::Abort,
            AuthCommand::StartSession { .. } | AuthCommand::SetUser(_) => continue,
        };
        if reply_tx.send(reply).is_err() {
            break;
        }
    }

    // Releases a callback blocked in `recv()` so the PAM thread can exit.
    drop(reply_tx);
    let _ = tokio::task::spawn_blocking(move || thread.join()).await;
}

fn pam_thread(
    service: &str,
    user: &str,
    reply_rx: sync_mpsc::Receiver<Reply>,
    event_tx: mpsc::Sender<AuthEvent>,
) {
    let conversation = Conversation {
        events: event_tx.clone(),
        replies: reply_rx,
        pending: None,
        first_prompt: None,
    };

    let mut context = match Context::new(service, Some(user), conversation) {
        Ok(context) => context,
        Err(e) => {
            log::error!("pam_start({service}) failed: {e}");
            let _ = event_tx.blocking_send(AuthEvent::Ended(format!(
                "PAM service \"{service}\" is not configured"
            )));
            return;
        }
    };

    // PAM expects retries on the same handle, keeping module state such as
    // failure delays and tallies.
    loop {
        // A pending attempt would fail however it ends, and pam_faillock
        // counts every failure: wait for an answer before starting one.
        let Some(answer) = context.conversation_mut().wait_for_answer() else {
            return;
        };
        context.conversation_mut().pending = Some(answer);

        let outcome = context
            .authenticate(Flag::NONE)
            .and_then(|()| context.acct_mgmt(Flag::NONE));
        context.conversation_mut().pending = None;

        let event = match outcome {
            Ok(()) => AuthEvent::Success,
            Err(e) => {
                log::info!("PAM attempt failed: {:?} ({e})", e.code());
                AuthEvent::Failed(describe(e.code(), e.message()))
            }
        };

        let succeeded = event == AuthEvent::Success;
        if event_tx.blocking_send(event).is_err() || succeeded {
            return;
        }
        if context.conversation_mut().show_first_prompt().is_err() {
            return;
        }
    }
}

/// "No such user" and "wrong password" read the same, so as not to leak
/// account existence.
fn describe(code: ErrorCode, message: Option<&str>) -> String {
    match code {
        ErrorCode::AUTH_ERR | ErrorCode::USER_UNKNOWN | ErrorCode::CRED_INSUFFICIENT => {
            "Incorrect password".to_owned()
        }
        ErrorCode::MAXTRIES => "Too many attempts, try again later".to_owned(),
        ErrorCode::ACCT_EXPIRED => "Account expired".to_owned(),
        ErrorCode::CRED_EXPIRED | ErrorCode::NEW_AUTHTOK_REQD => {
            "Password expired — log in on a TTY to change it".to_owned()
        }
        ErrorCode::PERM_DENIED => "Permission denied".to_owned(),
        ErrorCode::AUTHINFO_UNAVAIL => "Authentication service unavailable".to_owned(),
        // CONV_ERR comes from our own Abort reply.
        ErrorCode::CONV_ERR | ErrorCode::ABORT => String::new(),
        _ => message.unwrap_or("Authentication failed").to_owned(),
    }
}

struct Conversation {
    events: mpsc::Sender<AuthEvent>,
    replies: sync_mpsc::Receiver<Reply>,
    pending: Option<Secret>,
    /// Shown again between attempts.
    first_prompt: Option<Prompt>,
}

impl Conversation {
    /// `None` once the UI is gone. No PAM attempt is running yet, so Escape
    /// only shows the first question again.
    fn wait_for_answer(&mut self) -> Option<Secret> {
        loop {
            match self.replies.recv() {
                Ok(Reply::Answer(answer)) => return Some(answer),
                Ok(Reply::Abort) => self.show_first_prompt().ok()?,
                Err(_) => return None,
            }
        }
    }

    fn show_first_prompt(&self) -> Result<(), ()> {
        let prompt = self.first_prompt.clone().unwrap_or_else(|| Prompt {
            secret: true,
            message: "Password".to_owned(),
        });
        self.events
            .blocking_send(AuthEvent::Prompt(prompt))
            .map_err(|_| ())
    }

    fn ask(&mut self, prompt: &CStr, secret: bool) -> Result<CString, ErrorCode> {
        let prompt = Prompt {
            secret,
            message: prompt.to_string_lossy().into_owned(),
        };

        if let Some(answer) = self.pending.take() {
            self.first_prompt = Some(prompt);
            return CString::new(answer.expose()).map_err(|_| ErrorCode::CONV_ERR);
        }

        self.events
            .blocking_send(AuthEvent::Prompt(prompt))
            .map_err(|_| ErrorCode::CONV_ERR)?;

        match self.replies.recv() {
            // PAM would silently truncate at a NUL, so refuse it. pam-client2
            // drops this `CString` unwiped: the one copy outside protected
            // pages, kept out of core dumps only by the undumpable process.
            Ok(Reply::Answer(answer)) => {
                CString::new(answer.expose()).map_err(|_| ErrorCode::CONV_ERR)
            }
            Ok(Reply::Abort) | Err(_) => Err(ErrorCode::CONV_ERR),
        }
    }
}

impl ConversationHandler for Conversation {
    fn prompt_echo_on(&mut self, prompt: &CStr) -> Result<CString, ErrorCode> {
        self.ask(prompt, false)
    }

    fn prompt_echo_off(&mut self, prompt: &CStr) -> Result<CString, ErrorCode> {
        self.ask(prompt, true)
    }

    fn text_info(&mut self, msg: &CStr) {
        let _ = self
            .events
            .blocking_send(AuthEvent::Info(msg.to_string_lossy().into_owned()));
    }

    fn error_msg(&mut self, msg: &CStr) {
        let _ = self
            .events
            .blocking_send(AuthEvent::Error(msg.to_string_lossy().into_owned()));
    }
}
