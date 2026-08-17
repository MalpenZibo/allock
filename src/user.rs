//! Who the lock screen is asking for.

use uzers::os::unix::UserExt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserInfo {
    /// The account name PAM authenticates.
    pub name: String,
    /// The name shown on screen: the GECOS full name when set, otherwise the
    /// account name.
    pub display_name: String,
    /// `~/.face` if it exists, for the avatar.
    pub avatar: Option<std::path::PathBuf>,
}

pub fn current() -> UserInfo {
    let uid = uzers::get_current_uid();
    let Some(user) = uzers::get_user_by_uid(uid) else {
        // No passwd entry for our own uid is pathological, but the lock
        // screen still has to come up: PAM will simply reject an empty name.
        log::error!("no passwd entry for uid {uid}");
        return UserInfo {
            name: String::new(),
            display_name: format!("uid {uid}"),
            avatar: None,
        };
    };

    let name = user.name().to_string_lossy().into_owned();
    // GECOS is comma-separated (full name, office, phones); only the first
    // field is a name.
    let display_name = user
        .gecos()
        .to_string_lossy()
        .split(',')
        .next()
        .map(str::trim)
        .filter(|full_name| !full_name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| name.clone());

    let avatar = [".face", ".face.icon"]
        .iter()
        .map(|file| user.home_dir().join(file))
        .find(|path| path.is_file());

    UserInfo {
        name,
        display_name,
        avatar,
    }
}

/// Look a user up by name, for the greeter's preselected account.
pub fn by_name(name: &str) -> Option<UserInfo> {
    let user = uzers::get_user_by_name(name)?;
    let name = user.name().to_string_lossy().into_owned();
    let display_name = user
        .gecos()
        .to_string_lossy()
        .split(',')
        .next()
        .map(str::trim)
        .filter(|full_name| !full_name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| name.clone());
    let avatar = [".face", ".face.icon"]
        .iter()
        .map(|file| user.home_dir().join(file))
        .find(|path| path.is_file());

    Some(UserInfo {
        name,
        display_name,
        avatar,
    })
}
