//! Desktop › Apps (D-38): the apps the launcher shows, from their desktop files, and how each
//! starts. A change is saved at once to the user's own copy (lituus_xdg::edit), never to the
//! system's file; Restore original deletes the copy. Hiding is NoDisplay=true. When the page
//! opens, Lituus looks for problems: a command it can't find, an icon the theme doesn't have, a
//! copy older than the package's file. The apps are read on a thread, and their icons drawn there
//! too, so opening the page never waits for them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use lituus_system::terminal::Terminal;
use lituus_xdg::Dirs;
use lituus_xdg::desktop::{self, App};
use lituus_xdg::edit::{self, DesktopFile, ENTRY};
use lituus_xdg::exec;
use slint::{Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::{
    APP, AppAction, AppChange, AppChoice, AppFilter, AppFix, AppProblem, AppRow, AppSection, AppsView, ChipLine,
    MainWindow, input, update,
};

/// An icon drawn for the lists, and its file's time, to draw it again when the file changes.
type Drawn = (Option<SystemTime>, SharedPixelBuffer<Rgba8Pixel>);

/// The lists' icons are drawn this big (in logical pixels, times the screen's scale): the rows
/// show them at 28, the selection at 36, the hero at 20. The details' 56 px one is loaded from
/// its file, for the one app selected.
const LIST_ICON: f32 = 36.0;

/// The details' icon, and the icon picker's (36).
const DETAILS_ICON: f32 = 56.0;

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
    /// The icons' files drawn for the lists, at `scale`.
    pub drawn: HashMap<PathBuf, Drawn>,
    pub scale: f32,
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
    /// Counts the reads started and the changes made to `data`: a read's result is used only
    /// when nothing happened after it started, so a slow read never puts back an older list.
    generation: u64,
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

/// `base` with -2, -3… until `taken` says no: so adding never replaces a file of the same name.
fn free_name(base: &str, ext: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut name = format!("{base}.{ext}");
    let mut n = 2;
    while taken(&name) {
        name = format!("{base}-{n}.{ext}");
        n += 1;
    }
    name
}

/// A folder of Lituus's own to extract into, new each time: in $XDG_RUNTIME_DIR (only the user's),
/// made with 0700 and failing if the name is there already, so nothing another user put in a
/// shared /tmp is used.
fn private_dir(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    (0..100).find_map(|_| {
        let dir = base.join(format!("lituus-{name}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).ok().map(|()| dir)
    })
}

/// An AppImage of type 2, the kind whose runtime extracts files on `--appimage-extract`: an ELF
/// program with "AI" and 2 at byte 8. Anything else would just start, with those arguments.
fn is_appimage(path: &Path) -> bool {
    use std::io::Read;
    let mut start = [0u8; 11];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut start)).is_ok()
        && start.starts_with(b"\x7fELF")
        && start[8..] == *b"AI\x02"
}

/// An AppImage's own desktop file and icon, extracted into a folder: (name, comment, categories,
/// icon file). The AppImage is made executable first (for its owner, as a download isn't).
fn read_appimage(path: &Path, icons_dir: &Path) -> Option<(String, String, Vec<String>, Option<PathBuf>)> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = path.metadata().ok()?.permissions();
    if perms.mode() & 0o100 == 0 {
        perms.set_mode(perms.mode() | 0o100);
        std::fs::set_permissions(path, perms).ok()?;
    }
    if !is_appimage(path) {
        return None;
    }
    let tmp = private_dir("appimage")?;
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
            let ext = p.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| {
                // .DirIcon has no extension: an SVG is text that says so near its start.
                let start = std::fs::read(&p).unwrap_or_default();
                let svg = String::from_utf8_lossy(&start[..start.len().min(1024)]).contains("<svg");
                (if svg { "svg" } else { "png" }).into()
            });
            let base = Some(slug(&app.name)).filter(|s| !s.is_empty()).unwrap_or_else(|| "lituus-app".into());
            let to = icons_dir.join(free_name(&base, &ext, |n| icons_dir.join(n).symlink_metadata().is_ok()));
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

/// Reads the apps; their icons are drawn for the lists, those in `known` only when their file
/// changed since.
fn read(dirs: Dirs, scale: f32, known: &HashMap<PathBuf, Drawn>) -> Data {
    let apps = desktop::all(&dirs);
    let theme = icon_theme(&dirs);
    let path = std::env::var("PATH").unwrap_or_default();
    let icons: HashMap<String, PathBuf> =
        apps.iter().filter_map(|a| Some((a.id.clone(), desktop::icon_file(&dirs, &theme, &a.icon)?))).collect();
    let electron =
        apps.iter().map(|a| (a.id.clone(), a.flatpak.is_empty() && edit::is_electron(&a.exec, &path))).collect();
    let problems = problems(&apps, &icons, &path);
    let side = (LIST_ICON * scale).round().max(1.0) as u32;
    let mut drawn = HashMap::new();
    for file in icons.values() {
        if drawn.contains_key(file) {
            continue;
        }
        let modified = file.metadata().and_then(|m| m.modified()).ok();
        let pixels = match known.get(file) {
            Some((m, pixels)) if *m == modified => Some(pixels.clone()),
            _ => draw_icon(file, side),
        };
        if let Some(pixels) = pixels {
            drawn.insert(file.clone(), (modified, pixels));
        }
    }
    Data { dirs, apps, icons, theme, problems, electron, drawn, scale }
}

fn is_svg(file: &Path) -> bool {
    file.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

/// usvg's options for an icon: an `<image>` in it is drawn from data in the file itself, never
/// from a file it names. Any icon theme or desktop file can name one, and a FIFO there would hang
/// the read, /dev/zero fill the memory.
fn svg_options() -> resvg::usvg::Options<'static> {
    let mut options = resvg::usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    options
}

/// An icon's pixels, at most `side` square: an SVG drawn at that size, a bigger picture made
/// smaller. None for what can't be read here; the window's thread then tries its file.
pub(crate) fn draw_icon(file: &Path, side: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    // Not a FIFO or a device, which a read would wait on or never finish.
    if !file.is_file() {
        return None;
    }
    if is_svg(file) {
        let tree = resvg::usvg::Tree::from_data(&std::fs::read(file).ok()?, &svg_options()).ok()?;
        let size = tree.size();
        let scale = side as f32 / size.width().max(size.height());
        let (w, h) = ((size.width() * scale).round().max(1.0) as u32, (size.height() * scale).round().max(1.0) as u32);
        let mut canvas = resvg::tiny_skia::Pixmap::new(w, h)?;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut canvas.as_mut());
        // tiny-skia's pixels are premultiplied.
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
        for (to, from) in pixels.make_mut_slice().iter_mut().zip(canvas.pixels()) {
            let c = from.demultiply();
            *to = Rgba8Pixel { r: c.red(), g: c.green(), b: c.blue(), a: c.alpha() };
        }
        return Some(pixels);
    }
    let picture = image::open(file).ok()?;
    let picture = if picture.width().max(picture.height()) > side {
        picture.resize(side, side, image::imageops::FilterType::CatmullRom)
    } else {
        picture
    };
    let rgba = picture.to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}

/// An icon's file as a picture for the window, `side` pixels at most. An SVG is drawn here with
/// `svg_options`: Slint's own loader would read the files it names, on the window's thread, and
/// takes no options.
pub(crate) fn icon_image(file: &Path, side: u32) -> Image {
    if is_svg(file) || !file.is_file() {
        return draw_icon(file, side).map(Image::from_rgba8).unwrap_or_default();
    }
    Image::load_from_path(file).unwrap_or_default()
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

/// Reads the apps and draws their icons, on a thread; icons drawn before are kept.
pub fn open(weak: &slint::Weak<MainWindow>) {
    let scale = weak.upgrade().map(|w| slint::ComponentHandle::window(&w).scale_factor()).unwrap_or(1.0);
    APP.with(|app| read_on_thread(weak, &mut app.borrow_mut().apps, Dirs::from_env(), scale));
}

fn read_on_thread(weak: &slint::Weak<MainWindow>, state: &mut State, dirs: Dirs, scale: f32) {
    let known = state.data.as_ref().filter(|d| d.scale == scale).map(|d| d.drawn.clone()).unwrap_or_default();
    let generation = next_generation(state);
    let weak = weak.clone();
    std::thread::spawn(move || {
        let data = read(dirs, scale, &known);
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| update(&w, move |app| landed(&mut app.apps, generation, data)));
    });
}

/// A read starts, or `data` changes: the reads started before are out of date.
fn next_generation(state: &mut State) -> u64 {
    state.generation += 1;
    state.generation
}

/// A read's result, started at `generation`: shown only when the list didn't change since.
fn landed(state: &mut State, generation: u64, data: Data) {
    if generation == state.generation {
        state.data = Some(data);
        state.images.clear();
    }
}

pub fn status(state: &State) -> String {
    state.data.as_ref().map(|d| d.apps.iter().filter(|a| !a.no_display).count().to_string()).unwrap_or_default()
}

// ---- what the buttons do ------------------------------------------------------------------------

/// After a change to the apps `ids`: they are read again at once, one file each, so the page
/// shows the change; the whole list (every app, its program and icon) is read on a thread, as
/// `open` does, and not on the window's.
fn reload(weak: &slint::Weak<MainWindow>, state: &mut State, ids: &[String]) {
    refresh(state, ids);
    if weak.upgrade().is_some()
        && let Some(d) = &state.data
    {
        let (dirs, scale) = (d.dirs.clone(), d.scale);
        read_on_thread(weak, state, dirs, scale);
    }
}

/// The apps `ids` read again from their files, in place: an app gone, added, or changed.
fn refresh(state: &mut State, ids: &[String]) {
    next_generation(state);
    state.images.clear();
    let Some(d) = &mut state.data else { return };
    let path = std::env::var("PATH").unwrap_or_default();
    let side = (LIST_ICON * d.scale).round().max(1.0) as u32;
    for id in ids {
        let at = d.apps.iter().position(|a| &a.id == id);
        d.icons.remove(id);
        d.electron.remove(id);
        let problem_at = d.problems.iter().position(|(i, _)| i == id);
        d.problems.retain(|(i, _)| i != id);
        let Some(app) = desktop::find(&d.dirs, id) else {
            if let Some(i) = at {
                d.apps.remove(i);
            }
            continue;
        };
        if let Some(file) = desktop::icon_file(&d.dirs, &d.theme, &app.icon) {
            let modified = file.metadata().and_then(|m| m.modified()).ok();
            if !d.drawn.get(&file).is_some_and(|(m, _)| *m == modified)
                && let Some(pixels) = draw_icon(&file, side)
            {
                d.drawn.insert(file.clone(), (modified, pixels));
            }
            d.icons.insert(id.clone(), file);
        }
        d.electron.insert(id.clone(), app.flatpak.is_empty() && edit::is_electron(&app.exec, &path));
        for (n, p) in problems(std::slice::from_ref(&app), &d.icons, &path).into_iter().enumerate() {
            let i = problem_at.map_or(d.problems.len(), |i| (i + n).min(d.problems.len()));
            d.problems.insert(i, p);
        }
        match at {
            Some(i) => d.apps[i] = app,
            None => d.apps.push(app),
        }
    }
}

/// Changes the selected app's own copy.
fn change(weak: &slint::Weak<MainWindow>, state: &mut State, edit_: impl FnOnce(&mut DesktopFile)) {
    let (Some(d), Some(id)) = (&state.data, state.selected.clone()) else { return };
    let Some(app) = d.apps.iter().find(|a| a.id == id) else { return };
    match edit::change(&d.dirs, &id, &app.path, edit_) {
        Ok(_) => state.message.clear(),
        Err(e) => state.message = format!("Couldn't save your copy of {}: {e}.", app.name),
    }
    reload(weak, state, &[id]);
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
            reload(weak, state, &ids);
        }
        "show-all" => {
            let mut ids = Vec::new();
            if let Some(d) = &state.data {
                for a in d.apps.iter().filter(|a| a.no_display && a.own) {
                    let _ = edit::change(&d.dirs, &a.id, &a.path, |f| f.set(ENTRY, "NoDisplay", None));
                    ids.push(a.id.clone());
                }
            }
            reload(weak, state, &ids);
        }
        // The switches turn what the file says, which the page may not show yet.
        "shown" => change(weak, state, |f| {
            let hidden = f.get(ENTRY, "NoDisplay").as_deref() == Some("true");
            f.set_bool("NoDisplay", !hidden);
        }),
        "name" if !arg.trim().is_empty() => change(weak, state, |f| f.set(ENTRY, "Name", Some(arg.trim()))),
        "comment" => change(weak, state, |f| f.set(ENTRY, "Comment", Some(arg.trim()).filter(|c| !c.is_empty()))),
        "exec" if !arg.trim().is_empty() => change(weak, state, |f| f.set(ENTRY, "Exec", Some(arg.trim()))),
        "keywords" => change(weak, state, |f| {
            let l = edit::list(&split_list(arg));
            f.set(ENTRY, "Keywords", Some(l.as_str()).filter(|l| !l.is_empty()))
        }),
        "wm-class" => {
            change(weak, state, |f| f.set(ENTRY, "StartupWMClass", Some(arg.trim()).filter(|c| !c.is_empty())))
        }
        "terminal" => change(weak, state, |f| {
            let on = f.get(ENTRY, "Terminal").as_deref() != Some("true");
            f.set_bool("Terminal", on);
        }),
        "gpu" => change(weak, state, |f| {
            let on = f.get(ENTRY, "PrefersNonDefaultGPU").as_deref() != Some("true");
            f.set_bool("PrefersNonDefaultGPU", on);
        }),
        // The commands and categories come from the file too: an older list would put back
        // what it had before a change.
        "wayland" => change(weak, state, |f| {
            let exec = f.get(ENTRY, "Exec").unwrap_or_default();
            let on = !exec.contains(edit::WAYLAND_FLAG);
            f.set(ENTRY, "Exec", Some(&edit::with_wayland(&exec, on)));
            for group in f.actions() {
                if let Some(exec) = f.get(&group, "Exec") {
                    f.set(&group, "Exec", Some(&edit::with_wayland(&exec, on)));
                }
            }
        }),
        "category-add" if !arg.trim().is_empty() => change(weak, state, |f| {
            let mut cats = split_list(&f.get(ENTRY, "Categories").unwrap_or_default());
            for c in split_list(arg) {
                let c = category_key(&c);
                if !cats.contains(&c) {
                    cats.push(c);
                }
            }
            f.set(ENTRY, "Categories", Some(&edit::list(&cats)));
        }),
        "category-remove" => change(weak, state, |f| {
            let mut cats = split_list(&f.get(ENTRY, "Categories").unwrap_or_default());
            cats.retain(|c| category_label(c) != arg && c != arg);
            f.set(ENTRY, "Categories", Some(edit::list(&cats).as_str()).filter(|l| !l.is_empty()));
        }),
        "action-exec" => {
            if let Some((id, exec)) = arg.split_once('\u{1f}')
                && !exec.trim().is_empty()
            {
                change(weak, state, |f| f.set(&format!("Desktop Action {id}"), "Exec", Some(exec.trim())));
            }
        }
        "restore" => {
            let (Some(d), Some(id)) = (&state.data, state.selected.clone()) else { return };
            if let Err(e) = edit::restore(&d.dirs, &id) {
                state.message = format!("Couldn't restore it: {e}.");
            }
            reload(weak, state, &[id]);
        }
        "launch" => {
            let Some(app) = selected_app(state).cloned() else { return };
            if let Err(e) = launch(&app) {
                state.message = format!("Couldn't open {}: {e}.", app.name);
            }
        }
        "fix" => {
            let Some((id, verb)) = arg.split_once('\u{1f}') else { return };
            state.selected = Some(id.to_owned());
            fix(weak, state, id, verb);
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
            if f.program.contains('%') {
                f.program_note = PERCENT_NOTE.into();
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("appimage")) && path.is_file() {
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
        "add-save" => add_save(weak, state),
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
        "icon-pick" => set_icon(weak, state, arg),
        "icon-file" => {
            let weak = weak.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
                let chosen =
                    rt.block_on(crate::portal::open_file("Choose an icon", &[("Pictures", &["*.png", "*.svg"])], None));
                let w = weak.clone();
                let _ = weak.upgrade_in_event_loop(move |_| {
                    update(&w, |app| match chosen {
                        Ok(Some(p)) => set_icon(&w, &mut app.apps, &p.display().to_string()),
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
fn set_icon(weak: &slint::Weak<MainWindow>, state: &mut State, icon: &str) {
    let for_add = state.icon.take().is_some_and(|p| p.for_add);
    if for_add {
        if let Some(f) = &mut state.add {
            f.icon = icon.to_owned();
        }
    } else {
        change(weak, state, |f| f.set(ENTRY, "Icon", Some(icon)));
    }
}

/// The new app's file id: from its name, a website's from its address, with -2, -3… when an app
/// or a file in an applications folder has it already, so adding never replaces the user's file
/// or hides a package's (two websites on one host are two apps); "lituus-app" for a name with no
/// letter a file name can take.
fn new_id(d: &Data, f: &AddForm) -> String {
    let base = if f.website {
        let host = slug(&site(&f.address).0);
        if host.is_empty() { "lituus-web".to_owned() } else { format!("lituus-web-{host}") }
    } else {
        Some(slug(&f.name)).filter(|s| !s.is_empty()).unwrap_or_else(|| "lituus-app".into())
    };
    free_name(&base, "desktop", |id| {
        d.apps.iter().any(|a| a.id == id)
            || d.dirs.applications().iter().any(|dir| dir.join(id).symlink_metadata().is_ok())
    })
}

/// The window class of a website's app window, in the `chrome-…-Default` form Lituus writes: the
/// address's host (without its port) and its path with each `/` as `_`, as Chromium names the
/// window on X11 ("mail.google.com__mail_u_0" for https://mail.google.com/mail/u/0), not only
/// the host. Brave, Vivaldi and Edge have prefixes of their own.
fn wm_class(address: &str) -> String {
    let rest = address.trim().split_once("://").map_or(address.trim(), |(_, r)| r);
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.split(':').next().unwrap_or_default();
    format!("chrome-{host}__{}-Default", path.trim_end_matches('/').replace('/', "_"))
}

/// What the Program field says under it when the path has a `%`: launchers read it as a field
/// code, and GLib refuses the spec's `%%` in a program's path.
const PERCENT_NOTE: &str =
    "Launchers can't start a program whose path has a % in it. Rename the file or its folder first.";

/// Writes the new app's desktop file to ~/.local/share/applications, and selects it.
fn add_save(weak: &slint::Weak<MainWindow>, state: &mut State) {
    let (Some(f), Some(d)) = (state.add.clone(), &state.data) else { return };
    let mut file = DesktopFile::parse("[Desktop Entry]\nType=Application");
    if f.website {
        let Some((_, browser)) = browsers(d).get(f.browser).cloned() else { return };
        // The address is one argument, quoted, with a `%` in it as `%%`: a launcher would read
        // `%3A` as a field code.
        let command = format!("{} {}", exec::quote(&browser), exec::quote(&format!("--app={}", f.address)));
        file.set(ENTRY, "Exec", Some(&command));
        file.set(ENTRY, "StartupWMClass", Some(&wm_class(&f.address)));
        file.set(ENTRY, "Categories", Some("Network;WebApps;"));
    } else {
        let program = expand_home(&f.program);
        if program.contains('%') {
            state.message = PERCENT_NOTE.into();
            return;
        }
        file.set(ENTRY, "Exec", Some(&exec::quote(&program)));
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
    let id = new_id(d, &f);
    let path = edit::own_path(&d.dirs, &id);
    // A new file only: never over one that came since the id was chosen.
    let r = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| {
        if path.symlink_metadata().is_ok() {
            return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "a file with its name came meanwhile"));
        }
        lituus_fs::write_atomic(&path, file.text())
    });
    match r {
        Ok(()) => {
            state.add = None;
            state.selected = Some(id.clone());
            state.message.clear();
        }
        Err(e) => state.message = format!("Couldn't add {}: {e}.", f.name),
    }
    reload(weak, state, &[id]);
}

fn fix(weak: &slint::Weak<MainWindow>, state: &mut State, id: &str, verb: &str) {
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
            reload(weak, state, &[app.id]);
        }
        // The package's new file, with the user's values for the page's keys.
        "rebase" => {
            let (Some(pkg), Ok(own)) = (&app.package, std::fs::read_to_string(&app.path)) else { return };
            let Ok(new) = std::fs::read_to_string(pkg) else { return };
            let own = DesktopFile::parse(&own);
            let mut file = DesktopFile::parse(&new);
            // The keys the copy has. One it hasn't may be one the package added since (often a
            // fix, as a StartupWMClass), so the package's stays; a switch it hasn't is off.
            for key in ["Name", "Comment", "Icon", "Exec", "Categories", "Keywords", "StartupWMClass"] {
                if let Some(value) = own.get(ENTRY, key) {
                    file.set(ENTRY, key, Some(&value));
                }
            }
            for key in ["Terminal", "NoDisplay", "PrefersNonDefaultGPU"] {
                file.set(ENTRY, key, own.get(ENTRY, key).filter(|v| v == "true").as_deref());
            }
            // The quick actions' commands, for the actions the new file still has.
            let actions = file.actions();
            for group in own.actions().into_iter().filter(|g| actions.contains(g)) {
                if let Some(exec) = own.get(&group, "Exec") {
                    file.set(&group, "Exec", Some(&exec));
                }
            }
            if let Err(e) = edit::change(&d.dirs, id, &app.path, |f| *f = file) {
                state.message = format!("Couldn't save your copy: {e}.");
            }
            reload(weak, state, &[app.id]);
        }
        "show" => state.filter = "all".into(),
        "icon" => {
            let results = find_icons(d, &slug(&app.name));
            state.icon = Some(IconPicker { for_add: false, query: slug(&app.name), results });
        }
        other => {
            if let Some(path) = other.strip_prefix("use:") {
                state.selected = Some(id.to_owned());
                change(weak, state, |f| {
                    let exec = f.get(ENTRY, "Exec").unwrap_or_default();
                    f.set(ENTRY, "Exec", Some(&edit::with_program(&exec, path)));
                });
            }
        }
    }
}

/// What Open runs: the app's command as a launcher runs it with no file, in a terminal when its
/// file asks for one.
fn launch_argv(app: &App, terminal: Option<&Terminal>) -> Vec<String> {
    let mut argv = exec::argv(&app.exec, &app.name, &app.icon, &app.path);
    if let Some(t) = terminal.filter(|_| app.terminal && !argv.is_empty()) {
        argv.splice(0..0, t.argv.iter().cloned());
    }
    argv
}

/// Starts an app as a launcher would. Its words are run as they are, with no shell between (a `$`
/// in a path is a `$`), in a process group of their own, so a Ctrl+C in the terminal Lituus was
/// started from doesn't reach it; and waited for on a thread, so it isn't left a zombie.
fn launch(app: &App) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    let terminal = if app.terminal {
        let env = lituus_system::updates::Env::from_env();
        let dirs = lituus_system::terminal::Dirs::from_env(&env.home);
        let terminal =
            lituus_system::terminal::default_terminal(&env, &dirs, std::env::var("TERMINAL").ok().as_deref());
        Some(terminal.ok_or_else(|| std::io::Error::other("there's no terminal to run it in"))?)
    } else {
        None
    };
    let argv = launch_argv(app, terminal.as_ref());
    let Some((program, args)) = argv.split_first() else { return Ok(()) };
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
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
    // In sections also while apps are checked: the rows mustn't move under the pointer between
    // one Ctrl+click and the next.
    if matches!(state.filter.as_str(), "" | "all") && state.query.is_empty() {
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

/// An app with its icon as drawn for the lists (an image of the same pixels each time, so a row
/// that didn't change compares equal and is left alone).
fn choice(state: &mut State, app: &App) -> AppChoice {
    let d = state.data.as_ref();
    let file = d.and_then(|d| d.icons.get(&app.id)).cloned();
    let drawn = file.as_ref().and_then(|f| d?.drawn.get(f)).map(|(_, pixels)| pixels.clone());
    let icon = match (drawn, file) {
        (Some(pixels), _) => Image::from_rgba8(pixels),
        (None, Some(f)) => loaded(state, &f),
        (None, None) => Image::default(),
    };
    app_choice(app, icon)
}

/// The app selected, with its icon from its file, for the details' bigger one.
fn choice_large(state: &mut State, app: &App) -> AppChoice {
    let file = state.data.as_ref().and_then(|d| d.icons.get(&app.id)).cloned();
    let icon = file.map(|f| loaded(state, &f)).unwrap_or_default();
    app_choice(app, icon)
}

/// A picture loaded from its file, once, for the details and the icon picker.
fn loaded(state: &mut State, file: &Path) -> Image {
    let scale = state.data.as_ref().map_or(1.0, |d| d.scale);
    let side = (DETAILS_ICON * scale).round().max(1.0) as u32;
    state.images.entry(format!("icon:{}", file.display())).or_insert_with(|| icon_image(file, side)).clone()
}

fn app_choice(app: &App, icon: Image) -> AppChoice {
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
    match (changed, added) {
        (0, 0) => {}
        (c, 0) => line.push(format!("{c} changed by you")),
        (0, a) => line.push(format!("{a} added by you")),
        (c, a) => line.push(format!("{c} changed and {a} added by you")),
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
        _ if !state.query.is_empty() => "ALL APPS, A–Z".to_owned(),
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
    v.app = choice_large(state, &a);
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
    let labels: Vec<String> = a.categories.iter().map(|c| category_label(c)).collect();
    v.categories = model(labels.iter().map(s).collect());
    v.category_lines = model(
        chip_lines(&labels, CHIP_LINE)
            .into_iter()
            .map(|l| ChipLine { items: model(l.iter().map(s).collect()) })
            .collect(),
    );
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
                    let onoff = |v: String| match v.as_str() {
                        "true" => "On".to_owned(),
                        "" | "false" => "Off".to_owned(),
                        _ => v,
                    };
                    let (old, new) = match k.as_str() {
                        "Actions" => (format!("{old} actions"), format!("{new} actions")),
                        "Terminal" | "NoDisplay" | "PrefersNonDefaultGPU" => (onoff(old), onoff(new)),
                        _ => (old, new),
                    };
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
    let icon = file.map(|p| loaded(state, p)).unwrap_or_default();
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
        let id = new_id(d, &f);
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
            format!("Saved as {own_dir}/{id}")
        });
        v.add_can_save = !f.name.is_empty()
            && if f.website {
                !f.address.is_empty() && !browsers.is_empty()
            } else {
                !f.program.is_empty() && !f.program.contains('%')
            };
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

thread_local! {
    /// The list's sections and each one's rows, kept as models from one render to the next: only
    /// the rows that changed are replaced, instead of every row being built again at each render
    /// (several a second while the page is open).
    static LIST: Rc<VecModel<AppSection>> = Rc::new(VecModel::default());
    static SECTION_ROWS: RefCell<Vec<Rc<VecModel<AppRow>>>> = RefCell::default();
}

pub fn render(window: &MainWindow, state: &mut State) {
    if window.get_page() == "apps" {
        let mut v = view(state);
        let sections: Vec<AppSection> = SECTION_ROWS.with(|kept| {
            let mut kept = kept.borrow_mut();
            v.sections
                .iter()
                .enumerate()
                .map(|(i, mut section)| {
                    if kept.len() <= i {
                        kept.push(Rc::new(VecModel::default()));
                    }
                    section.rows = input::kept(&kept[i], &section.rows);
                    section
                })
                .collect()
        });
        v.sections = LIST.with(|kept| input::kept(kept, &model(sections)));
        window.set_apps(v);
    }
}

/// About how many characters of chips fit on a line of the details at their narrowest.
const CHIP_LINE: usize = 44;

/// Chips in lines of about `width` characters (each chip's padding and × counted as 4), so a long
/// list wraps instead of making the details wider than their card; a longer chip gets a line.
fn chip_lines(labels: &[String], width: usize) -> Vec<Vec<String>> {
    let mut lines: Vec<Vec<String>> = Vec::new();
    let mut used = 0;
    for label in labels {
        let n = label.chars().count() + 4;
        match lines.last_mut() {
            Some(line) if used + n <= width => line.push(label.clone()),
            _ => {
                lines.push(vec![label.clone()]);
                used = 0;
            }
        }
        used += n;
    }
    lines
}

#[cfg(test)]
mod tests {
    #[test]
    fn long_category_lists_wrap() {
        let labels = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            super::chip_lines(&labels(&["Graphics", "3D graphics"]), 44),
            [labels(&["Graphics", "3D graphics"])]
        );
        let gnome =
            labels(&["GNOME", "GTK", "Settings", "Desktop settings", "GNOME settings panel", "Unity settings panel"]);
        assert_eq!(
            super::chip_lines(&gnome, 44),
            [
                labels(&["GNOME", "GTK", "Settings"]),
                labels(&["Desktop settings", "GNOME settings panel"]),
                labels(&["Unity settings panel"])
            ]
        );
        assert!(super::chip_lines(&[], 44).is_empty());
    }

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
        let mut state = State { data: Some(read(dirs, 1.0, &HashMap::new())), ..Default::default() };
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
        let mut state = State { data: Some(read(dirs, 1.0, &HashMap::new())), ..Default::default() };
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
    fn icons_are_drawn_on_the_reading_thread_once() {
        let root = std::env::temp_dir().join(format!("lituus-apps-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let apps = "usr/share/applications";
        write(&root, &format!("{apps}/a.desktop"), "[Desktop Entry]\nType=Application\nName=A\nExec=a\nIcon=a\n");
        write(&root, &format!("{apps}/b.desktop"), "[Desktop Entry]\nType=Application\nName=B\nExec=b\nIcon=b\n");
        write(
            &root,
            "usr/share/icons/hicolor/scalable/apps/a.svg",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="64"><rect width="128" height="64" fill="#e33"/></svg>"##,
        );
        let png = root.join("usr/share/icons/hicolor/256x256/apps/b.png");
        std::fs::create_dir_all(png.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(256, 256, image::Rgba([10, 20, 30, 255])).save(&png).unwrap();
        let dirs = Dirs { data_dirs: vec![root.join("usr/share")], ..Default::default() };
        let d = read(dirs.clone(), 2.0, &HashMap::new());
        let size = |id: &str| {
            let (_, p) = &d.drawn[&d.icons[id]];
            (p.width(), p.height())
        };
        // 36 px at twice the scale: the SVG drawn to fit, the picture made smaller.
        assert_eq!((size("a.desktop"), size("b.desktop")), ((72, 36), (72, 72)));
        assert_eq!(d.drawn[&d.icons["a.desktop"]].1.as_slice()[0], Rgba8Pixel { r: 0xee, g: 0x33, b: 0x33, a: 255 });
        // Read again: the icons drawn before are kept (the same pixels), not drawn again.
        let again = read(dirs, 2.0, &d.drawn);
        let a = &d.icons["a.desktop"];
        assert_eq!(again.drawn[a].1.as_slice().as_ptr(), d.drawn[a].1.as_slice().as_ptr());
        // An image of them is equal to the one before, so the rows are left alone.
        let mut state = State { data: Some(again), ..Default::default() };
        let app = state.data.as_ref().unwrap().apps[0].clone();
        assert_eq!(choice(&mut state, &app), choice(&mut state, &app));
        let _ = std::fs::remove_dir_all(&root);
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

    /// An empty folder for a test.
    fn test_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("lituus-apps-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    /// The page, with the user's apps in `root`/home and the system's in `root`/usr.
    fn state_in(root: &Path) -> State {
        let dirs = Dirs {
            data_home: root.join("home/.local/share"),
            data_dirs: vec![root.join("usr/share")],
            ..Default::default()
        };
        State { data: Some(read(dirs, 1.0, &HashMap::new())), ..Default::default() }
    }

    /// Add an app › a program and its name › Add.
    fn add_program(state: &mut State, program: &Path, name: &str) {
        let weak = slint::Weak::default();
        act(&weak, state, "add-open", "");
        act(&weak, state, "add-program", &program.display().to_string());
        act(&weak, state, "add-name", name);
        act(&weak, state, "add-save", "");
    }

    const CHROMIUM: &str = "[Desktop Entry]\nType=Application\nName=Chromium\nExec=/usr/bin/chromium %U\n";

    #[test]
    fn an_added_program_with_a_space_in_its_path_can_start() {
        let root = test_root("space");
        write(&root, "My Apps/tool.sh", "#!/bin/sh\n");
        let mut state = state_in(&root);
        add_program(&mut state, &root.join("My Apps/tool.sh"), "Tool");
        let text = std::fs::read_to_string(root.join("home/.local/share/applications/tool.desktop")).unwrap();
        assert!(text.contains(&format!("Exec=\"{}\"\n", root.join("My Apps/tool.sh").display())), "{text}");
        let d = state.data.as_ref().unwrap();
        assert!(d.problems.is_empty(), "{:?}", d.problems);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_added_programs_path_is_escaped_as_the_spec_says() {
        let root = test_root("escape");
        write(&root, "Odd Apps/a \"pro\" $tool.sh", "#!/bin/sh\n");
        let mut state = state_in(&root);
        add_program(&mut state, &root.join("Odd Apps/a \"pro\" $tool.sh"), "Odd");
        let text = std::fs::read_to_string(root.join("home/.local/share/applications/odd.desktop")).unwrap();
        // In quotes, " and $ get a backslash, which a desktop file's string writes as two.
        let exec = format!("Exec=\"{}/Odd Apps/a \\\\\"pro\\\\\" \\\\$tool.sh\"\n", root.display());
        assert!(text.contains(&exec), "{text}");
        // A % can't be written in a program's path at all: the dialog says so.
        act(&slint::Weak::default(), &mut state, "add-open", "");
        act(&slint::Weak::default(), &mut state, "add-program", "/opt/50%off/run");
        let v = view(&mut state);
        assert!(!v.add_can_save && v.add_program_note == PERCENT_NOTE);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn using_a_newer_appimage_keeps_the_quotes_right() {
        let root = test_root("newer");
        write(&root, "Apps/Cursor 0.45.AppImage", "");
        let gone = root.join("Apps/Cursor-0.42.AppImage");
        write(
            &root,
            "home/.local/share/applications/cursor.desktop",
            &format!("[Desktop Entry]\nType=Application\nName=Cursor\nExec=\"{}\" %U\n", gone.display()),
        );
        let mut state = state_in(&root);
        let newer = root.join("Apps/Cursor 0.45.AppImage");
        let d = state.data.as_ref().unwrap();
        assert!(
            matches!(&d.problems[..], [(_, Problem::NotFound { newer: Some(n), .. })] if *n == newer),
            "{:?}",
            d.problems
        );
        act(&slint::Weak::default(), &mut state, "fix", &format!("cursor.desktop\u{1f}use:{}", newer.display()));
        let text = std::fs::read_to_string(root.join("home/.local/share/applications/cursor.desktop")).unwrap();
        assert!(text.contains(&format!("Exec=\"{}\" %U\n", newer.display())), "{text}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adding_never_replaces_the_users_file() {
        let root = test_root("clash");
        let own = "home/.local/share/applications/firefox.desktop";
        write(&root, own, "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox --my-profile %u\n");
        write(&root, "bin/ff", "#!/bin/sh\n");
        let mut state = state_in(&root);
        act(&slint::Weak::default(), &mut state, "add-open", "");
        act(&slint::Weak::default(), &mut state, "add-name", "Firefox");
        // The dialog says where it really goes.
        assert_eq!(
            view(&mut state).add_footer,
            format!(
                "Saved as {}",
                lituus_xdg::shorten_home(&root.join("home/.local/share/applications/firefox-2.desktop"))
            )
        );
        add_program(&mut state, &root.join("bin/ff"), "Firefox");
        assert!(std::fs::read_to_string(root.join(own)).unwrap().contains("--my-profile"));
        assert_eq!(state.selected.as_deref(), Some("firefox-2.desktop"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adding_never_takes_over_a_packages_app() {
        let root = test_root("shadow");
        write(
            &root,
            "usr/share/applications/obsidian.desktop",
            "[Desktop Entry]\nType=Application\nName=Obsidian\nExec=/usr/bin/obsidian %U\nMimeType=x-scheme-handler/obsidian;\n",
        );
        write(&root, "Apps/Obsidian.AppImage.sh", "#!/bin/sh\n");
        let mut state = state_in(&root);
        add_program(&mut state, &root.join("Apps/Obsidian.AppImage.sh"), "Obsidian");
        let apps = &state.data.as_ref().unwrap().apps;
        // The package's app is still there, with what it opens; the added one is another app.
        assert!(apps.iter().any(|a| a.opens("x-scheme-handler/obsidian")), "{apps:?}");
        assert_eq!(apps.iter().filter(|a| a.name == "Obsidian").count(), 2);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn two_websites_on_one_host() {
        let root = test_root("host");
        write(&root, "usr/share/applications/chromium.desktop", CHROMIUM);
        let mut state = state_in(&root);
        let weak = slint::Weak::default();
        for (address, name) in
            [("https://mail.google.com/mail/u/0", "Mail"), ("https://mail.google.com/mail/u/1", "Work mail")]
        {
            act(&weak, &mut state, "add-open", "");
            act(&weak, &mut state, "add-mode", "1");
            act(&weak, &mut state, "add-address", address);
            act(&weak, &mut state, "add-name", name);
            act(&weak, &mut state, "add-save", "");
        }
        let apps = &state.data.as_ref().unwrap().apps;
        let mut own: Vec<(&str, &str)> =
            apps.iter().filter(|a| a.own).map(|a| (a.id.as_str(), a.name.as_str())).collect();
        own.sort();
        assert_eq!(
            own,
            [("lituus-web-mail-google-com-2.desktop", "Work mail"), ("lituus-web-mail-google-com.desktop", "Mail")]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_name_without_ascii_letters() {
        let root = test_root("name");
        write(&root, "bin/notes", "#!/bin/sh\n");
        let mut state = state_in(&root);
        add_program(&mut state, &root.join("bin/notes"), "Блокнот");
        assert!(!root.join("home/.local/share/applications/.desktop").exists(), "written as \".desktop\"");
        assert!(state.data.as_ref().unwrap().apps.iter().any(|a| a.name == "Блокнот" && a.id == "lituus-app.desktop"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_percent_in_a_websites_address() {
        let root = test_root("percent");
        write(&root, "usr/share/applications/chromium.desktop", CHROMIUM);
        let mut state = state_in(&root);
        let weak = slint::Weak::default();
        act(&weak, &mut state, "add-open", "");
        act(&weak, &mut state, "add-mode", "1");
        act(
            &weak,
            &mut state,
            "add-address",
            "https://teams.microsoft.com/l/channel/19%3Aabc%40thread.skype/General?q=a&b=c",
        );
        act(&weak, &mut state, "add-save", "");
        let text =
            std::fs::read_to_string(root.join("home/.local/share/applications/lituus-web-teams-microsoft-com.desktop"))
                .unwrap();
        // The spec: a literal % is %%, and an argument with a reserved character (? and &) is quoted.
        assert!(
            text.contains("Exec=/usr/bin/chromium \"--app=https://teams.microsoft.com/l/channel/19%%3Aabc%%40thread.skype/General?q=a&b=c\"\n"),
            "{text}"
        );
        // Open runs the address as it was typed.
        let app = &state.data.as_ref().unwrap().apps.iter().find(|a| a.own).unwrap().clone();
        assert_eq!(
            launch_argv(app, None),
            [
                "/usr/bin/chromium",
                "--app=https://teams.microsoft.com/l/channel/19%3Aabc%40thread.skype/General?q=a&b=c"
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_websites_window_class_has_its_path() {
        // Chromium on X11 names the window from the host and the path, each / as _.
        assert_eq!(wm_class("https://web.whatsapp.com/"), "chrome-web.whatsapp.com__-Default");
        assert_eq!(wm_class("https://mail.google.com/mail/u/0?x=1#inbox"), "chrome-mail.google.com__mail_u_0-Default");
        assert_eq!(wm_class("http://127.0.0.1:9/mail/u/0/"), "chrome-127.0.0.1__mail_u_0-Default");
    }

    #[test]
    fn open_runs_the_commands_own_words() {
        let root = test_root("launch");
        let log = root.join("log");
        // Through a shell, `$tool` in the argument would be expanded, and `my.sh` would run.
        for name in ["my$tool.sh", "my.sh"] {
            write(
                &root,
                &format!("bin/{name}"),
                &format!("printf '%s %s\\n' '{name}' \"$#\" >> '{}'\n", log.display()),
            );
        }
        let script = exec::quote(&root.join("bin/my$tool.sh").display().to_string());
        let app = App { exec: format!("/bin/sh {script} %U"), ..Default::default() };
        launch(&app).unwrap();
        let start = std::time::Instant::now();
        let ran = || std::fs::read_to_string(&log).unwrap_or_default();
        while !ran().ends_with('\n') && start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(ran(), "my$tool.sh 0\n");
        // In a terminal when the file asks for one.
        let terminal = Terminal { name: "kitty".into(), argv: vec!["kitty".into()] };
        let htop = App { exec: "htop %U".into(), terminal: true, ..Default::default() };
        assert_eq!(launch_argv(&htop, Some(&terminal)), ["kitty", "htop"]);
        assert_eq!(launch_argv(&App { terminal: false, ..htop }, Some(&terminal)), ["htop"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn checking_apps_keeps_the_list_as_it_is() {
        let root = test_root("check");
        for (id, name, category) in [("a", "Blender", "Graphics"), ("b", "Vim", "Utility"), ("c", "Files", "System")] {
            write(
                &root,
                &format!("usr/share/applications/{id}.desktop"),
                &format!("[Desktop Entry]\nType=Application\nName={name}\nExec=sh\nCategories={category};\n"),
            );
        }
        let mut state = state_in(&root);
        let weak = slint::Weak::default();
        let shown = |state: &State| -> Vec<(String, Vec<String>)> {
            rows(state).into_iter().map(|(title, apps)| (title, apps.into_iter().map(|a| a.name).collect())).collect()
        };
        let before = shown(&state);
        assert_eq!(
            before.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(),
            ["GRAPHICS AND VIDEO", "ACCESSORIES", "SYSTEM"]
        );
        // A Ctrl+click: the rows stay where they are for the next one.
        act(&weak, &mut state, "select", "a.desktop");
        act(&weak, &mut state, "check", "c.desktop");
        assert_eq!(shown(&state), before);
        assert_eq!(view(&mut state).list_title, "");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_change_undone_leaves_no_copy() {
        let root = test_root("undo");
        write(
            &root,
            "usr/share/applications/obsidian.desktop",
            "[Desktop Entry]\nType=Application\nName=Obsidian\nExec=sh %U\n",
        );
        let mut state = state_in(&root);
        let weak = slint::Weak::default();
        let own = root.join("home/.local/share/applications/obsidian.desktop");
        act(&weak, &mut state, "select", "obsidian.desktop");
        act(&weak, &mut state, "name", "Obsidian"); // Enter, nothing changed
        assert!(!own.exists());
        act(&weak, &mut state, "shown", "");
        assert!(own.exists());
        act(&weak, &mut state, "shown", ""); // hidden, then shown again
        assert!(!own.exists());
        let v = view(&mut state);
        assert!(v.shown && !v.can_restore && v.hero_line == "0 hidden");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rebase_keeps_quick_action_changes_and_new_keys() {
        let root = test_root("rebase");
        write(
            &root,
            "usr/share/applications/app.desktop",
            "[Desktop Entry]\nType=Application\nName=App\nExec=sh %U\nActions=new;\nStartupWMClass=app-new\n\n[Desktop Action new]\nName=New\nExec=sh --new\n",
        );
        write(
            &root,
            "home/.local/share/applications/app.desktop",
            "[Desktop Entry]\nType=Application\nName=My App\nExec=sh %U\nActions=new;\n\n[Desktop Action new]\nName=New\nExec=sh --new --private\n",
        );
        let mut state = state_in(&root);
        act(&slint::Weak::default(), &mut state, "fix", "app.desktop\u{1f}rebase");
        let text = std::fs::read_to_string(root.join("home/.local/share/applications/app.desktop")).unwrap();
        assert!(text.contains("Name=My App"));
        assert!(text.contains("Exec=sh --new --private"), "{text}");
        assert!(text.contains("StartupWMClass=app-new"), "{text}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_read_that_ends_after_a_change_does_not_undo_it() {
        let root = test_root("race");
        write(
            &root,
            "usr/share/applications/obsidian.desktop",
            "[Desktop Entry]\nType=Application\nName=Obsidian\nExec=electron obsidian %U\n",
        );
        let mut state = state_in(&root);
        let dirs = state.data.as_ref().unwrap().dirs.clone();
        let weak = slint::Weak::default();
        // apps::open's thread starts reading now…
        let started = next_generation(&mut state);
        let (stale, older) = (read(dirs.clone(), 1.0, &HashMap::new()), read(dirs, 1.0, &HashMap::new()));
        act(&weak, &mut state, "select", "obsidian.desktop");
        act(&weak, &mut state, "exec", "electron obsidian --my-flag %U");
        let own = root.join("home/.local/share/applications/obsidian.desktop");
        assert!(std::fs::read_to_string(&own).unwrap().contains("--my-flag"));
        // …and its result arrives after the change: the page keeps showing the change.
        landed(&mut state, started, stale);
        assert_eq!(view(&mut state).exec, "electron obsidian --my-flag %U");
        // A switch changes what the file has, even with a list older than the file.
        state.data = Some(older);
        act(&weak, &mut state, "wayland", "");
        let text = std::fs::read_to_string(&own).unwrap();
        assert!(text.contains("Exec=electron --ozone-platform-hint=auto obsidian --my-flag %U"), "{text}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    // An SVG icon that names another file: an `<image>` of it.
    fn icon_naming(href: &Path) -> String {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="8" height="8"><image width="8" height="8" xlink:href="{}"/></svg>"##,
            href.display()
        )
    }

    const RED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#f00"/></svg>"##;

    #[test]
    fn an_svg_icon_does_not_load_files_it_names() {
        let root = test_root("svg");
        write(&root, "elsewhere/red.svg", RED);
        write(&root, "icon.svg", &icon_naming(&root.join("elsewhere/red.svg")));
        // The lists' icons, drawn on the reading thread…
        let pixels = draw_icon(&root.join("icon.svg"), 8).unwrap();
        assert_eq!(pixels.as_slice()[0].a, 0, "the icon drew a file from elsewhere: {:?}", pixels.as_slice()[0]);
        // …and the details', the icon picker's and Default apps', on the window's.
        let pixels = icon_image(&root.join("icon.svg"), 8).to_rgba8().unwrap();
        assert_eq!(pixels.as_slice()[0].a, 0, "the window drew a file from elsewhere: {:?}", pixels.as_slice()[0]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_svg_icon_naming_a_fifo_does_not_hang() {
        let root = test_root("fifo");
        std::fs::create_dir_all(&root).unwrap();
        let fifo = root.join("pipe.png");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        write(&root, "icon.svg", &icon_naming(&fifo));
        let (tx, rx) = std::sync::mpsc::channel();
        let icon = root.join("icon.svg");
        let fifo_icon = fifo.clone();
        let thread = std::thread::spawn(move || {
            let _ = draw_icon(&icon, 8);
            let _ = icon_image(&icon, 8);
            // The FIFO itself as an icon (a theme can have one by any name).
            let _ = icon_image(&fifo_icon, 8);
            let _ = tx.send(());
        });
        let done = rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok();
        if !done {
            // Lets the blocked reader go: a writer that opens and closes gives it the end.
            use std::os::unix::fs::OpenOptionsExt;
            for _ in 0..50 {
                if std::fs::OpenOptions::new().write(true).custom_flags(0o4000).open(&fifo).is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        thread.join().unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(done, "an icon was still reading the FIFO after 5 s");
    }

    #[test]
    fn reading_a_program_that_isnt_an_appimage_does_not_run_it() {
        use std::os::unix::fs::PermissionsExt;
        let root = test_root("appimage");
        let ran = root.join("ran");
        write(&root, "x.AppImage", &format!("#!/bin/sh\ntouch '{}'\n", ran.display()));
        let program = root.join("x.AppImage");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_appimage(&program, &root.join("icons")).is_none());
        // Executable for its owner, not readable by others; and not started.
        assert_eq!(program.metadata().unwrap().permissions().mode() & 0o777, 0o700);
        assert!(!ran.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
