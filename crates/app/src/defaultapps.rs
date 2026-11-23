//! Desktop › Default apps (D-37, D-39): which app opens links, each kind of file and a terminal.
//! Lituus reads every mimeapps.list the XDG spec names, in its order (lituus-xdg), so each row
//! shows what really opens; a choice is saved to the user's first own file. The terminal is
//! xdg-terminal-exec's (xdg-terminals.list), and choosing it doesn't change Super+Enter (D-40).
//! The apps and files are read on a thread when the page opens; icons are loaded here.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use lituus_xdg::desktop::{self, App};
use lituus_xdg::kinds::{self, KINDS, Kind};
use lituus_xdg::mime::MimeDb;
use lituus_xdg::mimeapps::{self, ListFile};
use lituus_xdg::{Dirs, shorten_home};
use slint::{Image, ModelRc, SharedString, VecModel};

use crate::{AppChoice, DaSource, DefaultAppsView, KindRow, MainWindow, TypeRow, update};

/// What the page reads.
pub struct Data {
    pub dirs: Dirs,
    pub apps: Vec<App>,
    pub files: Vec<ListFile>,
    pub db: MimeDb,
    /// The names of the file types the apps open ("PNG image").
    pub names: HashMap<String, String>,
    /// Icons as files, by app id.
    pub icons: HashMap<String, PathBuf>,
    /// "EndeavourOS", for "EndeavourOS's default".
    pub os_name: String,
    /// The terminal xdg-terminals.list names, its file, and whether xdg-terminal-exec is there.
    pub terminal: Option<String>,
    pub terminal_list: PathBuf,
    pub terminal_exec: bool,
    /// The terminal Lituus's Update buttons use when the list names none ("xterm").
    pub fallback: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Kind(&'static str),
    Type(String),
}

#[derive(Default)]
pub struct State {
    pub data: Option<Data>,
    pub query: String,
    /// The list of apps to choose from, and whether it has every app.
    pub picker: Option<(Target, bool)>,
    pub sources_open: bool,
    pub error: String,
    /// Icons, loaded on this thread (an Image can't cross threads).
    images: HashMap<String, Image>,
}

fn read() -> Data {
    let dirs = Dirs::from_env();
    let apps = desktop::all(&dirs);
    let files = mimeapps::files(&dirs);
    let db = MimeDb::load(&dirs);
    let handled: BTreeSet<&String> = apps.iter().flat_map(|a| &a.mime_types).collect();
    let names = handled.into_iter().map(|m| (m.clone(), db.name(m))).collect();
    let theme = icon_theme(&dirs);
    let icons = apps.iter().filter_map(|a| Some((a.id.clone(), desktop::icon_file(&dirs, &theme, &a.icon)?))).collect();
    let env = lituus_system::updates::Env::from_env();
    let tdirs = lituus_system::terminal::Dirs::from_env(&env.home);
    let fallback = lituus_system::terminal::default_terminal(&env, &tdirs, std::env::var("TERMINAL").ok().as_deref())
        .map(|t| t.name)
        .unwrap_or_default();
    Data {
        fallback,
        terminal: lituus_system::terminal::listed_terminal(&tdirs),
        terminal_list: lituus_system::terminal::list_to_save(&tdirs),
        terminal_exec: env.which("xdg-terminal-exec").is_some(),
        os_name: lituus_system::os(Path::new("/")).name,
        dirs,
        apps,
        files,
        db,
        names,
        icons,
    }
}

/// The GTK icon theme's name, from GTK's settings, else none (hicolor only).
fn icon_theme(dirs: &Dirs) -> String {
    for f in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
        if let Ok(text) = std::fs::read_to_string(dirs.config_home.join(f))
            && let Some(v) = text.lines().find_map(|l| l.trim().strip_prefix("gtk-icon-theme-name"))
        {
            return v.trim_start_matches([' ', '=']).trim().to_owned();
        }
    }
    String::new()
}

/// Reads the apps and files, on a thread.
pub fn open(weak: &slint::Weak<MainWindow>) {
    let weak = weak.clone();
    std::thread::spawn(move || {
        let data = read();
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| update(&w, move |app| app.default_apps.data = Some(data)));
    });
}

pub fn status(state: &State) -> String {
    let Some(d) = &state.data else { return String::new() };
    browser(d).map(|id| app_name(d, &id)).unwrap_or_default()
}

fn browser(d: &Data) -> Option<String> {
    mimeapps::resolve(&d.files, &d.apps, "x-scheme-handler/http").app
}

fn app_name(d: &Data, id: &str) -> String {
    d.apps
        .iter()
        .find(|a| a.id == id)
        .map(|a| a.name.clone())
        .unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned())
}

/// A kind's file types this computer knows.
fn kind_types(d: &Data, kind: &Kind) -> Vec<String> {
    kinds::state(kind, &d.files, &d.apps, &d.db).types
}

// ---- what the buttons do ------------------------------------------------------------------------

pub fn act(weak: &slint::Weak<MainWindow>, state: &mut State, verb: &str, arg: &str) {
    match verb {
        "search" => state.query = arg.to_owned(),
        "sources-open" => state.sources_open = true,
        "sources-close" => state.sources_open = false,
        "choose-kind" => state.picker = kinds::kind(arg).map(|k| (Target::Kind(k.key), false)),
        "choose-type" => state.picker = Some((Target::Type(arg.to_owned()), false)),
        "picker-all" => {
            if let Some((_, all)) = &mut state.picker {
                *all = true;
            }
        }
        "picker-close" => state.picker = None,
        "pick" => {
            if let Some((target, _)) = state.picker.take() {
                choose(weak, state, &target, arg);
            }
        }
        "fix" => {
            let Some(kind) = kinds::kind(arg) else { return };
            let app = state.data.as_ref().and_then(|d| kinds::state(kind, &d.files, &d.apps, &d.db).app);
            if let Some(app) = app {
                choose(weak, state, &Target::Kind(kind.key), &app);
            }
        }
        _ => {}
    }
}

/// Saves a choice, then reads the files again.
fn choose(_weak: &slint::Weak<MainWindow>, state: &mut State, target: &Target, app: &str) {
    let Some(d) = &mut state.data else { return };
    let result = match target {
        Target::Kind("terminal") => lituus_system::terminal::set_terminal(&d.terminal_list, app),
        Target::Kind(key) => {
            let kind = kinds::kind(key).expect("a kind");
            let types = kind_types(d, kind);
            mimeapps::set_defaults(&mimeapps::save_target(&d.files, &d.dirs), &types, app)
        }
        Target::Type(mime) => {
            mimeapps::set_defaults(&mimeapps::save_target(&d.files, &d.dirs), std::slice::from_ref(mime), app)
        }
    };
    state.error = match result {
        Ok(()) => String::new(),
        Err(e) => format!("Couldn't save the choice: {e}."),
    };
    d.files = mimeapps::files(&d.dirs);
    if matches!(target, Target::Kind("terminal")) {
        let env = lituus_system::updates::Env::from_env();
        d.terminal = lituus_system::terminal::listed_terminal(&lituus_system::terminal::Dirs::from_env(&env.home));
    }
}

// ---- the page -----------------------------------------------------------------------------------

fn s(text: impl AsRef<str>) -> SharedString {
    SharedString::from(text.as_ref())
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn choice(state: &mut State, id: Option<&str>, selected: bool) -> AppChoice {
    let Some(d) = &state.data else { return AppChoice::default() };
    let Some(id) = id else { return AppChoice::default() };
    let app = d.apps.iter().find(|a| a.id == id);
    let name = app_name(d, id);
    let detail = match app {
        Some(a) if !a.flatpak.is_empty() => format!("Flatpak · {}", a.flatpak),
        _ => id.to_owned(),
    };
    let icon = match d.icons.get(id) {
        Some(path) => {
            state.images.entry(id.to_owned()).or_insert_with(|| Image::load_from_path(path).unwrap_or_default()).clone()
        }
        None => Image::default(),
    };
    AppChoice {
        id: s(id),
        letter: s(name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()),
        name: s(name),
        detail: s(detail),
        icon,
        selected,
    }
}

/// "MP4, MKV, WebM and 14 more".
fn extensions_text(d: &Data, types: &[String]) -> String {
    let exts: Vec<String> = types.iter().filter_map(|t| d.db.extensions(t).first()).map(|e| e.to_uppercase()).collect();
    match exts.len() {
        0 => String::new(),
        1..=3 => exts.join(", "),
        n => format!("{} and {} more", exts[..3].join(", "), n - 3),
    }
}

/// "GIF" for image/gif: its first extension, else its name.
fn short_name(d: &Data, mime: &str) -> String {
    d.db.extensions(mime).first().map(|e| e.to_uppercase()).unwrap_or_else(|| type_name(d, mime))
}

fn type_name(d: &Data, mime: &str) -> String {
    d.names.get(mime).cloned().unwrap_or_else(|| d.db.name(mime))
}

fn kind_row(state: &mut State, kind: &Kind) -> KindRow {
    let Some(d) = &state.data else { return KindRow::default() };
    if kind.key == "terminal" {
        // Not chosen: the one Lituus falls back to, if an app is it.
        let fallback = (!d.fallback.is_empty())
            .then(|| {
                d.apps
                    .iter()
                    .find(|a| a.categories.iter().any(|c| c == "TerminalEmulator") && a.id.contains(&d.fallback))
            })
            .flatten()
            .map(|a| a.id.clone());
        let terminal = d.terminal.clone().or(fallback);
        let note = match (&d.terminal, d.terminal_exec) {
            (Some(_), true) => "for xdg-terminal-exec".to_owned(),
            (Some(_), false) => "xdg-terminal-exec isn't installed".to_owned(),
            (None, true) => "Not set: xdg-terminal-exec picks one".to_owned(),
            (None, false) if !d.fallback.is_empty() => {
                format!("Not set: Lituus uses {}, xdg-terminal-exec isn't installed", d.fallback)
            }
            (None, false) => "No terminal found".to_owned(),
        };
        let hint = kind.hint.to_owned();
        let app = choice(state, terminal.as_deref(), false);
        return KindRow {
            key: s(kind.key),
            label: s(kind.label),
            hint: s(hint),
            icon: s(kind.icon),
            app,
            note: s(note),
            ..Default::default()
        };
    }
    let st = kinds::state(kind, &d.files, &d.apps, &d.db);
    let hint = if kind.hint.is_empty() { extensions_text(d, &st.types) } else { kind.hint.to_owned() };
    let app_name_ = st.app.as_deref().map(|a| app_name(d, a)).unwrap_or_default();
    let (mixed, fix) = match (&st.app, st.others.as_slice()) {
        (Some(_), [(mime, Some(other))]) => (
            format!(
                "{} of {} file types · {} opens in {}",
                st.opened - 1,
                st.opened,
                short_name(d, mime),
                app_name(d, other)
            ),
            format!("Use {app_name_} for all"),
        ),
        (Some(_), [_, ..]) => (
            format!(
                "{} of {} file types · {} open in other apps",
                st.opened - st.others.len(),
                st.opened,
                st.others.len()
            ),
            format!("Use {app_name_} for all"),
        ),
        _ => (String::new(), String::new()),
    };
    let note = match st.from {
        _ if st.app.is_none() => "No app opens them".to_owned(),
        _ if !st.set_somewhere => "Not set: the first app that opens them".to_owned(),
        Some(i) if !d.files[i].own => format!("{}'s default", d.os_name),
        _ => String::new(),
    };
    let app = choice(state, st.app.as_deref(), false);
    KindRow {
        key: s(kind.key),
        label: s(kind.label),
        hint: s(hint),
        icon: s(kind.icon),
        app,
        mixed: s(mixed),
        fix: s(fix),
        note: s(note),
    }
}

fn type_row(state: &mut State, mime: &str) -> TypeRow {
    let Some(d) = &state.data else { return TypeRow::default() };
    let name = type_name(d, mime);
    let line = match d.db.extensions(mime).first() {
        Some(e) => format!("{mime} · .{e}"),
        None => mime.to_owned(),
    };
    let app = mimeapps::resolve(&d.files, &d.apps, mime).app;
    TypeRow { mime: s(mime), name: s(name), line: s(line), app: choice(state, app.as_deref(), false) }
}

/// The file types that aren't their kind's app, and those set by the user that have no kind.
fn exceptions(d: &Data) -> Vec<String> {
    let mut out = Vec::new();
    for kind in KINDS.iter().filter(|k| !k.types.is_empty()) {
        let st = kinds::state(kind, &d.files, &d.apps, &d.db);
        if st.app.is_some() {
            out.extend(st.others.into_iter().filter(|(_, a)| a.is_some()).map(|(m, _)| m));
        }
    }
    for f in d.files.iter().filter(|f| f.own) {
        for (mime, _) in &f.defaults {
            if kinds::kind_of(mime).is_none() && !out.contains(mime) {
                out.push(mime.clone());
            }
        }
    }
    out
}

fn sources(d: &Data) -> Vec<DaSource> {
    let target = mimeapps::save_target(&d.files, &d.dirs);
    let mut out = Vec::new();
    for f in d.files.iter().filter(|f| f.exists) {
        let n = f.defaults.len();
        let mut what = vec![match n {
            1 => "1 file type".to_owned(),
            n => format!("{n} file types"),
        }];
        if !f.desktop.is_empty() {
            what.push(format!("read first on {}", capital(&f.desktop)));
        }
        if !f.own {
            what.push(format!("{}'s defaults", d.os_name));
        }
        out.push(DaSource {
            path: s(shorten_home(&f.path)),
            what: s(what.join(" · ")),
            saves: f.path == target,
            mono: true,
        });
    }
    if !d.files.iter().any(|f| f.exists && f.path == target) {
        out.insert(
            0,
            DaSource {
                path: s(shorten_home(&target)),
                what: s("Not there yet: made by your first choice"),
                saves: true,
                mono: true,
            },
        );
    }
    out.push(DaSource {
        path: s("Apps' own lists"),
        what: s("Every other type: the first app that says it opens it"),
        saves: false,
        mono: false,
    });
    out.push(DaSource {
        path: s(shorten_home(&d.terminal_list)),
        what: s("The terminal, for xdg-terminal-exec"),
        saves: true,
        mono: true,
    });
    out
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

pub fn view(state: &mut State) -> DefaultAppsView {
    let mut v = DefaultAppsView::default();
    if state.data.is_none() {
        v.state = s("loading");
        return v;
    }
    v.state = s("list");
    v.everyday =
        model(KINDS.iter().filter(|k| k.group == kinds::Group::Everyday).map(|k| kind_row(state, k)).collect());
    v.media = model(KINDS.iter().filter(|k| k.group == kinds::Group::Media).map(|k| kind_row(state, k)).collect());
    let d = state.data.as_ref().expect("loaded");
    // The hero: the browser, the files and the terminal, and the other apps' icons.
    let mut shown: Vec<String> = Vec::new();
    for k in KINDS.iter() {
        let app = if k.key == "terminal" { d.terminal.clone() } else { kinds::state(k, &d.files, &d.apps, &d.db).app };
        if let Some(a) = app
            && !shown.contains(&a)
        {
            shown.push(a);
        }
    }
    let main: Vec<String> = ["browser", "files", "terminal"]
        .iter()
        .filter_map(|key| {
            let k = kinds::kind(key)?;
            if k.key == "terminal" { d.terminal.clone() } else { kinds::state(k, &d.files, &d.apps, &d.db).app }
        })
        .map(|id| app_name(d, &id))
        .collect();
    v.hero_title = s(match main.as_slice() {
        [] => "No apps chosen yet".to_owned(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    });
    v.hero_more = s(if shown.len() > 5 {
        format!("and {} more open your links and files", shown.len() - 5)
    } else {
        "open your links and files".to_owned()
    });
    let target = mimeapps::save_target(&d.files, &d.dirs);
    let banner = d.files.iter().find(|f| f.path == target && !f.desktop.is_empty()).map(|f| {
        format!(
            "Your choices go to {}: it's read before mimeapps.list on {}, so a choice saved anywhere else wouldn't count.",
            shorten_home(&f.path),
            capital(&f.desktop)
        )
    });
    v.banner = s(banner.unwrap_or_default());
    v.sources = model(sources(d));
    let handled = d.names.len();
    let query = state.query.trim().to_owned();
    let types: Vec<String> = if query.is_empty() {
        let ex = exceptions(d);
        v.types_title = s(format!("EXCEPTIONS · {}", ex.len()));
        v.types_right = s("Not their kind's app");
        v.types_note = s(format!("Search for any of the {handled} file types the apps here can open."));
        ex
    } else {
        let mut found: Vec<String> =
            d.names.iter().filter(|(m, n)| d.db.matches(m, n, &query)).map(|(m, _)| m.clone()).collect();
        found.sort();
        found.truncate(60);
        v.types_title = s(match found.len() {
            0 => "NO FILE TYPE MATCHES".to_owned(),
            1 => "1 FILE TYPE MATCHES".to_owned(),
            n => format!("{n} FILE TYPES MATCH"),
        });
        v.types_note = s("A file type changed here becomes an exception of its kind above.");
        found
    };
    v.types = model(types.iter().map(|m| type_row(state, m)).collect());
    v.hero_apps = model(shown.iter().take(5).map(|id| choice(state, Some(id), false)).collect());
    v.sources_open = state.sources_open;
    v.error = s(&state.error);
    if let Some((target, all)) = state.picker.clone() {
        let d = state.data.as_ref().expect("loaded");
        let (title, hint, current, ids): (String, String, Option<String>, Vec<String>) = match &target {
            Target::Kind("terminal") => (
                "Terminal".into(),
                "The terminals installed. Apps open this one when they ask for a terminal.".into(),
                d.terminal.clone(),
                d.apps
                    .iter()
                    .filter(|a| a.categories.iter().any(|c| c == "TerminalEmulator"))
                    .map(|a| a.id.clone())
                    .collect(),
            ),
            Target::Kind(key) => {
                let k = kinds::kind(key).expect("a kind");
                (
                    k.label.into(),
                    format!("The apps that say they open {}.", k.label.to_lowercase()),
                    kinds::state(k, &d.files, &d.apps, &d.db).app,
                    kinds::offered(k, &d.apps),
                )
            }
            Target::Type(mime) => (
                type_name(d, mime),
                format!("The apps that say they open {mime}."),
                mimeapps::resolve(&d.files, &d.apps, mime).app,
                mimeapps::candidates(&d.files, &d.apps, mime),
            ),
        };
        let ids = if all {
            let mut every: Vec<&App> = d.apps.iter().filter(|a| !a.no_display).collect();
            every.sort_by_key(|a| a.name.to_lowercase());
            every.into_iter().map(|a| a.id.clone()).collect()
        } else {
            ids
        };
        v.picker_open = true;
        v.picker_title = s(title);
        v.picker_hint = s(if all {
            "Every app. One that doesn't say it opens these may not open them well.".to_owned()
        } else {
            hint
        });
        v.picker_all = all;
        v.picker_apps =
            model(ids.iter().map(|id| choice(state, Some(id), current.as_deref() == Some(id.as_str()))).collect());
    }
    v
}

pub fn render(window: &MainWindow, state: &mut State) {
    if window.get_page() == "default-apps" {
        window.set_default_apps(view(state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(root: &Path) -> Data {
        let dirs = Dirs {
            config_home: root.join("home/.config"),
            config_dirs: vec![root.join("etc/xdg")],
            data_home: root.join("home/.local/share"),
            data_dirs: vec![root.join("usr/share")],
            desktops: vec!["hyprland".into()],
        };
        let apps = desktop::all(&dirs);
        Data {
            files: mimeapps::files(&dirs),
            db: MimeDb::load(&dirs),
            names: HashMap::new(),
            icons: HashMap::new(),
            os_name: "EndeavourOS".into(),
            terminal: Some("kitty.desktop".into()),
            terminal_list: root.join("home/.config/xdg-terminals.list"),
            terminal_exec: true,
            fallback: String::new(),
            apps,
            dirs,
        }
    }

    fn write(root: &Path, path: &str, text: &str) {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn the_page_and_a_choice() {
        use slint::Model;
        let root = std::env::temp_dir().join(format!("lituus-defaultapps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let app = |id: &str, name: &str, mimes: &str| {
            write(
                &root,
                &format!("usr/share/applications/{id}"),
                &format!("[Desktop Entry]\nType=Application\nName={name}\nMimeType={mimes}\n"),
            )
        };
        app("firefox.desktop", "Firefox", "x-scheme-handler/http;x-scheme-handler/https;text/html;image/gif;");
        app("imv.desktop", "imv", "image/png;image/jpeg;image/gif;");
        app("kitty.desktop", "kitty", "");
        write(&root, "usr/share/mime/globs2", "50:image/png:*.png\n50:image/jpeg:*.jpg\n50:image/gif:*.gif\n");
        write(&root, "home/.config/mimeapps.list", "[Default Applications]\nimage/gif=firefox.desktop;\n");
        write(
            &root,
            "usr/share/applications/mimeapps.list",
            "[Default Applications]\nx-scheme-handler/http=firefox.desktop;\n",
        );
        let mut state = State { data: Some(data(&root)), ..Default::default() };
        let v = view(&mut state);
        let images = v.media.row_data(0).unwrap();
        assert_eq!(images.app.name, "imv");
        assert_eq!(images.mixed, "2 of 3 file types · GIF opens in Firefox");
        assert_eq!(images.fix, "Use imv for all");
        let browser = v.everyday.row_data(0).unwrap();
        assert_eq!((browser.app.name.as_str(), browser.note.as_str()), ("Firefox", ""));
        assert_eq!(v.hero_title, "Firefox and kitty");
        assert_eq!(v.types_title, "EXCEPTIONS · 1");
        // Use imv for all: saved to ~/.config/mimeapps.list, and the exception is gone.
        let weak = slint::Weak::default();
        act(&weak, &mut state, "fix", "images");
        let text = std::fs::read_to_string(root.join("home/.config/mimeapps.list")).unwrap();
        assert!(text.contains("image/gif=imv.desktop;") && text.contains("image/png=imv.desktop;"), "{text}");
        let v = view(&mut state);
        assert_eq!(v.media.row_data(0).unwrap().mixed, "");
        assert_eq!(v.types_title, "EXCEPTIONS · 0");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
