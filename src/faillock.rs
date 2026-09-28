//! Whether `pam_faillock` has locked the account, read ahead of PAM.
//!
//! PAM only says so when the stack reaches `pam_faillock preauth`, and a
//! `pam_fprintd` ahead of it — polkit's stack has one — waits for a finger
//! first: the screen would take a password and only then say that no password
//! could have worked. So the screen reads the tally itself, which the account
//! owns (`faillock --user $USER` reads it the same way), and says so up front.
//!
//! It is a forecast, never a gate. The rule is pam_faillock's, applied to
//! `/etc/security/faillock.conf`; options written on the PAM line instead are
//! out of sight, so the field stays open and PAM has the last word. A finger
//! still works through a lock: `pam_fprintd` runs before `pam_faillock`.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CONFIG: &str = "/etc/security/faillock.conf";

/// `struct tally` in pam_faillock's `faillock.h`: the source, two reserved
/// bytes, the status and the time of the failure.
const RECORD: usize = 64;
const TALLY_STATUS_VALID: u16 = 0x1;

/// A lock in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    /// Lifted at this time.
    Until(SystemTime),
    /// `unlock_time = never`: until an administrator resets the tally.
    UntilReset,
}

impl Lock {
    /// What the screen says about it, at `now`.
    pub fn describe(self, now: SystemTime) -> String {
        match self {
            Lock::Until(until) => {
                let left = until.duration_since(now).unwrap_or_default().as_secs();
                match left.div_ceil(60) {
                    0 | 1 => "Too many attempts — locked for another minute".to_owned(),
                    minutes => format!("Too many attempts — locked for {minutes} more minutes"),
                }
            }
            Lock::UntilReset => "Too many attempts — the account is locked".to_owned(),
        }
    }

    /// When what [`describe`](Self::describe) says next changes: the next
    /// whole minute left, or the end. `None` when it never does.
    pub fn next_change(self, now: SystemTime) -> Option<Duration> {
        let Lock::Until(until) = self else {
            return None;
        };
        let left = until.duration_since(now).unwrap_or_default();
        let into_minute = left.as_secs() % 60;
        let wait = if into_minute == 0 {
            // On a boundary, or under a second from the end.
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

/// The times of the valid failures in a tally file, in seconds.
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

/// pam_faillock's `check_tally`: the failures within `fail_interval` of the
/// latest one, against `deny`, and the lock lasting `unlock_time` from the
/// latest.
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

/// The lock on `user`'s account, as far as can be told. `None` when there is
/// none, or the tally cannot be read.
pub fn status(user: &str) -> Option<Lock> {
    let options = std::fs::read_to_string(CONFIG)
        .map(|config| parse_options(&config))
        .unwrap_or_default();
    let tally = std::fs::read(Path::new(&options.dir).join(user)).ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    lock(&options, &failures(&tally), now)
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
        // The first is more than fail_interval before the latest.
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
    fn counts_down_in_whole_minutes() {
        let now = UNIX_EPOCH + Duration::from_secs(1000);
        let lock = Lock::Until(now + Duration::from_secs(250));
        assert_eq!(
            lock.describe(now),
            "Too many attempts — locked for 5 more minutes"
        );
        // 250 s left: the text changes when 240 are left, in 10 s.
        assert_eq!(lock.next_change(now), Some(Duration::from_millis(10_050)));
        let lock = Lock::Until(now + Duration::from_secs(30));
        assert_eq!(
            lock.describe(now),
            "Too many attempts — locked for another minute"
        );
    }
}
