//! Changing an app: Lituus never edits a system file. A change is written to the user's own copy
//! (~/.local/share/applications/<id>, which apps read first), made from the package's file the
//! first time; the rest of the file (other keys, translations, comments) stays as it is.
//! Restoring deletes the copy. Hiding is `NoDisplay=true` in the copy.

use std::path::{Path, PathBuf};

use crate::{Dirs, exec};

/// A desktop file's lines, edited key by key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DesktopFile {
    lines: Vec<String>,
}

pub const ENTRY: &str = "Desktop Entry";

impl DesktopFile {
    pub fn parse(text: &str) -> DesktopFile {
        DesktopFile { lines: text.lines().map(str::to_owned).collect() }
    }

    pub fn text(&self) -> String {
        let mut t = self.lines.join("\n");
        t.push('\n');
        t
    }

    /// The lines of a group: (first line after its header, end).
    fn group(&self, group: &str) -> Option<(usize, usize)> {
        let header = format!("[{group}]");
        let start = self.lines.iter().position(|l| l.trim() == header)? + 1;
        let end =
            self.lines[start..].iter().position(|l| l.trim().starts_with('[')).map_or(self.lines.len(), |p| start + p);
        Some((start, end))
    }

    pub fn get(&self, group: &str, key: &str) -> Option<String> {
        let (start, end) = self.group(group)?;
        self.lines[start..end].iter().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == key).then(|| v.trim().to_owned())
        })
    }

    /// Sets a key (`None` removes it); a new key goes after the group's last key.
    pub fn set(&mut self, group: &str, key: &str, value: Option<&str>) {
        let (start, end) = match self.group(group) {
            Some(g) => g,
            None => {
                if value.is_none() {
                    return;
                }
                if self.lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    self.lines.push(String::new());
                }
                self.lines.push(format!("[{group}]"));
                (self.lines.len(), self.lines.len())
            }
        };
        let at = (start..end).find(|&i| self.lines[i].split_once('=').is_some_and(|(k, _)| k.trim() == key));
        match (at, value) {
            (Some(i), Some(v)) => self.lines[i] = format!("{key}={v}"),
            (Some(i), None) => {
                self.lines.remove(i);
            }
            (None, Some(v)) => {
                let mut last = end;
                while last > start && self.lines[last - 1].trim().is_empty() {
                    last -= 1;
                }
                self.lines.insert(last, format!("{key}={v}"));
            }
            (None, None) => {}
        }
    }

    pub fn set_bool(&mut self, key: &str, on: bool) {
        self.set(ENTRY, key, on.then_some("true"));
    }
}

/// "a;b;" as desktop files write lists.
pub fn list(items: &[String]) -> String {
    let mut s = items.iter().map(|i| i.trim()).filter(|i| !i.is_empty()).collect::<Vec<_>>().join(";");
    if !s.is_empty() {
        s.push(';');
    }
    s
}

/// The user's copy of an app's file.
pub fn own_path(dirs: &Dirs, id: &str) -> PathBuf {
    dirs.data_home.join("applications").join(id)
}

/// Changes an app (by id; `current` is the file it's read from now), in the user's own copy.
pub fn change(dirs: &Dirs, id: &str, current: &Path, edit: impl FnOnce(&mut DesktopFile)) -> std::io::Result<PathBuf> {
    let own = own_path(dirs, id);
    let text = std::fs::read_to_string(if own.is_file() { &own } else { current })?;
    let mut file = DesktopFile::parse(&text);
    edit(&mut file);
    if let Some(dir) = own.parent() {
        std::fs::create_dir_all(dir)?;
    }
    lituus_fs::write_atomic(&own, file.text())?;
    Ok(own)
}

/// Back to the package's file: the user's copy goes (only when there is a package's file).
pub fn restore(dirs: &Dirs, id: &str) -> std::io::Result<()> {
    if crate::desktop::package_file(dirs, id).is_none() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "there's no package's file to go back to"));
    }
    std::fs::remove_file(own_path(dirs, id))
}

/// What a copy changes from the package's file: (key, the package's value, the copy's), for the
/// keys the page shows, quick actions counted as one.
pub fn differences(package: &str, own: &str) -> Vec<(String, String, String)> {
    let (p, o) = (DesktopFile::parse(package), DesktopFile::parse(own));
    let mut out = Vec::new();
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
        let (a, b) = (p.get(ENTRY, key).unwrap_or_default(), o.get(ENTRY, key).unwrap_or_default());
        if a != b {
            out.push((key.to_owned(), a, b));
        }
    }
    let actions = |f: &DesktopFile| -> Vec<String> {
        f.lines.iter().filter(|l| l.trim().starts_with("[Desktop Action")).map(|l| l.trim().to_owned()).collect()
    };
    let execs = |f: &DesktopFile| -> Vec<Option<String>> {
        actions(f).iter().map(|h| f.get(h.trim_start_matches('[').trim_end_matches(']'), "Exec")).collect()
    };
    if actions(&p) != actions(&o) || execs(&p) != execs(&o) {
        out.push(("Actions".to_owned(), actions(&p).len().to_string(), actions(&o).len().to_string()));
    }
    out
}

/// Where the program is among a command's words: the first, or after `env` and its settings.
fn program_at(words: &[exec::Word]) -> Option<usize> {
    let first = words.first()?;
    if first.text == "env" || first.text.ends_with("/env") {
        return words.iter().skip(1).position(|w| !w.text.contains('=') && !w.text.starts_with('-')).map(|p| p + 1);
    }
    Some(0)
}

/// The program a command runs: its first word, after `env`, unquoted as the spec says.
pub fn program(exec: &str) -> String {
    let words = exec::words(exec);
    program_at(&words).map(|i| words[i].text.clone()).unwrap_or_default()
}

/// The command with `program` in its program's place, quoted as Exec needs it.
pub fn with_program(exec: &str, program: &str) -> String {
    let words = exec::words(exec);
    match program_at(&words) {
        Some(i) => {
            let at = &words[i].span;
            format!("{}{}{}", &exec[..at.start], exec::quote(program), &exec[at.end..])
        }
        None => exec::quote(program),
    }
}

/// A program by name on PATH, or its path.
pub fn find_program(program: &str, path: &str) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    if program.contains('/') {
        let p = PathBuf::from(program);
        return p.is_file().then_some(p);
    }
    path.split(':').map(|d| Path::new(d).join(program)).find(|p| p.is_file())
}

/// An Electron app: the command names electron, its program is a script that runs electron, or
/// Chromium's files lie beside it.
pub fn is_electron(exec: &str, path: &str) -> bool {
    use std::io::Read;
    if exec.contains("electron") {
        return true;
    }
    let Some(p) = find_program(&program(exec), path) else { return false };
    let real = p.canonicalize().unwrap_or(p);
    // Only a script's first lines matter; programs are often hundreds of MB (Chromium, AppImages).
    let mut start = Vec::new();
    if std::fs::File::open(&real).and_then(|f| f.take(4096).read_to_end(&mut start)).is_ok()
        && start.starts_with(b"#!")
        && String::from_utf8_lossy(&start).contains("electron")
    {
        return true;
    }
    real.parent().is_some_and(|d| d.join("chrome_100_percent.pak").is_file() || d.join("resources/app.asar").is_file())
}

pub const WAYLAND_FLAG: &str = "--ozone-platform-hint=auto";

/// The command with Electron's Wayland flag after its program, or without it. The rest is kept
/// as written, quotes and spaces in them included.
pub fn with_wayland(exec: &str, on: bool) -> String {
    let words = exec::words(exec);
    let program = if on { program_at(&words) } else { None };
    let mut out = String::with_capacity(exec.len() + WAYLAND_FLAG.len() + 1);
    let mut copied = 0;
    for (i, w) in words.iter().enumerate() {
        if w.text == WAYLAND_FLAG {
            // The flag goes, with the space before it.
            copied = w.span.end;
            continue;
        }
        out.push_str(&exec[copied..w.span.end]);
        copied = w.span.end;
        if Some(i) == program {
            out.push(' ');
            out.push_str(WAYLAND_FLAG);
        }
    }
    out.push_str(&exec[copied..]);
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir;

    const FIREFOX: &str = "[Desktop Entry]\nType=Application\nName=Firefox\nName[de]=Firefox\nExec=firefox %u\nIcon=firefox\nActions=new;\n\n[Desktop Action new]\nName=New Window\nExec=firefox --new-window %u\n";

    #[test]
    fn keys_change_and_the_rest_stays() {
        let mut f = DesktopFile::parse(FIREFOX);
        f.set(ENTRY, "Exec", Some("firefox --private %u"));
        f.set(ENTRY, "NoDisplay", Some("true"));
        f.set(ENTRY, "Icon", None);
        f.set("Desktop Action new", "Exec", Some("firefox -P %u"));
        assert_eq!(
            f.text(),
            "[Desktop Entry]\nType=Application\nName=Firefox\nName[de]=Firefox\nExec=firefox --private %u\nActions=new;\nNoDisplay=true\n\n[Desktop Action new]\nName=New Window\nExec=firefox -P %u\n"
        );
        assert_eq!(f.get(ENTRY, "Name[de]").as_deref(), Some("Firefox"));
        let d = differences(FIREFOX, &f.text());
        let keys: Vec<&str> = d.iter().map(|(k, _, _)| k.as_str()).collect();
        assert_eq!(keys, ["Icon", "Exec", "NoDisplay", "Actions"]);
        assert_eq!(list(&["Office".into(), "Development".into()]), "Office;Development;");
    }

    #[test]
    fn a_copy_and_back() {
        let root = test_dir::with("edit", &[("usr/share/applications/firefox.desktop", FIREFOX)]);
        let dirs = test_dir::dirs(&root);
        let system = root.join("usr/share/applications/firefox.desktop");
        let own = change(&dirs, "firefox.desktop", &system, |f| f.set_bool("NoDisplay", true)).unwrap();
        assert_eq!(own, root.join("home/.local/share/applications/firefox.desktop"));
        assert_eq!(std::fs::read_to_string(&system).unwrap(), FIREFOX);
        let apps = crate::desktop::all(&dirs);
        assert!(apps[0].own && apps[0].no_display && apps[0].package == Some(system.clone()));
        assert_eq!(apps[0].actions, [("new".into(), "New Window".into(), "firefox --new-window %u".into())]);
        restore(&dirs, "firefox.desktop").unwrap();
        assert!(!own.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn electron_and_wayland() {
        assert_eq!(program("env GDK_BACKEND=x11 obsidian %U"), "obsidian");
        assert!(is_electron("electron37 /usr/lib/obsidian/app.asar %U", ""));
        assert!(!is_electron("firefox %u", ""));
        assert_eq!(with_wayland("obsidian %U", true), "obsidian --ozone-platform-hint=auto %U");
        assert_eq!(with_wayland("env A=1 obsidian %U", true), "env A=1 obsidian --ozone-platform-hint=auto %U");
        assert_eq!(with_wayland("obsidian --ozone-platform-hint=auto %U", false), "obsidian %U");
    }

    #[test]
    fn a_quoted_program_with_a_space() {
        assert_eq!(program("\"/home/u/My Apps/Tool.AppImage\" %U"), "/home/u/My Apps/Tool.AppImage");
        assert_eq!(program("env WINEPREFIX=\"/home/u/My Games/pfx\" wine start.exe"), "wine");
        assert_eq!(
            with_wayland("\"/opt/Electron Fiddle/electron-fiddle\" %U", true),
            "\"/opt/Electron Fiddle/electron-fiddle\" --ozone-platform-hint=auto %U"
        );
        // Two spaces inside a quoted argument are part of it.
        assert_eq!(with_wayland("app --title \"a  b\" %U", true), "app --ozone-platform-hint=auto --title \"a  b\" %U");
        // Another program in its place, quoted when its path needs it.
        assert_eq!(
            with_program("\"/Apps/Cursor-0.42.AppImage\" --no-sandbox %U", "/Apps/Cursor 0.45.AppImage"),
            "\"/Apps/Cursor 0.45.AppImage\" --no-sandbox %U"
        );
        assert_eq!(with_program("env A=1 /old/app %U", "/new/app"), "env A=1 /new/app %U");
    }

    #[test]
    fn the_electron_check_reads_only_the_start_of_a_program() {
        use std::os::unix::fs::PermissionsExt;
        let d = test_dir::with("electron-read", &[]);
        std::fs::create_dir_all(&d).unwrap();
        let bin = d.join("bigapp");
        // Sparse: 64 MiB of zeros that take no disk space.
        std::fs::File::create(&bin).unwrap().set_len(64 << 20).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let rchar = || {
            std::fs::read_to_string("/proc/self/io")
                .unwrap()
                .lines()
                .find_map(|l| l.strip_prefix("rchar: ")?.trim().parse::<u64>().ok())
                .unwrap()
        };
        let before = rchar();
        assert!(!is_electron("bigapp %U", d.to_str().unwrap()));
        // The process's own count: other tests read too, so the limit is generous.
        let read = rchar() - before;
        std::fs::remove_dir_all(&d).unwrap();
        assert!(read < 1 << 20, "is_electron read {read} bytes of a 64 MiB program to look at its first line");
    }
}
