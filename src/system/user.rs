//! Who the lock screen is asking for, and who the greeter can offer.

use std::path::{Path, PathBuf};

use uzers::User;
use uzers::os::unix::UserExt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserInfo {
    pub name: String,
    /// The GECOS full name when set, otherwise the account name.
    pub display_name: String,
    pub avatar: Option<PathBuf>,
}

impl UserInfo {
    fn from_user(user: &User) -> Self {
        Self::new(
            user.name().to_string_lossy().into_owned(),
            &user.gecos().to_string_lossy(),
            user.home_dir(),
        )
    }

    fn new(name: String, gecos: &str, home: &Path) -> Self {
        // GECOS is comma-separated; only the first field is the full name.
        let display_name = gecos
            .split(',')
            .next()
            .map(str::trim)
            .filter(|full_name| !full_name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| name.clone());
        let avatar = avatar(&name, home);
        Self {
            name,
            display_name,
            avatar,
        }
    }
}

/// `~/.face`, then AccountsService's icon, which the greeter user can read
/// when a `0700` home hides `~/.face`.
fn avatar(name: &str, home: &Path) -> Option<PathBuf> {
    [home.join(".face"), home.join(".face.icon")]
        .into_iter()
        .chain([Path::new("/var/lib/AccountsService/icons").join(name)])
        .find(|path| path.is_file())
}

pub fn current() -> UserInfo {
    let uid = uzers::get_current_uid();
    let Some(user) = uzers::get_user_by_uid(uid) else {
        // The lock screen must still come up; PAM rejects an empty name.
        log::error!("no passwd entry for uid {uid}");
        return UserInfo {
            name: String::new(),
            display_name: format!("uid {uid}"),
            avatar: None,
        };
    };
    UserInfo::from_user(&user)
}

pub fn by_name(name: &str) -> Option<UserInfo> {
    uzers::get_user_by_name(name).map(|user| UserInfo::from_user(&user))
}

/// Accounts in `/etc/passwd` with a uid in `login.defs`' `UID_MIN..=UID_MAX`
/// and a login shell. Network-only accounts go through the picker's "Other…".
pub fn login_users() -> Vec<UserInfo> {
    let defs = std::fs::read_to_string("/etc/login.defs").unwrap_or_default();
    let range = uid_range(&defs);
    let passwd = match std::fs::read_to_string("/etc/passwd") {
        Ok(passwd) => passwd,
        Err(e) => {
            log::warn!("cannot read /etc/passwd: {e}; the user picker offers only \"Other…\"");
            return Vec::new();
        }
    };
    login_accounts(&passwd, range)
        .into_iter()
        .map(|(name, gecos, home)| UserInfo::new(name.to_owned(), gecos, Path::new(home)))
        .collect()
}

fn uid_range(login_defs: &str) -> std::ops::RangeInclusive<u32> {
    let value = |key: &str| {
        login_defs.lines().find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some(key))
                .then(|| words.next()?.parse::<u32>().ok())
                .flatten()
        })
    };
    value("UID_MIN").unwrap_or(1000)..=value("UID_MAX").unwrap_or(60000)
}

/// Name, GECOS and home of each account that can log in, in uid order.
fn login_accounts(passwd: &str, uids: std::ops::RangeInclusive<u32>) -> Vec<(&str, &str, &str)> {
    let mut accounts: Vec<(u32, (&str, &str, &str))> = passwd
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let [name, _, uid, _, gecos, home, shell] = fields[..] else {
                return None;
            };
            let uid = uid.parse::<u32>().ok()?;
            let can_log_in =
                !shell.is_empty() && !shell.ends_with("/nologin") && !shell.ends_with("/false");
            (uids.contains(&uid) && can_log_in).then_some((uid, (name, gecos, home)))
        })
        .collect();
    accounts.sort_by_key(|(uid, _)| *uid);
    accounts.into_iter().map(|(_, account)| account).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "\
root:x:0:0::/root:/usr/bin/bash
greeter:x:968:968:greetd greeter user:/:/bin/bash
nobody:x:65534:65534:Kernel Overflow User:/:/usr/bin/nologin
zibo:x:1000:1000:Simone Camito:/home/zibo:/usr/bin/fish
backup:x:1001:1001::/home/backup:/usr/bin/nologin
anna:x:1002:1002::/home/anna:/bin/false
bruno:x:1003:1003::/home/bruno:/bin/bash
malformed line
";

    #[test]
    fn only_people_who_can_log_in_are_listed_in_uid_order() {
        let names: Vec<&str> = login_accounts(PASSWD, 1000..=60000)
            .into_iter()
            .map(|(name, _, _)| name)
            .collect();
        assert_eq!(names, ["zibo", "bruno"]);
    }

    #[test]
    fn a_full_name_is_the_first_gecos_field() {
        let user = UserInfo::new(
            "zibo".to_owned(),
            "Simone Camito,,,",
            Path::new("/nonexistent"),
        );
        assert_eq!(user.display_name, "Simone Camito");
        let user = UserInfo::new("bruno".to_owned(), "", Path::new("/nonexistent"));
        assert_eq!(user.display_name, "bruno");
    }

    #[test]
    fn the_uid_range_comes_from_login_defs() {
        let defs = "# comment\nUID_MIN\t\t\t 1000\nUID_MAX\t\t\t60000\nSYS_UID_MIN 500\n";
        assert_eq!(uid_range(defs), 1000..=60000);
        assert_eq!(uid_range("UID_MIN 500\n"), 500..=60000);
        assert_eq!(uid_range(""), 1000..=60000);
    }
}
