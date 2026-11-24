//! The default terminal: the one xdg-terminal-exec opens (its choice is in xdg-terminals.list),
//! or, without xdg-terminal-exec, $TERMINAL or the first well-known terminal installed. The
//! System page's Update buttons run their commands in it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crate::updates::Env;

/// How to open a command in a terminal: `argv` and then the command's own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terminal {
    /// Its name, to show ("kitty"); empty when xdg-terminal-exec picks one on its own.
    pub name: String,
    pub argv: Vec<String>,
}

/// Terminals Lituus knows how to start a command in, in the order it tries them.
const KNOWN: [(&str, &[&str]); 10] = [
    ("kitty", &["kitty"]),
    ("foot", &["foot"]),
    ("alacritty", &["alacritty", "-e"]),
    ("ghostty", &["ghostty", "-e"]),
    ("wezterm", &["wezterm", "start", "--"]),
    ("konsole", &["konsole", "-e"]),
    ("gnome-terminal", &["gnome-terminal", "--wait", "--"]),
    ("ptyxis", &["ptyxis", "-x"]),
    ("xfce4-terminal", &["xfce4-terminal", "-x"]),
    ("xterm", &["xterm", "-e"]),
];

/// The XDG folders, as the spec reads them.
#[derive(Clone, Debug)]
pub struct Dirs {
    pub config_home: PathBuf,
    pub config_dirs: Vec<PathBuf>,
    pub data_home: PathBuf,
    pub data_dirs: Vec<PathBuf>,
    /// $XDG_CURRENT_DESKTOP's names, lower case ("hyprland").
    pub desktops: Vec<String>,
}

impl Dirs {
    pub fn from_env(home: &Path) -> Dirs {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let list = |name: &str, default: &str| -> Vec<PathBuf> {
            var(name)
                .unwrap_or_else(|| default.into())
                .split(':')
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
                .collect()
        };
        Dirs {
            config_home: var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")),
            config_dirs: list("XDG_CONFIG_DIRS", "/etc/xdg"),
            data_home: var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")),
            data_dirs: list("XDG_DATA_DIRS", "/usr/local/share:/usr/share"),
            desktops: var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .split(':')
                .filter(|d| !d.is_empty())
                .map(str::to_lowercase)
                .collect(),
        }
    }

    /// The xdg-terminals.list files, first one first: per desktop, then the plain one, in the
    /// config folders, then in the data folders' xdg-terminal-exec.
    pub fn terminal_lists(&self) -> Vec<PathBuf> {
        let mut names: Vec<String> = self.desktops.iter().map(|d| format!("{d}-xdg-terminals.list")).collect();
        names.push("xdg-terminals.list".into());
        let mut out = Vec::new();
        for dir in std::iter::once(&self.config_home).chain(&self.config_dirs) {
            out.extend(names.iter().map(|n| dir.join(n)));
        }
        for dir in std::iter::once(&self.data_home).chain(&self.data_dirs) {
            out.extend(names.iter().map(|n| dir.join("xdg-terminal-exec").join(n)));
        }
        out
    }

    /// The folders with desktop files, first one first.
    pub fn applications(&self) -> Vec<PathBuf> {
        std::iter::once(&self.data_home).chain(&self.data_dirs).map(|d| d.join("applications")).collect()
    }
}

/// The first desktop file id a terminal list names ("kitty.desktop"), skipping removed ones
/// ("-foot.desktop") and actions ("kitty.desktop:new-window" counts as kitty.desktop).
pub fn listed_terminal(dirs: &Dirs) -> Option<String> {
    for file in dirs.terminal_lists() {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') || line.starts_with('-') || line.starts_with('/') {
                continue;
            }
            let id = line.split(':').next().unwrap_or(line);
            if id.ends_with(".desktop") {
                return Some(id.to_owned());
            }
        }
    }
    None
}

/// The list Lituus saves the terminal to: the user's per-desktop one if there is one (it's read
/// first), else ~/.config/xdg-terminals.list.
pub fn list_to_save(dirs: &Dirs) -> PathBuf {
    dirs.desktops
        .iter()
        .map(|d| dirs.config_home.join(format!("{d}-xdg-terminals.list")))
        .find(|p| p.is_file())
        .unwrap_or_else(|| dirs.config_home.join("xdg-terminals.list"))
}

/// Makes `id` the first terminal in the list at `path`, keeping the others after it.
pub fn set_terminal(path: &Path, id: &str) -> std::io::Result<()> {
    // A list that can't be read isn't written over.
    let text = lituus_fs::read_or_empty(path)?;
    let rest = text.lines().filter(|l| l.trim().trim_start_matches('-').split(':').next() != Some(id));
    let out: Vec<&str> = std::iter::once(id).chain(rest).collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    lituus_fs::write_atomic(path, out.join("\n") + "\n")
}

/// A desktop file's Name, found by its id in the application folders.
pub fn desktop_name(dirs: &Dirs, id: &str) -> Option<String> {
    dirs.applications().into_iter().find_map(|dir| {
        let text = std::fs::read_to_string(dir.join(id)).ok()?;
        let mut in_entry = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
            } else if in_entry && let Some(name) = line.strip_prefix("Name=") {
                return Some(name.trim().to_owned());
            }
        }
        None
    })
}

/// The terminal to run a command in, if there is one.
pub fn default_terminal(env: &Env, dirs: &Dirs, terminal_var: Option<&str>) -> Option<Terminal> {
    if env.which("xdg-terminal-exec").is_some() {
        let name = listed_terminal(dirs)
            .map(|id| desktop_name(dirs, &id).unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned()))
            .unwrap_or_default();
        return Some(Terminal { name, argv: vec!["xdg-terminal-exec".into()] });
    }
    if let Some(t) = terminal_var.filter(|t| !t.is_empty())
        && env.which(t).is_some()
    {
        let argv = KNOWN.iter().find(|(n, _)| *n == t).map(|(_, a)| a.iter().map(|s| s.to_string()).collect());
        return Some(Terminal { name: t.into(), argv: argv.unwrap_or_else(|| vec![t.into(), "-e".into()]) });
    }
    // The list names one (for xdg-terminal-exec, which isn't here): use it if Lituus knows it.
    let listed = listed_terminal(dirs).unwrap_or_default().to_lowercase();
    KNOWN
        .iter()
        .filter(|(n, _)| env.which(n).is_some())
        .find(|(n, _)| !listed.is_empty() && listed.contains(n))
        .or_else(|| KNOWN.iter().find(|(n, _)| env.which(n).is_some()))
        .map(|(n, a)| Terminal { name: (*n).into(), argv: a.iter().map(|s| s.to_string()).collect() })
}

/// Commands to run one after another in one terminal, each with a heading, and a line at the
/// end that keeps the window open until Enter: a shell script for `sh -c`.
pub fn script(commands: &[(String, String)]) -> String {
    let mut s = String::from("failed=0\n");
    for (label, command) in commands {
        s += &format!(
            "printf '\\033[1m:: %s\\033[0m\\n' {}\n{{\n{command}\n}} || failed=1\n",
            quote(&format!("Updating {label}"))
        );
    }
    s += "if [ $failed = 0 ]; then printf '\\n%s' 'Done. Press Enter to close this window.'\n";
    s += "else printf '\\n%s' 'Something went wrong: see above. Press Enter to close this window.'; fi\nread _\n";
    s
}

/// [`script`], telling when it ends: it writes its process id to `marker` (made empty first, by
/// [`new_marker`]) and removes the file at its end. Some terminals hand the command to a window
/// that's open already and leave at once (xfce4-terminal, Ptyxis, a running wezterm), so the end
/// of the terminal's process isn't the end of the update: [`wait_for`] waits for the script.
pub fn marked_script(commands: &[(String, String)], marker: &Path) -> String {
    format!(
        "lituus_marker={}\necho $$ > \"$lituus_marker\"\ntrap 'rm -f \"$lituus_marker\"' EXIT\n{}",
        quote(&marker.to_string_lossy()),
        script(commands)
    )
}

/// A new, empty marker file in `dir`, for [`marked_script`].
pub fn new_marker(dir: &Path) -> std::io::Result<PathBuf> {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let path = dir.join(format!("lituus-update-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
    // Never one that's there already (another user's link, in /tmp).
    std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
    Ok(path)
}

/// Waits for the script of `marker` (see [`marked_script`]) to end, then removes the marker: its
/// last line removed it, or the process it names is gone (its window was closed under it). A
/// script that hasn't started within `start` never will.
pub fn wait_for(marker: &Path, start: Duration) {
    let since = Instant::now();
    loop {
        match std::fs::read_to_string(marker) {
            Ok(pid) if pid.trim().is_empty() => {
                if since.elapsed() >= start {
                    break;
                }
            }
            Ok(pid) => match pid.trim().parse::<u32>() {
                Ok(pid) if Path::new("/proc").join(pid.to_string()).exists() => {}
                _ => break,
            },
            Err(_) => break,
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = std::fs::remove_file(marker);
}

/// A word for the shell, in single quotes.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(root: &Path) -> Dirs {
        Dirs {
            config_home: root.join("home/.config"),
            config_dirs: vec![root.join("etc/xdg")],
            data_home: root.join("home/.local/share"),
            data_dirs: vec![root.join("usr/share")],
            desktops: vec!["hyprland".into()],
        }
    }

    fn write(root: &Path, path: &str, text: &str) {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn the_list_names_the_terminal() {
        let root = std::env::temp_dir().join(format!("lituus-terminal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write(&root, "etc/xdg/xdg-terminals.list", "foot.desktop\n");
        write(&root, "home/.config/hyprland-xdg-terminals.list", "# mine\n-foot.desktop\nkitty.desktop:new-window\n");
        write(
            &root,
            "usr/share/applications/kitty.desktop",
            "[Desktop Entry]\nName=kitty\n[Desktop Action new]\nName=New\n",
        );
        let d = dirs(&root);
        assert_eq!(listed_terminal(&d).as_deref(), Some("kitty.desktop"));
        assert_eq!(desktop_name(&d, "kitty.desktop").as_deref(), Some("kitty"));
        assert_eq!(list_to_save(&d), root.join("home/.config/hyprland-xdg-terminals.list"));
        set_terminal(&list_to_save(&d), "foot.desktop").unwrap();
        assert_eq!(listed_terminal(&d).as_deref(), Some("foot.desktop"));
        assert_eq!(
            std::fs::read_to_string(list_to_save(&d)).unwrap(),
            "foot.desktop\n# mine\nkitty.desktop:new-window\n"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_script_for_the_shell() {
        let s = script(&[("pacman".into(), "sudo pacman -Syu".into()), ("AUR".into(), "yay -Sua".into())]);
        assert!(
            s.starts_with(
                "failed=0\nprintf '\\033[1m:: %s\\033[0m\\n' 'Updating pacman'\n{\nsudo pacman -Syu\n} || failed=1\n"
            ),
            "{s}"
        );
        // It really runs: one fails, the other still runs, and it says so.
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(script(&[("a".into(), "false && echo no".into()), ("b".into(), "echo ran".into())]))
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&out.stdout);
        assert!(out.contains("ran") && !out.contains("no\n") && out.contains("Something went wrong"), "{out}");
        assert!(s.ends_with("read _\n"));
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

    /// A terminal that hands the command to a window that's open already, and leaves at once:
    /// the update ends with the script, not with the terminal's process.
    #[test]
    fn the_update_ends_with_its_script() {
        let dir = std::env::temp_dir().join(format!("lituus-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let done = dir.join("done");
        let marker = new_marker(&dir).unwrap();
        let command = format!("sleep 0.6; touch {}", quote(&done.to_string_lossy()));
        let script = marked_script(&[("test".into(), command)], &marker);
        let start = Instant::now();
        // The script on its own, in the background (with nothing to read, it doesn't wait for
        // Enter); the "terminal" leaves at once.
        let status = std::process::Command::new("sh")
            .args(["-c", "sh -c \"$0\" </dev/null >/dev/null 2>&1 &", script.as_str()])
            .status()
            .unwrap();
        assert!(status.success());
        wait_for(&marker, Duration::from_secs(5));
        assert!(done.exists(), "waited {:?}, but the update hadn't ended", start.elapsed());
        assert!(!marker.exists());

        // A script that's gone without its last line (its window closed), and one that never
        // started.
        let gone = new_marker(&dir).unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        std::fs::write(&gone, pid.to_string()).unwrap();
        let start = Instant::now();
        wait_for(&gone, Duration::from_secs(5));
        assert!(start.elapsed() < Duration::from_secs(2) && !gone.exists());
        let never = new_marker(&dir).unwrap();
        wait_for(&never, Duration::from_millis(300));
        assert!(!never.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
