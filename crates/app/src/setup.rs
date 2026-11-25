//! Set up… (D-44; the whole setup in one go since K6): the changes Lituus makes once to files of
//! the user's, after asking, each kept as a .bak first. The line loading Lituus's file at the end
//! of the user's Hyprland config (Hyprland then reads its config again), and hypridle.conf taken
//! over for the Power page. The pages that need a step not done show Set up….

use lituus_hypr::{ConfigFile, Hyprland, config_dir, reads_lua};
use slint::{ComponentHandle, Model};

use crate::{MainWindow, Setup, SetupStep, model, update};

pub fn connect(window: &MainWindow) {
    let weak = window.as_weak();
    window.global::<Setup>().on_open(move || open(&weak));
    let weak = window.as_weak();
    window.global::<Setup>().on_toggle(move |id| toggle(&weak, &id));
    let weak = window.as_weak();
    window.global::<Setup>().on_confirm(move || confirm(&weak));
}

/// Lua or hyprlang as the Hyprland running reads its config, as every page decides.
pub(crate) fn file() -> ConfigFile {
    let version = Hyprland::from_env().and_then(|h| h.version()).ok();
    ConfigFile::new(config_dir(), reads_lua(version.as_ref(), &config_dir()))
}

/// Hyprland runs and its config doesn't load Lituus's file: what pages write there is lost when
/// it restarts.
pub(crate) fn not_loaded() -> bool {
    Hyprland::from_env().is_ok() && !file().loaded()
}

/// The steps as they are now: Hyprland's line, and hypridle.conf when there's one.
fn steps() -> Vec<SetupStep> {
    let file = file();
    let main = shorten_home(&file.main_path().display().to_string(), &home());
    let backup = file.backup_path().file_name().unwrap_or_default().to_string_lossy().into_owned();
    let (comment, line) = file.load_lines();
    let loaded = file.loaded();
    let exists = file.main_path().exists();
    let text = if loaded {
        format!("{main} loads Lituus's file.")
    } else if !exists {
        format!("There's no config at {main} to add it to.")
    } else {
        format!(
            "Added at the end of {main}, after your own settings and shortcuts, so Lituus's come last. Your file is kept as {backup}; Hyprland reads its config again after."
        )
    };
    let mut steps = vec![SetupStep {
        id: "hyprland".into(),
        title: "Hyprland loads Lituus's settings".into(),
        text: text.into(),
        comment: comment.into(),
        line: line.into(),
        done: loaded,
        chosen: !loaded && exists,
    }];
    if let Some(needed) = crate::power::idle_needs_takeover() {
        let backup = crate::power::idle_backup().file_name().unwrap_or_default().to_string_lossy().into_owned();
        steps.push(SetupStep {
            id: "hypridle".into(),
            title: "Lituus manages hypridle.conf".into(),
            text: if needed {
                format!(
                    "For the Power page's idle times and lock screen: Lituus writes its own with the same times. Yours is kept as {backup}."
                )
            } else {
                "The Power page's idle times and lock screen are Lituus's to change.".to_owned()
            }
            .into(),
            done: !needed,
            chosen: needed,
            ..SetupStep::default()
        });
    }
    steps
}

fn show_steps(setup: &Setup<'_>, steps: Vec<SetupStep>) {
    setup.set_ready(ready(&steps));
    setup.set_steps(model(steps));
}

/// Some step is chosen and not done.
fn ready(steps: &[SetupStep]) -> bool {
    steps.iter().any(|s| s.chosen && !s.done)
}

fn open(weak: &slint::Weak<MainWindow>) {
    let weak = weak.clone();
    std::thread::spawn(move || {
        let steps = steps();
        let _ = weak.upgrade_in_event_loop(move |window| {
            let setup = window.global::<Setup>();
            show_steps(&setup, steps);
            setup.set_error("".into());
            setup.set_working(false);
            setup.set_shown(true);
        });
    });
}

/// A step's switch.
fn toggle(weak: &slint::Weak<MainWindow>, id: &str) {
    let Some(window) = weak.upgrade() else { return };
    let setup = window.global::<Setup>();
    let steps: Vec<SetupStep> = setup
        .get_steps()
        .iter()
        .map(|s| if s.id == id && !s.done { SetupStep { chosen: !s.chosen, ..s } } else { s })
        .collect();
    show_steps(&setup, steps);
}

/// Set up: the chosen steps, Hyprland's first. What failed is said in the dialog, which stays open
/// with the steps done ticked; when all went well, it closes.
fn confirm(weak: &slint::Weak<MainWindow>) {
    let Some(window) = weak.upgrade() else { return };
    let setup = window.global::<Setup>();
    let chosen: Vec<String> =
        setup.get_steps().iter().filter(|s| s.chosen && !s.done).map(|s| s.id.to_string()).collect();
    setup.set_working(true);
    setup.set_error("".into());
    let weak = weak.clone();
    std::thread::spawn(move || {
        let mut errors = Vec::new();
        if chosen.iter().any(|c| c == "hyprland")
            && let Err(e) = add_load_line()
        {
            errors.push(e);
        }
        let idle = chosen.iter().any(|c| c == "hypridle");
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| {
            update(&w, |app| {
                if idle && let Err(e) = crate::power::take_over_idle(&mut app.power) {
                    errors.push(e);
                }
                crate::power::read_files(&mut app.power, &crate::config::load(&crate::config::path()));
                app.appearance.reread = true;
            });
            crate::input::refresh(&w);
            crate::displays::refresh(&w);
            crate::shortcuts::refresh(&w);
            let w2 = w.clone();
            std::thread::spawn(move || {
                let steps = steps();
                let _ = w2.upgrade_in_event_loop(move |window| {
                    let setup = window.global::<Setup>();
                    setup.set_working(false);
                    if errors.is_empty() {
                        setup.set_shown(false);
                    } else {
                        setup.set_error(errors.join(" ").into());
                    }
                    show_steps(&setup, steps);
                });
            });
        });
    });
}

/// The line at the end of the user's Hyprland config, and Hyprland reading it again.
fn add_load_line() -> Result<(), String> {
    let file = file();
    let main = shorten_home(&file.main_path().display().to_string(), &home());
    // Added once: after a reload that failed, Set up only reloads again.
    let added = if file.loaded() { Ok(()) } else { file.add_load_line().map(|_| ()) };
    added.map_err(|e| format!("Couldn't add it to {main}: {e}.")).and_then(|()| {
        Hyprland::from_env().and_then(|h| h.request("reload")).map(|_| ()).map_err(|e| {
            format!("Added to {main}, but Hyprland didn't reload: {e}. It loads Lituus's file the next time it starts.")
        })
    })
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// "/home/me/.config/hypr/hyprland.lua" → "~/.config/hypr/hyprland.lua".
fn shorten_home(path: &str, home: &str) -> String {
    let home = home.trim_end_matches('/');
    match path.strip_prefix(home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_up_does_something_only_with_a_step_chosen_and_not_done() {
        let step = |done, chosen| SetupStep { done, chosen, ..SetupStep::default() };
        assert!(ready(&[step(true, false), step(false, true)]));
        assert!(!ready(&[step(true, true), step(false, false)]));
        assert!(!ready(&[]));
    }

    #[test]
    fn the_home_folder_is_a_tilde() {
        assert_eq!(shorten_home("/home/me/.config/hypr/hyprland.lua", "/home/me"), "~/.config/hypr/hyprland.lua");
        assert_eq!(shorten_home("/home/me/.config/hypr/hyprland.lua", "/home/me/"), "~/.config/hypr/hyprland.lua");
        // Another folder whose name starts the same isn't in it.
        assert_eq!(
            shorten_home("/home/me2/.config/hypr/hyprland.lua", "/home/me"),
            "/home/me2/.config/hypr/hyprland.lua"
        );
        assert_eq!(shorten_home("/etc/hypr/hyprland.conf", "/home/me"), "/etc/hypr/hyprland.conf");
        // No home, or / as home: nothing to shorten.
        assert_eq!(shorten_home("/home/me/x", ""), "/home/me/x");
        assert_eq!(shorten_home("/home/me/x", "/"), "/home/me/x");
    }
}
