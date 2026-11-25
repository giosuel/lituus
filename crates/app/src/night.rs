//! The Night light page (D-57, S-177 to S-179) and the quick popup's row: hyprsunset warms the
//! screens. The temperature changes at once through its IPC (`hyprctl hyprsunset`); the schedule
//! goes to hyprsunset.conf as two profiles (one, all the time), read when hyprsunset starts, so
//! it's started again after a change. Sunset and sunrise come from the time zone's place in zone1970.tab (or a
//! location typed in), and are written again as the days get longer or shorter, while Lituus runs.

use std::path::PathBuf;
use std::time::Duration;

use slint::SharedString;

use crate::{HeroInfo, MainWindow, NightView, update};

/// The first line of a hyprsunset.conf Lituus wrote.
const HEADER: &str =
    "# Written by Lituus (Night light). Lituus rewrites this file; your own copy is hyprsunset.conf.bak.";
pub const COOLEST: u32 = 6500;
pub const WARMEST: u32 = 2500;

#[derive(Default)]
pub struct State {
    pub installed: bool,
    pub enabled: bool,
    /// On all the time, not only on its schedule.
    pub always: bool,
    pub temperature: u32,
    /// "sun" (sunset to sunrise) or "times".
    pub schedule: String,
    pub from: String,
    pub to: String,
    /// The location typed in, else the time zone's.
    pub manual: Option<(f64, f64)>,
    pub zone: String,
    pub zone_place: Option<(f64, f64)>,
    /// Now, as minutes after midnight; the day of the year and the UTC offset in minutes.
    pub now: u32,
    pub day: u32,
    pub offset: i32,
    pub error: Option<String>,
    /// The line that installs hyprsunset here.
    pub install: String,
    /// Counts the times typed and refused: the field shows the time in use again.
    pub refused: i32,
    /// hyprsunset.conf as Lituus last wrote it (none: someone else's, or none at all), to notice
    /// when the sun's times move.
    pub(crate) written: Option<String>,
    /// Counts the temperature's moves: the screens go back to the schedule after the last.
    pub(crate) moves: u64,
    /// What the screens show through hyprsunset, as far as Lituus knows: a temperature, or their
    /// own colours (`Some(None)`); `None` before Lituus told it anything.
    pub(crate) shown: Option<Option<u32>>,
    /// The running hyprsunset has an older schedule than the file: it's started again when that
    /// can't be seen (see `reach`).
    pub(crate) pending: bool,
}

pub fn path() -> PathBuf {
    lituus_hypr::config_dir().join("hyprsunset.conf")
}

/// Reads the choices (config.toml's [night_light]), the clock and the place, and whether hyprsunset
/// is installed.
pub fn read(state: &mut State, config: &crate::config::NightLightConfig) {
    state.installed = crate::power::installed("hyprsunset");
    state.enabled = config.enabled;
    state.always = config.always;
    state.temperature = config.temperature.clamp(WARMEST, COOLEST);
    state.schedule = if config.schedule == "times" { "times".into() } else { "sun".into() };
    state.from = config.from.clone();
    state.to = config.to.clone();
    state.manual = (config.location == "manual").then_some((config.latitude, config.longitude));
    state.zone = zone();
    state.zone_place = place(&state.zone);
    state.written = std::fs::read_to_string(path()).ok().filter(|text| ours(text));
    state.install = install_command(&std::fs::read_to_string("/etc/os-release").unwrap_or_default());
    clock(state);
}

/// A hyprsunset.conf Lituus wrote.
fn ours(text: &str) -> bool {
    text.lines().next() == Some(HEADER)
}

/// The line that installs hyprsunset, from os-release's ID and ID_LIKE; empty when unknown.
fn install_command(os_release: &str) -> String {
    let ids: Vec<String> = os_release
        .lines()
        .filter_map(|l| l.strip_prefix("ID=").or_else(|| l.strip_prefix("ID_LIKE=")))
        .flat_map(|v| v.trim_matches('"').split_whitespace().map(str::to_owned).collect::<Vec<_>>())
        .collect();
    let is = |id: &str| ids.iter().any(|i| i == id);
    if is("arch") {
        "sudo pacman -S hyprsunset"
    } else if is("fedora") {
        "sudo dnf install hyprsunset"
    } else if is("debian") || is("ubuntu") {
        "sudo apt install hyprsunset"
    } else if is("nixos") {
        "nix profile install nixpkgs#hyprsunset"
    } else {
        ""
    }
    .to_owned()
}

/// At Lituus's start: with Night light on and its schedule in place, hyprsunset should run.
pub fn ensure_running(state: &State) {
    if state.installed && state.enabled && state.written.is_some() && crate::power::pids("hyprsunset").is_empty() {
        std::thread::spawn(|| crate::power::start("hyprsunset"));
    }
}

/// What hyprsunset shows now, at once (its IPC): the temperature, or the screens as they are.
/// When it isn't running (Night light off since Lituus started), it's started first: it reads
/// the file just written, then takes the temperature.
fn show(warm: Option<u32>) {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let args: Vec<String> = match warm {
        Some(t) => vec!["temperature".into(), t.to_string()],
        None => vec!["identity".into()],
    };
    std::thread::spawn(move || {
        let _one = ONE_AT_A_TIME.lock();
        let send = || {
            std::process::Command::new("hyprctl")
                .arg("hyprsunset")
                .args(&args)
                .output()
                .is_ok_and(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).contains("Couldn't"))
        };
        if send() || !crate::power::pids("hyprsunset").is_empty() {
            return;
        }
        crate::power::start("hyprsunset");
        // Its socket comes a moment after it starts.
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(100));
            if send() {
                return;
            }
        }
    });
}

fn clock(state: &mut State) {
    if let Some((day, offset, now)) = read_clock() {
        (state.day, state.offset, state.now) = (day, offset, now);
    }
}

/// The day of the year, the UTC offset in minutes and the minutes after midnight, from `date`
/// (which follows TZ and the time zone database).
fn read_clock() -> Option<(u32, i32, u32)> {
    let out = std::process::Command::new("date").arg("+%j %z %H %M").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [day, offset, h, m] = parts[..] else { return None };
    Some((day.parse().ok()?, parse_offset(offset), h.parse::<u32>().ok()? * 60 + m.parse::<u32>().ok()?))
}

/// "+0100" → 60, "-0430" → -270.
fn parse_offset(z: &str) -> i32 {
    let sign = if z.starts_with('-') { -1 } else { 1 };
    let digits = z.trim_start_matches(['+', '-']);
    let (h, m) = digits.split_at(digits.len().min(2));
    sign * (h.parse::<i32>().unwrap_or(0) * 60 + m.parse::<i32>().unwrap_or(0))
}

/// The time zone's name, from TZ or where /etc/localtime points ("Europe/Zurich").
fn zone() -> String {
    // TZ first, as `date` (and so the clock here) follows it.
    if let Ok(tz) = std::env::var("TZ")
        && tz.trim_start_matches(':').contains('/')
    {
        return tz.trim_start_matches(':').to_owned();
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| p.to_str().and_then(|s| s.split("zoneinfo/").nth(1)).map(str::to_owned))
        .unwrap_or_default()
}

/// The zone's place, from tzdata's zone1970.tab (its main city).
fn place(zone: &str) -> Option<(f64, f64)> {
    let tab = std::fs::read_to_string("/usr/share/zoneinfo/zone1970.tab").ok()?;
    tab.lines().filter(|l| !l.starts_with('#')).find_map(|l| {
        let cols: Vec<&str> = l.split('\t').collect();
        (cols.get(2) == Some(&zone)).then(|| coordinates(cols.get(1)?)).flatten()
    })
}

/// ISO 6709 as zone1970.tab writes it: "+4723+00832" (±DDMM±DDDMM, or with seconds).
fn coordinates(text: &str) -> Option<(f64, f64)> {
    let split = text[1..].find(['+', '-'])? + 1;
    let (lat, lon) = text.split_at(split);
    let deg = |s: &str, d: usize| -> Option<f64> {
        let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
        let n = &s[1..];
        let whole: f64 = n.get(..d)?.parse().ok()?;
        let min: f64 = n.get(d..d + 2)?.parse().ok()?;
        let sec: f64 = n.get(d + 2..d + 4).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        Some(sign * (whole + min / 60.0 + sec / 3600.0))
    };
    Some((deg(lat, 2)?, deg(lon, 3)?))
}

/// Sunset and sunrise, in minutes after local midnight, by NOAA's formula; `None` in a polar day
/// or night.
pub fn sun(lat: f64, lon: f64, day: u32, offset: i32) -> Option<(u32, u32)> {
    let g = 2.0 * std::f64::consts::PI / 365.0 * (day as f64 - 1.0);
    let eqtime = 229.18
        * (0.000075 + 0.001868 * g.cos()
            - 0.032077 * g.sin()
            - 0.014615 * (2.0 * g).cos()
            - 0.040849 * (2.0 * g).sin());
    let decl = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin() - 0.006758 * (2.0 * g).cos()
        + 0.000907 * (2.0 * g).sin()
        - 0.002697 * (3.0 * g).cos()
        + 0.00148 * (3.0 * g).sin();
    let lat = lat.to_radians();
    let cos_ha = (90.833f64.to_radians().cos() / (lat.cos() * decl.cos())) - lat.tan() * decl.tan();
    if !(-1.0..=1.0).contains(&cos_ha) {
        return None;
    }
    let ha = cos_ha.acos().to_degrees();
    let local = |utc: f64| ((utc + offset as f64).round() as i64).rem_euclid(1440) as u32;
    Some((local(720.0 - 4.0 * (lon - ha) - eqtime), local(720.0 - 4.0 * (lon + ha) - eqtime)))
}

fn parse_time(t: &str) -> Option<u32> {
    let (h, m) = t.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

fn time_text(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// When it turns on and off today: (on, off) in minutes after midnight.
pub fn times(state: &State) -> Option<(u32, u32)> {
    if state.schedule == "times" {
        return Some((parse_time(&state.from)?, parse_time(&state.to)?));
    }
    let (lat, lon) = state.manual.or(state.zone_place)?;
    sun(lat, lon, state.day, state.offset)
}

/// It's on at `now`, between `on` and `off` (over midnight when `off` is earlier).
fn on_at(now: u32, (on, off): (u32, u32)) -> bool {
    if on < off { (on..off).contains(&now) } else { now >= on || now < off }
}

pub fn is_on(state: &State) -> bool {
    state.enabled && (state.always || times(state).is_some_and(|t| on_at(state.now, t)))
}

/// hyprsunset.conf for these choices. hyprsunset applies, when it starts, the profile whose time
/// passed last (wrapping round midnight); with none it would start at its own 6000 K, so off is
/// one profile, the screens as they are all day, and all the time one with the temperature.
pub fn conf(state: &State) -> String {
    let mut out = format!("{HEADER}\n");
    if state.enabled && state.always {
        out.push_str(&format!("\nprofile {{\n    time = 00:00\n    temperature = {}\n}}\n", state.temperature));
    } else if let (true, Some((on, off))) = (state.enabled, times(state)) {
        out.push_str(&format!(
            "\nprofile {{\n    time = {}\n    temperature = {}\n}}\n\nprofile {{\n    time = {}\n    identity = true\n}}\n",
            time_text(on),
            state.temperature,
            time_text(off)
        ));
    } else {
        out.push_str("\nprofile {\n    time = 00:00\n    identity = true\n}\n");
    }
    out
}

/// Writes the schedule (keeping someone else's file once, as .bak), and has the screens show it
/// (`reach`). `preview`: the temperature shows at once, also by day or with Night light off (the
/// slider being moved), and the screens go back to the schedule `PREVIEW` after the last move.
fn apply(weak: &slint::Weak<MainWindow>, state: &mut State, preview: bool) {
    if !state.installed {
        return;
    }
    let path = path();
    if let Ok(old) = std::fs::read_to_string(&path)
        && old.lines().next() != Some(HEADER)
    {
        let _ = std::fs::copy(&path, path.with_extension("conf.bak"));
    }
    let _ = std::fs::create_dir_all(lituus_hypr::config_dir());
    let text = conf(state);
    if let Err(e) = lituus_fs::write_atomic(&path, &text) {
        state.error = Some(format!("Couldn't save hyprsunset.conf: {e}"));
        return;
    }
    state.written = Some(text);
    if !preview {
        show_schedule(weak, state);
        return;
    }
    // Through its IPC (started first if it isn't running); the schedule after the last move.
    show(Some(state.temperature));
    state.shown = Some(Some(state.temperature));
    state.pending = true;
    state.moves += 1;
    let ticket = state.moves;
    let weak = weak.clone();
    std::thread::spawn(move || {
        std::thread::sleep(PREVIEW);
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| {
            update(&w, |app| {
                if app.night.moves == ticket {
                    show_schedule(&w, &mut app.night);
                }
            })
        });
    });
}

/// How a change reaches the screens. hyprsunset reads its schedule only when it starts, and
/// while it's started again the screens show their own colours for a moment (the owner on build
/// #28: "after a few seconds it glitches a bit"). So it's started again at once only when that
/// can't be seen: the screens show their own colours now, or are about to. Otherwise (warm before
/// and after) the colour goes through its IPC, and the new schedule waits (`pending`).
#[derive(Debug, PartialEq)]
enum Reach {
    Start,
    Restart,
    Tell,
}

fn reach(running: bool, shown: Option<Option<u32>>, wanted: Option<u32>) -> Reach {
    match (running, shown, wanted) {
        (false, _, _) => Reach::Start,
        (true, Some(None), _) | (true, _, None) => Reach::Restart,
        (true, _, Some(_)) => Reach::Tell,
    }
}

/// The screens as the schedule has them now, and hyprsunset with the file's schedule when that
/// can't be seen (`reach`).
fn show_schedule(weak: &slint::Weak<MainWindow>, state: &mut State) {
    let wanted = is_on(state).then_some(state.temperature);
    let running = !crate::power::pids("hyprsunset").is_empty();
    match reach(running, state.shown, wanted) {
        Reach::Tell => {
            show(wanted);
            state.pending = true;
        }
        Reach::Start | Reach::Restart => {
            restart_hyprsunset(weak);
            state.pending = false;
        }
    }
    state.shown = Some(wanted);
}

/// A schedule waiting (`pending`): hyprsunset is started again once the screens are without it,
/// or once it shows something else than it should (its older schedule switched). `running`:
/// what it shows, asked on another thread (None: it couldn't be asked).
fn settle(weak: &slint::Weak<MainWindow>, state: &mut State, running: Option<Option<u32>>) {
    let wanted = is_on(state).then_some(state.temperature);
    if state.pending && (wanted.is_none() || running != Some(wanted)) {
        restart_hyprsunset(weak);
        state.pending = false;
        state.shown = Some(wanted);
    }
}

/// What the running hyprsunset shows: a temperature, or the screens' own colours; None when it
/// can't be asked.
fn asked() -> Option<Option<u32>> {
    let ask = |args: &[&str]| {
        let out = std::process::Command::new("hyprctl").arg("hyprsunset").args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    if ask(&["identity", "get"])? == "true" {
        return Some(None);
    }
    ask(&["temperature"])?.parse().ok().map(Some)
}

/// When Lituus quits with a schedule waiting: hyprsunset is started again, so it follows the
/// file from now on.
pub fn quitting(state: &State) {
    if state.pending {
        let _ = crate::power::restart_now("hyprsunset");
    }
}

/// Started again on a thread (one at a time); if it doesn't keep running, the page says so.
fn restart_hyprsunset(weak: &slint::Weak<MainWindow>) {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let weak = weak.clone();
    std::thread::spawn(move || {
        let running = {
            let _one = ONE_AT_A_TIME.lock();
            crate::power::restart_now("hyprsunset")
        };
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| {
            update(&w, |app| {
                let error = &mut app.night.error;
                if !running {
                    *error = Some(NOT_RUNNING.into());
                } else if error.as_deref() == Some(NOT_RUNNING) {
                    *error = None;
                }
            });
        });
    });
}

/// How long a temperature moved to shows before the screens go back to the schedule.
const PREVIEW: Duration = Duration::from_secs(3);

/// When hyprsunset quits as soon as it's started.
const NOT_RUNNING: &str = "hyprsunset didn't keep running. Start it in a terminal (hyprsunset) to see why; the page tries again with your next change.";

/// A choice on the page or in the popup: (verb, number, text).
pub fn act(weak: &slint::Weak<MainWindow>, state: &mut State, verb: &str, value: f32, text: &str) {
    let path = crate::config::path();
    let save = |key: &str, v: toml_edit::Value| {
        let _ = crate::config::set(&path, "night_light", key, v);
    };
    match verb {
        "dismiss" | "copied" => state.error = None,
        "switch" => {
            state.enabled = !state.enabled;
            save("enabled", state.enabled.into());
            apply(weak, state, false);
        }
        // Shown at once, also by day or with Night light off; back to the schedule a few
        // seconds after the last move (the owner, 2026-11-25: "just briefly show the night light
        // for a few seconds everytime the slider is moved").
        "temperature" => {
            let t = ((value.clamp(WARMEST as f32, COOLEST as f32) / 100.0).round() * 100.0) as u32;
            if t != state.temperature {
                state.temperature = t;
                save("temperature", i64::from(t).into());
                apply(weak, state, true);
            }
        }
        // Turn on: Sunset to sunrise, Fixed times, or All the time (the owner on build #28: "lets
        // merge "all the time" into the "turn on" view switcher"). All the time keeps the
        // schedule chosen before, for when it's chosen again.
        "schedule" => {
            state.always = value as i32 == 2;
            save("always", state.always.into());
            if !state.always {
                state.schedule = if value as i32 == 1 { "times".into() } else { "sun".into() };
                save("schedule", state.schedule.as_str().into());
            }
            apply(weak, state, false);
        }
        "from" | "to" => {
            if parse_time(text).is_none() {
                state.error = Some("Type a time as 21:00.".into());
                state.refused += 1;
                return;
            }
            let t = time_text(parse_time(text).unwrap_or(0));
            if verb == "from" {
                state.from = t.clone();
            } else {
                state.to = t.clone();
            }
            save(verb, t.as_str().into());
            apply(weak, state, false);
        }
        "location" => {
            if value as i32 == 1 {
                state.manual = Some(state.manual.or(state.zone_place).unwrap_or((0.0, 0.0)));
                save("location", "manual".into());
            } else {
                state.manual = None;
                save("location", "zone".into());
            }
            apply(weak, state, false);
        }
        "latitude" | "longitude" => {
            let Ok(v) = text.trim().replace(',', ".").parse::<f64>() else {
                state.error = Some("Type the place in degrees, as 47.4.".into());
                state.refused += 1;
                return;
            };
            let (lat, lon) = state.manual.unwrap_or((0.0, 0.0));
            state.manual =
                Some(if verb == "latitude" { (v.clamp(-90.0, 90.0), lon) } else { (lat, v.clamp(-180.0, 180.0)) });
            save(verb, v.into());
            apply(weak, state, false);
        }
        _ => {}
    }
}

/// Every 20 seconds, the clock and whether hyprsunset is installed, read on this thread; when
/// either changed, the page and the popup follow ("On until 07:12" moves on), the schedule is
/// written again when the sun's times moved (the next day), and hyprsunset installed while Lituus
/// runs shows up, and is started if Night light is on.
pub fn spawn(weak: slint::Weak<MainWindow>) {
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            std::thread::sleep(Duration::from_secs(20));
            let now = (read_clock(), crate::power::installed("hyprsunset"));
            if last.as_ref() == Some(&now) {
                continue;
            }
            last = Some(now);
            let (clock, installed) = now;
            let w = weak.clone();
            let posted = weak.upgrade_in_event_loop(move |_| {
                update(&w.clone(), |app| {
                    let state = &mut app.night;
                    if let Some((day, offset, now)) = clock {
                        (state.day, state.offset, state.now) = (day, offset, now);
                    }
                    if installed != state.installed {
                        state.installed = installed;
                        ensure_running(state);
                    }
                    // Lituus's file, with yesterday's sunset: today's.
                    if state.installed && state.written.as_ref().is_some_and(|text| *text != conf(state)) {
                        apply(&w, state, false);
                    }
                    // A schedule waiting: what hyprsunset shows, asked on a thread, then settled.
                    if state.pending {
                        let w = w.clone();
                        std::thread::spawn(move || {
                            let running = asked();
                            let w2 = w.clone();
                            let _ = w.upgrade_in_event_loop(move |_| {
                                update(&w2, |app| settle(&w2, &mut app.night, running));
                            });
                        });
                    }
                })
            });
            if posted.is_err() {
                return;
            }
        }
    });
}

fn s(text: impl AsRef<str>) -> SharedString {
    SharedString::from(text.as_ref())
}

/// "On until 07:12" or "Off until 17:07"; "Off" with no schedule.
fn now_line(state: &State) -> String {
    if state.enabled && state.always {
        return "On".to_owned();
    }
    match (state.enabled, times(state)) {
        (true, Some(t)) if on_at(state.now, t) => format!("On until {}", time_text(t.1)),
        (true, Some(t)) => format!("Off until {}", time_text(t.0)),
        _ => "Off".to_owned(),
    }
}

/// The sidebar's status.
pub fn status(state: &State) -> String {
    match (state.installed, state.enabled) {
        (false, _) => "missing".into(),
        (true, true) if is_on(state) => "on".into(),
        (true, true) => "scheduled".into(),
        _ => "off".into(),
    }
}

/// The quick popup's row: shown, on, and its second line.
pub fn quick(state: &State) -> (bool, bool, String) {
    let line = if state.enabled { format!("{} · {} K", now_line(state), state.temperature) } else { "Off".into() };
    (state.installed, state.enabled, line)
}

fn city(zone: &str) -> String {
    zone.rsplit('/').next().unwrap_or(zone).replace('_', " ")
}

pub fn view(state: &State) -> NightView {
    let mut v = NightView { error: s(state.error.clone().unwrap_or_default()), ..NightView::default() };
    if !state.installed {
        v.state = s("missing");
        v.sub = s("Off");
        v.hero = HeroInfo {
            icon: s("moon"),
            name: s("Not available"),
            meta: s("hyprsunset isn't installed"),
            tone: s("warn"),
            detail: s(""),
        };
        v.install = s(&state.install);
        return v;
    }
    v.state = s("list");
    v.enabled = state.enabled;
    let t = times(state);
    v.sub = s(match (state.schedule.as_str(), t) {
        _ if state.always => "All the time".to_owned(),
        ("sun", Some((on, off))) => {
            format!("{} · sunset {}, sunrise {}", city(&state.zone), time_text(on), time_text(off))
        }
        ("times", Some((on, off))) => format!("From {} to {}", time_text(on), time_text(off)),
        _ => "No schedule".to_owned(),
    });
    v.hero = HeroInfo {
        icon: s("moon"),
        name: s(now_line(state)),
        meta: s(format!(
            "{} K · {}",
            state.temperature,
            if state.always {
                "all the time"
            } else if state.schedule == "times" {
                "at fixed times"
            } else {
                "from sunset to sunrise"
            }
        )),
        tone: s(if is_on(state) { "accent" } else { "muted" }),
        detail: s("hyprsunset"),
    };
    v.temperature = state.temperature as f32;
    // Turn on's choice: Sunset to sunrise, Fixed times, All the time.
    v.schedule = if state.always { 2 } else { i32::from(state.schedule == "times") };
    v.location = i32::from(state.manual.is_some());
    let (lat, lon) = state.manual.or(state.zone_place).unwrap_or((0.0, 0.0));
    let ns = if lat >= 0.0 { "N" } else { "S" };
    let ew = if lon >= 0.0 { "E" } else { "W" };
    v.location_text = s(if state.manual.is_some() || state.zone_place.is_some() {
        format!(
            "{}{:.1}° {ns}, {:.1}° {ew}",
            if state.manual.is_some() { String::new() } else { format!("{} · ", state.zone) },
            lat.abs(),
            lon.abs()
        )
    } else {
        "The time zone has no place; type one".to_owned()
    });
    v.latitude = s(format!("{lat:.2}"));
    v.longitude = s(format!("{lon:.2}"));
    v.refused = state.refused;
    v.from = s(&state.from);
    v.to = s(&state.to);
    v.tonight = s(t
        .map(|(on, off)| format!("{} to {}", time_text(on), time_text(off)))
        .unwrap_or_else(|| "The sun doesn't set or rise today".into()));
    v
}

pub fn render(window: &MainWindow, state: &State) {
    window.set_night(view(state));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zones_place_is_read_from_its_coordinates() {
        let (lat, lon) = coordinates("+4723+00832").unwrap();
        assert!((lat - 47.383).abs() < 0.01 && (lon - 8.533).abs() < 0.01, "{lat} {lon}");
        let (lat, lon) = coordinates("-3352+15113").unwrap();
        assert!(lat < -33.8 && lon > 151.2);
        let (lat, lon) = coordinates("+404251-0740023").unwrap();
        assert!((lat - 40.714).abs() < 0.01 && (lon + 74.006).abs() < 0.01, "{lat} {lon}");
        assert_eq!(parse_offset("+0100"), 60);
        assert_eq!(parse_offset("-0430"), -270);
    }

    #[test]
    fn zurichs_sun_sets_and_rises_when_it_does() {
        // 25 November (day 329), CET: about 16:40 and 07:40.
        let (set, rise) = sun(47.38, 8.54, 329, 60).unwrap();
        assert!((990..=1010).contains(&set), "sunset {}", time_text(set));
        assert!((450..=470).contains(&rise), "sunrise {}", time_text(rise));
        // 21 June, CEST: about 21:25 and 05:30.
        let (set, rise) = sun(47.38, 8.54, 172, 120).unwrap();
        assert!((1275..=1295).contains(&set), "sunset {}", time_text(set));
        assert!((320..=340).contains(&rise), "sunrise {}", time_text(rise));
        // Tromsø in December: no sunrise.
        assert_eq!(sun(69.65, 18.96, 355, 60), None);
    }

    #[test]
    fn the_schedule_is_two_profiles_and_on_goes_over_midnight() {
        let state = State {
            installed: true,
            enabled: true,
            temperature: 4000,
            schedule: "times".into(),
            from: "21:00".into(),
            to: "07:00".into(),
            now: 23 * 60,
            ..State::default()
        };
        let text = conf(&state);
        assert!(text.starts_with(HEADER));
        assert!(
            text.contains("time = 21:00\n    temperature = 4000") && text.contains("time = 07:00\n    identity = true"),
            "{text}"
        );
        assert!(is_on(&state));
        assert!(!is_on(&State {
            now: 12 * 60,
            ..State {
                schedule: "times".into(),
                from: "21:00".into(),
                to: "07:00".into(),
                enabled: true,
                ..State::default()
            }
        }));
        assert_eq!(now_line(&state), "On until 07:00");
        // Off: the screens as they are all day (with no profile hyprsunset would start at 6000 K).
        let off = conf(&State { enabled: false, ..state });
        assert_eq!(off.matches("profile").count(), 1, "{off}");
        assert!(off.contains("time = 00:00\n    identity = true"), "{off}");
        // All the time: one profile with the temperature, and on at noon too.
        let always = State {
            enabled: true,
            always: true,
            temperature: 4000,
            schedule: "times".into(),
            from: "21:00".into(),
            to: "07:00".into(),
            now: 12 * 60,
            ..State::default()
        };
        let text = conf(&always);
        assert_eq!(text.matches("profile").count(), 1, "{text}");
        assert!(text.contains("time = 00:00\n    temperature = 4000"), "{text}");
        assert!(is_on(&always));
        assert_eq!(now_line(&always), "On");
        assert_eq!(quick(&always).2, "On · 4000 K");
    }

    /// hyprsunset is started again only when the moment without it can't be seen; warm before
    /// and after, the colour goes through its IPC.
    #[test]
    fn a_change_restarts_hyprsunset_only_when_it_cant_be_seen() {
        // Not running: started.
        assert_eq!(reach(false, Some(Some(4000)), Some(4000)), Reach::Start);
        // On at night: the screens are without it, and warm up once it's started again.
        assert_eq!(reach(true, Some(None), Some(4000)), Reach::Restart);
        // Off: they're about to be without it.
        assert_eq!(reach(true, Some(Some(4000)), None), Reach::Restart);
        assert_eq!(reach(true, None, None), Reach::Restart);
        // Warm before and after (All the time at night, another temperature, unknown before):
        // told, and started again later.
        assert_eq!(reach(true, Some(Some(4000)), Some(4000)), Reach::Tell);
        assert_eq!(reach(true, Some(Some(4000)), Some(3000)), Reach::Tell);
        assert_eq!(reach(true, None, Some(4000)), Reach::Tell);
    }

    #[test]
    fn only_lituuss_own_file_is_read_back_and_the_install_line_follows_the_distribution() {
        let state = State {
            enabled: true,
            temperature: 4000,
            schedule: "times".into(),
            from: "21:00".into(),
            to: "07:00".into(),
            ..State::default()
        };
        assert!(ours(&conf(&state)));
        assert!(!ours("profile {\n    time = 20:00\n}\n"));
        assert_eq!(
            install_command("NAME=\"EndeavourOS\"\nID=endeavouros\nID_LIKE=arch\n"),
            "sudo pacman -S hyprsunset"
        );
        assert_eq!(install_command("ID=ubuntu\nID_LIKE=debian\n"), "sudo apt install hyprsunset");
        assert_eq!(install_command("ID=gentoo\n"), "");
    }

    #[test]
    fn the_view_says_what_it_does_now() {
        let mut state = State {
            installed: true,
            enabled: true,
            temperature: 4000,
            schedule: "sun".into(),
            zone: "Europe/Zurich".into(),
            zone_place: Some((47.38, 8.54)),
            day: 329,
            offset: 60,
            now: 12 * 60,
            ..State::default()
        };
        let v = view(&state);
        assert!(v.hero.name.starts_with("Off until 16:"), "{}", v.hero.name);
        assert!(v.sub.starts_with("Zurich · sunset 16:"), "{}", v.sub);
        assert_eq!(status(&state), "scheduled");
        state.installed = false;
        assert_eq!(view(&state).state, "missing");
        assert!(!quick(&state).0);
    }
}
