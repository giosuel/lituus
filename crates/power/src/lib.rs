//! Power through D-Bus: UPower's batteries (the computer's own and its devices'), the profile of
//! power-profiles-daemon, and how systemd-logind handles the lid and the power key; and
//! hypridle's idle times, read from anyone's hypridle.conf and written to Lituus's own.

mod client;
pub mod hypridle;
mod model;

pub use client::{Client, Error};
pub use model::{Battery, BatteryState, ChargeLimit, DeviceBattery, DeviceKind, Logind, Profiles, Snapshot};
