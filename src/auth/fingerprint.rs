//! Fingerprint verification through fprintd, for the lock screen.
//!
//! This runs *beside* the password conversation rather than inside it: PAM's
//! single conversation channel would make `pam_fprintd` block the password
//! field while it waits for a finger, which is the reason most lock screens
//! drive fprintd themselves. The trade is explicit — a `verify-match` from
//! fprintd is accepted as proof of identity, the same trust model hyprlock
//! and the GNOME/KDE lock screens use. `[auth] fingerprint = false` turns it
//! off and requires the password.
//!
//! Everything here fails towards [`FingerprintState::Unavailable`]: no
//! reader, no enrolled finger, no fprintd, a polkit refusal — all of them
//! just mean the password is the only way in. But only once it is clear the
//! failure is not passing: a reader that is busy with a `sudo` in a terminal,
//! or still waking up with the machine, is tried again a little later, and
//! every resume from suspend starts it over from scratch.

use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use guido::prelude::WriteSignal;
use tokio::sync::mpsc;
use zbus::proxy;
use zbus::zvariant::OwnedObjectPath;

use super::{AuthEvent, FingerprintConfig, FingerprintState};

#[proxy(
    interface = "net.reactivated.Fprint.Manager",
    default_service = "net.reactivated.Fprint",
    default_path = "/net/reactivated/Fprint/Manager"
)]
pub trait FprintManager {
    fn get_default_device(&self) -> zbus::Result<OwnedObjectPath>;
}

#[proxy(
    interface = "net.reactivated.Fprint.Device",
    default_service = "net.reactivated.Fprint"
)]
pub trait FprintDevice {
    fn claim(&self, username: &str) -> zbus::Result<()>;
    fn release(&self) -> zbus::Result<()>;
    fn verify_start(&self, finger_name: &str) -> zbus::Result<()>;
    fn verify_stop(&self) -> zbus::Result<()>;
    fn list_enrolled_fingers(&self, username: &str) -> zbus::Result<Vec<String>>;

    #[zbus(signal)]
    fn verify_status(&self, result: String, done: bool) -> zbus::Result<()>;

    /// Whether a finger is on the sensor *right now*.
    ///
    /// The only thing fprintd says between a touch and its verdict —
    /// `VerifyStatus` arrives when the scan is already decided. Without this
    /// the screen has nothing to show for the second or two the reader takes,
    /// which is exactly the second the user is wondering whether it read them
    /// at all. A daemon that never emits a change for it simply leaves the
    /// hint where it was, which is the behaviour this had before.
    #[zbus(property, name = "finger-present")]
    fn finger_present(&self) -> zbus::Result<bool>;
}

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

/// How many times in a row the reader may fail before it is left alone until
/// the next resume. With [`backoff`], a little under a minute: long enough to
/// outlast a `pam_fprintd` holding the reader for a `sudo` in a terminal.
const MAX_FAILURES: u32 = 8;

pub async fn run(
    config: FingerprintConfig,
    event_tx: mpsc::Sender<AuthEvent>,
    state: WriteSignal<FingerprintState>,
) {
    let connection = match zbus::Connection::system().await {
        Ok(connection) => connection,
        Err(e) => {
            log::info!("fingerprint unavailable: {e}");
            state.set(FingerprintState::Unavailable);
            return;
        }
    };
    let mut sleeps = sleep_signals(&connection).await;
    // Across rounds and restarts alike: a suspend is not a way to more tries.
    let mut misses = 0;
    let mut failures = 0;

    loop {
        let woke = tokio::select! {
            ended = verify_loop(&connection, &config, &event_tx, state, &mut misses, &mut failures) => {
                Wake::Ended(ended)
            }
            Some(asleep) = sleeps.next() => Wake::Sleep(asleep),
        };

        match woke {
            // The loop settled on a state of its own — `Exhausted` after too
            // many misses — and it is the one the user has to read.
            // Overwriting it here was how the screen came to say nothing at
            // all after five wrong fingers: the reader had given up, and the
            // only sign of it was the hint vanishing.
            Wake::Ended(Ok(Ended::Settled)) => return,
            // Nothing will change that while the screen is up: nobody enrolls
            // a finger through a lock screen.
            Wake::Ended(Ok(Ended::NoFingers)) => {
                state.set(FingerprintState::Unavailable);
                return;
            }
            Wake::Ended(Ok(Ended::Broken(reason))) => failures = broke(&reason, failures),
            Wake::Ended(Err(e)) => failures = broke(&e.to_string(), failures),
            // The round was dropped, and the claim with it: fprintd cancels
            // whatever the reader was doing as the machine goes down, and a
            // loop that restarted it straight away was how 120 "device busy"
            // errors fitted into the second before the suspend — and how the
            // last of them was still on screen when the lid opened.
            Wake::Sleep(true) => {
                state.set(FingerprintState::Unavailable);
                until_resumed(&mut sleeps).await;
                failures = 0;
                continue;
            }
            // Awake without having been seen going to sleep: start over all
            // the same, the reader may not be the one it was.
            Wake::Sleep(false) => {
                failures = 0;
                continue;
            }
        }

        if failures >= MAX_FAILURES {
            // Not an error the user needs to see: it only means the password
            // is the only way in, which the UI shows by hiding the hint.
            log::info!("fingerprint unavailable until the next resume");
            state.set(FingerprintState::Unavailable);
            until_resumed(&mut sleeps).await;
            failures = 0;
            continue;
        }

        tokio::select! {
            () = tokio::time::sleep(backoff(failures)) => {}
            Some(asleep) = sleeps.next() => {
                if asleep {
                    state.set(FingerprintState::Unavailable);
                    until_resumed(&mut sleeps).await;
                }
                failures = 0;
            }
        }
    }
}

/// What woke the loop in [`run`].
enum Wake {
    Ended(zbus::Result<Ended>),
    /// logind's `PrepareForSleep`: `true` going down, `false` coming back.
    Sleep(bool),
}

/// How one claim of the reader ended.
enum Ended {
    /// The loop has already set the state the user should see.
    Settled,
    /// A reader, but no finger enrolled on it for this user.
    NoFingers,
    /// The reader stopped working mid-way; worth trying again.
    Broken(String),
}

/// Counts one failure, and says so where someone debugging will look.
fn broke(reason: &str, failures: u32) -> u32 {
    log::info!("fingerprint reader failed ({reason}), will retry");
    failures + 1
}

/// How long to leave the reader alone after `failures` failures in a row.
fn backoff(failures: u32) -> Duration {
    Duration::from_secs(1 << failures.saturating_sub(1).min(3))
}

/// logind's `PrepareForSleep`, as `true` going down and `false` coming back.
///
/// Never ends — without logind it simply never says anything — so a `select!`
/// on it cannot spin on a stream that has run dry.
async fn sleep_signals(connection: &zbus::Connection) -> BoxStream<'static, bool> {
    let signals = match Login1ManagerProxy::new(connection).await {
        Ok(logind) => logind.receive_prepare_for_sleep().await.ok(),
        Err(_) => None,
    };
    match signals {
        Some(signals) => signals
            .filter_map(|signal| async move { signal.args().ok().map(|args| args.start) })
            .chain(futures::stream::pending())
            .boxed(),
        None => {
            log::debug!("no logind: the reader will not be restarted on resume");
            futures::stream::pending().boxed()
        }
    }
}

async fn until_resumed(sleeps: &mut BoxStream<'static, bool>) {
    while let Some(asleep) = sleeps.next().await {
        if !asleep {
            return;
        }
    }
}

/// What one `VerifyStatus` means for the screen.
#[derive(Debug, PartialEq)]
enum Step {
    Matched,
    /// A finger was read and it was not the right one.
    Missed,
    /// Something to tell the user about the scan itself.
    Hint(&'static str),
    /// The reader itself failed — not the scan. Ends the round: fprintd has
    /// to be given time before it is asked again.
    Failed,
    Ignored,
}

fn step_for(result: &str) -> Step {
    match result {
        "verify-match" => Step::Matched,
        "verify-no-match" => Step::Missed,
        "verify-retry-scan" => Step::Hint("Try scanning again"),
        "verify-swipe-too-short" => Step::Hint("Swipe was too short"),
        "verify-finger-not-centered" => Step::Hint("Centre your finger"),
        "verify-remove-and-retry" => Step::Hint("Lift your finger and retry"),
        "verify-disconnected" | "verify-unknown-error" => Step::Failed,
        _ => Step::Ignored,
    }
}

/// What to say about a finger that was read and not recognised.
///
/// With the count in it, because the reader stopping is otherwise a surprise:
/// the screen went quiet and the sensor stopped answering, with nothing having
/// said that the tries were running out.
fn miss_hint(misses: u32, retries: u32) -> String {
    match retries.saturating_sub(misses) {
        0 => "Not recognised".to_owned(),
        1 => "Not recognised — one try left".to_owned(),
        left => format!("Not recognised — {left} tries left"),
    }
}

/// What a change in `finger-present` does to the hint beside the sensor.
///
/// A finger arriving is always worth saying. A finger leaving is worth saying
/// only when nothing else has spoken since it arrived — otherwise this is the
/// module's oldest bug in a new place: "Not recognised" would appear and then
/// be wiped a moment later by the finger that earned it lifting off.
fn hint_for_finger(present: bool, reading: bool) -> Option<FingerprintState> {
    match (present, reading) {
        (true, _) => Some(FingerprintState::Reading),
        (false, true) => Some(FingerprintState::Waiting),
        (false, false) => None,
    }
}

async fn verify_loop(
    connection: &zbus::Connection,
    config: &FingerprintConfig,
    event_tx: &mpsc::Sender<AuthEvent>,
    state: WriteSignal<FingerprintState>,
    misses: &mut u32,
    failures: &mut u32,
) -> zbus::Result<Ended> {
    // Asked afresh every time: a reader that went away and came back is a
    // new object at a new path.
    let manager = FprintManagerProxy::new(connection).await?;
    let device_path = manager.get_default_device().await?;

    let device: FprintDeviceProxy<'static> = FprintDeviceProxy::builder(connection)
        .path(device_path)?
        .build()
        .await?;

    if device
        .list_enrolled_fingers(&config.user)
        .await
        .is_ok_and(|fingers| fingers.is_empty())
    {
        log::info!("no enrolled fingers for {}", config.user);
        return Ok(Ended::NoFingers);
    }

    // Claiming is exclusive: hold it only while the screen is up. Dropping
    // this task (unlock, app exit) unwinds through the guard below.
    device.claim(&config.user).await?;
    let _guard = ClaimGuard(device.clone());

    let mut status = device.receive_verify_status().await?;
    let mut finger = device.receive_finger_present_changed().await;
    // Whether the hint on screen is this scan's and nothing else has replaced
    // it — see `hint_for_finger`.
    let mut reading = false;

    // Said once per claim, before the first finger. Setting it again at the
    // top of every round is what wiped the answer to the round before it: a
    // hint would appear for as long as it took fprintd to be asked again, and
    // vanish. After a failure or a resume it is the right thing to wipe: the
    // reader is listening again.
    state.set(FingerprintState::Waiting);

    while *misses < config.retries {
        device.verify_start("any").await?;

        let done = loop {
            let status = tokio::select! {
                status = status.next() => status,
                // The touch, not its verdict: the hint says a finger is being
                // read, and the screen turns its ring for as long as it does.
                Some(change) = finger.next() => {
                    let present = change.get().await.unwrap_or(false);
                    if let Some(hint) = hint_for_finger(present, reading) {
                        state.set(hint);
                    }
                    reading = present;
                    continue;
                }
            };

            let Some(signal) = status else {
                return Ok(Ended::Broken("fprintd went away".to_owned()));
            };
            let Ok(args) = signal.args() else { continue };

            log::debug!("fprintd: {} (done={})", args.result, args.done);

            let step = step_for(&args.result);
            if step != Step::Failed {
                // The reader answered: whatever failed before has passed.
                *failures = 0;
            }

            match step {
                // The only accepting branch in the whole module.
                Step::Matched => {
                    let _ = event_tx.send(AuthEvent::Success).await;
                    let _ = device.verify_stop().await;
                    return Ok(Ended::Settled);
                }
                Step::Missed => {
                    *misses += 1;
                    reading = false;
                    state.set(FingerprintState::Retry(miss_hint(*misses, config.retries)));
                    let _ = event_tx.send(AuthEvent::Rejected).await;
                }
                Step::Hint(hint) => {
                    reading = false;
                    state.set(FingerprintState::Retry(hint.to_owned()));
                }
                // Said while the reader is left alone, and replaced by the
                // invitation as soon as it is listening again.
                Step::Failed => {
                    state.set(FingerprintState::Retry("Fingerprint reader failed".to_owned()));
                    return Ok(Ended::Broken(args.result.to_string()));
                }
                Step::Ignored => {}
            }

            if args.done {
                break true;
            }
        };

        if done {
            // fprintd requires a stop between verification rounds.
            let _ = device.verify_stop().await;
        }
    }

    state.set(FingerprintState::Exhausted);
    Ok(Ended::Settled)
}

/// Releases the device however the loop leaves — including a cancellation of
/// the whole task, which is how the reader gets freed on unlock.
///
/// Belt and braces: fprintd also drops the claim when our bus connection
/// closes, which is what covers the case where the runtime is gone too.
struct ClaimGuard(FprintDeviceProxy<'static>);

impl Drop for ClaimGuard {
    fn drop(&mut self) {
        // The proxy calls are async and we are in `drop`: hand them to the
        // runtime rather than blocking.
        let device = self.0.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = device.verify_stop().await;
                let _ = device.release().await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The result that made the screen go quiet. A finger that is read and
    /// rejected is the most ordinary thing that can happen at a lock screen,
    /// and it was the one status with nothing to say about it: `hint_for`
    /// returned `None` for it and the loop only counted it.
    #[test]
    fn a_finger_that_is_not_recognised_has_something_to_say() {
        assert_eq!(step_for("verify-no-match"), Step::Missed);
        assert!(!miss_hint(1, 5).is_empty());
    }

    #[test]
    fn the_hint_counts_down_to_the_last_try() {
        assert_eq!(miss_hint(1, 5), "Not recognised — 4 tries left");
        assert_eq!(miss_hint(4, 5), "Not recognised — one try left");
        assert_eq!(miss_hint(5, 5), "Not recognised");
        // However the config is set, the count never runs past zero.
        assert_eq!(miss_hint(9, 5), "Not recognised");
    }

    #[test]
    fn a_match_is_the_only_accepting_step() {
        assert_eq!(step_for("verify-match"), Step::Matched);
        for other in [
            "verify-no-match",
            "verify-retry-scan",
            "verify-swipe-too-short",
            "verify-finger-not-centered",
            "verify-remove-and-retry",
            "verify-disconnected",
            "verify-unknown-error",
            "something-fprintd-grew-later",
        ] {
            assert_ne!(step_for(other), Step::Matched, "{other} must not unlock");
        }
    }

    /// The reader used to say nothing at all between the touch and the
    /// verdict, which is the whole second the user is asking whether it read
    /// them.
    #[test]
    fn a_finger_on_the_sensor_says_so() {
        assert_eq!(
            hint_for_finger(true, false),
            Some(FingerprintState::Reading)
        );
    }

    /// The same mistake this module already made once, one signal further out:
    /// the finger that earned "Not recognised" lifts off a moment later, and
    /// the lifting must not be what erases it.
    #[test]
    fn a_finger_leaving_does_not_wipe_the_answer_it_earned() {
        assert_eq!(hint_for_finger(false, false), None);
        // Nothing answered, so the sensor goes back to asking.
        assert_eq!(
            hint_for_finger(false, true),
            Some(FingerprintState::Waiting)
        );
    }

    /// fprintd answers a reader error with `done`, and asking it again at
    /// once only earns the same error: before a suspend that was 120 rounds
    /// in the same second. A failure has to end the round, not hint and loop.
    #[test]
    fn a_reader_error_ends_the_round() {
        assert_eq!(step_for("verify-unknown-error"), Step::Failed);
        assert_eq!(step_for("verify-disconnected"), Step::Failed);
    }

    #[test]
    fn the_reader_is_given_more_time_after_each_failure() {
        let delays: Vec<u64> = (1..=MAX_FAILURES).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 8, 8, 8, 8]);
    }

    /// A status allock has never heard of is not a scan result: saying
    /// something about it would be inventing it.
    #[test]
    fn an_unknown_status_says_nothing() {
        assert_eq!(step_for("verify-something-new"), Step::Ignored);
    }
}
