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
//! just mean the password is the only way in.

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
}

pub async fn run(
    config: FingerprintConfig,
    event_tx: mpsc::Sender<AuthEvent>,
    state: WriteSignal<FingerprintState>,
) {
    match verify_loop(&config, &event_tx, state).await {
        // The loop settled on a state of its own — `Exhausted` after too many
        // misses — and it is the one the user has to read. Overwriting it here
        // was how the screen came to say nothing at all after five wrong
        // fingers: the reader had given up, and the only sign of it was the
        // hint vanishing.
        Ok(Outcome::Settled) => {}
        Ok(Outcome::Unavailable) => state.set(FingerprintState::Unavailable),
        Err(e) => {
            // Not an error the user needs to see: it only means the password
            // is the only way in, which the UI shows by hiding the hint.
            log::info!("fingerprint unavailable: {e}");
            state.set(FingerprintState::Unavailable);
        }
    }
}

/// How the loop ended, for the one decision the caller still has to make.
enum Outcome {
    /// The loop has already set the state the user should see.
    Settled,
    /// Nothing to verify against — no reader, no enrolled finger, no daemon.
    Unavailable,
}

/// What one `VerifyStatus` means for the screen.
#[derive(Debug, PartialEq)]
enum Step {
    Matched,
    /// A finger was read and it was not the right one.
    Missed,
    /// Something to tell the user about the scan itself.
    Hint(&'static str),
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
        "verify-disconnected" | "verify-unknown-error" => Step::Hint("Fingerprint reader failed"),
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

async fn verify_loop(
    config: &FingerprintConfig,
    event_tx: &mpsc::Sender<AuthEvent>,
    state: WriteSignal<FingerprintState>,
) -> zbus::Result<Outcome> {
    let connection = zbus::Connection::system().await?;
    let manager = FprintManagerProxy::new(&connection).await?;
    let device_path = manager.get_default_device().await?;

    let device: FprintDeviceProxy<'static> = FprintDeviceProxy::builder(&connection)
        .path(device_path)?
        .build()
        .await?;

    if device
        .list_enrolled_fingers(&config.user)
        .await
        .is_ok_and(|fingers| fingers.is_empty())
    {
        log::info!("no enrolled fingers for {}", config.user);
        return Ok(Outcome::Unavailable);
    }

    // Claiming is exclusive: hold it only while the screen is up. Dropping
    // this task (unlock, app exit) unwinds through the guard below.
    device.claim(&config.user).await?;
    let _guard = ClaimGuard(device.clone());

    let mut status = device.receive_verify_status().await?;
    let mut misses = 0;

    // Said once, before the first finger. Setting it again at the top of every
    // round is what wiped the answer to the round before it: a hint would
    // appear for as long as it took fprintd to be asked again, and vanish.
    state.set(FingerprintState::Waiting);

    while misses < config.retries {
        device.verify_start("any").await?;

        let done = loop {
            use futures::StreamExt;
            let Some(signal) = status.next().await else {
                return Ok(Outcome::Unavailable);
            };
            let Ok(args) = signal.args() else { continue };

            log::debug!("fprintd: {} (done={})", args.result, args.done);

            match step_for(&args.result) {
                // The only accepting branch in the whole module.
                Step::Matched => {
                    let _ = event_tx.send(AuthEvent::Success).await;
                    let _ = device.verify_stop().await;
                    return Ok(Outcome::Settled);
                }
                Step::Missed => {
                    misses += 1;
                    state.set(FingerprintState::Retry(miss_hint(misses, config.retries)));
                }
                Step::Hint(hint) => state.set(FingerprintState::Retry(hint.to_owned())),
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
    Ok(Outcome::Settled)
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

    /// A status allock has never heard of is not a scan result: saying
    /// something about it would be inventing it.
    #[test]
    fn an_unknown_status_says_nothing() {
        assert_eq!(step_for("verify-something-new"), Step::Ignored);
    }
}
