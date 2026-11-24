//! Files over Bluetooth (OBEX Object Push), through BlueZ's obexd on the session bus: sending
//! files to a device, and an agent that asks the user about files a device sends.
//!
//! obexd saves received files in its own folder; the app moves them where the user wants them
//! once they're complete (see [`Transfer::filename`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::StreamExt;
use tokio::sync::{mpsc, oneshot};
use zbus::fdo::DBusProxy;
use zbus::message::Type as MessageType;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, MatchRule, Message, MessageStream};

use crate::Error;

/// obexd's well-known name on the session bus.
pub const OBEX_NAME: &str = "org.bluez.obex";
const ROOT: &str = "/org/bluez/obex";
pub const OBEX_AGENT_PATH: &str = "/ch/giosuel/Lituus/ObexAgent";
const AGENT_IFACE: &str = "org.bluez.obex.Agent1";
const AGENT_MANAGER: &str = "org.bluez.obex.AgentManager1";
const TRANSFER_IFACE: &str = "org.bluez.obex.Transfer1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// Where a transfer is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transfer {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub transferred: u64,
    /// "queued", "active", "suspended", "complete" or "error".
    pub status: String,
    /// Where the file is (sent from, or received into).
    pub filename: String,
}

impl Transfer {
    pub fn done(&self) -> bool {
        matches!(self.status.as_str(), "complete" | "error")
    }
}

/// A device wants to send a file. Answer with [`Self::accept`] or [`Self::reject`]; dropping it
/// rejects.
#[derive(Debug)]
pub struct IncomingFile {
    pub transfer: String,
    pub name: String,
    pub size: u64,
    /// A MIME type, when the sender gave one.
    pub kind: String,
    /// The sender's Bluetooth address.
    pub address: String,
    /// obexd's folder for received files.
    cache: PathBuf,
    reply: oneshot::Sender<Option<PathBuf>>,
}

impl IncomingFile {
    /// Accepts the file, saved in obexd's folder as `name`, or as `name (2)` and so on when that's
    /// taken: an earlier file may still be there, waiting to be moved, and obexd would write over
    /// it. Gives the path obexd writes the file to.
    pub fn accept(self, name: &str) -> std::io::Result<PathBuf> {
        match reserve(&self.cache, &safe_name(name)) {
            Ok(path) => {
                let _ = self.reply.send(Some(path.clone()));
                Ok(path)
            }
            Err(e) => {
                let _ = self.reply.send(None);
                Err(e)
            }
        }
    }

    pub fn reject(self) {
        let _ = self.reply.send(None);
    }
}

#[derive(Debug)]
pub enum ObexEvent {
    Push(IncomingFile),
    /// obexd gave up waiting: close the dialog.
    Cancel,
    /// A transfer moved on: its new Status, when that changed, and how much is through. Only this
    /// tells how a transfer ended: obexd drops a file it sent as soon as it ends, and uses one
    /// transfer for all the files a device sends in one go. Changes come in the order obexd made
    /// them, and a file's end before the request for the next one.
    Changed {
        transfer: String,
        status: Option<String>,
        transferred: Option<u64>,
    },
}

/// obexd on the session bus.
#[derive(Clone, Debug)]
pub struct Obex {
    conn: Connection,
    /// obexd's folder for received files.
    cache: PathBuf,
    /// The agent is registered, so it's registered again when obexd restarts.
    agent: Arc<AtomicBool>,
}

impl Obex {
    pub async fn session() -> Result<Self, Error> {
        Ok(Self::new(Connection::session().await?))
    }

    pub fn new(conn: Connection) -> Self {
        Self { conn, cache: cache_dir(), agent: Arc::default() }
    }

    /// With another folder for received files than obexd's usual one (for tests).
    pub fn with_cache(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cache = dir.into();
        self
    }

    async fn call<B, R>(&self, path: &str, iface: &str, method: &str, body: &B) -> Result<R, Error>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        let reply =
            self.conn.call_method(Some(OBEX_NAME), path, Some(iface), method, body).await.map_err(Error::from_dbus)?;
        Ok(reply.body().deserialize()?)
    }

    /// Starts sending files to a device; they go one after another. Returns the session (to
    /// close with [`Self::close`]) and the transfers, in order.
    pub async fn send_files(&self, address: &str, files: &[PathBuf]) -> Result<(String, Vec<String>), Error> {
        let mut args: HashMap<&str, Value> = HashMap::new();
        args.insert("Target", Value::from("opp"));
        let session: OwnedObjectPath =
            self.call(ROOT, "org.bluez.obex.Client1", "CreateSession", &(address, args)).await?;
        let mut transfers = Vec::new();
        for file in files {
            let file = file.to_string_lossy().into_owned();
            let sent: Result<(OwnedObjectPath, HashMap<String, OwnedValue>), Error> =
                self.call(session.as_str(), "org.bluez.obex.ObjectPush1", "SendFile", &(file,)).await;
            match sent {
                Ok((transfer, _)) => transfers.push(transfer.to_string()),
                Err(e) => {
                    // The files queued so far would go on being sent, with nobody to show it to,
                    // and the connection to the device would stay open.
                    let _ = self.close(session.as_str()).await;
                    return Err(e);
                }
            }
        }
        Ok((session.to_string(), transfers))
    }

    pub async fn close(&self, session: &str) -> Result<(), Error> {
        let path = OwnedObjectPath::try_from(session).map_err(zbus::Error::from)?;
        self.call::<_, ()>(ROOT, "org.bluez.obex.Client1", "RemoveSession", &(path,)).await
    }

    /// A transfer as it is now, or `None` once obexd has dropped it. How it ended came as its
    /// last [`ObexEvent::Changed`].
    pub async fn transfer(&self, path: &str) -> Result<Option<Transfer>, Error> {
        let props: Result<HashMap<String, OwnedValue>, Error> =
            self.call(path, PROPERTIES, "GetAll", &(TRANSFER_IFACE,)).await;
        let props = match props {
            Ok(p) => p,
            // UnknownObject, or UnknownMethod from a libdbus service such as obexd.
            Err(Error::DBus(e)) if e.to_string().contains("Unknown") => return Ok(None),
            Err(e) => return Err(e),
        };
        let text = |k: &str| props.get(k).and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
        let number = |k: &str| props.get(k).and_then(|v| u64::try_from(v.clone()).ok()).unwrap_or(0);
        Ok(Some(Transfer {
            path: path.into(),
            name: text("Name"),
            size: number("Size"),
            transferred: number("Transferred"),
            status: text("Status"),
            filename: text("Filename"),
        }))
    }

    pub async fn cancel(&self, transfer: &str) -> Result<(), Error> {
        self.call::<_, ()>(transfer, TRANSFER_IFACE, "Cancel", &()).await
    }

    /// Follows obexd: every change to a transfer comes as [`ObexEvent::Changed`], and once
    /// [`Self::register_agent`] is done, every file a device wants to send as
    /// [`ObexEvent::Push`]. Stops when the receiver is dropped.
    pub async fn follow(&self) -> Result<mpsc::UnboundedReceiver<ObexEvent>, Error> {
        let dbus = DBusProxy::new(&self.conn).await?;
        // obexd forgets its agent when it restarts.
        let owners = MatchRule::builder()
            .msg_type(MessageType::Signal)
            .sender("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg(0, OBEX_NAME)?
            .build();
        let transfers = MatchRule::builder()
            .msg_type(MessageType::Signal)
            .sender(OBEX_NAME)?
            .interface(PROPERTIES)?
            .member("PropertiesChanged")?
            .path_namespace(ROOT)?
            .arg(0, TRANSFER_IFACE)?
            .build();
        dbus.add_match_rule(owners).await.map_err(zbus::Error::from)?;
        dbus.add_match_rule(transfers).await.map_err(zbus::Error::from)?;
        // Everything the connection gets, read in one place and in order: a file's end comes
        // before the request for the next one. Nothing here may wait for another call while
        // messages pile up: zbus holds 64, then stops reading, and that call's reply never
        // comes (01-2).
        let mut messages = MessageStream::from(&self.conn);
        let mut early = Vec::new();
        let ask = dbus.get_name_owner(OBEX_NAME.try_into().expect("a valid name"));
        let mut ask = std::pin::pin!(ask);
        let owner = loop {
            tokio::select! {
                owner = &mut ask => break owner.ok().map(|o| o.to_string()),
                Some(Ok(message)) = messages.next() => early.push(message),
            }
        };
        let (events, receiver) = mpsc::unbounded_channel();
        tokio::spawn(serve(self.clone(), owner, early, messages, events));
        Ok(receiver)
    }

    /// Registers the agent for files devices send; they come to [`Self::follow`]'s receiver.
    pub async fn register_agent(&self) -> Result<(), Error> {
        self.agent.store(true, Ordering::SeqCst);
        self.register().await
    }

    async fn register(&self) -> Result<(), Error> {
        let path = OwnedObjectPath::try_from(OBEX_AGENT_PATH).expect("a valid path");
        self.call::<_, ()>(ROOT, AGENT_MANAGER, "RegisterAgent", &(path,)).await
    }

    async fn sender_address(&self, session: &str) -> String {
        let props: Result<HashMap<String, OwnedValue>, Error> =
            self.call(session, PROPERTIES, "GetAll", &("org.bluez.obex.Session1",)).await;
        props.ok().and_then(|p| p.get("Destination").and_then(|v| String::try_from(v.clone()).ok())).unwrap_or_default()
    }
}

/// Reads everything the connection gets: obexd's owner changing, its transfers' changes, and
/// its calls to the agent. `owner` is obexd's unique name: only obexd may ask, or report.
async fn serve(
    obex: Obex,
    mut owner: Option<String>,
    early: Vec<Message>,
    mut messages: MessageStream,
    events: mpsc::UnboundedSender<ObexEvent>,
) {
    let mut early = early.into_iter();
    loop {
        let message = match early.next() {
            Some(m) => m,
            None => match messages.next().await {
                Some(Ok(m)) => m,
                Some(Err(_)) => continue,
                None => break,
            },
        };
        if events.is_closed() {
            break;
        }
        let header = message.header();
        let sender = header.sender().map(|s| s.to_string());
        let from_obex = sender.is_some() && sender == owner;
        let member = header.member().map(|m| m.to_string()).unwrap_or_default();
        let interface = header.interface().map(|i| i.to_string()).unwrap_or_default();
        let path = header.path().map(|p| p.to_string()).unwrap_or_default();
        match header.message_type() {
            MessageType::Signal
                if member == "NameOwnerChanged" && sender.as_deref() == Some("org.freedesktop.DBus") =>
            {
                let Ok((name, _, new)) = message.body().deserialize::<(String, String, String)>() else { continue };
                if name != OBEX_NAME {
                    continue;
                }
                owner = (!new.is_empty()).then_some(new);
                if owner.is_some() && obex.agent.load(Ordering::SeqCst) {
                    let obex = obex.clone();
                    tokio::spawn(async move {
                        let _ = obex.register().await;
                    });
                }
            }
            MessageType::Signal if member == "PropertiesChanged" && from_obex && path.starts_with(ROOT) => {
                type Changed = (String, HashMap<String, OwnedValue>, Vec<String>);
                let Ok((iface, changed, _)) = message.body().deserialize::<Changed>() else { continue };
                if iface != TRANSFER_IFACE {
                    continue;
                }
                let status = changed.get("Status").and_then(|v| String::try_from(v.clone()).ok());
                let transferred = changed.get("Transferred").and_then(|v| u64::try_from(v.clone()).ok());
                if status.is_some() || transferred.is_some() {
                    let _ = events.send(ObexEvent::Changed { transfer: path, status, transferred });
                }
            }
            MessageType::MethodCall if path == OBEX_AGENT_PATH => {
                let conn = obex.conn.clone();
                if interface != AGENT_IFACE {
                    let _ = conn
                        .reply_error(&header, "org.freedesktop.DBus.Error.UnknownMethod", &("Unknown method",))
                        .await;
                } else if !from_obex {
                    let _ = conn.reply_error(&header, "org.bluez.obex.Error.Rejected", &("Not obexd",)).await;
                } else if member == "AuthorizePush" {
                    tokio::spawn(authorize(obex.clone(), message.clone(), events.clone()));
                } else {
                    if member == "Cancel" {
                        let _ = events.send(ObexEvent::Cancel);
                    }
                    let _ = conn.reply(&header, &()).await;
                }
            }
            _ => {}
        }
    }
}

/// obexd asks about a file a device sends: hand it to the app, and answer with where to save it.
async fn authorize(obex: Obex, message: Message, events: mpsc::UnboundedSender<ObexEvent>) {
    let header = message.header();
    let conn = obex.conn.clone();
    let Ok((transfer,)) = message.body().deserialize::<(OwnedObjectPath,)>() else { return };
    let props: HashMap<String, OwnedValue> =
        obex.call(transfer.as_str(), PROPERTIES, "GetAll", &(TRANSFER_IFACE,)).await.unwrap_or_default();
    let text = |k: &str| props.get(k).and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
    let session = props
        .get("Session")
        .and_then(|v| OwnedObjectPath::try_from(v.clone()).ok())
        .map(|p| p.to_string())
        .unwrap_or_default();
    let (reply, answer) = oneshot::channel();
    let file = IncomingFile {
        transfer: transfer.to_string(),
        name: text("Name"),
        size: props.get("Size").and_then(|v| u64::try_from(v.clone()).ok()).unwrap_or(0),
        kind: text("Type"),
        address: obex.sender_address(&session).await,
        cache: obex.cache.clone(),
        reply,
    };
    if events.send(ObexEvent::Push(file)).is_err() {
        let _ = conn.reply_error(&header, "org.bluez.obex.Error.Rejected", &("Not now",)).await;
        return;
    }
    match answer.await {
        Ok(Some(path)) => {
            let _ = conn.reply(&header, &(path.to_string_lossy().into_owned(),)).await;
        }
        _ => {
            let _ = conn.reply_error(&header, "org.bluez.obex.Error.Rejected", &("Declined",)).await;
        }
    }
}

/// obexd's folder for files it receives: `$XDG_CACHE_HOME/obexd`.
pub fn cache_dir() -> PathBuf {
    match std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        Some(c) => PathBuf::from(c).join("obexd"),
        None => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".cache/obexd"),
    }
}

/// A sender's file name, without folders or a leading dot.
pub fn safe_name(name: &str) -> String {
    let base = Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let base = base.trim_start_matches('.').to_owned();
    if base.is_empty() { "file".to_owned() } else { base }
}

/// `dir/name`, or `dir/name (2).ext` and so on if that's taken.
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    candidates(dir, name).find(|p| !p.exists()).expect("a free name")
}

/// Like [`free_path`], but creates the file (empty), so that it's taken from then on.
fn reserve(dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for path in candidates(dir, name) {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    unreachable!("the names go on")
}

/// `dir/name`, `dir/name (2).ext`, `dir/name (3).ext` and so on.
fn candidates(dir: &Path, name: &str) -> impl Iterator<Item = PathBuf> {
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let dir = dir.to_owned();
    std::iter::once(dir.join(name)).chain((2..).map(move |n| dir.join(format!("{stem} ({n}){ext}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_stay_in_their_folder() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name(".bashrc"), "bashrc");
        assert_eq!(safe_name("PXL_1.jpg"), "PXL_1.jpg");
        assert_eq!(safe_name(""), "file");
    }

    #[test]
    fn taken_names_get_a_number() {
        let dir = std::env::temp_dir().join(format!("lituus-obex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_path(&dir, "a.jpg"), dir.join("a.jpg"));
        std::fs::write(dir.join("a.jpg"), b"x").unwrap();
        assert_eq!(free_path(&dir, "a.jpg"), dir.join("a (2).jpg"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two files with the same name, both accepted before either is moved: each gets its own.
    #[test]
    fn a_reserved_name_is_taken() {
        let dir = std::env::temp_dir().join(format!("lituus-obex-reserve-{}", std::process::id()));
        assert_eq!(reserve(&dir, "image.jpg").unwrap(), dir.join("image.jpg"));
        assert_eq!(reserve(&dir, "image.jpg").unwrap(), dir.join("image (2).jpg"));
        assert_eq!(reserve(&dir, "notes").unwrap(), dir.join("notes"));
        assert_eq!(reserve(&dir, "notes").unwrap(), dir.join("notes (2)"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
