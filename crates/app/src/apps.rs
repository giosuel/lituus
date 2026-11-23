//! Desktop › Apps (D-38): the apps the launcher shows, from their desktop files, and how each
//! starts. A change is saved at once to the user's own copy (lituus_xdg::edit), never to the
//! system's file; Restore original deletes the copy. Hiding is NoDisplay=true. When the page
//! opens, Lituus looks for problems: a command it can't find, an icon the theme doesn't have, a
//! copy older than the package's file. The apps are read on a thread; icons are loaded here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lituus_xdg::Dirs;
use lituus_xdg::desktop::{self, App};
use lituus_xdg::edit::{self, DesktopFile, ENTRY};
use slint::{Image, ModelRc, SharedString, VecModel};

use crate::{
    AppAction, AppChange, AppChoice, AppFilter, AppFix, AppProblem, AppRow, AppSection, AppsView, MainWindow, update,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The command's program isn't there; maybe a newer AppImage beside where it was.
    NotFound { program: String, newer: Option<PathBuf> },
    /// The package's file changed after the user's copy was made.
    Outdated,
    /// The icon theme has no icon by its name.
    NoIcon,
}

pub struct Data {
    pub dirs: Dirs,
    pub apps: Vec<App>,
    pub icons: HashMap<String, PathBuf>,
    pub theme: String,
    pub problems: Vec<(String, Problem)>,
    pub electron: HashMap<String, bool>,
}

#[derive(Default)]
pub struct State {
    pub data: Option<Data>,
    pub filter: String,
    pub query: String,
    pub selected: Option<String>,
    /// Checked to hide at once (Ctrl+click, Shift+click).
    pub checked: Vec<String>,
    last_checked: Option<String>,
    pub message: String,
    images: HashMap<String, Image>,
    pub add: Option<AddForm>,
    /// Choosing an icon: for the app being added, or the selected one.
    pub icon: Option<IconPicker>,
}

#[derive(Clone, Debug, Default)]
pub struct AddForm {
    pub website: bool,
    pub program: String,
    pub program_note: String,
    pub name: String,
    pub comment: String,
    pub category: usize,
    pub terminal: bool,
    pub address: String,
    pub browser: usize,
    /// A theme icon's name, or a file.
    pub icon: String,
}

#[derive(Clone, Debug, Default)]
pub struct IconPicker {
    pub for_add: bool,
    pub query: String,
    pub results: Vec<(String, PathBuf)>,
}

/// The categories offered for an app added: (key, in words).
const ADD_CATEGORIES: [(&str, &str); 9] = [
    ("Utility", "Accessories"),
    ("Network", "Internet"),
    ("Office", "Office"),
    ("Graphics", "Graphics"),
    ("AudioVideo", "Sound and video"),
    ("Development", "Development"),
    ("Game", "Games"),
    ("Education", "Education"),
    ("System", "System"),
];

/// Chromium-based browsers, which open a website as an app (--app=); Firefox can't.
const CHROMIUMS: [&str; 10] = [
    "chromium",
    "chromium-browser",
    "google-chrome-stable",
    "google-chrome",
    "brave",
    "brave-browser",
    "vivaldi-stable",
    "vivaldi",
    "microsoft-edge-stable",
    "thorium-browser",
];

/// The browsers here that can: (their name, their program).
fn browsers(d: &Data) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for a in &d.apps {
        let program = edit::program(&a.exec);
        let base = Path::new(&program).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        if CHROMIUMS.contains(&base.as_str()) && !out.iter().any(|(_, p)| *p == program) {
            out.push((a.name.clone(), program));
        }
    }
    out
}

/// "https://app.todoist.com/x" → ("app.todoist.com", "Todoist").
fn site(address: &str) -> (String, String) {
    let host = address
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_owned();
    let labels: Vec<&str> = host.split('.').collect();
    let main = if labels.len() >= 2 { labels[labels.len() - 2] } else { labels.first().copied().unwrap_or_default() };
    let mut c = main.chars();
    let name = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
    (host, name)
}

fn slug(name: &str) -> String {
    let s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    s.split('-').filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-")
}

/// An AppImage's own desktop file and icon, extracted into a folder: (name, comment, categories,
/// icon file). The AppImage is made executable first.
fn read_appimage(path: &Path, icons_dir: &Path) -> Option<(String, String, Vec<String>, Option<PathBuf>)> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = path.metadata().ok()?.permissions();
    if perms.mode() & 0o100 == 0 {
        perms.set_mode(perms.mode() | 0o755);
        std::fs::set_permissions(path, perms).ok()?;
    }
    let tmp = std::env::temp_dir().join(format!("lituus-appimage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).ok()?;
    for pattern in ["*.desktop", ".DirIcon"] {
        let _ = std::process::Command::new(path)
            .args(["--appimage-extract", pattern])
            .current_dir(&tmp)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let root = tmp.join("squashfs-root");
    let desktop = std::fs::read_dir(&root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "desktop"))?;
    let app = desktop::parse("x.desktop", &desktop, &std::fs::read_to_string(&desktop).ok()?)?;
    let icon = root.join(".DirIcon");
    let icon = icon
        .canonicalize()
        .ok()
        .filter(|p| p.starts_with(root.canonicalize().unwrap_or_default()) && p.is_file())
        .and_then(|p| {
            let ext = p.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "png".into());
            let to = icons_dir.join(format!("{}.{ext}", slug(&app.name)));
            std::fs::create_dir_all(icons_dir).ok()?;
            std::fs::copy(&p, &to).ok()?;
            Some(to)
        });
    let _ = std::fs::remove_dir_all(&tmp);
    Some((app.name, app.comment, app.categories, icon))
}

/// The icon theme's icons whose names have `query` in them: (name, a file to show it).
fn find_icons(d: &Data, query: &str) -> Vec<(String, PathBuf)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let themes: Vec<&str> = [d.theme.as_str(), "hicolor"].into_iter().filter(|t| !t.is_empty()).collect();
    for base in d.dirs.data().map(|p| p.join("icons")) {
        for t in &themes {
            for size in ["scalable", "48x48", "64x64", "128x128", "256x256", "32x32"] {
                let Ok(entries) = std::fs::read_dir(base.join(t).join(size).join("apps")) else { continue };
                for e in entries.flatten() {
                    let p = e.path();
                    let Some(name) = p.file_stem().map(|n| n.to_string_lossy().to_string()) else { continue };
                    if name.to_lowercase().contains(&q) && !out.iter().any(|(n, _)| *n == name) {
                        out.push((name, p));
                    }
                }
            }
        }
    }
    out.sort_by_key(|(n, _)| (!n.to_lowercase().starts_with(&q), n.len(), n.clone()));
    out.truncate(24);
    out
}

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

fn read(dirs: Dirs) -> Data {
    let apps = desktop::all(&dirs);
    let theme = icon_theme(&dirs);
    let path = std::env::var("PATH").unwrap_or_default();
    let icons: HashMap<String, PathBuf> =
        apps.iter().filter_map(|a| Some((a.id.clone(), desktop::icon_file(&dirs, &theme, &a.icon)?))).collect();
    let electron =
        apps.iter().map(|a| (a.id.clone(), a.flatpak.is_empty() && edit::is_electron(&a.exec, &path))).collect();
    let problems = problems(&apps, &icons, &path);
    Data { dirs, apps, icons, theme, problems, electron }
}

/// What's wrong with the apps, found by looking at their files.
fn problems(apps: &[App], icons: &HashMap<String, PathBuf>, path: &str) -> Vec<(String, Problem)> {
    let flatpak = edit::find_program("flatpak", path).is_some();
    let mut out = Vec::new();
    for a in apps {
        let program = edit::program(&a.exec);
        let flatpak_app = !a.flatpak.is_empty() || program == "flatpak";
        if !program.is_empty() && !(flatpak_app && flatpak) && edit::find_program(&program, path).is_none() {
            out.push((a.id.clone(), Problem::NotFound { newer: newer_appimage(&program), program }));
            continue;
        }
        if let Some(pkg) = &a.package {
            let modified = |p: &Path| p.metadata().and_then(|m| m.modified()).ok();
            if let (Some(p), Some(o)) = (modified(pkg), modified(&a.path))
                && p > o
            {
                out.push((a.id.clone(), Problem::Outdated));
                continue;
            }
        }
        if !a.icon.is_empty() && !icons.contains_key(&a.id) && !a.no_display {
            out.push((a.id.clone(), Problem::NoIcon));
        }
    }
    out
}

/// An AppImage that went missing: a newer one beside where it was ("Cursor-0.45.1.AppImage" for
/// "Cursor-0.42.3.AppImage").
fn newer_appimage(program: &str) -> Option<PathBuf> {
    let p = Path::new(program);
    if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("appimage")) {
        return None;
    }
    let name = p.file_name()?.to_string_lossy().to_string();
    let stem: String = name.split(['-', '_']).next()?.to_lowercase();
    let mut found: Vec<PathBuf> = std::fs::read_dir(p.parent()?)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|f| {
            let n = f.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            n.starts_with(&stem) && n.ends_with(".appimage")
        })
        .collect();
    found.sort_by_key(|f| f.metadata().and_then(|m| m.modified()).ok());
    found.pop()
}

/// Reads the apps, on a thread.
pub fn open(weak: &slint::Weak<MainWindow>) {
    let weak = weak.clone();
    std::thread::spawn(move || {
        let data = read(Dirs::from_env());
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| update(&w, move |app| app.apps.data = Some(data)));
    });
}

pub fn status(state: &State) -> String {
    state.data.as_ref().map(|d| d.apps.iter().filter(|a| !a.no_display).count().to_string()).unwrap_or_default()
}

// ---- what the buttons do ------------------------------------------------------------------------

fn reload(state: &mut State) {
    if let Some(d) = &mut state.data {
        let dirs = d.dirs.clone();
        *d = read(dirs);
    }
    state.images.clear();
}

/// Changes the selected app's own copy.
fn change(state: &mut State, edit_: impl FnOnce(&mut DesktopFile)) {
    let (Some(d), Some(id)) = (&state.data, &state.selected) else { return };
    let Some(app) = d.apps.iter().find(|a| &a.id == id) else { return };
    match edit::change(&d.dirs, id, &app.path, edit_) {
        Ok(_) => state.message.clear(),
        Err(e) => state.message = format!("Couldn't save your copy of {}: {e}.", app.name),
    }
    reload(state);
}

fn selected_app(state: &State) -> Option<&App> {
    let d = state.data.as_ref()?;
    d.apps.iter().find(|a| Some(&a.id) == state.selected.as_ref())
}

fn split_list(text: &str) -> Vec<String> {
    text.split([';', ',']).map(str::trim).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

pub fn act(weak: &slint::Weak<MainWindow>, state: &mut State, verb: &str, arg: &str) {
    if add_or_icon(weak, state, verb, arg) {
        return;
    }
    match verb {
        "filter" => {
            state.filter = arg.to_owned();
            if arg == "problems" {
                state.selected = None;
            }
        }
        "search" => state.query = arg.to_owned(),
        "select" => {
            state.selected = Some(arg.to_owned());
            state.checked.clear();
        }
        "check" => {
            if let Some(i) = state.checked.iter().position(|c| c == arg) {
                state.checked.remove(i);
            } else {
                // The one selected joins the first time.
                if state.checked.is_empty()
                    && let Some(s) = state.selected.clone()
                    && s != arg
                {
                    state.checked.push(s);
                }
                state.checked.push(arg.to_owned());
            }
            state.last_checked = Some(arg.to_owned());
        }
        "check-range" => {
            let order: Vec<String> = rows(state).into_iter().flat_map(|(_, r)| r).map(|a| a.id).collect();
            let from = state.last_checked.clone().or(state.selected.clone());
            let (Some(a), Some(b)) =
                (from.and_then(|f| order.iter().position(|o| *o == f)), order.iter().position(|o| o == arg))
            else {
                return act(weak, state, "check", arg);
            };
            for id in &order[a.min(b)..=a.max(b)] {
                if !state.checked.contains(id) {
                    state.checked.push(id.clone());
                }
            }
            state.last_checked = Some(arg.to_owned());
        }
        "check-clear" => state.checked.clear(),
        "hide-checked" => {
            let ids = std::mem::take(&mut state.checked);
            if let Some(d) = &state.data {
                for id in &ids {
                    if let Some(a) = d.apps.iter().find(|a| &a.id == id)
                        && let Err(e) = edit::change(&d.dirs, id, &a.path, |f| f.set_bool("NoDisplay", true))
                    {
                        state.message = format!("Couldn't hide {}: {e}.", a.name);
                    }
                }
            }
            reload(state);
        }
        "show-all" => {
            if let Some(d) = &state.data {
                for a in d.apps.iter().filter(|a| a.no_display && a.own) {
                    let _ = edit::change(&d.dirs, &a.id, &a.path, |f| f.set(ENTRY, "NoDisplay", None));
                }
            }
            reload(state);
        }
        "shown" => {
            let hidden = selected_app(state).is_some_and(|a| a.no_display);
            change(state, |f| f.set_bool("NoDisplay", !hidden));
        }
        "name" if !arg.trim().is_empty() => change(state, |f| f.set(ENTRY, "Name", Some(arg.trim()))),
        "comment" => change(state, |f| f.set(ENTRY, "Comment", Some(arg.trim()).filter(|c| !c.is_empty()))),
        "exec" if !arg.trim().is_empty() => change(state, |f| f.set(ENTRY, "Exec", Some(arg.trim()))),
        "keywords" => change(state, |f| {
            let l = edit::list(&split_list(arg));
            f.set(ENTRY, "Keywords", Some(l.as_str()).filter(|l| !l.is_empty()))
        }),
        "wm-class" => change(state, |f| f.set(ENTRY, "StartupWMClass", Some(arg.trim()).filter(|c| !c.is_empty()))),
        "terminal" => {
            let on = !selected_app(state).is_some_and(|a| a.terminal);
            change(state, |f| f.set_bool("Terminal", on));
        }
        "gpu" => {
            let on = !selected_app(state).is_some_and(|a| a.gpu);
            change(state, |f| f.set_bool("PrefersNonDefaultGPU", on));
        }
        "wayland" => {
            let Some(app) = selected_app(state).cloned() else { return };
            let on = !app.exec.contains(edit::WAYLAND_FLAG);
            change(state, |f| {
                f.set(ENTRY, "Exec", Some(&edit::with_wayland(&app.exec, on)));
                for (id, _, exec) in &app.actions {
                    f.set(&format!("Desktop Action {id}"), "Exec", Some(&edit::with_wayland(exec, on)));
                }
            });
        }
        "category-add" if !arg.trim().is_empty() => {
            let Some(mut cats) = selected_app(state).map(|a| a.categories.clone()) else { return };
            for c in split_list(arg) {
                let c = category_key(&c);
                if !cats.contains(&c) {
                    cats.push(c);
                }
            }
            change(state, |f| f.set(ENTRY, "Categories", Some(&edit::list(&cats))));
        }
        "category-remove" => {
            let Some(mut cats) = selected_app(state).map(|a| a.categories.clone()) else { return };
            cats.retain(|c| category_label(c) != arg && c != arg);
            change(state, |f| f.set(ENTRY, "Categories", Some(edit::list(&cats).as_str()).filter(|l| !l.is_empty())));
        }
        "action-exec" => {
            if let Some((id, exec)) = arg.split_once('\u{1f}')
                && !exec.trim().is_empty()
            {
                change(state, |f| f.set(&format!("Desktop Action {id}"), "Exec", Some(exec.trim())));
            }
        }
        "restore" => {
            if let (Some(d), Some(id)) = (&state.data, &state.selected) {
                if let Err(e) = edit::restore(&d.dirs, id) {
                    state.message = format!("Couldn't restore it: {e}.");
                }
                reload(state);
            }
        }
        "launch" => {
            if let Some(a) = selected_app(state) {
                launch(a);
            }
        }
        "fix" => {
            let Some((id, verb)) = arg.split_once('\u{1f}') else { return };
            state.selected = Some(id.to_owned());
            fix(state, id, verb);
        }
        _ => {}
    }
}

/// The two dialogs' buttons; false for any other.
fn add_or_icon(weak: &slint::Weak<MainWindow>, state: &mut State, verb: &str, arg: &str) -> bool {
    let index = arg.parse::<usize>().unwrap_or(0);
    match verb {
        "add-open" => state.add = Some(AddForm::default()),
        "add-close" => state.add = None,
        "add-mode" => {
            if let Some(f) = &mut state.add {
                f.website = index == 1;
            }
        }
        "add-program" => {
            let path = PathBuf::from(expand_home(arg.trim()));
            let Some(f) = &mut state.add else { return true };
            f.program = arg.trim().to_owned();
            f.program_note.clear();
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("appimage")) && path.is_file() {
                f.program_note = "Reading the AppImage…".into();
                let icons_dir = state.data.as_ref().map(|d| d.dirs.data_home.join("icons")).unwrap_or_default();
                let weak = weak.clone();
                std::thread::spawn(move || {
                    let found = read_appimage(&path, &icons_dir);
                    let w = weak.clone();
                    let _ = weak.upgrade_in_event_loop(move |_| {
                        update(&w, move |app| {
                            let Some(f) = &mut app.apps.add else { return };
                            match found {
                                Some((name, comment, cats, icon)) => {
                                    f.name = name;
                                    f.comment = comment;
                                    if let Some(i) = ADD_CATEGORIES.iter().position(|(k, _)| cats.iter().any(|c| c == k)) {
                                        f.category = i;
                                    }
                                    if let Some(icon) = icon {
                                        f.icon = icon.display().to_string();
                                    }
                                    f.program_note = "Lituus made it executable; name, description and icon are from its own desktop file.".into();
                                }
                                None => f.program_note = "Lituus made it executable, but couldn't read its desktop file.".into(),
                            }
                        })
                    });
                });
            } else if !path.as_os_str().is_empty() && f.name.is_empty() {
                f.name = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            }
        }
        "add-choose" => {
            let weak = weak.clone();
            let home = PathBuf::from(expand_home("~/Applications"));
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
                let from = home.is_dir().then_some(home.as_path());
                let chosen = rt.block_on(crate::portal::open_file("Choose a program", &[], from));
                let w = weak.clone();
                let _ = weak.upgrade_in_event_loop(move |_| {
                    update(&w, |app| match chosen {
                        Ok(Some(p)) => act(&w, &mut app.apps, "add-program", &p.display().to_string()),
                        Ok(None) => {}
                        Err(e) => app.apps.message = format!("{e}."),
                    })
                });
            });
        }
        "add-name" => {
            if let Some(f) = &mut state.add {
                f.name = arg.trim().to_owned();
            }
        }
        "add-comment" => {
            if let Some(f) = &mut state.add {
                f.comment = arg.trim().to_owned();
            }
        }
        "add-category" => {
            if let Some(f) = &mut state.add {
                f.category = index.min(ADD_CATEGORIES.len() - 1);
            }
        }
        "add-terminal" => {
            if let Some(f) = &mut state.add {
                f.terminal = !f.terminal;
            }
        }
        "add-address" => {
            if let Some(f) = &mut state.add {
                f.address = arg.trim().to_owned();
                if !f.address.is_empty() && !f.address.contains("://") {
                    f.address = format!("https://{}", f.address);
                }
                if f.name.is_empty() {
                    f.name = site(&f.address).1;
                }
            }
        }
        "add-browser" => {
            if let Some(f) = &mut state.add {
                f.browser = index;
            }
        }
        "add-save" => add_save(state),
        "icon-open" => {
            let for_add = arg == "add";
            let query = if for_add {
                state.add.as_ref().map(|f| slug(&f.name)).unwrap_or_default()
            } else {
                selected_app(state)
                    .map(|a| if a.icon.contains('/') { slug(&a.name) } else { a.icon.clone() })
                    .unwrap_or_default()
            };
            let results = state.data.as_ref().map(|d| find_icons(d, &query)).unwrap_or_default();
            state.icon = Some(IconPicker { for_add, query, results });
        }
        "icon-close" => state.icon = None,
        "icon-query" => {
            let results = state.data.as_ref().map(|d| find_icons(d, arg)).unwrap_or_default();
            if let Some(p) = &mut state.icon {
                p.query = arg.to_owned();
                p.results = results;
            }
        }
        "icon-pick" => set_icon(state, arg),
        "icon-file" => {
            let weak = weak.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
                let chosen =
                    rt.block_on(crate::portal::open_file("Choose an icon", &[("Pictures", &["*.png", "*.svg"])], None));
                let w = weak.clone();
                let _ = weak.upgrade_in_event_loop(move |_| {
                    update(&w, |app| match chosen {
                        Ok(Some(p)) => set_icon(&mut app.apps, &p.display().to_string()),
                        Ok(None) => {}
                        Err(e) => app.apps.message = format!("{e}."),
                    })
                });
            });
        }
        _ => return false,
    }
    true
}

fn expand_home(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
        None => path.to_owned(),
    }
}

/// An icon chosen: for the app being added, or the selected app's (in its own copy).
fn set_icon(state: &mut State, icon: &str) {
    let for_add = state.icon.take().is_some_and(|p| p.for_add);
    if for_add {
        if let Some(f) = &mut state.add {
            f.icon = icon.to_owned();
        }
    } else {
        change(state, |f| f.set(ENTRY, "Icon", Some(icon)));
    }
}

/// Writes the new app's desktop file to ~/.local/share/applications, and selects it.
fn add_save(state: &mut State) {
    let (Some(f), Some(d)) = (state.add.clone(), &state.data) else { return };
    let mut file = DesktopFile::parse("[Desktop Entry]\nType=Application");
    let id;
    if f.website {
        let (host, _) = site(&f.address);
        let Some((_, browser)) = browsers(d).get(f.browser).cloned() else { return };
        id = format!("lituus-web-{}.desktop", slug(&host));
        file.set(ENTRY, "Exec", Some(&format!("{browser} --app={}", f.address)));
        file.set(ENTRY, "StartupWMClass", Some(&format!("chrome-{host}__-Default")));
        file.set(ENTRY, "Categories", Some("Network;WebApps;"));
    } else {
        id = format!("{}.desktop", slug(&f.name));
        let program = expand_home(&f.program);
        file.set(ENTRY, "Exec", Some(&shell_word(&program)));
        file.set(ENTRY, "Categories", Some(&format!("{};", ADD_CATEGORIES[f.category].0)));
        file.set_bool("Terminal", f.terminal);
        if !f.comment.is_empty() {
            file.set(ENTRY, "Comment", Some(&f.comment));
        }
    }
    file.set(ENTRY, "Name", Some(&f.name));
    if !f.icon.is_empty() {
        file.set(ENTRY, "Icon", Some(&f.icon));
    }
    let path = edit::own_path(&d.dirs, &id);
    let r = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, file.text()));
    match r {
        Ok(()) => {
            state.add = None;
            state.selected = Some(id);
            state.message.clear();
        }
        Err(e) => state.message = format!("Couldn't add {}: {e}.", f.name),
    }
    reload(state);
}

fn fix(state: &mut State, id: &str, verb: &str) {
    let Some(d) = &state.data else { return };
    let Some(app) = d.apps.iter().find(|a| a.id == id).cloned() else { return };
    match verb {
        // Added by the user: its file goes; the package's: hidden.
        "remove" => {
            let r = if app.own && app.package.is_none() {
                std::fs::remove_file(&app.path)
            } else {
                edit::change(&d.dirs, id, &app.path, |f| f.set_bool("NoDisplay", true)).map(|_| ())
            };
            if let Err(e) = r {
                state.message = format!("Couldn't remove {}: {e}.", app.name);
            }
            state.selected = None;
            reload(state);
        }
        // The package's new file, with the user's values for the page's keys.
        "rebase" => {
            let (Some(pkg), Ok(own)) = (&app.package, std::fs::read_to_string(&app.path)) else { return };
            let Ok(new) = std::fs::read_to_string(pkg) else { return };
            let own = DesktopFile::parse(&own);
            let mut file = DesktopFile::parse(&new);
            for key in [
                "Name",
                "Comment",
                "Icon",
                "Exec",
                "Terminal",
                "Categories",
                "Keywords",
                "StartupWMClass",
                "NoDisplay",
                "PrefersNonDefaultGPU",
            ] {
                file.set(ENTRY, key, own.get(ENTRY, key).as_deref());
            }
            if let Err(e) = std::fs::write(&app.path, file.text()) {
                state.message = format!("Couldn't save your copy: {e}.");
            }
            reload(state);
        }
        "show" => state.filter = "all".into(),
        "icon" => {
            let results = find_icons(d, &slug(&app.name));
            state.icon = Some(IconPicker { for_add: false, query: slug(&app.name), results });
        }
        other => {
            if let Some(path) = other.strip_prefix("use:") {
                let new = app.exec.replacen(&edit::program(&app.exec), &shell_word(path), 1);
                state.selected = Some(id.to_owned());
                change(state, |f| f.set(ENTRY, "Exec", Some(&new)));
            }
        }
    }
}

fn shell_word(text: &str) -> String {
    if text.contains(' ') { format!("\"{text}\"") } else { text.to_owned() }
}

/// Starts an app as a launcher would: its command without the field codes.
fn launch(app: &App) {
    let exec: Vec<&str> = app.exec.split_whitespace().filter(|w| !(w.len() == 2 && w.starts_with('%'))).collect();
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{} >/dev/null 2>&1 &", exec.join(" ")))
        .stdin(std::process::Stdio::null())
        .spawn();
}

// ---- the page -----------------------------------------------------------------------------------

fn s(text: impl AsRef<str>) -> SharedString {
    SharedString::from(text.as_ref())
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// The freedesktop main categories, in words, as the list's sections.
const SECTIONS: [(&[&str], &str); 9] = [
    (&["Network"], "INTERNET"),
    (&["Office"], "OFFICE"),
    (&["Graphics", "AudioVideo", "Audio", "Video"], "GRAPHICS AND VIDEO"),
    (&["Development"], "DEVELOPMENT"),
    (&["Game"], "GAMES"),
    (&["Education", "Science"], "EDUCATION AND SCIENCE"),
    (&["Utility"], "ACCESSORIES"),
    (&["Settings", "System"], "SYSTEM"),
    (&[], "OTHER"),
];

fn section_of(app: &App) -> usize {
    SECTIONS
        .iter()
        .position(|(cats, _)| app.categories.iter().any(|c| cats.contains(&c.as_str())))
        .unwrap_or(SECTIONS.len() - 1)
}

/// A category as people say it: "WebBrowser" → "Web browser".
fn category_label(key: &str) -> String {
    let known = [
        ("AudioVideo", "Sound and video"),
        ("WebBrowser", "Web browser"),
        ("TextEditor", "Text editor"),
        ("TerminalEmulator", "Terminal"),
        ("FileManager", "File manager"),
        ("Network", "Internet"),
        ("Utility", "Accessories"),
        ("Game", "Games"),
        ("IDE", "IDE"),
    ];
    if let Some((_, l)) = known.iter().find(|(k, _)| *k == key) {
        return (*l).to_owned();
    }
    // Camel case into words; acronyms ("GNOME", "2D") stay as they are.
    let chars: Vec<char> = key.trim_start_matches("X-").replace('-', " ").chars().collect();
    let mut words: Vec<String> = vec![String::new()];
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1);
        let split = c == ' '
            || (c.is_uppercase() && prev.is_some_and(|p| p.is_lowercase()))
            || (c.is_uppercase()
                && prev.is_some_and(|p| p.is_uppercase() || p.is_ascii_digit())
                && next.is_some_and(|n| n.is_lowercase()));
        if split && !words.last().is_some_and(String::is_empty) {
            words.push(String::new());
        }
        if c != ' ' {
            words.last_mut().expect("a word").push(c);
        }
    }
    words
        .iter()
        .filter(|w| !w.is_empty())
        .enumerate()
        .map(|(i, w)| if i > 0 && w.chars().any(char::is_lowercase) { w.to_lowercase() } else { w.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What someone types, as a category key: "Web browser" → "WebBrowser".
fn category_key(text: &str) -> String {
    let t = text.trim();
    let reverse =
        [("Sound and video", "AudioVideo"), ("Internet", "Network"), ("Accessories", "Utility"), ("Games", "Game")];
    if let Some((_, k)) = reverse.iter().find(|(l, _)| l.eq_ignore_ascii_case(t)) {
        return (*k).to_owned();
    }
    t.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect()
}

fn problem_of<'a>(d: &'a Data, id: &str) -> Option<&'a Problem> {
    d.problems.iter().find(|(i, _)| i == id).map(|(_, p)| p)
}

fn matches(a: &App, q: &str) -> bool {
    let q = q.to_lowercase();
    q.is_empty()
        || a.name.to_lowercase().contains(&q)
        || a.exec.to_lowercase().contains(&q)
        || a.id.to_lowercase().contains(&q)
        || a.generic_name.to_lowercase().contains(&q)
        || a.keywords.iter().any(|k| k.to_lowercase().contains(&q))
}

/// The list as shown: (section title, apps).
fn rows(state: &State) -> Vec<(String, Vec<App>)> {
    let Some(d) = &state.data else { return Vec::new() };
    let keep = |a: &App| match state.filter.as_str() {
        "changed" => a.own && a.package.is_some(),
        "hidden" => a.no_display,
        "added" => a.own && a.package.is_none(),
        "problems" => problem_of(d, &a.id).is_some(),
        _ => true,
    };
    let mut apps: Vec<App> = d.apps.iter().filter(|a| keep(a) && matches(a, &state.query)).cloned().collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    if matches!(state.filter.as_str(), "" | "all") && state.query.is_empty() && state.checked.is_empty() {
        let mut out: Vec<(String, Vec<App>)> = Vec::new();
        for (i, (_, title)) in SECTIONS.iter().enumerate() {
            let in_it: Vec<App> = apps.iter().filter(|a| section_of(a) == i).cloned().collect();
            if !in_it.is_empty() {
                out.push(((*title).to_owned(), in_it));
            }
        }
        return out;
    }
    vec![(String::new(), apps)]
}

fn choice(state: &mut State, app: &App) -> AppChoice {
    let icon = match state.data.as_ref().and_then(|d| d.icons.get(&app.id)) {
        Some(path) => state
            .images
            .entry(app.id.clone())
            .or_insert_with(|| Image::load_from_path(path).unwrap_or_default())
            .clone(),
        None => Image::default(),
    };
    AppChoice {
        id: s(&app.id),
        name: s(&app.name),
        detail: s(&app.id),
        letter: s(app.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()),
        icon,
        selected: false,
    }
}

fn source(a: &App) -> String {
    if !a.flatpak.is_empty() {
        format!("Flatpak · {}", a.flatpak)
    } else if a.own && a.package.is_none() {
        "Added by you".into()
    } else if a.own {
        "changed by you".into()
    } else {
        "from the system".into()
    }
}

fn kind_of(a: &App) -> String {
    if !a.generic_name.is_empty() {
        a.generic_name.clone()
    } else {
        a.categories
            .iter()
            .rev()
            .find(|c| !["GTK", "Qt", "KDE", "GNOME"].contains(&c.as_str()))
            .map(|c| category_label(c))
            .unwrap_or_default()
    }
}

fn join(parts: &[String]) -> String {
    parts.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join(" · ")
}

fn problem_view(state: &mut State, app: &App, p: &Problem) -> AppProblem {
    let fix = |verb: &str, label: &str| AppFix { verb: s(verb), label: s(label) };
    let (title, text, fixes) = match p {
        Problem::NotFound { program, newer } => {
            let mut fixes = Vec::new();
            let mut text = format!("Its command runs {program}, which isn't there any more.");
            if let Some(n) = newer {
                let name = n.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                text += &format!(" A newer one is in the same folder: {name}.");
                fixes.push(fix(&format!("use:{}", n.display()), &format!("Use {name}")));
            }
            fixes.push(fix("remove", "Remove from launcher"));
            (format!("{} can't start", app.name), text, fixes)
        }
        Problem::Outdated => (
            format!("{}'s package changed its file", app.name),
            "Your copy still has the old one, with your changes. Lituus can put your changes on the new one."
                .to_owned(),
            vec![fix("rebase", "Put my change on the new one"), fix("show", "Show")],
        ),
        Problem::NoIcon => (
            format!("{} has no icon", app.name),
            format!("Your icon theme has no icon named {}.", app.icon),
            vec![fix("icon", "Choose an icon…"), fix("show", "Show")],
        ),
    };
    AppProblem { id: s(&app.id), title: s(title), text: s(text), fixes: model(fixes), app: choice(state, app) }
}

pub fn view(state: &mut State) -> AppsView {
    let mut v = AppsView::default();
    let Some(d) = &state.data else {
        v.state = s("loading");
        return v;
    };
    v.state = s("list");
    let apps = d.apps.clone();
    let problems = d.problems.clone();
    let shown = apps.iter().filter(|a| !a.no_display).count();
    let hidden = apps.len() - shown;
    let changed = apps.iter().filter(|a| a.own && a.package.is_some()).count();
    let added = apps.iter().filter(|a| a.own && a.package.is_none()).count();
    v.hero_title =
        s(if shown == 1 { "1 app in your launcher".to_owned() } else { format!("{shown} apps in your launcher") });
    let mut line = vec![format!("{hidden} hidden")];
    if changed + added > 0 {
        line.push(format!("{changed} changed and {added} added by you"));
    }
    v.hero_line = s(line.join(" · "));
    v.hero_problems = s(match problems.len() {
        0 => String::new(),
        1 => "· 1 problem".into(),
        n => format!("· {n} problems"),
    });
    let mut with_icons: Vec<App> =
        apps.iter().filter(|a| !a.no_display && d.icons.contains_key(&a.id)).take(6).cloned().collect();
    with_icons.truncate(6);
    v.hero_apps = model(with_icons.iter().map(|a| choice(state, a)).collect());
    let filter = if state.filter.is_empty() { "all".to_owned() } else { state.filter.clone() };
    v.filters = model(
        [
            ("all", "All", apps.len()),
            ("changed", "Changed", changed),
            ("hidden", "Hidden", hidden),
            ("added", "Added", added),
            ("problems", "Problems", problems.len()),
        ]
        .iter()
        .map(|(id, label, n)| AppFilter {
            id: s(id),
            label: s(label),
            count: s(n.to_string()),
            selected: filter == *id,
            warn: *id == "problems" && *n > 0,
        })
        .collect(),
    );
    let lists = rows(state);
    v.list_title = s(match filter.as_str() {
        "changed" => "CHANGED BY YOU".to_owned(),
        "hidden" => format!("HIDDEN · {hidden}"),
        "added" => "ADDED BY YOU".to_owned(),
        "problems" => format!("PROBLEMS · {}", problems.len()),
        _ if !state.query.is_empty() || !state.checked.is_empty() => "ALL APPS, A–Z".to_owned(),
        _ => String::new(),
    });
    let selecting = !state.checked.is_empty();
    let mut sections = Vec::new();
    for (title, list) in lists {
        let mut out = Vec::new();
        for a in list {
            let p = problems.iter().find(|(i, _)| *i == a.id).map(|(_, p)| p.clone());
            let (badge, tone) = match (&p, a.own && a.package.is_some()) {
                (Some(Problem::NotFound { .. }), _) => ("Can't start", "warn"),
                (Some(Problem::Outdated), _) => ("Outdated", "warn"),
                (_, true) => ("Changed", "accent"),
                _ if selecting => ("select", ""),
                _ => ("", ""),
            };
            let sub = match (&p, filter.as_str()) {
                (Some(Problem::NotFound { .. }), "problems") => "Its program is gone".to_owned(),
                (Some(Problem::Outdated), "problems") => "The package's file changed".to_owned(),
                (Some(Problem::NoIcon), "problems") => "No icon".to_owned(),
                _ => join(&[kind_of(&a), source(&a)]),
            };
            out.push(AppRow {
                id: s(&a.id),
                name: s(&a.name),
                sub: s(sub),
                app: choice(state, &a),
                badge: s(if selecting && badge.is_empty() { "select" } else { badge }),
                badge_tone: s(tone),
                selected: !selecting && state.selected.as_deref() == Some(a.id.as_str()),
                checked: state.checked.contains(&a.id),
                faint: a.no_display,
            });
        }
        sections.push(AppSection { title: s(title), rows: model(out) });
    }
    v.sections = model(sections);
    v.list_note = s(if filter == "hidden" { "Hidden apps still open their files and links." } else { "" });
    v.show_all = filter == "hidden" && apps.iter().any(|a| a.no_display && a.own);
    v.selecting = selecting;
    v.checked = state.checked.len() as i32;
    let checked_apps: Vec<App> =
        state.checked.iter().filter_map(|id| apps.iter().find(|a| &a.id == id)).take(6).cloned().collect();
    v.checked_apps = model(checked_apps.iter().map(|a| choice(state, a)).collect());
    v.message = s(&state.message);
    add_view(state, &mut v);
    // The selected app.
    let app = state.selected.as_ref().and_then(|id| apps.iter().find(|a| &a.id == id)).cloned();
    let Some(a) = app else {
        if filter == "problems" {
            let list: Vec<AppProblem> = problems
                .iter()
                .filter_map(|(id, p)| Some((apps.iter().find(|a| &a.id == id)?.clone(), p.clone())))
                .map(|(a, p)| problem_view(state, &a, &p))
                .collect();
            v.other_problems = model(list);
        }
        v.footer = s(if filter == "problems" {
            "Lituus looks for these when the page opens: commands it can't find, icons your icon theme doesn't have, copies older than the package's file."
        } else {
            "A change makes your own copy in ~/.local/share/applications, which apps read first. The system's file stays as it is."
        });
        return v;
    };
    let d = state.data.as_ref().expect("loaded");
    v.has_app = true;
    v.id = s(&a.id);
    v.title = s(&a.name);
    v.line = s(join(&[kind_of(&a), source(&a)]));
    v.path = s(lituus_xdg::shorten_home(&a.path));
    v.shown = !a.no_display;
    v.shown_note = s(if !a.no_display {
        "Off hides it from rofi, wofi, fuzzel and the like; it stays installed and still opens links."
    } else if a.own {
        "Hidden by you: your copy says NoDisplay=true. It stays installed; turn it on to show it again."
    } else {
        "Its package hides it from launchers. Turn it on to show it anyway (in your own copy)."
    });
    v.name = s(&a.name);
    v.comment = s(&a.comment);
    v.exec = s(&a.exec);
    v.exec_note = s(if a.exec.contains("%u") || a.exec.contains("%U") {
        "%u is the link it was asked to open."
    } else if a.exec.contains("%f") || a.exec.contains("%F") {
        "%f is the file it was asked to open."
    } else {
        ""
    });
    v.terminal = a.terminal;
    v.gpu = a.gpu;
    v.electron = d.electron.get(&a.id).copied().unwrap_or(false);
    v.wayland = a.exec.contains(edit::WAYLAND_FLAG);
    v.categories = model(a.categories.iter().map(|c| s(category_label(c))).collect());
    v.keywords = s(a.keywords.join("; "));
    v.actions =
        model(a.actions.iter().map(|(id, name, exec)| AppAction { id: s(id), name: s(name), exec: s(exec) }).collect());
    v.wm_class = s(&a.wm_class);
    let labels = [
        ("Name", "Name"),
        ("Comment", "Description"),
        ("Icon", "Icon"),
        ("Exec", "Command"),
        ("Terminal", "In a terminal"),
        ("Categories", "Categories"),
        ("Keywords", "Search words"),
        ("StartupWMClass", "Window class"),
        ("NoDisplay", "Hidden"),
        ("PrefersNonDefaultGPU", "Dedicated GPU"),
        ("Actions", "Quick actions"),
    ];
    if let Some(pkg) = &a.package
        && let (Ok(p), Ok(o)) = (std::fs::read_to_string(pkg), std::fs::read_to_string(&a.path))
    {
        v.changes = model(
            edit::differences(&p, &o)
                .into_iter()
                .map(|(k, old, new)| {
                    let label = labels.iter().find(|(key, _)| *key == k).map_or(k.clone(), |(_, l)| (*l).to_owned());
                    let (old, new) =
                        if k == "Actions" { (format!("{old} actions"), format!("{new} actions")) } else { (old, new) };
                    AppChange { label: s(label), old: s(old), new: s(new) }
                })
                .collect(),
        );
        v.can_restore = true;
        v.footer = s(format!(
            "Your copy is read instead of {}. When the package changes its file, Lituus offers to put your changes on the new one.",
            pkg.display()
        ));
    } else if a.own {
        v.footer = s(format!(
            "Added by you: {}. Remove it under Problems or delete the file.",
            lituus_xdg::shorten_home(&a.path)
        ));
    } else {
        v.footer = s(
            "A change makes your own copy in ~/.local/share/applications, which apps read first. The system's file stays as it is.",
        );
    }
    if let Some(p) = problem_of(d, &a.id).cloned() {
        v.problem = problem_view(state, &a, &p);
    }
    v
}

fn icon_choice(state: &mut State, key: &str, name: &str, file: Option<&Path>) -> AppChoice {
    let icon = match file {
        Some(p) => state
            .images
            .entry(format!("icon:{}", p.display()))
            .or_insert_with(|| Image::load_from_path(p).unwrap_or_default())
            .clone(),
        None => Image::default(),
    };
    AppChoice {
        id: s(key),
        name: s(name),
        letter: s(name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()),
        icon,
        ..Default::default()
    }
}

fn add_view(state: &mut State, v: &mut AppsView) {
    if let Some(f) = state.add.clone() {
        let d = state.data.as_ref().expect("loaded");
        let browsers = browsers(d);
        let icon_file = if f.icon.contains('/') {
            Some(PathBuf::from(&f.icon))
        } else {
            desktop::icon_file(&d.dirs, &d.theme, &f.icon)
        };
        let own_dir = lituus_xdg::shorten_home(&d.dirs.data_home.join("applications"));
        v.add_open = true;
        v.add_website = f.website;
        v.add_program = s(&f.program);
        v.add_program_note = s(if f.program_note.is_empty() {
            "Lituus makes an AppImage executable if it isn't."
        } else {
            &f.program_note
        });
        v.add_name = s(&f.name);
        v.add_name_note = s(if f.website { "Click the icon to choose one." } else { "Click the icon to change it." });
        v.add_comment = s(&f.comment);
        v.add_categories = model(ADD_CATEGORIES.iter().map(|(_, l)| s(l)).collect());
        v.add_category = f.category as i32;
        v.add_terminal = f.terminal;
        v.add_address = s(&f.address);
        v.add_browsers = model(browsers.iter().map(|(n, _)| s(n)).collect());
        v.add_browser = f.browser as i32;
        v.add_browser_note = s(if browsers.is_empty() {
            "Websites as apps need a Chromium-based browser (Chromium, Brave, Vivaldi); none is installed. Firefox has no app windows."
        } else {
            "Websites as apps need a Chromium-based browser (Chromium, Brave, Vivaldi); Firefox has no app windows."
        });
        let name = if f.name.is_empty() { "?".to_owned() } else { f.name.clone() };
        v.add_icon = icon_choice(state, "add", &name, icon_file.as_deref());
        v.add_footer = s(if f.website {
            match browsers.get(f.browser) {
                Some((_, b)) if !f.address.is_empty() => format!("Runs {b} --app={}", f.address),
                _ => String::new(),
            }
        } else if f.name.is_empty() {
            String::new()
        } else {
            format!("Saved as {own_dir}/{}.desktop", slug(&f.name))
        });
        v.add_can_save = !f.name.is_empty()
            && if f.website { !f.address.is_empty() && !browsers.is_empty() } else { !f.program.is_empty() };
    }
    if let Some(p) = state.icon.clone() {
        let current = if p.for_add {
            state.add.as_ref().map(|f| f.icon.clone()).unwrap_or_default()
        } else {
            selected_app(state).map(|a| a.icon.clone()).unwrap_or_default()
        };
        let whose = if p.for_add {
            state.add.as_ref().map(|f| f.name.clone()).unwrap_or_default()
        } else {
            selected_app(state).map(|a| a.name.clone()).unwrap_or_default()
        };
        v.icon_open = true;
        v.icon_title =
            s(if whose.is_empty() { "Choose an icon".to_owned() } else { format!("Choose an icon for {whose}") });
        v.icon_query = s(&p.query);
        v.icon_note = s(match p.results.len() {
            0 if p.query.trim().is_empty() => "Type part of an icon's name.".to_owned(),
            0 => format!("No icon in your icon theme matches \"{}\".", p.query.trim()),
            n => format!(
                "{n} icons in your icon theme match \"{}\". A name follows the theme when you change it; a file stays the same.",
                p.query.trim()
            ),
        });
        let results: Vec<AppChoice> = p
            .results
            .iter()
            .map(|(name, file)| {
                let mut c = icon_choice(state, name, name, Some(file));
                c.selected = *name == current;
                c
            })
            .collect();
        v.icon_results = model(results);
    }
}

pub fn render(window: &MainWindow, state: &mut State) {
    if window.get_page() == "apps" {
        window.set_apps(view(state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    fn write(root: &Path, path: &str, text: &str) {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn changing_hiding_and_restoring() {
        let root = std::env::temp_dir().join(format!("lituus-apps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write(&root, "bin/obsidian", "#!/bin/sh\nexec electron37 /usr/lib/obsidian/app.asar \"$@\"\n");
        write(
            &root,
            "usr/share/applications/obsidian.desktop",
            &format!(
                "[Desktop Entry]\nType=Application\nName=Obsidian\nGenericName=Notes\nExec={} %U\nCategories=Office;\n",
                root.join("bin/obsidian").display()
            ),
        );
        write(
            &root,
            "usr/share/applications/avahi.desktop",
            "[Desktop Entry]\nType=Application\nName=Avahi SSH Server Browser\nExec=bssh\nCategories=Network;\n",
        );
        write(
            &root,
            "home/.local/share/applications/cursor.desktop",
            "[Desktop Entry]\nType=Application\nName=Cursor\nExec=/nowhere/Cursor-0.42.3.AppImage\n",
        );
        let dirs = Dirs {
            config_home: root.join("home/.config"),
            config_dirs: vec![],
            data_home: root.join("home/.local/share"),
            data_dirs: vec![root.join("usr/share")],
            desktops: vec![],
        };
        let mut state = State { data: Some(read(dirs)), ..Default::default() };
        let weak = slint::Weak::default();
        // bssh isn't there either: two that can't start.
        let v = view(&mut state);
        assert_eq!(v.hero_title, "3 apps in your launcher");
        assert_eq!(v.hero_problems, "· 2 problems");
        act(&weak, &mut state, "select", "obsidian.desktop");
        let v = view(&mut state);
        assert!(v.electron && !v.wayland && v.changes.row_count() == 0);
        act(&weak, &mut state, "wayland", "");
        act(&weak, &mut state, "category-add", "Development");
        let v = view(&mut state);
        assert!(v.wayland && v.can_restore, "{:?}", v.exec);
        assert!(v.exec.contains("obsidian --ozone-platform-hint=auto %U"));
        let changes: Vec<String> = v.changes.iter().map(|c| c.label.to_string()).collect();
        assert_eq!(changes, ["Command", "Categories"]);
        assert_eq!(v.categories.row_count(), 2);
        // Hiding two at once, then all back.
        act(&weak, &mut state, "check", "avahi.desktop");
        assert_eq!(state.checked, ["obsidian.desktop", "avahi.desktop"]);
        act(&weak, &mut state, "hide-checked", "");
        assert_eq!(view(&mut state).hero_title, "1 app in your launcher");
        act(&weak, &mut state, "filter", "hidden");
        assert!(view(&mut state).show_all);
        act(&weak, &mut state, "show-all", "");
        act(&weak, &mut state, "select", "obsidian.desktop");
        act(&weak, &mut state, "restore", "");
        let v = view(&mut state);
        assert!(!v.can_restore && !v.wayland);
        assert!(!root.join("home/.local/share/applications/obsidian.desktop").exists());
        // Cursor added by the user can't start: removing it deletes its file.
        act(&weak, &mut state, "fix", "cursor.desktop\u{1f}remove");
        assert!(!root.join("home/.local/share/applications/cursor.desktop").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adding_a_website() {
        let root = std::env::temp_dir().join(format!("lituus-apps-add-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write(
            &root,
            "usr/share/applications/chromium.desktop",
            "[Desktop Entry]\nType=Application\nName=Chromium\nExec=/usr/bin/chromium %U\n",
        );
        let dirs = Dirs {
            data_home: root.join("home/.local/share"),
            data_dirs: vec![root.join("usr/share")],
            ..Default::default()
        };
        let mut state = State { data: Some(read(dirs)), ..Default::default() };
        let weak = slint::Weak::default();
        act(&weak, &mut state, "add-open", "");
        act(&weak, &mut state, "add-mode", "1");
        act(&weak, &mut state, "add-address", "app.todoist.com");
        let v = view(&mut state);
        assert_eq!(
            (v.add_name.as_str(), v.add_footer.as_str()),
            ("Todoist", "Runs /usr/bin/chromium --app=https://app.todoist.com")
        );
        assert!(v.add_can_save);
        act(&weak, &mut state, "add-save", "");
        let text =
            std::fs::read_to_string(root.join("home/.local/share/applications/lituus-web-app-todoist-com.desktop"))
                .unwrap();
        assert!(
            text.contains("Exec=/usr/bin/chromium --app=https://app.todoist.com") && text.contains("Name=Todoist"),
            "{text}"
        );
        assert_eq!(state.selected.as_deref(), Some("lituus-web-app-todoist-com.desktop"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn categories_in_words() {
        assert_eq!(category_label("WebBrowser"), "Web browser");
        assert_eq!(category_label("Office"), "Office");
        assert_eq!(category_label("GNOME"), "GNOME");
        assert_eq!(category_label("2DGraphics"), "2D graphics");
        assert_eq!(category_label("VectorGraphics"), "Vector graphics");
        assert_eq!(category_label("X-GNOME-SystemSettings"), "GNOME system settings");
        assert_eq!(category_key("Web browser"), "WebBrowser");
        assert_eq!(category_key("Internet"), "Network");
    }
}
