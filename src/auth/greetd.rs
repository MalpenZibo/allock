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
use guido::prelude::Secret;
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use super::{AuthCommand, AuthEvent};

/// What the UI came back with while the protocol loop was waiting.
enum Answer {
    Text(Secret),
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

/// What goes to greetd next.
enum Outgoing {
    Request(Request),
    /// The answer to a prompt, written by [`write_answer`] so that the
    /// password never passes through a `String`.
    Answer(Secret),
}

/// Where the loop goes after handling one response.
enum Step {
    /// Send this next, staying in the same greetd session.
    Send(Outgoing),
    /// Cancel the session greetd holds, then create a new one. greetd keeps
    /// the session under configuration until told otherwise — after the user
    /// abandons an attempt, and after an error too — and rejects every
    /// `create_session` meanwhile with "a session is already being
    /// configured".
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

        let mut request = Outgoing::Request(Request::CreateSession { username });
        // Distinguishes the two `Success` responses: the first ends the auth
        // conversation, the second acknowledges `StartSession`.
        let mut authenticated = false;

        loop {
            let written = match &request {
                Outgoing::Request(request) => write(&mut stream, request, &event_tx).await,
                Outgoing::Answer(answer) => write_answer(&mut stream, answer, &event_tx).await,
            };
            if !written {
                return;
            }
            let Some(response) = read(&mut stream, &event_tx).await else {
                return;
            };

            match handle(response, &mut authenticated, &mut cmd_rx, &event_tx).await {
                Step::Send(next_request) => request = next_request,
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
                    Answer::Text(answer) => Step::Send(Outgoing::Answer(answer)),
                    Answer::Restart => Step::CancelAndRestart,
                    Answer::User(user) => Step::CancelAndSwitch(user),
                    // Nothing to start before authenticating: answer with
                    // nothing rather than stalling greetd.
                    Answer::Start { .. } => {
                        Step::Send(Outgoing::Request(Request::PostAuthMessageResponse {
                            response: None,
                        }))
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
                Step::Send(Outgoing::Request(Request::PostAuthMessageResponse {
                    response: None,
                }))
            }
        },

        Response::Success if !*authenticated => {
            *authenticated = true;
            let _ = event_tx.send(AuthEvent::Success).await;
            loop {
                match next(cmd_rx).await {
                    Answer::Start { command, env } => {
                        break Step::Send(Outgoing::Request(Request::StartSession {
                            cmd: command,
                            env,
                        }));
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
            // greetd keeps the failed session as the one being configured:
            // it has to be cancelled before the next attempt can start.
            Step::CancelAndRestart
        }

        Response::Error {
            error_type: ErrorType::Error,
            description,
        } => {
            log::error!("greetd error: {description}");
            let _ = event_tx.send(AuthEvent::Error(description)).await;
            // An error that comes back on every attempt would otherwise have
            // the loop and greetd spinning as fast as they can talk.
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            Step::CancelAndRestart
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
            Answer::Text(name) if !name.expose().trim().is_empty() => {
                return Some(name.expose().trim().to_owned());
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
///
/// After an auth error greetd's session worker has already exited, so the
/// cancel may be answered with an error of its own (the worker's socket is
/// closed). greetd has dropped the session all the same; only the connection
/// matters here.
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

/// `post_auth_message_response`, written without the answer passing through a
/// `String` or a buffer that is freed unwiped.
///
/// greetd's framing is a native-endian `u32` length and a JSON body.
/// `greetd_ipc` would take the answer as a `String` and serialize it into a
/// `Vec` it drops as it is; here the JSON is written straight from the
/// secret's `&str` into a buffer reserved large enough never to grow — JSON
/// escapes a byte into at most six — and wiped when it drops.
async fn write_answer(
    stream: &mut UnixStream,
    answer: &Secret,
    event_tx: &mpsc::Sender<AuthEvent>,
) -> bool {
    let Some(body) = encode_answer(answer.expose()) else {
        return false;
    };
    let length = (body.len() as u32).to_ne_bytes();
    let sent = match stream.write_all(&length).await {
        Ok(()) => stream.write_all(&body).await,
        Err(e) => Err(e),
    };
    match sent {
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

/// The JSON body of a `post_auth_message_response`, in a buffer that never
/// reallocates and is wiped when it drops.
fn encode_answer(answer: &str) -> Option<Zeroizing<Vec<u8>>> {
    #[derive(Serialize)]
    struct PostAuthMessageResponse<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        response: &'a str,
    }

    let mut body = Zeroizing::new(Vec::with_capacity(64 + 6 * answer.len()));
    let message = PostAuthMessageResponse {
        kind: "post_auth_message_response",
        response: answer,
    };
    match serde_json::to_writer(&mut *body, &message) {
        Ok(()) => Some(body),
        Err(e) => {
            log::error!("cannot encode the answer for greetd: {e}");
            None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_is_encoded_as_greetd_ipc_would_without_growing() {
        for answer in [
            "hunter2",
            "quo\"te\\back",
            "tab\tnew\nline\u{1}",
            "àèì 🔒",
            "",
        ] {
            let body = encode_answer(answer).unwrap();
            let expected = serde_json::to_vec(&Request::PostAuthMessageResponse {
                response: Some(answer.to_owned()),
            })
            .unwrap();
            assert_eq!(*body, expected, "{answer:?}");
            // Grown means reallocated, and a reallocation frees the old
            // buffer unwiped.
            assert_eq!(body.capacity(), 64 + 6 * answer.len(), "{answer:?}");
        }
    }
}
