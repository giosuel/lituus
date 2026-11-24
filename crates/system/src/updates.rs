//! Package updates, counted without a password: pacman through checkupdates (a copy of the
//! database), the AUR through yay or paru, DNF, zypper, APT (as of root's package lists), XBPS,
//! Flatpak and Snap. A package manager that isn't installed isn't listed. Installing them is the
//! terminal's job: each has a command (`default_command`, which the user can change) that the
//! System page's Update buttons run there.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Manager {
    Pacman,
    Aur,
    Dnf,
    Zypper,
    Apt,
    Xbps,
    Flatpak,
    Snap,
}

impl Manager {
    pub const ALL: [Manager; 8] = [
        Manager::Pacman,
        Manager::Aur,
        Manager::Dnf,
        Manager::Zypper,
        Manager::Apt,
        Manager::Xbps,
        Manager::Flatpak,
        Manager::Snap,
    ];

    /// Its key in Lituus's config (`[updates]`).
    pub fn key(self) -> &'static str {
        match self {
            Manager::Pacman => "pacman",
            Manager::Aur => "aur",
            Manager::Dnf => "dnf",
            Manager::Zypper => "zypper",
            Manager::Apt => "apt",
            Manager::Xbps => "xbps",
            Manager::Flatpak => "flatpak",
            Manager::Snap => "snap",
        }
    }

    pub fn from_key(key: &str) -> Option<Manager> {
        Manager::ALL.into_iter().find(|m| m.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Manager::Pacman => "pacman",
            Manager::Aur => "AUR",
            Manager::Dnf => "DNF",
            Manager::Zypper => "zypper",
            Manager::Apt => "APT",
            Manager::Xbps => "XBPS",
            Manager::Flatpak => "Flatpak",
            Manager::Snap => "Snap",
        }
    }
}

/// A package manager on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub manager: Manager,
    /// The program that counts and installs: "pacman", "yay", "paru", "dnf", …
    pub program: String,
    /// Where it looks, in words: "core, extra, multilib", "Through yay", "Flathub · 6 apps".
    pub source: String,
    /// What its Update button runs unless the user changed it.
    pub default_command: String,
}

/// Where to look: the root folder (`/` but for tests), the home folder, and PATH.
#[derive(Clone, Debug)]
pub struct Env {
    pub root: PathBuf,
    pub home: PathBuf,
    pub path: String,
}

impl Env {
    pub fn from_env() -> Env {
        Env {
            root: PathBuf::from("/"),
            home: std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default(),
            path: std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
        }
    }

    /// A program on PATH (below the root for tests).
    pub fn which(&self, program: &str) -> Option<PathBuf> {
        self.path.split(':').filter(|d| !d.is_empty()).find_map(|dir| {
            let p = self.root.join(dir.trim_start_matches('/')).join(program);
            is_executable(&p).then_some(p)
        })
    }

    fn file(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The package managers installed here, in `Manager::ALL`'s order. `os_id` is os-release's ID
/// (openSUSE Tumbleweed updates with `zypper dup`).
pub fn found(env: &Env, os_id: &str) -> Vec<Found> {
    let mut out = Vec::new();
    let mut add = |manager: Manager, program: &str, source: String, command: &str| {
        out.push(Found { manager, program: program.into(), source, default_command: command.into() })
    };
    if env.which("pacman").is_some() {
        let repos = ini_sections(&std::fs::read_to_string(env.file("etc/pacman.conf")).unwrap_or_default())
            .into_iter()
            .filter(|(name, _)| name != "options")
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        add(Manager::Pacman, "pacman", list_or(&repos, "Repositories in /etc/pacman.conf"), "sudo pacman -Syu");
        if let Some(helper) = ["paru", "yay"].into_iter().find(|h| env.which(h).is_some()) {
            add(Manager::Aur, helper, format!("Through {helper}"), &format!("{helper} -Sua"));
        }
    }
    if env.which("dnf").is_some() {
        add(
            Manager::Dnf,
            "dnf",
            list_or(&enabled_repos(&env.file("etc/yum.repos.d")), "Fedora's repositories"),
            "sudo dnf upgrade",
        );
    }
    if env.which("zypper").is_some() {
        let command = if os_id.contains("tumbleweed") { "sudo zypper dup" } else { "sudo zypper update" };
        add(
            Manager::Zypper,
            "zypper",
            list_or(&enabled_repos(&env.file("etc/zypp/repos.d")), "openSUSE's repositories"),
            command,
        );
    }
    if env.which("apt").is_some() {
        let age = env.file("var/lib/apt/lists").metadata().and_then(|m| m.modified()).ok();
        add(Manager::Apt, "apt", apt_source(age, SystemTime::now()), "sudo apt update && sudo apt upgrade");
    }
    if env.which("xbps-install").is_some() {
        add(Manager::Xbps, "xbps-install", "Void's repositories".into(), "sudo xbps-install -Su");
    }
    if env.which("flatpak").is_some() {
        add(Manager::Flatpak, "flatpak", flatpak_source(env), "flatpak update");
    }
    if env.which("snap").is_some() {
        add(Manager::Snap, "snap", "Snap Store".into(), "sudo snap refresh");
    }
    out
}

fn list_or(items: &[String], none: &str) -> String {
    if items.is_empty() { none.to_owned() } else { items.join(", ") }
}

/// An INI file's sections and their `key=value` lines.
fn ini_sections(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            out.push((name.trim().to_owned(), Vec::new()));
        } else if let (Some((k, v)), Some(last)) = (line.split_once('='), out.last_mut()) {
            last.1.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    out
}

/// The enabled repositories in a folder of `.repo` files (DNF's and zypper's).
fn enabled_repos(dir: &Path) -> Vec<String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "repo")).collect())
        .unwrap_or_default();
    files.sort();
    let mut out = Vec::new();
    for file in files {
        for (name, keys) in ini_sections(&std::fs::read_to_string(&file).unwrap_or_default()) {
            let enabled = keys.iter().find(|(k, _)| k == "enabled").is_none_or(|(_, v)| v == "1");
            if enabled {
                out.push(name);
            }
        }
    }
    out
}

/// APT only reads the lists root last downloaded: say how old they are.
fn apt_source(lists: Option<SystemTime>, now: SystemTime) -> String {
    let Some(days) = lists.and_then(|t| now.duration_since(t).ok()).map(|d| d.as_secs() / 86400) else {
        return "As of the package lists root last downloaded".into();
    };
    match days {
        0 => "As of today's package lists".into(),
        1 => "As of the package lists from yesterday".into(),
        n => format!("As of the package lists from {n} days ago"),
    }
}

/// "Flathub · 6 apps": the remotes (system and the user's) and how many apps are installed.
fn flatpak_source(env: &Env) -> String {
    let mut remotes = Vec::new();
    let mut apps = 0;
    for base in [env.file("var/lib/flatpak"), env.home.join(".local/share/flatpak")] {
        for (name, _) in ini_sections(&std::fs::read_to_string(base.join("repo/config")).unwrap_or_default()) {
            if let Some(remote) = name.strip_prefix("remote \"").and_then(|n| n.strip_suffix('"')) {
                let remote = capital(remote);
                if !remotes.contains(&remote) {
                    remotes.push(remote);
                }
            }
        }
        apps += std::fs::read_dir(base.join("app")).map(|d| d.flatten().count()).unwrap_or(0);
    }
    let apps = match apps {
        0 => String::new(),
        1 => "1 app".into(),
        n => format!("{n} apps"),
    };
    [remotes.join(", "), apps].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

// ---- counting ----------------------------------------------------------------------------------

/// How many updates a package manager has, or why it couldn't tell.
pub fn count(env: &Env, found: &Found) -> Result<u32, String> {
    match found.manager {
        Manager::Pacman => {
            if env.which("checkupdates").is_some() {
                // 0: updates; 2: none; 1: it failed.
                let out = run("checkupdates", &[])?;
                match out.code {
                    0 | 2 => Ok(lines(&out.stdout)),
                    _ => Err(reason(&out)),
                }
            } else {
                // Without pacman-contrib: as of the last sync.
                query_count(&run("pacman", &["-Qu"])?)
            }
        }
        Manager::Aur => query_count(&run(&found.program, &["-Qua"])?),
        Manager::Dnf => {
            // 100: there are updates; 0: none.
            let out = run("dnf", &["check-update", "-q"])?;
            match out.code {
                0 => Ok(0),
                // At least one, though none could be read.
                100 => Ok(dnf_count(&out.stdout).max(1)),
                _ => Err(reason(&out)),
            }
        }
        Manager::Zypper => {
            let out = run("zypper", &["--quiet", "--non-interactive", "--no-refresh", "list-updates"])?;
            if out.code == 0 { Ok(zypper_count(&out.stdout)) } else { Err(reason(&out)) }
        }
        Manager::Apt => {
            let out = run("apt", &["list", "--upgradable"])?;
            if out.code == 0 {
                Ok(out.stdout.lines().filter(|l| l.contains("[upgradable from")).count() as u32)
            } else {
                Err(reason(&out))
            }
        }
        Manager::Xbps => {
            let out = run("xbps-install", &["-Mun"])?;
            if out.code == 0 { Ok(lines(&out.stdout)) } else { Err(reason(&out)) }
        }
        Manager::Flatpak => {
            let out = run("flatpak", &["remote-ls", "--updates", "--columns=application"])?;
            if out.code == 0 { Ok(lines(&out.stdout)) } else { Err(reason(&out)) }
        }
        Manager::Snap => {
            let out = run("snap", &["refresh", "--list"])?;
            if out.code == 0 { Ok(snap_count(&out.stdout)) } else { Err(reason(&out)) }
        }
    }
}

fn lines(text: &str) -> u32 {
    text.lines().filter(|l| !l.trim().is_empty()).count() as u32
}

/// `pacman -Qu` and `yay -Qua` / `paru -Qua`: a line per update. With nothing to update they end
/// with 1 and say nothing, as when they failed, but then they say why on stderr ("error: …",
/// "-> Error during AUR search: …" when the AUR can't be reached). Warnings alone aren't a failure.
fn query_count(out: &Output) -> Result<u32, String> {
    let failed = out.stderr.lines().any(|l| {
        let l = l.to_lowercase();
        l.contains("error") || l.contains("fail") || l.contains("unable")
    });
    if out.code == 0 || (out.stdout.trim().is_empty() && !failed) { Ok(lines(&out.stdout)) } else { Err(reason(out)) }
}

/// `dnf check-update`: a line per package ("name.arch  version  repo"), then perhaps
/// "Obsoleting Packages" and what they replace, which aren't more updates. dnf4 puts the version
/// and repo of a name too long for its column on the next line.
fn dnf_count(text: &str) -> u32 {
    let mut count = 0;
    let mut name_alone = false;
    for line in text.lines().take_while(|l| !l.starts_with("Obsoleting")) {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words[..] {
            [name, _, _] if name.contains('.') => count += 1,
            [_, _] if name_alone => count += 1,
            _ => {}
        }
        name_alone = matches!(words[..], [name] if name.contains('.'));
    }
    count
}

/// `zypper list-updates`: a table whose rows start with "v |".
fn zypper_count(text: &str) -> u32 {
    text.lines().filter(|l| l.trim_start().starts_with("v ")).count() as u32
}

/// `snap refresh --list`: a header line, then a line per snap; nothing when all are up to date.
fn snap_count(text: &str) -> u32 {
    lines(text).saturating_sub(1)
}

struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

/// What went wrong, in a line: the last thing it said on stderr.
fn reason(out: &Output) -> String {
    out.stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.trim_start_matches("==> ERROR:").trim_start_matches("error:").trim().to_owned())
        .unwrap_or_else(|| format!("It stopped with code {}", out.code))
}

/// Runs a program in the C locale, giving it two minutes.
fn run(program: &str, args: &[&str]) -> Result<Output, String> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't start {program}: {e}"))?;
    // Read both pipes on threads, so a full one doesn't stop it.
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{program} took longer than two minutes"));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    Ok(Output {
        code: status.code().unwrap_or(-1),
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str, programs: &[&str], files: &[(&str, &str)]) -> Env {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("lituus-updates-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for p in programs {
            let f = root.join("usr/bin").join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(&f, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        for (path, text) in files {
            let f = root.join(path);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        Env { home: root.join("home"), root, path: "/usr/bin".into() }
    }

    #[test]
    fn arch_with_paru_and_flatpak() {
        let e = env(
            "arch",
            &["pacman", "checkupdates", "paru", "yay", "flatpak"],
            &[
                (
                    "etc/pacman.conf",
                    "[options]\nHoldPkg = pacman\n#[testing]\n[core]\nInclude = x\n[extra]\n[endeavouros]\n",
                ),
                (
                    "var/lib/flatpak/repo/config",
                    "[core]\nmode=bare\n[remote \"flathub\"]\nurl=https://dl.flathub.org/repo/\n",
                ),
                ("var/lib/flatpak/app/org.gimp.GIMP/x", ""),
                ("home/.local/share/flatpak/repo/config", "[remote \"flathub\"]\n"),
                ("home/.local/share/flatpak/app/com.spotify.Client/x", ""),
            ],
        );
        let f = found(&e, "endeavouros");
        let rows: Vec<(Manager, &str, &str, &str)> =
            f.iter().map(|f| (f.manager, f.program.as_str(), f.source.as_str(), f.default_command.as_str())).collect();
        assert_eq!(
            rows,
            [
                (Manager::Pacman, "pacman", "core, extra, endeavouros", "sudo pacman -Syu"),
                (Manager::Aur, "paru", "Through paru", "paru -Sua"),
                (Manager::Flatpak, "flatpak", "Flathub · 2 apps", "flatpak update"),
            ]
        );
        std::fs::remove_dir_all(&e.root).unwrap();
    }

    #[test]
    fn fedora_and_tumbleweed() {
        let e = env(
            "fedora",
            &["dnf", "zypper"],
            &[
                ("etc/yum.repos.d/fedora.repo", "[fedora]\nenabled=1\n[fedora-debuginfo]\nenabled=0\n"),
                ("etc/yum.repos.d/rpmfusion-free.repo", "[rpmfusion-free]\nenabled=1\n"),
                ("etc/zypp/repos.d/oss.repo", "[repo-oss]\nenabled=1\n"),
            ],
        );
        let f = found(&e, "opensuse-tumbleweed");
        assert_eq!(f[0].source, "fedora, rpmfusion-free");
        assert_eq!((f[1].source.as_str(), f[1].default_command.as_str()), ("repo-oss", "sudo zypper dup"));
        std::fs::remove_dir_all(&e.root).unwrap();
    }

    #[test]
    fn nothing_installed_nothing_listed() {
        let e = env("none", &[], &[("etc/pacman.conf", "[core]\n")]);
        assert!(found(&e, "arch").is_empty());
        std::fs::remove_dir_all(&e.root).unwrap();
    }

    #[test]
    fn reading_their_output() {
        let dnf = "\nfirefox.x86_64   146.0-1.fc43   updates\nkernel.x86_64  6.17.8-300.fc43  updates\n\
                   Obsoleting Packages\nfoo.noarch  1-1  updates\n    bar.noarch  0-1  @System\n";
        assert_eq!(dnf_count(dnf), 2);
        // dnf4, with a name too long for its column.
        let dnf = "\nfirefox.x86_64   146.0-1.fc43   updates\ngnome-shell-extension-appindicator.noarch\n\
                   \x20                                         58-1.fc39   updates\n";
        assert_eq!(dnf_count(dnf), 2);
        let zypper = "S | Repository | Name | Current Version | Available Version | Arch\n\
                      --+------------+------+-----------------+-------------------+-------\n\
                      v | repo-oss   | curl | 8.10            | 8.11              | x86_64\n\
                      v | repo-oss   | vim  | 9.1             | 9.2               | x86_64\n";
        assert_eq!(zypper_count(zypper), 2);
        assert_eq!(
            snap_count("Name  Version  Rev  Size  Publisher  Notes\nfirefox  147.0  7000  300MB  mozilla  -\n"),
            1
        );
        assert_eq!(snap_count(""), 0);
        let day = Duration::from_secs(86400);
        let now = SystemTime::UNIX_EPOCH + day * 100;
        assert_eq!(apt_source(Some(now - day * 5), now), "As of the package lists from 5 days ago");
        assert_eq!(apt_source(Some(now - day), now), "As of the package lists from yesterday");
        let out = Output { code: 1, stdout: String::new(), stderr: "==> ERROR: Cannot fetch updates\n".into() };
        assert_eq!(reason(&out), "Cannot fetch updates");
    }

    /// yay and paru end with 1 both when nothing is to be updated and when the AUR can't be
    /// reached.
    #[test]
    fn an_aur_that_cant_be_reached_isnt_up_to_date() {
        let out = |code, stdout: &str, stderr: &str| Output { code, stdout: stdout.into(), stderr: stderr.into() };
        assert_eq!(query_count(&out(1, "", "")), Ok(0));
        assert_eq!(query_count(&out(0, "foo 1-1 -> 1-2\nbar 2-1 -> 3-1\n", "")), Ok(2));
        assert_eq!(
            query_count(&out(1, "", "error: dial tcp: lookup aur.archlinux.org: no such host\n")),
            Err("dial tcp: lookup aur.archlinux.org: no such host".into())
        );
        assert!(
            query_count(&out(1, "", " -> Error during AUR search: Get \"https://aur.archlinux.org/rpc\"\n")).is_err()
        );
        // A warning isn't a failure.
        assert_eq!(query_count(&out(1, "", "warning: foo: ignoring package upgrade (1-1 => 1-2)\n")), Ok(0));
    }
}
