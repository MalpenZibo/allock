//! polkit agent conversation. PAM runs in `polkit-agent-helper-1`, which
//! reports to polkitd itself; since polkit 126 each connection to
//! [`HELPER_SOCKET`] is a fresh socket-activated helper.
//!
//! Protocol: we write the user and cookie as lines; the helper answers with
//! `g_strescape`d `PAM_*` lines and ends on `SUCCESS` or `FAILURE`. An answer
//! is one unescaped line. A retry is a new connection with the same cookie.
//!
//! After a refusal no connection is made until the next answer, since
//! `pam_faillock` counts each one that reaches `pam_unix` without a password.
//! Answers are written as soon as given, so a password typed while
//! `pam_fprintd` waits is already in the socket when asked for.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use guido::prelude::Secret;

use super::{AuthCommand, AuthEvent};

pub const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";

/// `FAILURE` carries no reason, and it may not be the password.
const FAILED: &str = "Authentication failed";

enum Attempt {
    Succeeded,
    /// The last error the stack gave, if any.
    Failed(Option<String>),
    Restarted,
    Closed,
    Broken(String),
}

pub async fn run(
    user: String,
    cookie: String,
    mut cmd_rx: mpsc::Receiver<AuthCommand>,
    event_tx: mpsc::Sender<AuthEvent>,
) {
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
        // No new helper until there is an answer to give it.
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

    // Returning would read as a backend that stopped.
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

    // Answers and questions pair up in order; a question already answered
    // ahead is not shown.
    let mut questions_open = 0u32;
    let mut answers_ahead = 0u32;
    let mut answered = false;
    // Only errors after the last answer concern it. Consecutive errors are
    // one multi-line message (pam_faillock's lock and its duration).
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
                    // Hanging up silently means the helper is broken.
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

/// The helper reads answers with `fgets` into a C string, so a line break or
/// NUL would split or truncate one.
fn unsendable(answer: &Secret) -> Option<&'static str> {
    answer
        .expose()
        .contains(['\n', '\0'])
        .then_some("That cannot be typed here")
}

/// One buffer, wiped on drop.
async fn send_answer(write: &mut OwnedWriteHalf, answer: &str) -> std::io::Result<()> {
    let mut line = Zeroizing::new(Vec::with_capacity(answer.len() + 1));
    line.extend_from_slice(answer.as_bytes());
    line.push(b'\n');
    write.write_all(&line).await
}

/// Without its line break; `None` at EOF.
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

/// Like polkit's agent library: unescape the whole line, then match the prefix.
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

/// `g_strcompress`: non-ASCII bytes arrive as octal escapes.
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
        assert_eq!(
            parse(br"PAM_TEXT_INFO line one\nline two"),
            Line::Info("line one\nline two".to_owned())
        );
    }
}
