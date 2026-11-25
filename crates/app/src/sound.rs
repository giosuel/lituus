//! The Sound page: a thread talks to pipewire-pulse (lituus-sound) and hands the window each new
//! state; the page sends its changes back as commands. Volumes from a dragged slider come fast,
//! so the thread only keeps the last one for each slider.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use lituus_sound::{Session, Snapshot, Stream, Volume};
use lituus_xdg::{Dirs, desktop};
use slint::{Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::{APP, HeroInfo, MainWindow, SoundApp, SoundCard, SoundView, model, update};

/// What the page asks the sound server for.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    DefaultSink(String),
    DefaultSource(String),
    SinkVolume(u32, Volume),
    SourceVolume(u32, Volume),
    StreamVolume(u32, Volume),
    SinkMute(u32, bool),
    SourceMute(u32, bool),
    StreamMute(u32, bool),
    Move(u32, u32),
    SinkPort(u32, String),
    Profile(String, String),
}

impl Command {
    /// Commands that only the last of in a row matters: the same slider, dragged.
    fn replaces(&self, other: &Command) -> bool {
        use Command::*;
        matches!(
            (self, other),
            (SinkVolume(a, _), SinkVolume(b, _)) | (SourceVolume(a, _), SourceVolume(b, _)) | (StreamVolume(a, _), StreamVolume(b, _))
            if a == b
        )
    }
}

enum Message {
    Changed,
    Run(Command),
    /// The watcher of the connection with this number stopped: the server went away.
    Lost(u64),
}

#[derive(Default)]
pub struct State {
    pub snapshot: Option<Snapshot>,
    /// pipewire-pulse isn't there.
    pub not_running: bool,
    pub error: Option<String>,
    pub overamplify: bool,
    /// The apps' icons by `icon_key`, drawn on the sound thread: `None` when there's none to find.
    icons: HashMap<String, Option<Image>>,
    commands: Option<Sender<Message>>,
}

/// The apps playing that the quick popup shows; the rest are a link to the page.
pub const QUICK_APPS: usize = 3;

/// The pixels of an app's icon, as the sound thread hands them over.
type Drawn = Option<SharedPixelBuffer<Rgba8Pixel>>;

/// The size the apps' icons are drawn at: twice the page's 24 px, for scaled screens.
const ICON_SIDE: u32 = 48;

/// Finds and draws the apps' icons on the sound thread, each once: looking through the icon
/// themes and drawing an SVG would hold up the window.
struct Icons {
    dirs: Dirs,
    theme: String,
    done: HashSet<String>,
}

impl Icons {
    fn new() -> Icons {
        let dirs = Dirs::from_env();
        let theme = crate::apps::icon_theme(&dirs);
        Icons { dirs, theme, done: HashSet::new() }
    }

    /// The icons of the apps not seen before.
    fn new_ones(&mut self, streams: &[Stream]) -> Vec<(String, Drawn)> {
        let mut out = Vec::new();
        for st in streams {
            let key = icon_key(st);
            if self.done.insert(key.clone()) {
                let file = icon_names(st).iter().find_map(|n| desktop::icon_file(&self.dirs, &self.theme, n));
                out.push((key, file.and_then(|f| crate::apps::draw_icon(&f, ICON_SIDE))));
            }
        }
        out
    }
}

/// The names an app's icon may go by: the one it gives, then its app id, its program, and its
/// name in lower case ("spotify"). Only the first may be a file's path; the others are names in
/// the icon themes, never a way out of them.
fn icon_names(st: &Stream) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let given = st.icon.trim();
    if !given.is_empty() && (!given.contains('/') || given.starts_with('/')) && !given.contains("..") {
        names.push(given.to_owned());
    }
    for n in [st.app_id.trim().to_owned(), st.binary.trim().to_owned(), st.app.trim().to_lowercase()] {
        if !n.is_empty() && !n.contains('/') && n != "." && n != ".." && !names.contains(&n) {
            names.push(n);
        }
    }
    names
}

fn icon_key(st: &Stream) -> String {
    icon_names(st).join("\n")
}

/// Starts the sound thread; it reconnects when the server restarts.
pub fn spawn(weak: slint::Weak<MainWindow>) {
    let (tx, rx) = channel();
    APP.with(|app| app.borrow_mut().sound.commands = Some(tx.clone()));
    std::thread::spawn(move || run(weak, tx, rx));
}

fn post(weak: &slint::Weak<MainWindow>, change: impl FnOnce(&mut State) + Send + 'static) {
    let weak2 = weak.clone();
    let _ = weak.upgrade_in_event_loop(move |_| update(&weak2, |app| change(&mut app.sound)));
}

fn run(weak: slint::Weak<MainWindow>, tx: Sender<Message>, rx: Receiver<Message>) {
    // Each connection's number: a watcher that ends tells which one it watched.
    let mut connection = 0;
    let mut icons = Icons::new();
    loop {
        connection += 1;
        let (mut session, watcher) = match (Session::connect(), Session::connect()) {
            (Ok(s), Ok(w)) => (s, w),
            (Err(lituus_sound::Error::NotRunning), _) | (_, Err(lituus_sound::Error::NotRunning)) => {
                post(&weak, |s| {
                    s.not_running = true;
                    s.snapshot = None;
                });
                // Drop what was asked meanwhile; check again in a while.
                std::thread::sleep(Duration::from_secs(3));
                while rx.try_recv().is_ok() {}
                continue;
            }
            (Err(e), _) | (_, Err(e)) => {
                let error = e.to_string();
                post(&weak, move |s| s.error = Some(error));
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        if connected(&weak, &mut session, watcher, &tx, &rx, connection, &mut icons).is_err() {
            post(&weak, |s| s.snapshot = None);
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

/// One connection's life: a watcher on a thread of its own, and the commands until the
/// connection breaks or the watcher finds the server gone.
fn connected(
    weak: &slint::Weak<MainWindow>,
    session: &mut Session,
    watcher: Session,
    tx: &Sender<Message>,
    rx: &Receiver<Message>,
    connection: u64,
    icons: &mut Icons,
) -> Result<(), lituus_sound::Error> {
    let closer = watcher.closer().ok();
    watch(watcher, tx.clone(), connection);
    let result = serve(weak, session, rx, connection, icons);
    // With the server still there (a reply that didn't come in time, say), the watcher would go
    // on: one more thread and connection on every pass, each change told once more.
    if let Some(closer) = closer {
        closer.close();
    }
    result
}

/// Watches for changes on a thread of its own, and tells when the server is gone.
fn watch(watcher: Session, tx: Sender<Message>, connection: u64) {
    std::thread::spawn(move || {
        let changes = tx.clone();
        let _ = watcher.watch(move || changes.send(Message::Changed).is_ok());
        let _ = tx.send(Message::Lost(connection));
    });
}

/// Answers commands and changes until the connection breaks, or connection number
/// `connection`'s watcher finds the server gone.
fn serve(
    weak: &slint::Weak<MainWindow>,
    session: &mut Session,
    rx: &Receiver<Message>,
    connection: u64,
    icons: &mut Icons,
) -> Result<(), lituus_sound::Error> {
    let mut first = true;
    loop {
        let mut commands: Vec<Command> = Vec::new();
        if !first {
            let Ok(message) = rx.recv() else { return Ok(()) };
            let mut queue = vec![message];
            // Whatever else came meanwhile, after a moment for a burst of events to settle.
            std::thread::sleep(Duration::from_millis(15));
            queue.extend(rx.try_iter());
            for m in queue {
                match m {
                    Message::Run(c) => {
                        commands.retain(|old| !c.replaces(old));
                        commands.push(c);
                    }
                    // The changes asked for meanwhile name the old server's outputs and apps by
                    // its numbers, so they're dropped rather than sent to the new one.
                    Message::Lost(n) if n == connection => {
                        return Err(std::io::Error::from(std::io::ErrorKind::ConnectionReset).into());
                    }
                    Message::Changed | Message::Lost(_) => {}
                }
            }
        }
        first = false;
        if !commands.is_empty() {
            post(weak, |s| s.error = None);
        }
        for command in commands {
            if let Err(e) = execute(session, &command) {
                if matches!(e, lituus_sound::Error::Io(_)) {
                    return Err(e);
                }
                let error = format!("Couldn't change that: {e}");
                post(weak, move |s| s.error = Some(error));
            }
        }
        let snapshot = session.snapshot()?;
        let drawn = icons.new_ones(&snapshot.streams);
        post(weak, move |s| {
            for (key, pixels) in drawn {
                s.icons.insert(key, pixels.map(Image::from_rgba8));
            }
            s.not_running = false;
            s.snapshot = Some(snapshot);
        });
    }
}

fn execute(session: &mut Session, command: &Command) -> Result<(), lituus_sound::Error> {
    match command {
        Command::DefaultSink(name) => session.set_default_sink(name),
        Command::DefaultSource(name) => session.set_default_source(name),
        Command::SinkVolume(i, v) => session.set_sink_volume(*i, v),
        Command::SourceVolume(i, v) => session.set_source_volume(*i, v),
        Command::StreamVolume(i, v) => session.set_stream_volume(*i, v),
        Command::SinkMute(i, m) => session.set_sink_mute(*i, *m),
        Command::SourceMute(i, m) => session.set_source_mute(*i, *m),
        Command::StreamMute(i, m) => session.set_stream_mute(*i, *m),
        Command::Move(stream, sink) => session.move_stream(*stream, *sink),
        Command::SinkPort(i, port) => session.set_sink_port(*i, port),
        Command::Profile(card, profile) => session.set_card_profile(card, profile),
    }
}

fn send(state: &State, command: Command) {
    if let Some(tx) = &state.commands {
        let _ = tx.send(Message::Run(command));
    }
}

/// The command a page needs to start PipeWire's PulseAudio service.
pub const NOT_RUNNING_COMMAND: &str = "systemctl --user enable --now pipewire-pulse";

/// What a click or a drag on the page does: (verb, id, value). The device lists send the
/// device's place in the list as the value.
pub fn act(state: &mut State, verb: &str, id: &str, value: f32) -> Option<(&'static str, &'static str, bool)> {
    if verb == "dismiss" {
        state.error = None;
        return None;
    }
    let snap = state.snapshot.as_ref()?;
    let percent = (value.max(0.0) * 100.0).round() as u32;
    let command = match verb {
        "default-sink" => Command::DefaultSink(snap.sinks.get(value as usize)?.name.clone()),
        "default-source" => Command::DefaultSource(snap.sources.get(value as usize)?.name.clone()),
        "out-volume" => {
            let d = snap.default_sink()?;
            Command::SinkVolume(d.index, d.volume.with_percent(percent))
        }
        "out-mute" => {
            let d = snap.default_sink()?;
            Command::SinkMute(d.index, !d.muted)
        }
        "balance" => {
            let d = snap.default_sink()?;
            Command::SinkVolume(d.index, d.volume.with_balance(value))
        }
        "out-port" => {
            let d = snap.default_sink()?;
            let port = d.ports.get(id.parse::<usize>().ok()?)?;
            Command::SinkPort(d.index, port.name.clone())
        }
        "in-volume" => {
            let d = snap.default_source()?;
            Command::SourceVolume(d.index, d.volume.with_percent(percent))
        }
        "in-mute" => {
            let d = snap.default_source()?;
            Command::SourceMute(d.index, !d.muted)
        }
        "app-volume" | "app-mute" | "app-output" => {
            let s = snap.streams.iter().find(|s| s.index.to_string() == id)?;
            match verb {
                "app-volume" => Command::StreamVolume(s.index, s.volume.with_percent(percent)),
                "app-mute" => Command::StreamMute(s.index, !s.muted),
                _ => Command::Move(s.index, snap.sinks.get(value as usize)?.index),
            }
        }
        "profile" => {
            let c = snap.cards.iter().find(|c| c.name == id)?;
            Command::Profile(c.name.clone(), c.profiles.get(value as usize)?.name.clone())
        }
        "overamplify" => {
            state.overamplify = !state.overamplify;
            return Some(("sound", "overamplify", state.overamplify));
        }
        _ => return None,
    };
    send(state, command);
    None
}

fn icon(kind: &str) -> &'static str {
    match kind {
        "headphones" | "headset" => "headphones",
        "tv" => "tv",
        "webcam" | "microphone" => "mic",
        _ => "volume",
    }
}

/// What it is, for a device that says nothing about its bus or ports.
fn kind_name(kind: &str) -> &'static str {
    match kind {
        "headphones" => "Headphones",
        "headset" => "Headset",
        "tv" => "TV or monitor",
        "webcam" => "Webcam",
        "microphone" => "Microphone",
        _ => "Speakers",
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    match n {
        0 => format!("no {one}"),
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

fn s(text: impl AsRef<str>) -> SharedString {
    SharedString::from(text.as_ref())
}

/// The quick popup's Volume: the output in use's volume (1.0 is 100 %), whether it's muted, the
/// highest the sliders go (as on the page), and the apps playing: `QUICK_APPS` at most, and how
/// many more there are.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Quick {
    pub volume: f32,
    pub muted: bool,
    pub max: f32,
    pub apps: Vec<SoundApp>,
    pub more: usize,
}

/// The quick popup's Volume; `None` without an output.
pub fn quick(state: &State) -> Option<Quick> {
    let snap = state.snapshot.as_ref()?;
    let d = snap.default_sink()?;
    let order = quick_order(&snap.streams);
    Some(Quick {
        volume: d.volume.percent() as f32 / 100.0,
        muted: d.muted,
        max: if state.overamplify { 1.5 } else { 1.0 },
        apps: order.iter().take(QUICK_APPS).map(|st| app_row(state, snap, st)).collect(),
        more: order.len().saturating_sub(QUICK_APPS),
    })
}

/// The apps in the popup's order: those playing before those paused, the newest first in each
/// (the server numbers streams as they start).
fn quick_order(streams: &[Stream]) -> Vec<&Stream> {
    let mut order: Vec<&Stream> = streams.iter().collect();
    order.sort_by_key(|st| (st.corked, std::cmp::Reverse(st.index)));
    order
}

/// An app's row, on the page and in the popup.
fn app_row(state: &State, snap: &Snapshot, st: &Stream) -> SoundApp {
    let icon = state.icons.get(&icon_key(st)).cloned().flatten();
    SoundApp {
        id: s(st.index.to_string()),
        name: s(&st.app),
        sub: s(if st.corked { format!("Paused · {}", st.title) } else { st.title.clone() }),
        volume: st.volume.percent() as f32 / 100.0,
        muted: st.muted,
        output: snap.sinks.iter().position(|d| d.index == st.sink).unwrap_or(0) as i32,
        has_icon: icon.is_some(),
        icon: icon.unwrap_or_default(),
    }
}

/// The sidebar's status: the volume of the output in use.
pub fn status(state: &State) -> String {
    match state.snapshot.as_ref().and_then(|s| s.default_sink()) {
        Some(d) if d.muted => "Muted".to_owned(),
        Some(d) => format!("{}%", d.volume.percent()),
        None if state.not_running => "Not running".to_owned(),
        None => String::new(),
    }
}

pub fn view(state: &State) -> SoundView {
    let max = if state.overamplify { 1.5 } else { 1.0 };
    let mut v = SoundView { max, overamplify: state.overamplify, ..SoundView::default() };
    let Some(snap) = state.snapshot.as_ref() else {
        if state.not_running {
            v.state = s("not-running");
            v.sub = s("Not running");
            v.empty_title = s("PipeWire isn't running");
            v.empty_text = s("Lituus uses PipeWire's PulseAudio service. Start it with:");
            v.empty_command = s(NOT_RUNNING_COMMAND);
        } else if let Some(error) = &state.error {
            v.state = s("loading");
            v.empty_title = s("Couldn't reach PipeWire");
            v.empty_text = s(format!("{error}. Lituus tries again every few seconds."));
        } else {
            v.state = s("loading");
            v.empty_title = s("Reading PipeWire…");
        }
        return v;
    };
    v.state = s("list");
    v.error = s(state.error.clone().unwrap_or_default());
    // How many of each, as the other pages' lines say it.
    let playing = snap.streams.iter().filter(|s| !s.corked).count();
    let mut sub = format!(
        "{} · {}",
        plural(snap.sinks.len(), "output", "outputs"),
        plural(snap.sources.len(), "input", "inputs")
    );
    match playing {
        0 => {}
        1 => sub.push_str(" · 1 app playing"),
        n => sub.push_str(&format!(" · {n} apps playing")),
    }
    v.sub = s(sub);
    let port_name = |d: &lituus_sound::Device| {
        d.active_port.as_deref().and_then(|p| d.ports.iter().find(|x| x.name == p)).map(|p| p.description.clone())
    };
    if let Some(d) = snap.default_sink() {
        v.hero = HeroInfo {
            icon: s(icon(d.kind())),
            name: s(&d.description),
            meta: s(port_name(d).unwrap_or_else(|| bus(&d.bus).to_owned())),
            tone: s("muted"),
            detail: s(""),
        };
        v.out_volume = d.volume.percent() as f32 / 100.0;
        v.out_muted = d.muted;
        v.out_ports = model(d.ports.iter().map(|p| s(&p.description)).collect());
        v.out_port =
            d.active_port.as_deref().and_then(|a| d.ports.iter().position(|p| p.name == a)).unwrap_or(0) as i32;
        v.has_balance = d.volume.has_balance();
        v.out_balance = d.volume.balance();
    } else {
        v.hero = HeroInfo {
            icon: s("volume-off"),
            name: s("No output"),
            meta: s("Nothing to play sound through"),
            ..HeroInfo::default()
        };
    }
    // The line under a device in its list: "Bluetooth · Headphones", "USB", "Headphones".
    let details = |d: &lituus_sound::Device| {
        s(match (port_name(d), bus(&d.bus)) {
            (Some(p), "") => p,
            (Some(p), b) => format!("{b} · {p}"),
            (None, "") => kind_name(d.kind()).to_owned(),
            (None, b) => b.to_owned(),
        })
    };
    let place = |devices: &[lituus_sound::Device], used: Option<&lituus_sound::Device>| {
        used.and_then(|u| devices.iter().position(|d| d.name == u.name)).map_or(-1, |i| i as i32)
    };
    v.outputs_names = model(snap.sinks.iter().map(|d| s(&d.description)).collect());
    v.outputs_details = model(snap.sinks.iter().map(details).collect());
    v.output = place(&snap.sinks, snap.default_sink());
    v.inputs_names = model(snap.sources.iter().map(|d| s(&d.description)).collect());
    v.inputs_details = model(snap.sources.iter().map(details).collect());
    v.input = place(&snap.sources, snap.default_source());
    if let Some(d) = snap.default_source() {
        v.in_volume = d.volume.percent() as f32 / 100.0;
        v.in_muted = d.muted;
    }
    v.apps = model(snap.streams.iter().map(|st| app_row(state, snap, st)).collect());
    v.cards =
        model(
            snap.cards
                .iter()
                .map(|c| SoundCard {
                    id: s(&c.name),
                    name: s(&c.description),
                    profiles: model(
                        c.profiles
                            .iter()
                            .map(|p| {
                                if p.available {
                                    s(&p.description)
                                } else {
                                    s(format!("{} (unplugged)", p.description))
                                }
                            })
                            .collect(),
                    ),
                    selected: c.active.as_deref().and_then(|a| c.profiles.iter().position(|p| p.name == a)).unwrap_or(0)
                        as i32,
                })
                .collect(),
        );
    v
}

fn bus(bus: &str) -> &'static str {
    match bus {
        "bluetooth" => "Bluetooth",
        "usb" => "USB",
        "pci" => "Built in",
        _ => "",
    }
}

thread_local! {
    /// The apps' rows, kept as one model from one render to the next: a new one would build the
    /// row being dragged again, and its volume slider would stop after the first step.
    static APPS: Rc<VecModel<SoundApp>> = Rc::new(VecModel::default());
    /// The same for the quick popup's apps.
    static QUICK_ROWS: Rc<VecModel<SoundApp>> = Rc::new(VecModel::default());
}

/// The quick popup's apps, in the model it keeps (see `QUICK_ROWS`).
pub fn quick_rows(apps: &[SoundApp]) -> ModelRc<SoundApp> {
    QUICK_ROWS.with(|kept| crate::input::kept(kept, &model(apps.to_vec())))
}

pub fn render(window: &MainWindow, state: &State) {
    let mut view = view(state);
    view.apps = APPS.with(|kept| crate::input::kept(kept, &view.apps));
    // The Cards tab goes when the last card with profiles does.
    if view.cards.row_count() == 0 && window.get_sound_tab() > 1 {
        window.set_sound_tab(0);
    }
    window.set_sound(view);
}

#[cfg(test)]
mod tests {
    use super::*;
    use lituus_sound::{Device, NORM, Port, Stream};

    fn stereo(percent: u32) -> Volume {
        Volume { channels: vec![NORM * percent / 100; 2], sides: vec![-1, 1] }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            server: "PulseAudio (on PipeWire 1.6.2) 15.0.0".into(),
            default_sink: Some("speakers".into()),
            default_source: Some("yeti".into()),
            sinks: vec![
                Device {
                    index: 40,
                    name: "speakers".into(),
                    description: "Speakers".into(),
                    bus: "pci".into(),
                    volume: stereo(45),
                    ports: vec![
                        Port { name: "analog-output-speaker".into(), description: "Speakers".into(), available: None },
                        Port {
                            name: "analog-output-headphones".into(),
                            description: "Headphones".into(),
                            available: Some(false),
                        },
                    ],
                    active_port: Some("analog-output-speaker".into()),
                    ..Device::default()
                },
                Device {
                    index: 41,
                    name: "bluez_output.AC_80".into(),
                    description: "Sony WH-1000XM5".into(),
                    bus: "bluetooth".into(),
                    form_factor: "headphone".into(),
                    volume: stereo(60),
                    ..Device::default()
                },
            ],
            sources: vec![Device {
                index: 50,
                name: "yeti".into(),
                description: "Blue Yeti".into(),
                bus: "usb".into(),
                volume: stereo(70),
                ..Device::default()
            }],
            streams: vec![Stream {
                index: 7,
                app: "Firefox".into(),
                title: "YouTube".into(),
                sink: 41,
                volume: stereo(80),
                ..Stream::default()
            }],
            cards: Vec::new(),
        }
    }

    #[test]
    fn the_quick_popup_has_the_outputs_volume() {
        let mut state = State { snapshot: Some(snapshot()), ..State::default() };
        let q = quick(&state).unwrap();
        assert!((q.volume - 0.45).abs() < 0.001 && !q.muted && q.max == 1.0);
        state.overamplify = true;
        assert_eq!(quick(&state).map(|q| q.max), Some(1.5));
        assert_eq!(quick(&State::default()), None);
    }

    /// The popup shows three apps at most: those playing, newest first, then those paused; the
    /// rest are counted for its link to the page.
    #[test]
    fn the_quick_popup_shows_three_apps_playing_first() {
        let mut snap = snapshot();
        let app = |index: u32, app: &str, corked: bool| Stream {
            index,
            app: app.into(),
            sink: 41,
            volume: stereo(50),
            corked,
            ..Stream::default()
        };
        snap.streams =
            vec![app(3, "mpv", true), app(5, "Firefox", false), app(9, "Spotify", true), app(8, "Discord", false)];
        let q = quick(&State { snapshot: Some(snap), ..State::default() }).unwrap();
        let names: Vec<&str> = q.apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Discord", "Firefox", "Spotify"]);
        assert_eq!(q.apps[2].sub, "Paused · ");
        assert_eq!(q.more, 1);
    }

    /// An app's icon is looked for under the name it gives, then its app id, program and name;
    /// none of them may lead out of the icon themes.
    #[test]
    fn an_apps_icon_goes_by_its_names() {
        let st = Stream {
            app: "Spotify".into(),
            icon: "../../etc/passwd".into(),
            binary: "spotify".into(),
            app_id: "com.spotify.Client".into(),
            ..Stream::default()
        };
        assert_eq!(icon_names(&st), ["com.spotify.Client", "spotify"]);
        let st =
            Stream { app: "Firefox".into(), icon: "firefox".into(), binary: "firefox".into(), ..Stream::default() };
        assert_eq!(icon_names(&st), ["firefox"]);
        let st =
            Stream { app: "x".into(), icon: "/opt/x/icon.png".into(), binary: "bin/x".into(), ..Stream::default() };
        assert_eq!(icon_names(&st), ["/opt/x/icon.png", "x"]);
    }

    /// A drawn icon reaches the app's row; one that wasn't found leaves the line icon.
    #[test]
    fn the_rows_carry_the_icons_found() {
        let mut state = State { snapshot: Some(snapshot()), ..State::default() };
        let key = icon_key(&state.snapshot.as_ref().unwrap().streams[0]);
        assert!(!view(&state).apps.row_data(0).unwrap().has_icon);
        state.icons.insert(key.clone(), None);
        assert!(!view(&state).apps.row_data(0).unwrap().has_icon);
        state.icons.insert(key, Some(Image::from_rgba8(SharedPixelBuffer::new(2, 2))));
        assert!(view(&state).apps.row_data(0).unwrap().has_icon);
        assert!(quick(&state).unwrap().apps[0].has_icon);
    }

    #[test]
    fn the_page_shows_the_output_in_use() {
        let state = State { snapshot: Some(snapshot()), ..State::default() };
        let v = view(&state);
        assert_eq!(v.state, "list");
        assert_eq!(v.hero.name, "Speakers");
        assert_eq!(v.hero.meta, "Speakers");
        assert!((v.out_volume - 0.45).abs() < 0.001);
        assert_eq!(v.out_ports.row_count(), 2);
        assert!(v.has_balance);
        let names: Vec<String> = v.outputs_names.iter().map(|n| n.to_string()).collect();
        assert_eq!(names, ["Speakers", "Sony WH-1000XM5"]);
        let details: Vec<String> = v.outputs_details.iter().map(|n| n.to_string()).collect();
        assert_eq!(details, ["Built in · Speakers", "Bluetooth"]);
        assert_eq!(v.output, 0, "the output in use is the one chosen");
        assert_eq!((v.inputs_names.row_count(), v.input), (1, 0));
        assert_eq!(v.inputs_details.row_data(0).unwrap(), "USB");
        let app = v.apps.row_data(0).unwrap();
        assert_eq!((app.name.as_str(), app.output), ("Firefox", 1));
        assert_eq!(v.sub, "2 outputs · 1 input · 1 app playing");
        assert_eq!(status(&state), "45%");
    }

    /// A device that says nothing about where it's plugged in is described by what it is.
    #[test]
    fn a_device_without_a_bus_says_what_it_is() {
        let mut snap = snapshot();
        snap.sinks[1].bus.clear();
        snap.sources.clear();
        snap.streams.clear();
        snap.default_sink = Some("bluez_output.AC_80".into());
        let v = view(&State { snapshot: Some(snap), ..State::default() });
        assert_eq!(v.outputs_details.row_data(1).unwrap(), "Headphones");
        assert_eq!(v.output, 1);
        assert_eq!((v.inputs_names.row_count(), v.input), (0, -1));
        assert_eq!(v.sub, "2 outputs · no input");
    }

    #[test]
    fn clicks_become_commands() {
        let (tx, rx) = channel();
        let mut state = State { snapshot: Some(snapshot()), commands: Some(tx), ..State::default() };
        let mut sent = |verb: &str, id: &str, value: f32| {
            act(&mut state, verb, id, value);
            match rx.try_recv() {
                Ok(Message::Run(c)) => Some(c),
                _ => None,
            }
        };
        assert_eq!(sent("out-volume", "", 0.3), Some(Command::SinkVolume(40, stereo(30))));
        assert_eq!(sent("out-mute", "", 0.0), Some(Command::SinkMute(40, true)));
        assert_eq!(sent("out-port", "1", 0.0), Some(Command::SinkPort(40, "analog-output-headphones".into())));
        assert_eq!(sent("app-output", "7", 0.0), Some(Command::Move(7, 40)));
        // The device lists send the chosen device's place in the list.
        assert_eq!(sent("default-sink", "", 1.0), Some(Command::DefaultSink("bluez_output.AC_80".into())));
        assert_eq!(sent("default-source", "", 0.0), Some(Command::DefaultSource("yeti".into())));
        assert_eq!(sent("default-sink", "", 5.0), None, "no such output");
        assert_eq!(sent("in-volume", "", 0.5), Some(Command::SourceVolume(50, stereo(50))));
        assert_eq!(sent("in-mute", "", 0.0), Some(Command::SourceMute(50, true)));
        assert_eq!(sent("app-volume", "99", 0.5), None, "no such app");
    }

    #[test]
    fn a_refused_change_shows_until_dismissed() {
        let mut state =
            State { snapshot: Some(snapshot()), error: Some("Couldn't change that: busy".into()), ..State::default() };
        assert_eq!(view(&state).error, "Couldn't change that: busy");
        act(&mut state, "dismiss", "", 0.0);
        assert_eq!(view(&state).error, "");
    }

    #[test]
    fn a_dragged_slider_keeps_only_its_last_volume() {
        let a = Command::SinkVolume(1, stereo(10));
        assert!(Command::SinkVolume(1, stereo(20)).replaces(&a));
        assert!(!Command::SinkVolume(2, stereo(20)).replaces(&a));
        assert!(!Command::SinkMute(1, true).replaces(&a));
    }

    #[test]
    fn no_pipewire_is_said_plainly() {
        let v = view(&State { not_running: true, ..State::default() });
        assert_eq!(v.state, "not-running");
        // The sidebar says so too, not with a dash.
        assert_eq!(status(&State { not_running: true, ..State::default() }), "Not running");
        assert!(v.empty_text.contains("PulseAudio"));
        // The command is on its own, for its Copy button.
        assert_eq!(v.empty_command, NOT_RUNNING_COMMAND);
        let v = view(&State { error: Some("Connection refused".into()), ..State::default() });
        assert_eq!((v.state.as_str(), v.empty_title.as_str()), ("loading", "Couldn't reach PipeWire"));
    }

    /// pipewire-pulse on the other end: the handshake, empty lists, and an ack for the rest,
    /// until the connection closes.
    fn fake_server(server: std::os::unix::net::UnixStream) {
        use pulseaudio::protocol::{self, Command};
        let mut socket = std::io::BufReader::new(server);
        let v = protocol::MAX_VERSION;
        while let Ok((seq, command)) = protocol::read_command_message(&mut socket, v) {
            let w = socket.get_mut();
            let _ = match command {
                Command::Auth(_) => {
                    protocol::write_reply_message(w, seq, &protocol::AuthReply { version: v, ..Default::default() }, v)
                }
                Command::SetClientName(_) => {
                    protocol::write_reply_message(w, seq, &protocol::SetClientNameReply { client_id: 1 }, v)
                }
                Command::GetServerInfo => protocol::write_reply_message(w, seq, &protocol::ServerInfo::default(), v),
                Command::GetSinkInfoList => protocol::write_reply_message(w, seq, &protocol::SinkInfoList::new(), v),
                Command::GetSourceInfoList => {
                    protocol::write_reply_message(w, seq, &protocol::SourceInfoList::new(), v)
                }
                Command::GetSinkInputInfoList => {
                    protocol::write_reply_message(w, seq, &protocol::SinkInputInfoList::new(), v)
                }
                Command::GetCardInfoList => protocol::write_reply_message(w, seq, &protocol::CardInfoList::new(), v),
                _ => protocol::write_ack_message(w, seq),
            };
        }
    }

    /// A session and a watcher on a fake server; gives the server's ends, and a receiver that
    /// hears when the watcher's server side sees its connection closed.
    fn fake_connection()
    -> (Session, Session, (std::os::unix::net::UnixStream, std::os::unix::net::UnixStream), Receiver<()>) {
        use std::os::unix::net::UnixStream;
        let (session_end, session_server) = UnixStream::pair().unwrap();
        let (watch_end, watch_server) = UnixStream::pair().unwrap();
        let ends = (session_server.try_clone().unwrap(), watch_server.try_clone().unwrap());
        let (closed, watch_closed) = channel();
        std::thread::spawn(move || fake_server(session_server));
        std::thread::spawn(move || {
            fake_server(watch_server);
            let _ = closed.send(());
        });
        let session = Session::over(session_end, Vec::new()).unwrap();
        let watcher = Session::over(watch_end, Vec::new()).unwrap();
        (session, watcher, ends, watch_closed)
    }

    /// pipewire-pulse restarts: the watcher's connection closes, and the thread connects again,
    /// rather than wait for a change that never comes and lose the next one (03-3).
    #[test]
    fn the_sound_thread_notices_a_server_that_went_away() {
        let (mut session, watcher, ends, _) = fake_connection();
        let (done, returned) = channel();
        std::thread::spawn(move || {
            // As run() sets them up: it keeps `tx` for as long as it lives.
            let (tx, rx) = channel();
            let result = connected(&slint::Weak::default(), &mut session, watcher, &tx, &rx, 1, &mut Icons::new());
            let _ = done.send(result.is_err());
        });
        // The first snapshot is read, the watcher has subscribed.
        std::thread::sleep(Duration::from_millis(300));
        ends.0.shutdown(std::net::Shutdown::Both).unwrap();
        ends.1.shutdown(std::net::Shutdown::Both).unwrap();
        assert_eq!(
            returned.recv_timeout(Duration::from_secs(3)),
            Ok(true),
            "serve() still waits for a command on a connection the server closed"
        );
    }

    /// The session's connection breaks while the server is still there: the thread connects
    /// again, and the old watcher and its connection go with it (03-8).
    #[test]
    fn a_reconnect_leaves_no_watcher_behind() {
        let (mut session, watcher, ends, watch_closed) = fake_connection();
        let (tx, rx) = channel();
        let commands = tx.clone();
        let (done, returned) = channel();
        std::thread::spawn(move || {
            let result = connected(&slint::Weak::default(), &mut session, watcher, &tx, &rx, 1, &mut Icons::new());
            let _ = done.send(result.is_err());
        });
        std::thread::sleep(Duration::from_millis(300));
        ends.0.shutdown(std::net::Shutdown::Both).unwrap();
        commands.send(Message::Run(Command::SinkMute(0, true))).unwrap();
        assert_eq!(returned.recv_timeout(Duration::from_secs(3)), Ok(true), "the broken session went unnoticed");
        assert!(watch_closed.recv_timeout(Duration::from_secs(3)).is_ok(), "the old watcher is still connected");
    }
}
