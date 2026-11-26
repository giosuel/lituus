//! The system bus: UPower, power-profiles-daemon (under UPower's name since 0.20, net.hadess before)
//! and systemd-logind. Each is read on its own, so one that's missing leaves only its part empty.

use std::collections::HashMap;

use futures_util::stream::{self, Stream, StreamExt};
use zbus::message::Type as MessageType;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, MatchRule, MessageStream};

use crate::model::{Battery, BatteryState, ChargeLimit, DeviceBattery, DeviceKind, Logind, Profiles, Snapshot};

pub const UPOWER: &str = "org.freedesktop.UPower";
const UPOWER_PATH: &str = "/org/freedesktop/UPower";
const DEVICE: &str = "org.freedesktop.UPower.Device";
/// power-profiles-daemon's names: its own under UPower's since 0.20, the old one before.
const PROFILES: [(&str, &str); 2] = [
    ("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];
const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("power-profiles-daemon isn't running")]
    NoProfiles,
    /// polkit or the bus policy said no.
    #[error("not allowed: {0}")]
    Denied(String),
    #[error(transparent)]
    DBus(#[from] zbus::Error),
}

impl From<zbus::fdo::Error> for Error {
    fn from(e: zbus::fdo::Error) -> Self {
        match e {
            zbus::fdo::Error::AccessDenied(m) | zbus::fdo::Error::AuthFailed(m) => Self::Denied(m),
            other => Self::DBus(other.into()),
        }
    }
}

/// A connection to the system bus (or, in tests, a private one).
#[derive(Clone)]
pub struct Client {
    conn: Connection,
}

type Props = HashMap<String, OwnedValue>;

fn get<T: TryFrom<OwnedValue>>(props: &Props, key: &str) -> Option<T> {
    props.get(key)?.try_clone().ok()?.try_into().ok()
}

impl Client {
    pub async fn system() -> Result<Self, Error> {
        Ok(Self::new(Connection::system().await?))
    }

    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    async fn all(&self, dest: &str, path: &str, iface: &str) -> Result<Props, Error> {
        let props = zbus::fdo::PropertiesProxy::builder(&self.conn).destination(dest)?.path(path)?.build().await?;
        Ok(props.get_all(iface.try_into().map_err(zbus::Error::from)?).await?)
    }

    /// Everything, read afresh.
    pub async fn snapshot(&self) -> Snapshot {
        let mut snap = Snapshot { profiles: self.profiles().await, logind: self.logind().await, ..Snapshot::default() };
        if let Ok(main) = self.all(UPOWER, UPOWER_PATH, UPOWER).await {
            snap.on_battery = get(&main, "OnBattery").unwrap_or(false);
            snap.lid = get(&main, "LidIsPresent").unwrap_or(false);
            for path in self.device_paths().await {
                let Ok(props) = self.all(UPOWER, path.as_str(), DEVICE).await else { continue };
                // A device not there (an empty bay) or not a battery (the mains) isn't shown.
                let kind: u32 = get(&props, "Type").unwrap_or(0);
                if !get::<bool>(&props, "IsPresent").unwrap_or(true) || kind == 1 || kind == 0 {
                    continue;
                }
                if kind == 2 && get(&props, "PowerSupply").unwrap_or(false) {
                    if snap.battery.is_none() {
                        snap.battery = Some(battery(path.as_str(), &props));
                    }
                } else {
                    snap.devices.push(device(path.as_str(), &props));
                }
            }
        }
        snap
    }

    async fn device_paths(&self) -> Vec<OwnedObjectPath> {
        let reply = self.conn.call_method(Some(UPOWER), UPOWER_PATH, Some(UPOWER), "EnumerateDevices", &()).await;
        reply.ok().and_then(|m| m.body().deserialize::<Vec<OwnedObjectPath>>().ok()).unwrap_or_default()
    }

    async fn profiles(&self) -> Option<Profiles> {
        for (name, path) in PROFILES {
            let Ok(props) = self.all(name, path, name).await else { continue };
            let available: Vec<Props> = get(&props, "Profiles").unwrap_or_default();
            return Some(Profiles {
                active: get(&props, "ActiveProfile").unwrap_or_default(),
                available: available.iter().filter_map(|p| get::<String>(p, "Profile")).collect(),
                degraded: get(&props, "PerformanceDegraded").unwrap_or_default(),
            });
        }
        None
    }

    async fn logind(&self) -> Option<Logind> {
        let props = self.all(LOGIND, LOGIND_PATH, LOGIND_MANAGER).await.ok()?;
        let s = |key: &str| get::<String>(&props, key).unwrap_or_default();
        Some(Logind {
            lid: s("HandleLidSwitch"),
            lid_external_power: s("HandleLidSwitchExternalPower"),
            lid_docked: s("HandleLidSwitchDocked"),
            power_key: s("HandlePowerKey"),
        })
    }

    /// Switches the profile, on whichever name power-profiles-daemon answers.
    pub async fn set_profile(&self, profile: &str) -> Result<(), Error> {
        for (name, path) in PROFILES {
            let Ok(props) =
                zbus::fdo::PropertiesProxy::builder(&self.conn).destination(name)?.path(path)?.build().await
            else {
                continue;
            };
            match props.set(name.try_into().map_err(zbus::Error::from)?, "ActiveProfile", Value::from(profile)).await {
                Ok(()) => return Ok(()),
                Err(zbus::fdo::Error::ServiceUnknown(_)) | Err(zbus::fdo::Error::UnknownObject(_)) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(Error::NoProfiles)
    }

    /// Turns the battery's charge limit on or off (UPower's charge threshold).
    pub async fn set_charge_limit(&self, battery: &str, enabled: bool) -> Result<(), Error> {
        self.conn.call_method(Some(UPOWER), battery, Some(DEVICE), "EnableChargeThreshold", &(enabled,)).await?;
        Ok(())
    }

    /// One item each time UPower or power-profiles-daemon says something changed, or one of them
    /// comes or goes.
    pub async fn changes(&self) -> Result<impl Stream<Item = ()> + Send + 'static, Error> {
        let mut streams = Vec::new();
        for sender in [UPOWER, PROFILES[0].0, PROFILES[1].0] {
            let rule = MatchRule::builder().msg_type(MessageType::Signal).sender(sender)?.build();
            // Only what says something changed: python-dbusmock also sends MethodCalled for every
            // call it answers, and each read would start the next one, for ever.
            let changes = MessageStream::for_match_rule(rule, &self.conn, Some(256)).await?.filter(|m| {
                std::future::ready(m.as_ref().is_ok_and(|m| {
                    m.header().interface().is_some_and(|i| {
                        let i = i.as_str();
                        i == "org.freedesktop.DBus.Properties" || i == UPOWER
                    })
                }))
            });
            streams.push(changes.map(|_| ()).boxed());
            let owner = MatchRule::builder()
                .msg_type(MessageType::Signal)
                .sender("org.freedesktop.DBus")?
                .interface("org.freedesktop.DBus")?
                .member("NameOwnerChanged")?
                .arg(0, sender)?
                .build();
            streams.push(MessageStream::for_match_rule(owner, &self.conn, Some(16)).await?.map(|_| ()).boxed());
        }
        Ok(stream::select_all(streams))
    }
}

fn battery(path: &str, props: &Props) -> Battery {
    let native: String = get(props, "NativePath").unwrap_or_default();
    let cycles: i32 = get(props, "ChargeCycles").unwrap_or(-1);
    let full: f64 = get(props, "EnergyFull").unwrap_or(0.0);
    let design: f64 = get(props, "EnergyFullDesign").unwrap_or(0.0);
    let capacity: f64 = get(props, "Capacity").unwrap_or(0.0);
    Battery {
        path: path.to_owned(),
        name: native.rsplit('/').next().unwrap_or(&native).to_owned(),
        percent: get(props, "Percentage").unwrap_or(0.0),
        state: BatteryState::from_upower(get(props, "State").unwrap_or(0)),
        time_to_empty: get(props, "TimeToEmpty").unwrap_or(0),
        time_to_full: get(props, "TimeToFull").unwrap_or(0),
        energy_full: full,
        energy_full_design: design,
        health: if capacity > 0.0 {
            capacity
        } else if design > 0.0 {
            full / design * 100.0
        } else {
            0.0
        },
        cycles: u32::try_from(cycles).ok().filter(|_| cycles > 0),
        charge_limit: get::<bool>(props, "ChargeThresholdSupported").filter(|s| *s).map(|_| ChargeLimit {
            enabled: get(props, "ChargeThresholdEnabled").unwrap_or(false),
            end: get(props, "ChargeEndThreshold").unwrap_or(100),
        }),
    }
}

fn device(path: &str, props: &Props) -> DeviceBattery {
    let native: String = get(props, "NativePath").unwrap_or_default();
    let model: String = get(props, "Model").unwrap_or_default();
    let kind = DeviceKind::from_upower(get(props, "Type").unwrap_or(0));
    DeviceBattery {
        path: path.to_owned(),
        name: if model.trim().is_empty() { format!("{kind:?}") } else { model.trim().to_owned() },
        kind,
        percent: get(props, "Percentage").unwrap_or(0.0),
        state: BatteryState::from_upower(get(props, "State").unwrap_or(0)),
        link: link(&native).to_owned(),
    }
}

/// How UPower reaches a device, from its native path: bluez's are "/org/bluez/hci0/dev_…",
/// Logitech receivers' "hidpp_battery_0".
fn link(native: &str) -> &'static str {
    if native.contains("bluez") || native.contains("dev_") {
        "Bluetooth"
    } else if native.starts_with("hidpp") {
        "Logitech receiver"
    } else if native.contains("usb") || native.starts_with("hid-") || native.contains("ps-controller") {
        "USB"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_devices_link_comes_from_its_native_path() {
        assert_eq!(link("/org/bluez/hci0/dev_AA_BB"), "Bluetooth");
        assert_eq!(link("hidpp_battery_0"), "Logitech receiver");
        assert_eq!(link("hid-00:11:22-battery"), "USB");
        assert_eq!(link("BAT0"), "");
    }

    #[test]
    fn health_comes_from_capacity_or_the_energies() {
        let mut props = Props::new();
        props.insert("EnergyFull".into(), OwnedValue::from(52.1));
        props.insert("EnergyFullDesign".into(), OwnedValue::from(57.0));
        props.insert("ChargeCycles".into(), OwnedValue::from(-1i32));
        props.insert("NativePath".into(), OwnedValue::try_from(Value::from("BAT0")).unwrap());
        let b = battery("/org/freedesktop/UPower/devices/battery_BAT0", &props);
        assert!((b.health - 91.4).abs() < 0.1, "{}", b.health);
        assert_eq!((b.cycles, b.name.as_str(), b.charge_limit), (None, "BAT0", None));
        props.insert("Capacity".into(), OwnedValue::from(88.0));
        props.insert("ChargeCycles".into(), OwnedValue::from(312i32));
        props.insert("ChargeThresholdSupported".into(), OwnedValue::from(true));
        props.insert("ChargeThresholdEnabled".into(), OwnedValue::from(true));
        props.insert("ChargeEndThreshold".into(), OwnedValue::from(80u32));
        let b = battery("/b", &props);
        assert_eq!(b.health, 88.0);
        assert_eq!(b.cycles, Some(312));
        assert_eq!(b.charge_limit, Some(ChargeLimit { enabled: true, end: 80 }));
    }
}
