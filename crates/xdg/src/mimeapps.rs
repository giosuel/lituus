//! The XDG MIME Applications spec: which app opens a file type. Apps read every mimeapps.list in
//! this order (per desktop first, in each folder):
//!
//! 1. $XDG_CONFIG_HOME/$desktop-mimeapps.list, mimeapps.list (the user's)
//! 2. $XDG_CONFIG_DIRS/… (the system's)
//! 3. $XDG_DATA_HOME/applications/… (the user's, deprecated)
//! 4. $XDG_DATA_DIRS/applications/… (the distribution's)
//!
//! The first file whose [Default Applications] names an installed app for a type wins. Without
//! one, the type opens in the first app that says it opens it: [Added Associations], then the
//! desktop files' MimeType, minus [Removed Associations]. Lituus reads them the same way, so
//! what it shows is what really opens, and saves to the user's first own file that exists
//! (hyprland-mimeapps.list before mimeapps.list), else ~/.config/mimeapps.list.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::Dirs;
use crate::desktop::App;

/// A mimeapps.list: where it is, whether it's the user's own, and what it says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListFile {
    pub path: PathBuf,
    /// In the user's config or data folder.
    pub own: bool,
    /// The desktop's ("hyprland"), for a $desktop-mimeapps.list.
    pub desktop: String,
    pub exists: bool,
    pub defaults: Vec<(String, Vec<String>)>,
    pub added: Vec<(String, Vec<String>)>,
    pub removed: Vec<(String, Vec<String>)>,
}

impl ListFile {
    fn get<'a>(section: &'a [(String, Vec<String>)], mime: &str) -> &'a [String] {
        section.iter().find(|(m, _)| m == mime).map(|(_, ids)| ids.as_slice()).unwrap_or(&[])
    }

    pub fn default_for(&self, mime: &str) -> &[String] {
        Self::get(&self.defaults, mime)
    }
}

/// Every file the spec names, in its order, read or not there.
pub fn files(dirs: &Dirs) -> Vec<ListFile> {
    let mut out = Vec::new();
    let mut add = |dir: &Path, own: bool| {
        for d in &dirs.desktops {
            out.push(read(dir.join(format!("{d}-mimeapps.list")), own, d));
        }
        out.push(read(dir.join("mimeapps.list"), own, ""));
    };
    add(&dirs.config_home, true);
    for d in &dirs.config_dirs {
        add(d, false);
    }
    add(&dirs.data_home.join("applications"), true);
    for d in &dirs.data_dirs {
        add(&d.join("applications"), false);
    }
    out
}

fn read(path: PathBuf, own: bool, desktop: &str) -> ListFile {
    let text = std::fs::read_to_string(&path);
    let mut f = ListFile { path, own, desktop: desktop.into(), exists: text.is_ok(), ..Default::default() };
    let mut section = "";
    for line in text.unwrap_or_default().lines().map(str::trim) {
        if line.starts_with('[') {
            section = match line {
                "[Default Applications]" => "default",
                "[Added Associations]" => "added",
                "[Removed Associations]" => "removed",
                _ => "",
            };
            continue;
        }
        let Some((mime, ids)) = line.split_once('=') else { continue };
        let ids: Vec<String> = ids.split(';').map(str::trim).filter(|i| !i.is_empty()).map(str::to_owned).collect();
        let list = match section {
            "default" => &mut f.defaults,
            "added" => &mut f.added,
            "removed" => &mut f.removed,
            _ => continue,
        };
        // A type named twice (also in a section written twice): the later line counts, as GLib reads it.
        let mime = mime.trim();
        match list.iter_mut().find(|(m, _)| m == mime) {
            Some(entry) => entry.1 = ids,
            None => list.push((mime.to_owned(), ids)),
        }
    }
    f
}

/// Where Lituus saves: the user's first own file that exists, else ~/.config/mimeapps.list.
pub fn save_target(files: &[ListFile], dirs: &Dirs) -> PathBuf {
    files
        .iter()
        .find(|f| f.own && f.exists && f.path.starts_with(&dirs.config_home))
        .map(|f| f.path.clone())
        .unwrap_or_else(|| dirs.config_home.join("mimeapps.list"))
}

/// What opens a file type, and where that comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// The app's desktop file id; `None` when no app opens it.
    pub app: Option<String>,
    /// The file (an index into `files`) that names it; `None`: the first app that opens it.
    pub from: Option<usize>,
}

/// What really opens `mime`, as apps work it out.
pub fn resolve(files: &[ListFile], apps: &[App], mime: &str) -> Resolved {
    let installed = |id: &str| apps.iter().any(|a| a.id == id);
    for (i, f) in files.iter().enumerate() {
        if let Some(id) = f.default_for(mime).iter().find(|id| installed(id)) {
            return Resolved { app: Some(id.clone()), from: Some(i) };
        }
    }
    Resolved { app: candidates(files, apps, mime).into_iter().next(), from: None }
}

/// The apps that open `mime`, in the order apps would try them: added associations (file by
/// file), then the desktop files that say so; minus the removed ones.
pub fn candidates(files: &[ListFile], apps: &[App], mime: &str) -> Vec<String> {
    let installed = |id: &str| apps.iter().any(|a| a.id == id);
    let mut removed: HashSet<&str> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for f in files {
        for id in ListFile::get(&f.added, mime) {
            if installed(id) && !removed.contains(id.as_str()) && !out.contains(id) {
                out.push(id.clone());
            }
        }
        removed.extend(ListFile::get(&f.removed, mime).iter().map(String::as_str));
    }
    for a in apps {
        if a.opens(mime) && !removed.contains(a.id.as_str()) && !out.contains(&a.id) {
            out.push(a.id.clone());
        }
    }
    out
}

/// Makes `app` the default for each of `mimes` in the file at `path`, keeping the rest of it. A
/// file that can't be read is left as it is: written back, it would lose everything else in it.
pub fn set_defaults(path: &Path, mimes: &[String], app: &str) -> std::io::Result<()> {
    let text = lituus_fs::read_or_empty(path)?;
    let text = with_defaults(&text, mimes, app);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    lituus_fs::write_atomic(path, text)
}

fn with_defaults(text: &str, mimes: &[String], app: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    if !lines.iter().any(|l| l.trim() == "[Default Applications]") {
        if lines.last().is_some_and(|l| !l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push("[Default Applications]".into());
    }
    // A file may have the section twice (scripts that append one); GLib reads them as one.
    let sections = |lines: &[String]| -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if l.trim() == "[Default Applications]" {
                let end =
                    lines[i + 1..].iter().position(|l| l.trim().starts_with('[')).map_or(lines.len(), |p| i + 1 + p);
                out.push((i + 1, end));
            }
        }
        out
    };
    for mime in mimes {
        let line = format!("{mime}={app};");
        let found: Vec<usize> = sections(&lines)
            .into_iter()
            .flat_map(|(start, end)| start..end)
            .filter(|&i| lines[i].split_once('=').is_some_and(|(m, _)| m.trim() == mime))
            .collect();
        match found.split_first() {
            // Its first line is changed, and any later one goes: a later line would win.
            Some((&first, rest)) => {
                lines[first] = line;
                for &i in rest.iter().rev() {
                    lines.remove(i);
                }
            }
            // New: at the end of the first section, before the blank lines that end it.
            None => {
                let (start, end) = sections(&lines)[0];
                let mut at = end;
                while at > start && lines[at - 1].trim().is_empty() {
                    at -= 1;
                }
                lines.insert(at, line);
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{desktop, test_dir};

    fn app(id: &str, mimes: &[&str]) -> App {
        App {
            id: id.into(),
            name: id.into(),
            mime_types: mimes.iter().map(|m| (*m).into()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn the_first_file_that_names_an_installed_app_wins() {
        let root = test_dir::with(
            "mimeapps",
            &[
                (
                    "home/.config/hyprland-mimeapps.list",
                    "[Default Applications]\nimage/png=gone.desktop;imv.desktop;\n",
                ),
                (
                    "home/.config/mimeapps.list",
                    "[Default Applications]\nimage/png=gimp.desktop\ntext/html=firefox.desktop\n",
                ),
                (
                    "usr/share/applications/mimeapps.list",
                    "[Default Applications]\napplication/pdf=papers.desktop\n[Removed Associations]\nimage/gif=gimp.desktop\n",
                ),
            ],
        );
        let dirs = test_dir::dirs(&root);
        let files = files(&dirs);
        let apps = vec![
            app("imv.desktop", &["image/png", "image/gif"]),
            app("gimp.desktop", &["image/png", "image/gif"]),
            app("firefox.desktop", &["text/html", "image/gif"]),
            app("papers.desktop", &["application/pdf"]),
        ];
        let where_ =
            |r: Resolved| (r.app, r.from.map(|i| files[i].path.strip_prefix(&root).unwrap().display().to_string()));
        assert_eq!(
            where_(resolve(&files, &apps, "image/png")),
            (Some("imv.desktop".into()), Some("home/.config/hyprland-mimeapps.list".into()))
        );
        assert_eq!(
            where_(resolve(&files, &apps, "application/pdf")),
            (Some("papers.desktop".into()), Some("usr/share/applications/mimeapps.list".into()))
        );
        // Not set anywhere: the first app that opens it, without the removed one.
        assert_eq!(candidates(&files, &apps, "image/gif"), ["imv.desktop", "firefox.desktop"]);
        assert_eq!(resolve(&files, &apps, "image/gif"), Resolved { app: Some("imv.desktop".into()), from: None });
        assert_eq!(save_target(&files, &dirs), root.join("home/.config/hyprland-mimeapps.list"));
        let _ = desktop::all(&dirs);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn saving_keeps_the_rest() {
        let text = "# mine\n[Added Associations]\ntext/plain=nvim.desktop;\n\n[Default Applications]\nimage/png=gimp.desktop;\n\n[Removed Associations]\n";
        let out = with_defaults(text, &["image/png".into(), "image/jpeg".into()], "imv.desktop");
        assert_eq!(
            out,
            "# mine\n[Added Associations]\ntext/plain=nvim.desktop;\n\n[Default Applications]\nimage/png=imv.desktop;\nimage/jpeg=imv.desktop;\n\n[Removed Associations]\n"
        );
        assert_eq!(
            with_defaults("", &["text/html".into()], "firefox.desktop"),
            "[Default Applications]\ntext/html=firefox.desktop;\n"
        );
        assert_eq!(
            with_defaults("[Added Associations]\na/b=c.desktop;\n", &["text/html".into()], "f.desktop"),
            "[Added Associations]\na/b=c.desktop;\n\n[Default Applications]\ntext/html=f.desktop;\n"
        );
    }

    #[test]
    fn a_second_default_applications_group_is_changed_too() {
        // GLib merges the two, and the later line wins: Lituus reads and writes it the same way.
        let text = "[Default Applications]\ntext/html=firefox.desktop;\n\n[Default Applications]\ntext/html=chromium.desktop;\n";
        let root = test_dir::with("mimeapps-twice", &[("home/.config/mimeapps.list", text)]);
        let f = read(root.join("home/.config/mimeapps.list"), true, "");
        assert_eq!(f.default_for("text/html"), ["chromium.desktop"]);
        let out = with_defaults(text, &["text/html".into()], "brave.desktop");
        assert_eq!(out, "[Default Applications]\ntext/html=brave.desktop;\n\n[Default Applications]\n");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_that_cant_be_read_is_left_as_it_is() {
        let root = test_dir::with("mimeapps-unreadable", &[]);
        let path = root.join("home/.config/mimeapps.list");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Not UTF-8: read as empty, it would be written back with only the new line.
        let text = b"[Default Applications]\ntext/plain=caf\xe9.desktop;\n";
        std::fs::write(&path, text).unwrap();
        assert!(set_defaults(&path, &["text/html".into()], "firefox.desktop").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), text);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
