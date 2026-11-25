//! Against python-dbusmock's UPower, power-profiles-daemon and logind on a private bus, so the real
//! system bus is never touched. Needs dbus-daemon and python-dbusmock, so it runs only with
//! LITUUS_DBUSMOCK=1 (scripts/test.sh and CI set it). LITUUS_PYTHON picks the interpreter.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use futures_util::StreamExt;
use lituus_power::{BatteryState, ChargeLimit, Client, DeviceKind};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use zbus::Connection;
use zbus::zvariant::{ObjectPath, Value};

const MOCK: &str = "org.freedesktop.DBus.Mock";

struct Bus {
    address: String,
    _daemon: Child,
    mocks: Vec<Child>,
}

impl Bus {
    async fn start(templates: &[(&str, &str)]) -> Option<Self> {
        if std::env::var("LITUUS_DBUSMOCK").as_deref() != Ok("1") {
            eprintln!("skipped: set LITUUS_DBUSMOCK=1 (scripts/test.sh does) to run against the mock power services");
            return None;
        }
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("dbus-daemon runs");
        let mut line = String::new();
        BufReader::new(daemon.stdout.take().unwrap()).read_line(&mut line).await.unwrap();
        let mut bus = Self { address: line.trim().to_owned(), _daemon: daemon, mocks: Vec::new() };
        let python = std::env::var("LITUUS_PYTHON").unwrap_or_else(|_| "python3".into());
        for (template, name) in templates {
            bus.mocks.push(
                Command::new(&python)
                    .args(["-m", "dbusmock", "--session", "--template", template])
                    .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
                    .stdout(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .expect("python-dbusmock runs"),
            );
            bus.wait_for(name).await;
        }
        Some(bus)
    }

    async fn connect(&self) -> Connection {
        zbus::connection::Builder::address(self.address.as_str()).unwrap().build().await.unwrap()
    }

    async fn wait_for(&self, name: &str) {
        let conn = self.connect().await;
        let dbus = zbus::fdo::DBusProxy::new(&conn).await.unwrap();
        for _ in 0..100 {
            if dbus.name_has_owner(name.try_into().unwrap()).await.unwrap() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("{name} didn't start");
    }

    async fn call<B, R>(&self, dest: &str, path: &str, iface: &str, method: &str, body: &B) -> R
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        let conn = self.connect().await;
        let reply = conn.call_method(Some(dest), path, Some(iface), method, body).await.unwrap();
        reply.body().deserialize().unwrap()
    }
}

impl Bus {
    /// Sets a mock UPower device's properties: the template's own through its SetDeviceProperties,
    /// the others (the charge limit's, newer than the template) added to the object.
    async fn device_props(&self, path: &str, props: &[(&str, Value<'_>)]) {
        let conn = self.connect().await;
        let upower = "org.freedesktop.UPower";
        for (name, value) in props {
            let added = conn
                .call_method(
                    Some(upower),
                    path,
                    Some(MOCK),
                    "AddProperty",
                    &("org.freedesktop.UPower.Device", *name, value),
                )
                .await;
            if added.is_err() {
                let one: HashMap<&str, &Value> = HashMap::from([(*name, value)]);
                let path = ObjectPath::try_from(path).unwrap();
                conn.call_method(
                    Some(upower),
                    "/org/freedesktop/UPower",
                    Some(MOCK),
                    "SetDeviceProperties",
                    &(path, one),
                )
                .await
                .unwrap();
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_the_battery_the_devices_the_profile_and_logind() {
    let Some(bus) = Bus::start(&[
        ("upower", "org.freedesktop.UPower"),
        ("power_profiles_daemon", "net.hadess.PowerProfiles"),
        ("logind", "org.freedesktop.login1"),
    ])
    .await
    else {
        return;
    };
    let upower = ("org.freedesktop.UPower", "/org/freedesktop/UPower");
    // A laptop's battery, charging, with a charge limit; and a mouse.
    // The mock gives the new device's path as a string.
    let bat: String = bus
        .call(upower.0, upower.1, MOCK, "AddChargingBattery", &("mock_BAT0", "Mock Battery", 64.0f64, 7800i64))
        .await;
    bus.device_props(
        &bat,
        &[
            ("NativePath", Value::from("BAT0")),
            ("EnergyFull", Value::from(52.1f64)),
            ("EnergyFullDesign", Value::from(57.0f64)),
            ("ChargeCycles", Value::from(312i32)),
            ("ChargeThresholdSupported", Value::from(true)),
            ("ChargeThresholdEnabled", Value::from(true)),
            ("ChargeEndThreshold", Value::from(80u32)),
        ],
    )
    .await;
    let mouse: String = bus
        .call(upower.0, upower.1, MOCK, "AddDischargingBattery", &("mock_mouse", "MX Master 3S", 80.0f64, 0i64))
        .await;
    bus.device_props(
        &mouse,
        &[
            ("Type", Value::from(5u32)),
            ("PowerSupply", Value::from(false)),
            ("NativePath", Value::from("hidpp_battery_0")),
        ],
    )
    .await;
    // logind's handling, as a stock systemd has it.
    for (name, value) in [
        ("HandleLidSwitch", "suspend"),
        ("HandleLidSwitchExternalPower", "suspend"),
        ("HandleLidSwitchDocked", "ignore"),
        ("HandlePowerKey", "poweroff"),
    ] {
        let () = bus
            .call(
                "org.freedesktop.login1",
                "/org/freedesktop/login1",
                MOCK,
                "AddProperty",
                &("org.freedesktop.login1.Manager", name, Value::from(value)),
            )
            .await;
    }

    let client = Client::new(bus.connect().await);
    let snap = client.snapshot().await;
    let battery = snap.battery.as_ref().expect("the laptop's battery");
    assert_eq!((battery.name.as_str(), battery.percent, battery.state), ("BAT0", 64.0, BatteryState::Charging));
    assert_eq!(battery.time_to_full, 7800);
    assert_eq!(battery.cycles, Some(312));
    assert_eq!(battery.charge_limit, Some(ChargeLimit { enabled: true, end: 80 }));
    assert_eq!(snap.devices.len(), 1, "{:?}", snap.devices);
    let m = &snap.devices[0];
    assert_eq!(
        (m.name.as_str(), m.kind, m.percent, m.link.as_str()),
        ("MX Master 3S", DeviceKind::Mouse, 80.0, "Logitech receiver")
    );
    let profiles = snap.profiles.as_ref().expect("power-profiles-daemon");
    assert_eq!(profiles.active, "balanced");
    assert!(profiles.available.contains(&"power-saver".to_owned()), "{:?}", profiles.available);
    let logind = snap.logind.as_ref().expect("logind");
    assert_eq!((logind.lid.as_str(), logind.power_key.as_str()), ("suspend", "poweroff"));
    assert!(!logind.leaves_lid() && !logind.leaves_power_key());

    // Changing the profile is heard as a change, and read back.
    let mut changes = client.changes().await.unwrap();
    client.set_profile("power-saver").await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), changes.next()).await.expect("a change").expect("the stream goes on");
    assert_eq!(client.snapshot().await.profiles.unwrap().active, "power-saver");
}

/// A desktop: no UPower battery, no power-profiles-daemon, no logind. Every part is just empty.
#[tokio::test(flavor = "multi_thread")]
async fn missing_services_leave_their_parts_empty() {
    let Some(bus) = Bus::start(&[]).await else { return };
    let client = Client::new(bus.connect().await);
    let snap = client.snapshot().await;
    assert_eq!(snap, lituus_power::Snapshot::default());
    assert!(matches!(client.set_profile("balanced").await, Err(lituus_power::Error::NoProfiles)));
}
