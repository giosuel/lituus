//! The Accessibility page (D-58, S-185): text size, high contrast, reduce motion and the cursor
//! size (Appearance's); Hyprland's cursor zoom, with Super + = and Super + -; the Orca screen
//! reader, with Super + Alt + S. Text size, contrast and motion go to gsettings (GTK, and the apps
//! that read GNOME's keys); Qt's text size, motion, the zoom and the binds to Lituus's Hyprland
//! file; the binds run `lituus --zoom in|out` and `lituus --screen-reader`.

use std::process::Command;

use slint::SharedString;

use crate::{AccessView, HeroInfo, MainWindow, model, update};

const SECTION: &str = "Accessibility";
const INTERFACE: &str = "org.gnome.desktop.interface";
const A11Y: &str = "org.gnome.desktop.a11y.interface";
const APPLICATIONS: &str = "org.gnome.desktop.a11y.applications";
/// Text sizes, in percent of the toolkits' own.
pub const TEXT_SIZES: [u32; 6] = [100, 110, 125, 150, 175, 200];
/// The zoom's range and a key's step.
pub const ZOOM_MIN: f64 = 1.0;
pub const ZOOM_MAX: f64 = 4.0;
pub const ZOOM_STEP: f64 = 0.5;

/// The choices, as config.toml's [accessibility] keeps them.
#[derive(Clone, Debug, PartialEq)]
pub struct Choices {
    /// Percent.
    pub text: u32,
    pub contrast: bool,
    pub reduce_motion: bool,
    pub zoom: f64,
}

impl Default for Choices {
    fn default() -> Self {
        Self { text: 100, contrast: false, reduce_motion: false, zoom: 1.0 }
    }
}

#[derive(Default)]
pub struct State {
    pub loaded: bool,
    pub choices: Choices,
    /// gsettings is there.
    pub gsettings: bool,
    pub orca: bool,
    /// Orca runs.
    pub reader: bool,
    pub install: String,
    /// Hyprland runs and doesn't load Lituus's file.
    pub not_loaded: bool,
    pub error: Option<String>,
    /// The page was opened: read it again.
    pub reread: bool,
    /// Counts the zoom slider's moves: the file is written after the last.
    pub(crate) moves: u64,
}

// ---- reading -------------------------------------------------------------------------------------

/// Reads the page on a thread: gsettings's keys, the zoom Hyprland has now, Orca.
pub fn spawn(weak: slint::Weak<MainWindow>) {
    std::thread::spawn(move || {
        let config = crate::config::load(&crate::config::path()).accessibility;
        let gsettings = gsettings_get(INTERFACE, "text-scaling-factor").is_some();
        let mut choices = Choices {
            text: config.text,
            contrast: config.contrast,
            reduce_motion: config.reduce_motion,
            zoom: config.zoom,
        };
        if gsettings {
            if let Some(f) = gsettings_get(INTERFACE, "text-scaling-factor").and_then(|v| v.parse::<f64>().ok()) {
                choices.text = nearest_text((f * 100.0).round() as u32);
            }
            choices.contrast = gsettings_get(A11Y, "high-contrast").as_deref() == Some("true");
            choices.reduce_motion = gsettings_get(INTERFACE, "enable-animations").as_deref() == Some("false");
        }
        if let Some(z) = live_zoom() {
            choices.zoom = z;
        }
        let orca = crate::power::installed("orca");
        let reader = !crate::power::pids("orca").is_empty();
        let install =
            crate::power::install_line(&std::fs::read_to_string("/etc/os-release").unwrap_or_default(), "orca");
        let not_loaded = crate::setup::not_loaded();
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| {
            update(&w, |app| {
                let state = &mut app.access;
                state.choices = choices;
                state.gsettings = gsettings;
                state.orca = orca;
                state.reader = reader;
                state.install = install;
                state.not_loaded = not_loaded;
                state.loaded = true;
                // Read again: a note from before (Orca missing) may no longer hold.
                state.error = None;
            })
        });
    });
}

fn gsettings_get(schema: &str, key: &str) -> Option<String> {
    let out = Command::new("gsettings").args(["get", schema, key]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn gsettings_set(schema: &str, key: &str, value: &str) -> bool {
    Command::new("gsettings").args(["set", schema, key, value]).status().is_ok_and(|s| s.success())
}

/// The text size listed nearest to `percent`.
fn nearest_text(percent: u32) -> u32 {
    *TEXT_SIZES.iter().min_by_key(|t| t.abs_diff(percent)).unwrap_or(&100)
}

/// Hyprland's cursor zoom now.
fn live_zoom() -> Option<f64> {
    let hypr = lituus_hypr::Hyprland::from_env().ok()?;
    match hypr.option("cursor:zoom_factor").ok()?? {
        lituus_hypr::Value::Number(f) => Some(f),
        _ => None,
    }
}

// ---- writing -------------------------------------------------------------------------------------

/// The zoom, clamped and on a step of 0.1 ("1.5").
fn zoom_text(z: f64) -> String {
    let z = (z.clamp(ZOOM_MIN, ZOOM_MAX) * 10.0).round() / 10.0;
    if z.fract() == 0.0 { format!("{z:.0}") } else { format!("{z:.1}") }
}

/// Qt's font DPI for a text size: 96 at 100 %.
fn qt_dpi(text: u32) -> u32 {
    96 * text / 100
}

/// Lituus's part of the Hyprland config: Qt's text size, animations off, the zoom, and the binds.
pub fn part(c: &Choices, lua: bool) -> String {
    let comment = if lua { "--" } else { "#" };
    let mut lines = vec![format!(
        "{comment} Written by Lituus ({SECTION}). Change it in Lituus: text size, motion, zoom and the screen reader."
    )];
    if c.text != 100 {
        let dpi = qt_dpi(c.text).to_string();
        lines.push(if lua {
            format!("hl.env(\"QT_FONT_DPI\", {})", lituus_hypr::lua_string(&dpi))
        } else {
            format!("env = QT_FONT_DPI,{dpi}")
        });
    }
    if c.reduce_motion {
        lines.push(if lua {
            "hl.config({ animations = { enabled = false } })".into()
        } else {
            "animations:enabled = false".into()
        });
    }
    if c.zoom > ZOOM_MIN {
        let z = zoom_text(c.zoom);
        lines.push(if lua {
            format!("hl.config({{ cursor = {{ zoom_factor = {z} }} }})")
        } else {
            format!("cursor:zoom_factor = {z}")
        });
    }
    for (keys, mods, key, command) in [
        ("SUPER + equal", "SUPER", "equal", "lituus --zoom in"),
        ("SUPER + minus", "SUPER", "minus", "lituus --zoom out"),
        ("SUPER + ALT + S", "SUPER ALT", "S", "lituus --screen-reader"),
    ] {
        lines.push(if lua {
            format!("hl.bind({}, hl.dsp.exec_cmd({}))", lituus_hypr::lua_string(keys), lituus_hypr::lua_string(command))
        } else {
            format!("bind = {mods}, {key}, exec, {command}")
        });
    }
    lines.join("\n") + "\n"
}

/// Hyprland's zoom now, live (not in the file).
fn set_live_zoom(z: f64) -> Result<(), String> {
    let hypr = lituus_hypr::Hyprland::from_env().map_err(|e| e.to_string())?;
    let lua = crate::setup::file().lua;
    let z = zoom_text(z);
    if lua {
        hypr.eval(&format!("hl.config({{ cursor = {{ zoom_factor = {z} }} }})")).map_err(|e| e.to_string())
    } else {
        hypr.keyword("cursor:zoom_factor", &z).map_err(|e| e.to_string())
    }
}

/// Hyprland's animations now, live.
fn set_live_animations(on: bool) -> Result<(), String> {
    let hypr = lituus_hypr::Hyprland::from_env().map_err(|e| e.to_string())?;
    if crate::setup::file().lua {
        hypr.eval(&format!("hl.config({{ animations = {{ enabled = {on} }} }})")).map_err(|e| e.to_string())
    } else {
        hypr.keyword("animations:enabled", if on { "1" } else { "0" }).map_err(|e| e.to_string())
    }
}

/// `lituus --zoom in|out`, from the binds: one step, live.
pub fn zoom_key(direction: &str) {
    let now = live_zoom().unwrap_or(ZOOM_MIN);
    let next = if direction == "out" { now - ZOOM_STEP } else { now + ZOOM_STEP };
    let _ = set_live_zoom(next.clamp(ZOOM_MIN, ZOOM_MAX));
}

/// `lituus --screen-reader`, from the bind: Orca on or off.
pub fn reader_key() {
    let on = crate::power::pids("orca").is_empty();
    set_reader(on);
}

/// Orca started or stopped, and GNOME's key that says so (apps read it).
fn set_reader(on: bool) {
    let _ = gsettings_set(APPLICATIONS, "screen-reader-enabled", if on { "true" } else { "false" });
    if on {
        crate::power::start("orca");
    } else {
        for pid in crate::power::pids("orca") {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
    }
}

/// Writes the choices: config.toml, gsettings, Hyprland live and Lituus's file; on a thread.
/// (Hyprland reads its config again when the file changes, so a slider being dragged writes it
/// once, after the last move: see `act`.)
fn apply(weak: &slint::Weak<MainWindow>, before: Choices, after: Choices, gsettings: bool) {
    let path = crate::config::path();
    let _ = crate::config::set(&path, "accessibility", "text", i64::from(after.text));
    let _ = crate::config::set(&path, "accessibility", "contrast", after.contrast);
    let _ = crate::config::set(&path, "accessibility", "reduce_motion", after.reduce_motion);
    let _ = crate::config::set(&path, "accessibility", "zoom", after.zoom);
    let weak = weak.clone();
    std::thread::spawn(move || {
        let mut errors = Vec::new();
        if gsettings {
            if after.text != before.text
                && !gsettings_set(INTERFACE, "text-scaling-factor", &format!("{:.2}", f64::from(after.text) / 100.0))
            {
                errors.push("Couldn't set the text size through gsettings.");
            }
            if after.contrast != before.contrast && !gsettings_set(A11Y, "high-contrast", &after.contrast.to_string()) {
                errors.push("Couldn't set high contrast through gsettings.");
            }
            if after.reduce_motion != before.reduce_motion
                && !gsettings_set(INTERFACE, "enable-animations", &(!after.reduce_motion).to_string())
            {
                errors.push("Couldn't set the apps' animations through gsettings.");
            }
        }
        if after.reduce_motion != before.reduce_motion {
            let _ = set_live_animations(!after.reduce_motion);
        }
        if after.zoom != before.zoom {
            let _ = set_live_zoom(after.zoom);
        }
        let file = crate::setup::file();
        if file.write_section(SECTION, &part(&after, file.lua)).is_err() {
            errors.push("Couldn't save the change in Lituus's Hyprland file.");
        }
        if !errors.is_empty() {
            let text = errors.join(" ");
            let w = weak.clone();
            let _ = weak.upgrade_in_event_loop(move |_| update(&w, |app| app.access.error = Some(text)));
        }
    });
}

/// What a click on the page does: (verb, number).
pub fn act(weak: &slint::Weak<MainWindow>, state: &mut State, verb: &str, value: f32) {
    let before = state.choices.clone();
    let mut after = before.clone();
    match verb {
        "dismiss" | "copied" => {
            state.error = None;
            return;
        }
        "text" => after.text = TEXT_SIZES.get(value.max(0.0) as usize).copied().unwrap_or(after.text),
        "contrast" => after.contrast = !after.contrast,
        "motion" => after.reduce_motion = !after.reduce_motion,
        // Live at once; saved, and in Lituus's file, after the last move.
        "zoom" => {
            let z = (f64::from(value) * 10.0).round() / 10.0;
            if z == before.zoom {
                return;
            }
            state.choices.zoom = z;
            std::thread::spawn(move || {
                let _ = set_live_zoom(z);
            });
            state.moves += 1;
            let ticket = state.moves;
            let weak = weak.clone();
            let gsettings = state.gsettings;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(700));
                let w = weak.clone();
                let _ = weak.upgrade_in_event_loop(move |_| {
                    update(&w, |app| {
                        if app.access.moves == ticket {
                            let now = app.access.choices.clone();
                            apply(&w, now.clone(), now, gsettings);
                        }
                    })
                });
            });
            return;
        }
        "reader" => {
            if !state.orca {
                state.error = Some(if state.install.is_empty() {
                    "Orca isn't installed. Install it with your package manager, then turn it on here.".into()
                } else {
                    format!("Orca isn't installed: {}", state.install)
                });
                return;
            }
            // From whether Orca runs now: its key may have started or stopped it meanwhile.
            let on = crate::power::pids("orca").is_empty();
            state.reader = on;
            std::thread::spawn(move || set_reader(on));
            return;
        }
        _ => return,
    }
    if after == before {
        return;
    }
    state.choices = after.clone();
    apply(weak, before, after, state.gsettings);
}

// ---- the page ------------------------------------------------------------------------------------

fn s(text: impl AsRef<str>) -> SharedString {
    SharedString::from(text.as_ref())
}

/// The sidebar's status.
pub fn status(state: &State) -> String {
    if state.choices.zoom > ZOOM_MIN {
        "zoom".into()
    } else if state.reader {
        "reader".into()
    } else {
        String::new()
    }
}

pub fn view(state: &State, appearance: &crate::appearance::State) -> AccessView {
    let mut v = AccessView { error: s(state.error.clone().unwrap_or_default()), ..AccessView::default() };
    if !state.loaded {
        v.state = s("loading");
        return v;
    }
    let c = &state.choices;
    v.state = s("list");
    v.sub = s("Seeing, zoom and the screen reader");
    let mut on = Vec::new();
    if c.text > 100 {
        on.push("Larger text");
    }
    if c.contrast {
        on.push("High contrast");
    }
    if c.reduce_motion {
        on.push("Reduced motion");
    }
    let on = on.join(" · ").to_lowercase();
    v.hero = HeroInfo {
        icon: s("access"),
        name: s(if c.zoom > ZOOM_MIN { format!("Zoom {}×", zoom_text(c.zoom)) } else { "No zoom".into() }),
        meta: s(if on.is_empty() { "Nothing turned on".to_owned() } else { capitalised(&on) }),
        tone: s(if on.is_empty() && c.zoom <= ZOOM_MIN { "muted" } else { "accent" }),
        detail: s(if state.reader { "screen reader on" } else { "screen reader off" }),
    };
    v.gsettings = state.gsettings;
    v.texts = model(TEXT_SIZES.iter().map(|t| s(format!("{t} %"))).collect());
    v.text = TEXT_SIZES.iter().position(|t| *t == c.text).map_or(0, |i| i as i32);
    v.contrast = c.contrast;
    v.motion = c.reduce_motion;
    v.zoom = c.zoom as f32;
    v.zoom_text = s(format!("{}×", zoom_text(c.zoom)));
    v.orca = state.orca;
    v.reader = state.reader;
    v.install = s(&state.install);
    v.not_loaded = state.not_loaded;
    let (sizes, at) = crate::appearance::cursor_choices(appearance);
    v.cursor_sizes = model(sizes.iter().map(s).collect());
    v.cursor_size = at;
    v
}

fn capitalised(text: &str) -> String {
    let mut c = text.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

pub fn render(window: &MainWindow, state: &State, appearance: &crate::appearance::State) {
    window.set_access(view(state, appearance));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lituus_file_part_in_both_languages() {
        let c = Choices { text: 125, contrast: true, reduce_motion: true, zoom: 1.5 };
        let lua = part(&c, true);
        assert!(lua.starts_with("-- Written by Lituus (Accessibility)."), "{lua}");
        assert!(lua.contains("hl.env(\"QT_FONT_DPI\", \"120\")"), "{lua}");
        assert!(lua.contains("hl.config({ animations = { enabled = false } })"), "{lua}");
        assert!(lua.contains("hl.config({ cursor = { zoom_factor = 1.5 } })"), "{lua}");
        assert!(lua.contains(r#"hl.bind("SUPER + equal", hl.dsp.exec_cmd("lituus --zoom in"))"#), "{lua}");
        assert!(lua.contains(r#"hl.bind("SUPER + ALT + S", hl.dsp.exec_cmd("lituus --screen-reader"))"#), "{lua}");
        let conf = part(&c, false);
        assert!(conf.contains("env = QT_FONT_DPI,120") && conf.contains("animations:enabled = false"), "{conf}");
        assert!(
            conf.contains("cursor:zoom_factor = 1.5") && conf.contains("bind = SUPER, minus, exec, lituus --zoom out"),
            "{conf}"
        );
        // Nothing turned on: only the binds.
        let plain = part(&Choices::default(), true);
        assert!(
            !plain.contains("QT_FONT_DPI") && !plain.contains("animations") && !plain.contains("zoom_factor"),
            "{plain}"
        );
    }

    #[test]
    fn sizes_and_zoom_steps() {
        assert_eq!(nearest_text(124), 125);
        assert_eq!(nearest_text(100), 100);
        assert_eq!(nearest_text(300), 200);
        assert_eq!(qt_dpi(125), 120);
        assert_eq!(zoom_text(1.5), "1.5");
        assert_eq!(zoom_text(2.0), "2");
        assert_eq!(zoom_text(9.0), "4");
        assert_eq!(zoom_text(0.5), "1");
    }
}
