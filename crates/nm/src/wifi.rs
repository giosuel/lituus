//! Wi-Fi details derived from access point properties: security type, band and channel, signal
//! bars, and the grouping of access points into the networks a user picks from.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use crate::model::{AccessPoint, SavedConnection, Snapshot};

// NM80211ApFlags and NM80211ApSecurityFlags (nm-dbus-types).
const AP_FLAGS_PRIVACY: u32 = 0x1;
const SEC_KEY_MGMT_PSK: u32 = 0x100;
const SEC_KEY_MGMT_802_1X: u32 = 0x200;
const SEC_KEY_MGMT_SAE: u32 = 0x400;
const SEC_KEY_MGMT_OWE: u32 = 0x800;
const SEC_KEY_MGMT_OWE_TM: u32 = 0x1000;
const SEC_KEY_MGMT_EAP_SUITE_B_192: u32 = 0x2000;

/// How a network is secured, as far as its beacons tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Security {
    Open,
    /// Enhanced Open: encrypted, no password.
    Owe,
    Wep,
    Wpa,
    Wpa2,
    /// WPA3 where the client supports it, WPA2 otherwise (transition mode).
    Wpa2Wpa3,
    Wpa3,
    Enterprise,
    Wpa3Enterprise,
}

impl Security {
    /// From an access point's `Flags`, `WpaFlags` and `RsnFlags`.
    pub fn from_flags(flags: u32, wpa: u32, rsn: u32) -> Self {
        let any = wpa | rsn;
        // NetworkManager sets EAP_SUITE_B_192 without 802_1X for an access point whose only key
        // management is the 192-bit suite, so either bit means Enterprise.
        if any & (SEC_KEY_MGMT_802_1X | SEC_KEY_MGMT_EAP_SUITE_B_192) != 0 {
            if any & SEC_KEY_MGMT_EAP_SUITE_B_192 != 0 { Self::Wpa3Enterprise } else { Self::Enterprise }
        } else if rsn & SEC_KEY_MGMT_SAE != 0 {
            if rsn & SEC_KEY_MGMT_PSK != 0 { Self::Wpa2Wpa3 } else { Self::Wpa3 }
        } else if rsn & SEC_KEY_MGMT_PSK != 0 {
            Self::Wpa2
        } else if wpa & SEC_KEY_MGMT_PSK != 0 {
            Self::Wpa
        } else if rsn & SEC_KEY_MGMT_OWE != 0 && (flags & AP_FLAGS_PRIVACY != 0 || rsn & SEC_KEY_MGMT_OWE_TM == 0) {
            // In a transition pair (OWE_TM), the encrypted half (privacy set) is Enhanced Open,
            // and the open half stays open.
            Self::Owe
        } else if flags & AP_FLAGS_PRIVACY != 0 {
            Self::Wep
        } else {
            Self::Open
        }
    }

    /// Whether connecting asks for something (a password, or 802.1X credentials).
    pub fn needs_secrets(self) -> bool {
        !matches!(self, Self::Open | Self::Owe)
    }

    /// The short name the UI shows, e.g. "WPA2/WPA3".
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Owe => "Enhanced Open",
            Self::Wep => "WEP",
            Self::Wpa => "WPA",
            Self::Wpa2 => "WPA2",
            Self::Wpa2Wpa3 => "WPA2/WPA3",
            Self::Wpa3 => "WPA3",
            Self::Enterprise => "Enterprise",
            Self::Wpa3Enterprise => "WPA3 Enterprise",
        }
    }
}

/// A Wi-Fi frequency band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Band {
    Ghz2_4,
    Ghz5,
    Ghz6,
    Ghz60,
}

impl Band {
    /// The band of a frequency in MHz, if it's a Wi-Fi one.
    pub fn from_mhz(mhz: u32) -> Option<Self> {
        match mhz {
            2400..=2500 => Some(Self::Ghz2_4),
            4900..=5899 => Some(Self::Ghz5),
            5925..=7125 => Some(Self::Ghz6),
            57000..=71000 => Some(Self::Ghz60),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ghz2_4 => "2.4 GHz",
            Self::Ghz5 => "5 GHz",
            Self::Ghz6 => "6 GHz",
            Self::Ghz60 => "60 GHz",
        }
    }
}

/// The channel number of a frequency in MHz (IEEE 802.11 numbering per band).
pub fn channel(mhz: u32) -> Option<u32> {
    match Band::from_mhz(mhz)? {
        Band::Ghz2_4 if mhz == 2484 => Some(14),
        Band::Ghz2_4 => mhz.checked_sub(2407).map(|d| d / 5),
        Band::Ghz5 => mhz.checked_sub(5000).map(|d| d / 5),
        Band::Ghz6 => mhz.checked_sub(5950).map(|d| d / 5),
        Band::Ghz60 => mhz.checked_sub(56160).map(|d| d / 2160),
    }
}

/// Signal strength (0–100 %) as 0–4 bars.
pub fn bars(strength: u8) -> u8 {
    match strength {
        0 => 0,
        1..=24 => 1,
        25..=49 => 2,
        50..=74 => 3,
        _ => 4,
    }
}

/// One network as the user sees it: every access point with the same SSID and security.
#[derive(Clone, Debug, PartialEq)]
pub struct Network {
    pub ssid: Vec<u8>,
    pub security: Security,
    /// The access points, strongest first.
    pub access_points: Vec<AccessPoint>,
    /// Every band any of the access points is on, ascending.
    pub bands: Vec<Band>,
    /// The saved connection for this network, if there is one: the one in use, else the one
    /// NetworkManager would pick.
    pub saved: Option<String>,
    /// Whether the device is connected to it now.
    pub active: bool,
    /// The access point in use (an index into `access_points`), or else the strongest.
    current: usize,
}

impl Network {
    /// The SSID for display: invalid UTF-8 becomes U+FFFD.
    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.ssid).into_owned()
    }

    pub fn strongest(&self) -> &AccessPoint {
        &self.access_points[0]
    }

    pub fn strength(&self) -> u8 {
        self.strongest().strength
    }

    /// The access point the device is on while it's connected to this network; the strongest
    /// otherwise. What the network's signal, band and BSSID are about.
    pub fn current(&self) -> &AccessPoint {
        &self.access_points[self.current]
    }
}

/// Groups a Wi-Fi device's access points into networks, as the network list shows them: the
/// connected one first, then saved ones, then by signal strength. Hidden networks (empty SSID)
/// are left out; they're joined by name instead.
pub fn networks(access_points: &[AccessPoint], active_ap: Option<&str>, saved: &[SavedConnection]) -> Vec<Network> {
    group(access_points.iter(), active_ap, None, &saved.iter().collect::<Vec<_>>())
}

/// [`networks`], knowing which profile is in use (`active_uuid`), from the profiles that may be
/// used on the device.
fn group<'a>(
    access_points: impl Iterator<Item = &'a AccessPoint>,
    active_ap: Option<&str>,
    active_uuid: Option<&str>,
    saved: &[&SavedConnection],
) -> Vec<Network> {
    let mut groups: BTreeMap<(Vec<u8>, Security), Vec<AccessPoint>> = BTreeMap::new();
    for ap in access_points.filter(|ap| !ap.ssid.is_empty()) {
        groups.entry((ap.ssid.clone(), ap.security)).or_default().push(ap.clone());
    }
    let mut networks: Vec<Network> = groups
        .into_iter()
        .map(|((ssid, security), mut aps)| {
            aps.sort_by(|a, b| b.strength.cmp(&a.strength).then_with(|| a.bssid.cmp(&b.bssid)));
            let mut bands: Vec<Band> = aps.iter().filter_map(|ap| Band::from_mhz(ap.frequency)).collect();
            bands.sort();
            bands.dedup();
            let current = active_ap.and_then(|path| aps.iter().position(|ap| ap.path == path));
            // With more than one profile for the name: the one in use, else the one
            // NetworkManager would pick (highest priority, then used last). A hotspot's profile
            // runs an access point; it isn't one to join a network with.
            let saved = saved
                .iter()
                .filter(|c| !c.hotspot && c.wifi_ssid() == Some(&ssid[..]))
                .min_by_key(|c| {
                    (Some(c.uuid.as_str()) != active_uuid, Reverse(c.autoconnect_priority), Reverse(c.timestamp))
                })
                .map(|c| c.uuid.clone());
            let active = current.is_some();
            Network { ssid, security, access_points: aps, bands, saved, active, current: current.unwrap_or(0) }
        })
        .collect();
    networks.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then_with(|| b.saved.is_some().cmp(&a.saved.is_some()))
            .then_with(|| b.strength().cmp(&a.strength()))
            .then_with(|| a.ssid.cmp(&b.ssid))
    });
    networks
}

impl Snapshot {
    /// The networks seen by every Wi-Fi device, grouped as [`networks`] does, per device path.
    pub fn networks(&self) -> Vec<(String, Vec<Network>)> {
        self.devices
            .iter()
            .filter_map(|d| {
                let wifi = d.wifi.as_ref()?;
                let active = d.active_connection.as_deref().and_then(|a| self.active_connection(a));
                // The device's active access point, or else the one its active connection is on.
                let active_ap = wifi.active_access_point.as_deref().or_else(|| active?.specific_object.as_deref());
                // A profile bound to another interface can't be used on this one.
                let saved: Vec<&SavedConnection> = self
                    .connections
                    .iter()
                    .filter(|c| c.interface.as_deref().is_none_or(|i| i == d.interface))
                    .collect();
                let active_uuid = active.map(|a| a.uuid.as_str());
                Some((d.path.clone(), group(wifi.access_points.iter(), active_ap, active_uuid, &saved)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ConnectionKind;

    fn ap(path: &str, ssid: &str, bssid: &str, mhz: u32, strength: u8, security: Security) -> AccessPoint {
        AccessPoint {
            path: path.into(),
            ssid: ssid.as_bytes().to_vec(),
            bssid: bssid.into(),
            frequency: mhz,
            strength,
            security,
            max_bitrate: 0,
        }
    }

    fn saved_wifi(uuid: &str, ssid: &str) -> SavedConnection {
        SavedConnection {
            path: format!("/c/{uuid}"),
            uuid: uuid.into(),
            id: ssid.into(),
            kind: ConnectionKind::Wifi { ssid: ssid.as_bytes().to_vec() },
            autoconnect: true,
            autoconnect_priority: 0,
            interface: None,
            metered: None,
            hotspot: false,
            port: None,
            timestamp: 0,
        }
    }

    #[test]
    fn security_from_beacon_flags() {
        let (psk, sae, x1, b192, owe) = (0x100, 0x400, 0x200, 0x2000, 0x800);
        assert_eq!(Security::from_flags(0, 0, 0), Security::Open);
        assert_eq!(Security::from_flags(1, 0, 0), Security::Wep);
        assert_eq!(Security::from_flags(1, psk, 0), Security::Wpa);
        assert_eq!(Security::from_flags(1, psk, psk), Security::Wpa2);
        assert_eq!(Security::from_flags(1, 0, psk | sae), Security::Wpa2Wpa3);
        assert_eq!(Security::from_flags(1, 0, sae), Security::Wpa3);
        assert_eq!(Security::from_flags(1, 0, x1), Security::Enterprise);
        assert_eq!(Security::from_flags(1, 0, x1 | b192), Security::Wpa3Enterprise);
        assert_eq!(Security::from_flags(1, 0, owe), Security::Owe);
        // An open network advertising OWE transition mode is still open to connect to.
        assert_eq!(Security::from_flags(0, 0, owe | 0x1000), Security::Open);
        assert!(!Security::Owe.needs_secrets());
        assert!(Security::Wpa2.needs_secrets());
    }

    /// NetworkManager sets EAP_SUITE_B_192 (0x2000) without 802_1X (0x200) for an access point
    /// whose only key management is wpa-eap-suite-b-192.
    #[test]
    fn suite_b_192_only_is_wpa3_enterprise() {
        assert_eq!(Security::from_flags(1, 0, 0x2000), Security::Wpa3Enterprise);
    }

    /// An Enhanced Open transition pair: the encrypted half has OWE and OWE_TM, with privacy;
    /// the open half has OWE_TM alone, without.
    #[test]
    fn the_encrypted_half_of_an_owe_transition_pair_is_enhanced_open() {
        assert_eq!(Security::from_flags(1, 0, 0x800 | 0x1000), Security::Owe);
        assert_eq!(Security::from_flags(0, 0, 0x1000), Security::Open);
    }

    #[test]
    fn bands_and_channels() {
        assert_eq!(Band::from_mhz(2412), Some(Band::Ghz2_4));
        assert_eq!(channel(2412), Some(1));
        assert_eq!(channel(2484), Some(14));
        assert_eq!(Band::from_mhz(5180), Some(Band::Ghz5));
        assert_eq!(channel(5180), Some(36));
        assert_eq!(Band::from_mhz(5955), Some(Band::Ghz6));
        assert_eq!(channel(5955), Some(1));
        assert_eq!(Band::from_mhz(1000), None);
        assert_eq!(channel(1000), None);
    }

    #[test]
    fn signal_bars() {
        assert_eq!([0, 10, 30, 60, 90].map(bars), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn access_points_group_into_networks() {
        let aps = [
            ap("/ap/1", "Home", "AA", 2437, 40, Security::Wpa2),
            ap("/ap/2", "Home", "BB", 5180, 80, Security::Wpa2),
            ap("/ap/3", "Cafe", "CC", 2412, 90, Security::Open),
            ap("/ap/4", "Office", "DD", 5200, 30, Security::Enterprise),
            ap("/ap/5", "", "EE", 2412, 99, Security::Wpa2),
        ];
        let saved = [saved_wifi("u-office", "Office")];
        let nets = networks(&aps, Some("/ap/1"), &saved);
        let names: Vec<String> = nets.iter().map(Network::name).collect();
        // Connected first, then saved, then by strength; the hidden one is left out.
        assert_eq!(names, ["Home", "Office", "Cafe"]);
        let home = &nets[0];
        assert!(home.active);
        assert_eq!(home.strength(), 80);
        assert_eq!(home.strongest().bssid, "BB");
        // Connected through the weaker one: that's the one its signal and band are about.
        assert_eq!(home.current().bssid, "AA");
        assert_eq!(nets[1].current().bssid, "DD");
        assert_eq!(home.bands, [Band::Ghz2_4, Band::Ghz5]);
        assert_eq!(nets[1].saved.as_deref(), Some("u-office"));
        assert_eq!(nets[2].saved, None);
    }

    /// Two profiles for one name: the one NetworkManager would pick (priority, then used last),
    /// never the hotspot's.
    #[test]
    fn a_network_with_two_profiles_uses_the_one_networkmanager_picks() {
        let aps = [ap("/ap/1", "Home", "AA", 2437, 40, Security::Wpa2)];
        let mut old = saved_wifi("u-old", "Home");
        old.timestamp = 100;
        let mut new = saved_wifi("u-new", "Home");
        new.timestamp = 200;
        let mut hotspot = saved_wifi("u-hs", "Home");
        (hotspot.hotspot, hotspot.autoconnect_priority) = (true, 10);
        let picked = |saved: &[SavedConnection]| networks(&aps, None, saved)[0].saved.clone();
        assert_eq!(picked(&[hotspot.clone(), old.clone(), new.clone()]).as_deref(), Some("u-new"));
        old.autoconnect_priority = 1;
        assert_eq!(picked(&[hotspot.clone(), old, new]).as_deref(), Some("u-old"));
        assert_eq!(picked(&[hotspot]), None);
    }

    #[test]
    fn same_name_different_security_are_different_networks() {
        let aps =
            [ap("/ap/1", "Lab", "AA", 2412, 50, Security::Open), ap("/ap/2", "Lab", "BB", 2412, 60, Security::Wpa2)];
        assert_eq!(networks(&aps, None, &[]).len(), 2);
    }
}
