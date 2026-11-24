//! The hotspot: a Wi-Fi connection in access point mode that shares this computer's internet
//! connection (NetworkManager's "shared" IPv4 method runs dnsmasq for the devices that join).

use std::collections::HashMap;

use lituus_nm::Settings;
use zbus::zvariant::{OwnedValue, Value};

/// `802-11-wireless.band` by the Band choice: automatic, 2.4 GHz, 5 GHz.
pub const BANDS: [&str; 3] = ["", "bg", "a"];

fn put(s: &mut Settings, setting: &str, key: &str, value: Value<'_>) {
    s.entry(setting.to_owned()).or_default().insert(key.to_owned(), OwnedValue::try_from(value).expect("no fds"));
}

/// A new hotspot connection, like `nmcli device wifi hotspot` makes: WPA2 (CCMP only, so phones
/// that can't do WPA3 join too), the password kept by NetworkManager so it starts without a
/// keyring, and the connection the user's own.
pub fn settings(ssid: &str, password: &str, band: usize, uuid: &str, owner: Option<&str>) -> Settings {
    let mut s: Settings = HashMap::new();
    put(&mut s, "connection", "id", Value::from("Hotspot"));
    put(&mut s, "connection", "uuid", Value::from(uuid));
    put(&mut s, "connection", "type", Value::from("802-11-wireless"));
    put(&mut s, "connection", "autoconnect", Value::from(false));
    if let Some(owner) = owner {
        put(&mut s, "connection", "permissions", Value::from(vec![format!("user:{owner}:")]));
    }
    apply(&mut s, ssid, password, band);
    put(&mut s, "802-11-wireless", "mode", Value::from("ap"));
    put(&mut s, "802-11-wireless-security", "key-mgmt", Value::from("wpa-psk"));
    put(&mut s, "802-11-wireless-security", "proto", Value::from(vec!["rsn".to_owned()]));
    put(&mut s, "802-11-wireless-security", "pairwise", Value::from(vec!["ccmp".to_owned()]));
    put(&mut s, "802-11-wireless-security", "group", Value::from(vec!["ccmp".to_owned()]));
    put(&mut s, "802-11-wireless-security", "pmf", Value::from(1i32));
    put(&mut s, "802-11-wireless-security", "psk-flags", Value::from(0u32));
    put(&mut s, "ipv4", "method", Value::from("shared"));
    put(&mut s, "ipv6", "method", Value::from("ignore"));
    s
}

/// Writes the name, password and band into a hotspot's settings. An empty password leaves the
/// saved one: the update then carries no secret, and NetworkManager keeps it.
pub fn apply(s: &mut Settings, ssid: &str, password: &str, band: usize) {
    put(s, "802-11-wireless", "ssid", Value::from(ssid.as_bytes().to_vec()));
    match BANDS[band.min(2)] {
        "" => {
            if let Some(w) = s.get_mut("802-11-wireless") {
                w.remove("band");
                w.remove("channel");
            }
        }
        band => put(s, "802-11-wireless", "band", Value::from(band)),
    }
    if !password.is_empty() {
        put(s, "802-11-wireless-security", "psk", Value::from(password));
    }
}

/// Whether the hotspot's (name, password) are wrong: a name of 1 to 32 bytes, and a WPA2
/// password of 8 to 63 characters, or none to keep a saved hotspot's own.
pub fn invalid(ssid: &str, password: &str, saved: bool) -> (bool, bool) {
    let ssid = ssid.trim();
    let password = if password.is_empty() { !saved } else { !(8..=63).contains(&password.chars().count()) };
    (ssid.is_empty() || ssid.len() > 32, password)
}

/// What a suggested password is made of: lower case letters and digits, without the 0, 1, l and
/// o that are easy to mix up. 32 of them, so each random byte picks one evenly.
const ALPHABET: &[u8; 32] = b"23456789abcdefghijkmnpqrstuvwxyz";
/// 12 of them: 60 bits. WPA2 is cracked offline from one captured handshake, and the scheme is
/// public, so that's what an attacker has to try (two words and a number had 18 bits: seconds).
const LENGTH: usize = 12;

/// A password that's easy to type on a phone: twelve letters and digits in groups of four, such
/// as "k7mq-x2hd-9fwa". `None` when there are no random numbers to make one from.
pub fn new_password() -> Option<String> {
    let mut bytes = [0u8; LENGTH];
    let mut random = std::fs::File::open("/dev/urandom").ok()?;
    std::io::Read::read_exact(&mut random, &mut bytes).ok()?;
    let chars: Vec<char> = bytes.iter().map(|b| char::from(ALPHABET[usize::from(b % 32)])).collect();
    Some(chars.chunks(4).map(|group| group.iter().collect::<String>()).collect::<Vec<_>>().join("-"))
}

/// A device on the hotspot, from dnsmasq's lease file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    pub name: String,
    pub address: String,
    /// When the lease ends (seconds since 1970), for how long it's been there.
    pub expires: u64,
}

/// The devices on the hotspot of `interface`, if NetworkManager's lease file can be read.
pub fn clients(interface: &str) -> Vec<Client> {
    std::fs::read_to_string(format!("/var/lib/NetworkManager/dnsmasq-{interface}.leases"))
        .map(|text| parse_leases(&text))
        .unwrap_or_default()
}

/// dnsmasq leases: `expiry mac ip hostname client-id` per line, `*` for an unknown name.
pub fn parse_leases(text: &str) -> Vec<Client> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let (expires, mac, ip, name) = (f.first()?, f.get(1)?, f.get(2)?, f.get(3)?);
            Some(Client {
                name: if *name == "*" { mac.to_uppercase() } else { (*name).to_owned() },
                address: (*ip).to_owned(),
                expires: expires.parse().unwrap_or(0),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotspot_settings() {
        let s = settings("thinkpad-x1", "tidal-maple-41", 2, "u-1", Some("giosuel"));
        assert_eq!(<&str>::try_from(&*s["802-11-wireless"]["mode"]).unwrap(), "ap");
        assert_eq!(<&str>::try_from(&*s["802-11-wireless"]["band"]).unwrap(), "a");
        assert_eq!(<&str>::try_from(&*s["ipv4"]["method"]).unwrap(), "shared");
        assert_eq!(<&str>::try_from(&*s["802-11-wireless-security"]["psk"]).unwrap(), "tidal-maple-41");
    }

    /// Count what an attacker has to try.
    #[test]
    fn the_suggested_password_is_not_guessable() {
        let bits = (ALPHABET.len() as f64).log2() * LENGTH as f64;
        assert!(bits >= 60.0, "{bits:.1} bits: a captured WPA2 handshake is cracked offline");
        assert_eq!(256 % ALPHABET.len(), 0, "each letter as likely as the others");
        let p = new_password().unwrap();
        assert!(p.split('-').all(|g| g.len() == 4 && g.bytes().all(|b| ALPHABET.contains(&b))), "{p}");
        assert_ne!(p, new_password().unwrap());
    }

    #[test]
    fn names_and_passwords_are_checked() {
        assert_eq!(invalid("thinkpad-x1", "k7mq-x2hd-9fwa", false), (false, false));
        assert_eq!(invalid(" ", "abc", true), (true, true));
        assert_eq!(invalid(&"x".repeat(33), &"y".repeat(64), true), (true, true));
        // A saved hotspot whose password couldn't be read starts with the one it has.
        assert_eq!(invalid("thinkpad-x1", "", true), (false, false));
        assert_eq!(invalid("thinkpad-x1", "", false), (false, true));
        let mut s = settings("thinkpad-x1", "k7mq-x2hd-9fwa", 0, "u-1", None);
        s.get_mut("802-11-wireless-security").unwrap().remove("psk");
        apply(&mut s, "thinkpad-x1", "", 1);
        assert!(!s["802-11-wireless-security"].contains_key("psk"), "no secret: NetworkManager keeps its own");
    }

    #[test]
    fn passwords_and_leases() {
        let p = new_password().unwrap();
        assert!(p.len() >= 8 && p.matches('-').count() == 2, "{p}");
        let leases =
            "1792600000 aa:bb:cc:dd:ee:ff 10.42.0.12 Pixel-8 01:aa\n1792600100 11:22:33:44:55:66 10.42.0.37 * *\n";
        let c = parse_leases(leases);
        assert_eq!(c[0], Client { name: "Pixel-8".into(), address: "10.42.0.12".into(), expires: 1792600000 });
        assert_eq!(c[1].name, "11:22:33:44:55:66");
    }
}
