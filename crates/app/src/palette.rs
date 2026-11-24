//! The command palette (M10, D-47; boards on the canvas page "Command palette"). Ctrl K, or the
//! sidebar's search button, opens it over the page. It finds pages, settings (by name and by the
//! words they're known by, each with its value now) and actions, and works from the keyboard:
//! ↑↓ select; ↵ does the row's main thing (a page opens, an action runs, a switch flips, a choice
//! or a slider is stepped into); Ctrl ↵ goes to the row's page, with the setting highlighted.
//! Stepped into a choice or a slider, ← → change it live, ↵ keeps it and Esc puts the old value
//! back. Empty, it shows what was used last (this session) and the quick settings.
//!
//! Every change goes through the pages' own handlers: this module only says which, as
//! [`Effect`]s, which `main` runs outside `update` (the handlers update the state themselves).

use slint::{ModelRc, SharedString, VecModel};

use crate::{App, Hint, PaletteRow, PaletteView, bluetooth, config, input, nav, system, xkb};

/// The most rows a section shows; its title counts them all.
const SETTINGS_SHOWN: usize = 6;
const ACTIONS_SHOWN: usize = 6;
const PAGES_SHOWN: usize = 4;
/// What the empty palette remembers as used last.
const RECENT: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Index(usize),
    Num(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Percent,
    Ms,
    PerSecond,
    Speed,
    Times,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    None,
    Switch(bool),
    Choice { options: Vec<String>, index: usize },
    Slider { value: f64, min: f64, max: f64, step: f64, unit: Unit },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Setting,
    Action,
    Page,
}

/// How a setting is changed: which page handler, and its verb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setter {
    Wifi,
    Airplane,
    Bt(&'static str),
    /// A volume, 0 to its maximum.
    Sound(&'static str),
    /// A mute: toggled.
    SoundToggle(&'static str),
    /// Keyboard: toggled, an option by its index, or a number.
    InputToggle(&'static str),
    InputIndex(&'static str),
    InputNum(&'static str),
    /// Every mouse's: the same, after selecting the mice's defaults.
    MouseToggle(&'static str),
    MouseIndex(&'static str),
    MouseNum(&'static str),
    /// Settings › Appearance: an index, or toggled.
    Appearance(&'static str),
    AppearanceToggle(&'static str),
    /// Settings: tray, close to tray, notifications.
    Option(&'static str),
}

/// What running a row does, through the pages' handlers.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Navigate(&'static str),
    Wifi,
    Airplane,
    Bt(&'static str, String),
    Sound(&'static str, f32),
    Input(&'static str, String, f32),
    Appearance(&'static str, i32),
    Option(&'static str),
    Wallpaper(&'static str),
    System(&'static str),
    Shortcuts(&'static str),
    Apps(&'static str),
    Hostname,
    /// Text to the clipboard.
    Copy(String),
    /// A Wi-Fi network's or a VPN's row (`wifi:…`, `conn:…`) and what to do with it, as its
    /// page's button does, without going there.
    Net {
        id: String,
        verb: &'static str,
    },
    /// The setting's row on its page ("Repeat delay"), for a moment.
    Highlight(String),
}

/// A row the palette can show.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    pub icon: &'static str,
    pub title: String,
    /// Where it lives, and its state: "Settings › Appearance · now Auto".
    pub place: String,
    /// More words it's found by ("dark" finds Mode).
    pub keywords: String,
    /// The page it's on (a sidebar id), "" for none.
    pub page: &'static str,
    /// That page's name, for "Go to it in Keyboard".
    pub page_name: &'static str,
    /// Its row's name on that page, when not the title.
    pub label: &'static str,
    pub control: Control,
    pub setter: Option<Setter>,
    /// What ↵ does for a page or an action.
    pub run: Vec<Effect>,
    /// Listed under "Quick settings" while nothing is typed.
    pub quick: bool,
}

impl Item {
    fn setting(
        id: &str,
        icon: &'static str,
        title: &str,
        place: String,
        keywords: &str,
        page: (&'static str, &'static str),
    ) -> Self {
        Item {
            id: format!("set:{id}"),
            kind: Kind::Setting,
            icon,
            title: title.into(),
            place,
            keywords: keywords.into(),
            page: page.0,
            page_name: page.1,
            label: "",
            control: Control::None,
            setter: None,
            run: Vec::new(),
            quick: false,
        }
    }

    fn action(
        id: String,
        icon: &'static str,
        title: String,
        place: String,
        keywords: &str,
        page: &'static str,
        run: Vec<Effect>,
    ) -> Self {
        Item {
            id: format!("act:{id}"),
            kind: Kind::Action,
            icon,
            title,
            place,
            keywords: keywords.into(),
            page,
            page_name: "",
            label: "",
            control: Control::None,
            setter: None,
            run,
            quick: false,
        }
    }

    fn with(mut self, control: Control, setter: Setter) -> Self {
        self.control = control;
        self.setter = Some(setter);
        self
    }

    fn quick(mut self) -> Self {
        self.quick = true;
        self
    }

    fn labelled(mut self, label: &'static str) -> Self {
        self.label = label;
        self
    }

    fn value(&self) -> Option<Value> {
        match &self.control {
            Control::None => None,
            Control::Switch(on) => Some(Value::Bool(*on)),
            Control::Choice { index, .. } => Some(Value::Index(*index)),
            Control::Slider { value, .. } => Some(Value::Num(*value)),
        }
    }

    /// The value in words: "Dark", "600 ms", "On".
    fn value_text(&self, value: &Value) -> String {
        match (&self.control, value) {
            (Control::Choice { options, .. }, Value::Index(i)) => options.get(*i).cloned().unwrap_or_default(),
            (Control::Slider { unit, .. }, Value::Num(v)) => number(*v, *unit),
            (_, Value::Bool(on)) => (if *on { "On" } else { "Off" }).into(),
            _ => String::new(),
        }
    }
}

fn number(v: f64, unit: Unit) -> String {
    match unit {
        Unit::Percent => format!("{:.0} %", v * 100.0),
        Unit::Ms => format!("{v:.0} ms"),
        Unit::PerSecond => format!("{v:.0} /s"),
        Unit::Speed => format!("{v:+.2}"),
        Unit::Times => format!("{v:.1}×"),
    }
}

/// The palette's own state; the rows are worked out from the app each time.
#[derive(Default)]
pub struct State {
    pub open: bool,
    pub query: String,
    /// The selected row, among the rows shown (not counting the sections' titles).
    selected: usize,
    editing: Option<Editing>,
    /// Item ids, the last used first.
    recent: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Editing {
    id: String,
    was: Value,
    now: Value,
}

pub fn open(state: &mut State) {
    state.open = true;
    state.query.clear();
    state.selected = 0;
    state.editing = None;
}

/// Closed with Esc, Ctrl K or a click beside it: a choice stepped into keeps its value.
pub fn close(state: &mut State) {
    state.open = false;
    state.editing = None;
}

pub fn edited(state: &mut State, text: &str) {
    if state.editing.is_none() {
        state.query = text.to_owned();
        state.selected = 0;
    }
}

// ---------------------------------------------------------------------------------------------
// What it finds.

/// Every page, setting and action, with their values now.
pub fn items(app: &App) -> Vec<Item> {
    let mut items = Vec::new();
    settings(app, &mut items);
    actions(app, &mut items);
    pages(app, &mut items);
    items
}

fn settings(app: &App, items: &mut Vec<Item>) {
    const SETTINGS: (&str, &str) = ("settings", "Settings");
    if let Some(snap) = &app.snapshot
        && snap.wireless_hardware_enabled
    {
        items.push(
            Item::setting(
                "wifi",
                "wifi",
                "Wi-Fi",
                "Network › Wi-Fi".into(),
                "wifi wi-fi wireless wlan internet radio",
                ("wifi", "Wi-Fi"),
            )
            .with(Control::Switch(snap.wireless_enabled), Setter::Wifi)
            .quick(),
        );
    }
    let (bt_on, _) = bluetooth::quick(&app.bt);
    if let Some(on) = bt_on {
        items.push(
            Item::setting(
                "bluetooth",
                "bluetooth",
                "Bluetooth",
                "Bluetooth".into(),
                "bt bluetooth radio",
                ("bluetooth", "Bluetooth"),
            )
            .with(Control::Switch(on), Setter::Bt("power"))
            .quick(),
        );
    }
    if app.snapshot.is_some() || bt_on.is_some() {
        items.push(
            Item::setting(
                "airplane",
                "plane",
                "Airplane mode",
                "Turns off Wi-Fi and Bluetooth".into(),
                "airplane flight offline radios",
                ("", ""),
            )
            .with(Control::Switch(app.quick.airplane), Setter::Airplane)
            .quick(),
        );
    }
    if let Some(adapter) = app.bt.snapshot.as_ref().and_then(|s| s.adapter())
        && adapter.powered
    {
        items.push(
            Item::setting(
                "bt-visible",
                "bluetooth",
                "Visible to other devices",
                "Bluetooth · while you pair".into(),
                "visible discoverable pairing bluetooth",
                ("bluetooth", "Bluetooth"),
            )
            .with(Control::Switch(adapter.discoverable), Setter::Bt("visible"))
            .labelled("Visible"),
        );
    }
    if let Some(snap) = &app.sound.snapshot {
        let max = if app.sound.overamplify { 1.5 } else { 1.0 };
        if let Some(sink) = snap.default_sink() {
            let volume = f64::from(sink.volume.percent()) / 100.0;
            items.push(
                Item::setting(
                    "out-volume",
                    "volume",
                    "Output volume",
                    format!("Sound · {}", sink.description),
                    "volume sound loud quiet speakers headphones output",
                    ("sound", "Sound"),
                )
                .with(
                    Control::Slider { value: volume.min(max), min: 0.0, max, step: 0.05, unit: Unit::Percent },
                    Setter::Sound("out-volume"),
                )
                .quick(),
            );
            items.push(
                Item::setting(
                    "out-mute",
                    "volume",
                    "Mute the output",
                    format!("Sound · {}", sink.description),
                    "mute silence sound speakers",
                    ("sound", "Sound"),
                )
                .with(Control::Switch(sink.muted), Setter::SoundToggle("out-mute")),
            );
        }
        if let Some(source) = snap.default_source() {
            let volume = f64::from(source.volume.percent()) / 100.0;
            items.push(
                Item::setting(
                    "in-volume",
                    "mic",
                    "Microphone volume",
                    format!("Sound · {}", source.description),
                    "microphone mic input volume recording",
                    ("sound", "Sound"),
                )
                .with(
                    Control::Slider { value: volume.min(max), min: 0.0, max, step: 0.05, unit: Unit::Percent },
                    Setter::Sound("in-volume"),
                ),
            );
            items.push(
                Item::setting(
                    "in-mute",
                    "mic",
                    "Mute the microphone",
                    format!("Sound · {}", source.description),
                    "mute microphone mic input",
                    ("sound", "Sound"),
                )
                .with(Control::Switch(source.muted), Setter::SoundToggle("in-mute")),
            );
        }
    }
    if input::has_settings(&app.input) {
        const KEYBOARD: (&str, &str) = ("keyboard", "Keyboard");
        const MOUSE: (&str, &str) = ("mouse", "Mouse & touchpad");
        let k = &app.input.settings.keyboard;
        items.push(
            Item::setting(
                "repeat-delay",
                "keyboard",
                "Repeat delay",
                "Input › Keyboard · before a held key repeats".into(),
                "keyboard key repeat delay hold",
                KEYBOARD,
            )
            .with(
                Control::Slider { value: k.repeat_delay as f64, min: 100.0, max: 1000.0, step: 10.0, unit: Unit::Ms },
                Setter::InputNum("repeat-delay"),
            ),
        );
        items.push(
            Item::setting(
                "repeat-rate",
                "keyboard",
                "Repeat rate",
                "Input › Keyboard · repeats per second".into(),
                "keyboard key repeat rate speed",
                KEYBOARD,
            )
            .with(
                Control::Slider { value: k.repeat_rate as f64, min: 5.0, max: 80.0, step: 1.0, unit: Unit::PerSecond },
                Setter::InputNum("repeat-rate"),
            ),
        );
        for (id, title, keywords, choice, label) in [
            (
                "caps",
                "Caps Lock acts as",
                "caps lock escape ctrl control remap backspace super",
                xkb::CAPS,
                "Caps Lock acts as",
            ),
            ("compose", "Compose key", "compose key accents special characters", xkb::COMPOSE, "Compose key"),
        ] {
            let now = choice.current(&k.options);
            if let Some(index) = choice.values.iter().position(|(o, _)| *o == now) {
                let options = choice.values.iter().map(|(_, l)| (*l).to_owned()).collect();
                items.push(
                    Item::setting(id, "keyboard", title, "Input › Keyboard · Keys".into(), keywords, KEYBOARD)
                        .with(
                            Control::Choice { options, index },
                            Setter::InputIndex(if id == "caps" { "caps" } else { "compose" }),
                        )
                        .labelled(label),
                );
            }
        }
        items.push(
            Item::setting(
                "numlock",
                "keyboard",
                "Num Lock on at login",
                "Input › Keyboard · Keys".into(),
                "num lock numpad numbers login",
                KEYBOARD,
            )
            .with(Control::Switch(k.numlock), Setter::InputToggle("numlock")),
        );
        let m = &app.input.settings.mouse;
        items.push(
            Item::setting(
                "speed",
                "mouse",
                "Pointer speed",
                "Input › Mouse & touchpad · every mouse".into(),
                "mouse pointer speed sensitivity cursor fast slow",
                MOUSE,
            )
            .with(
                Control::Slider { value: m.sensitivity, min: -1.0, max: 1.0, step: 0.05, unit: Unit::Speed },
                Setter::MouseNum("speed"),
            )
            .labelled("Speed"),
        );
        items.push(
            Item::setting(
                "accel",
                "mouse",
                "Acceleration",
                "Input › Mouse & touchpad · every mouse".into(),
                "mouse acceleration flat adaptive pointer",
                MOUSE,
            )
            .with(
                Control::Choice {
                    options: vec!["Flat".into(), "Adaptive".into()],
                    index: usize::from(m.accel_profile != "flat"),
                },
                Setter::MouseIndex("accel"),
            ),
        );
        items.push(
            Item::setting(
                "natural",
                "mouse",
                "Natural scrolling",
                "Input › Mouse & touchpad · every mouse".into(),
                "natural scrolling reverse invert wheel",
                MOUSE,
            )
            .with(Control::Switch(m.natural_scroll), Setter::MouseToggle("natural")),
        );
        items.push(
            Item::setting(
                "scroll-factor",
                "mouse",
                "Scroll speed",
                "Input › Mouse & touchpad · every mouse".into(),
                "scroll speed wheel factor",
                MOUSE,
            )
            .with(
                Control::Slider { value: m.scroll_factor, min: 0.1, max: 3.0, step: 0.1, unit: Unit::Times },
                Setter::MouseNum("scroll-factor"),
            ),
        );
        items.push(
            Item::setting(
                "left-handed",
                "mouse",
                "Left-handed",
                "Input › Mouse & touchpad · swaps the buttons".into(),
                "left handed buttons swap",
                MOUSE,
            )
            .with(Control::Switch(m.left_handed), Setter::MouseToggle("left-handed")),
        );
    }
    if app.wallpaper.is_some() {
        items.push(
            Item::setting(
                "follow",
                "picture",
                "Colours from your wallpaper",
                "Settings › Appearance · the accent only".into(),
                "wallpaper colours colors matugen accent follow",
                SETTINGS,
            )
            .with(Control::Switch(app.follow_wallpaper), Setter::AppearanceToggle("follow"))
            .labelled("Follow wallpaper"),
        );
    }
    // Both are off while the wallpaper's colours are in use, as on the page.
    if app.palette.is_none() {
        let index = config::Builtin::ALL.iter().position(|b| *b == app.builtin).unwrap_or(0);
        let options = config::Builtin::ALL.iter().map(|b| b.label().to_owned()).collect();
        items.push(
            Item::setting(
                "theme",
                "palette",
                "Built-in theme",
                "Settings › Appearance · for dark and light".into(),
                "theme colour color accent teal blue lavender coral mono",
                SETTINGS,
            )
            .with(Control::Choice { options, index }, Setter::Appearance("theme")),
        );
        let modes = ["Auto", "Dark", "Light"];
        let index = config::Mode::ALL.iter().position(|m| *m == app.mode).unwrap_or(0);
        items.push(
            Item::setting(
                "mode",
                "palette",
                "Mode",
                format!("Settings › Appearance · now {}", modes[index]),
                "dark light night day theme scheme appearance",
                SETTINGS,
            )
            .with(
                Control::Choice { options: modes.iter().map(|m| (*m).to_owned()).collect(), index },
                Setter::Appearance("mode"),
            ),
        );
    }
    items.push(
        Item::setting(
            "gradient",
            "palette",
            "Hero gradient",
            "Settings › Appearance · the glow behind each page's picture".into(),
            "gradient glow hero background",
            SETTINGS,
        )
        .with(Control::Switch(app.gradient), Setter::AppearanceToggle("gradient")),
    );
    for (id, title, place, keywords, on) in [
        ("tray", "Tray icon", "Settings · Lituus in the system tray", "tray icon waybar panel", app.settings.tray),
        (
            "keep_running",
            "Close to tray",
            "Settings · closing the window keeps the tray icon",
            "close tray background keep running",
            app.settings.keep_running,
        ),
        (
            "notifications",
            "Notifications",
            "Settings · connected, disconnected and failed",
            "notifications notify alerts messages",
            app.settings.notifications,
        ),
    ] {
        items.push(
            Item::setting(id, "settings", title, place.into(), keywords, SETTINGS)
                .with(Control::Switch(on), Setter::Option(id)),
        );
    }
}

fn actions(app: &App, items: &mut Vec<Item>) {
    for n in &app.quick.networks {
        if n.connecting {
            continue;
        }
        let (verb, title) = if n.active {
            ("disconnect", format!("Disconnect {}", n.name))
        } else {
            ("connect", format!("Connect to {}", n.name))
        };
        items.push(Item::action(
            format!("{verb}:{}", n.id),
            "wifi",
            title,
            format!("Network › Wi-Fi · {}", n.meta),
            "wifi wi-fi network connect disconnect join",
            "wifi",
            vec![Effect::Net { id: n.id.clone(), verb }],
        ));
    }
    for v in &app.quick.vpns {
        let signing_in = v.id.strip_prefix("conn:").is_some_and(|uuid| app.signing_in.iter().any(|u| u == uuid));
        let (verb, title) = if signing_in {
            ("cancel-sign-in", format!("Stop signing in to {}", v.name))
        } else if v.on {
            ("disconnect", format!("Disconnect {}", v.name))
        } else {
            ("connect", format!("Connect to {}", v.name))
        };
        items.push(Item::action(
            format!("{verb}:{}", v.id),
            "vpn",
            title,
            format!("Network › VPN · {}", v.meta),
            "vpn tunnel wireguard openvpn connect disconnect",
            "vpn",
            vec![Effect::Net { id: v.id.clone(), verb }],
        ));
    }
    if let Some(snap) = &app.bt.snapshot
        && let Some(adapter) = snap.adapter()
        && adapter.powered
    {
        for d in snap.devices_of(&adapter.path).filter(|d| d.paired && !d.blocked) {
            let title = if d.connected { format!("Disconnect {}", d.alias) } else { format!("Connect {}", d.alias) };
            let state = if d.connected { "connected now" } else { "paired" };
            items.push(Item::action(
                format!("bt:{}", d.path),
                "bluetooth",
                title,
                format!("Bluetooth · {state}"),
                "bluetooth connect disconnect headphones speaker device",
                "bluetooth",
                vec![Effect::Bt("row-button", d.path.clone())],
            ));
        }
    }
    let updates: u32 = app.system.updates.results.values().filter_map(|r| r.as_ref().ok()).sum();
    if updates > 0 {
        items.push(Item::action(
            "update-all".into(),
            "rescan",
            "Update all".into(),
            format!("System › Updates · {updates} updates"),
            "update upgrade packages system",
            "system",
            vec![Effect::System("update-all")],
        ));
    }
    items.push(Item::action(
        "rename".into(),
        "monitor",
        "Rename this computer…".into(),
        "System · its name on the network".into(),
        "rename hostname computer name",
        "system",
        vec![Effect::Hostname],
    ));
    if let Some(info) = &app.system.info {
        items.push(Item::action(
            "report".into(),
            "bug",
            "Copy the system report".into(),
            "System · for a bug report".into(),
            "copy report bug system info",
            "system",
            vec![Effect::Copy(system::report(&app.system, info))],
        ));
    }
    if input::has_settings(&app.input) {
        let layouts = app.input.settings.keyboard.layouts.len();
        if layouts > 1 {
            items.push(Item::action(
                "switch-layout".into(),
                "keyboard",
                "Switch to the next layout".into(),
                "Input › Keyboard · as Switch now does".into(),
                "keyboard layout switch language next",
                "keyboard",
                vec![Effect::Input("switch-now", String::new(), 0.0)],
            ));
        }
        if layouts < input::MAX_LAYOUTS {
            items.push(Item::action(
                "add-layout".into(),
                "keyboard",
                "Add a keyboard layout…".into(),
                "Input › Keyboard".into(),
                "keyboard layout add language",
                "keyboard",
                vec![Effect::Navigate("keyboard"), Effect::Input("add-open", String::new(), 0.0)],
            ));
        }
    }
    items.push(Item::action(
        "add-shortcut".into(),
        "keyboard",
        "Add a shortcut…".into(),
        "Input › Shortcuts".into(),
        "shortcut keybind bind add key",
        "shortcuts",
        vec![Effect::Navigate("shortcuts"), Effect::Shortcuts("add")],
    ));
    items.push(Item::action(
        "add-app".into(),
        "app",
        "Add an app…".into(),
        "Desktop › Apps · an AppImage or a website".into(),
        "app add appimage website launcher",
        "apps",
        vec![Effect::Navigate("apps"), Effect::Apps("add-open")],
    ));
    items.push(Item::action(
        "wallpaper-folder".into(),
        "folder",
        "Choose a wallpaper folder…".into(),
        "Desktop › Wallpaper".into(),
        "wallpaper folder pictures background",
        "wallpaper",
        vec![Effect::Wallpaper("choose-folder")],
    ));
}

fn pages(app: &App, items: &mut Vec<Item>) {
    let entries = crate::nav_entries(app);
    for e in entries.iter().filter(|e| e.off.is_empty()) {
        let parent = e.parent.and_then(|p| entries.iter().find(|x| x.id == p)).map(|p| p.label);
        let group = match e.group {
            nav::LITUUS => "Lituus",
            "CONNECTIONS" => "Connections",
            "DEVICES" => "Devices",
            "DESKTOP" => "Desktop",
            _ => "System",
        };
        let place = match (parent, e.status.is_empty()) {
            (Some(p), true) => p.to_owned(),
            (Some(p), false) => format!("{p} · {}", e.status),
            (None, true) => group.to_owned(),
            (None, false) => format!("{group} · {}", e.status),
        };
        items.push(Item {
            id: format!("page:{}", e.id),
            kind: Kind::Page,
            icon: e.icon,
            title: e.label.to_owned(),
            place,
            keywords: e.keywords.to_owned(),
            page: e.id,
            page_name: e.label,
            label: "",
            control: Control::None,
            setter: None,
            run: vec![Effect::Navigate(e.id)],
            quick: false,
        });
    }
}

// ---------------------------------------------------------------------------------------------
// The search.

/// How well `query` finds `item`, and where in its title (in chars), for the accent.
fn score(query: &str, item: &Item) -> Option<(i32, Hit)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    let title = item.title.to_lowercase();
    let at = |byte: usize| title[..byte].chars().count();
    let len = q.chars().count();
    if title.starts_with(&q) {
        return Some((100, Some((0, len))));
    }
    if let Some(b) =
        title.match_indices(&q).map(|(b, _)| b).find(|b| *b > 0 && !title[..*b].ends_with(char::is_alphanumeric))
    {
        return Some((90, Some((at(b), len))));
    }
    if let Some(b) = title.find(&q) {
        return Some((70, Some((at(b), len))));
    }
    // Every word, each the start of a word in the title or its keywords; where it is ranks lower
    // ("dark" finds Mode by its words before the theme that's "for dark and light").
    let words: Vec<&str> = q.split_whitespace().collect();
    let starts = |text: &str| -> bool {
        let hay: Vec<&str> = text.split(|c: char| !c.is_alphanumeric() && c != '-').filter(|w| !w.is_empty()).collect();
        words.iter().all(|w| hay.iter().any(|h| h.starts_with(w)))
    };
    let own = format!("{title} {}", item.keywords.to_lowercase());
    if starts(&title) {
        return Some((60, None));
    }
    if starts(&own) {
        return Some((50, None));
    }
    if starts(&format!("{own} {}", item.place.to_lowercase())) {
        return Some((35, None));
    }
    // Its letters in order in the title ("bltth"), for three letters or more.
    if len >= 3 && !q.contains(' ') {
        let mut letters = title.chars();
        if q.chars().all(|c| letters.any(|t| t == c)) {
            return Some((20, None));
        }
    }
    None
}

/// Where the search found an item in its title: (first char, chars).
type Hit = Option<(usize, usize)>;

/// A row shown: the item's index, and where the search found it.
type Found = (usize, Hit);

/// A section of rows shown: its title and its rows.
struct Section {
    title: String,
    rows: Vec<Found>,
}

fn sections(items: &[Item], query: &str, recent: &[String]) -> Vec<Section> {
    if query.trim().is_empty() {
        let mut out = Vec::new();
        let used: Vec<usize> = recent.iter().filter_map(|id| items.iter().position(|i| &i.id == id)).collect();
        if !used.is_empty() {
            out.push(Section { title: "RECENT".into(), rows: used.iter().map(|&i| (i, None)).collect() });
        }
        let quick: Vec<Found> = items
            .iter()
            .enumerate()
            .filter(|(i, item)| item.quick && !used.contains(i))
            .map(|(i, _)| (i, None))
            .collect();
        if !quick.is_empty() {
            out.push(Section { title: "QUICK SETTINGS".into(), rows: quick });
        }
        return out;
    }
    let mut out: Vec<(i32, Section)> = Vec::new();
    for (kind, name, shown) in [
        (Kind::Setting, "SETTINGS", SETTINGS_SHOWN),
        (Kind::Action, "ACTIONS", ACTIONS_SHOWN),
        (Kind::Page, "PAGES", PAGES_SHOWN),
    ] {
        let mut found: Vec<(i32, usize, Hit)> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.kind == kind)
            .filter_map(|(i, item)| score(query, item).map(|(s, hit)| (s, i, hit)))
            .collect();
        if found.is_empty() {
            continue;
        }
        // Best first; equal ones in their order here.
        found.sort_by_key(|(s, i, _)| (-s, *i));
        let best = found[0].0;
        let title = if kind == Kind::Page { name.to_owned() } else { format!("{name} · {}", found.len()) };
        out.push((best, Section { title, rows: found.into_iter().take(shown).map(|(_, i, hit)| (i, hit)).collect() }));
    }
    // The section with the best match first; Settings before Actions before Pages otherwise.
    out.sort_by_key(|(best, _)| -best);
    out.into_iter().map(|(_, s)| s).collect()
}

fn shown_items(items: &[Item], state: &State) -> Vec<usize> {
    sections(items, &state.query, &state.recent).iter().flat_map(|s| s.rows.iter().map(|(i, _)| *i)).collect()
}

// ---------------------------------------------------------------------------------------------
// Keys and clicks.

/// A key in the palette: "up", "down", "enter", "ctrl-enter", "left", "right", "esc".
pub fn key(app: &mut App, key: &str) -> Vec<Effect> {
    let items = items(app);
    let shown = shown_items(&items, &app.command);
    let state = &mut app.command;
    if let Some(editing) = state.editing.clone() {
        let Some(item) = items.iter().find(|i| i.id == editing.id) else {
            state.editing = None;
            return Vec::new();
        };
        return match key {
            "left" | "right" => {
                let now = step(&item.control, &editing.now, if key == "left" { -1.0 } else { 1.0 });
                if now == editing.now {
                    return Vec::new();
                }
                state.editing = Some(Editing { now: now.clone(), ..editing });
                change(item, &now)
            }
            "esc" => {
                state.editing = None;
                if editing.now == editing.was { Vec::new() } else { change(item, &editing.was) }
            }
            "up" | "down" | "enter" => {
                state.editing = None;
                if key != "enter" {
                    move_selection(state, shown.len(), key);
                }
                Vec::new()
            }
            _ => Vec::new(),
        };
    }
    match key {
        "up" | "down" => {
            move_selection(state, shown.len(), key);
            Vec::new()
        }
        "esc" | "close" => {
            close(state);
            Vec::new()
        }
        "enter" | "ctrl-enter" => match shown.get(state.selected) {
            Some(&i) => run(state, &items[i], key == "ctrl-enter"),
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn move_selection(state: &mut State, count: usize, key: &str) {
    if count == 0 {
        return;
    }
    state.selected = if key == "up" { state.selected.saturating_sub(1) } else { (state.selected + 1).min(count - 1) };
}

/// A click on a row: selected, then its main thing.
pub fn clicked(app: &mut App, row: usize) -> Vec<Effect> {
    let items = items(app);
    let shown = shown_items(&items, &app.command);
    let state = &mut app.command;
    state.editing = None;
    let Some(&i) = shown.get(row) else { return Vec::new() };
    state.selected = row;
    run(state, &items[i], false)
}

/// A row's own control used with the mouse: a switch flipped, a choice's option, a slider moved.
pub fn set(app: &mut App, id: &str, value: f32) -> Vec<Effect> {
    let items = items(app);
    let Some(item) = items.iter().find(|i| i.id == id) else { return Vec::new() };
    let value = match &item.control {
        Control::Switch(on) => Value::Bool(!on),
        Control::Choice { .. } => Value::Index(value.max(0.0) as usize),
        Control::Slider { min, max, .. } => Value::Num(f64::from(value).clamp(*min, *max)),
        Control::None => return Vec::new(),
    };
    let state = &mut app.command;
    if let Some(editing) = &mut state.editing
        && editing.id == item.id
    {
        editing.now = value.clone();
    }
    remember(state, &item.id);
    change(item, &value)
}

/// ↵ (or Ctrl ↵, `to_page`) on a row.
fn run(state: &mut State, item: &Item, to_page: bool) -> Vec<Effect> {
    remember(state, &item.id);
    match (item.kind, to_page) {
        (Kind::Page, _) => {
            close(state);
            item.run.clone()
        }
        (Kind::Action, _) => {
            close(state);
            let mut effects = item.run.clone();
            if to_page && !item.page.is_empty() && !effects.contains(&Effect::Navigate(item.page)) {
                effects.insert(0, Effect::Navigate(item.page));
            }
            effects
        }
        (Kind::Setting, true) => {
            if item.page.is_empty() {
                return Vec::new();
            }
            close(state);
            vec![
                Effect::Navigate(item.page),
                Effect::Highlight(if item.label.is_empty() { item.title.clone() } else { item.label.to_owned() }),
            ]
        }
        (Kind::Setting, false) => match (&item.control, item.value()) {
            (Control::Switch(on), _) => change(item, &Value::Bool(!on)),
            (Control::Choice { .. } | Control::Slider { .. }, Some(now)) => {
                state.editing = Some(Editing { id: item.id.clone(), was: now.clone(), now });
                Vec::new()
            }
            _ => Vec::new(),
        },
    }
}

fn remember(state: &mut State, id: &str) {
    state.recent.retain(|r| r != id);
    state.recent.insert(0, id.to_owned());
    state.recent.truncate(RECENT);
}

/// The value one step (`by` -1 or 1) from `now`: the next option, or the slider's step.
fn step(control: &Control, now: &Value, by: f64) -> Value {
    match (control, now) {
        (Control::Choice { options, .. }, Value::Index(i)) => {
            Value::Index(if by < 0.0 { i.saturating_sub(1) } else { (*i + 1).min(options.len().saturating_sub(1)) })
        }
        (Control::Slider { min, max, step, .. }, Value::Num(v)) => {
            let next = ((v + by * step) / step).round() * step;
            Value::Num(next.clamp(*min, *max))
        }
        _ => now.clone(),
    }
}

/// What setting `item` to `value` takes.
fn change(item: &Item, value: &Value) -> Vec<Effect> {
    let Some(setter) = item.setter else { return Vec::new() };
    let num = |v: &Value| if let Value::Num(n) = v { *n as f32 } else { 0.0 };
    let index = |v: &Value| if let Value::Index(i) = v { *i } else { 0 };
    // Toggles only when it changes.
    if let (Value::Bool(want), Some(Value::Bool(on))) = (value, item.value())
        && *want == on
    {
        return Vec::new();
    }
    let mouse = Effect::Input("select", "mouse".into(), 0.0);
    match setter {
        Setter::Wifi => vec![Effect::Wifi],
        Setter::Airplane => vec![Effect::Airplane],
        Setter::Bt(verb) => vec![Effect::Bt(verb, String::new())],
        Setter::Sound(verb) => vec![Effect::Sound(verb, num(value))],
        Setter::SoundToggle(verb) => vec![Effect::Sound(verb, 0.0)],
        Setter::InputToggle(verb) => vec![Effect::Input(verb, String::new(), 0.0)],
        Setter::InputIndex(verb) => vec![Effect::Input(verb, index(value).to_string(), 0.0)],
        Setter::InputNum(verb) => vec![Effect::Input(verb, String::new(), num(value))],
        Setter::MouseToggle(verb) => vec![mouse, Effect::Input(verb, String::new(), 0.0)],
        Setter::MouseIndex(verb) => vec![mouse, Effect::Input(verb, index(value).to_string(), 0.0)],
        Setter::MouseNum(verb) => vec![mouse, Effect::Input(verb, String::new(), num(value))],
        Setter::Appearance(verb) => vec![Effect::Appearance(verb, index(value) as i32)],
        Setter::AppearanceToggle(verb) => vec![Effect::Appearance(verb, 0)],
        Setter::Option(id) => vec![Effect::Option(id)],
    }
}

// ---------------------------------------------------------------------------------------------
// What the window shows.

fn s(text: impl AsRef<str>) -> SharedString {
    text.as_ref().into()
}

pub fn view(app: &App) -> PaletteView {
    let state = &app.command;
    if !state.open {
        return PaletteView::default();
    }
    let items = items(app);
    let sections = sections(&items, &state.query, &state.recent);
    let count: usize = sections.iter().map(|s| s.rows.len()).sum();
    let selected = state.selected.min(count.saturating_sub(1));
    let mut rows = Vec::new();
    let mut n = 0;
    let mut keys = Vec::new();
    for section in &sections {
        rows.push(PaletteRow { kind: s("header"), title: s(&section.title), ..Default::default() });
        for &(i, hit) in &section.rows {
            let item = &items[i];
            let editing = state.editing.as_ref().filter(|e| e.id == item.id);
            let value = editing.map(|e| e.now.clone()).or_else(|| item.value());
            let mut row = row(item, value.as_ref(), hit);
            row.index = n as i32;
            row.selected = n == selected;
            if let Some(e) = editing {
                row.editing = true;
                row.was = s(format!("was {}", item.value_text(&e.was)));
                row.place = s(format!("Applied now: ↵ keeps it, Esc puts {} back", item.value_text(&e.was)));
            }
            if n == selected {
                keys = hints(item, editing.is_some());
            }
            rows.push(row);
            n += 1;
        }
    }
    let q = state.query.trim();
    PaletteView {
        open: true,
        editing: state.editing.is_some(),
        empty: s(if count == 0 && !q.is_empty() {
            format!("Nothing found for “{q}”: try a page, a setting or what it does")
        } else {
            String::new()
        }),
        rows: ModelRc::new(VecModel::from(rows)),
        keys: ModelRc::new(VecModel::from(if keys.is_empty() { vec![Hint::new("Esc", "Close")] } else { keys })),
    }
}

fn row(item: &Item, value: Option<&Value>, hit: Hit) -> PaletteRow {
    let chars: Vec<char> = item.title.chars().collect();
    let (pre, hit, post) = match hit {
        Some((at, len)) if at + len <= chars.len() => (
            chars[..at].iter().collect::<String>(),
            chars[at..at + len].iter().collect::<String>(),
            chars[at + len..].iter().collect::<String>(),
        ),
        _ => (item.title.clone(), String::new(), String::new()),
    };
    let mut row = PaletteRow {
        kind: s(match (&item.control, item.kind) {
            (Control::Switch(_), _) => "switch",
            (Control::Choice { .. }, _) => "choice",
            (Control::Slider { .. }, _) => "slider",
            (_, Kind::Page) => "page",
            _ => "action",
        }),
        id: s(&item.id),
        icon: s(item.icon),
        title: s(&item.title),
        pre: s(pre),
        hit: s(hit),
        post: s(post),
        place: s(&item.place),
        ..Default::default()
    };
    match (&item.control, value) {
        (Control::Switch(_), Some(Value::Bool(on))) => row.on = *on,
        (Control::Choice { options, .. }, Some(Value::Index(i))) => {
            row.options = ModelRc::new(VecModel::from(options.iter().map(s).collect::<Vec<_>>()));
            row.choice = *i as i32;
            row.value_text = s(options.get(*i).cloned().unwrap_or_default());
        }
        (Control::Slider { min, max, step, unit, .. }, Some(Value::Num(v))) => {
            row.value = *v as f32;
            row.minimum = *min as f32;
            row.maximum = *max as f32;
            row.step = *step as f32;
            row.value_text = s(number(*v, *unit));
        }
        _ => {}
    }
    row
}

/// The footer's keys for the selected row.
fn hints(item: &Item, editing: bool) -> Vec<Hint> {
    if editing {
        return vec![Hint::new("←→", "Change"), Hint::new("↵", "Keep"), Hint::new("Esc", "Put it back")];
    }
    let mut keys = vec![Hint::new("↑↓", "Select")];
    match (item.kind, &item.control) {
        (Kind::Page, _) => keys.push(Hint::new("↵", "Open")),
        (Kind::Action, _) => {
            keys.push(Hint::new("↵", "Run"));
            if !item.page.is_empty() {
                keys.push(Hint::new("Ctrl ↵", "Run and go to its page"));
            }
        }
        (Kind::Setting, control) => {
            keys.push(Hint::new(
                "↵",
                match control {
                    Control::Switch(true) => "Turn off",
                    Control::Switch(false) => "Turn on",
                    _ => "Change it",
                },
            ));
            if !item.page_name.is_empty() {
                keys.push(Hint::new("Ctrl ↵", &format!("Go to it in {}", item.page_name)));
            }
        }
    }
    keys.push(Hint::new("Esc", "Close"));
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: Kind, title: &str, keywords: &str, control: Control, setter: Option<Setter>) -> Item {
        Item {
            id: format!("x:{title}"),
            kind,
            icon: "",
            title: title.into(),
            place: String::new(),
            keywords: keywords.into(),
            page: "settings",
            page_name: "Settings",
            label: "",
            control,
            setter,
            run: vec![Effect::Navigate("settings")],
            quick: false,
        }
    }

    fn mode() -> Item {
        item(
            Kind::Setting,
            "Mode",
            "dark light night day theme scheme",
            Control::Choice { options: vec!["Auto".into(), "Dark".into(), "Light".into()], index: 0 },
            Some(Setter::Appearance("mode")),
        )
    }

    #[test]
    fn finds_by_name_then_by_what_it_does() {
        let items = vec![
            mode(),
            item(Kind::Setting, "Built-in theme", "theme colour", Control::Switch(false), None),
            item(Kind::Page, "Settings", "theme appearance tray dark", Control::None, None),
            item(Kind::Setting, "Repeat delay", "keyboard key repeat", Control::Switch(false), None),
        ];
        // "dark" is in Mode's words and Settings' keywords: the setting comes first.
        let found = sections(&items, "dark", &[]);
        assert_eq!(found.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["SETTINGS · 1", "PAGES"]);
        assert_eq!(found[0].rows, [(0, None)]);
        // A name's start, with where it was found.
        assert_eq!(score("rep", &items[3]), Some((100, Some((0, 3)))));
        assert_eq!(score("delay", &items[3]), Some((90, Some((7, 5)))));
        assert_eq!(score("theme", &items[1]), Some((90, Some((9, 5)))));
        // Words in any order, letters in order.
        assert!(score("key rep", &items[3]).is_some());
        assert_eq!(score("rptdl", &items[3]), Some((20, None)));
        assert_eq!(score("zz", &items[3]), None);
        // Where it lives counts less than its own words.
        let mut theme = item(Kind::Setting, "Built-in theme", "theme colour", Control::Switch(false), None);
        theme.place = "Settings › Appearance · for dark and light".into();
        assert!(score("dark", &theme).unwrap().0 < score("dark", &mode()).unwrap().0);
    }

    #[test]
    fn empty_it_shows_what_was_used_and_the_quick_settings() {
        let mut wifi = item(Kind::Setting, "Wi-Fi", "", Control::Switch(true), Some(Setter::Wifi));
        wifi.quick = true;
        let items = vec![mode(), wifi];
        let none = sections(&items, " ", &[]);
        assert_eq!(none.len(), 1);
        assert_eq!(none[0].title, "QUICK SETTINGS");
        let used = sections(&items, "", &["x:Mode".into()]);
        assert_eq!(
            used.iter().map(|s| (s.title.as_str(), s.rows.len())).collect::<Vec<_>>(),
            [("RECENT", 1), ("QUICK SETTINGS", 1)]
        );
    }

    #[test]
    fn enter_steps_into_a_choice_and_esc_puts_it_back() {
        let item = mode();
        let mut state = State { open: true, ..Default::default() };
        // ↵ steps into it, nothing changes yet.
        assert_eq!(run(&mut state, &item, false), []);
        let editing = state.editing.clone().unwrap();
        assert_eq!(editing.was, Value::Index(0));
        // → Dark, → Light, → stays Light.
        let dark = step(&item.control, &editing.now, 1.0);
        assert_eq!(change(&item, &dark), [Effect::Appearance("mode", 1)]);
        let light = step(&item.control, &dark, 1.0);
        assert_eq!(step(&item.control, &light, 1.0), Value::Index(2));
        // Esc: back to Auto.
        assert_eq!(change(&item, &editing.was), [Effect::Appearance("mode", 0)]);
        // Ctrl ↵ goes to its row on its page.
        assert_eq!(run(&mut state, &item, true), [Effect::Navigate("settings"), Effect::Highlight("Mode".into())]);
        assert!(!state.open);
        assert_eq!(state.recent, ["x:Mode"]);
    }

    #[test]
    fn a_switch_flips_and_the_mouse_is_selected_first() {
        let mut state = State { open: true, ..Default::default() };
        let wifi = item(Kind::Setting, "Wi-Fi", "", Control::Switch(true), Some(Setter::Wifi));
        assert_eq!(run(&mut state, &wifi, false), [Effect::Wifi]);
        assert!(state.open, "a switch leaves the palette open");
        // Already where it should be: nothing.
        assert_eq!(change(&wifi, &Value::Bool(true)), []);
        let natural =
            item(Kind::Setting, "Natural scrolling", "", Control::Switch(false), Some(Setter::MouseToggle("natural")));
        assert_eq!(
            change(&natural, &Value::Bool(true)),
            [Effect::Input("select", "mouse".into(), 0.0), Effect::Input("natural", String::new(), 0.0)]
        );
    }

    #[test]
    fn sliders_step_and_stop_at_their_ends() {
        let delay = Control::Slider { value: 600.0, min: 100.0, max: 1000.0, step: 10.0, unit: Unit::Ms };
        assert_eq!(step(&delay, &Value::Num(600.0), 1.0), Value::Num(610.0));
        assert_eq!(step(&delay, &Value::Num(1000.0), 1.0), Value::Num(1000.0));
        let volume = Control::Slider { value: 0.65, min: 0.0, max: 1.0, step: 0.05, unit: Unit::Percent };
        let Value::Num(v) = step(&volume, &Value::Num(0.65), -1.0) else { panic!() };
        assert!((v - 0.6).abs() < 1e-9);
        assert_eq!(number(0.65, Unit::Percent), "65 %");
        assert_eq!(number(-0.25, Unit::Speed), "-0.25");
    }

    #[test]
    fn actions_run_and_close_ctrl_enter_also_opens_their_page() {
        let mut state = State { open: true, ..Default::default() };
        let mut update = item(Kind::Action, "Update all", "", Control::None, None);
        update.page = "system";
        update.run = vec![Effect::System("update-all")];
        assert_eq!(run(&mut state, &update, true), [Effect::Navigate("system"), Effect::System("update-all")]);
        assert!(!state.open);
        for id in ["a", "b", "c", "d"] {
            remember(&mut state, id);
        }
        assert_eq!(state.recent, ["d", "c", "b"]);
    }
}
