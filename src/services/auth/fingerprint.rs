//! Fingerprint verification through fprintd, beside the PAM conversation so
//! `pam_fprintd` cannot block the password field. A fprintd `verify-match` is
//! trusted as proof of identity, as in hyprlock and GNOME/KDE.
//!
//! Every failure falls back to [`FingerprintState::Unavailable`] (password
//! only), after retries for transient ones. The reader is released while the
//! user is idle, since a scanning sensor warms up; a touch cannot wake it.

use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use guido::prelude::WriteSignal;
use tokio::sync::{mpsc, watch};
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

    /// The only signal between a touch and `VerifyStatus`'s verdict.
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

/// Consecutive reader failures before giving up until the next resume. With
/// [`backoff`], just under a minute: enough to outlast a `sudo`'s `pam_fprintd`.
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
    let mut idle = config.idle.clone();
    // Kept across restarts: a suspend must not reset the tries.
    let mut misses = 0;
    let mut failures = 0;

    loop {
        let woke = tokio::select! {
            ended = verify_loop(&connection, &config, &event_tx, state, &mut misses, &mut failures) => {
                Wake::Ended(ended)
            }
            Some(asleep) = sleeps.next() => Wake::Sleep(asleep),
            () = became_idle(&mut idle) => Wake::Idle,
        };

        match woke {
            // The loop already set the state the user must see; keep it.
            Wake::Ended(Ok(Ended::Settled)) => return,
            Wake::Ended(Ok(Ended::NoFingers)) => {
                state.set(FingerprintState::Unavailable);
                return;
            }
            Wake::Ended(Ok(Ended::Broken(reason))) => failures = broke(&reason, failures),
            Wake::Ended(Err(e)) => failures = broke(&e.to_string(), failures),
            // Round and claim dropped; fprintd cancels verification on
            // suspend, so wait for the resume before claiming again.
            Wake::Sleep(true) => {
                state.set(FingerprintState::Unavailable);
                until_resumed(&mut sleeps).await;
                failures = 0;
                continue;
            }
            // Resumed without a seen suspend: the reader may have changed.
            Wake::Sleep(false) => {
                failures = 0;
                continue;
            }
            Wake::Idle => {
                log::debug!("idle: fingerprint reader released");
                state.set(FingerprintState::Resting);
                rest(&mut idle, &mut sleeps).await;
                failures = 0;
                continue;
            }
        }

        if failures >= MAX_FAILURES {
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

enum Wake {
    Ended(zbus::Result<Ended>),
    /// `true` going down, `false` coming back.
    Sleep(bool),
    Idle,
}

enum Ended {
    Settled,
    NoFingers,
    Broken(String),
}

fn broke(reason: &str, failures: u32) -> u32 {
    log::info!("fingerprint reader failed ({reason}), will retry");
    failures + 1
}

fn backoff(failures: u32) -> Duration {
    Duration::from_secs(1 << failures.saturating_sub(1).min(3))
}

/// Never ends, even without logind, so a `select!` on it cannot spin.
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

/// Never ends when idleness cannot be known.
async fn became_idle(idle: &mut Option<watch::Receiver<bool>>) {
    let Some(idle) = idle else {
        return std::future::pending().await;
    };
    if idle.wait_for(|idle| *idle).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Consumes sleep signals meanwhile, or they would restart a reader just
/// reclaimed on the user's return.
async fn rest(idle: &mut Option<watch::Receiver<bool>>, sleeps: &mut BoxStream<'static, bool>) {
    let Some(watch) = idle else { return };
    let gone = loop {
        tokio::select! {
            // The guard `wait_for` returns cannot cross an await.
            gone = async { watch.wait_for(|idle| !*idle).await.is_err() } => break gone,
            Some(_) = sleeps.next() => {}
        }
    };
    if gone {
        *idle = None;
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    Matched,
    Missed,
    Hint(&'static str),
    /// Ends the round: fprintd answers again at once with the same error.
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

fn miss_hint(misses: u32, retries: u32) -> String {
    match retries.saturating_sub(misses) {
        0 => "Not recognised".to_owned(),
        1 => "Not recognised — one try left".to_owned(),
        left => format!("Not recognised — {left} tries left"),
    }
}

/// A finger leaving resets the hint only if no verdict replaced "Reading",
/// so lifting off does not wipe "Not recognised".
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
    // A reader that reappears is at a new path.
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

    // Claiming is exclusive; the guard releases it when the task is dropped.
    device.claim(&config.user).await?;
    let _guard = ClaimGuard(device.clone());

    let mut status = device.receive_verify_status().await?;
    let mut finger = device.receive_finger_present_changed().await;
    let mut reading = false;

    // Once per claim, not per round, or it wipes the last round's verdict.
    state.set(FingerprintState::Waiting);

    while *misses < config.retries {
        device.verify_start("any").await?;

        let done = loop {
            let status = tokio::select! {
                status = status.next() => status,
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
                *failures = 0;
            }

            match step {
                // The only accepting branch in the module.
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
                Step::Failed => {
                    state.set(FingerprintState::Retry(
                        "Fingerprint reader failed".to_owned(),
                    ));
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

/// Releases the device however the loop ends, cancellation included. fprintd
/// also drops the claim when the bus connection closes.
struct ClaimGuard(FprintDeviceProxy<'static>);

impl Drop for ClaimGuard {
    fn drop(&mut self) {
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

    #[test]
    fn a_finger_on_the_sensor_says_so() {
        assert_eq!(
            hint_for_finger(true, false),
            Some(FingerprintState::Reading)
        );
    }

    #[test]
    fn a_finger_leaving_does_not_wipe_the_answer_it_earned() {
        assert_eq!(hint_for_finger(false, false), None);
        assert_eq!(
            hint_for_finger(false, true),
            Some(FingerprintState::Waiting)
        );
    }

    /// Retrying at once only earns the same error, in a tight loop.
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

    #[test]
    fn an_unknown_status_says_nothing() {
        assert_eq!(step_for("verify-something-new"), Step::Ignored);
    }
}
