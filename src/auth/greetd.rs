//! greetd conversation, used by the greeter.
//!
//! The protocol is strictly request/response over `$GREETD_SOCK`: every
//! request we send gets exactly one response back, so this is written as a
//! "send, read, decide the next request" loop rather than two independent
//! halves. Deciding the next request is where we wait on the UI.
//!
//! Unlike the lock screen there is no local verdict to make — greetd runs the
//! PAM stack itself and only tells us whether it worked.

use greetd_ipc::codec::TokioCodec;
use greetd_ipc::{AuthMessageType, ErrorType, Request, Response};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use super::{AuthCommand, AuthEvent};

/// What the UI came back with while the protocol loop was waiting.
enum Answer {
    Text(String),
    Start {
        command: Vec<String>,
        env: Vec<String>,
    },
    /// Throw the attempt away and start a new session.
    Restart,
    /// Throw the attempt away and start one for this account, or ask for one.
    User(Option<String>),
    /// The UI is gone.
    Closed,
}

/// Where the loop goes after handling one response.
enum Step {
    /// Send this next, staying in the same greetd session.
    Send(Request),
    /// Create a new session. greetd has already torn the old one down — this
    /// is the path after an auth error, where sending `CancelSession` would
    /// itself be an error.
    NewSession,
    /// Cancel the in-flight session first, then create a new one. The path
    /// when the *user* abandons an attempt: greetd still holds the session
    /// open and would reject a second `create_session`.
    CancelAndRestart,
    /// `CancelAndRestart`, for another account — `None` asks for its name.
    CancelAndSwitch(Option<String>),
    /// Nothing left to do.
    Stop,
}

async fn next(cmd_rx: &mut mpsc::Receiver<AuthCommand>) -> Answer {
    match cmd_rx.recv().await {
        Some(AuthCommand::Respond(text)) => Answer::Text(text),
        Some(AuthCommand::StartSession { command, env }) => Answer::Start { command, env },
        Some(AuthCommand::Restart) => Answer::Restart,
        Some(AuthCommand::SetUser(user)) => Answer::User(user),
        None => Answer::Closed,
    }
}

pub async fn run(
    configured_user: Option<String>,
    mut cmd_rx: mpsc::Receiver<AuthCommand>,
    event_tx: mpsc::Sender<AuthEvent>,
) {
    let socket = match std::env::var("GREETD_SOCK") {
        Ok(path) => path,
        Err(_) => {
            let _ = event_tx
                .send(AuthEvent::Ended(
                    "GREETD_SOCK is not set — allock is not running as a greetd greeter".to_owned(),
                ))
                .await;
            return;
        }
    };

    let mut stream = match UnixStream::connect(&socket).await {
        Ok(stream) => stream,
        Err(e) => {
            log::error!("cannot connect to greetd at {socket}: {e}");
            let _ = event_tx
                .send(AuthEvent::Ended("Cannot reach greetd".to_owned()))
                .await;
            return;
        }
    };

    // `None` means "ask in the form". Once known, the name is reused after a
    // failed attempt so the user only retypes the password. The user picker
    // replaces it with `SetUser`.
    let mut user = configured_user.filter(|u| !u.trim().is_empty());

    'session: loop {
        let username = match user.clone() {
            Some(username) => username,
            None => match ask_username(&mut cmd_rx, &event_tx).await {
                Some(username) => {
                    user = Some(username.clone());
                    username
                }
                None => return,
            },
        };

        let mut request = Request::CreateSession { username };
        // Distinguishes the two `Success` responses: the first ends the auth
        // conversation, the second acknowledges `StartSession`.
        let mut authenticated = false;

        loop {
            if !write(&mut stream, &request, &event_tx).await {
                return;
            }
            let Some(response) = read(&mut stream, &event_tx).await else {
                return;
            };

            match handle(response, &mut authenticated, &mut cmd_rx, &event_tx).await {
                Step::Send(next_request) => request = next_request,
                Step::NewSession => continue 'session,
                Step::CancelAndRestart => {
                    if !cancel(&mut stream, &event_tx).await {
                        return;
                    }
                    continue 'session;
                }
                Step::CancelAndSwitch(next_user) => {
                    if !cancel(&mut stream, &event_tx).await {
                        return;
                    }
                    user = next_user;
                    continue 'session;
                }
                Step::Stop => return,
            }
        }
    }
}

async fn handle(
    response: Response,
    authenticated: &mut bool,
    cmd_rx: &mut mpsc::Receiver<AuthCommand>,
    event_tx: &mpsc::Sender<AuthEvent>,
) -> Step {
    match response {
        Response::AuthMessage {
            auth_message_type,
            auth_message,
        } => match auth_message_type {
            kind @ (AuthMessageType::Secret | AuthMessageType::Visible) => {
                let _ = event_tx
                    .send(AuthEvent::Prompt {
                        secret: matches!(kind, AuthMessageType::Secret),
                        message: auth_message,
                    })
                    .await;

                match next(cmd_rx).await {
                    Answer::Text(answer) => Step::Send(Request::PostAuthMessageResponse {
                        response: Some(answer),
                    }),
                    Answer::Restart => Step::CancelAndRestart,
                    Answer::User(user) => Step::CancelAndSwitch(user),
                    // Nothing to start before authenticating: answer with
                    // nothing rather than stalling greetd.
                    Answer::Start { .. } => {
                        Step::Send(Request::PostAuthMessageResponse { response: None })
                    }
                    Answer::Closed => Step::Stop,
                }
            }

            // Info and error messages still need an (empty) answer.
            kind => {
                let event = match kind {
                    AuthMessageType::Error => AuthEvent::Error(auth_message),
                    _ => AuthEvent::Info(auth_message),
                };
                let _ = event_tx.send(event).await;
                Step::Send(Request::PostAuthMessageResponse { response: None })
            }
        },

        Response::Success if !*authenticated => {
            *authenticated = true;
            let _ = event_tx.send(AuthEvent::Success).await;
            loop {
                match next(cmd_rx).await {
                    Answer::Start { command, env } => {
                        break Step::Send(Request::StartSession { cmd: command, env });
                    }
                    Answer::Closed => break Step::Stop,
                    // Already authenticated: the only thing left that matters
                    // is which session to launch.
                    _ => continue,
                }
            }
        }

        // The second Success acknowledges `StartSession`. greetd starts the
        // session only once the greeter process has exited, so the greeter
        // has to go now — see `greetd-ipc(7)`.
        Response::Success => {
            let _ = event_tx
                .send(AuthEvent::Info("Starting session…".to_owned()))
                .await;
            let _ = event_tx.send(AuthEvent::SessionAccepted).await;
            Step::Stop
        }

        Response::Error {
            error_type: ErrorType::AuthError,
            description,
        } => {
            log::info!("greetd auth error: {description}");
            let _ = event_tx
                .send(AuthEvent::Failed("Incorrect password".to_owned()))
                .await;
            // greetd cancels the session itself on an error, so going straight
            // back to CreateSession is right — sending CancelSession as well
            // would itself be an error.
            Step::NewSession
        }

        Response::Error {
            error_type: ErrorType::Error,
            description,
        } => {
            log::error!("greetd error: {description}");
            let _ = event_tx.send(AuthEvent::Error(description)).await;
            Step::NewSession
        }
    }
}

/// Ask the form for a user name, ignoring blank answers.
async fn ask_username(
    cmd_rx: &mut mpsc::Receiver<AuthCommand>,
    event_tx: &mpsc::Sender<AuthEvent>,
) -> Option<String> {
    let _ = event_tx
        .send(AuthEvent::Prompt {
            secret: false,
            message: "Username".to_owned(),
        })
        .await;

    loop {
        match next(cmd_rx).await {
            Answer::Text(name) if !name.trim().is_empty() => {
                return Some(name.trim().to_owned());
            }
            // Picked from the list while the form was asking: that is the
            // answer. "Other…" again is what is already being asked.
            Answer::User(Some(name)) => return Some(name),
            Answer::Closed => return None,
            _ => continue,
        }
    }
}

/// Cancel the in-flight session. Returns false if the connection is gone.
async fn cancel(stream: &mut UnixStream, event_tx: &mpsc::Sender<AuthEvent>) -> bool {
    if !write(stream, &Request::CancelSession, event_tx).await {
        return false;
    }
    // Drain the acknowledgement: leaving it queued would desynchronise every
    // later request from its response.
    read(stream, event_tx).await.is_some()
}

async fn write(
    stream: &mut UnixStream,
    request: &Request,
    event_tx: &mpsc::Sender<AuthEvent>,
) -> bool {
    match request.write_to(stream).await {
        Ok(()) => true,
        Err(e) => {
            log::error!("greetd write failed: {e}");
            let _ = event_tx
                .send(AuthEvent::Ended("Lost the connection to greetd".to_owned()))
                .await;
            false
        }
    }
}

async fn read(stream: &mut UnixStream, event_tx: &mpsc::Sender<AuthEvent>) -> Option<Response> {
    match Response::read_from(stream).await {
        Ok(response) => Some(response),
        Err(e) => {
            log::error!("greetd read failed: {e}");
            let _ = event_tx
                .send(AuthEvent::Ended("Lost the connection to greetd".to_owned()))
                .await;
            None
        }
    }
}
