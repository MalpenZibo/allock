//! The greeter's memory of the last login, kept in the cache directory. It is
//! only ever a suggestion, and failing to read or write it is never fatal.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::SessionDef;

#[derive(Default, Deserialize, Serialize, Debug, PartialEq)]
pub struct LastLogin {
    pub user: Option<String>,
    /// A name, not an index: installing a session would shift indices.
    pub session: Option<String>,
}

impl LastLogin {
    pub fn session_index(&self, sessions: &[SessionDef]) -> usize {
        self.session
            .as_deref()
            .and_then(|name| sessions.iter().position(|session| session.name == name))
            .unwrap_or(0)
    }
}

/// The configured default (case-insensitive), else the remembered session, else the first.
pub fn preselect(sessions: &[SessionDef], default: Option<&str>, last: &LastLogin) -> usize {
    default
        .and_then(|name| {
            sessions
                .iter()
                .position(|session| session.name.eq_ignore_ascii_case(name.trim()))
        })
        .unwrap_or_else(|| last.session_index(sessions))
}

pub fn load() -> LastLogin {
    path().map(|path| read(&path)).unwrap_or_default()
}

pub fn save(user: Option<&str>, session: &str) {
    let Some(path) = path() else {
        log::info!("no writable cache directory; not remembering this login");
        return;
    };
    write(
        &path,
        &LastLogin {
            user: user.map(str::to_owned),
            session: Some(session.to_owned()),
        },
    );
}

fn read(path: &Path) -> LastLogin {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            log::warn!("ignoring unreadable {}: {e}", path.display());
            LastLogin::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => LastLogin::default(),
        Err(e) => {
            log::warn!("cannot read {}: {e}", path.display());
            LastLogin::default()
        }
    }
}

fn write(path: &Path, last: &LastLogin) {
    let text = match toml::to_string(last) {
        Ok(text) => text,
        Err(e) => {
            log::warn!("cannot serialize {last:?}: {e}");
            return;
        }
    };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        log::warn!("cannot create {}: {e}", dir.display());
        return;
    }
    // The greeter user's home may be read-only; never refuse a session over it.
    if let Err(e) = std::fs::write(path, text) {
        log::warn!("cannot write {}: {e}", path.display());
    }
}

fn path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(dir.join("allock").join("last-login.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str) -> SessionDef {
        SessionDef {
            name: name.to_owned(),
            command: vec![name.to_owned()],
            env: Vec::new(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("allock-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("allock").join("last-login.toml")
    }

    #[test]
    fn a_saved_login_comes_back() {
        let path = scratch("roundtrip");
        let saved = LastLogin {
            user: Some("zibo".to_owned()),
            session: Some("niri".to_owned()),
        };

        write(&path, &saved);

        assert_eq!(read(&path), saved, "written to {}", path.display());
    }

    #[test]
    fn nothing_written_yet_is_not_a_failure() {
        assert_eq!(read(&scratch("missing")), LastLogin::default());
    }

    #[test]
    fn a_corrupt_file_is_ignored_rather_than_fatal() {
        let path = scratch("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "this is not toml = = =").unwrap();

        assert_eq!(read(&path), LastLogin::default());
    }

    #[test]
    fn the_picker_starts_on_the_remembered_session() {
        let sessions = [session("sway"), session("niri"), session("hyprland")];
        let last = LastLogin {
            user: None,
            session: Some("niri".to_owned()),
        };

        assert_eq!(last.session_index(&sessions), 1);
    }

    #[test]
    fn a_session_that_no_longer_exists_falls_back_to_the_first() {
        let sessions = [session("sway"), session("niri")];
        let uninstalled = LastLogin {
            user: None,
            session: Some("gnome".to_owned()),
        };

        assert_eq!(uninstalled.session_index(&sessions), 0);
        assert_eq!(LastLogin::default().session_index(&sessions), 0);
    }

    #[test]
    fn an_empty_session_list_answers_zero_without_panicking() {
        let last = LastLogin {
            user: None,
            session: Some("niri".to_owned()),
        };

        assert_eq!(last.session_index(&[]), 0);
    }

    #[test]
    fn a_configured_default_session_wins_over_the_remembered_one() {
        let sessions = [session("Hyprland"), session("Mango"), session("Niri")];
        let last = LastLogin {
            user: None,
            session: Some("Mango".to_owned()),
        };
        assert_eq!(preselect(&sessions, Some("niri"), &last), 2);
        assert_eq!(preselect(&sessions, None, &last), 1);
        assert_eq!(preselect(&sessions, Some("sway"), &last), 1);
    }
}
