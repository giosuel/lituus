//! Against a mock obexd (scripts/mock-obex.py) on a private bus: sending files, and the agent
//! that asks about files a device sends. Runs only with LITUUS_DBUSMOCK=1, as the BlueZ tests.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use lituus_bt::obex::{OBEX_AGENT_PATH, OBEX_NAME, Obex};
use lituus_bt::{IncomingFile, ObexEvent};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::UnboundedReceiver;
use zbus::Connection;
use zbus::zvariant::{ObjectPath, Value};

const PHONE: &str = "58:24:29:B1:0E:77";

/// A private bus. None without LITUUS_DBUSMOCK=1.
async fn private_bus() -> Option<(Child, String)> {
    if std::env::var("LITUUS_DBUSMOCK").as_deref() != Ok("1") {
        eprintln!("skipped: set LITUUS_DBUSMOCK=1 (scripts/test.sh does) to run against the mock obexd");
        return None;
    }
    let mut daemon = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("dbus-daemon runs");
    let mut address = String::new();
    BufReader::new(daemon.stdout.take().unwrap()).read_line(&mut address).await.unwrap();
    Some((daemon, address.trim().to_owned()))
}

async fn connect(address: &str) -> Connection {
    zbus::connection::Builder::address(address).unwrap().build().await.unwrap()
}

/// A folder for the test's files.
fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lituus-obex-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A private bus with the mock obexd on it, and a folder for the test's files. None without
/// LITUUS_DBUSMOCK=1.
async fn mock_obexd(tag: &str) -> Option<(Child, Child, Connection, PathBuf)> {
    let (daemon, address) = private_bus().await?;
    let python = std::env::var("LITUUS_PYTHON").unwrap_or_else(|_| "python3".into());
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/mock-obex.py");
    let mock =
        Command::new(python).arg(script).env("DBUS_SESSION_BUS_ADDRESS", &address).kill_on_drop(true).spawn().unwrap();
    let conn = connect(&address).await;
    let dbus = zbus::fdo::DBusProxy::new(&conn).await.unwrap();
    for _ in 0..100 {
        if dbus.name_has_owner(OBEX_NAME.try_into().unwrap()).await.unwrap() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Some((daemon, mock, conn, folder(tag)))
}

async fn next(events: &mut UnboundedReceiver<ObexEvent>) -> ObexEvent {
    tokio::time::timeout(Duration::from_secs(10), events.recv()).await.expect("an event in time").expect("events")
}

/// Waits for `transfer` to end, and gives how: "complete" or "error".
async fn ended(events: &mut UnboundedReceiver<ObexEvent>, transfer: &str) -> String {
    loop {
        if let ObexEvent::Changed { transfer: t, status: Some(status), .. } = next(events).await
            && t == transfer
            && matches!(status.as_str(), "complete" | "error")
        {
            return status;
        }
    }
}

/// Waits for a device to want to send a file.
async fn pushed(events: &mut UnboundedReceiver<ObexEvent>) -> IncomingFile {
    loop {
        if let ObexEvent::Push(file) = next(events).await {
            return file;
        }
    }
}

/// The mock obexd plays a phone sending a file.
fn push(conn: &Connection, name: &'static str, size: u64) {
    let conn = conn.clone();
    tokio::spawn(async move {
        let _ = conn
            .call_method(Some(OBEX_NAME), "/org/bluez/obex", Some("org.bluez.obex.Mock"), "Push", &(PHONE, name, size))
            .await;
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_and_receives_files() {
    let Some((_daemon, _mock, conn, dir)) = mock_obexd("test").await else { return };
    let obex = Obex::new(conn.clone()).with_cache(dir.join("obexd"));
    let mut events = obex.follow().await.unwrap();

    // Sending two files: they go one after the other and finish.
    let (a, b) = (dir.join("a.jpg"), dir.join("b.pdf"));
    std::fs::write(&a, vec![1u8; 4000]).unwrap();
    std::fs::write(&b, vec![2u8; 1000]).unwrap();
    let (session, transfers) = obex.send_files(PHONE, &[a, b]).await.unwrap();
    assert_eq!(transfers.len(), 2);
    let first = obex.transfer(&transfers[0]).await.unwrap().expect("there while it's sent");
    assert_eq!((first.name.as_str(), first.size), ("a.jpg", 4000));
    assert_eq!(ended(&mut events, &transfers[0]).await, "complete");
    assert_eq!(ended(&mut events, &transfers[1]).await, "complete");
    obex.close(&session).await.unwrap();

    // A phone sends a photo: the agent asks, and the file lands in obexd's folder.
    obex.register_agent().await.unwrap();
    push(&conn, "../PXL_1.jpg", 3000);
    let file = pushed(&mut events).await;
    assert_eq!((file.name.as_str(), file.size, file.address.as_str()), ("../PXL_1.jpg", 3000, PHONE));
    let transfer = file.transfer.clone();
    let path = file.accept("../PXL_1.jpg").unwrap();
    assert_eq!(ended(&mut events, &transfer).await, "complete");
    // The sender's "../" doesn't take it out of the folder.
    assert_eq!(path, dir.join("obexd/PXL_1.jpg"));
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 3000);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The phone declines a file (or goes out of range): obexd sets the transfer's Status to "error"
/// and drops the object in the same breath. That must not read as sent (04-1).
#[tokio::test(flavor = "multi_thread")]
async fn a_send_the_phone_declined_is_not_sent() {
    let Some((_daemon, _mock, conn, dir)) = mock_obexd("declined").await else { return };
    let obex = Obex::new(conn.clone()).with_cache(dir.join("obexd"));
    let mut events = obex.follow().await.unwrap();
    let a = dir.join("a.jpg");
    std::fs::write(&a, vec![1u8; 4000]).unwrap();
    let (session, transfers) = obex.send_files(PHONE, &[a]).await.unwrap();
    assert_eq!(obex.transfer(&transfers[0]).await.unwrap().map(|t| t.status).as_deref(), Some("active"));
    let path = ObjectPath::try_from(transfers[0].as_str()).unwrap();
    conn.call_method(Some(OBEX_NAME), "/org/bluez/obex", Some("org.bluez.obex.Mock"), "Fail", &(path,)).await.unwrap();
    let how = ended(&mut events, &transfers[0]).await;
    let after = obex.transfer(&transfers[0]).await.unwrap();
    obex.close(&session).await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(how, "error", "a file the phone declined reads as {how:?}");
    assert_eq!(after, None, "obexd dropped the transfer, but it reads as {after:?}");
}

/// One of the files can't be sent (gone since it was picked): the files before it aren't sent
/// either, and the connection to the phone is closed (04-4).
#[tokio::test(flavor = "multi_thread")]
async fn a_send_that_fails_half_way_leaves_no_session() {
    let Some((_daemon, _mock, conn, dir)) = mock_obexd("half-way").await else { return };
    let obex = Obex::new(conn.clone()).with_cache(dir.join("obexd"));
    let a = dir.join("a.jpg");
    std::fs::write(&a, vec![1u8; 4000]).unwrap();
    assert!(obex.send_files(PHONE, &[a, dir.join("gone.jpg")]).await.is_err());
    // The mock numbers sessions from 0, and transfers in a session too.
    let session = conn
        .call_method(
            Some(OBEX_NAME),
            "/org/bluez/obex/client/session0",
            Some("org.freedesktop.DBus.Properties"),
            "GetAll",
            &("org.bluez.obex.Session1",),
        )
        .await;
    let first = obex.transfer("/org/bluez/obex/client/session0/transfer0").await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(session.is_err(), "the session to the phone is still open");
    assert_eq!(first, None, "the first file is still being sent");
}

/// Two pushes with the same name: each needs its own file in obexd's folder, or the second
/// overwrites the first before it's moved (04-3).
#[tokio::test(flavor = "multi_thread")]
async fn files_with_the_same_name_dont_overwrite_each_other() {
    let Some((_daemon, _mock, conn, dir)) = mock_obexd("same-name").await else { return };
    let obex = Obex::new(conn.clone()).with_cache(dir.join("obexd"));
    let mut events = obex.follow().await.unwrap();
    obex.register_agent().await.unwrap();
    let mut paths = Vec::new();
    for size in [3000u64, 1000] {
        push(&conn, "image.jpg", size);
        let file = pushed(&mut events).await;
        let transfer = file.transfer.clone();
        paths.push(file.accept("image.jpg").unwrap());
        assert_eq!(ended(&mut events, &transfer).await, "complete");
    }
    let first = std::fs::metadata(&paths[0]).map(|m| m.len());
    std::fs::remove_dir_all(&dir).unwrap();
    assert_ne!(paths[0], paths[1], "both files were written to {}", paths[0].display());
    assert_eq!(first.unwrap(), 3000, "the first file survives");
}

/// A phone sends two files in one go, over one transfer (obexd 5.86 and later), and aborts the
/// second half-way. The first file's end comes before the request for the second, so the app
/// can tell which file each end is for (04-2).
#[tokio::test(flavor = "multi_thread")]
async fn files_sent_in_one_go_end_one_by_one() {
    let Some((_daemon, _mock, conn, dir)) = mock_obexd("batch").await else { return };
    let obex = Obex::new(conn.clone()).with_cache(dir.join("obexd"));
    let mut events = obex.follow().await.unwrap();
    obex.register_agent().await.unwrap();
    let batch = conn.clone();
    tokio::spawn(async move {
        let files = vec![("a.jpg", 3000u64), ("b.jpg", 1000)];
        let _ = batch
            .call_method(
                Some(OBEX_NAME),
                "/org/bluez/obex",
                Some("org.bluez.obex.Mock"),
                "PushBatch",
                &(PHONE, files, 1i32, 500u32),
            )
            .await;
    });
    let mut seen = Vec::new();
    let mut paths = Vec::new();
    while seen.last().is_none_or(|s: &String| s != "error") {
        match next(&mut events).await {
            ObexEvent::Push(file) => {
                seen.push(format!("push {}", file.name));
                let name = file.name.clone();
                paths.push(file.accept(&name).unwrap());
            }
            ObexEvent::Changed { status: Some(status), .. } if matches!(status.as_str(), "complete" | "error") => {
                seen.push(status)
            }
            _ => {}
        }
    }
    let first = std::fs::read(&paths[0]);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(seen, ["push a.jpg", "complete", "push b.jpg", "error"]);
    assert_eq!(first.unwrap(), b"1".repeat(3000), "a.jpg is whole");
}

/// Plays obexd's agent manager, so the test can call the agent as obexd would.
struct AgentManager;

#[zbus::interface(name = "org.bluez.obex.AgentManager1")]
impl AgentManager {
    fn register_agent(&self, _agent: zbus::zvariant::OwnedObjectPath) {}
}

/// A request with a burst of other messages right behind it (a file's progress, another
/// device's): the agent still gets it, and the connection still works. zbus queues 64 messages
/// for the reader and then stops reading; the agent used to wait for obexd's owner there, and
/// with 65 messages behind the request that answer never came (01-2). On one thread, so the
/// whole burst is there before the agent reads the request.
#[tokio::test]
async fn a_request_in_a_burst_of_messages_is_still_answered() {
    let Some((_daemon, address)) = private_bus().await else { return };
    let obexd = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .name(OBEX_NAME)
        .unwrap()
        .serve_at("/org/bluez/obex", AgentManager)
        .unwrap()
        .build()
        .await
        .unwrap();
    let app = connect(&address).await;
    let app_name = app.unique_name().unwrap().to_string();
    let dir = folder("burst");
    let obex = Obex::new(app.clone()).with_cache(&dir);
    let mut events = obex.follow().await.unwrap();
    obex.register_agent().await.unwrap();

    let transfer = "/org/bluez/obex/server/session0/transfer0";
    let ask = zbus::Message::method_call(OBEX_AGENT_PATH, "AuthorizePush")
        .unwrap()
        .destination(app_name.as_str())
        .unwrap()
        .interface("org.bluez.obex.Agent1")
        .unwrap()
        .build(&(ObjectPath::try_from(transfer).unwrap(),))
        .unwrap();
    obexd.send(&ask).await.unwrap();
    for n in 0..200u64 {
        let changed = HashMap::from([("Transferred", Value::from(n))]);
        let signal = zbus::Message::signal(transfer, "org.freedesktop.DBus.Properties", "PropertiesChanged")
            .unwrap()
            .destination(app_name.as_str())
            .unwrap()
            .build(&("org.bluez.obex.Transfer1", changed, Vec::<String>::new()))
            .unwrap();
        obexd.send(&signal).await.unwrap();
    }
    let asked = tokio::time::timeout(Duration::from_secs(5), pushed(&mut events)).await;
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(asked.is_ok(), "the request never reached the app: the agent's connection is stuck");
    let answers = tokio::time::timeout(Duration::from_secs(5), async {
        zbus::fdo::DBusProxy::new(&app).await.unwrap().get_id().await.unwrap()
    })
    .await;
    assert!(answers.is_ok(), "calls on the agent's connection no longer return");
}
