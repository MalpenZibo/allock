//! greetd conversation, used by the greeter. The protocol is strictly one
//! response per request, hence a single send/read/decide loop. greetd runs PAM
//! itself; there is no local verdict.

use greetd_ipc::codec::TokioCodec;
use greetd_ipc::{AuthMessageType, ErrorType, Request, Response};
use guido::prelude::Secret;
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use super::{AuthCommand, AuthEvent, Prompt};

enum Outgoing {
    Request(Request),
    /// Written by [`write_answer`], never through a `String`.
    Answer(Secret),
}

enum Step {
    Send(Outgoing),
    /// greetd rejects `create_session` while a session, even a failed one, is
    /// still being configured, so it must be cancelled first.
    CancelAndRestart,
    /// `None` asks for the name.
    CancelAndSwitch(Option<String>),
    Stop,
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

    // `None` asks in the form; once known, the name survives failed attempts.
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
                    .send(AuthEvent::Prompt(Prompt {
                        secret: matches!(kind, AuthMessageType::Secret),
                        message: auth_message,
                    }))
                    .await;

                match cmd_rx.recv().await {
                    Some(AuthCommand::Respond(answer)) => Step::Send(Outgoing::Answer(answer)),
                    Some(AuthCommand::Restart) => Step::CancelAndRestart,
                    Some(AuthCommand::SetUser(user)) => Step::CancelAndSwitch(user),
                    // Answer empty rather than stall greetd.
                    Some(AuthCommand::StartSession { .. }) => {
                        Step::Send(Outgoing::Request(Request::PostAuthMessageResponse {
                            response: None,
                        }))
                    }
                    None => Step::Stop,
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
                match cmd_rx.recv().await {
                    Some(AuthCommand::StartSession { command, env }) => {
                        break Step::Send(Outgoing::Request(Request::StartSession {
                            cmd: command,
                            env,
                        }));
                    }
                    None => break Step::Stop,
                    // Already authenticated: the only thing left that matters
                    // is which session to launch.
                    _ => continue,
                }
            }
        }

        // Acknowledges `StartSession`; greetd starts the session only once
        // the greeter has exited (`greetd-ipc(7)`).
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
            Step::CancelAndRestart
        }

        Response::Error {
            error_type: ErrorType::Error,
            description,
        } => {
            log::error!("greetd error: {description}");
            let _ = event_tx.send(AuthEvent::Error(description)).await;
            // Keeps a persistent error from spinning the loop.
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            Step::CancelAndRestart
        }
    }
}

async fn ask_username(
    cmd_rx: &mut mpsc::Receiver<AuthCommand>,
    event_tx: &mpsc::Sender<AuthEvent>,
) -> Option<String> {
    let _ = event_tx
        .send(AuthEvent::Prompt(Prompt {
            secret: false,
            message: "Username".to_owned(),
        }))
        .await;

    loop {
        match cmd_rx.recv().await {
            Some(AuthCommand::Respond(name)) if !name.expose().trim().is_empty() => {
                return Some(name.expose().trim().to_owned());
            }
            Some(AuthCommand::SetUser(Some(name))) => return Some(name),
            None => return None,
            _ => continue,
        }
    }
}

/// False if the connection is gone. After an auth error the cancel may be
/// answered with an error, but greetd has dropped the session regardless.
async fn cancel(stream: &mut UnixStream, event_tx: &mpsc::Sender<AuthEvent>) -> bool {
    if !write(stream, &Request::CancelSession, event_tx).await {
        return false;
    }
    // Drain the reply, or every later response is off by one.
    read(stream, event_tx).await.is_some()
}

async fn write(
    stream: &mut UnixStream,
    request: &Request,
    event_tx: &mpsc::Sender<AuthEvent>,
) -> bool {
    match request.write_to(stream).await {
        Ok(()) => true,
        Err(e) => lost(event_tx, "write", e).await,
    }
}

/// `post_auth_message_response` without `greetd_ipc`, which would leave the
/// answer in an unwiped `String` and `Vec`. Framing: native-endian `u32`
/// length, then the JSON body.
async fn write_answer(
    stream: &mut UnixStream,
    answer: &Secret,
    event_tx: &mpsc::Sender<AuthEvent>,
) -> bool {
    let Some(body) = encode_answer(answer.expose()) else {
        return false;
    };
    let length = (body.len() as u32).to_ne_bytes();
    let sent = async {
        stream.write_all(&length).await?;
        stream.write_all(&body).await
    };
    match sent.await {
        Ok(()) => true,
        Err(e) => lost(event_tx, "write", e).await,
    }
}

/// In a buffer that never reallocates (JSON escapes a byte into at most six)
/// and is wiped on drop.
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
            lost(event_tx, "read", e).await;
            None
        }
    }
}

/// Says the backend is gone. Always false, for a writer to return.
async fn lost(event_tx: &mpsc::Sender<AuthEvent>, what: &str, e: impl std::fmt::Display) -> bool {
    log::error!("greetd {what} failed: {e}");
    let _ = event_tx
        .send(AuthEvent::Ended("Lost the connection to greetd".to_owned()))
        .await;
    false
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
            // A reallocation would free the old buffer unwiped.
            assert_eq!(body.capacity(), 64 + 6 * answer.len(), "{answer:?}");
        }
    }
}
