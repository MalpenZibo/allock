//! allock configuration.
//!
//! allock has its own file (`~/.config/allock/config.toml`) but does **not**
//! own its look: the `[appearance]` table is read from ashell's config so a
//! theme change there moves the lock screen with it. allock's own
//! `[appearance]` table, if present, is deep-merged on top key by key — a
//! partial override changes only the keys it names instead of resetting the
//! rest to allock's defaults, which is what a plain serde `default` would do.

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
    /// Where to import `[appearance]` from. `None` (or a missing file) keeps
    /// allock's built-in ashell-flavoured defaults.
    pub ashell_config: Option<String>,
    /// Which outputs show the login form. Every output is always covered by a
    /// lock surface — this only decides where the form is drawn; the others
    /// get the clock alone.
    pub form_outputs: Outputs,
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
            ashell_config: Some("~/.config/ashell/config.toml".to_owned()),
            form_outputs: Outputs::default(),
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

/// Same shape as ashell's `outputs` key.
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
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            pam_service: "allock".to_owned(),
            fingerprint: true,
            fingerprint_retries: 5,
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

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct GreeterConfig {
    /// Preselected user name. Empty means the form asks for it first.
    pub user: Option<String>,
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
            sessions: Vec::new(),
            remember: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Appearance — shared with ashell
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
        /// dropping the field would make every ashell colour that sets it
        /// fail to deserialize and silently fall back to the defaults.
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

/// The subset of ashell's `[appearance]` that means something on a lock
/// screen. Unknown keys from ashell's table (`style`, `bar`, `menu`,
/// `workspace_colors`, …) are ignored rather than rejected, so the same
/// table can serve both programs.
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

/// Recursively merge `over` into `base`; on a key both tables define, the
/// deeper of the two wins only when both sides are tables, otherwise `over`
/// replaces `base` wholesale.
fn merge(base: toml::Value, over: toml::Value) -> toml::Value {
    match (base, over) {
        (toml::Value::Table(mut b), toml::Value::Table(o)) => {
            for (key, value) in o {
                let merged = match b.remove(&key) {
                    Some(existing) => merge(existing, value),
                    None => value,
                };
                b.insert(key, merged);
            }
            toml::Value::Table(b)
        }
        (_, over) => over,
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

/// Load allock's config, importing `[appearance]` from ashell's.
pub fn load(path: &Path) -> Config {
    let mut table = read_toml(path).unwrap_or_default();
    if !path.exists() {
        log::info!("No config file at {path:?}, using defaults");
    }

    // Which ashell config to import from: allock's own key if it sets one,
    // otherwise the default location.
    let ashell_key = table
        .get("ashell_config")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned())
        .or_else(|| Config::default().ashell_config);

    if let Some(ashell_path) = ashell_key.filter(|s| !s.is_empty()) {
        let ashell_path = expand_tilde(&ashell_path);
        match read_toml(&ashell_path).and_then(|mut t| t.remove("appearance")) {
            Some(imported) => {
                log::info!("Imported [appearance] from {ashell_path:?}");
                let merged = match table.remove("appearance") {
                    Some(own) => merge(imported, own),
                    None => imported,
                };
                table.insert("appearance".to_owned(), merged);
            }
            None => log::info!("No [appearance] to import from {ashell_path:?}"),
        }
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

    fn table(s: &str) -> toml::Value {
        toml::Value::Table(toml::from_str(s).unwrap())
    }

    #[test]
    fn merge_overrides_key_by_key() {
        // The point of deep-merging instead of letting serde default: an
        // allock-side override of one colour must not drop the imported ones.
        let base = table("[appearance]\ntext_color = \"#111111\"\nprimary_color = \"#222222\"");
        let over = table("[appearance]\nprimary_color = \"#333333\"");
        let merged = merge(base, over);
        let appearance = merged.get("appearance").unwrap();
        assert_eq!(
            appearance.get("text_color").unwrap().as_str(),
            Some("#111111")
        );
        assert_eq!(
            appearance.get("primary_color").unwrap().as_str(),
            Some("#333333")
        );
    }

    #[test]
    fn merge_replaces_scalars_and_arrays() {
        let merged = merge(table("a = [1, 2, 3]"), table("a = [9]"));
        assert_eq!(merged.get("a").unwrap().as_array().unwrap().len(), 1);
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
}
