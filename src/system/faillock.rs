//! Whether `pam_faillock` has locked the account, read from the tally ahead of
//! PAM, since a `pam_fprintd` before `preauth` delays PAM's own answer. It is a
//! forecast, never a gate: options on the PAM line are invisible here.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CONFIG: &str = "/etc/security/faillock.conf";

/// `struct tally` in pam_faillock's `faillock.h`: source, reserved, status, time.
const RECORD: usize = 64;
const TALLY_STATUS_VALID: u16 = 0x1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    Until(SystemTime),
    /// `unlock_time = never`: until an administrator resets the tally.
    UntilReset,
}

impl Lock {
    /// When to read the tally again: on each minute of the lock, and as it ends.
    pub fn next_change(self, now: SystemTime) -> Option<Duration> {
        let Lock::Until(until) = self else {
            return None;
        };
        let left = until.duration_since(now).unwrap_or_default();
        let into_minute = left.as_secs() % 60;
        let wait = if into_minute == 0 {
            left.min(Duration::from_secs(60))
        } else {
            Duration::from_secs(into_minute)
        };
        Some(wait + Duration::from_millis(50))
    }
}

/// The options that decide a lock, with pam_faillock's defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    dir: String,
    deny: u32,
    fail_interval: u64,
    /// `0` is `never`.
    unlock_time: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            dir: "/var/run/faillock".to_owned(),
            deny: 3,
            fail_interval: 900,
            unlock_time: 600,
        }
    }
}

fn parse_options(config: &str) -> Options {
    let mut options = Options::default();
    for line in config.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "dir" if !value.is_empty() => options.dir = value.to_owned(),
            "deny" => options.deny = value.parse().unwrap_or(options.deny),
            "fail_interval" => {
                options.fail_interval = value.parse().unwrap_or(options.fail_interval)
            }
            "unlock_time" if value == "never" => options.unlock_time = 0,
            "unlock_time" => options.unlock_time = value.parse().unwrap_or(options.unlock_time),
            _ => {}
        }
    }
    options
}

fn failures(tally: &[u8]) -> Vec<u64> {
    tally
        .chunks_exact(RECORD)
        .filter_map(|record| {
            let status = u16::from_ne_bytes([record[54], record[55]]);
            let time = u64::from_ne_bytes(record[56..64].try_into().ok()?);
            (status & TALLY_STATUS_VALID != 0).then_some(time)
        })
        .collect()
}

/// Mirrors pam_faillock's `check_tally`.
fn lock(options: &Options, failures: &[u64], now: u64) -> Option<Lock> {
    let latest = *failures.iter().max()?;
    let recent = failures
        .iter()
        .filter(|&&time| latest - time < options.fail_interval)
        .count();
    if options.deny == 0 || recent < options.deny as usize {
        return None;
    }
    if options.unlock_time == 0 {
        return Some(Lock::UntilReset);
    }
    let until = latest.saturating_add(options.unlock_time);
    (until >= now).then(|| Lock::Until(UNIX_EPOCH + Duration::from_secs(until)))
}

/// How many of the attempts `pam_faillock` allows have been used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    /// Failures still inside `fail_interval`.
    pub failed: u32,
    /// How many lock the account.
    pub deny: u32,
    /// How long that lock lasts; `None` until an administrator resets it.
    pub lockout: Option<Duration>,
}

impl Tally {
    pub fn left(&self) -> u32 {
        self.deny.saturating_sub(self.failed)
    }
}

/// Mirrors pam_faillock's window: a failure older than `fail_interval` no
/// longer counts towards a lock.
fn tally(options: &Options, failures: &[u64], now: u64) -> Option<Tally> {
    if options.deny == 0 {
        return None;
    }
    let failed = failures
        .iter()
        .filter(|&&time| now.saturating_sub(time) < options.fail_interval)
        .count();
    Some(Tally {
        failed: u32::try_from(failed).unwrap_or(u32::MAX),
        deny: options.deny,
        lockout: (options.unlock_time > 0).then(|| Duration::from_secs(options.unlock_time)),
    })
}

/// The lock, and the attempts used towards one. No tally file yet is no
/// failure yet.
pub fn status(user: &str) -> (Option<Lock>, Option<Tally>) {
    let options = std::fs::read_to_string(CONFIG)
        .map(|config| parse_options(&config))
        .unwrap_or_default();
    let Ok(now) = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|now| now.as_secs())
    else {
        return (None, None);
    };
    let failures = match std::fs::read(Path::new(&options.dir).join(user)) {
        Ok(record) => failures(&record),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        // Unreadable: say nothing rather than a count that may be wrong.
        Err(_) => return (None, None),
    };
    (
        lock(&options, &failures, now),
        tally(&options, &failures, now),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(status: u16, time: u64) -> Vec<u8> {
        let mut record = vec![0u8; RECORD];
        record[..8].copy_from_slice(b"polkit-1");
        record[54..56].copy_from_slice(&status.to_ne_bytes());
        record[56..64].copy_from_slice(&time.to_ne_bytes());
        record
    }

    #[test]
    fn counts_the_failures_inside_the_interval() {
        let options = Options::default();
        let now = 10_000;
        let tally = tally(&options, &[now - 1000, now - 100, now - 10], now).unwrap();
        assert_eq!((tally.failed, tally.deny, tally.left()), (2, 3, 1));
        assert_eq!(tally.lockout, Some(Duration::from_secs(600)));
        let unlimited = Options {
            deny: 0,
            ..Options::default()
        };
        assert_eq!(super::tally(&unlimited, &[now], now), None);
    }

    #[test]
    fn reads_the_valid_records() {
        let tally = [record(1, 100), record(0, 200), record(1, 300)].concat();
        assert_eq!(failures(&tally), vec![100, 300]);
    }

    #[test]
    fn three_failures_lock_for_ten_minutes_from_the_last() {
        let options = Options::default();
        let lock = lock(&options, &[1000, 1100, 1200], 1300);
        assert_eq!(
            lock,
            Some(Lock::Until(UNIX_EPOCH + Duration::from_secs(1800)))
        );
    }

    #[test]
    fn the_lock_lifts_after_unlock_time() {
        assert_eq!(lock(&Options::default(), &[1000, 1100, 1200], 1801), None);
    }

    #[test]
    fn failures_too_far_apart_do_not_add_up() {
        assert_eq!(lock(&Options::default(), &[0, 1000, 1200], 1300), None);
    }

    #[test]
    fn two_failures_are_not_a_lock() {
        assert_eq!(lock(&Options::default(), &[1000, 1100], 1200), None);
    }

    #[test]
    fn never_means_until_reset() {
        let options = parse_options("unlock_time = never\n");
        assert_eq!(lock(&options, &[1, 2, 3], 4), Some(Lock::UntilReset));
    }

    #[test]
    fn deny_zero_never_locks() {
        let options = parse_options("deny = 0");
        assert_eq!(lock(&options, &[1, 2, 3, 4], 5), None);
    }

    #[test]
    fn reads_the_config_and_ignores_comments() {
        let options = parse_options(
            "# deny = 9\n dir = /tmp/tally \ndeny=5 # five\nfail_interval = 60\nsilent\n",
        );
        assert_eq!(
            options,
            Options {
                dir: "/tmp/tally".to_owned(),
                deny: 5,
                fail_interval: 60,
                unlock_time: 600,
            }
        );
    }

    #[test]
    fn is_read_again_on_the_minute_and_at_the_end() {
        let now = UNIX_EPOCH + Duration::from_secs(1000);
        let lock = Lock::Until(now + Duration::from_secs(250));
        assert_eq!(lock.next_change(now), Some(Duration::from_millis(10_050)));
        let lock = Lock::Until(now + Duration::from_secs(60));
        assert_eq!(lock.next_change(now), Some(Duration::from_millis(60_050)));
        assert_eq!(Lock::UntilReset.next_change(now), None);
    }
}
