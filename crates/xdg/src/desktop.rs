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
    let mut kind = String::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
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
            _ => {}
        }
    }
    (kind == "Application" && !app.name.is_empty()).then_some(app)
}

/// Every app, the user's first; an id is read from the first folder that has it.
pub fn all(dirs: &Dirs) -> Vec<App> {
    let mut out: Vec<App> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in dirs.applications() {
        let mut files = Vec::new();
        walk(&dir, &dir, &mut files);
        files.sort();
        for (id, path) in files {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(app) = std::fs::read_to_string(&path).ok().and_then(|t| parse(&id, &path, &t)) {
                out.push(app);
            }
        }
    }
    out
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
