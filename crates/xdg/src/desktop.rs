//! Desktop files: the apps. Read from the application folders, the user's first; a file id
//! ("org.gnome.Nautilus.desktop", "kde-dolphin.desktop" for kde/dolphin.desktop) found in an
//! earlier folder hides the same id in later ones, as the spec says.

use std::path::{Path, PathBuf};

use crate::Dirs;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct App {
    /// The desktop file id, "firefox.desktop".
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub generic_name: String,
    pub comment: String,
    pub icon: String,
    pub exec: String,
    pub terminal: bool,
    /// Not shown in menus (but it can still open files).
    pub no_display: bool,
    pub mime_types: Vec<String>,
    pub categories: Vec<String>,
    /// A Flatpak's app id (X-Flatpak), "app.zen_browser.zen".
    pub flatpak: String,
    pub keywords: Vec<String>,
    /// StartupWMClass: what Hyprland's window rules match.
    pub wm_class: String,
    /// PrefersNonDefaultGPU.
    pub gpu: bool,
    /// Quick actions: (id, name, command).
    pub actions: Vec<(String, String, String)>,
    /// Read from the user's folder (~/.local/share/applications).
    pub own: bool,
    /// The package's file with the same id, if there is one (`own` is then a changed copy).
    pub package: Option<PathBuf>,
}

impl App {
    pub fn opens(&self, mime: &str) -> bool {
        self.mime_types.iter().any(|m| m == mime)
    }
}

/// A desktop file's [Desktop Entry] (an application's; links and folders aren't apps). Hidden
/// ones are deleted ones: `None`.
pub fn parse(id: &str, path: &Path, text: &str) -> Option<App> {
    let mut app = App { id: id.into(), path: path.into(), ..Default::default() };
    let mut in_entry = false;
    let mut action: Option<String> = None;
    let mut action_ids: Vec<String> = Vec::new();
    let mut found: Vec<(String, String, String)> = Vec::new();
    let mut kind = String::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            action = line.strip_prefix("[Desktop Action ").and_then(|l| l.strip_suffix(']')).map(str::to_owned);
            if let Some(a) = &action {
                found.push((a.clone(), String::new(), String::new()));
            }
            continue;
        }
        if action.is_some()
            && let Some((key, value)) = line.split_once('=')
            && let Some(last) = found.last_mut()
        {
            match key.trim() {
                "Name" => last.1 = value.trim().to_owned(),
                "Exec" => last.2 = value.trim().to_owned(),
                _ => {}
            }
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        let list = || value.split(';').map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned).collect::<Vec<_>>();
        match key {
            "Type" => kind = value.into(),
            "Name" => app.name = value.into(),
            "GenericName" => app.generic_name = value.into(),
            "Comment" => app.comment = value.into(),
            "Icon" => app.icon = value.into(),
            "Exec" => app.exec = value.into(),
            "Terminal" => app.terminal = value == "true",
            "NoDisplay" => app.no_display = value == "true",
            "Hidden" if value == "true" => return None,
            "MimeType" => app.mime_types = list(),
            "Categories" => app.categories = list(),
            "X-Flatpak" => app.flatpak = value.into(),
            "Keywords" => app.keywords = list(),
            "StartupWMClass" => app.wm_class = value.into(),
            "PrefersNonDefaultGPU" => app.gpu = value == "true",
            "Actions" => action_ids = list(),
            _ => {}
        }
    }
    // In the order Actions lists them.
    app.actions = action_ids.iter().filter_map(|id| found.iter().find(|(a, _, _)| a == id).cloned()).collect();
    (kind == "Application" && !app.name.is_empty()).then_some(app)
}

/// Every app, the user's first; an id is read from the first folder that has it.
pub fn all(dirs: &Dirs) -> Vec<App> {
    let mut out: Vec<App> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let own_dir = dirs.data_home.join("applications");
    for dir in dirs.applications() {
        let mut files = Vec::new();
        walk(&dir, &dir, &mut files);
        files.sort();
        for (id, path) in files {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(mut app) = std::fs::read_to_string(&path).ok().and_then(|t| parse(&id, &path, &t)) {
                app.own = dir == own_dir;
                if app.own {
                    app.package = package_file(dirs, &id);
                }
                out.push(app);
            }
        }
    }
    out
}

/// The package's (the system's) file for an id, in the data folders after the user's.
pub fn package_file(dirs: &Dirs, id: &str) -> Option<PathBuf> {
    dirs.data_dirs.iter().map(|d| d.join("applications")).find_map(|dir| {
        let direct = dir.join(id);
        if direct.is_file() {
            return Some(direct);
        }
        // kde-dolphin.desktop is kde/dolphin.desktop.
        let mut files = Vec::new();
        walk(&dir, &dir, &mut files);
        files.into_iter().find(|(i, _)| i == id).map(|(_, p)| p)
    })
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            walk(base, &path, out);
        } else if path.extension().is_some_and(|x| x == "desktop")
            && let Ok(rel) = path.strip_prefix(base)
        {
            out.push((rel.to_string_lossy().replace('/', "-"), path.clone()));
        }
    }
}

/// An app's icon as a file: an absolute path, or the name looked up in the hicolor theme (and
/// the current icon theme `theme`, first) and /usr/share/pixmaps. Largest raster first, then SVG.
pub fn icon_file(dirs: &Dirs, theme: &str, icon: &str) -> Option<PathBuf> {
    if icon.is_empty() {
        return None;
    }
    let p = Path::new(icon);
    if p.is_absolute() {
        return p.is_file().then(|| p.to_owned());
    }
    let sizes = ["scalable", "256x256", "128x128", "96x96", "64x64", "48x48", "32x32"];
    let themes: Vec<&str> = [theme, "hicolor"].into_iter().filter(|t| !t.is_empty()).collect();
    for base in dirs.data().map(|d| d.join("icons")) {
        for t in &themes {
            for size in sizes {
                for ext in ["svg", "png"] {
                    let f = base.join(t).join(size).join("apps").join(format!("{icon}.{ext}"));
                    if f.is_file() {
                        return Some(f);
                    }
                }
            }
        }
    }
    dirs.data()
        .flat_map(|d| ["svg", "png"].map(|ext| d.join("pixmaps").join(format!("{icon}.{ext}"))))
        .find(|f| f.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir;

    #[test]
    fn the_users_copy_wins() {
        let root = test_dir::with(
            "desktop",
            &[
                (
                    "usr/share/applications/firefox.desktop",
                    "[Desktop Entry]\nType=Application\nName=Firefox\nIcon=firefox\nMimeType=text/html;x-scheme-handler/http;\n[Desktop Action new]\nName=New window\n",
                ),
                (
                    "home/.local/share/applications/firefox.desktop",
                    "[Desktop Entry]\nType=Application\nName=My Firefox\nMimeType=text/html;\n",
                ),
                (
                    "usr/share/applications/kde/dolphin.desktop",
                    "[Desktop Entry]\nType=Application\nName=Dolphin\nMimeType=inode/directory;\n",
                ),
                ("usr/share/applications/gone.desktop", "[Desktop Entry]\nType=Application\nName=Gone\nHidden=true\n"),
                ("usr/share/applications/link.desktop", "[Desktop Entry]\nType=Link\nName=Site\n"),
                ("usr/share/icons/hicolor/48x48/apps/firefox.png", "png"),
                ("usr/share/icons/hicolor/scalable/apps/firefox.svg", "<svg/>"),
            ],
        );
        let dirs = test_dir::dirs(&root);
        let apps = all(&dirs);
        let names: Vec<(&str, &str)> = apps.iter().map(|a| (a.id.as_str(), a.name.as_str())).collect();
        assert_eq!(names, [("firefox.desktop", "My Firefox"), ("kde-dolphin.desktop", "Dolphin")]);
        assert!(apps[1].opens("inode/directory"));
        assert_eq!(
            icon_file(&dirs, "", "firefox"),
            Some(root.join("usr/share/icons/hicolor/scalable/apps/firefox.svg"))
        );
        assert_eq!(icon_file(&dirs, "", "nothing"), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
