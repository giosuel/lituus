//! `~/.config/lituus/config.toml`. Every key is optional; a missing file means the defaults.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub theme: ThemeConfig,
    pub tray: TrayConfig,
    pub notifications: NotificationsConfig,
    pub sound: SoundConfig,
    pub power: PowerConfig,
    pub night_light: NightLightConfig,
    pub bluetooth: BluetoothConfig,
    pub wallpaper: WallpaperConfig,
    /// The System page's update commands the user changed, by package manager ("pacman").
    pub updates: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct WallpaperConfig {
    /// The folder of pictures shown; empty: ~/Pictures/Wallpapers if there is one, else
    /// ~/Pictures.
    pub folder: String,
    /// Folders shown before, the last first.
    pub recent: Vec<String>,
    /// The folder's subfolders' pictures shown with its own.
    pub subfolders: bool,
    /// "newest" or "name".
    pub sort: String,
    /// How a picture covers its monitor: "cover", "contain", "stretch", "center" or "tile".
    pub fill: String,
    /// The bars' colour around a picture that doesn't fill its monitor (awww), "rrggbb".
    pub bar_colour: String,
    /// awww's transition, and its length in seconds.
    pub transition: String,
    pub duration: f64,
    /// The slideshow: a picture from the folder every `every` minutes, "shuffle" or "in-order".
    pub slideshow: bool,
    pub every: u32,
    pub order: String,
    /// A command run after each change, `{file}` the picture: `matugen image {file}`.
    pub after: String,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            folder: String::new(),
            recent: Vec::new(),
            subfolders: false,
            sort: "newest".into(),
            fill: "cover".into(),
            bar_colour: "000000".into(),
            transition: "fade".into(),
            duration: 1.5,
            slideshow: false,
            every: 30,
            order: "shuffle".into(),
            after: String::new(),
        }
    }
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BluetoothConfig {
    /// Devices (by address) whose files are saved without asking.
    pub accept_files_from: Vec<String>,
}

/// What the lid and the power button do (the Power page): "suspend", "lock" (the lid),
/// "poweroff" (the button) or "nothing"; empty until chosen, when logind's own handling stands and
/// Lituus writes no bind.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PowerConfig {
    pub lid_battery: String,
    pub lid_plugged: String,
    /// With another screen connected.
    pub lid_external: String,
    pub power_key: String,
}

/// The Night light page: hyprsunset's warmth and when it's on.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct NightLightConfig {
    pub enabled: bool,
    /// Kelvin, 2500 to 6500.
    pub temperature: u32,
    /// "sun" (sunset to sunrise) or "times" (`from` to `to`, as "21:00").
    pub schedule: String,
    pub from: String,
    pub to: String,
    /// "manual" for `latitude` and `longitude`; otherwise the time zone's place.
    pub location: String,
    pub latitude: f64,
    pub longitude: f64,
}

impl Default for NightLightConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            temperature: 4000,
            schedule: "sun".into(),
            from: "21:00".into(),
            to: "07:00".into(),
            location: String::new(),
            latitude: 0.0,
            longitude: 0.0,
        }
    }
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SoundConfig {
    /// Volume sliders go to 150 %.
    pub overamplify: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TrayConfig {
    /// Show the tray icon (when a bar with a tray runs).
    pub enabled: bool,
    /// Closing the window keeps Lituus running in the tray.
    pub keep_running: bool,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self { enabled: true, keep_running: true }
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationsConfig {
    /// Connected, disconnected and failed, while the window isn't showing.
    pub enabled: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeConfig {
    /// Use the wallpaper's colours (the sources) when there are some.
    pub follow_wallpaper: bool,
    /// Where colours come from, in order: the first that exists and is a usable palette wins.
    pub sources: Vec<ColourSource>,
    /// The theme when not following the wallpaper, or when no source is usable.
    pub builtin: Builtin,
    /// Dark or light, for the built-in theme.
    pub mode: Mode,
    /// The accent glow behind each page's hero.
    pub gradient: bool,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            sources: vec![
                ColourSource { path: "~/.config/lituus/colors.json".into() },
                // ML4W writes every matugen colour role here on each wallpaper change.
                ColourSource { path: "~/.config/ml4w/colors/colors.json".into() },
            ],
            follow_wallpaper: true,
            builtin: Builtin::Teal,
            mode: Mode::Auto,
            gradient: true,
        }
    }
}

/// A matugen-style `colors.json`: Material colour roles (`primary`, `surface`, …) as hex strings.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ColourSource {
    pub path: String,
}

impl ColourSource {
    pub fn resolved(&self) -> PathBuf {
        expand_home(&self.path)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Builtin {
    #[default]
    Teal,
    Blue,
    Lavender,
    Coral,
    Mono,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Dark or light, as the desktop prefers.
    #[default]
    Auto,
    Dark,
    Light,
}

/// `$XDG_CONFIG_HOME/lituus`, or `~/.config/lituus`.
pub fn dir() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(config) => PathBuf::from(config).join("lituus"),
        None => expand_home("~/.config/lituus"),
    }
}

pub fn path() -> PathBuf {
    dir().join("config.toml")
}

/// Reads the config, or the defaults if there's none. What Lituus can't use is left out and the
/// rest is used, so a typo never keeps the app from starting nor turns every setting back to its
/// default; `load_noting` says what was left out.
pub fn load(path: &Path) -> Config {
    load_noting(path).0
}

/// The config, and what the Settings page says about the file when Lituus couldn't use all of
/// it: a title and a line.
pub fn load_noting(path: &Path) -> (Config, Option<(String, String)>) {
    let not_used = || "config.toml isn't used".to_owned();
    let text = match lituus_fs::read_or_empty(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            return (Config::default(), Some((not_used(), "It isn't UTF-8 text, so Lituus uses its defaults.".into())));
        }
        Err(e) => {
            return (
                Config::default(),
                Some((not_used(), format!("Lituus couldn't read it ({e}), so it uses its defaults."))),
            );
        }
    };
    match parse_leaving_out(&text) {
        Ok((config, left_out)) if left_out.is_empty() => (config, None),
        Ok((config, left_out)) => {
            let text = format!(
                "Unknown, or a value Lituus can't use: {}. The rest of the file is in use.",
                left_out.join(", ")
            );
            (config, Some(("Part of config.toml isn't used".into(), text)))
        }
        Err(e) => {
            let mut problem = describe(&text, &e);
            if let Some(first) = problem.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            (Config::default(), Some((not_used(), format!("{problem}. Lituus uses its defaults until it's fixed."))))
        }
    }
}

/// The config, leaving out what can't be used: a table Lituus doesn't know whole, and in a table
/// it knows, each key it doesn't know or whose value doesn't fit. Returns those left out
/// ("appearance", "theme.mode"). Only a file that isn't TOML at all is an error.
fn parse_leaving_out(text: &str) -> Result<(Config, Vec<String>), toml::de::Error> {
    use toml::{Table, Value};
    let fits = |table: Table| table.try_into::<Config>().is_ok();
    let one = |key: &str, value: Value| Table::from_iter([(key.to_owned(), value)]);
    let file: Table = text.parse()?;
    let mut kept = Table::new();
    let mut left_out = Vec::new();
    for (name, value) in file {
        if fits(one(&name, value.clone())) {
            kept.insert(name, value);
        } else if let Value::Table(table) = value
            && fits(one(&name, Value::Table(Table::new())))
        {
            let mut part = Table::new();
            for (key, value) in table {
                if fits(one(&name, Value::Table(one(&key, value.clone())))) {
                    part.insert(key, value);
                } else {
                    left_out.push(format!("{name}.{key}"));
                }
            }
            kept.insert(name, Value::Table(part));
        } else {
            left_out.push(name);
        }
    }
    Ok((kept.try_into()?, left_out))
}

/// A TOML error in a line: "line 16: unknown field `transfrom`". serde's list of the fields it
/// expected instead is left out: it's long, and the file's own comments say what goes where.
pub fn describe(text: &str, error: &toml::de::Error) -> String {
    let message = error.message().split(", expected one of").next().unwrap_or_default().trim();
    match error.span().and_then(|span| text.get(..span.start)) {
        Some(before) => format!("line {}: {message}", before.matches('\n').count() + 1),
        None => message.to_owned(),
    }
}

impl Builtin {
    pub const ALL: [Builtin; 5] = [Builtin::Teal, Builtin::Blue, Builtin::Lavender, Builtin::Coral, Builtin::Mono];

    pub fn key(self) -> &'static str {
        match self {
            Self::Teal => "teal",
            Self::Blue => "blue",
            Self::Lavender => "lavender",
            Self::Coral => "coral",
            Self::Mono => "mono",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Teal => "Teal",
            Self::Blue => "Blue",
            Self::Lavender => "Lavender",
            Self::Coral => "Coral",
            Self::Mono => "Mono",
        }
    }
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Auto, Mode::Dark, Mode::Light];

    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn dark(self, prefers_dark: bool) -> bool {
        match self {
            Self::Auto => prefers_dark,
            Self::Dark => true,
            Self::Light => false,
        }
    }
}

/// Sets one key, keeping the rest of the file as it is (comments and order included).
/// A file that can't be read is an error, not an empty file to write the key to.
pub fn set(path: &Path, table: &str, key: &str, value: impl Into<toml_edit::Value>) -> std::io::Result<()> {
    let text = lituus_fs::read_or_empty(path)?;
    let text = with(&text, table, key, value).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    lituus_fs::write_atomic(path, text)
}

/// Removes one key, keeping the rest of the file (and the table, even when it's empty).
pub fn unset(path: &Path, table: &str, key: &str) -> std::io::Result<()> {
    let text = match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        read => read?,
    };
    let mut doc: toml_edit::DocumentMut =
        text.parse().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if let Some(t) = doc.get_mut(table).and_then(|t| t.as_table_like_mut()) {
        t.remove(key);
    }
    lituus_fs::write_atomic(path, doc.to_string())
}

/// Sets one key to a list of strings, keeping the rest of the file.
pub fn set_list(path: &Path, table: &str, key: &str, values: &[String]) -> std::io::Result<()> {
    let array: toml_edit::Array = values.iter().map(String::as_str).collect();
    set(path, table, key, array)
}

fn with(
    text: &str,
    table: &str,
    key: &str,
    value: impl Into<toml_edit::Value>,
) -> Result<String, toml_edit::TomlError> {
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    let entry = doc.entry(table).or_insert_with(toml_edit::table);
    if let Some(t) = entry.as_table_like_mut() {
        let mut value = value.into();
        // A comment after the old value stays after the new one.
        if let Some(old) = t.get(key).and_then(|item| item.as_value()) {
            *value.decor_mut() = old.decor().clone();
        }
        t.insert(key, toml_edit::Item::Value(value));
    }
    Ok(doc.to_string())
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
        None => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole file, strictly: every key known, every value usable.
    fn parse(text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    #[test]
    fn empty_config_has_the_default_sources() {
        let config = parse("").unwrap();
        assert_eq!(config, Config::default());
        let paths: Vec<&str> = config.theme.sources.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["~/.config/lituus/colors.json", "~/.config/ml4w/colors/colors.json"]);
        assert_eq!((config.theme.builtin, config.theme.mode), (Builtin::Teal, Mode::Auto));
        assert!(config.theme.follow_wallpaper && config.theme.gradient);
    }

    #[test]
    fn update_commands() {
        let c = parse("[updates]\npacman = \"sudo pacman -Syu --noconfirm\"\n").unwrap();
        assert_eq!(c.updates.get("pacman").map(String::as_str), Some("sudo pacman -Syu --noconfirm"));
        let dir = std::env::temp_dir().join(format!("lituus-config-updates-{}", std::process::id()));
        let path = dir.join("config.toml");
        set(&path, "updates", "aur", "yay -Sua --devel").unwrap();
        set(&path, "updates", "pacman", "sudo pacman -Syu").unwrap();
        unset(&path, "updates", "aur").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[updates]\npacman = \"sudo pacman -Syu\"\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sources_are_an_ordered_list() {
        let config = parse(
            r#"
            [theme]
            builtin = "coral"
            mode = "light"
            follow_wallpaper = false
            [[theme.sources]]
            path = "~/wal.json"
            [[theme.sources]]
            path = "/etc/colors.json"
            "#,
        )
        .unwrap();
        let paths: Vec<&str> = config.theme.sources.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["~/wal.json", "/etc/colors.json"]);
        assert_eq!((config.theme.builtin, config.theme.mode), (Builtin::Coral, Mode::Light));
        assert!(!config.theme.follow_wallpaper);
    }

    #[test]
    fn settings_keep_the_rest_of_the_file() {
        let text = "# my colours\n[theme]\nmode = \"dark\" # night owl\n";
        let text = with(text, "tray", "keep_running", false).unwrap();
        let text = with(&text, "theme", "mode", "light").unwrap();
        assert_eq!(text, "# my colours\n[theme]\nmode = \"light\" # night owl\n\n[tray]\nkeep_running = false\n");
        let config = parse(&text).unwrap();
        assert_eq!(config.theme.mode, Mode::Light);
        assert!(config.tray.enabled && !config.tray.keep_running && config.notifications.enabled);
    }

    /// One key or table Lituus doesn't know (a typo, or a newer Lituus's) costs only that key, and
    /// Settings says which.
    #[test]
    fn what_cant_be_used_is_left_out_and_said() {
        let dir = std::env::temp_dir().join(format!("lituus-config-left-out-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let text = "[appearance]\nfont = \"Inter\"\n\n[theme]\nmode = \"light\"\nmood = \"calm\"\nbuiltin = \"purple\"\n\n[tray]\nenabled = false\n\n[updates]\npacman = \"sudo pacman -Syu\"\naur = 3\n";
        std::fs::write(&path, text).unwrap();
        let (config, note) = load_noting(&path);
        assert_eq!((config.theme.mode, config.theme.builtin), (Mode::Light, Builtin::Teal));
        assert!(!config.tray.enabled && config.tray.keep_running);
        assert_eq!(config.updates.keys().collect::<Vec<_>>(), ["pacman"]);
        let (title, text) = note.unwrap();
        assert_eq!(title, "Part of config.toml isn't used");
        for name in ["appearance", "theme.mood", "theme.builtin", "updates.aur"] {
            assert!(text.contains(name), "{name} isn't named: {text}");
        }
        assert!(!text.contains("theme.mode") && !text.contains("tray"), "{text}");
        // Setting a key keeps what was left out, in the file.
        set(&path, "tray", "enabled", true).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("font = \"Inter\""));
        // A file all Lituus's own has nothing to say.
        std::fs::write(&path, "[theme]\nmode = \"dark\"\n").unwrap();
        assert_eq!(load_noting(&path), (parse("[theme]\nmode = \"dark\"\n").unwrap(), None));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file that isn't TOML, or can't be read, means the defaults, and Settings says why; and
    /// a setting changed then isn't written over it.
    #[test]
    fn a_file_lituus_cant_read_is_said_and_kept() {
        let dir = std::env::temp_dir().join(format!("lituus-config-unreadable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[theme]\nmode = \"dark\"\n[tray\n").unwrap();
        let (config, note) = load_noting(&path);
        assert_eq!(config, Config::default());
        let (title, text) = note.unwrap();
        assert_eq!(title, "config.toml isn't used");
        assert!(text.starts_with("Line 3: "), "{text}");
        let latin1 = b"# caf\xe9\n[theme]\nmode = \"dark\"\n";
        std::fs::write(&path, latin1).unwrap();
        assert_eq!(load_noting(&path).1.unwrap().1, "It isn't UTF-8 text, so Lituus uses its defaults.");
        assert!(set(&path, "tray", "enabled", false).is_err());
        assert!(unset(&path, "theme", "mode").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), latin1);
        // No file: the defaults, and nothing to say.
        assert_eq!(load_noting(&dir.join("none.toml")), (Config::default(), None));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn typos_are_errors_not_silently_ignored() {
        assert!(parse("[theme]\nbuiltn = \"dark\"").is_err());
        assert!(parse("[theme]\nbuiltin = \"purple\"").is_err());
        assert!(parse("[theme]\nmode = \"dim\"").is_err());
    }
}
