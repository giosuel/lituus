//! The Printers page (0.7.0, D-59; S-186 to S-188, updated in S-217 and S-218): the printers CUPS
//! has, the selected one's queue, what it prints by default, its address and driver; a test page,
//! Remove, and Make default (the user's own, D-65). Everything goes to cupsd through lituus-print.
//! It's read on a thread when the page opens, again when cupsd says something changed
//! (com.redhat.PrinterSpooler), and every 2 seconds while a job is on its way. Users outside
//! CUPS's admin group change printers through cups-pk-helper and polkit's password prompt; without
//! it, a Notice says what to install and the changes are greyed out (D-68).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use lituus_print::avahi::{self, Event, Found, Link, Service};
use lituus_print::pkhelper::{self, Access, PkHelper};
use lituus_print::{
    Client, Defaults, Device, Driver, Error, Job, JobState, NewPrinter, Options, Printer, Snapshot,
    State as PrinterState,
};
use slint::{SharedString, VecModel};

use crate::{FoundRow, HeroInfo, JobRow, MainWindow, PrinterRow, PrintersView, input, model, update};

/// What a read found.
#[derive(Debug, Clone, Default)]
pub struct Read {
    pub snapshot: Snapshot,
    pub options: HashMap<String, Options>,
    /// Nothing answers on cupsd's socket.
    pub not_running: bool,
    /// cupsd is installed (so starting it is enough).
    pub installed: bool,
    /// The commands that install cups, cups-pk-helper and hyprpolkitagent here.
    pub install: String,
    pub pk_command: String,
    pub agent_command: String,
    /// Apps print through another server (CUPS_SERVER, ~/.cups/client.conf).
    pub elsewhere: Option<String>,
    /// Network printers that didn't answer (D-67): checked when the page opens, not stored.
    pub missing: HashSet<String>,
    pub access: Access,
    /// A polkit agent runs, for cups-pk-helper's password prompt.
    pub agent: bool,
    /// The local time's offset from UTC, in seconds, and now: for the jobs' times.
    pub offset: i64,
    pub now: i64,
    /// A read that failed otherwise.
    pub error: Option<String>,
}

/// The Add dialog (S-187): Avahi's printers as they come, USB printers CUPS sees, or an address.
#[derive(Default)]
pub struct Add {
    pub open: bool,
    /// Avahi answers on the bus; None until it's known.
    pub avahi: Option<bool>,
    pub services: Vec<Service>,
    /// USB printers that need a driver (admins only): the device, an installed driver for it,
    /// else the package that has one.
    pub usb: Vec<(Device, Option<String>, Option<&'static str>)>,
    /// Still looking: the first few seconds.
    pub searching: bool,
    /// The row being added ("address" for the address).
    pub adding: Option<String>,
    pub error: Option<String>,
    /// Tells the dialog's thread to stop looking.
    stop: Option<Arc<AtomicBool>>,
}

#[derive(Default)]
pub struct State {
    pub loaded: bool,
    pub add: Add,
    pub read: Read,
    /// The printer the details are of (its queue name).
    pub selected: Option<String>,
    pub working: bool,
    pub error: Option<String>,
    /// The page is showing: reads follow cupsd's signals and, while a job is on its way, a tick.
    pub open: bool,
    /// The page was opened: read again, and look for the network printers.
    pub reread: bool,
    /// A read is on its way; another was asked for meanwhile; when the last one started.
    pub reading: bool,
    pub again: bool,
    pub last: Option<Instant>,
}

// ---- reading -------------------------------------------------------------------------------------

fn block_on<T>(future: impl std::future::Future<Output = T>) -> Option<T> {
    Some(tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?.block_on(future))
}

/// The first read, when Lituus starts (for the sidebar's count).
pub fn spawn(weak: slint::Weak<MainWindow>) {
    let w = weak.clone();
    let _ = weak.upgrade_in_event_loop(move |_| update(&w.clone(), |app| refresh(&w, &mut app.printers, false)));
}

/// Reads again on a thread; `probe` also checks the network printers answer (D-67).
pub fn refresh(weak: &slint::Weak<MainWindow>, state: &mut State, probe: bool) {
    if state.reading {
        state.again = true;
        return;
    }
    state.reading = true;
    state.last = Some(Instant::now());
    let weak = weak.clone();
    std::thread::spawn(move || {
        let read = read_all(probe);
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| {
            update(&w.clone(), |app| {
                let state = &mut app.printers;
                let old = std::mem::replace(&mut state.read, read);
                if !probe {
                    // Kept from the last time the page opened: who may change printers, and the
                    // network printers that didn't answer (those still there).
                    let r = &mut state.read;
                    (r.access, r.agent) = (old.access, old.agent);
                    r.missing =
                        old.missing.into_iter().filter(|n| r.snapshot.printers.iter().any(|p| &p.name == n)).collect();
                }
                state.loaded = true;
                state.reading = false;
                if std::mem::take(&mut state.again) {
                    refresh(&w, state, false);
                }
            })
        });
    });
}

fn read_all(probe: bool) -> Read {
    let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut r = Read {
        installed: lituus_print::installed(),
        install: crate::power::install_line(&os, "cups"),
        pk_command: crate::power::install_line(&os, "cups-pk-helper"),
        agent_command: crate::power::install_line(&os, "hyprpolkitagent"),
        elsewhere: lituus_print::elsewhere(),
        agent: probe && crate::datetime::agent_running(),
        offset: local_offset(),
        now: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        ..Read::default()
    };
    let _ = block_on(async {
        let client = Client::local();
        match client.snapshot().await {
            Ok(snap) => {
                for p in &snap.printers {
                    if let Ok(o) = client.options(&p.name).await {
                        r.options.insert(p.name.clone(), o);
                    }
                }
                if probe {
                    r.missing = missing(&snap.printers).await;
                }
                r.snapshot = snap;
            }
            Err(Error::NotRunning) => r.not_running = true,
            Err(e) => r.error = Some(format!("Couldn't read the printers: {e}.")),
        }
        if probe {
            let system = zbus::Connection::system().await.ok();
            r.access = match pkhelper::access(system.as_ref()).await {
                Access::Admin => Access::Admin,
                // Not by the usual groups: cupsd may still say yes (another SystemGroup, root).
                _ if !r.not_running && Client::local().can_admin().await => Access::Admin,
                other => other,
            };
        }
    });
    r
}

/// The local time's offset from UTC in seconds ("+0100" from `date`).
fn local_offset() -> i64 {
    let out = std::process::Command::new("date").arg("+%z").output().ok();
    out.and_then(|o| parse_offset(String::from_utf8_lossy(&o.stdout).trim())).unwrap_or(0)
}

fn parse_offset(z: &str) -> Option<i64> {
    let sign = match z.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let digits = z.get(1..5)?;
    Some(sign * (digits[..2].parse::<i64>().ok()? * 3600 + digits[2..].parse::<i64>().ok()? * 60))
}

/// A network printer's host and port, when its address names one; a DNS-SD service name
/// ("Brother._ipp._tcp.local") is Avahi's to find.
fn host_port(uri: &str) -> Option<(String, u16)> {
    let (scheme, rest) = uri.split_once("://")?;
    let port = match scheme {
        "ipp" | "ipps" | "http" | "https" => 631,
        "socket" => 9100,
        "lpd" => 515,
        _ => return None,
    };
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit('@').next()?;
    if authority.contains("._tcp") || authority.contains("%2E_tcp") || authority.is_empty() {
        return None;
    }
    if let Some(v6) = authority.strip_prefix('[') {
        let (host, rest) = v6.split_once(']')?;
        let port = rest.strip_prefix(':').and_then(|p| p.parse().ok()).unwrap_or(port);
        return Some((host.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, p)) => Some((host.to_string(), p.parse().ok()?)),
        None => Some((authority.to_string(), port)),
    }
}

/// The network printers that don't answer within 2 seconds.
async fn missing(printers: &[Printer]) -> HashSet<String> {
    let checks =
        printers.iter().filter(|p| p.networked()).filter_map(|p| Some((p.name.clone(), host_port(&p.device_uri)?)));
    let probes = checks.map(|(name, (host, port))| async move {
        let reached =
            tokio::time::timeout(Duration::from_secs(2), tokio::net::TcpStream::connect((host.as_str(), port))).await;
        (!matches!(reached, Ok(Ok(_)))).then_some(name)
    });
    futures_util::future::join_all(probes).await.into_iter().flatten().collect()
}

/// Follows cupsd's signals (read again once they're quiet for 150 ms), and ticks every 2 seconds
/// for a job on its way or cupsd starting.
pub fn spawn_watch(weak: slint::Weak<MainWindow>) {
    let w = weak.clone();
    std::thread::spawn(move || {
        let _ = block_on(async move {
            loop {
                let changes = match zbus::Connection::system().await {
                    Ok(conn) => lituus_print::changes(&conn).await.ok().map(|s| (conn, s)),
                    Err(_) => None,
                };
                let Some((_conn, mut changes)) = changes else {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    continue;
                };
                while changes.next().await.is_some() {
                    while let Ok(Some(())) = tokio::time::timeout(Duration::from_millis(150), changes.next()).await {}
                    let w2 = w.clone();
                    let posted = w.upgrade_in_event_loop(move |_| {
                        update(&w2.clone(), |app| {
                            if app.printers.open {
                                refresh(&w2, &mut app.printers, false);
                            } else {
                                app.printers.reread = true;
                            }
                        })
                    });
                    if posted.is_err() {
                        return;
                    }
                }
                // The bus went away: try again in a while, not at once.
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });
    });
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let w = weak.clone();
            if weak.upgrade_in_event_loop(move |_| update(&w.clone(), |app| tick(&w, &mut app.printers))).is_err() {
                return;
            }
        }
    });
}

fn tick(weak: &slint::Weak<MainWindow>, state: &mut State) {
    if !state.open || state.reading {
        return;
    }
    let busy = state.read.snapshot.jobs.iter().any(|j| matches!(j.state, JobState::Pending | JobState::Processing));
    let waited = |s| state.last.is_none_or(|t| t.elapsed() >= Duration::from_secs(s));
    // In a Flatpak, cupsd's signals don't come through (it has no name on the bus to allow).
    let flatpak = std::env::var_os("FLATPAK_ID").is_some();
    // cupsd's signals are the trigger; a read every 10 s while the page shows covers one that
    // never came (cupsd built without D-Bus, another bus).
    if busy || (state.read.not_running && waited(5)) || (flatpak && waited(3)) || waited(10) {
        refresh(weak, state, false);
    }
}

// ---- changes -------------------------------------------------------------------------------------

enum Change {
    Cancel(i32),
    Options(String, Defaults),
    TestPage(String),
    Remove(String),
}

fn problem(e: &Error, through_helper: bool) -> String {
    match e {
        Error::NotAllowed(_) if through_helper => "The password wasn't given, so nothing changed.".into(),
        Error::NotAllowed(_) => {
            "Only CUPS's administrators can change printers. With cups-pk-helper you can, with your password.".into()
        }
        Error::NotRunning => "CUPS isn't running.".into(),
        Error::Timeout => "CUPS didn't answer in time.".into(),
        Error::Ipp { message, .. } if !message.is_empty() => format!("CUPS said: {}.", message.trim_end_matches('.')),
        other => format!("That didn't work: {other}."),
    }
}

/// A change on a thread: through the socket, else through cups-pk-helper; then the page is read
/// again, or says why not.
fn change(weak: &slint::Weak<MainWindow>, state: &mut State, change: Change) {
    state.working = true;
    state.error = None;
    let weak = weak.clone();
    std::thread::spawn(move || {
        let result = block_on(async {
            let client = Client::local();
            let direct = match &change {
                Change::Cancel(id) => client.cancel(*id).await,
                Change::Options(name, d) => client.set_options(name, d).await,
                Change::TestPage(name) => client.print_test_page(name).await.map(drop),
                Change::Remove(name) => client.remove(name).await,
            };
            let Err(Error::NotAllowed(_)) = direct else { return direct.map_err(|e| problem(&e, false)) };
            if matches!(change, Change::TestPage(_)) {
                return direct.map_err(|e| problem(&e, false));
            }
            // Not one of CUPS's admins: cups-pk-helper, if it's there; polkit asks for the password.
            let Ok(conn) = zbus::Connection::system().await else { return direct.map_err(|e| problem(&e, false)) };
            if !pkhelper::available(&conn).await {
                return direct.map_err(|e| problem(&e, false));
            }
            let pk = PkHelper::new(conn);
            let helped = match &change {
                Change::Cancel(id) => pk.cancel(*id).await,
                Change::Options(name, d) => {
                    let (_, set) = client.ppd_choices(name, d).await.unwrap_or_default();
                    pk.set_options(name, d, &set).await
                }
                Change::Remove(name) => pk.remove(name).await,
                Change::TestPage(_) => Ok(()),
            };
            helped.map_err(|e| problem(&e, true))
        })
        .unwrap_or_else(|| Err("Couldn't start the change.".into()));
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| {
            update(&w.clone(), |app| {
                let state = &mut app.printers;
                state.working = false;
                if let Err(e) = result {
                    state.error = Some(e);
                }
                refresh(&w, state, false);
            })
        });
    });
}

/// After the confirmation: removes the printer.
pub fn remove(weak: &slint::Weak<MainWindow>, state: &mut State, name: String) {
    if state.selected.as_deref() == Some(name.as_str()) {
        state.selected = None;
    }
    change(weak, state, Change::Remove(name));
}

/// The confirmation for Remove: (title, text).
pub fn remove_question(state: &State) -> Option<(String, String, String)> {
    let p = selected(state)?;
    Some((
        format!("Remove {}?", p.title()),
        "Its queue and its settings go from this computer, and apps can't print to it until it's added again.".into(),
        p.name.clone(),
    ))
}

fn selected(state: &State) -> Option<&Printer> {
    let printers = &state.read.snapshot.printers;
    let by_name = |n: &str| printers.iter().find(|p| p.name == n);
    state
        .selected
        .as_deref()
        .and_then(by_name)
        .or_else(|| state.read.snapshot.default.as_deref().and_then(by_name))
        .or_else(|| printers.first())
}

/// The choices the page offers for an option, as (keyword, label); paper without the custom ranges.
fn papers(o: &Options) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    o.media
        .supported
        .iter()
        .filter(|m| !lituus_print::media::is_custom_range(m))
        .map(|m| (m.clone(), lituus_print::media::name(m)))
        .filter(|(_, label)| seen.insert(label.clone()))
        .collect()
}

fn side_label(s: &str) -> String {
    match s {
        "one-sided" => "Off".into(),
        "two-sided-long-edge" => "Long edge (book)".into(),
        "two-sided-short-edge" => "Short edge (notepad)".into(),
        other => other.into(),
    }
}

fn quality_label(q: &str) -> String {
    match q {
        "draft" => "Draft".into(),
        "normal" => "Normal".into(),
        "high" => "High".into(),
        other => other.into(),
    }
}

fn is_mono(c: &str) -> bool {
    c.contains("monochrome") || c == "bi-level"
}

pub fn act(weak: &slint::Weak<MainWindow>, state: &mut State, verb: &str, value: i32, text: &str) {
    let name = selected(state).map(|p| p.name.clone()).unwrap_or_default();
    let options = state.read.options.get(&name).cloned().unwrap_or_default();
    let pick = |list: &[String]| list.get(value.max(0) as usize).cloned();
    let defaults = match verb {
        "paper" => papers(&options)
            .get(value.max(0) as usize)
            .map(|(k, _)| Defaults { media: Some(k.clone()), ..Defaults::default() }),
        "sides" => pick(&options.sides.supported).map(|s| Defaults { sides: Some(s), ..Defaults::default() }),
        "quality" => pick(&options.quality.supported).map(|q| Defaults { quality: Some(q), ..Defaults::default() }),
        "color" => {
            let want = if value == 1 {
                options.color.supported.iter().find(|c| is_mono(c)).cloned().or(Some("monochrome".into()))
            } else {
                options.color.supported.iter().find(|c| *c == "color").cloned().or(Some("color".into()))
            };
            want.map(|c| Defaults { color: Some(c), ..Defaults::default() })
        }
        _ => None,
    };
    if let Some(d) = defaults {
        change(weak, state, Change::Options(name, d));
        return;
    }
    match verb {
        "select" => state.selected = Some(text.to_owned()),
        "cancel" => change(weak, state, Change::Cancel(value)),
        "test-page" => change(weak, state, Change::TestPage(name)),
        "make-default" => {
            match Client::local().set_default(&name) {
                Ok(()) => state.read.snapshot.default = Some(name),
                Err(e) => state.error = Some(format!("Couldn't make it your default: {e}.")),
            }
            refresh(weak, state, false);
        }
        "dismiss" | "copied" => state.error = None,
        "add-open" => open_add(weak, state),
        "add-close" => close_add(state),
        "add-dismiss" => state.add.error = None,
        "add-found" => add_found(weak, state, text),
        "add-address" => add_address(weak, state, text),
        _ => {}
    }
}

// ---- adding --------------------------------------------------------------------------------------

fn close_add(state: &mut State) {
    if let Some(stop) = state.add.stop.take() {
        stop.store(true, Ordering::Relaxed);
    }
    state.add = Add::default();
}

/// Opens the dialog and looks for printers until it closes.
fn open_add(weak: &slint::Weak<MainWindow>, state: &mut State) {
    close_add(state);
    let stop = Arc::new(AtomicBool::new(false));
    state.add = Add { open: true, searching: true, stop: Some(stop.clone()), ..Add::default() };
    let admin = state.read.access == Access::Admin;
    let weak = weak.clone();
    std::thread::spawn(move || {
        let post = |weak: &slint::Weak<MainWindow>, change: Box<dyn FnOnce(&mut Add) + Send>| {
            let w = weak.clone();
            weak.upgrade_in_event_loop(move |_| {
                update(&w, |app| {
                    if app.printers.add.open {
                        change(&mut app.printers.add);
                    }
                })
            })
            .is_ok()
        };
        let _ = block_on(async {
            // USB printers that need a driver: CUPS lists them for admins only (anyone else would
            // be asked for the password just to open the dialog).
            if admin {
                let client = Client::local();
                let mut usb = Vec::new();
                for d in client.usb_devices().await.unwrap_or_default() {
                    let ppd = client.ppd_for(&d.id).await.ok().flatten();
                    let package = lituus_print::drivers::package(&lituus_print::drivers::device_id(&d.id));
                    usb.push((d, ppd, package));
                }
                if !post(&weak, Box::new(move |a: &mut Add| a.usb = usb)) {
                    return;
                }
            }
            let Ok(conn) = zbus::Connection::system().await else {
                post(&weak, Box::new(|a: &mut Add| (a.avahi, a.searching) = (Some(false), false)));
                return;
            };
            let running = avahi::running(&conn).await;
            post(&weak, Box::new(move |a: &mut Add| a.avahi = Some(running)));
            let events = if running { avahi::browse(&conn).await.ok() } else { None };
            let Some(mut events) = events else {
                post(&weak, Box::new(|a: &mut Add| a.searching = false));
                return;
            };
            let started = Instant::now();
            let mut all_for_now = 0;
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let event = tokio::time::timeout(Duration::from_millis(400), events.next()).await;
                let change: Box<dyn FnOnce(&mut Add) + Send> = match event {
                    Ok(Some(Event::Found(s))) => Box::new(move |a: &mut Add| {
                        a.services.retain(|o| !(o.name == s.name && o.kind == s.kind));
                        a.services.push(s);
                    }),
                    Ok(Some(Event::Gone { name, kind })) => {
                        Box::new(move |a: &mut Add| a.services.retain(|o| !(o.name == name && o.kind == kind)))
                    }
                    Ok(Some(Event::AllForNow)) => {
                        all_for_now += 1;
                        if all_for_now < avahi::TYPES.len() {
                            continue;
                        }
                        Box::new(|a: &mut Add| a.searching = false)
                    }
                    Ok(None) => return,
                    // Printers answer within a few seconds; after that it's "none found".
                    Err(_) if started.elapsed() > Duration::from_secs(4) => Box::new(|a: &mut Add| a.searching = false),
                    Err(_) => continue,
                };
                if !post(&weak, change) {
                    return;
                }
            }
        });
    });
}

/// The dialog's rows: Avahi's printers and USB ones, each once.
fn found(state: &State) -> Vec<Found> {
    let queues = &state.read.snapshot.printers;
    let mut rows = avahi::printers(&state.add.services, queues, avahi::loopback_index());
    for (d, ppd, package) in &state.add.usb {
        let id = lituus_print::drivers::device_id(&d.id);
        let title =
            lituus_print::drivers::title(&id, if d.make_and_model.is_empty() { &d.info } else { &d.make_and_model });
        // ipp-usb already shows it as a USB printer that needs no driver.
        if rows.iter().any(|r| r.link == Link::Usb && r.name.to_lowercase().contains(&title.to_lowercase())) {
            continue;
        }
        rows.push(Found {
            id: d.uri.clone(),
            name: title.clone(),
            link: Link::Usb,
            uri: d.uri.clone(),
            driverless: false,
            added: queues.iter().any(|q| q.device_uri == d.uri),
            ppd: ppd.clone(),
            needs: if ppd.is_none() { package.map(String::from) } else { None },
            make_and_model: title,
        });
    }
    rows
}

fn taken(state: &State) -> Vec<String> {
    state.read.snapshot.printers.iter().map(|p| p.name.clone()).collect()
}

fn local_paper() -> String {
    let lc = ["LC_ALL", "LC_PAPER", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
    lituus_print::media::local_paper(lc.as_deref()).to_string()
}

/// Adds on a thread: through the socket, else cups-pk-helper; then selects the new printer.
fn add(weak: &slint::Weak<MainWindow>, state: &mut State, row: String, new: NewPrinter, probe: Option<(String, u16)>) {
    state.add.adding = Some(row);
    state.add.error = None;
    let weak = weak.clone();
    std::thread::spawn(move || {
        let result = block_on(async {
            if let Some((host, port)) = &probe
                && !lituus_print::answers(host, *port).await
            {
                return Err(format!("Nothing answers at {host}. Is the printer on, and on this network?"));
            }
            let client = Client::local();
            let paper = local_paper();
            match client.add(&new, &paper).await {
                Err(Error::NotAllowed(_)) => {
                    let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
                    if !pkhelper::available(&conn).await {
                        return Err(problem(&Error::NotAllowed(String::new()), false));
                    }
                    PkHelper::new(conn)
                        .add(&new.name, &new.uri, &new.ppd, &new.info, &new.location)
                        .await
                        .map_err(|e| problem(&e, true))
                }
                Err(Error::Timeout) => Err("The printer didn't answer CUPS in time, so it wasn't added.".into()),
                Err(Error::Ipp { message, .. }) => {
                    Err(format!("CUPS couldn't set it up: {}.", message.trim_end_matches('.')))
                }
                Err(e) => Err(problem(&e, false)),
                Ok(()) => {
                    // Added by its address: named after the model the printer told cupsd.
                    if probe.is_some()
                        && let Ok(printers) = client.printers().await
                        && let Some(p) = printers.iter().find(|p| p.name == new.name)
                        && !p.model_name().is_empty()
                    {
                        let _ = client.set_info(&new.name, &p.model_name()).await;
                    }
                    Ok(())
                }
            }
        })
        .unwrap_or_else(|| Err("Couldn't start adding it.".into()));
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| {
            update(&w.clone(), |app| {
                let state = &mut app.printers;
                state.add.adding = None;
                match result {
                    // The dialog closes on the new printer, selected.
                    Ok(()) => {
                        state.selected = Some(new.name.clone());
                        close_add(state);
                    }
                    Err(e) => state.add.error = Some(e),
                }
                refresh(&w, state, false);
            })
        });
    });
}

fn add_found(weak: &slint::Weak<MainWindow>, state: &mut State, id: &str) {
    let Some(f) = found(state).into_iter().find(|f| f.id == id) else { return };
    let ppd = match (&f.ppd, f.driverless) {
        (_, true) => "everywhere".to_string(),
        (Some(p), false) => p.clone(),
        (None, false) => return,
    };
    let new = NewPrinter {
        name: lituus_print::queue_name(&f.name, &taken(state)),
        uri: f.uri.clone(),
        ppd,
        info: f.name.clone(),
        location: String::new(),
    };
    add(weak, state, f.id, new, None);
}

fn add_address(weak: &slint::Weak<MainWindow>, state: &mut State, typed: &str) {
    let Some((uri, host, port)) = lituus_print::address(typed) else {
        state.add.error = Some(
            "Type the printer's address, such as 192.168.1.40, printer.local or ipp://printer.local/ipp/print.".into(),
        );
        return;
    };
    let new = NewPrinter {
        name: lituus_print::queue_name(&host, &taken(state)),
        uri,
        ppd: "everywhere".into(),
        info: host.clone(),
        location: String::new(),
    };
    add(weak, state, "address".into(), new, Some((host, port)));
}

// ---- the page ------------------------------------------------------------------------------------

fn s(text: impl AsRef<str>) -> SharedString {
    SharedString::from(text.as_ref())
}

/// Printers besides Print to PDF.
fn real(state: &State) -> usize {
    state.read.snapshot.printers.iter().filter(|p| p.driver != Driver::Pdf).count()
}

/// The sidebar's status: how many printers, "off" without CUPS.
pub fn status(state: &State) -> String {
    if !state.loaded {
        String::new()
    } else if state.read.not_running {
        "off".into()
    } else {
        match real(state) {
            0 => String::new(),
            n => n.to_string(),
        }
    }
}

/// "2 printers and Print to PDF".
fn sub(state: &State) -> String {
    let n = real(state);
    let pdf = state.read.snapshot.printers.iter().any(|p| p.driver == Driver::Pdf);
    let printers = match n {
        0 => "No printers".to_string(),
        1 => "1 printer".to_string(),
        n => format!("{n} printers"),
    };
    if pdf { format!("{printers} and Print to PDF") } else { printers }
}

/// The printer's state in words, and its tone.
fn state_words(state: &State, p: &Printer) -> (String, &'static str) {
    if state.read.missing.contains(&p.name) {
        return ("Not found on the network".into(), "warn");
    }
    let has = |r: &str| p.reasons.iter().any(|x| x.starts_with(r));
    if p.state == PrinterState::Stopped {
        return ("Paused".into(), "warn");
    }
    if has("offline") {
        return ("Offline".into(), "warn");
    }
    if has("media-empty") || has("media-needed") {
        return ("Out of paper".into(), "warn");
    }
    if has("media-jam") {
        return ("Paper jam".into(), "warn");
    }
    if has("toner-empty") || has("marker-supply-empty") {
        return ("Out of toner or ink".into(), "warn");
    }
    if has("door-open") || has("cover-open") {
        return ("A cover is open".into(), "warn");
    }
    if p.state == PrinterState::Printing {
        return ("Printing".into(), "accent");
    }
    if !p.accepting {
        return ("Not taking jobs".into(), "warn");
    }
    ("Ready".into(), "accent")
}

fn is_default(state: &State, p: &Printer) -> bool {
    state.read.snapshot.default.as_deref() == Some(p.name.as_str())
}

fn row_meta(state: &State, p: &Printer) -> (String, &'static str) {
    let (words, tone) = state_words(state, p);
    let extra = if is_default(state, p) {
        " · your default"
    } else if p.driver == Driver::Pdf {
        " · cups-pdf"
    } else {
        ""
    };
    (format!("{words}{extra}"), tone)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "14:32", or "25 Nov 14:32" when it isn't today.
fn when(at: i64, now: i64, offset: i64) -> String {
    let (local, today) = (at + offset, now + offset);
    let clock = format!("{:02}:{:02}", local.rem_euclid(86_400) / 3600, local.rem_euclid(3600) / 60);
    if local.div_euclid(86_400) == today.div_euclid(86_400) {
        return clock;
    }
    // Days since 1970 to a date (Howard Hinnant's civil_from_days).
    let z = local.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{day} {} {clock}", MONTHS[(month - 1) as usize])
}

/// "Printing · page 1 of 2 · 14:31", "Waiting · 12 pages · 14:32 · by anna" (D-66).
fn job_meta(state: &State, j: &Job) -> String {
    let words = match j.state {
        JobState::Processing => "Printing",
        JobState::Held => "Held",
        JobState::Stopped => "Stopped",
        JobState::Pending => "Waiting",
        JobState::Canceled => "Cancelled",
        JobState::Aborted => "Failed",
        JobState::Completed => "Done",
    };
    let mut parts = vec![words.to_string()];
    match (j.state, j.pages) {
        (JobState::Processing, Some(n)) => parts.push(format!("page {} of {n}", j.pages_done.max(1).min(n))),
        (JobState::Processing, None) => parts.push(format!("page {}", j.pages_done.max(1))),
        (_, Some(1)) => parts.push("1 page".into()),
        (_, Some(n)) => parts.push(format!("{n} pages")),
        (_, None) => {}
    }
    if j.created > 0 {
        parts.push(when(j.created, state.read.now, state.read.offset));
    }
    if !j.mine && !j.user.is_empty() {
        parts.push(format!("by {}", j.user));
    }
    parts.join(" · ")
}

fn driver_text(p: &Printer) -> String {
    match &p.driver {
        Driver::Driverless => "Driverless (IPP Everywhere)".into(),
        Driver::Ppd(name) => name.clone(),
        Driver::Raw => "None: sent as it is (raw)".into(),
        Driver::Pdf => "cups-pdf: writes PDF files".into(),
    }
}

/// A device URI for people: DNS-SD names decoded ("ipp://Brother HL._ipp._tcp.local/").
fn address(uri: &str) -> String {
    let mut out = Vec::new();
    let b = uri.as_bytes();
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

fn add_view(state: &State, v: &mut PrintersView) {
    let a = &state.add;
    v.add_open = a.open;
    if !a.open {
        return;
    }
    let rows = found(state);
    let mut primary = true;
    v.add_rows = model(
        rows.iter()
            .map(|f| {
                let link = if f.link == Link::Usb { "USB" } else { "Network" };
                let (meta, action, note) = if f.added {
                    (format!("{link} · already added"), "added", String::new())
                } else if f.driverless {
                    (format!("{link} · driverless, nothing to install"), "add", String::new())
                } else if f.ppd.is_some() {
                    (format!("{link} · its driver is installed"), "add", String::new())
                } else if let Some(p) = &f.needs {
                    (format!("{link} · needs a driver: {p}"), "install", format!("Install {p} first"))
                } else {
                    (format!("{link} · needs a driver"), "install", "Needs a driver".to_string())
                };
                let row = FoundRow {
                    id: s(&f.id),
                    name: s(&f.name),
                    meta: s(meta),
                    action: s(action),
                    note: s(note),
                    primary: primary && action == "add",
                    busy: a.adding.as_deref() == Some(f.id.as_str()),
                };
                if action == "add" {
                    primary = false;
                }
                row
            })
            .collect(),
    );
    v.add_searching = a.searching;
    v.add_avahi = s(match a.avahi {
        Some(false) => "off",
        Some(true) => "on",
        None => "",
    });
    v.avahi_command = s("sudo systemctl enable --now avahi-daemon");
    v.add_error = s(a.error.clone().unwrap_or_default());
    v.add_busy = a.adding.is_some();
    v.add_address_busy = a.adding.as_deref() == Some("address");
    let needs_driver = rows.iter().any(|f| !f.added && !f.driverless && f.ppd.is_none());
    v.driver_link = s(if needs_driver { lituus_print::drivers::OPENPRINTING } else { "" });
}

pub fn view(state: &State) -> PrintersView {
    let mut v = PrintersView {
        error: s(state.error.clone().or(state.read.error.clone()).unwrap_or_default()),
        ..PrintersView::default()
    };
    let read = &state.read;
    if !state.loaded {
        v.state = s("loading");
        return v;
    }
    add_view(state, &mut v);
    v.elsewhere = s(read
        .elsewhere
        .as_deref()
        .map(|e| format!("Apps print through {e}, which Lituus doesn't manage."))
        .unwrap_or_default());
    if read.not_running {
        v.state = s("no-cups");
        v.sub = s("Not running");
        v.installed = read.installed;
        v.command = s(if read.installed {
            "sudo systemctl enable --now cups.socket".to_string()
        } else {
            read.install.clone()
        });
        v.hero = HeroInfo {
            icon: s("printer"),
            name: s("Printing isn't set up"),
            meta: s(if read.installed { "CUPS isn't running" } else { "CUPS isn't installed" }),
            tone: s("warn"),
            detail: SharedString::default(),
        };
        return v;
    }
    v.sub = s(sub(state));
    v.working = state.working;
    v.can_change = read.access != Access::None;
    v.access = s(match read.access {
        Access::Admin => "admin",
        Access::PkHelper if read.agent => "password",
        Access::PkHelper => "no-agent",
        Access::None => "none",
    });
    v.pk_command = s(&read.pk_command);
    v.agent_command = s(&read.agent_command);
    let Some(p) = selected(state) else {
        v.state = s("empty");
        v.hero = HeroInfo {
            icon: s("printer"),
            name: s("No printers yet"),
            meta: s("Add one from your network or USB"),
            tone: s("muted"),
            detail: s(&read.snapshot.server),
        };
        return v;
    };
    v.state = s("list");
    v.printers = model(
        read.snapshot
            .printers
            .iter()
            .map(|q| {
                let (meta, tone) = row_meta(state, q);
                PrinterRow {
                    id: s(&q.name),
                    name: s(q.title()),
                    meta: s(meta),
                    tone: s(tone),
                    selected: q.name == p.name,
                }
            })
            .collect(),
    );
    let default = is_default(state, p);
    let (words, tone) = state_words(state, p);
    let queued: Vec<&Job> = read.snapshot.jobs.iter().filter(|j| j.printer == p.name && !j.state.done()).collect();
    let mut meta = words.clone();
    if default {
        meta.push_str(" · your default printer");
    }
    if !queued.is_empty() {
        meta.push_str(&format!(" · {} in the queue", queued.len()));
    }
    let kind = match p.driver {
        Driver::Driverless => "driverless",
        Driver::Ppd(_) => "with a driver",
        Driver::Raw => "raw",
        Driver::Pdf => "cups-pdf",
    };
    v.hero = HeroInfo {
        icon: s("printer"),
        name: s(p.title()),
        meta: s(meta),
        tone: s(if tone == "warn" { "warn" } else { "accent" }),
        detail: s([read.snapshot.server.as_str(), kind]
            .iter()
            .filter(|t| !t.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" · ")),
    };
    v.name = s(p.title());
    v.id = s(&p.name);
    v.is_default = default;
    v.jobs = model(
        queued
            .iter()
            .map(|j| JobRow {
                id: j.id,
                name: s(if j.name.is_empty() { format!("Job {}", j.id) } else { j.name.clone() }),
                meta: s(job_meta(state, j)),
                can_cancel: j.mine || read.access != Access::None,
            })
            .collect(),
    );
    let o = read.options.get(&p.name).cloned().unwrap_or_default();
    let paper = papers(&o);
    v.papers = model(paper.iter().map(|(_, l)| s(l)).collect());
    v.paper = paper
        .iter()
        .position(|(k, _)| *k == o.media.default)
        .or_else(|| paper.iter().position(|(_, l)| *l == lituus_print::media::name(&o.media.default)))
        .map_or(-1, |i| i as i32);
    v.sides = model(o.sides.supported.iter().map(|x| s(side_label(x))).collect());
    v.side = o.sides.supported.iter().position(|x| *x == o.sides.default).map_or(-1, |i| i as i32);
    v.qualities = model(o.quality.supported.iter().map(|x| s(quality_label(x))).collect());
    v.quality = o.quality.supported.iter().position(|x| *x == o.quality.default).map_or(-1, |i| i as i32);
    v.color_enabled = o.color_supported && o.color.supported.iter().any(|c| is_mono(c));
    v.color = if !o.color_supported || is_mono(&o.color.default) { 1 } else { 0 };
    v.color_note = s(if o.color_supported { "" } else { "This printer prints in black only" });
    v.has_options = !o.media.supported.is_empty() || !o.sides.supported.is_empty();
    v.address = s(address(&p.device_uri));
    v.driver = s(driver_text(p));
    v.can_test = p.driver != Driver::Raw;
    v
}

thread_local! {
    // The rows' models, kept across renders: a row replaced while it's clicked loses the click.
    static PRINTERS: Rc<VecModel<PrinterRow>> = Rc::new(VecModel::default());
    static JOBS: Rc<VecModel<JobRow>> = Rc::new(VecModel::default());
    static FOUND: Rc<VecModel<FoundRow>> = Rc::new(VecModel::default());
}

pub fn render(window: &MainWindow, state: &State) {
    let mut v = view(state);
    v.printers = PRINTERS.with(|m| input::kept(m, &v.printers));
    v.jobs = JOBS.with(|m| input::kept(m, &v.jobs));
    v.add_rows = FOUND.with(|m| input::kept(m, &v.add_rows));
    window.set_printers(v);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use slint::Model;

    fn printer(name: &str, info: &str, uri: &str, driver: Driver) -> Printer {
        Printer {
            name: name.into(),
            info: info.into(),
            device_uri: uri.into(),
            driver,
            accepting: true,
            ..Printer::default()
        }
    }

    pub(crate) fn sample() -> State {
        let mut state = State { loaded: true, ..State::default() };
        state.read.snapshot = Snapshot {
            server: "CUPS 2.4".into(),
            printers: vec![
                printer("Brother", "Brother HL-L2350DW", "ipp://brother.local/ipp/print", Driver::Driverless),
                printer("HP", "HP OfficeJet Pro 8020", "ipp://hp-officejet.local/ipp/print", Driver::Driverless),
                printer("PDF", "Print to PDF", "cups-pdf:/", Driver::Pdf),
            ],
            default: Some("Brother".into()),
            jobs: vec![
                Job {
                    id: 4,
                    printer: "Brother".into(),
                    name: "invoice-2026-11.pdf".into(),
                    user: "me".into(),
                    mine: true,
                    state: JobState::Processing,
                    pages_done: 1,
                    pages: Some(2),
                    created: 1_795_659_060,
                },
                Job {
                    id: 5,
                    printer: "Brother".into(),
                    name: "report.odt".into(),
                    user: "anna".into(),
                    mine: false,
                    state: JobState::Pending,
                    pages: Some(12),
                    created: 1_795_659_120,
                    ..Job::default()
                },
            ],
        };
        let mut o = Options::default();
        o.media.supported = vec!["iso_a4_210x297mm".into(), "na_letter_8.5x11in".into(), "custom_min_3x5in".into()];
        o.media.default = "iso_a4_210x297mm".into();
        o.sides.supported = vec!["one-sided".into(), "two-sided-long-edge".into(), "two-sided-short-edge".into()];
        o.sides.default = "two-sided-long-edge".into();
        o.color.supported = vec!["monochrome".into()];
        o.color.default = "monochrome".into();
        o.quality.supported = vec!["draft".into(), "normal".into(), "high".into()];
        o.quality.default = "normal".into();
        state.read.options.insert("Brother".into(), o.clone());
        o.color.supported = vec!["monochrome".into(), "color".into()];
        o.color.default = "color".into();
        o.color_supported = true;
        state.read.options.insert("HP".into(), o);
        state.read.now = 1_795_660_000;
        state.read.missing.insert("HP".into());
        state
    }

    #[test]
    fn the_page_shows_the_default_printer_its_queue_and_options() {
        let v = view(&sample());
        assert_eq!(v.state, "list");
        assert_eq!(v.sub, "2 printers and Print to PDF");
        assert_eq!(
            (v.hero.name.as_str(), v.hero.meta.as_str()),
            ("Brother HL-L2350DW", "Ready · your default printer · 2 in the queue")
        );
        assert_eq!(v.hero.detail, "CUPS 2.4 · driverless");
        let rows: Vec<_> = v.printers.iter().map(|r| (r.name.to_string(), r.meta.to_string())).collect();
        assert_eq!(rows[0], ("Brother HL-L2350DW".into(), "Ready · your default".into()));
        assert_eq!(rows[1], ("HP OfficeJet Pro 8020".into(), "Not found on the network".into()));
        assert_eq!(rows[2], ("Print to PDF".into(), "Ready · cups-pdf".into()));
        let jobs: Vec<_> = v.jobs.iter().map(|j| (j.name.to_string(), j.meta.to_string())).collect();
        assert_eq!(jobs[0].1, format!("Printing · page 1 of 2 · {}", when(1_795_659_060, 1_795_660_000, 0)));
        assert!(jobs[1].1.starts_with("Waiting · 12 pages · ") && jobs[1].1.ends_with(" · by anna"), "{jobs:?}");
        assert_eq!(v.papers.iter().map(|p| p.to_string()).collect::<Vec<_>>(), ["A4", "Letter"]);
        assert_eq!((v.paper, v.side, v.quality), (0, 1, 1));
        assert_eq!(
            v.sides.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            ["Off", "Long edge (book)", "Short edge (notepad)"]
        );
        assert!(!v.color_enabled && v.color == 1 && v.color_note == "This printer prints in black only");
        assert!(v.is_default && v.can_test && v.can_change);
        assert_eq!(status(&sample()), "2");
    }

    #[test]
    fn another_printer_can_be_made_the_default_and_says_it_isnt_found() {
        let mut state = sample();
        state.selected = Some("HP".into());
        let v = view(&state);
        assert!(!v.is_default);
        assert_eq!((v.hero.meta.as_str(), v.hero.tone.as_str()), ("Not found on the network", "warn"));
        assert!(v.color_enabled && v.color == 0 && v.color_note.is_empty());
        assert_eq!(v.jobs.row_count(), 0);
    }

    #[test]
    fn with_no_printers_the_page_says_none_yet() {
        let mut state = State { loaded: true, ..State::default() };
        state.read.snapshot.server = "CUPS 2.4".into();
        let v = view(&state);
        assert_eq!((v.state.as_str(), v.sub.as_str()), ("empty", "No printers"));
        assert_eq!((v.hero.name.as_str(), v.hero.detail.as_str()), ("No printers yet", "CUPS 2.4"));
        assert_eq!(v.printers.row_count(), 0);
        assert!(v.error.is_empty(), "{}", v.error);
        assert_eq!(status(&state), "");
    }

    #[test]
    fn without_cups_the_page_says_how_to_start_or_install_it() {
        let mut state = State { loaded: true, ..State::default() };
        state.read.not_running = true;
        state.read.installed = true;
        let v = view(&state);
        assert_eq!((v.state.as_str(), v.command.as_str()), ("no-cups", "sudo systemctl enable --now cups.socket"));
        assert_eq!(status(&state), "off");
        state.read.installed = false;
        state.read.install = "sudo pacman -S cups".into();
        assert_eq!(view(&state).command, "sudo pacman -S cups");
        assert_eq!(view(&State::default()).state, "loading");
    }

    #[test]
    fn who_may_change_printers_shows() {
        let mut state = sample();
        state.read.access = Access::None;
        let v = view(&state);
        assert!(!v.can_change && v.access == "none");
        // Other users' jobs need an admin; one's own never.
        let cancel: Vec<bool> = v.jobs.iter().map(|j| j.can_cancel).collect();
        assert_eq!(cancel, [true, false]);
        state.read.access = Access::PkHelper;
        assert_eq!(view(&state).access, "no-agent");
        state.read.agent = true;
        assert_eq!(view(&state).access, "password");
    }

    #[test]
    fn the_add_dialog_lists_what_it_found_once_each() {
        let mut state = sample();
        state.add.open = true;
        state.add.avahi = Some(true);
        let svc = |name: &str, kind: &str, txt: &[(&str, &str)]| Service {
            name: name.into(),
            kind: kind.into(),
            domain: "local".into(),
            host: format!("{}.local.", name.split(' ').next().unwrap().to_lowercase()),
            port: 631,
            interface: 2,
            txt: txt.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        };
        state.add.services = vec![
            svc("Canon TS5300 series", "_ipps._tcp", &[("urf", "V1.4"), ("uuid", "c")]),
            svc("Canon TS5300 series", "_ipp._tcp", &[("urf", "V1.4"), ("uuid", "c")]),
            svc("Brother HL-L2350DW", "_ipp._tcp", &[("pdl", "image/pwg-raster")]),
        ];
        state.read.snapshot.printers[0].device_uri = "ipp://Brother%20HL-L2350DW._ipp._tcp.local/".into();
        state.add.usb = vec![(
            Device {
                uri: "usb://Samsung/ML-2160".into(),
                id: "MFG:Samsung;MDL:ML-2160 Series;CMD:SPL;".into(),
                ..Device::default()
            },
            None,
            Some("splix"),
        )];
        let v = view(&state);
        let rows: Vec<_> = v
            .add_rows
            .iter()
            .map(|r| (r.name.to_string(), r.meta.to_string(), r.action.to_string(), r.primary))
            .collect();
        assert_eq!(
            rows,
            [
                ("Canon TS5300 series".into(), "Network · driverless, nothing to install".into(), "add".into(), true),
                ("Brother HL-L2350DW".into(), "Network · already added".into(), "added".into(), false),
                ("Samsung ML-2160 Series".into(), "USB · needs a driver: splix".into(), "install".into(), false),
            ]
        );
        assert_eq!(v.add_rows.row_data(2).unwrap().note, "Install splix first");
        assert_eq!(v.driver_link, lituus_print::drivers::OPENPRINTING);
    }

    #[test]
    fn times_and_addresses_read_well() {
        // 2026-11-26 02:11 UTC.
        assert_eq!(when(1_795_659_060, 1_795_660_000, 0), "02:11");
        assert_eq!(when(1_795_659_060, 1_795_660_000, 3600), "03:11");
        assert_eq!(when(1_795_659_060 - 86_400, 1_795_660_000, 0), "25 Nov 02:11");
        assert_eq!(parse_offset("+0100"), Some(3600));
        assert_eq!(parse_offset("-0530"), Some(-19_800));
        assert_eq!(
            address("ipps://Brother%20HL-L2350DW._ipps._tcp.local/"),
            "ipps://Brother HL-L2350DW._ipps._tcp.local/"
        );
        assert_eq!(host_port("ipp://brother.local/ipp/print"), Some(("brother.local".into(), 631)));
        assert_eq!(host_port("socket://10.0.0.5:9101"), Some(("10.0.0.5".into(), 9101)));
        assert_eq!(host_port("ipp://[fe80::1]:8631/ipp"), Some(("fe80::1".into(), 8631)));
        assert_eq!(host_port("ipps://Brother%20HL._ipps._tcp.local/"), None);
        assert_eq!(host_port("usb://Brother/HL?serial=1"), None);
    }
}
