//! PAM conversation, used by the lock screen.
//!
//! `pam_authenticate()` is blocking and drives the conversation by calling
//! back into us, so PAM gets a dedicated OS thread. The thread and the async
//! side talk over channels: prompts go out on a tokio channel, answers come
//! back on a `std::sync::mpsc` one the conversation handler blocks on.
//!
//! Note that `/etc/pam.d/<service>` must exist for this to work at all. On a
//! typical Linux system `pam_unix` reads `/etc/shadow` through the setuid
//! `unix_chkpwd` helper, which is what lets allock authenticate without being
//! privileged itself.

use std::ffi::{CStr, CString};
use std::sync::mpsc as sync_mpsc;

use pam_client2::{Context, ConversationHandler, ErrorCode, Flag};
use tokio::sync::mpsc;

use guido::prelude::Secret;

use super::{AuthCommand, AuthEvent};

/// Stock policies to borrow when allock has none of its own, best first.
///
/// `system-auth` on Arch/Fedora/SUSE and `common-auth` on Debian are the
/// password-checking stacks every other service includes. `login` is the last
/// resort, and only because something has to be: it pulls in `pam_shells` and
/// `pam_nologin`, so an `/etc/nologin` file or a shell missing from
/// `/etc/shells` would make the screen impossible to unlock. See `pam/allock`
/// for why that matters here.
const FALLBACK_SERVICES: [&str; 3] = ["system-auth", "common-auth", "login"];

fn service_exists(service: &str) -> bool {
    // Both directories are searched by libpam; Arch ships defaults in
    // /usr/lib/pam.d and lets /etc/pam.d override them.
    ["/etc/pam.d", "/usr/lib/pam.d"]
        .iter()
        .any(|dir| std::path::Path::new(dir).join(service).is_file())
}

/// Pick the PAM service to authenticate against, or `None` if nothing usable
/// exists.
///
/// Resolved *before* taking the lock. With no matching `/etc/pam.d` entry PAM
/// falls back to the `other` policy, which is `pam_deny` on essentially every
/// distribution — the screen would lock and then refuse every password, with
/// no way back except a TTY.
///
/// Distributions ship a policy alongside the binary, which is why hyprlock
/// and greetd appear to need no setup. allock is usually built from source,
/// so borrowing a stock stack keeps `cargo build && allock` working instead
/// of demanding root before the first run. It is a fallback, not the plan —
/// installing `pam/allock` is still the right thing, and it is the only way
/// to choose the stack deliberately.
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

/// What the async side hands back to a blocked conversation callback.
enum Reply {
    Answer(Secret),
    /// Abandon the attempt: makes the callback fail, which unwinds
    /// `authenticate()` so a fresh attempt can start.
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

    // Forward UI commands until either side hangs up. `StartSession` is
    // greeter-only and meaningless here: unlocking reveals the session that
    // is already running, there is nothing to launch.
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

    // Dropping the sender releases a callback blocked in `recv()`, so the
    // thread unwinds instead of holding a PAM handle open forever.
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

    // One context, many attempts: PAM expects a failed `authenticate()` to be
    // retried on the same handle, and reusing it keeps module state (failure
    // delays from pam_faildelay, tally counters) intact across attempts.
    loop {
        let outcome = context
            .authenticate(Flag::NONE)
            .and_then(|()| context.acct_mgmt(Flag::NONE));

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
    }
}

/// Turn a PAM error code into something worth showing on a lock screen.
///
/// Nothing here distinguishes "no such user" from "wrong password" in the UI —
/// on a lock screen the user is fixed so it makes no difference, and keeping
/// one message avoids the habit of leaking account existence.
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
        // CONV_ERR is what our own Abort reply produces; it is not a failure
        // the user should be told about.
        ErrorCode::CONV_ERR | ErrorCode::ABORT => String::new(),
        _ => message.unwrap_or("Authentication failed").to_owned(),
    }
}

struct Conversation {
    events: mpsc::Sender<AuthEvent>,
    replies: sync_mpsc::Receiver<Reply>,
}

impl Conversation {
    fn ask(&mut self, prompt: &CStr, secret: bool) -> Result<CString, ErrorCode> {
        let message = prompt.to_string_lossy().into_owned();
        self.events
            .blocking_send(AuthEvent::Prompt { secret, message })
            .map_err(|_| ErrorCode::CONV_ERR)?;

        match self.replies.recv() {
            // A NUL in the answer cannot come from the text field, but PAM
            // would silently truncate at it, so refuse instead.
            //
            // The one copy of a password outside guido's protected pages:
            // pam-client2 asks for a `CString`, `strdup`s it for PAM — which
            // wipes its own copy — and drops ours unwiped. Out of reach here;
            // the process is undumpable (see `main`), so it cannot reach a
            // core dump. The `Secret` itself is wiped as it drops.
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
