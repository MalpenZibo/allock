//! polkit's side of the conversation, used by the polkit agent.
//!
//! An agent does not run PAM itself: polkitd only takes "this user proved who
//! they are" from root, so the stack runs in `polkit-agent-helper-1`, which
//! reports the verdict to polkitd on its own. Since polkit 126 the helper is
//! not setuid but socket-activated: every connection to
//! [`HELPER_SOCKET`] is a fresh helper, which learns who is asking from the
//! socket's peer credentials.
//!
//! The protocol is lines of text. The agent writes the user to authenticate
//! and the cookie polkitd handed it; the helper answers with
//! `PAM_PROMPT_ECHO_OFF`, `PAM_PROMPT_ECHO_ON`, `PAM_ERROR_MSG` and
//! `PAM_TEXT_INFO` lines, escaped with `g_strescape`, and ends on `SUCCESS` or
//! `FAILURE`. An answer is one line back, unescaped. A retry is a new
//! connection with the same cookie.
//!
//! The first connection is made as the dialog opens, so a finger works
//! without typing anything. After a refusal the next one waits for the next
//! answer: every connection ends in `pam_unix` unless a finger ends it first,
//! and `pam_faillock` counts each one that ends there without a password —
//! a dialog cancelled after a wrong password used to cost two attempts, and
//! with the account locked, a fresh prompt after every refusal read as a
//! screen still willing to take one.
//!
//! The helper runs the `polkit-1` PAM stack in order, so a `pam_fprintd` in it
//! waits for a finger before the password is ever asked for. An answer is
//! written as soon as it is given, not when it is asked for, and waits in the
//! socket: typing the password while the reader waits is how it is used
//! without the finger, as hyprpolkitagent does.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use guido::prelude::Secret;

use super::{AuthCommand, AuthEvent};

pub const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";

/// What the helper refuses without a reason: its `FAILURE` carries none, and
/// the stack may have failed on something other than the password.
const FAILED: &str = "Authentication failed";

/// How one connection to the helper ended.
enum Attempt {
    Succeeded,
    /// Refused. `String` is the last error the stack gave, if it gave one.
    Failed(Option<String>),
    /// The UI asked for a fresh attempt.
    Restarted,
    /// The UI is gone.
    Closed,
    /// The helper cannot be reached or stopped making sense.
    Broken(String),
}

pub async fn run(
    user: String,
    cookie: String,
    mut cmd_rx: mpsc::Receiver<AuthCommand>,
    event_tx: mpsc::Sender<AuthEvent>,
) {
    // The answer the next attempt starts with, once there has been a refusal.
    let mut answer = None;
    loop {
        let event = match attempt(&user, &cookie, answer.take(), &mut cmd_rx, &event_tx).await {
            Attempt::Succeeded => AuthEvent::Success,
            Attempt::Failed(reason) => {
                log::info!("polkit attempt failed");
                AuthEvent::Failed(reason.unwrap_or_else(|| FAILED.to_owned()))
            }
            Attempt::Restarted => continue,
            Attempt::Closed => return,
            Attempt::Broken(reason) => {
                log::error!("polkit helper: {reason}");
                AuthEvent::Ended("Authentication is unavailable".to_owned())
            }
        };
        let again = matches!(event, AuthEvent::Failed(_));
        if event_tx.send(event).await.is_err() {
            return;
        }
        if !again {
            break;
        }
        // The refusal stays on screen, and no helper is asked anything,
        // until there is something to ask it with.
        loop {
            match cmd_rx.recv().await {
                None => return,
                Some(AuthCommand::Respond(given)) => {
                    answer = Some(given);
                    break;
                }
                Some(AuthCommand::Restart) => break,
                // Greeter only.
                Some(AuthCommand::StartSession { .. } | AuthCommand::SetUser(_)) => {}
            }
        }
    }

    // Nothing more to ask, but the conversation is not over until the UI
    // says so: returning would read as a backend that stopped.
    while cmd_rx.recv().await.is_some() {}
}

async fn attempt(
    user: &str,
    cookie: &str,
    answer: Option<Secret>,
    cmd_rx: &mut mpsc::Receiver<AuthCommand>,
    event_tx: &mpsc::Sender<AuthEvent>,
) -> Attempt {
    let stream = match UnixStream::connect(HELPER_SOCKET).await {
        Ok(stream) => stream,
        Err(e) => return Attempt::Broken(format!("cannot connect to {HELPER_SOCKET}: {e}")),
    };
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);

    if let Err(e) = write
        .write_all(format!("{user}\n{cookie}\n").as_bytes())
        .await
    {
        return Attempt::Broken(format!("cannot write to the helper: {e}"));
    }

    // Answers and questions pair up in order, whichever comes first. An
    // answer given ahead of its question is already in the socket, so the
    // question is not shown: the field stays busy with the answer in flight.
    let mut questions_open = 0u32;
    let mut answers_ahead = 0u32;
    let mut answered = false;
    // Only what the stack says after the last answer is about that answer:
    // before it, the fingerprint reader's complaints. Errors in a row are one
    // message over several lines — pam_faillock says the account is locked,
    // then how long for — so they are kept together.
    let mut last_error: Option<String> = None;
    let mut errors_in_a_row = false;
    let mut heard_anything = false;

    if let Some(answer) = answer {
        if let Some(refusal) = unsendable(&answer) {
            return Attempt::Failed(Some(refusal.to_owned()));
        }
        if let Err(e) = send_answer(&mut write, answer.expose()).await {
            return Attempt::Broken(format!("cannot write to the helper: {e}"));
        }
        answers_ahead = 1;
        answered = true;
    }

    loop {
        tokio::select! {
            line = read_line(&mut read) => {
                let line = match line {
                    Ok(Some(line)) => line,
                    // A helper that hangs up without a verdict has failed,
                    // unless it never said anything: then it is not working.
                    Ok(None) if heard_anything => return Attempt::Failed(last_error),
                    Ok(None) => return Attempt::Broken("the helper hung up".to_owned()),
                    Err(e) => return Attempt::Broken(format!("cannot read from the helper: {e}")),
                };
                heard_anything = true;
                let line = parse(&line);
                let continues_error = errors_in_a_row;
                errors_in_a_row = matches!(line, Line::Error(_));
                let event = match line {
                    Line::Prompt { secret, message } => {
                        if answers_ahead > 0 {
                            answers_ahead -= 1;
                            continue;
                        }
                        questions_open += 1;
                        if answered {
                            AuthEvent::Prompt { secret, message }
                        } else {
                            AuthEvent::Asked { secret, message }
                        }
                    }
                    Line::Info(text) => AuthEvent::Info(text),
                    Line::Error(text) => {
                        let text = match last_error.take() {
                            Some(before) if continues_error => format!("{before}\n{text}"),
                            _ => text,
                        };
                        last_error = Some(text.clone());
                        AuthEvent::Error(text)
                    }
                    Line::Success => return Attempt::Succeeded,
                    Line::Failure => return Attempt::Failed(last_error),
                    Line::Unknown => {
                        return Attempt::Broken("unexpected line from the helper".to_owned());
                    }
                };
                if event_tx.send(event).await.is_err() {
                    return Attempt::Closed;
                }
            }
            command = cmd_rx.recv() => match command {
                None => return Attempt::Closed,
                Some(AuthCommand::Respond(answer)) => {
                    if let Some(refusal) = unsendable(&answer) {
                        if event_tx.send(AuthEvent::Failed(refusal.to_owned())).await.is_err() {
                            return Attempt::Closed;
                        }
                        continue;
                    }
                    if let Err(e) = send_answer(&mut write, answer.expose()).await {
                        return Attempt::Broken(format!("cannot write to the helper: {e}"));
                    }
                    if questions_open > 0 {
                        questions_open -= 1;
                    } else {
                        answers_ahead += 1;
                    }
                    answered = true;
                    last_error = None;
                    errors_in_a_row = false;
                }
                Some(AuthCommand::Restart) => return Attempt::Restarted,
                // Greeter only.
                Some(AuthCommand::StartSession { .. } | AuthCommand::SetUser(_)) => {}
            },
        }
    }
}

/// Why `answer` cannot go to the helper, if it cannot.
///
/// The helper reads an answer with `fgets` and keeps it as a C string: a line
/// break would end it early and answer the next question with the rest, a NUL
/// would cut it short.
fn unsendable(answer: &Secret) -> Option<&'static str> {
    answer
        .expose()
        .contains(['\n', '\0'])
        .then_some("That cannot be typed here")
}

/// Write one answer and its line break in a single buffer that is wiped as it
/// drops. The socket's own buffer is the kernel's, and out of reach.
async fn send_answer(write: &mut OwnedWriteHalf, answer: &str) -> std::io::Result<()> {
    let mut line = Zeroizing::new(Vec::with_capacity(answer.len() + 1));
    line.extend_from_slice(answer.as_bytes());
    line.push(b'\n');
    write.write_all(&line).await
}

/// One line from the helper, without its line break. `None` at the end.
async fn read_line(read: &mut BufReader<OwnedReadHalf>) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    if read.read_until(b'\n', &mut line).await? == 0 {
        return Ok(None);
    }
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    Ok(Some(line))
}

#[derive(Debug, PartialEq, Eq)]
enum Line {
    Prompt { secret: bool, message: String },
    Info(String),
    Error(String),
    Success,
    Failure,
    Unknown,
}

/// Read one line the way polkit's own agent library does: unescape the whole
/// line, then look at how it starts.
fn parse(line: &[u8]) -> Line {
    let line = unescape(line);
    let line = String::from_utf8_lossy(&line);
    let rest = |prefix: &str| line.strip_prefix(prefix).map(str::to_owned);

    if let Some(message) = rest("PAM_PROMPT_ECHO_OFF ") {
        Line::Prompt {
            secret: true,
            message,
        }
    } else if let Some(message) = rest("PAM_PROMPT_ECHO_ON ") {
        Line::Prompt {
            secret: false,
            message,
        }
    } else if let Some(text) = rest("PAM_ERROR_MSG ") {
        Line::Error(text)
    } else if let Some(text) = rest("PAM_TEXT_INFO ") {
        Line::Info(text)
    } else if line.starts_with("SUCCESS") {
        Line::Success
    } else if line.starts_with("FAILURE") {
        Line::Failure
    } else {
        Line::Unknown
    }
}

/// `g_strcompress`: undo `g_strescape`.
///
/// The helper escapes every byte outside printable ASCII as three octal
/// digits, so a message in any language but English arrives as `\303\250`
/// and has to be put back together before it is UTF-8 again.
fn unescape(line: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len());
    let mut bytes = line.iter().copied().peekable();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        let Some(escaped) = bytes.next() else {
            // A trailing backslash: GLib stops there too.
            break;
        };
        match escaped {
            b'0'..=b'7' => {
                let mut value = u32::from(escaped - b'0');
                for _ in 0..2 {
                    match bytes.peek() {
                        Some(&digit @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(digit - b'0');
                            bytes.next();
                        }
                        _ => break,
                    }
                }
                // Three octal digits reach 511; GLib keeps the low byte.
                out.push(value as u8);
            }
            b'b' => out.push(b'\x08'),
            b'f' => out.push(b'\x0c'),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(b'\x0b'),
            // `\\`, `\"` and anything GLib does not know: the character itself.
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_what_g_strescape_writes() {
        assert_eq!(unescape(br#"a\\b\"c"#), br#"a\b"c"#);
        assert_eq!(unescape(br"\t\n\r\b\f\v"), b"\t\n\r\x08\x0c\x0b");
        assert_eq!(unescape(br"x\qy"), b"xqy");
    }

    #[test]
    fn octal_escapes_become_utf8_again() {
        // "Appoggia il dito sul lettore" is ASCII; an accent is not.
        let line = br"PAM_TEXT_INFO Perch\303\251 no";
        assert_eq!(parse(line), Line::Info("Perché no".to_owned()));
    }

    #[test]
    fn an_octal_escape_takes_at_most_three_digits() {
        assert_eq!(unescape(br"\1011"), b"A1");
        assert_eq!(unescape(br"\0"), b"\0");
        assert_eq!(unescape(br"\7z"), b"\x07z");
    }

    #[test]
    fn a_trailing_backslash_is_dropped() {
        assert_eq!(unescape(br"end\"), b"end");
    }

    #[test]
    fn reads_every_line_the_helper_writes() {
        assert_eq!(
            parse(b"PAM_PROMPT_ECHO_OFF Password: "),
            Line::Prompt {
                secret: true,
                message: "Password: ".to_owned()
            }
        );
        assert_eq!(
            parse(b"PAM_PROMPT_ECHO_ON login:"),
            Line::Prompt {
                secret: false,
                message: "login:".to_owned()
            }
        );
        assert_eq!(
            parse(b"PAM_TEXT_INFO Place your finger on the fingerprint reader"),
            Line::Info("Place your finger on the fingerprint reader".to_owned())
        );
        assert_eq!(
            parse(b"PAM_ERROR_MSG The account is locked"),
            Line::Error("The account is locked".to_owned())
        );
        assert_eq!(parse(b"SUCCESS"), Line::Success);
        assert_eq!(parse(b"FAILURE"), Line::Failure);
        assert_eq!(parse(b"polkit-agent-helper-1: huh"), Line::Unknown);
    }

    #[test]
    fn an_escaped_prefix_is_still_the_prefix() {
        // polkit unescapes the whole line before looking at it, so this is
        // what it does too.
        assert_eq!(
            parse(br"PAM_TEXT_INFO line one\nline two"),
            Line::Info("line one\nline two".to_owned())
        );
    }
}
