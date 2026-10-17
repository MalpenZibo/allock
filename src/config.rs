//! allock configuration, read from `~/.config/allock/config.toml`.
//!
//! Every key is optional. A table that names only some of its keys keeps the
//! defaults for the rest, `[appearance]` included.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use guido::prelude::Color;
use hex_color::HexColor;
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub log_level: String,
    /// Which outputs show the clock and login card; every output is still locked.
    pub main_output: Outputs,
    pub background: BackgroundConfig,
    pub clock: ClockConfig,
    pub auth: AuthConfig,
    pub indicators: IndicatorsConfig,
    pub greeter: GreeterConfig,
    pub appearance: Appearance,
    pub island: IslandConfig,
    pub handoff: HandoffConfig,
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
            island: IslandConfig::default(),
            handoff: HandoffConfig::default(),
        }
    }
}

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

    /// Whether the output at `index` of `all` is selected. When no target is
    /// connected the first output stands in, so the password field is always
    /// on some screen.
    pub fn selects(&self, index: usize, all: &[(&str, &str, &str)]) -> bool {
        let Some(&(name, make, model)) = all.get(index) else {
            return false;
        };
        if self.matches(name, make, model) {
            return true;
        }
        // Every surface runs this over the same list, so exactly one stands in.
        index == 0
            && !all
                .iter()
                .any(|&(name, make, model)| self.matches(name, make, model))
    }
}

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
    /// Image behind the form, `~` expanded; unset or unreadable uses the background colour.
    pub path: Option<String>,
    pub fit: BackgroundFit,
    /// Black scrim over the image while someone is at the card, `0.0`–`1.0`.
    /// Resting darkens it further.
    pub dim: f32,
    /// Blur radius of the image in logical pixels; `0` leaves it sharp.
    pub blur: f32,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            path: None,
            fit: BackgroundFit::default(),
            dim: 0.45,
            blur: 26.0,
        }
    }
}

impl BackgroundConfig {
    pub fn resolved_path(&self) -> Option<PathBuf> {
        let path = expand_tilde(self.path.as_deref()?);
        // Canonicalised because wallpaper daemons use extensionless symlinks and
        // the decoders pick the format from the extension; drops dangling links too.
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

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct AuthConfig {
    /// PAM service name, with a matching `/etc/pam.d/<name>`. Lock mode only.
    pub pam_service: String,
    /// Accept an fprintd match alongside the password; `false` requires the password.
    pub fingerprint: bool,
    /// Consecutive no-match scans before the reader is given up on.
    pub fingerprint_retries: u32,
    /// Seconds without input before the form and fingerprint reader rest; `0` disables.
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
    /// The argv to start, through `systemd_cat` when given so the session's
    /// console output goes to the journal tagged with the command's name.
    /// `systemd-cat` execs the command, so greetd still waits on the session.
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
    /// Account preselected in the user picker, over the remembered one.
    pub user: Option<String>,
    /// Session preselected by name, case-insensitive, over the remembered one.
    pub default_session: Option<String>,
    /// Where the session's console output goes.
    pub session_output: SessionOutput,
    /// Offer "Other…" to type an account the picker does not list.
    pub other_user: bool,
    /// Sessions offered in the picker; empty scans `wayland-sessions` and `xsessions`.
    #[serde(rename = "session")]
    pub sessions: Vec<SessionDef>,
    /// Preselect the last user and session that signed in; off stores nothing on disk.
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

/// Colours and font. Unknown keys are ignored rather than rejected.
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Appearance {
    pub font_name: Option<String>,
    /// The island, and the seed it starts and ends as.
    pub island_color: HexColor,
    /// Behind the wallpaper, and instead of it when there is none.
    pub background_color: HexColor,
    pub text_color: HexColor,
    pub primary_color: HexColor,
    pub success_color: HexColor,
    pub warning_color: HexColor,
    pub danger_color: HexColor,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            font_name: None,
            island_color: HexColor::rgb(5, 5, 8),
            background_color: HexColor::rgb(11, 11, 18),
            text_color: HexColor::rgb(205, 214, 244),
            primary_color: HexColor::rgb(250, 179, 135),
            success_color: HexColor::rgb(166, 227, 161),
            warning_color: HexColor::rgb(249, 226, 175),
            danger_color: HexColor::rgb(243, 139, 168),
        }
    }
}

pub fn color(c: HexColor) -> Color {
    Color::rgb(c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0)
}

/// The island's resting place. ashell reads the same `[island.seed]`, so the
/// dot both processes draw across the compositor's cut is the same dot.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct IslandConfig {
    pub seed: SeedConfig,
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(default)]
pub struct SeedConfig {
    /// Diameter of the seed, in logical pixels.
    pub size: f32,
    /// From the top edge of the output to the seed, in logical pixels.
    pub top: f32,
}

impl Default for SeedConfig {
    fn default() -> Self {
        Self {
            size: 30.0,
            top: 6.0,
        }
    }
}

/// Coordination with a bar that folds its island into the seed before the
/// lock takes the screen, over `org.ashell.Island1`. Optional: without a bar
/// allock locks at once.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(default)]
pub struct HandoffConfig {
    pub enabled: bool,
    /// How long the bar has to fold before the session is locked anyway.
    pub timeout_ms: u64,
}

impl Default for HandoffConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_ms: 600,
        }
    }
}

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

/// Read just `log_level`, so the logger is up before [`load`] logs anything.
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
            // `Name[it]` and the like are skipped: the greeter has no locale.
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
            (shown.island_color, defaults.island_color),
            (shown.background_color, defaults.background_color),
            (shown.primary_color, defaults.primary_color),
            (shown.text_color, defaults.text_color),
            (shown.success_color, defaults.success_color),
            (shown.warning_color, defaults.warning_color),
            (shown.danger_color, defaults.danger_color),
        ] {
            assert_eq!(shown, default);
        }
        let (seed, default) = (example.island.seed, SeedConfig::default());
        assert_eq!((seed.size, seed.top), (default.size, default.top));
    }

    #[test]
    fn a_partial_appearance_keeps_the_other_defaults() {
        let config: Config = toml::from_str("[appearance]\nprimary_color = \"#333333\"").unwrap();
        assert_eq!(
            color(config.appearance.primary_color),
            Color::rgb(0.2, 0.2, 0.2)
        );
        assert_eq!(
            config.appearance.text_color,
            Appearance::default().text_color
        );
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
