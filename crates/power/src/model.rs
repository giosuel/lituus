//! What the power services say.

/// Everything the Power page reads from D-Bus. Each part is `None` or empty when its service
/// isn't there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// power-profiles-daemon's profiles.
    pub profiles: Option<Profiles>,
    /// The computer's own battery.
    pub battery: Option<Battery>,
    /// Other devices' batteries: a mouse, a keyboard, a headset.
    pub devices: Vec<DeviceBattery>,
    /// Running on the battery right now.
    pub on_battery: bool,
    /// It has a lid: a laptop.
    pub lid: bool,
    /// What logind does itself with the lid and the power key.
    pub logind: Option<Logind>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profiles {
    /// "performance", "balanced" or "power-saver".
    pub active: String,
    pub available: Vec<String>,
    /// Why performance is held back ("lap-detected", "high-operating-temperature"); empty if it
    /// isn't.
    pub degraded: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BatteryState {
    #[default]
    Unknown,
    Charging,
    Discharging,
    Empty,
    Full,
    /// Plugged in but not charging (held below a charge limit).
    PendingCharge,
    PendingDischarge,
}

impl BatteryState {
    pub(crate) fn from_upower(state: u32) -> Self {
        match state {
            1 => Self::Charging,
            2 => Self::Discharging,
            3 => Self::Empty,
            4 => Self::Full,
            5 => Self::PendingCharge,
            6 => Self::PendingDischarge,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Battery {
    /// UPower's object path, for changing its charge limit.
    pub path: String,
    /// "BAT0".
    pub name: String,
    pub percent: f64,
    pub state: BatteryState,
    /// Seconds; 0 when UPower doesn't know.
    pub time_to_empty: i64,
    pub time_to_full: i64,
    /// Wh: what it holds full now, and what it held new.
    pub energy_full: f64,
    pub energy_full_design: f64,
    /// What it holds full now, as a percentage of new.
    pub health: f64,
    pub cycles: Option<u32>,
    /// `None` when the laptop can't limit charging.
    pub charge_limit: Option<ChargeLimit>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChargeLimit {
    pub enabled: bool,
    /// It stops charging here, in percent.
    pub end: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceKind {
    Mouse,
    Keyboard,
    Headset,
    Headphones,
    Speakers,
    Gamepad,
    Tablet,
    Phone,
    Touchpad,
    Pen,
    #[default]
    Other,
}

impl DeviceKind {
    /// UPower's device types.
    pub(crate) fn from_upower(kind: u32) -> Self {
        match kind {
            5 => Self::Mouse,
            6 => Self::Keyboard,
            8 => Self::Phone,
            10 => Self::Tablet,
            12 => Self::Gamepad,
            13 => Self::Pen,
            14 => Self::Touchpad,
            17 => Self::Headset,
            18 => Self::Speakers,
            19 => Self::Headphones,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceBattery {
    pub path: String,
    /// "MX Master 3S".
    pub name: String,
    pub kind: DeviceKind,
    pub percent: f64,
    pub state: BatteryState,
    /// How UPower reaches it, from its native path: "Bluetooth", "Logitech receiver", "USB" or "".
    pub link: String,
}

/// logind's HandleLidSwitch, HandleLidSwitchExternalPower, HandleLidSwitchDocked and
/// HandlePowerKey: "suspend", "poweroff", "ignore", "lock" and so on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Logind {
    pub lid: String,
    pub lid_external_power: String,
    pub lid_docked: String,
    pub power_key: String,
}

impl Logind {
    /// logind leaves the lid to others (Hyprland's binds) in every case.
    pub fn leaves_lid(&self) -> bool {
        [&self.lid, &self.lid_external_power, &self.lid_docked].iter().all(|v| v.as_str() == "ignore")
    }

    pub fn leaves_power_key(&self) -> bool {
        self.power_key == "ignore"
    }
}
