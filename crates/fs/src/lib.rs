//! Writing the user's files safely: config.toml, displays.toml, lituus.lua, mimeapps.list and the
//! rest. `std::fs::write` empties a file before it writes the new text, so a full disk, a kill or a
//! power cut in between leaves it empty or cut short. `write_atomic` writes a temporary file next
//! to it and renames that over it: the file is the old one or the new one, never half of either.
//! And `read_or_empty` keeps "there's no file yet" apart from "the file couldn't be read", so a
//! caller that changes a file and writes it back never takes an unreadable one for an empty one.

use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A file's text; a file that isn't there is empty. Any other error (permissions, not UTF-8) is
/// returned: a caller that writes the file back must stop there, not write over what it couldn't
/// read.
pub fn read_or_empty(path: &Path) -> io::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(String::new()),
        // read_to_string's own message doesn't say which file.
        Err(e) if e.kind() == ErrorKind::InvalidData => {
            Err(io::Error::new(ErrorKind::InvalidData, format!("{} isn't UTF-8 text", path.display())))
        }
        Err(e) => Err(e),
    }
}

/// Replaces the file at `path` with `contents`, whole or not at all. The file keeps its mode; a
/// new one gets the usual 0666 less the umask. The folder must exist.
///
/// - A link (into the user's dotfiles: stow, chezmoi, home-manager) stays a link: its target is
///   replaced. A target in a folder that can't be written (the Nix store) is an error, and the
///   link stays as it is.
/// - A file that can't be written (`chmod 444`) is an error, as `std::fs::write` would give: a
///   rename would otherwise go past it.
/// - The temporary file is in the same folder (a rename is atomic only within one file system),
///   under a name of its own for each write, so writers on other threads never share one. It is
///   removed again on any error.
/// - The text is on the disk before the rename, and the rename before this returns (fsync of
///   the file, then of the folder): otherwise a power cut can leave the new name on an empty file.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    write(path, contents.as_ref(), None)
}

/// `write_atomic` for a file only the user may read (private keys, certificates): mode 0600,
/// whatever the file had before. Its folder is made too, only the user's (0700), if it isn't there.
pub fn write_private(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    write(path, contents.as_ref(), Some(0o600))
}

fn write(path: &Path, contents: &[u8], mode: Option<u32>) -> io::Result<()> {
    let target = resolve(path)?;
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, format!("{} isn't a file name", path.display())))?;
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_owned(),
        _ => PathBuf::from("."),
    };
    let old_mode = match std::fs::metadata(&target) {
        Ok(meta) => {
            if meta.is_file() {
                // Fails as std::fs::write would on a file the user made read-only; it changes nothing.
                OpenOptions::new().write(true).open(&target)?;
            }
            Some(meta.permissions().mode() & 0o7777)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let (tmp, mut file) = temp_file(&dir, &name.to_string_lossy(), mode.is_some())?;
    let written = (|| {
        // Only when it differs: usually it doesn't (0644, or 0600 for a private one), and the
        // document portal's files (an export from the Flatpak) may not take a chmod.
        if let Some(mode) = mode.or(old_mode)
            && file.metadata()?.permissions().mode() & 0o7777 != mode
        {
            file.set_permissions(Permissions::from_mode(mode))?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &target)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // The file is in place already; a file system that can't sync a folder (some FUSE ones)
    // isn't a reason to report a failure.
    if let Ok(d) = File::open(&dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// The file a path names, following links (relative ones from the link's folder). A path that
/// isn't there, or a link to nothing yet, names itself or the link's target.
fn resolve(path: &Path) -> io::Result<PathBuf> {
    let mut path = path.to_owned();
    // As the kernel's limit on links in a row (ELOOP).
    for _ in 0..40 {
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let link = std::fs::read_link(&path)?;
                path = path.parent().map_or_else(|| link.clone(), |dir| dir.join(&link));
            }
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(path),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(ErrorKind::InvalidInput, format!("{}: too many links in a row", path.display())))
}

/// A new, empty temporary file in `dir`: `.<name>.<pid>-<n>.tmp`, made only if no file has that
/// name. A private one is 0600 from the start.
fn temp_file(dir: &Path, name: &str, private: bool) -> io::Result<(PathBuf, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut last = None;
    for _ in 0..100 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".{name}.{}-{n}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).mode(if private { 0o600 } else { 0o666 }).open(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            // Left over from an earlier process with the same pid.
            Err(e) if e.kind() == ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::from(ErrorKind::AlreadyExists)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lituus-fs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// What's in the folder besides `keep`: temporary files left behind.
    fn others(d: &Path, keep: &[&str]) -> Vec<String> {
        std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !keep.contains(&n.as_str()))
            .collect()
    }

    #[test]
    fn writes_a_new_file_and_replaces_an_old_one() {
        let d = dir("plain");
        let path = d.join("config.toml");
        write_atomic(&path, "a = 1\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 1\n");
        write_atomic(&path, "b = 2\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "b = 2\n");
        assert_eq!(others(&d, &["config.toml"]), Vec::<String>::new());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_link_stays_a_link() {
        let d = dir("link");
        let dotfiles = dir("link-dotfiles");
        std::fs::write(dotfiles.join("lituus.lua"), "old\n").unwrap();
        std::os::unix::fs::symlink(dotfiles.join("lituus.lua"), d.join("lituus.lua")).unwrap();
        // And a relative link, to that link.
        std::os::unix::fs::symlink("lituus.lua", d.join("again.lua")).unwrap();
        write_atomic(&d.join("again.lua"), "new\n").unwrap();
        assert!(std::fs::symlink_metadata(d.join("lituus.lua")).unwrap().file_type().is_symlink());
        assert!(std::fs::symlink_metadata(d.join("again.lua")).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(dotfiles.join("lituus.lua")).unwrap(), "new\n");
        // The temporary file was next to the target, and is gone.
        assert_eq!(others(&dotfiles, &["lituus.lua"]), Vec::<String>::new());
        // A link to a file that isn't there yet makes it there.
        std::os::unix::fs::symlink(dotfiles.join("new.toml"), d.join("new.toml")).unwrap();
        write_atomic(&d.join("new.toml"), "x = 1\n").unwrap();
        assert!(std::fs::symlink_metadata(d.join("new.toml")).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(dotfiles.join("new.toml")).unwrap(), "x = 1\n");
        std::fs::remove_dir_all(&d).unwrap();
        std::fs::remove_dir_all(&dotfiles).unwrap();
    }

    #[test]
    fn the_file_keeps_its_mode() {
        let d = dir("mode");
        let path = d.join("config.toml");
        std::fs::write(&path, "old\n").unwrap();
        std::fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
        write_atomic(&path, "new\n").unwrap();
        assert_eq!(mode(&path), 0o600);
        std::fs::set_permissions(&path, Permissions::from_mode(0o640)).unwrap();
        write_atomic(&path, "newer\n").unwrap();
        assert_eq!(mode(&path), 0o640);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_private_file_is_private_even_when_it_was_there() {
        let d = dir("private");
        let path = d.join("wg0.conf");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        write_private(&path, "PrivateKey = x").unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "PrivateKey = x");
        // A new folder for it is the user's only.
        let key = d.join("certificates/office/ca.pem");
        write_private(&key, "key").unwrap();
        assert_eq!((mode(&key), mode(&d.join("certificates/office"))), (0o600, 0o700));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn nothing_is_left_behind_after_an_error() {
        let d = dir("error");
        // A rename can't put a file over a folder.
        std::fs::create_dir(d.join("taken")).unwrap();
        assert!(write_atomic(&d.join("taken"), "x").is_err());
        assert_eq!(others(&d, &["taken"]), Vec::<String>::new());
        // No folder: nothing made.
        assert!(write_atomic(&d.join("missing/config.toml"), "x").is_err());
        assert_eq!(others(&d, &["taken"]), Vec::<String>::new());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_read_only_file_isnt_replaced() {
        let d = dir("read-only");
        let path = d.join("mimeapps.list");
        std::fs::write(&path, "mine\n").unwrap();
        std::fs::set_permissions(&path, Permissions::from_mode(0o444)).unwrap();
        // Root may write to it anyway.
        if OpenOptions::new().write(true).open(&path).is_ok() {
            return;
        }
        assert_eq!(write_atomic(&path, "x").unwrap_err().kind(), ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine\n");
        assert_eq!(others(&d, &["mimeapps.list"]), Vec::<String>::new());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn writers_on_other_threads_dont_get_in_each_others_way() {
        let d = dir("threads");
        let path = d.join("lituus.lua");
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    (0..50).map(|i| write_atomic(&path, format!("{t} {i}\n"))).collect::<Vec<_>>()
                })
            })
            .collect();
        for t in threads {
            for result in t.join().unwrap() {
                result.unwrap();
            }
        }
        assert!(std::fs::read_to_string(&path).unwrap().ends_with(" 49\n"));
        assert_eq!(others(&d, &["lituus.lua"]), Vec::<String>::new());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn only_a_missing_file_reads_as_empty() {
        let d = dir("read");
        assert_eq!(read_or_empty(&d.join("none.toml")).unwrap(), "");
        std::fs::write(d.join("latin1.toml"), b"# caf\xe9\n").unwrap();
        let e = read_or_empty(&d.join("latin1.toml")).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidData);
        assert!(e.to_string().ends_with("latin1.toml isn't UTF-8 text"), "{e}");
        // A folder where the file should be.
        assert!(read_or_empty(&d).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
