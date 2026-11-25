//! The Appearance page (D-57, S-176): how other apps look. The style (gsettings' color-scheme), the
//! GTK 3 theme, the icons, the cursor and the fonts, read from gsettings (GTK's settings.ini without
//! it) and written to both; the cursor also to XCursor's default theme, and to Hyprland, live and in
//! Lituus's own file. Lituus's own colours are on Settings.

use std::path::{Path, PathBuf};

use lituus_xdg::Dirs;
use slint::SharedString;

use crate::{AppearanceView, HeroInfo, MainWindow, model, update};

const SCHEMA: &str = "org.gnome.desktop.interface";
/// The styles, as color-scheme says them, in the page's order.
const SCHEMES: [&str; 3] = ["default", "prefer-dark", "prefer-light"];
const CURSOR_SIZES: [u32; 6] = [16, 24, 32, 48, 64, 96];
const FONT_SIZES: [u32; 9] = [8, 9, 10, 11, 12, 13, 14, 16, 18];
/// Lituus's section of its Hyprland file.
const SECTION: &str = "Appearance";

/// What the apps look like now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    pub scheme: String,
    pub gtk: String,
    pub icons: String,
    pub cursor: String,
    pub cursor_size: u32,
    pub font: String,
    pub font_size: u32,
    pub mono: String,
    pub mono_size: u32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            scheme: "default".into(),
            gtk: "Adwaita".into(),
            icons: "Adwaita".into(),
            cursor: "Adwaita".into(),
            cursor_size: 24,
            font: "Cantarell".into(),
            font_size: 11,
            mono: "Monospace".into(),
            mono_size: 11,
        }
    }
}

/// What's installed to choose from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Installed {
    pub gtk: Vec<String>,
    pub icons: Vec<String>,
    pub cursors: Vec<String>,
    pub fonts: Vec<String>,
    pub monos: Vec<String>,
}

#[derive(Default)]
pub struct State {
    pub look: Option<Look>,
    pub installed: Installed,
    /// gsettings is there (it's how GTK 4 and libadwaita apps, Firefox and others read the style).
    pub gsettings: bool,
    pub error: Option<String>,
    /// The page was opened: read the look and what's installed again (a theme installed while
    /// Lituus runs in the background shows up).
    pub reread: bool,
}

/// Reads the look and what's installed, off the window's thread (fc-list takes a moment).
pub fn spawn(weak: slint::Weak<MainWindow>) {
    std::thread::spawn(move || {
        let (look, gsettings) = read_look();
        let installed = read_installed(&Dirs::from_env(), &home());
        let weak2 = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| {
            update(&weak2, |app| {
                app.appearance.look = Some(look);
                app.appearance.installed = installed;
                app.appearance.gsettings = gsettings;
            })
        });
    });
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn gsettings_get(key: &str) -> Option<String> {
    let out = std::process::Command::new("gsettings").args(["get", SCHEMA, key]).output().ok()?;
    out.status.success().then(|| unquote(String::from_utf8_lossy(&out.stdout).trim()))
}

/// A GVariant's text without its quotes ('Adwaita' → Adwaita; uint32 24 → 24).
fn unquote(value: &str) -> String {
    let v = value.strip_prefix("uint32 ").unwrap_or(value);
    match v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        Some(inner) => inner.replace("\\'", "'").replace("\\\\", "\\"),
        None => v.to_owned(),
    }
}

/// "Inter 11" → ("Inter", 11); a name without a size keeps `fallback`.
fn split_font(font: &str, fallback: u32) -> (String, u32) {
    match font.rsplit_once(' ') {
        Some((family, size)) if size.parse::<f32>().is_ok() => {
            (family.trim().to_owned(), size.parse::<f32>().map_or(fallback, |s| s.round() as u32))
        }
        _ => (font.trim().to_owned(), fallback),
    }
}

/// The look, from gsettings when it's there, else from GTK 3's settings.ini; and whether gsettings is.
fn read_look() -> (Look, bool) {
    let mut look = Look::default();
    let gsettings = gsettings_get("gtk-theme").is_some();
    if gsettings {
        let get = |k: &str| gsettings_get(k).unwrap_or_default();
        look.scheme =
            Some(get("color-scheme")).filter(|s| SCHEMES.contains(&s.as_str())).unwrap_or_else(|| "default".into());
        look.gtk = get("gtk-theme");
        look.icons = get("icon-theme");
        look.cursor = get("cursor-theme");
        look.cursor_size = get("cursor-size").parse().unwrap_or(24);
        (look.font, look.font_size) = split_font(&get("font-name"), 11);
        (look.mono, look.mono_size) = split_font(&get("monospace-font-name"), 11);
    } else if let Ok(text) = std::fs::read_to_string(home().join(".config/gtk-3.0/settings.ini")) {
        let ini = |k: &str| ini_get(&text, k);
        look.gtk = ini("gtk-theme-name").unwrap_or(look.gtk);
        look.icons = ini("gtk-icon-theme-name").unwrap_or(look.icons);
        look.cursor = ini("gtk-cursor-theme-name").unwrap_or(look.cursor);
        look.cursor_size = ini("gtk-cursor-theme-size").and_then(|s| s.parse().ok()).unwrap_or(24);
        if let Some(f) = ini("gtk-font-name") {
            (look.font, look.font_size) = split_font(&f, 11);
        }
        if ini("gtk-application-prefer-dark-theme").as_deref() == Some("1") {
            look.scheme = "prefer-dark".into();
        }
    }
    (look, gsettings)
}

fn ini_get(text: &str, key: &str) -> Option<String> {
    let mut in_settings = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_settings = line == "[Settings]";
        } else if in_settings
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().trim_matches('"').to_owned());
        }
    }
    None
}

/// `text` (a settings.ini) with these keys set under [Settings], everything else kept.
fn ini_set(text: &str, values: &[(&str, String)]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut done = vec![false; values.len()];
    let mut in_settings = false;
    let mut saw_settings = false;
    let flush = |out: &mut Vec<String>, done: &mut Vec<bool>| {
        for (i, (k, v)) in values.iter().enumerate() {
            if !done[i] {
                out.push(format!("{k}={v}"));
                done[i] = true;
            }
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_settings {
                flush(&mut out, &mut done);
            }
            in_settings = t == "[Settings]";
            saw_settings |= in_settings;
            out.push(line.to_owned());
            continue;
        }
        if in_settings
            && let Some((k, _)) = t.split_once('=')
            && let Some(i) = values.iter().position(|(key, _)| *key == k.trim())
        {
            out.push(format!("{}={}", values[i].0, values[i].1));
            done[i] = true;
            continue;
        }
        out.push(line.to_owned());
    }
    if !saw_settings {
        out.push("[Settings]".into());
    }
    flush(&mut out, &mut done);
    out.join("\n") + "\n"
}

/// The themes and fonts installed, by name, sorted, each once.
pub fn read_installed(dirs: &Dirs, home: &Path) -> Installed {
    let roots = |sub: &str, dot: &str| -> Vec<PathBuf> {
        let mut r = vec![dirs.data_home.join(sub), home.join(dot)];
        r.extend(dirs.data_dirs.iter().map(|d| d.join(sub)));
        r
    };
    let names = |roots: &[PathBuf], keep: &dyn Fn(&Path) -> bool| -> Vec<String> {
        let mut out: Vec<String> = roots
            .iter()
            .filter_map(|r| std::fs::read_dir(r).ok())
            .flatten()
            .flatten()
            .filter(|e| keep(&e.path()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n != "default" && n != "hicolor")
            .collect();
        out.sort_by_key(|n| n.to_lowercase());
        out.dedup();
        out
    };
    let themes = roots("themes", ".themes");
    let icons = roots("icons", ".icons");
    Installed {
        // A GTK 3 theme has a gtk.css; Default and Emacs have only key bindings (gtk-keys.css).
        gtk: names(&themes, &|p| p.join("gtk-3.0/gtk.css").is_file()),
        icons: names(&icons, &|p| {
            std::fs::read_to_string(p.join("index.theme")).is_ok_and(|t| {
                t.lines().any(|l| l.trim().strip_prefix("Directories=").is_some_and(|d| !d.trim().is_empty()))
            })
        }),
        cursors: names(&icons, &|p| {
            p.join("cursors").is_dir() || p.join("hyprcursors").is_dir() || p.join("manifest.hl").is_file()
        }),
        fonts: fonts(&[]),
        monos: fonts(&[":spacing=100"]),
    }
}

fn fonts(pattern: &[&str]) -> Vec<String> {
    let Ok(out) = std::process::Command::new("fc-list").args(pattern).args(["--format", "%{family[0]}\n"]).output()
    else {
        return Vec::new();
    };
    let mut families: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_owned)
        .collect();
    families.sort_by_key(|f| f.to_lowercase());
    families.dedup();
    families
}

/// A choice on the page: (verb, its place in the list). The look is written off the window's thread
/// (gsettings is a program per key), and an error comes back to the page.
pub fn act(weak: &slint::Weak<MainWindow>, state: &mut State, verb: &str, value: i32) {
    let Some(mut look) = state.look.clone() else { return };
    let i = value.max(0) as usize;
    let pick = |list: &[String]| list.get(i).cloned();
    match verb {
        "dismiss" => {
            state.error = None;
            return;
        }
        "scheme" => look.scheme = SCHEMES.get(i).map(|s| (*s).to_owned()).unwrap_or(look.scheme),
        "gtk" => look.gtk = pick(&with(&state.installed.gtk, &look.gtk)).unwrap_or(look.gtk),
        "icons" => look.icons = pick(&with(&state.installed.icons, &look.icons)).unwrap_or(look.icons),
        "cursor" => look.cursor = pick(&with(&state.installed.cursors, &look.cursor)).unwrap_or(look.cursor),
        "cursor-size" => {
            look.cursor_size = with_n(&CURSOR_SIZES, look.cursor_size).get(i).copied().unwrap_or(look.cursor_size)
        }
        "font" => look.font = pick(&with(&state.installed.fonts, &look.font)).unwrap_or(look.font),
        "mono" => look.mono = pick(&with(&state.installed.monos, &look.mono)).unwrap_or(look.mono),
        "font-size" => look.font_size = with_n(&FONT_SIZES, look.font_size).get(i).copied().unwrap_or(look.font_size),
        _ => return,
    }
    if Some(&look) == state.look.as_ref() {
        return;
    }
    let cursor_changed =
        state.look.as_ref().is_some_and(|l| l.cursor != look.cursor || l.cursor_size != look.cursor_size);
    state.look = Some(look.clone());
    let gsettings = state.gsettings;
    let weak = weak.clone();
    std::thread::spawn(move || {
        if let Err(e) = apply(&look, gsettings, cursor_changed) {
            let weak2 = weak.clone();
            let _ = weak.upgrade_in_event_loop(move |_| update(&weak2, |app| app.appearance.error = Some(e)));
        }
    });
}

/// Writes the look: gsettings (when it's there), GTK 3's and 4's settings.ini, and for the cursor
/// XCursor's default theme and Hyprland (live, and in Lituus's file).
fn apply(look: &Look, gsettings: bool, cursor: bool) -> Result<(), String> {
    if gsettings {
        for (key, value) in [
            ("color-scheme", look.scheme.clone()),
            ("gtk-theme", look.gtk.clone()),
            ("icon-theme", look.icons.clone()),
            ("cursor-theme", look.cursor.clone()),
            ("font-name", format!("{} {}", look.font, look.font_size)),
            ("monospace-font-name", format!("{} {}", look.mono, look.mono_size)),
        ] {
            let quoted = format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"));
            let ok = std::process::Command::new("gsettings")
                .args(["set", SCHEMA, key, &quoted])
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                return Err(format!("Couldn't set {key} through gsettings."));
            }
        }
        let _ = std::process::Command::new("gsettings")
            .args(["set", SCHEMA, "cursor-size", &look.cursor_size.to_string()])
            .status();
    }
    let values = [
        ("gtk-theme-name", look.gtk.clone()),
        ("gtk-icon-theme-name", look.icons.clone()),
        ("gtk-cursor-theme-name", look.cursor.clone()),
        ("gtk-cursor-theme-size", look.cursor_size.to_string()),
        ("gtk-font-name", format!("{} {}", look.font, look.font_size)),
        ("gtk-application-prefer-dark-theme", if look.scheme == "prefer-dark" { "1" } else { "0" }.to_owned()),
    ];
    for version in ["gtk-3.0", "gtk-4.0"] {
        let path = home().join(".config").join(version).join("settings.ini");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        // GTK 4 reads only some of them; the theme it takes from the style.
        let keep: Vec<(&str, String)> =
            values.iter().filter(|(k, _)| version == "gtk-3.0" || *k != "gtk-theme-name").cloned().collect();
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
        lituus_fs::write_atomic(&path, ini_set(&text, &keep))
            .map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    }
    if cursor {
        // X apps (XWayland) take the default cursor theme's Inherits.
        let path = home().join(".icons/default/index.theme");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
        let _ = lituus_fs::write_atomic(&path, inherits(&text, &look.cursor));
        let file = crate::setup::file();
        if let Err(e) = file.write_section(SECTION, &cursor_part(look, file.lua)) {
            return Err(format!("Couldn't save the cursor for Hyprland: {e}."));
        }
        if let Ok(hypr) = lituus_hypr::Hyprland::from_env() {
            let _ = hypr.request(&format!("setcursor {} {}", look.cursor, look.cursor_size));
        }
    }
    Ok(())
}

/// The default cursor theme's index.theme, inheriting `theme`.
fn inherits(text: &str, theme: &str) -> String {
    if text.trim().is_empty() {
        return format!("[Icon Theme]\nInherits={theme}\n");
    }
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            if l.trim().starts_with("Inherits=") {
                found = true;
                format!("Inherits={theme}")
            } else {
                l.to_owned()
            }
        })
        .collect();
    if !found {
        out.push(format!("Inherits={theme}"));
    }
    out.join("\n") + "\n"
}

/// The cursor for Hyprland and the apps it starts, in Lituus's file.
pub fn cursor_part(look: &Look, lua: bool) -> String {
    let vars = [
        ("XCURSOR_THEME", look.cursor.clone()),
        ("XCURSOR_SIZE", look.cursor_size.to_string()),
        ("HYPRCURSOR_THEME", look.cursor.clone()),
        ("HYPRCURSOR_SIZE", look.cursor_size.to_string()),
    ];
    let comment = if lua { "--" } else { "#" };
    let mut lines = vec![format!("{comment} Written by Lituus ({SECTION}). Change it in Lituus: the cursor.")];
    for (k, v) in vars {
        lines.push(if lua {
            format!("hl.env({}, {})", lituus_hypr::lua_string(k), lituus_hypr::lua_string(&v))
        } else {
            format!("env = {k},{v}")
        });
    }
    lines.join("\n") + "\n"
}

/// A list with `current` in it, so a choice that isn't installed (or wasn't found) still shows.
fn with(list: &[String], current: &str) -> Vec<String> {
    let mut l = list.to_vec();
    if !current.is_empty() && !l.iter().any(|x| x == current) {
        l.insert(0, current.to_owned());
    }
    l
}

fn with_n(list: &[u32], current: u32) -> Vec<u32> {
    let mut l = list.to_vec();
    if !l.contains(&current) {
        l.push(current);
        l.sort_unstable();
    }
    l
}

fn s(text: impl AsRef<str>) -> SharedString {
    SharedString::from(text.as_ref())
}

fn scheme_name(scheme: &str) -> &'static str {
    match scheme {
        "prefer-dark" => "Dark",
        "prefer-light" => "Light",
        _ => "Default",
    }
}

/// The sidebar's status: the style.
pub fn status(state: &State) -> String {
    state.look.as_ref().map(|l| scheme_name(&l.scheme).to_owned()).unwrap_or_default()
}

pub fn view(state: &State) -> AppearanceView {
    let mut v = AppearanceView { error: s(state.error.clone().unwrap_or_default()), ..AppearanceView::default() };
    let Some(look) = &state.look else {
        v.state = s("loading");
        return v;
    };
    v.state = s("list");
    v.gsettings = state.gsettings;
    v.sub = s("How your apps look · Lituus's own colours are in Settings");
    v.hero = HeroInfo {
        icon: s("palette"),
        name: s(scheme_name(&look.scheme)),
        meta: s(format!("{} · {} · {}", look.gtk, look.icons, look.cursor)),
        tone: s("accent"),
        detail: s(format!("{} {} · {}", look.font, look.font_size, look.mono)),
    };
    v.scheme = SCHEMES.iter().position(|x| *x == look.scheme).map_or(0, |i| i as i32);
    let list = |l: &[String], current: &str| {
        let l = with(l, current);
        let at = l.iter().position(|x| x == current).map_or(-1, |i| i as i32);
        (model(l.iter().map(s).collect()), at)
    };
    (v.gtk_themes, v.gtk) = list(&state.installed.gtk, &look.gtk);
    (v.icon_themes, v.icons) = list(&state.installed.icons, &look.icons);
    (v.cursor_themes, v.cursor) = list(&state.installed.cursors, &look.cursor);
    (v.fonts, v.font) = list(&state.installed.fonts, &look.font);
    (v.monos, v.mono) = list(&state.installed.monos, &look.mono);
    let sizes = |l: &[u32], current: u32| {
        let l = with_n(l, current);
        let at = l.iter().position(|x| *x == current).map_or(0, |i| i as i32);
        (model(l.iter().map(|n| s(n.to_string())).collect()), at)
    };
    (v.cursor_sizes, v.cursor_size) = sizes(&CURSOR_SIZES, look.cursor_size);
    (v.font_sizes, v.font_size) = sizes(&FONT_SIZES, look.font_size);
    v.font_family = s(&look.font);
    v.mono_family = s(&look.mono);
    v
}

pub fn render(window: &MainWindow, state: &State) {
    window.set_appearance(view(state));
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    #[test]
    fn gsettings_values_and_fonts_are_read() {
        assert_eq!(unquote("'Adwaita'"), "Adwaita");
        assert_eq!(unquote("'It\\'s'"), "It's");
        assert_eq!(unquote("uint32 24"), "24");
        assert_eq!(split_font("Inter 11", 10), ("Inter".into(), 11));
        assert_eq!(split_font("JetBrains Mono 10.5", 10), ("JetBrains Mono".into(), 11));
        assert_eq!(split_font("Monospace", 10), ("Monospace".into(), 10));
    }

    #[test]
    fn settings_ini_keeps_what_it_doesnt_set() {
        let text = "[Settings]\ngtk-theme-name=Adwaita\ngtk-enable-animations=true\n\n[Other]\nx=1\n";
        let out = ini_set(
            text,
            &[("gtk-theme-name", "adw-gtk3-dark".into()), ("gtk-icon-theme-name", "Papirus-Dark".into())],
        );
        assert_eq!(ini_get(&out, "gtk-theme-name").as_deref(), Some("adw-gtk3-dark"));
        assert_eq!(ini_get(&out, "gtk-icon-theme-name").as_deref(), Some("Papirus-Dark"));
        assert_eq!(ini_get(&out, "gtk-enable-animations").as_deref(), Some("true"));
        assert!(out.contains("[Other]\nx=1"), "{out}");
        // No file yet: a [Settings] section.
        assert_eq!(ini_set("", &[("gtk-font-name", "Inter 11".into())]), "[Settings]\ngtk-font-name=Inter 11\n");
    }

    #[test]
    fn the_default_cursor_inherits_the_chosen_one() {
        assert_eq!(inherits("", "Bibata"), "[Icon Theme]\nInherits=Bibata\n");
        assert_eq!(
            inherits("[Icon Theme]\nName=Default\nInherits=Adwaita\n", "Bibata"),
            "[Icon Theme]\nName=Default\nInherits=Bibata\n"
        );
    }

    #[test]
    fn the_cursor_goes_to_hyprland_in_its_language() {
        let look = Look { cursor: "Bibata-Modern-Ice".into(), cursor_size: 32, ..Look::default() };
        let lua = cursor_part(&look, true);
        assert!(lua.starts_with("-- Written by Lituus (Appearance)."), "{lua}");
        assert!(
            lua.contains(r#"hl.env("XCURSOR_THEME", "Bibata-Modern-Ice")"#)
                && lua.contains(r#"hl.env("HYPRCURSOR_SIZE", "32")"#),
            "{lua}"
        );
        let conf = cursor_part(&look, false);
        assert!(
            conf.contains("env = XCURSOR_THEME,Bibata-Modern-Ice") && conf.contains("env = HYPRCURSOR_SIZE,32"),
            "{conf}"
        );
    }

    #[test]
    fn installed_themes_are_found_by_what_they_have() {
        let root = std::env::temp_dir().join(format!("lituus-appearance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data = root.join("share");
        let mk = |p: &str| std::fs::create_dir_all(data.join(p)).unwrap();
        mk("themes/adw-gtk3-dark/gtk-3.0");
        std::fs::write(data.join("themes/adw-gtk3-dark/gtk-3.0/gtk.css"), "").unwrap();
        mk("themes/Openbox-only/openbox-3");
        mk("themes/Emacs/gtk-3.0");
        std::fs::write(data.join("themes/Emacs/gtk-3.0/gtk-keys.css"), "").unwrap();
        mk("icons/Papirus-Dark");
        std::fs::write(data.join("icons/Papirus-Dark/index.theme"), "[Icon Theme]\nDirectories=16x16/apps\n").unwrap();
        mk("icons/Bibata-Modern-Ice/cursors");
        std::fs::write(data.join("icons/Bibata-Modern-Ice/index.theme"), "[Icon Theme]\nInherits=x\n").unwrap();
        mk("icons/hicolor");
        let dirs = Dirs {
            data_home: data.clone(),
            data_dirs: vec![],
            config_home: root.join("config"),
            config_dirs: vec![],
            desktops: vec![],
        };
        let found = read_installed(&dirs, &root);
        assert_eq!(found.gtk, ["adw-gtk3-dark"]);
        assert_eq!(found.icons, ["Papirus-Dark"]);
        assert_eq!(found.cursors, ["Bibata-Modern-Ice"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_view_keeps_a_choice_that_isnt_installed() {
        let state = State {
            look: Some(Look { scheme: "prefer-dark".into(), gtk: "Custom".into(), ..Look::default() }),
            installed: Installed { gtk: vec!["Adwaita".into(), "adw-gtk3-dark".into()], ..Installed::default() },
            gsettings: true,
            error: None,
            reread: false,
        };
        let v = view(&state);
        assert_eq!((v.hero.name.as_str(), v.scheme), ("Dark", 1));
        assert_eq!(v.gtk_themes.row_count(), 3);
        assert_eq!(v.gtk_themes.row_data(v.gtk as usize).unwrap(), "Custom");
        assert_eq!(status(&state), "Dark");
    }
}
