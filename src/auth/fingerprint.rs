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
    if let Err(e) = verify_loop(&config, &event_tx, state).await {
        // Not an error the user needs to see: it only means the password is
        // the only way in, which the UI already shows by hiding the hint.
        log::info!("fingerprint unavailable: {e}");
    }
    state.set(FingerprintState::Unavailable);
}

async fn verify_loop(
    config: &FingerprintConfig,
    event_tx: &mpsc::Sender<AuthEvent>,
    state: WriteSignal<FingerprintState>,
) -> zbus::Result<()> {
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
        return Ok(());
    }

    // Claiming is exclusive: hold it only while the screen is up. Dropping
    // this task (unlock, app exit) unwinds through the guard below.
    device.claim(&config.user).await?;
    let _guard = ClaimGuard(device.clone());

    let mut status = device.receive_verify_status().await?;
    let mut misses = 0;

    while misses < config.retries {
        device.verify_start("any").await?;
        state.set(FingerprintState::Waiting);

        let done = loop {
            use futures::StreamExt;
            let Some(signal) = status.next().await else {
                return Ok(());
            };
            let Ok(args) = signal.args() else { continue };

            log::debug!("fprintd: {} (done={})", args.result, args.done);

            // The only accepting branch in the whole module.
            if args.result == "verify-match" {
                let _ = event_tx.send(AuthEvent::Success).await;
                let _ = device.verify_stop().await;
                return Ok(());
            }

            if args.result == "verify-no-match" {
                misses += 1;
            }
            if let Some(hint) = hint_for(&args.result) {
                state.set(FingerprintState::Retry(hint.to_owned()));
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
    Ok(())
}

/// fprintd result strings worth repeating to the user. `verify-match` and
/// `verify-no-match` are handled by the caller and deliberately absent.
fn hint_for(result: &str) -> Option<&'static str> {
    match result {
        "verify-retry-scan" => Some("Try scanning again"),
        "verify-swipe-too-short" => Some("Swipe was too short"),
        "verify-finger-not-centered" => Some("Centre your finger"),
        "verify-remove-and-retry" => Some("Lift your finger and retry"),
        "verify-disconnected" | "verify-unknown-error" => Some("Fingerprint reader failed"),
        _ => None,
    }
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
