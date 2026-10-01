//! allock configuration, read from `~/.config/allock/config.toml`.
//!
//! Every key is optional. A table that names only some of its keys keeps the
//! defaults for the rest, `[appearance]` included.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use guido::prelude::Color;
use hex_color::HexColor;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Top-level Config
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub log_level: String,
    /// Which outputs draw the interface — clock, login card, status row.
    /// Every output is always covered by a lock surface showing the
    /// background; this only decides which of them carry the controls.
    pub main_output: Outputs,
    pub background: BackgroundConfig,
    pub clock: ClockConfig,
    pub auth: AuthConfig,
    pub indicators: IndicatorsConfig,
    pub greeter: GreeterConfig,
    pub appearance: Appearance,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            log_level: "warn".to_owned(),
            main_output: Outputs::default(),
            background: BackgroundConfig::default(),
            clock: ClockConfig::default(),
            auth: AuthConfig::default(),
            indicators: IndicatorsConfig::default(),
            greeter: GreeterConfig::default(),
            appearance: Appearance::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Outputs
// ---------------------------------------------------------------------------

/// Which outputs something is drawn on.
#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub enum Outputs {
    #[default]
    All,
    /// Exact connector name (`eDP-1`) or a substring of "name make model".
    Targets(Vec<String>),
}

impl Outputs {
    pub fn matches(&self, name: &str, make: &str, model: &str) -> bool {
        match self {
            Outputs::All => true,
            Outputs::Targets(targets) => {
                let description = format!("{name} {make} {model}");
                targets
                    .iter()
                    .any(|t| name == t.as_str() || description.contains(t))
            }
        }
    }

    /// Whether the output at `index` is one of the selected ones, given every
    /// connected output described as `(name, make, model)` in the order the
    /// compositor advertised them.
    ///
    /// `All` selects all of them. `Targets` selects each one it names, and
    /// every one it names may be missing: a monitor unplugged while the screen
    /// is locked, or one that was never there. The controls then fall back to
    /// the first output, because a lock screen whose password field is on no
    /// screen at all is a lock screen you cannot get out of.
    pub fn selects(&self, index: usize, all: &[(&str, &str, &str)]) -> bool {
        let Some(&(name, make, model)) = all.get(index) else {
            return false;
        };
        if self.matches(name, make, model) {
            return true;
        }
        // Not named — and nothing else is either, so this one stands in if it
        // is the first. Every surface runs this over the same list, so exactly
        // one of them says yes.
        index == 0
            && !all
                .iter()
                .any(|&(name, make, model)| self.matches(name, make, model))
    }
}

// ---------------------------------------------------------------------------
// Background
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackgroundFit {
    /// Fill the output, cropping the overflow.
    #[default]
    Cover,
    /// Fit inside the output, letterboxing the remainder.
    Contain,
    /// Stretch to the output, ignoring the aspect ratio.
    Stretch,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct BackgroundConfig {
    /// Image to draw behind the form. `~` is expanded. When unset (or
    /// unreadable) the appearance background colour is used instead.
    pub path: Option<String>,
    pub fit: BackgroundFit,
    /// Black scrim over the image, `0.0`–`1.0`. Keeps the form readable on a
    /// bright wallpaper.
    pub dim: f32,
    /// Blur radius in logical pixels for the wallpaper showing through the
    /// cards. `0` turns the frosted effect off and leaves the cards plainly
    /// translucent.
    ///
    /// Handed to guido's `backdrop_blur`. Not the compositor's blur: a lock
    /// surface has nothing composited underneath it, so what softens the
    /// wallpaper is guido's in-surface pass.
    pub blur: f32,
    /// How much of the card tint covers the blurred wallpaper, `0.0`-`1.0`.
    /// Only applies when there is a wallpaper — with none there is nothing
    /// behind the card to reveal, so it stays opaque.
    pub card_opacity: f32,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            path: None,
            fit: BackgroundFit::default(),
            dim: 0.35,
            blur: 32.0,
            card_opacity: 0.72,
        }
    }
}

impl BackgroundConfig {
    pub fn resolved_path(&self) -> Option<PathBuf> {
        let path = expand_tilde(self.path.as_deref()?);
        // Canonicalised, not merely existence-checked. Wallpaper daemons point
        // an extensionless symlink (`~/.config/niri-wallpaper/current`) at the
        // real file, and both guido's decoder and `image::open` choose their
        // format from the *extension* — following the link is what makes the
        // wallpaper decodable at all. It also drops dangling links, which are
        // not images however much they look like a path.
        let resolved = std::fs::canonicalize(&path)
            .inspect_err(|e| log::warn!("background.path {path:?} is unusable: {e}"))
            .ok()?;
        if resolved.is_file() {
            Some(resolved)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ClockConfig {
    /// `strftime` format for the big line.
    pub time_format: String,
    /// `strftime` format for the line under it. Empty hides it.
    pub date_format: String,
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            time_format: "%H:%M".to_owned(),
            date_format: "%A %e %B".to_owned(),
        }
    }
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct AuthConfig {
    /// PAM service name — needs a matching `/etc/pam.d/<name>`. Lock mode
    /// only; the greeter authenticates through greetd instead.
    pub pam_service: String,
    /// Verify against fprintd in parallel with the password.
    ///
    /// This is an *additional* way in, never a replacement: the password
    /// field keeps working and a fingerprint match is accepted on fprintd's
    /// word, the same trust model hyprlock and the GNOME/KDE lock screens
    /// use. Set to `false` to require the password.
    pub fingerprint: bool,
    /// How many consecutive no-match scans before the reader is given up on
    /// for this attempt (fprintd stops on its own after a few anyway).
    pub fingerprint_retries: u32,
    /// Seconds without input before the lock screen rests: the form gives way
    /// to an invitation to press a key, and the fingerprint reader is switched
    /// off until one is — a sensor left scanning while nobody is there warms
    /// up. `0` keeps the form, and the reader, up for as long as the screen
    /// is locked.
    pub idle_timeout: u64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            pam_service: "allock".to_owned(),
            fingerprint: true,
            fingerprint_retries: 5,
            idle_timeout: 60,
        }
    }
}

// ---------------------------------------------------------------------------
// Indicators
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(default)]
pub struct IndicatorsConfig {
    pub battery: bool,
}

impl Default for IndicatorsConfig {
    fn default() -> Self {
        Self { battery: true }
    }
}

// ---------------------------------------------------------------------------
// Greeter
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
pub struct SessionDef {
    /// Shown in the picker.
    pub name: String,
    /// argv handed to greetd's `start_session`.
    pub command: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
}

impl SessionDef {
    /// The argv to start, with its output sent to the journal through
    /// `systemd_cat` when there is one.
    ///
    /// greetd starts a session on the greeter's VT with the console as its
    /// output, so whatever its start script prints — `niri-session`'s
    /// deprecation warning from `systemctl import-environment`, say — is read
    /// on screen between the greeter and the desktop. `systemd-cat` replaces
    /// itself with the command, so what greetd waits on is still the session.
    /// It is tagged with the command's name: `journalctl -t niri-session`.
    pub fn argv(&self, systemd_cat: Option<&std::path::Path>) -> Vec<String> {
        let (Some(systemd_cat), Some(program)) = (systemd_cat, self.command.first()) else {
            return self.command.clone();
        };
        let tag = std::path::Path::new(program)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| program.clone());
        [
            systemd_cat.to_string_lossy().into_owned(),
            "-t".to_owned(),
            tag,
        ]
        .into_iter()
        .chain(self.command.iter().cloned())
        .collect()
    }
}

/// Where a started session's console output goes.
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionOutput {
    /// The journal, through `systemd-cat` — the console when it is missing.
    #[default]
    Journal,
    /// The greeter's VT, as greetd leaves it.
    Console,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct GreeterConfig {
    /// Preselected account in the user picker, over the remembered one. An
    /// account the picker does not list is ignored.
    pub user: Option<String>,
    /// Preselected session, by name, over the remembered one. Case does not
    /// matter; a name no session has is ignored.
    pub default_session: Option<String>,
    /// Where the session's console output goes. See [`SessionDef::argv`].
    pub session_output: SessionOutput,
    /// Offer "Other…" after the listed accounts, for one the list does not
    /// have — a network directory, systemd-homed. Off, only listed accounts
    /// can sign in here, unless nobody is listed: then the form asks anyway.
    pub other_user: bool,
    /// Sessions offered in the picker. Empty means scan the standard
    /// `wayland-sessions` / `xsessions` directories.
    #[serde(rename = "session")]
    pub sessions: Vec<SessionDef>,
    /// Preselect the last user and session that signed in successfully. Off
    /// leaves nothing on disk to read *or* to write — on a shared machine the
    /// last user name is a hint about who uses it.
    pub remember: bool,
}

impl Default for GreeterConfig {
    fn default() -> Self {
        Self {
            user: None,
            default_session: None,
            other_user: true,
            session_output: SessionOutput::Journal,
            sessions: Vec::new(),
            remember: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Appearance
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(untagged)]
pub enum AppearanceColor {
    Simple(HexColor),
    Complete {
        base: HexColor,
        /// Unused by allock's palette, but kept for the same reason as
        /// `text`: the variant is matched untagged.
        #[allow(dead_code)]
        strong: Option<HexColor>,
        weak: Option<HexColor>,
        /// Unused by allock's layout, but the variant is matched untagged:
        /// dropping the field would make a colour that sets it fail to
        /// deserialize and silently fall back to the defaults.
        #[allow(dead_code)]
        text: Option<HexColor>,
    },
}

impl AppearanceColor {
    pub fn base(&self) -> Color {
        match self {
            AppearanceColor::Simple(c) => hex_to_color(*c),
            AppearanceColor::Complete { base, .. } => hex_to_color(*base),
        }
    }

    pub fn weak(&self) -> Option<Color> {
        match self {
            AppearanceColor::Simple(_) => None,
            AppearanceColor::Complete { weak, .. } => weak.map(hex_to_color),
        }
    }
}

fn hex_to_color(c: HexColor) -> Color {
    Color::rgb(c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0)
}

static PRIMARY: HexColor = HexColor::rgb(250, 179, 135);

/// Colours and font. Unknown keys are ignored rather than rejected.
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Appearance {
    pub font_name: Option<String>,
    /// Opacity of the login card over the background.
    pub opacity: f32,
    pub background_color: AppearanceColor,
    pub primary_color: AppearanceColor,
    pub warning_color: AppearanceColor,
    pub success_color: AppearanceColor,
    pub danger_color: AppearanceColor,
    pub text_color: AppearanceColor,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            font_name: None,
            opacity: 1.0,
            background_color: AppearanceColor::Complete {
                base: HexColor::rgb(30, 30, 46),
                strong: Some(HexColor::rgb(69, 71, 90)),
                weak: Some(HexColor::rgb(49, 50, 68)),
                text: None,
            },
            primary_color: AppearanceColor::Complete {
                base: PRIMARY,
                strong: None,
                weak: None,
                text: Some(HexColor::rgb(30, 30, 46)),
            },
            warning_color: AppearanceColor::Simple(HexColor::rgb(224, 175, 104)),
            success_color: AppearanceColor::Simple(HexColor::rgb(166, 227, 161)),
            danger_color: AppearanceColor::Simple(HexColor::rgb(243, 139, 168)),
            text_color: AppearanceColor::Simple(HexColor::rgb(205, 214, 244)),
        }
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

pub fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".config")
        .join("allock")
        .join("config.toml")
}

fn read_toml(path: &Path) -> Option<toml::Table> {
    let content = std::fs::read_to_string(path).ok()?;
    match toml::from_str::<toml::Table>(&content) {
        Ok(table) => Some(table),
        Err(e) => {
            log::warn!("Failed to parse {path:?}: {e}");
            None
        }
    }
}

/// Read just `log_level`, before the logger exists.
///
/// The logger has to be up before [`load`] runs, or every diagnostic about
/// loading the config — which file was found, which `[appearance]` was
/// imported, what failed to parse — is emitted into a logger that is not
/// listening yet. Reading the file twice is cheaper than losing those.
pub fn log_level(path: &Path) -> String {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| toml::from_str::<toml::Table>(&content).ok())
        .and_then(|table| {
            table
                .get("log_level")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| Config::default().log_level)
}

/// Load allock's config, falling back to the defaults for anything it does
/// not say — or for all of it, when the file is missing or malformed.
pub fn load(path: &Path) -> Config {
    let table = read_toml(path).unwrap_or_default();
    if !path.exists() {
        log::info!("No config file at {path:?}, using defaults");
    }

    match toml::Value::Table(table).try_into::<Config>() {
        Ok(config) => config,
        Err(e) => {
            log::warn!("Failed to load config {path:?}: {e}; using defaults");
            Config::default()
        }
    }
}

/// Sessions parsed out of the standard desktop-entry directories, used when
/// `[greeter]` lists none. Keyed by name so duplicates across dirs collapse.
pub fn scan_sessions() -> Vec<SessionDef> {
    const DIRS: [(&str, &str); 4] = [
        ("/usr/share/wayland-sessions", "wayland"),
        ("/usr/local/share/wayland-sessions", "wayland"),
        ("/usr/share/xsessions", "x11"),
        ("/usr/local/share/xsessions", "x11"),
    ];

    let mut found: HashMap<String, SessionDef> = HashMap::new();
    for (dir, kind) in DIRS {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(session) = parse_desktop_entry(&content, kind) else {
                continue;
            };
            found.entry(session.name.clone()).or_insert(session);
        }
    }

    let mut sessions: Vec<SessionDef> = found.into_values().collect();
    sessions.sort_by(|a, b| a.name.cmp(&b.name));
    sessions
}

/// Minimal desktop-entry reader: `Name` and `Exec` from the `[Desktop Entry]`
/// group, skipping hidden entries. Field codes (`%f`, `%U`, …) are dropped —
/// a session never receives file arguments.
fn parse_desktop_entry(content: &str, kind: &str) -> Option<SessionDef> {
    let mut in_group = false;
    let mut name = None;
    let mut exec = None;
    let mut hidden = false;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            // Localised variants (`Name[it]`) are deliberately skipped: the
            // greeter has no locale of its own to pick between them.
            "Name" => name = Some(value.trim().to_owned()),
            "Exec" => exec = Some(value.trim().to_owned()),
            "Hidden" | "NoDisplay" => hidden |= value.trim() == "true",
            _ => {}
        }
    }

    if hidden {
        return None;
    }
    let command: Vec<String> = exec?
        .split_whitespace()
        .filter(|token| !(token.len() == 2 && token.starts_with('%')))
        .map(str::to_owned)
        .collect();
    if command.is_empty() {
        return None;
    }

    Some(SessionDef {
        name: name.unwrap_or_else(|| command[0].clone()),
        command,
        env: vec![format!("XDG_SESSION_TYPE={kind}")],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_config_parses_and_shows_the_defaults() {
        let example: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        let (shown, defaults) = (example.appearance, Appearance::default());
        for (shown, default) in [
            (shown.background_color, defaults.background_color),
            (shown.primary_color, defaults.primary_color),
            (shown.text_color, defaults.text_color),
            (shown.success_color, defaults.success_color),
            (shown.warning_color, defaults.warning_color),
            (shown.danger_color, defaults.danger_color),
        ] {
            assert_eq!(shown.base(), default.base());
            assert_eq!(shown.weak(), default.weak());
        }
        assert_eq!(shown.opacity, defaults.opacity);
    }

    #[test]
    fn a_partial_appearance_keeps_the_other_defaults() {
        let config: Config = toml::from_str("[appearance]\nprimary_color = \"#333333\"").unwrap();
        let defaults = Appearance::default();
        assert_eq!(
            config.appearance.primary_color.base(),
            Color::rgb(0.2, 0.2, 0.2)
        );
        assert_eq!(
            config.appearance.text_color.base(),
            defaults.text_color.base()
        );
        assert_eq!(config.appearance.opacity, defaults.opacity);
    }

    #[test]
    fn desktop_entry_drops_field_codes_and_hidden() {
        let entry = "[Desktop Entry]\nName=Niri\nExec=niri-session %U\n";
        let parsed = parse_desktop_entry(entry, "wayland").unwrap();
        assert_eq!(parsed.name, "Niri");
        assert_eq!(parsed.command, vec!["niri-session"]);

        let hidden = "[Desktop Entry]\nName=Nope\nExec=nope\nNoDisplay=true\n";
        assert!(parse_desktop_entry(hidden, "wayland").is_none());
    }

    #[test]
    fn desktop_entry_ignores_other_groups() {
        let entry = "[Desktop Action X]\nExec=wrong\n[Desktop Entry]\nName=Ok\nExec=right\n";
        assert_eq!(
            parse_desktop_entry(entry, "wayland").unwrap().command,
            vec!["right"]
        );
    }

    #[test]
    fn outputs_targets_match_name_or_description() {
        let outputs = Outputs::Targets(vec!["eDP-1".into(), "Dell".into()]);
        assert!(outputs.matches("eDP-1", "BOE", "0x0BCA"));
        assert!(outputs.matches("DP-2", "Dell Inc.", "U2720Q"));
        assert!(!outputs.matches("HDMI-A-1", "Acme", "X"));
        assert!(Outputs::All.matches("anything", "", ""));
    }

    #[test]
    fn main_output_reads_both_shapes() {
        // The key and its two spellings are the user-facing contract; a typo
        // here would fall back to the defaults with only a log line to say so.
        let all: Config = toml::from_str(r#"main_output = "All""#).unwrap();
        assert_eq!(all.main_output, Outputs::All);

        let targets: Config = toml::from_str(r#"main_output = { Targets = ["DP-2"] }"#).unwrap();
        assert_eq!(
            targets.main_output,
            Outputs::Targets(vec!["DP-2".to_owned()])
        );

        assert_eq!(Config::default().main_output, Outputs::All);
    }

    const LAPTOP: (&str, &str, &str) = ("eDP-1", "BOE", "0x0BCA");
    const DESK: (&str, &str, &str) = ("DP-2", "Dell Inc.", "U2720Q");
    const TV: (&str, &str, &str) = ("HDMI-A-1", "Acme", "X");

    #[test]
    fn all_selects_every_output() {
        let all = [LAPTOP, DESK];
        assert!(Outputs::All.selects(0, &all));
        assert!(Outputs::All.selects(1, &all));
    }

    #[test]
    fn targets_select_every_match_and_nothing_else() {
        let outputs = Outputs::Targets(vec!["DP-2".into()]);
        let all = [LAPTOP, DESK, TV];
        assert!(!outputs.selects(0, &all));
        assert!(outputs.selects(1, &all));
        assert!(!outputs.selects(2, &all));
    }

    #[test]
    fn several_targets_mirror() {
        let outputs = Outputs::Targets(vec!["DP-2".into(), "eDP-1".into()]);
        let all = [LAPTOP, DESK, TV];
        assert!(outputs.selects(0, &all));
        assert!(outputs.selects(1, &all));
        assert!(!outputs.selects(2, &all));
    }

    #[test]
    fn the_first_output_stands_in_for_a_target_that_is_not_there() {
        // The monitor named in the config was unplugged, or never arrived.
        let outputs = Outputs::Targets(vec!["DP-2".into()]);
        let all = [LAPTOP, TV];
        assert!(outputs.selects(0, &all));
        assert!(!outputs.selects(1, &all));
    }

    #[test]
    fn the_stand_in_steps_aside_when_the_target_comes_back() {
        let outputs = Outputs::Targets(vec!["DP-2".into()]);
        // Hotplugged back in, appended to the list it left.
        let all = [LAPTOP, TV, DESK];
        assert!(!outputs.selects(0, &all));
        assert!(outputs.selects(2, &all));
    }

    #[test]
    fn an_output_that_is_gone_selects_nothing() {
        let all: [(&str, &str, &str); 0] = [];
        assert!(!Outputs::All.selects(0, &all));
        assert!(!Outputs::Targets(vec!["DP-2".into()]).selects(0, &all));
    }

    #[test]
    fn a_session_goes_through_systemd_cat_when_there_is_one() {
        let niri = SessionDef {
            name: "Niri".to_owned(),
            command: vec!["/usr/bin/niri-session".to_owned(), "--flag".to_owned()],
            env: Vec::new(),
        };
        let cat = std::path::Path::new("/usr/bin/systemd-cat");
        assert_eq!(
            niri.argv(Some(cat)),
            [
                "/usr/bin/systemd-cat",
                "-t",
                "niri-session",
                "/usr/bin/niri-session",
                "--flag"
            ]
        );
        assert_eq!(niri.argv(None), niri.command);
    }
}
