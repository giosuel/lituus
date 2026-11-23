//! What opens what, as apps see it: desktop files (the apps), the XDG MIME Applications spec's
//! mimeapps.list files (which app opens a file type: read in the spec's order, written to the
//! user's first own file), shared-mime-info's file types (names and extensions), and the kinds
//! the Default apps page groups them in. Every path comes from `Dirs`, so tests use a folder.

use std::path::PathBuf;

pub mod desktop;
pub mod edit;
pub mod kinds;
pub mod mime;
pub mod mimeapps;

/// The XDG base folders, and the desktop's names ($XDG_CURRENT_DESKTOP, lower case).
#[derive(Clone, Debug, Default)]
pub struct Dirs {
    pub config_home: PathBuf,
    pub config_dirs: Vec<PathBuf>,
    pub data_home: PathBuf,
    pub data_dirs: Vec<PathBuf>,
    pub desktops: Vec<String>,
}

impl Dirs {
    pub fn from_env() -> Dirs {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let home = var("HOME").map(PathBuf::from).unwrap_or_default();
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

    /// The data folders, the user's first.
    pub fn data(&self) -> impl Iterator<Item = &PathBuf> {
        std::iter::once(&self.data_home).chain(&self.data_dirs)
    }

    /// The folders with desktop files, the user's first.
    pub fn applications(&self) -> Vec<PathBuf> {
        self.data().map(|d| d.join("applications")).collect()
    }
}

/// "/home/me/.config/x" → "~/.config/x".
pub fn shorten_home(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && text.starts_with(&format!("{home}/")) => format!("~{}", &text[home.len()..]),
        _ => text,
    }
}

#[cfg(test)]
pub(crate) mod test_dir {
    use std::path::{Path, PathBuf};

    /// A folder for a test, with files in it.
    pub fn with(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lituus-xdg-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (path, text) in files {
            write(&d, path, text);
        }
        d
    }

    pub fn write(root: &Path, path: &str, text: &str) {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    pub fn dirs(root: &Path) -> super::Dirs {
        super::Dirs {
            config_home: root.join("home/.config"),
            config_dirs: vec![root.join("etc/xdg")],
            data_home: root.join("home/.local/share"),
            data_dirs: vec![root.join("usr/share")],
            desktops: vec!["hyprland".into()],
        }
    }
}
