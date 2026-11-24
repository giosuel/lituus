//! Lituus's own file next to the user's Hyprland config: `lituus.lua`, which hyprland.lua loads
//! with `require("lituus")` (Hyprland puts its config folder on Lua's path), or `lituus.conf`
//! with `source =` before Hyprland 0.55. The user's own config is only read, never written,
//! except by Set up… (D-44): after asking, it adds the load line at the end, with a copy first.
//! Each page that writes to it has a part of its own, from its first line ("-- Written by Lituus
//! (Displays).") to the next page's, and writes only that part.

use std::path::{Path, PathBuf};

/// `$XDG_CONFIG_HOME/hypr`, or `~/.config/hypr`.
pub fn config_dir() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(config) => PathBuf::from(config).join("hypr"),
        None => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config/hypr"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFile {
    pub dir: PathBuf,
    /// Lua (Hyprland 0.55 and later) or hyprlang.
    pub lua: bool,
}

impl ConfigFile {
    pub fn new(dir: impl Into<PathBuf>, lua: bool) -> Self {
        Self { dir: dir.into(), lua }
    }

    /// `lituus.lua` or `lituus.conf`.
    pub fn path(&self) -> PathBuf {
        self.dir.join(if self.lua { "lituus.lua" } else { "lituus.conf" })
    }

    /// The user's main config.
    pub fn main_path(&self) -> PathBuf {
        self.dir.join(if self.lua { "hyprland.lua" } else { "hyprland.conf" })
    }

    /// The line the user's config needs to load Lituus's file.
    pub fn load_line(&self) -> String {
        if self.lua { "require(\"lituus\")".to_owned() } else { "source = ~/.config/hypr/lituus.conf".to_owned() }
    }

    /// The comment and the line Set up… adds at the end of the user's config (D-44).
    pub fn load_lines(&self) -> (String, String) {
        let comment = if self.lua { "--" } else { "#" };
        (format!("{comment} Lituus's settings (Displays, Input, Shortcuts)"), self.load_line())
    }

    /// Where Set up… keeps a copy of the user's config first: hyprland.lua.bak, or .bak.1 and on
    /// when that's taken (the user's own copy, or an earlier one).
    pub fn backup_path(&self) -> PathBuf {
        let name = file_name(&self.main_path());
        (0u32..)
            .map(|n| self.dir.join(if n == 0 { format!("{name}.bak") } else { format!("{name}.bak.{n}") }))
            .find(|p| !p.exists())
            .expect("a free name")
    }

    /// Adds the load line at the end of the user's main config, after copying it to
    /// `backup_path` (D-44: only from the dialog that asks first). It appends to the file where
    /// it is, so a config that's a link into the user's dotfiles stays a link. Returns the copy.
    pub fn add_load_line(&self) -> std::io::Result<PathBuf> {
        use std::io::Write;
        let main = self.main_path();
        let text = std::fs::read_to_string(&main)?;
        let backup = self.backup_path();
        std::fs::copy(&main, &backup)?;
        let (comment, line) = self.load_lines();
        // One empty line before it.
        let gap = match () {
            _ if text.is_empty() || text.ends_with("\n\n") => "",
            _ if text.ends_with('\n') => "\n",
            _ => "\n\n",
        };
        std::fs::OpenOptions::new()
            .append(true)
            .open(&main)?
            .write_all(format!("{gap}{comment}\n{line}\n").as_bytes())?;
        Ok(backup)
    }

    /// Whether the user's config loads Lituus's file: a `require("lituus")` in a Lua file of the
    /// config folder (other than ours), or a `source` of lituus.conf.
    pub fn loaded(&self) -> bool {
        let files: Vec<PathBuf> = if self.lua {
            std::fs::read_dir(&self.dir)
                .map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect::<Vec<PathBuf>>())
                .unwrap_or_default()
                .into_iter()
                .filter(|p: &PathBuf| p.extension().is_some_and(|e| e == "lua") && *p != self.path())
                .collect()
        } else {
            vec![self.main_path()]
        };
        files.iter().any(|f| std::fs::read_to_string(f).is_ok_and(|text| self.loads(&text)))
    }

    fn loads(&self, text: &str) -> bool {
        text.lines().map(str::trim).filter(|l| !l.starts_with("--") && !l.starts_with('#')).any(|line| {
            if self.lua {
                let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
                ["require(\"lituus\")", "require('lituus')", "require\"lituus\"", "require'lituus'"]
                    .iter()
                    .any(|r| compact.contains(r))
            } else {
                line.starts_with("source") && line.ends_with("lituus.conf")
            }
        })
    }

    pub fn read(&self) -> Option<String> {
        std::fs::read_to_string(self.path()).ok()
    }

    /// The part of the file the page `name` wrote ("Displays", "Input").
    pub fn read_section(&self, name: &str) -> Option<String> {
        section(&self.read()?, name)
    }

    /// Writes the page's part of the file, keeping the other pages' parts. A file that's there
    /// but can't be read is an error, not an empty file: the other parts would be lost.
    pub fn write_section(&self, name: &str, part: &str) -> std::io::Result<()> {
        let _writing = WRITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let text = lituus_fs::read_or_empty(&self.path())?;
        self.write_unlocked(&with_section(&text, name, part))
    }

    /// Writes the file whole, through a temporary file, so Hyprland never reads half of it. A
    /// file that's a link into the user's dotfiles stays a link.
    pub fn write(&self, text: &str) -> std::io::Result<()> {
        let _writing = WRITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.write_unlocked(text)
    }

    fn write_unlocked(&self, text: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        lituus_fs::write_atomic(&self.path(), text)
    }
}

/// Held while the file is read, changed and written: Displays (on the window's thread), Input
/// (its worker) and Shortcuts (a thread per change) each write their part, and without it one
/// could write back another's part as it was before that one's change.
static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The file's parts: (page, its lines). Text before the first part has no page.
fn sections(text: &str) -> Vec<(Option<String>, String)> {
    let mut parts: Vec<(Option<String>, String)> = vec![(None, String::new())];
    for line in text.lines() {
        let trimmed = line.trim_start_matches(['-', '#']).trim_start();
        let page = (line.starts_with("--") || line.starts_with('#'))
            .then(|| trimmed.strip_prefix("Written by Lituus (")?.split_once(')').map(|(p, _)| p.to_owned()))
            .flatten();
        if page.is_some() {
            parts.push((page, String::new()));
        }
        let last = &mut parts.last_mut().expect("there's always a part").1;
        last.push_str(line);
        last.push('\n');
    }
    for (_, text) in &mut parts {
        let kept = text.trim_end().len();
        text.truncate(kept);
        if !text.is_empty() {
            text.push('\n');
        }
    }
    parts.retain(|(page, text)| page.is_some() || !text.is_empty());
    parts
}

/// The part the page `name` wrote, if the file has one.
pub fn section(text: &str, name: &str) -> Option<String> {
    sections(text).into_iter().find(|(page, _)| page.as_deref() == Some(name)).map(|(_, text)| text)
}

/// The file with the page's part replaced by `part` (or added at the end), the rest as it was.
pub fn with_section(text: &str, name: &str, part: &str) -> String {
    let mut parts = sections(text);
    let part = format!("{}\n", part.trim_end());
    match parts.iter_mut().find(|(page, _)| page.as_deref() == Some(name)) {
        Some(p) => p.1 = part,
        None => parts.push((Some(name.to_owned()), part)),
    }
    parts.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>().join("\n")
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lituus-hypr-config-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn set_up_adds_the_line_at_the_end_after_a_copy() {
        let d = dir("setup-lua");
        let file = ConfigFile::new(&d, true);
        std::fs::write(file.main_path(), "hl.config({ general = { gaps_in = 4 } })").unwrap();
        assert!(!file.loaded());
        let backup = file.add_load_line().unwrap();
        assert_eq!(backup, d.join("hyprland.lua.bak"));
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "hl.config({ general = { gaps_in = 4 } })");
        assert_eq!(
            std::fs::read_to_string(file.main_path()).unwrap(),
            "hl.config({ general = { gaps_in = 4 } })\n\n-- Lituus's settings (Displays, Input, Shortcuts)\nrequire(\"lituus\")\n"
        );
        assert!(file.loaded());
        // A copy that's already there is kept.
        assert_eq!(file.backup_path(), d.join("hyprland.lua.bak.1"));
    }

    #[test]
    fn set_up_writes_a_source_line_for_hyprlang() {
        let d = dir("setup-conf");
        let file = ConfigFile::new(&d, false);
        std::fs::write(file.main_path(), "monitor = , preferred, auto, 1\n").unwrap();
        file.add_load_line().unwrap();
        assert_eq!(
            std::fs::read_to_string(file.main_path()).unwrap(),
            "monitor = , preferred, auto, 1\n\n# Lituus's settings (Displays, Input, Shortcuts)\nsource = ~/.config/hypr/lituus.conf\n"
        );
        assert!(file.loaded());
    }

    #[test]
    fn set_up_keeps_a_linked_config_a_link() {
        let d = dir("setup-link");
        let dotfiles = dir("setup-link-dotfiles").join("hyprland.lua");
        std::fs::write(&dotfiles, "-- mine\n").unwrap();
        std::os::unix::fs::symlink(&dotfiles, d.join("hyprland.lua")).unwrap();
        let file = ConfigFile::new(&d, true);
        file.add_load_line().unwrap();
        assert!(std::fs::symlink_metadata(file.main_path()).unwrap().file_type().is_symlink());
        assert!(std::fs::read_to_string(&dotfiles).unwrap().ends_with("require(\"lituus\")\n"));
        // No config at all: nothing to add it to.
        assert!(ConfigFile::new(dir("setup-none"), true).add_load_line().is_err());
    }

    #[test]
    fn finds_the_require_in_any_lua_file() {
        let d = dir("lua");
        let f = ConfigFile::new(&d, true);
        std::fs::write(d.join("hyprland.lua"), "-- require(\"lituus\") is commented out\nrequire(\"monitors\")\n")
            .unwrap();
        assert!(!f.loaded());
        std::fs::write(d.join("monitors.lua"), "require ( 'lituus' )\n").unwrap();
        assert!(f.loaded());
        // Our own file doesn't count.
        std::fs::remove_file(d.join("monitors.lua")).unwrap();
        f.write("require(\"lituus\")\n").unwrap();
        assert!(!f.loaded());
        assert_eq!(f.read().as_deref(), Some("require(\"lituus\")\n"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn each_page_writes_its_own_part() {
        let displays = "-- Written by Lituus (Displays). Change it in Lituus.\n\nhl.monitor({ output = \"DP-1\" })\n";
        let input = "-- Written by Lituus (Input). Change it in Lituus.\n\nhl.config({ input = {} })\n";
        // A file from before there were parts is the Displays part.
        assert_eq!(section(displays, "Displays").as_deref(), Some(displays));
        assert_eq!(section(displays, "Input"), None);
        let both = with_section(displays, "Input", input);
        assert_eq!(both, format!("{displays}\n{input}"));
        assert_eq!(section(&both, "Displays").as_deref(), Some(displays));
        assert_eq!(section(&both, "Input").as_deref(), Some(input));
        // Replacing one keeps the other and the order.
        let moved = displays.replace("DP-1", "DP-2");
        let again = with_section(&both, "Displays", &moved);
        assert_eq!(again, format!("{moved}\n{input}"));
        assert_eq!(with_section("", "Input", input), input);
        // hyprlang's comments too.
        let conf = "# Written by Lituus (Displays).\nmonitor = DP-1, preferred, auto, 1\n";
        assert_eq!(
            section(&with_section(conf, "Input", "# Written by Lituus (Input).\ninput {\n}\n"), "Displays").as_deref(),
            Some(conf)
        );
        let d = dir("sections");
        let f = ConfigFile::new(&d, true);
        f.write_section("Input", input).unwrap();
        f.write_section("Displays", displays).unwrap();
        assert_eq!(f.read_section("Input").as_deref(), Some(input));
        assert_eq!(f.read().unwrap(), format!("{input}\n{displays}"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn lituus_file_thats_a_link_stays_a_link() {
        let d = dir("own-link");
        let dotfiles = dir("own-link-dotfiles").join("lituus.lua");
        std::fs::write(&dotfiles, "").unwrap();
        std::os::unix::fs::symlink(&dotfiles, d.join("lituus.lua")).unwrap();
        let f = ConfigFile::new(&d, true);
        f.write_section("Input", "-- Written by Lituus (Input).\n").unwrap();
        assert!(std::fs::symlink_metadata(f.path()).unwrap().file_type().is_symlink());
        assert!(std::fs::read_to_string(&dotfiles).unwrap().contains("Input"));
        std::fs::remove_dir_all(&d).unwrap();
        std::fs::remove_dir_all(dotfiles.parent().unwrap()).unwrap();
    }

    /// A file that can't be read isn't taken as empty: writing one part would drop the others.
    #[test]
    fn a_file_that_cant_be_read_isnt_written_over() {
        let d = dir("own-unreadable");
        let f = ConfigFile::new(&d, true);
        let text = b"-- Written by Lituus (Displays).\n-- caf\xe9\nhl.monitor({ output = \"DP-1\" })\n";
        std::fs::write(f.path(), text).unwrap();
        assert!(f.write_section("Input", "-- Written by Lituus (Input).\n").is_err());
        assert_eq!(std::fs::read(f.path()).unwrap(), text);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Displays, Input and Shortcuts write their parts from their own threads: none is lost.
    #[test]
    fn pages_writing_at_once_keep_each_others_parts() {
        let d = dir("own-threads");
        let f = ConfigFile::new(&d, true);
        let threads: Vec<_> = ["Displays", "Input", "Shortcuts"]
            .into_iter()
            .map(|page| {
                let f = f.clone();
                std::thread::spawn(move || {
                    for i in 0..100 {
                        f.write_section(page, &format!("-- Written by Lituus ({page}).\n-- {i}\n")).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        for page in ["Displays", "Input", "Shortcuts"] {
            assert_eq!(f.read_section(page), Some(format!("-- Written by Lituus ({page}).\n-- 99\n")));
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn finds_the_source_line() {
        let d = dir("conf");
        let f = ConfigFile::new(&d, false);
        std::fs::write(d.join("hyprland.conf"), "monitor = , preferred, auto, 1\n").unwrap();
        assert!(!f.loaded());
        std::fs::write(d.join("hyprland.conf"), "source = ~/.config/hypr/lituus.conf\n").unwrap();
        assert!(f.loaded());
        assert_eq!(f.path(), d.join("lituus.conf"));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
