//! One Lituus per user: the first one owns `ch.giosuel.Lituus` on the session bus, and a
//! second start passes its request on instead (`lituus` shows the window, `lituus --quick`
//! the popup, `lituus --toggle-wifi` switches Wi-Fi; from a keybind or Waybar). Only one can be
//! NetworkManager's secret agent anyway.

use std::sync::mpsc;
use std::time::Duration;

use futures_util::StreamExt;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::message::Type as MessageType;
use zbus::{Connection, MessageStream};

const PATH: &str = "/ch/giosuel/Lituus";
const IFACE: &str = "ch.giosuel.Lituus1";

/// What a start asks the running Lituus for: the methods of `ch.giosuel.Lituus1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Show,
    Quick,
    ToggleWifi,
}

impl Request {
    fn method(self) -> &'static str {
        match self {
            Self::Show => "Show",
            Self::Quick => "Quick",
            Self::ToggleWifi => "ToggleWifi",
        }
    }

    fn from_method(name: &str) -> Option<Self> {
        [Self::Show, Self::Quick, Self::ToggleWifi].into_iter().find(|r| r.method() == name)
    }
}

/// Whether this process is the one to run.
pub enum Claim {
    /// It is: `handle` is called with what later starts ask for.
    First,
    /// Another one is running, and took the request.
    Passed,
}

/// Claims the name on a thread of its own, which then keeps answering requests. If another
/// process has it, that one gets `request` (with none, a second `lituus --background` just
/// leaves). Without a session bus there's nothing to share, so this process runs.
pub fn claim(app_id: &'static str, request: Option<Request>, handle: impl Fn(Request) + Send + 'static) -> Claim {
    let (result, claimed) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
        runtime.block_on(async move {
            let Ok(conn) = Connection::session().await else {
                let _ = result.send(Claim::First);
                return;
            };
            match claim_on(&conn, app_id, request).await {
                Claimed::Owner => {
                    let _ = result.send(Claim::First);
                    serve(&conn, handle).await;
                }
                Claimed::Passed => {
                    let _ = result.send(Claim::Passed);
                }
                // Another one has the name but didn't answer: this one runs, without it.
                Claimed::Unanswered => {
                    let _ = result.send(Claim::First);
                }
            }
        });
    });
    claimed.recv_timeout(Duration::from_secs(5)).unwrap_or(Claim::First)
}

/// Hands `request` to the running Lituus without claiming the name, for a start that does one
/// thing and leaves (`lituus --toggle-wifi`). False when none is running, or there's no session
/// bus.
pub fn pass(app_id: &str, request: Request) -> bool {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
    runtime.block_on(async {
        let Ok(conn) = Connection::session().await else { return false };
        call(&conn, app_id, request).await
    })
}

enum Claimed {
    Owner,
    Passed,
    Unanswered,
}

async fn claim_on(conn: &Connection, app_id: &str, request: Option<Request>) -> Claimed {
    match conn.request_name_with_flags(app_id, RequestNameFlags::DoNotQueue.into()).await {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Claimed::Owner,
        _ => match request {
            None => Claimed::Passed,
            Some(r) => {
                if call(conn, app_id, r).await {
                    Claimed::Passed
                } else {
                    Claimed::Unanswered
                }
            }
        },
    }
}

/// Asks the process that has the name for `request`.
async fn call(conn: &Connection, app_id: &str, request: Request) -> bool {
    let call = conn.call_method(Some(app_id), PATH, Some(IFACE), request.method(), &());
    matches!(tokio::time::timeout(Duration::from_secs(5), call).await, Ok(Ok(_)))
}

async fn serve(conn: &Connection, handle: impl Fn(Request)) {
    let mut calls = MessageStream::from(conn);
    while let Some(Ok(message)) = calls.next().await {
        let header = message.header();
        if header.message_type() != MessageType::MethodCall || header.path().map(|p| p.as_str()) != Some(PATH) {
            continue;
        }
        let request = header.member().and_then(|m| Request::from_method(m.as_str()));
        if let Some(request) = request.filter(|_| header.interface().map(|i| i.as_str()) == Some(IFACE)) {
            handle(request);
            let _ = conn.reply(&header, &()).await;
        } else {
            let _ = conn.reply_error(&header, "org.freedesktop.DBus.Error.UnknownMethod", &("Unknown method",)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};

    use super::*;

    const ID: &str = "ch.giosuel.LituusTest";

    /// A private session bus, stopped when dropped; none without dbus-daemon.
    struct Bus(Child, String);

    impl Bus {
        fn start() -> Option<Self> {
            let mut daemon = match Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .spawn()
            {
                Ok(daemon) => daemon,
                Err(e) => {
                    eprintln!("skipped: no dbus-daemon ({e})");
                    return None;
                }
            };
            let mut address = String::new();
            BufReader::new(daemon.stdout.take()?).read_line(&mut address).ok()?;
            Some(Self(daemon, address.trim().to_owned()))
        }

        async fn connect(&self) -> Connection {
            zbus::connection::Builder::address(self.1.as_str()).unwrap().build().await.unwrap()
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[tokio::test]
    async fn a_second_start_passes_its_request_on() {
        let Some(bus) = Bus::start() else { return };
        let first = bus.connect().await;
        assert!(matches!(claim_on(&first, ID, Some(Request::Show)).await, Claimed::Owner));
        let (requests, handled) = mpsc::channel();
        let serving = first.clone();
        tokio::spawn(async move {
            serve(&serving, move |r| {
                let _ = requests.send(r);
            })
            .await
        });
        // Its stream of calls exists before the first one comes.
        tokio::task::yield_now().await;
        let second = bus.connect().await;
        assert!(matches!(claim_on(&second, ID, Some(Request::Quick)).await, Claimed::Passed));
        assert_eq!(handled.try_recv(), Ok(Request::Quick));
        // `lituus --toggle-wifi`.
        assert!(call(&second, ID, Request::ToggleWifi).await);
        assert_eq!(handled.try_recv(), Ok(Request::ToggleWifi));
        // A second `lituus --background` just leaves.
        assert!(matches!(claim_on(&second, ID, None).await, Claimed::Passed));
        assert!(handled.try_recv().is_err());
        // Anything else is refused.
        assert!(second.call_method(Some(ID), PATH, Some(IFACE), "Quit", &()).await.is_err());
        assert!(handled.try_recv().is_err());
    }

    /// `lituus --toggle-wifi` with no Lituus running leaves the name alone: a start in that
    /// moment runs, rather than handing its request to a process that's about to leave.
    #[tokio::test]
    async fn passing_a_request_on_never_takes_the_name() {
        let Some(bus) = Bus::start() else { return };
        let toggle = bus.connect().await;
        assert!(!call(&toggle, ID, Request::ToggleWifi).await);
        let start = bus.connect().await;
        assert!(matches!(claim_on(&start, ID, Some(Request::Show)).await, Claimed::Owner));
    }
}
