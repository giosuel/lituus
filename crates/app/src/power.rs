//! The Power page (D-57): the profile (power-profiles-daemon), the batteries (UPower) and how
//! logind handles the lid and the power key, read by a thread on the system bus; the idle times
//! (hypridle.conf, read from anyone's file and written once it's Lituus's); and what the lid and
//! the power button do, as Hyprland binds in Lituus's own file (`lituus --lid` decides for the
//! lid, as it depends on the battery and the screens at that moment).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use futures_util::{FutureExt, StreamExt};
use lituus_power::hypridle::{self, Idle};
use lituus_power::{BatteryState, Client, DeviceKind, Snapshot};
use slint::SharedString;

use crate::{HeroInfo, MainWindow, PowerDevice, PowerView, model, update};

/// What the page asks the power services for.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Profile(String),
    ChargeLimit(String, bool),
}

#[derive(Default)]
pub struct State {
    pub snapshot: Option<Snapshot>,
    /// The system bus couldn't be reached.
    pub unreachable: bool,
    pub error: Option<String>,
    /// hypridle.conf as last read, and whether there's one.
    pub idle: Idle,
    pub idle_file: bool,
    pub hypridle: bool,
    pub backlight: bool,
    /// The lock screens installed, of those Lituus knows.
    pub lockers: Vec<String>,
    /// The confirmation before Lituus takes over hypridle.conf is shown.
    pub confirm_takeover: bool,
    /// The lid and the power button, as chosen here (config.toml's [power]); empty: not chosen,
    /// so logind's own handling is shown and no bind is written.
    pub lid_battery: String,
    pub lid_plugged: String,
    pub lid_external: String,
    pub power_key: String,
    commands: Option<Sender<Command>>,
}

/// The idle times offered, in minutes; a time from someone's file that isn't one is added.
const MINUTES: [u32; 12] = [1, 2, 3, 5, 10, 15, 20, 30, 45, 60, 90, 120];
/// The lid's choices, and the power button's.
const LID: [&str; 3] = ["suspend", "lock", "nothing"];
const KEY: [&str; 3] = ["suspend", "poweroff", "nothing"];
/// Lituus's section of its Hyprland file.
const SECTION: &str = "Power";

pub fn idle_path() -> PathBuf {
    lituus_hypr::config_dir().join("hypridle.conf")
}

/// Reads what isn't on the bus: hypridle.conf, what's installed, the backlight, and the choices
/// in config.toml.
pub fn read_files(state: &mut State, config: &crate::config::Config) {
    let text = std::fs::read_to_string(idle_path());
    state.idle_file = text.is_ok();
    state.idle =
        text.map(|t| hypridle::parse(&t)).unwrap_or_else(|_| Idle { locker: "hyprlock".into(), ..Idle::default() });
    state.hypridle = installed("hypridle");
    state.lockers = hypridle::LOCKERS.iter().filter(|l| installed(l)).map(|l| (*l).to_owned()).collect();
    state.backlight = std::fs::read_dir("/sys/class/backlight").is_ok_and(|mut d| d.next().is_some());
    state.lid_battery = config.power.lid_battery.clone();
    state.lid_plugged = config.power.lid_plugged.clone();
    state.lid_external = config.power.lid_external.clone();
    state.power_key = config.power.power_key.clone();
}

fn installed(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file()))
}

/// Starts the thread that reads the power services, again whenever they change.
pub fn spawn(weak: slint::Weak<MainWindow>) {
    let (tx, rx) = channel();
    update(&weak, |app| app.power.commands = Some(tx));
    std::thread::Builder::new()
        .name("power".into())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            runtime.block_on(run(weak, rx));
        })
        .ok();
}

fn post(weak: &slint::Weak<MainWindow>, change: impl FnOnce(&mut State) + Send + 'static) {
    let weak2 = weak.clone();
    let _ = weak.upgrade_in_event_loop(move |_| update(&weak2, |app| change(&mut app.power)));
}

async fn run(weak: slint::Weak<MainWindow>, rx: Receiver<Command>) {
    let Ok(client) = Client::system().await else {
        post(&weak, |state| state.unreachable = true);
        return;
    };
    let Ok(changes) = client.changes().await else { return };
    let mut changes = Box::pin(changes);
    // Commands come from the window's thread; a small tokio channel forwards them here.
    let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(c) = rx.recv() {
            if tx.send(c).is_err() {
                return;
            }
        }
    });
    loop {
        let snapshot = client.snapshot().await;
        post(&weak, move |state| state.snapshot = Some(snapshot));
        tokio::select! {
            change = changes.next() => {
                if change.is_none() {
                    return;
                }
                // A burst of changes (a battery's every property) is read once.
                tokio::time::sleep(Duration::from_millis(100)).await;
                while let Some(Some(())) = changes.next().now_or_never() {}
            }
            command = commands.recv() => {
                let Some(command) = command else { return };
                let result = match &command {
                    Command::Profile(p) => client.set_profile(p).await,
                    Command::ChargeLimit(path, on) => client.set_charge_limit(path, *on).await,
                };
                let error = result.err().map(|e| format!("Couldn't change that: {e}"));
                post(&weak, move |state| state.error = error);
            }
        }
    }
}

fn send(state: &State, command: Command) {
    if let Some(tx) = &state.commands {
        let _ = tx.send(command);
    }
}

/// What a click on the page does: (verb, value). Changes to the files are made here.
pub fn act(state: &mut State, verb: &str, value: i32) {
    match verb {
        "dismiss" => state.error = None,
        "profile" => {
            if let Some(p) =
                state.snapshot.as_ref().and_then(|s| s.profiles.as_ref()).and_then(|p| p.available.get(value as usize))
            {
                send(state, Command::Profile(p.clone()));
            }
        }
        "charge-limit" => {
            if let Some(b) = state.snapshot.as_ref().and_then(|s| s.battery.as_ref())
                && let Some(limit) = b.charge_limit
            {
                send(state, Command::ChargeLimit(b.path.clone(), !limit.enabled));
            }
        }
        "dim" | "lock" | "screens-off" | "suspend" => {
            let choices = time_choices(current(&state.idle, verb));
            let minutes = choices.get(value as usize).copied().flatten();
            let mut idle = state.idle.clone();
            *slot(&mut idle, verb) = minutes.map(|m| m * 60);
            write_idle(state, idle);
        }
        "lock-before-sleep" => {
            let idle = Idle { lock_before_sleep: !state.idle.lock_before_sleep, ..state.idle.clone() };
            write_idle(state, idle);
        }
        "locker" => {
            if let Some(l) = state.lockers.get(value as usize) {
                let idle = Idle { locker: l.clone(), ..state.idle.clone() };
                write_idle(state, idle);
            }
        }
        "takeover" => state.confirm_takeover = true,
        "takeover-cancel" => state.confirm_takeover = false,
        "takeover-confirm" => {
            state.confirm_takeover = false;
            let path = idle_path();
            let backup = path.with_extension("conf.bak");
            match std::fs::copy(&path, &backup) {
                Ok(_) => {
                    let idle = Idle { ours: true, ..state.idle.clone() };
                    write_idle(state, idle);
                }
                Err(e) => state.error = Some(format!("Couldn't keep a copy of hypridle.conf: {e}")),
            }
        }
        "lid-battery" | "lid-plugged" | "lid-external" | "power-key" => {
            let (choices, field): (&[&str], &mut String) = match verb {
                "lid-battery" => (&LID, &mut state.lid_battery),
                "lid-plugged" => (&LID, &mut state.lid_plugged),
                "lid-external" => (&LID, &mut state.lid_external),
                _ => (&KEY, &mut state.power_key),
            };
            let Some(choice) = choices.get(value as usize) else { return };
            *field = (*choice).to_owned();
            // The others start from what logind does now, so choosing one doesn't change them.
            let logind = state.snapshot.as_ref().and_then(|s| s.logind.clone()).unwrap_or_default();
            for (f, now) in [
                (&mut state.lid_battery, lid_choice(&logind.lid)),
                (&mut state.lid_plugged, lid_choice(&logind.lid_external_power)),
                (&mut state.lid_external, lid_choice(&logind.lid_docked)),
                (&mut state.power_key, key_choice(&logind.power_key)),
            ] {
                if f.is_empty() {
                    *f = now.to_owned();
                }
            }
            let path = crate::config::path();
            for (key, value) in [
                ("lid_battery", &state.lid_battery),
                ("lid_plugged", &state.lid_plugged),
                ("lid_external", &state.lid_external),
                ("power_key", &state.power_key),
            ] {
                let _ = crate::config::set(&path, "power", key, value.as_str());
            }
            let file = crate::setup::file();
            let text = binds(state, file.lua);
            if let Err(e) = file.write_section(SECTION, &text) {
                state.error = Some(format!("Couldn't save the change: {e}."));
            }
        }
        _ => {}
    }
}

/// Writes Lituus's hypridle.conf (only when the file is Lituus's, or there's none) and has
/// hypridle read it again.
fn write_idle(state: &mut State, idle: Idle) {
    if state.idle_file && !state.idle.ours && !idle.ours {
        return;
    }
    let path = idle_path();
    let text = hypridle::write(&idle, state.backlight);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&path, &text) {
        Ok(()) => {
            state.idle = hypridle::parse(&text);
            state.idle_file = true;
            restart_hypridle();
        }
        Err(e) => state.error = Some(format!("Couldn't save hypridle.conf: {e}")),
    }
}

/// hypridle reads its file once, when it starts: so it's started again, through its systemd unit
/// when it runs as one, else by Hyprland.
fn restart_hypridle() {
    std::thread::spawn(|| {
        let unit = std::process::Command::new("systemctl")
            .args(["--user", "--quiet", "is-active", "hypridle.service"])
            .status()
            .is_ok_and(|s| s.success());
        if unit {
            let _ = std::process::Command::new("systemctl").args(["--user", "restart", "hypridle.service"]).status();
            return;
        }
        for pid in pids("hypridle") {
            let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
        }
        if let Ok(hypr) = lituus_hypr::Hyprland::from_env() {
            let _ = hypr.request("dispatch exec hypridle");
        }
    });
}

/// This user's processes running `name`.
fn pids(name: &str) -> Vec<u32> {
    let me = std::fs::metadata("/proc/self").map(|m| std::os::unix::fs::MetadataExt::uid(&m)).ok();
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| {
            let pid: u32 = e.file_name().to_str()?.parse().ok()?;
            let comm = std::fs::read_to_string(e.path().join("comm")).ok()?;
            let uid = std::fs::metadata(e.path()).map(|m| std::os::unix::fs::MetadataExt::uid(&m)).ok();
            (comm.trim() == name && uid == me).then_some(pid)
        })
        .collect()
}

/// The lid's and the power button's binds, for Lituus's file.
pub fn binds(state: &State, lua: bool) -> String {
    let mut lines = vec!["-- The lid and the power button (Power page).".to_owned()];
    if !lua {
        lines[0] = "# The lid and the power button (Power page).".to_owned();
    }
    let mut bind = |keys: &str, command: &str| {
        lines.push(if lua {
            format!(
                "hl.bind({}, hl.dsp.exec_cmd({}), {{ locked = true }})",
                lituus_hypr::lua_string(keys),
                lituus_hypr::lua_string(command)
            )
        } else {
            format!("bindl = , {keys}, exec, {command}")
        });
    };
    if [&state.lid_battery, &state.lid_plugged, &state.lid_external]
        .iter()
        .any(|c| !c.is_empty() && c.as_str() != "nothing")
    {
        bind("switch:on:Lid Switch", "lituus --lid");
    }
    match state.power_key.as_str() {
        "suspend" => bind("XF86PowerOff", "systemctl suspend"),
        "poweroff" => bind("XF86PowerOff", "systemctl poweroff"),
        _ => {}
    }
    lines.join("\n") + "\n"
}

fn lid_choice(logind: &str) -> &'static str {
    match logind {
        "lock" => "lock",
        "ignore" | "" => "nothing",
        _ => "suspend",
    }
}

fn key_choice(logind: &str) -> &'static str {
    match logind {
        "poweroff" | "reboot" | "halt" => "poweroff",
        "ignore" | "" => "nothing",
        _ => "suspend",
    }
}

fn slot<'a>(idle: &'a mut Idle, verb: &str) -> &'a mut Option<u32> {
    match verb {
        "dim" => &mut idle.dim,
        "lock" => &mut idle.lock,
        "screens-off" => &mut idle.screens_off,
        _ => &mut idle.suspend,
    }
}

fn current(idle: &Idle, verb: &str) -> Option<u32> {
    match verb {
        "dim" => idle.dim,
        "lock" => idle.lock,
        "screens-off" => idle.screens_off,
        _ => idle.suspend,
    }
}

/// A row's choices in minutes (`None`: never): the usual ones, and the file's own when it isn't
/// one of them.
fn time_choices(now: Option<u32>) -> Vec<Option<u32>> {
    let mut minutes: Vec<u32> = MINUTES.to_vec();
    if let Some(s) = now
        && s % 60 == 0
        && !minutes.contains(&(s / 60))
    {
        minutes.push(s / 60);
        minutes.sort_unstable();
    }
    std::iter::once(None).chain(minutes.into_iter().map(Some)).collect()
}

fn time_text(seconds: Option<u32>) -> String {
    match seconds {
        None => "Never".to_owned(),
        Some(s) if s % 60 == 0 => format!("After {} min", s / 60),
        Some(s) => format!("After {s} s"),
    }
}

fn s(text: impl AsRef<str>) -> SharedString {
    SharedString::from(text.as_ref())
}

fn duration(seconds: i64) -> String {
    let minutes = (seconds + 30) / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn profile_name(p: &str) -> &str {
    match p {
        "performance" => "Performance",
        "balanced" => "Balanced",
        "power-saver" => "Power saver",
        other => other,
    }
}

/// The sidebar's status: the battery, or the profile.
pub fn status(state: &State) -> String {
    let Some(snap) = &state.snapshot else { return String::new() };
    if let Some(b) = &snap.battery {
        return format!("{:.0}%", b.percent);
    }
    snap.profiles.as_ref().map(|p| profile_name(&p.active).to_owned()).unwrap_or_default()
}

fn device_icon(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Mouse | DeviceKind::Touchpad => "mouse",
        DeviceKind::Keyboard => "keyboard",
        DeviceKind::Headset | DeviceKind::Headphones | DeviceKind::Speakers => "headphones",
        _ => "battery",
    }
}

pub fn view(state: &State) -> PowerView {
    let mut v = PowerView { error: s(state.error.clone().unwrap_or_default()), ..PowerView::default() };
    let snap = state.snapshot.clone().unwrap_or_default();
    v.state = s(if state.snapshot.is_some() || state.unreachable { "list" } else { "loading" });
    // The hero: the battery on a laptop, else the mains.
    let profile = snap.profiles.as_ref().map(|p| profile_name(&p.active).to_owned());
    let idle_line = state.idle.dim.or(state.idle.screens_off).map(|t| {
        let what = if state.idle.dim.is_some() && state.backlight { "the screens dim" } else { "the screens turn off" };
        format!("{what} after {}", time_text(Some(t)).trim_start_matches("After ").to_owned())
    });
    if let Some(b) = &snap.battery {
        let (line, tone) = match b.state {
            BatteryState::Charging if b.time_to_full > 0 => {
                (format!("Charging · {} to full", duration(b.time_to_full)), "accent")
            }
            BatteryState::Charging => ("Charging".to_owned(), "accent"),
            BatteryState::Discharging if b.time_to_empty > 0 => {
                (format!("On battery · {} left", duration(b.time_to_empty)), "muted")
            }
            BatteryState::Full => ("Fully charged".to_owned(), "accent"),
            BatteryState::PendingCharge => ("Plugged in, not charging".to_owned(), "muted"),
            _ => ("On battery".to_owned(), "muted"),
        };
        v.hero = HeroInfo {
            icon: s("battery"),
            name: s(format!("{:.0} %", b.percent)),
            meta: s(line),
            tone: s(tone),
            detail: s(format!("{} · health {:.0} %", b.name, b.health)),
        };
        v.sub = s(format!(
            "{} profile{}",
            profile.clone().unwrap_or_else(|| "No".into()),
            idle_line.map(|l| format!(" · {l}")).unwrap_or_default()
        ));
        v.has_battery = true;
        v.health = s(format!("{:.0} % · {:.1} of {:.0} Wh", b.health, b.energy_full, b.energy_full_design));
        v.cycles = s(b.cycles.map(|c| c.to_string()).unwrap_or_else(|| "Not known".into()));
        v.has_charge_limit = b.charge_limit.is_some();
        v.charge_limit = b.charge_limit.is_some_and(|l| l.enabled);
        v.charge_limit_label = s(format!("Limit charging to {} %", b.charge_limit.map(|l| l.end).unwrap_or(80)));
    } else {
        v.hero = HeroInfo {
            icon: s("plug"),
            name: s("Plugged in"),
            meta: s("No battery · desktop"),
            tone: s("accent"),
            detail: s(if snap.profiles.is_some() { "power-profiles-daemon" } else { "" }),
        };
        v.sub = s(format!(
            "{}{}",
            profile.as_ref().map(|p| format!("{p} profile")).unwrap_or_else(|| "Plugged in".into()),
            idle_line.map(|l| format!(" · {l}")).unwrap_or_default()
        ));
    }
    if let Some(p) = &snap.profiles {
        v.profiles = model(p.available.iter().map(|a| s(profile_name(a))).collect());
        v.profile = p.available.iter().position(|a| *a == p.active).map_or(-1, |i| i as i32);
        v.degraded = s(match p.degraded.as_str() {
            "" => "",
            "lap-detected" => "Performance is held back: the laptop is on a lap",
            "high-operating-temperature" => "Performance is held back: the computer is hot",
            _ => "Performance is held back",
        });
    }
    // When idle.
    let idle = &state.idle;
    v.idle_editable = !state.idle_file || idle.ours;
    v.hypridle = state.hypridle;
    v.backlight = state.backlight;
    for (verb, now, names, at) in [
        ("dim", idle.dim, &mut v.dim_choices, &mut v.dim),
        ("lock", idle.lock, &mut v.lock_choices, &mut v.lock),
        ("screens-off", idle.screens_off, &mut v.screens_off_choices, &mut v.screens_off),
        ("suspend", idle.suspend, &mut v.suspend_choices, &mut v.suspend),
    ] {
        let _ = verb;
        let choices = time_choices(now);
        *names = model(choices.iter().map(|c| s(time_text(c.map(|m| m * 60)))).collect());
        *at = choices.iter().position(|c| c.map(|m| m * 60) == now).map_or(0, |i| i as i32);
    }
    v.dim_text = s(time_text(idle.dim));
    v.lock_text = s(time_text(idle.lock));
    v.screens_off_text = s(time_text(idle.screens_off));
    v.suspend_text = s(time_text(idle.suspend));
    v.lock_before_sleep = idle.lock_before_sleep;
    v.lockers = model(state.lockers.iter().map(s).collect());
    v.locker = state.lockers.iter().position(|l| *l == idle.locker).map_or(-1, |i| i as i32);
    v.confirm_takeover = state.confirm_takeover;
    // Batteries of the devices.
    v.devices = model(
        snap.devices
            .iter()
            .map(|d| PowerDevice {
                icon: s(device_icon(d.kind)),
                name: s(&d.name),
                sub: s(match (d.state, d.link.as_str()) {
                    (BatteryState::Charging, _) => "Charging",
                    (_, "") => "",
                    (_, link) => link,
                }),
                percent: (d.percent / 100.0) as f32,
            })
            .collect(),
    );
    // The lid and the power button: as chosen here, else as logind does it.
    let logind = snap.logind.clone().unwrap_or_default();
    let pick = |chosen: &str, logind_now: &'static str, choices: &[&str]| {
        let c = if chosen.is_empty() { logind_now } else { chosen };
        choices.iter().position(|x| *x == c).map_or(0, |i| i as i32)
    };
    v.has_lid = snap.lid;
    v.lid_battery = pick(&state.lid_battery, lid_choice(&logind.lid), &LID);
    v.lid_plugged = pick(&state.lid_plugged, lid_choice(&logind.lid_external_power), &LID);
    v.lid_external = pick(&state.lid_external, lid_choice(&logind.lid_docked), &LID);
    v.power_key = pick(&state.power_key, key_choice(&logind.power_key), &KEY);
    let chosen_lid = !state.lid_battery.is_empty();
    let chosen_key = !state.power_key.is_empty();
    v.lid_note = s(if chosen_lid && snap.logind.is_some() && !logind.leaves_lid() {
        "systemd-logind still handles the lid itself. Add HandleLidSwitch=ignore, HandleLidSwitchExternalPower=ignore and \
         HandleLidSwitchDocked=ignore to /etc/systemd/logind.conf.d/lituus.conf and restart it."
    } else {
        "Lituus writes these as Hyprland binds on the lid switch, in its own file."
    });
    v.key_note = s(if chosen_key && snap.logind.is_some() && !logind.leaves_power_key() {
        "systemd-logind still handles the key itself. Add HandlePowerKey=ignore to /etc/systemd/logind.conf.d/lituus.conf and restart it."
    } else {
        "A Hyprland bind on the power key, in Lituus's own file."
    });
    v
}

pub fn render(window: &MainWindow, state: &State) {
    window.set_power(view(state));
}

/// `lituus --lid`, from the lid's Hyprland bind: does what the Power page chose for now: on
/// battery, plugged in, or with another screen connected.
pub fn lid_closed() {
    let config = crate::config::load(&crate::config::path());
    let on_battery = tokio::runtime::Builder::new_current_thread().enable_all().build().is_ok_and(|rt| {
        rt.block_on(async {
            match Client::system().await {
                Ok(client) => client.snapshot().await.on_battery,
                Err(_) => false,
            }
        })
    });
    let external = lituus_hypr::Hyprland::from_env()
        .and_then(|h| h.monitors())
        .is_ok_and(|ms| ms.iter().any(|m| !is_internal(&m.name)));
    let choice = if external {
        &config.power.lid_external
    } else if on_battery {
        &config.power.lid_battery
    } else {
        &config.power.lid_plugged
    };
    let command: &[&str] = match choice.as_str() {
        "suspend" => &["systemctl", "suspend"],
        "lock" => &["loginctl", "lock-session"],
        _ => return,
    };
    let _ = std::process::Command::new(command[0]).args(&command[1..]).status();
}

/// A laptop's own panel: eDP, LVDS or DSI.
fn is_internal(name: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lituus_power::{Battery, ChargeLimit, DeviceBattery, Logind, Profiles};
    use slint::Model;

    fn laptop() -> Snapshot {
        Snapshot {
            profiles: Some(Profiles {
                active: "balanced".into(),
                available: vec!["power-saver".into(), "balanced".into(), "performance".into()],
                degraded: String::new(),
            }),
            battery: Some(Battery {
                path: "/bat".into(),
                name: "BAT0".into(),
                percent: 64.0,
                state: BatteryState::Charging,
                time_to_full: 7800,
                energy_full: 52.1,
                energy_full_design: 57.0,
                health: 91.4,
                cycles: Some(312),
                charge_limit: Some(ChargeLimit { enabled: true, end: 80 }),
                ..Battery::default()
            }),
            devices: vec![DeviceBattery {
                name: "MX Master 3S".into(),
                kind: DeviceKind::Mouse,
                percent: 80.0,
                link: "Logitech receiver".into(),
                ..DeviceBattery::default()
            }],
            lid: true,
            logind: Some(Logind {
                lid: "suspend".into(),
                lid_external_power: "suspend".into(),
                lid_docked: "ignore".into(),
                power_key: "poweroff".into(),
            }),
            ..Snapshot::default()
        }
    }

    #[test]
    fn a_laptop_shows_its_battery_profile_devices_and_lid() {
        let state = State { snapshot: Some(laptop()), ..State::default() };
        let v = view(&state);
        assert_eq!((v.hero.name.as_str(), v.hero.meta.as_str()), ("64 %", "Charging · 2 h 10 min to full"));
        assert_eq!(v.hero.detail, "BAT0 · health 91 %");
        assert_eq!(v.profile, 1);
        assert_eq!(
            v.profiles.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            ["Power saver", "Balanced", "Performance"]
        );
        assert_eq!(v.health, "91 % · 52.1 of 57 Wh");
        assert!(v.has_charge_limit && v.charge_limit);
        assert_eq!(v.charge_limit_label, "Limit charging to 80 %");
        assert_eq!(v.devices.row_count(), 1);
        // Not chosen here: logind's handling is shown.
        assert_eq!((v.lid_battery, v.lid_plugged, v.lid_external, v.power_key), (0, 0, 2, 1));
        assert_eq!(status(&state), "64%");
    }

    #[test]
    fn a_desktop_shows_the_mains_and_the_profile() {
        let mut snap = laptop();
        snap.battery = None;
        snap.lid = false;
        let state = State { snapshot: Some(snap), ..State::default() };
        let v = view(&state);
        assert_eq!((v.hero.name.as_str(), v.hero.meta.as_str()), ("Plugged in", "No battery · desktop"));
        assert!(!v.has_battery && !v.has_lid);
        assert_eq!(status(&state), "Balanced");
    }

    #[test]
    fn idle_times_offer_the_files_own_and_say_never() {
        assert_eq!(time_choices(None)[0], None);
        assert!(time_choices(Some(660)).contains(&Some(11)), "11 min from a dotfiles' file");
        assert_eq!(time_text(Some(660)), "After 11 min");
        assert_eq!(time_text(Some(150)), "After 150 s");
        assert_eq!(time_text(None), "Never");
        let idle = Idle { lock: Some(600), screens_off: Some(660), ..Idle::default() };
        let state = State { idle, idle_file: true, ..State::default() };
        let v = view(&state);
        assert!(!v.idle_editable, "someone else's file is read only");
        assert_eq!(v.screens_off_text, "After 11 min");
    }

    #[test]
    fn binds_go_to_lituus_file_in_its_language() {
        let state = State {
            lid_battery: "suspend".into(),
            lid_plugged: "lock".into(),
            lid_external: "nothing".into(),
            power_key: "suspend".into(),
            ..State::default()
        };
        let lua = binds(&state, true);
        assert!(
            lua.contains(r#"hl.bind("switch:on:Lid Switch", hl.dsp.exec_cmd("lituus --lid"), { locked = true })"#),
            "{lua}"
        );
        assert!(
            lua.contains(r#"hl.bind("XF86PowerOff", hl.dsp.exec_cmd("systemctl suspend"), { locked = true })"#),
            "{lua}"
        );
        let conf = binds(&state, false);
        assert!(conf.contains("bindl = , switch:on:Lid Switch, exec, lituus --lid"), "{conf}");
        assert!(conf.contains("bindl = , XF86PowerOff, exec, systemctl suspend"), "{conf}");
        // Nothing on every case: no bind at all.
        let none = State {
            lid_battery: "nothing".into(),
            lid_plugged: "nothing".into(),
            lid_external: "nothing".into(),
            power_key: "nothing".into(),
            ..State::default()
        };
        assert_eq!(binds(&none, true).lines().count(), 1);
    }

    #[test]
    fn the_internal_panel_is_known_by_its_name() {
        assert!(is_internal("eDP-1") && is_internal("LVDS-1") && !is_internal("DP-3") && !is_internal("HDMI-A-2"));
    }
}
