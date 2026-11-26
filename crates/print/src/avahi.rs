//! Printers on the network, as Avahi finds them (mDNS/DNS-SD, `_ipp._tcp` and `_ipps._tcp`), over
//! its D-Bus API. CUPS's own device list needs admin rights and waits for its whole timeout, so
//! it's used only for USB. ipp-usb and Printer Applications advertise on the loopback interface:
//! those are USB printers that need no driver.

use std::collections::HashMap;

use futures_util::stream::{self, Stream, StreamExt};
use zbus::message::Type as MessageType;
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, MatchRule, MessageStream};

use crate::Error;
use crate::model::Printer;

pub const NAME: &str = "org.freedesktop.Avahi";
const SERVER: &str = "org.freedesktop.Avahi.Server";
const BROWSER: &str = "org.freedesktop.Avahi.ServiceBrowser";
pub const TYPES: [&str; 2] = ["_ipp._tcp", "_ipps._tcp"];

/// A printer service Avahi resolved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Service {
    /// The service's name ("Canon TS5300 series").
    pub name: String,
    /// `_ipp._tcp` or `_ipps._tcp`.
    pub kind: String,
    pub domain: String,
    pub host: String,
    pub port: u16,
    /// The network interface's index: on the loopback one, it's a USB printer (ipp-usb).
    pub interface: i32,
    pub txt: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Found(Service),
    Gone {
        name: String,
        kind: String,
    },
    /// Everything there was at the start has been found.
    AllForNow,
}

/// Avahi runs (or starts when asked).
pub async fn running(conn: &Connection) -> bool {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else { return false };
    if let Ok(name) = NAME.try_into()
        && dbus.name_has_owner(name).await.unwrap_or(false)
    {
        return true;
    }
    dbus.list_activatable_names().await.map(|n| n.iter().any(|n| n.as_str() == NAME)).unwrap_or(false)
}

fn txt(records: Vec<Vec<u8>>) -> HashMap<String, String> {
    records
        .into_iter()
        .map(|r| {
            let r = String::from_utf8_lossy(&r).into_owned();
            let (k, v) = r.split_once('=').unwrap_or((&r, ""));
            (k.to_ascii_lowercase(), v.to_string())
        })
        .collect()
}

type Resolved = (i32, i32, String, String, String, String, i32, String, u16, Vec<Vec<u8>>, u32);

async fn resolve(
    conn: &Connection,
    interface: i32,
    protocol: i32,
    name: &str,
    kind: &str,
    domain: &str,
) -> Option<Service> {
    let reply = conn
        .call_method(
            Some(NAME),
            "/",
            Some(SERVER),
            "ResolveService",
            &(interface, protocol, name, kind, domain, -1i32, 0u32),
        )
        .await
        .ok()?;
    let (iface, _, name, kind, domain, host, _, _address, port, records, _): Resolved =
        reply.body().deserialize().ok()?;
    Some(Service { name, kind, domain, host, port, interface: iface, txt: txt(records) })
}

/// Avahi's printers as they come and go, for as long as the stream is kept.
pub async fn browse(conn: &Connection) -> Result<impl Stream<Item = Event> + Unpin + use<>, Error> {
    // Listen first, so nothing found before the browsers start is missed.
    let rule = MatchRule::builder().msg_type(MessageType::Signal).sender(NAME)?.interface(BROWSER)?.build();
    let signals = MessageStream::for_match_rule(rule, conn, Some(256)).await?;
    let mut browsers = Vec::new();
    for kind in TYPES {
        // ServiceBrowserPrepare (Avahi 0.8) waits for Start, so no item comes before the match
        // rule; ServiceBrowserNew starts at once.
        let prepared = conn
            .call_method(Some(NAME), "/", Some(SERVER), "ServiceBrowserPrepare", &(-1i32, -1i32, kind, "", 0u32))
            .await;
        let path: OwnedObjectPath = match prepared {
            Ok(reply) => {
                let path: OwnedObjectPath = reply.body().deserialize()?;
                conn.call_method(Some(NAME), path.as_str(), Some(BROWSER), "Start", &()).await?;
                path
            }
            Err(_) => conn
                .call_method(Some(NAME), "/", Some(SERVER), "ServiceBrowserNew", &(-1i32, -1i32, kind, "", 0u32))
                .await?
                .body()
                .deserialize()?,
        };
        browsers.push(path);
    }
    let conn = conn.clone();
    let events = signals.filter_map(move |m| {
        let conn = conn.clone();
        async move {
            let m = m.ok()?;
            let member = m.header().member()?.to_string();
            match member.as_str() {
                "ItemNew" | "ItemRemove" => {
                    let (interface, protocol, name, kind, domain, _): (i32, i32, String, String, String, u32) =
                        m.body().deserialize().ok()?;
                    if member == "ItemRemove" {
                        return Some(Event::Gone { name, kind });
                    }
                    resolve(&conn, interface, protocol, &name, &kind, &domain).await.map(Event::Found)
                }
                "AllForNow" => Some(Event::AllForNow),
                _ => None,
            }
        }
    });
    // The browsers live as long as the stream: Avahi frees them when the connection closes.
    Ok(stream::unfold((events.boxed(), browsers), |(mut events, b)| async move {
        events.next().await.map(|e| (e, (events, b)))
    })
    .boxed())
}

// ---- one row per printer --------------------------------------------------------------------------

/// How a found printer is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Link {
    #[default]
    Network,
    Usb,
}

/// A printer that can be added.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Found {
    /// Stays the same while it's found: its UUID, or its service name.
    pub id: String,
    pub name: String,
    pub link: Link,
    /// The address CUPS keeps for it: DNS-SD's, which follows the printer when its IP changes;
    /// or a usb:// one.
    pub uri: String,
    /// IPP Everywhere or AirPrint: added with nothing installed.
    pub driverless: bool,
    /// A queue for it exists.
    pub added: bool,
    /// A driver that's installed for it (a PPD's name), for a USB printer.
    pub ppd: Option<String>,
    /// The package with its driver, when none is installed ("splix").
    pub needs: Option<String>,
    pub make_and_model: String,
}

/// IPP Everywhere (pwg-raster) or AirPrint (URF): nothing to install.
pub fn driverless(txt: &HashMap<String, String>) -> bool {
    let pdl = txt.get("pdl").map(|p| p.to_ascii_lowercase()).unwrap_or_default();
    txt.contains_key("urf") || pdl.contains("image/pwg-raster") || pdl.contains("image/urf")
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// `ipps://Canon%20TS5300%20series._ipps._tcp.local/`: CUPS resolves it each time.
pub fn dnssd_uri(s: &Service) -> String {
    let scheme = if s.kind.starts_with("_ipps") { "ipps" } else { "ipp" };
    let domain = if s.domain.is_empty() { "local" } else { s.domain.as_str() };
    format!("{scheme}://{}.{}.{domain}/", encode(&s.name), s.kind)
}

fn decoded(uri: &str) -> String {
    let b = uri.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = uri.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A queue already prints to this service: by the name in a DNS-SD address, or by host and path.
fn has_queue(s: &Service, queues: &[Printer]) -> bool {
    let rp = s.txt.get("rp").map(String::as_str).unwrap_or("ipp/print");
    queues.iter().any(|q| {
        let uri = decoded(&q.device_uri);
        uri.contains(&format!("{}.{}", s.name, s.kind))
            || uri.contains(&format!("{}._ipp", s.name))
            || (!s.host.is_empty() && uri.contains(&format!("{}/{rp}", s.host.trim_end_matches('.'))))
            || (!s.host.is_empty() && uri.contains(&format!("{}:{}/{rp}", s.host.trim_end_matches('.'), s.port)))
    })
}

/// The services as one row per printer (by UUID, else by name; `_ipps` preferred), without this
/// computer's own shared queues. `loopback` is the loopback interface's index.
pub fn printers(services: &[Service], queues: &[Printer], loopback: i32) -> Vec<Found> {
    let mut by_id: Vec<(String, &Service)> = Vec::new();
    for s in services {
        // CUPS's own shared queues carry "printer-type".
        if s.txt.contains_key("printer-type") {
            continue;
        }
        let id = s.txt.get("uuid").filter(|u| !u.is_empty()).cloned().unwrap_or_else(|| s.name.clone());
        match by_id.iter_mut().find(|(i, _)| *i == id) {
            Some(slot) if s.kind.starts_with("_ipps") => slot.1 = s,
            Some(_) => {}
            None => by_id.push((id, s)),
        }
    }
    let mut out: Vec<Found> = by_id
        .into_iter()
        .map(|(id, s)| Found {
            id,
            name: s.name.clone(),
            link: if s.interface == loopback { Link::Usb } else { Link::Network },
            uri: dnssd_uri(s),
            driverless: driverless(&s.txt),
            added: has_queue(s, queues),
            ppd: None,
            needs: None,
            make_and_model: s.txt.get("ty").cloned().unwrap_or_default(),
        })
        .collect();
    out.sort_by_key(|f| (f.added, f.name.to_lowercase()));
    out
}

/// The loopback interface's index (usually 1).
pub fn loopback_index() -> i32 {
    std::fs::read_to_string("/sys/class/net/lo/ifindex").ok().and_then(|s| s.trim().parse().ok()).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(name: &str, kind: &str, txt: &[(&str, &str)], interface: i32) -> Service {
        Service {
            name: name.into(),
            kind: kind.into(),
            domain: "local".into(),
            host: format!("{}.local.", name.split(' ').next().unwrap().to_lowercase()),
            port: 631,
            interface,
            txt: txt.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn one_row_per_printer_and_its_kind() {
        let services = vec![
            svc(
                "Canon TS5300 series",
                "_ipp._tcp",
                &[("uuid", "u1"), ("pdl", "application/pdf,image/urf"), ("ty", "Canon TS5300")],
                2,
            ),
            svc("Canon TS5300 series", "_ipps._tcp", &[("uuid", "u1"), ("pdl", "application/pdf,image/urf")], 2),
            svc("Old Laser", "_ipp._tcp", &[("pdl", "application/postscript")], 2),
            svc("Office", "_ipp._tcp", &[("printer-type", "0x809006")], 2),
            svc("Epson ET-2850", "_ipp._tcp", &[("urf", "V1.4")], 1),
        ];
        let queue =
            Printer { name: "Old".into(), device_uri: "ipp://old.local:631/ipp/print".into(), ..Printer::default() };
        let found = printers(&services, &[queue], 1);
        let rows: Vec<_> =
            found.iter().map(|f| (f.name.as_str(), f.uri.as_str(), f.driverless, f.added, f.link)).collect();
        assert_eq!(
            rows,
            [
                ("Canon TS5300 series", "ipps://Canon%20TS5300%20series._ipps._tcp.local/", true, false, Link::Network),
                ("Epson ET-2850", "ipp://Epson%20ET-2850._ipp._tcp.local/", true, false, Link::Usb),
                ("Old Laser", "ipp://Old%20Laser._ipp._tcp.local/", false, true, Link::Network),
            ]
        );
    }

    #[test]
    fn a_dnssd_queue_counts_as_added() {
        let s = svc("Brother HL-L2350DW", "_ipp._tcp", &[], 2);
        let q = Printer { device_uri: "ipps://Brother%20HL-L2350DW._ipps._tcp.local/".into(), ..Printer::default() };
        assert!(has_queue(&s, &[q]));
        assert!(!has_queue(&s, &[]));
    }

    #[test]
    fn txt_records_are_read() {
        let t = txt(vec![b"rp=ipp/print".to_vec(), b"URF=V1.4,W8".to_vec(), b"Color=T".to_vec(), b"bare".to_vec()]);
        assert_eq!(t.get("rp").map(String::as_str), Some("ipp/print"));
        assert_eq!(t.get("urf").map(String::as_str), Some("V1.4,W8"));
        assert_eq!(t.get("bare").map(String::as_str), Some(""));
        assert!(driverless(&t));
    }
}
