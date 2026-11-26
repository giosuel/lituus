//! The cups-pk-helper fallback and cupsd's change signals against python-dbusmock on a private bus. Needs dbus-daemon and
//! python-dbusmock, so it runs only with LITUUS_DBUSMOCK=1 (scripts/test.sh and CI set it).

use std::process::Stdio;
use std::time::Duration;

use lituus_print::pkhelper::{self, PkHelper};
use lituus_print::{Defaults, Error};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use zbus::Connection;
use zbus::zvariant::OwnedValue;

const MOCK: &str = "org.freedesktop.DBus.Mock";

struct Bus {
    address: String,
    _daemon: Child,
    _mock: Option<Child>,
}

async fn bus(with_helper: bool) -> Option<Bus> {
    if std::env::var("LITUUS_DBUSMOCK").as_deref() != Ok("1") {
        eprintln!("skipped: set LITUUS_DBUSMOCK=1 (scripts/test.sh does) to run against a mock cups-pk-helper");
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
    let address = line.trim().to_owned();
    let mock = if with_helper {
        let python = std::env::var("LITUUS_PYTHON").unwrap_or_else(|_| "python3".into());
        Some(
            Command::new(python)
                .args(["-m", "dbusmock", "--session", pkhelper::NAME, "/", pkhelper::NAME])
                .env("DBUS_SESSION_BUS_ADDRESS", &address)
                .stdout(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .expect("python-dbusmock runs"),
        )
    } else {
        None
    };
    let b = Bus { address, _daemon: daemon, _mock: mock };
    if with_helper {
        let conn = b.connect().await;
        let dbus = zbus::fdo::DBusProxy::new(&conn).await.unwrap();
        for _ in 0..100 {
            if dbus.name_has_owner(pkhelper::NAME.try_into().unwrap()).await.unwrap() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        for (name, sig, answer) in [
            ("PrinterDelete", "s", "''"),
            ("PrinterAddOptionDefault", "ssas", "''"),
            ("PrinterAdd", "sssss", "''"),
            ("JobCancelPurge", "ib", "'Not authorized'"),
        ] {
            let _: () = b.call(MOCK, "AddMethod", &(pkhelper::NAME, name, sig, "s", format!("ret = {answer}"))).await;
        }
    }
    Some(b)
}

impl Bus {
    async fn connect(&self) -> Connection {
        zbus::connection::Builder::address(self.address.as_str()).unwrap().build().await.unwrap()
    }

    async fn call<B, R>(&self, iface: &str, method: &str, body: &B) -> R
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        let conn = self.connect().await;
        let reply = conn.call_method(Some(pkhelper::NAME), "/", Some(iface), method, body).await.unwrap();
        reply.body().deserialize().unwrap()
    }

    async fn calls(&self, method: &str) -> Vec<Vec<OwnedValue>> {
        let calls: Vec<(u64, Vec<OwnedValue>)> = self.call(MOCK, "GetMethodCalls", &(method,)).await;
        calls.into_iter().map(|(_, args)| args).collect()
    }
}

fn text(v: &OwnedValue) -> String {
    v.try_clone().ok().and_then(|v| String::try_from(v).ok()).unwrap_or_default()
}

fn texts(v: &OwnedValue) -> Vec<String> {
    v.try_clone().ok().and_then(|v| Vec::<String>::try_from(v).ok()).unwrap_or_default()
}

#[tokio::test]
async fn changes_go_through_cups_pk_helper() {
    let Some(b) = bus(true).await else { return };
    let conn = b.connect().await;
    assert!(pkhelper::available(&conn).await);
    let pk = PkHelper::new(conn);

    pk.remove("Office").await.unwrap();
    assert_eq!(b.calls("PrinterDelete").await.iter().map(|a| text(&a[0])).collect::<Vec<_>>(), ["Office"]);

    let d = Defaults { media: Some("iso_a4_210x297mm".into()), quality: Some("high".into()), ..Defaults::default() };
    pk.set_options("Office", &d, &[("PageSize".into(), "A4".into())]).await.unwrap();
    let set: Vec<(String, String, Vec<String>)> =
        b.calls("PrinterAddOptionDefault").await.iter().map(|a| (text(&a[0]), text(&a[1]), texts(&a[2]))).collect();
    let want = [("media", "iso_a4_210x297mm"), ("print-quality", "5"), ("PageSize", "A4")];
    assert_eq!(set.len(), want.len(), "{set:?}");
    for ((printer, option, values), (o, v)) in set.iter().zip(want) {
        assert_eq!((printer.as_str(), option.as_str(), values.as_slice()), ("Office", o, &[v.to_string()][..]));
    }

    pk.add("New", "ipp://printer.local/ipp/print", "everywhere", "New printer", "").await.unwrap();
    let added = &b.calls("PrinterAdd").await[0];
    assert_eq!((text(&added[0]), text(&added[2])), ("New".to_string(), "everywhere".to_string()));

    // cups-pk-helper answers with the error's text.
    let got = pk.cancel(7).await;
    assert!(matches!(got, Err(Error::NotAllowed(_))), "{got:?}");
}

#[tokio::test]
async fn without_cups_pk_helper_nothing_is_offered() {
    let Some(b) = bus(false).await else { return };
    assert!(!pkhelper::available(&b.connect().await).await);
}

#[tokio::test]
async fn cupsds_signals_are_changes() {
    use futures_util::StreamExt;
    let Some(b) = bus(true).await else { return };
    let mut changes = lituus_print::changes(&b.connect().await).await.unwrap();
    let _: () = b
        .call(
            MOCK,
            "EmitSignal",
            &(lituus_print::SPOOLER, "QueueChanged", "s", vec![zbus::zvariant::Value::from("Office")]),
        )
        .await;
    let got = tokio::time::timeout(Duration::from_secs(5), changes.next()).await;
    assert!(matches!(got, Ok(Some(()))), "{got:?}");
}
