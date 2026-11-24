//! Set up… (D-44): the dialog that adds the line loading Lituus's file to the end of the user's
//! Hyprland config, after asking, with a copy of the file first. Hyprland then reads its config
//! again, and the pages that write Lituus's file read whether it's loaded again.

use lituus_hypr::{ConfigFile, Hyprland, config_dir, reads_lua};
use slint::ComponentHandle;

use crate::{MainWindow, Setup};

pub fn connect(window: &MainWindow) {
    let weak = window.as_weak();
    window.global::<Setup>().on_open(move || open(&weak));
    let weak = window.as_weak();
    window.global::<Setup>().on_confirm(move || confirm(&weak));
}

/// Lua or hyprlang as the Hyprland running reads its config, as every page decides.
fn file() -> ConfigFile {
    let version = Hyprland::from_env().and_then(|h| h.version()).ok();
    ConfigFile::new(config_dir(), reads_lua(version.as_ref(), &config_dir()))
}

fn open(weak: &slint::Weak<MainWindow>) {
    let weak = weak.clone();
    std::thread::spawn(move || {
        let file = file();
        let _ = weak.upgrade_in_event_loop(move |window| {
            let setup = window.global::<Setup>();
            let main = file.main_path();
            let (comment, line) = file.load_lines();
            setup.set_file(shorten_home(&main.display().to_string()).into());
            setup.set_comment(comment.into());
            setup.set_line(line.into());
            setup.set_backup(file.backup_path().file_name().unwrap_or_default().to_string_lossy().into_owned().into());
            setup.set_error(
                if main.exists() { String::new() } else { "There's no config file here to add it to.".to_owned() }
                    .into(),
            );
            setup.set_working(false);
            setup.set_shown(true);
        });
    });
}

fn confirm(weak: &slint::Weak<MainWindow>) {
    let Some(window) = weak.upgrade() else { return };
    window.global::<Setup>().set_working(true);
    window.global::<Setup>().set_error("".into());
    let weak = weak.clone();
    std::thread::spawn(move || {
        let file = file();
        let main = shorten_home(&file.main_path().display().to_string());
        // Added once: after a reload that failed, Add the line only reloads again.
        let added = if file.loaded() { Ok(()) } else { file.add_load_line().map(|_| ()) };
        let done = added.map_err(|e| format!("Couldn't add it to {main}: {e}.")).and_then(|()| {
            Hyprland::from_env().and_then(|h| h.request("reload")).map(|_| ()).map_err(|e| {
                format!(
                    "Added to {main}, but Hyprland didn't reload: {e}. It loads Lituus's file the next time it starts."
                )
            })
        });
        let _ = weak.clone().upgrade_in_event_loop(move |window| {
            let setup = window.global::<Setup>();
            setup.set_working(false);
            match done {
                Ok(()) => setup.set_shown(false),
                Err(e) => setup.set_error(e.into()),
            }
            crate::input::refresh(&weak);
            crate::displays::refresh(&weak);
            crate::shortcuts::refresh(&weak);
        });
    });
}

fn shorten_home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_owned(),
    }
}
