//! Files over Bluetooth: "Send files…" on a device, with its progress dialog, and the dialog for
//! a file a device sends. A thread talks to obexd on the session bus; received files are moved
//! from obexd's folder to the Downloads folder once complete.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lituus_bt::obex::safe_name;
use lituus_bt::{IncomingFile, Obex, ObexEvent, Transfer};
use tokio::sync::mpsc;

use crate::{FileRow, FilesView, MainWindow, model, update};

pub enum Command {
    Send {
        address: String,
        device: String,
        files: Vec<PathBuf>,
    },
    Cancel,
    /// The answer to the incoming file: accept it, and whether to always accept from its sender.
    Answer {
        accept: bool,
        always: bool,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sending {
    pub device: String,
    pub transfers: Vec<Transfer>,
}

impl Sending {
    /// Files are still on their way.
    pub fn running(&self) -> bool {
        !self.transfers.iter().all(Transfer::done)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Incoming {
    pub device: String,
    pub address: String,
    pub name: String,
    pub size: u64,
    pub kind: String,
    /// Where it goes, as shown.
    pub folder: String,
}

#[derive(Default)]
pub struct State {
    pub sending: Option<Sending>,
    /// The dialog for the transfer is hidden; it carries on.
    pub hidden: bool,
    pub incoming: Option<Incoming>,
    pub always: bool,
    /// The last thing to tell: the files received, or why sending failed.
    pub note: Option<String>,
    commands: Option<mpsc::UnboundedSender<Command>>,
}

impl State {
    pub fn send(&self, command: Command) {
        if let Some(tx) = &self.commands {
            let _ = tx.send(command);
        }
    }
}

fn post(weak: &slint::Weak<MainWindow>, change: impl FnOnce(&mut State) + Send + 'static) {
    let w = weak.clone();
    let _ = weak.upgrade_in_event_loop(move |_| update(&w, |app| change(&mut app.files)));
}

/// Where received files go: `$XDG_DOWNLOAD_DIR`, else the Downloads folder in user-dirs.dirs
/// (where xdg-user-dirs keeps it, in the user's language), else ~/Downloads.
pub fn downloads() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if let Some(dir) = std::env::var_os("XDG_DOWNLOAD_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| home.join(".config"), PathBuf::from);
    std::fs::read_to_string(config.join("user-dirs.dirs"))
        .ok()
        .and_then(|text| user_dir(&text, "XDG_DOWNLOAD_DIR", &home))
        .unwrap_or_else(|| home.join("Downloads"))
}

/// A folder from user-dirs.dirs: `KEY="$HOME/Downloads"` or `KEY="/absolute/path"`.
fn user_dir(text: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let value = text.lines().find_map(|line| line.trim().strip_prefix(key)?.trim_start().strip_prefix('='))?;
    let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None if value.starts_with('/') => PathBuf::from(value),
        None => return None,
    };
    Some(path)
}

/// Asks for files (the desktop's dialog), then sends them.
pub fn choose_and_send(weak: slint::Weak<MainWindow>, address: String, device: String) {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
        let chosen = runtime.block_on(crate::portal::open_files(&format!("Send files to {device}")));
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| {
            update(&w, |app| match chosen {
                Ok(files) if !files.is_empty() => app.files.send(Command::Send { address, device, files }),
                Ok(_) => {}
                Err(e) => app.files.note = Some(format!("{e}.")),
            })
        });
    });
}

/// The name the Bluetooth page knows a device by.
fn device_name(app: &crate::App, address: &str) -> String {
    app.bt
        .snapshot
        .as_ref()
        .and_then(|s| s.devices.iter().find(|d| d.address.eq_ignore_ascii_case(address)))
        .map_or_else(|| address.to_owned(), |d| d.alias.clone())
}

/// A device wants to send a file: ask. In the background or the tray the window isn't shown,
/// and obexd gives up waiting, so it's brought up, as for a Wi-Fi password.
fn ask(window: &MainWindow, incoming: Incoming) {
    crate::APP.with(|app| {
        let mut app = app.borrow_mut();
        let device = device_name(&app, &incoming.address);
        app.files.incoming = Some(Incoming { device, ..incoming });
        app.files.always = false;
    });
    crate::show_for_question(window);
    crate::render(window);
}

/// Starts the thread; without obexd (or a session bus) files simply can't be sent or received.
pub fn spawn(weak: slint::Weak<MainWindow>, always: Vec<String>) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    crate::APP.with(|app| app.borrow_mut().files.commands = Some(tx));
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
        runtime.block_on(async move {
            let Ok(obex) = Obex::session().await else { return };
            let Ok(mut events) = obex.follow().await else { return };
            // Without the agent (another app has one), files can still be sent.
            let _ = obex.register_agent().await;
            let mut always = always;
            let mut waiting: Option<IncomingFile> = None;
            let mut receiving = Receiving::default();
            // What's being sent, and its obexd session.
            let mut sending: Vec<Transfer> = Vec::new();
            let mut session: Option<String> = None;
            // Sent files obexd dropped, and since when: how they ended comes as an event.
            let mut gone: HashMap<String, Instant> = HashMap::new();
            let mut tick = tokio::time::interval(Duration::from_millis(300));
            loop {
                tokio::select! {
                    // What obexd said first: a transfer seen gone may have said how it ended.
                    biased;
                    Some(event) = events.recv() => match event {
                        ObexEvent::Push(file) => {
                            if always.iter().any(|a| a.eq_ignore_ascii_case(&file.address)) {
                                let name = file.name.clone();
                                if let Some(note) = receiving.accept(file, &name) {
                                    post(&weak, move |s| s.note = Some(note));
                                }
                                continue;
                            }
                            let incoming = Incoming { device: String::new(), address: file.address.clone(), name: file.name.clone(), size: file.size, kind: file.kind.clone(), folder: shorten(&downloads()) };
                            if let Some(old) = waiting.replace(file) {
                                old.reject();
                            }
                            let _ = weak.upgrade_in_event_loop(move |window| ask(&window, incoming));
                        }
                        ObexEvent::Cancel => {
                            waiting = None;
                            post(&weak, |s| s.incoming = None);
                        }
                        ObexEvent::Changed { transfer, status, transferred } => {
                            if let Some(t) = sending.iter_mut().find(|t| t.path == transfer && !t.done()) {
                                progress(t, status.as_deref(), transferred);
                                if sending.iter().all(Transfer::done) && let Some(s) = session.take() {
                                    let _ = obex.close(&s).await;
                                }
                                let list = sending.clone();
                                post(&weak, move |s| if let Some(sending) = &mut s.sending { sending.transfers = list; });
                            } else if let Some(status) = status
                                && let Some(note) = receiving.ended(&transfer, &status, &downloads())
                            {
                                post(&weak, move |s| s.note = Some(note));
                            }
                        }
                    },
                    Some(command) = rx.recv() => match command {
                        Command::Answer { accept, always: keep } => {
                            if let Some(file) = waiting.take() {
                                if accept {
                                    if keep && !always.contains(&file.address) {
                                        always.push(file.address.clone());
                                        let _ = crate::config::set_list(&crate::config::path(), "bluetooth", "accept_files_from", &always);
                                    }
                                    let name = file.name.clone();
                                    if let Some(note) = receiving.accept(file, &name) {
                                        post(&weak, move |s| s.note = Some(note));
                                    }
                                } else {
                                    file.reject();
                                }
                            }
                            post(&weak, |s| s.incoming = None);
                        }
                        Command::Send { device, .. } if sending.iter().any(|t| !t.done()) => {
                            // One send at a time: the one running comes back, to be waited for or cancelled.
                            let note = format!("Still sending files: cancel them first to send to {device}");
                            post(&weak, move |s| { s.hidden = false; s.note = Some(note); });
                        }
                        Command::Send { address, device, files } => match obex.send_files(&address, &files).await {
                            Ok((s, transfers)) => {
                                if let Some(old) = session.replace(s) {
                                    let _ = obex.close(&old).await;
                                }
                                sending.clear();
                                gone.clear();
                                for (t, file) in transfers.iter().zip(&files) {
                                    let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
                                    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                    let queued = Transfer { path: t.clone(), name, size, status: "queued".into(), ..Transfer::default() };
                                    sending.push(obex.transfer(t).await.ok().flatten().unwrap_or(queued));
                                }
                                let list = sending.clone();
                                post(&weak, move |s| { s.sending = Some(Sending { device, transfers: list }); s.hidden = false; });
                            }
                            Err(e) => {
                                let note = format!("Couldn't send to {device}: {e}");
                                post(&weak, move |s| s.note = Some(note));
                            }
                        },
                        Command::Cancel => {
                            for t in sending.iter().filter(|t| !t.done()) {
                                let _ = obex.cancel(&t.path).await;
                            }
                            if let Some(s) = session.take() {
                                let _ = obex.close(&s).await;
                            }
                            sending.clear();
                            post(&weak, |s| s.sending = None);
                        }
                    },
                    _ = tick.tick() => {
                        // Progress of what's being sent, between obexd's reports.
                        if sending.iter().any(|t| !t.done()) {
                            for t in sending.iter_mut().filter(|t| !t.done()) {
                                match obex.transfer(&t.path).await {
                                    Ok(Some(now)) => {
                                        gone.remove(&t.path);
                                        progress(t, Some(now.status.as_str()), Some(now.transferred));
                                        if now.size > 0 {
                                            t.size = now.size;
                                        }
                                    }
                                    // Dropped without a word for a while: obexd went away.
                                    Ok(None) if gone.entry(t.path.clone()).or_insert_with(Instant::now).elapsed() > GONE => {
                                        t.status = "error".into();
                                    }
                                    _ => {}
                                }
                            }
                            if sending.iter().all(Transfer::done) && let Some(s) = session.take() {
                                let _ = obex.close(&s).await;
                            }
                            let list = sending.clone();
                            post(&weak, move |s| if let Some(sending) = &mut s.sending { sending.transfers = list; });
                        }
                        // Received files whose transfer obexd dropped without saying how they ended.
                        for transfer in receiving.transfers() {
                            if let Ok(now) = obex.transfer(&transfer).await
                                && let Some(note) = receiving.seen(&transfer, now.is_some(), Instant::now(), &downloads())
                            {
                                post(&weak, move |s| s.note = Some(note));
                            }
                        }
                    },
                }
            }
        });
    });
}

/// How long a transfer obexd dropped waits for word of how it ended. It's on its way when the
/// transfer is found gone; longer means obexd went away.
const GONE: Duration = Duration::from_secs(1);

/// What obexd says about a file being sent.
fn progress(t: &mut Transfer, status: Option<&str>, transferred: Option<u64>) {
    if let Some(n) = transferred {
        t.transferred = n;
    }
    if let Some(status) = status.filter(|s| !s.is_empty()) {
        t.status = status.into();
    }
    if t.status == "complete" {
        t.transferred = t.size;
    }
}

/// A file being received.
#[derive(Debug)]
struct Received {
    /// obexd's transfer: one for all the files a device sends in one go.
    transfer: String,
    /// Where obexd writes it.
    path: PathBuf,
    name: String,
    /// As the sender said; 0 if it didn't.
    size: u64,
    /// Since when its transfer is gone.
    gone: Option<Instant>,
}

/// How receiving a file went.
#[derive(Clone, Debug, PartialEq)]
enum Outcome {
    Received,
    NotMoved(String),
    Failed,
}

/// The files being received, in the order they were accepted, until obexd says how each ended;
/// and how the files of the last batch went, for the note.
#[derive(Debug, Default)]
struct Receiving {
    files: Vec<Received>,
    batch: Option<String>,
    outcomes: Vec<(String, Outcome)>,
}

impl Receiving {
    /// Accepts a file, and gives a note if obexd's folder can't take it.
    fn accept(&mut self, file: IncomingFile, name: &str) -> Option<String> {
        let (transfer, size) = (file.transfer.clone(), file.size);
        match file.accept(name) {
            Ok(path) => {
                self.files.push(Received { transfer, path, name: name.to_owned(), size, gone: None });
                None
            }
            Err(e) => Some(format!("Couldn't receive {}: {e}", safe_name(name))),
        }
    }

    /// The transfers with files waiting.
    fn transfers(&self) -> Vec<String> {
        let mut list: Vec<String> = self.files.iter().map(|f| f.transfer.clone()).collect();
        list.dedup();
        list
    }

    /// A transfer's status changed. A device's files come one after another over one transfer,
    /// each with its own end: "complete" or "error" ends the first file still waiting on it.
    fn ended(&mut self, transfer: &str, status: &str, downloads: &Path) -> Option<String> {
        let ok = match status {
            "complete" => true,
            "error" => false,
            _ => return None,
        };
        let i = self.files.iter().position(|f| f.transfer == transfer)?;
        Some(self.settle(i, ok, downloads))
    }

    /// Whether a transfer is still there. One gone for a while without an end (the device
    /// went before it sent the file, or obexd went away) leaves its files as they are on disk:
    /// whole if they have the size the sender said.
    fn seen(&mut self, transfer: &str, there: bool, now: Instant, downloads: &Path) -> Option<String> {
        let mut note = None;
        while let Some(i) = self.files.iter().position(|f| f.transfer == transfer) {
            let f = &mut self.files[i];
            if there {
                f.gone = None;
                return note;
            }
            if now.duration_since(*f.gone.get_or_insert(now)) < GONE {
                return note;
            }
            let ok = std::fs::metadata(&f.path).is_ok_and(|m| m.len() > 0 && (f.size == 0 || m.len() == f.size));
            note = Some(self.settle(i, ok, downloads));
        }
        note
    }

    /// Moves a received file to the Downloads folder, or removes what's left of a failed one;
    /// gives the note for its batch.
    fn settle(&mut self, i: usize, ok: bool, downloads: &Path) -> String {
        let f = self.files.remove(i);
        let mut name = safe_name(&f.name);
        let outcome = if ok {
            match move_to(&f.path, &name, downloads) {
                // As it's called there: "image (2).jpg" when there was an image.jpg.
                Ok(to) => {
                    name = to.file_name().map_or(name, |n| n.to_string_lossy().into_owned());
                    Outcome::Received
                }
                Err(e) => Outcome::NotMoved(e.to_string()),
            }
        } else {
            // obexd removes a file that broke off, but not one it never started.
            let _ = std::fs::remove_file(&f.path);
            Outcome::Failed
        };
        if self.batch.as_deref() != Some(f.transfer.as_str()) {
            self.batch = Some(f.transfer);
            self.outcomes.clear();
        }
        self.outcomes.push((name, outcome));
        summary(&self.outcomes, downloads)
    }
}

/// Moves a file into `dir`, under a free name; gives where it went.
fn move_to(from: &Path, name: &str, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let to = lituus_bt::obex::free_path(dir, &safe_name(name));
    std::fs::rename(from, &to).or_else(|_| std::fs::copy(from, &to).and_then(|_| std::fs::remove_file(from)))?;
    Ok(to)
}

/// One note for the files a device sent in one go: "Received a.jpg and b.jpg, in ~/Downloads;
/// receiving c.jpg failed".
fn summary(outcomes: &[(String, Outcome)], downloads: &Path) -> String {
    let folder = shorten(downloads);
    let names =
        |want: &Outcome| -> Vec<&str> { outcomes.iter().filter(|(_, o)| o == want).map(|(n, _)| n.as_str()).collect() };
    let mut parts = Vec::new();
    let received = names(&Outcome::Received);
    if !received.is_empty() {
        parts.push(format!("received {}, in {folder}", list(&received)));
    }
    for (name, outcome) in outcomes {
        if let Outcome::NotMoved(e) = outcome {
            parts.push(format!("received {name}, but couldn't move it to {folder}: {e}"));
        }
    }
    let failed = names(&Outcome::Failed);
    if !failed.is_empty() {
        parts.push(format!("receiving {} failed", list(&failed)));
    }
    let text = parts.join("; ");
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// "a.jpg", "a.jpg and b.jpg", "a.jpg, b.jpg and c.jpg", "4 files".
fn list(names: &[&str]) -> String {
    match names {
        [one] => (*one).to_owned(),
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        more => format!("{} files", more.len()),
    }
}

fn shorten(path: &std::path::Path) -> String {
    let p = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && p.starts_with(&home) => format!("~{}", &p[home.len()..]),
        _ => p,
    }
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / f64::from(1u32 << 30)),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / f64::from(1u32 << 20)),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} bytes"),
    }
}

/// "JPEG image", from a MIME type or the name.
fn kind(mime: &str, name: &str) -> String {
    let ext = std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let known = match (mime, ext.as_str()) {
        (m, _) if m.starts_with("image/jpeg") => "JPEG image",
        (_, "jpg" | "jpeg") => "JPEG image",
        (m, _) if m.starts_with("image/png") => "PNG image",
        (_, "png") => "PNG image",
        (m, _) if m.starts_with("image/") => "Image",
        (m, _) if m.starts_with("video/") => "Video",
        (_, "mp4" | "mkv" | "mov" | "webm") => "Video",
        (m, _) if m.starts_with("audio/") => "Audio",
        (_, "pdf") => "PDF document",
        (m, _) if m.contains("vcard") => "Contact card",
        (_, "vcf") => "Contact card",
        _ => "",
    };
    if known.is_empty() { ext.to_uppercase() + " file" } else { known.to_owned() }
}

pub fn view(state: &State) -> FilesView {
    let mut v = FilesView::default();
    if let Some(s) = &state.sending {
        v.sending = !state.hidden;
        v.send_title = format!("Sending files to {}", s.device).into();
        let total: u64 = s.transfers.iter().map(|t| t.size).sum();
        let sent: u64 = s.transfers.iter().map(|t| if t.status == "complete" { t.size } else { t.transferred }).sum();
        let done = s.transfers.iter().filter(|t| t.status == "complete").count();
        let failed = s.transfers.iter().any(|t| t.status == "error");
        v.send_done = s.transfers.iter().all(Transfer::done);
        v.send_summary = if v.send_done && !failed {
            format!("All {} sent · {}", s.transfers.len(), size(total)).into()
        } else {
            format!("{done} of {} sent · {} of {}", s.transfers.len(), size(sent), size(total)).into()
        };
        v.send_fraction = if total > 0 { sent as f32 / total as f32 } else { 0.0 };
        v.files = model(
            s.transfers
                .iter()
                .map(|t| FileRow {
                    name: safe_name(if t.name.is_empty() { &t.filename } else { &t.name }).into(),
                    size: size(t.size).into(),
                    status: match t.status.as_str() {
                        "complete" => "Sent".into(),
                        "error" => "Failed".into(),
                        "active" if t.size > 0 => format!("{} %", t.transferred * 100 / t.size).into(),
                        _ => "Waiting".into(),
                    },
                    fraction: if t.size > 0 { t.transferred as f32 / t.size as f32 } else { 0.0 },
                    active: t.status == "active",
                    tone: match t.status.as_str() {
                        "complete" => "accent".into(),
                        "error" => "danger".into(),
                        _ => "muted".into(),
                    },
                })
                .collect(),
        );
    }
    if let Some(i) = &state.incoming {
        v.incoming = true;
        v.in_title = format!("{} wants to send you a file", i.device).into();
        v.in_name = safe_name(&i.name).into();
        v.in_meta = format!("{} · {}", kind(&i.kind, &i.name), size(i.size)).into();
        v.in_folder = i.folder.as_str().into();
        v.in_always_label = format!("Always accept files from {}", i.device).into();
        v.in_always = state.always;
    }
    v.note = state.note.clone().unwrap_or_default().into();
    v
}

pub fn render(window: &MainWindow, state: &State) {
    window.set_files(view(state));
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::{ComponentHandle, Model};

    fn t(name: &str, size: u64, transferred: u64, status: &str) -> Transfer {
        Transfer {
            path: format!("/t/{name}"),
            name: name.into(),
            size,
            transferred,
            status: status.into(),
            filename: String::new(),
        }
    }

    #[test]
    fn the_send_dialog_adds_up() {
        let state = State {
            sending: Some(Sending {
                device: "Pixel 8".into(),
                transfers: vec![
                    t("a.jpg", 4 << 20, 4 << 20, "complete"),
                    t("b.mp4", 10 << 20, 5 << 20, "active"),
                    t("c.md", 6 << 10, 0, "queued"),
                ],
            }),
            ..State::default()
        };
        let v = view(&state);
        assert!(v.sending && !v.send_done);
        assert_eq!(v.send_title, "Sending files to Pixel 8");
        assert_eq!(v.send_summary, "1 of 3 sent · 9.0 MB of 14.0 MB");
        let statuses: Vec<String> = v.files.iter().map(|f| f.status.to_string()).collect();
        assert_eq!(statuses, ["Sent", "50 %", "Waiting"]);
    }

    #[test]
    fn an_incoming_file_says_what_it_is() {
        let state = State {
            incoming: Some(Incoming {
                device: "Pixel 8".into(),
                address: "AA".into(),
                name: "../PXL_1.jpg".into(),
                size: 3_565_158,
                kind: "image/jpeg".into(),
                folder: "~/Téléchargements".into(),
            }),
            ..State::default()
        };
        let v = view(&state);
        assert_eq!(v.in_title, "Pixel 8 wants to send you a file");
        assert_eq!(v.in_name, "PXL_1.jpg");
        assert_eq!(v.in_meta, "JPEG image · 3.4 MB");
        assert_eq!(v.in_folder, "~/Téléchargements");
    }

    /// A file being received, accepted into `path`.
    fn received(transfer: &str, path: &Path, size: u64) -> Received {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        Received { transfer: transfer.into(), path: path.into(), name, size, gone: None }
    }

    /// obexd's folder and a Downloads folder, for one test.
    fn folders(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("lituus-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("obexd")).unwrap();
        (dir.join("obexd"), dir.join("Downloads"))
    }

    /// A phone sends two files in one go, over one transfer, and the second breaks off: the
    /// first, whole, is moved to Downloads, and the note tells about both (04-2, and one note
    /// for two results).
    #[test]
    fn files_sent_in_one_go_end_one_by_one() {
        let (cache, downloads) = folders("batch");
        let t = "/org/bluez/obex/server/session1/transfer0";
        let mut r = Receiving::default();
        std::fs::write(cache.join("a.jpg"), vec![1; 3000]).unwrap();
        r.files.push(received(t, &cache.join("a.jpg"), 3000));
        assert_eq!(r.ended(t, "active", &downloads), None);
        let note = r.ended(t, "complete", &downloads);
        assert_eq!(note.unwrap(), format!("Received a.jpg, in {}", shorten(&downloads)));
        // The next file: queued, accepted, and it breaks off (obexd removes what came of it).
        assert_eq!(r.ended(t, "queued", &downloads), None);
        r.files.push(received(t, &cache.join("b.jpg"), 1000));
        let note = r.ended(t, "error", &downloads);
        assert_eq!(note.unwrap(), format!("Received a.jpg, in {}; receiving b.jpg failed", shorten(&downloads)));
        assert_eq!(std::fs::read(downloads.join("a.jpg")).unwrap().len(), 3000);
        assert!(r.files.is_empty());
        std::fs::remove_dir_all(cache.parent().unwrap()).unwrap();
    }

    /// Two files with the same name: both are kept, and the note says what the second is called.
    #[test]
    fn a_second_file_with_the_same_name_is_kept_too() {
        let (cache, downloads) = folders("same-name");
        let t = "/org/bluez/obex/server/session1/transfer0";
        let mut r = Receiving::default();
        for (path, size) in [(cache.join("image.jpg"), 3000), (cache.join("image (2).jpg"), 1000)] {
            std::fs::write(&path, vec![1; size]).unwrap();
            r.files.push(Received { name: "image.jpg".into(), ..received(t, &path, size as u64) });
        }
        r.ended(t, "complete", &downloads);
        let note = r.ended(t, "complete", &downloads).unwrap();
        assert_eq!(note, format!("Received image.jpg and image (2).jpg, in {}", shorten(&downloads)));
        assert_eq!(std::fs::metadata(downloads.join("image.jpg")).unwrap().len(), 3000);
        assert_eq!(std::fs::metadata(downloads.join("image (2).jpg")).unwrap().len(), 1000);
        std::fs::remove_dir_all(cache.parent().unwrap()).unwrap();
    }

    /// Another device's files are a batch of their own.
    #[test]
    fn each_device_gets_its_own_note() {
        let (cache, downloads) = folders("devices");
        let mut r = Receiving::default();
        for (t, name) in [("/s1/transfer0", "a.jpg"), ("/s2/transfer0", "b.jpg")] {
            std::fs::write(cache.join(name), b"x").unwrap();
            r.files.push(received(t, &cache.join(name), 1));
        }
        assert_eq!(
            r.ended("/s2/transfer0", "complete", &downloads).unwrap(),
            format!("Received b.jpg, in {}", shorten(&downloads))
        );
        assert_eq!(
            r.ended("/s1/transfer0", "complete", &downloads).unwrap(),
            format!("Received a.jpg, in {}", shorten(&downloads))
        );
        std::fs::remove_dir_all(cache.parent().unwrap()).unwrap();
    }

    /// The transfer went without saying how it ended (the phone left before sending, or obexd
    /// went away): the file on disk tells, once word of the end had time to come (03-7).
    #[test]
    fn a_transfer_gone_without_a_word_is_judged_by_its_file() {
        let (cache, downloads) = folders("gone");
        let mut r = Receiving::default();
        // a.jpg came whole; b.jpg never started, so only the empty file kept for it is there.
        std::fs::write(cache.join("a.jpg"), vec![0; 3000]).unwrap();
        std::fs::write(cache.join("b.jpg"), b"").unwrap();
        r.files.push(received("/s1/transfer0", &cache.join("a.jpg"), 3000));
        r.files.push(received("/s2/transfer0", &cache.join("b.jpg"), 1000));
        let now = Instant::now();
        assert_eq!(r.seen("/s1/transfer0", true, now, &downloads), None);
        assert_eq!(r.seen("/s1/transfer0", false, now, &downloads), None, "how it ended may be on its way");
        let later = now + GONE * 2;
        assert_eq!(
            r.seen("/s1/transfer0", false, later, &downloads).unwrap(),
            format!("Received a.jpg, in {}", shorten(&downloads))
        );
        assert_eq!(r.seen("/s2/transfer0", false, now, &downloads), None);
        assert_eq!(r.seen("/s2/transfer0", false, later, &downloads).unwrap(), "Receiving b.jpg failed");
        assert!(downloads.join("a.jpg").exists());
        assert!(!cache.join("b.jpg").exists(), "the empty file is cleared away");
        std::fs::remove_dir_all(cache.parent().unwrap()).unwrap();
    }

    #[test]
    fn notes_list_the_files() {
        let d = Path::new("/srv/in");
        let n = |name: &str, o: Outcome| (name.to_owned(), o);
        assert_eq!(
            summary(&[n("a.jpg", Outcome::Received), n("b.jpg", Outcome::Received), n("c.jpg", Outcome::Received)], d),
            "Received a.jpg, b.jpg and c.jpg, in /srv/in"
        );
        let four: Vec<_> = (0..4).map(|i| n(&format!("{i}.jpg"), Outcome::Received)).collect();
        assert_eq!(summary(&four, d), "Received 4 files, in /srv/in");
        assert_eq!(
            summary(&[n("a.jpg", Outcome::NotMoved("Permission denied".into())), n("b.jpg", Outcome::Failed)], d),
            "Received a.jpg, but couldn't move it to /srv/in: Permission denied; receiving b.jpg failed"
        );
    }

    /// The Downloads folder has the name xdg-user-dirs gave it, in the user's language (04-5).
    #[test]
    fn the_downloads_folder_comes_from_user_dirs() {
        let home = Path::new("/home/me");
        let dirs = "# written by xdg-user-dirs-update\nXDG_DESKTOP_DIR=\"$HOME/Bureau\"\nXDG_DOWNLOAD_DIR=\"$HOME/Téléchargements\"\n";
        assert_eq!(user_dir(dirs, "XDG_DOWNLOAD_DIR", home), Some(home.join("Téléchargements")));
        assert_eq!(user_dir("XDG_DOWNLOAD_DIR=\"/data/in\"", "XDG_DOWNLOAD_DIR", home), Some("/data/in".into()));
        assert_eq!(user_dir("XDG_DOWNLOAD_DIRS=\"/x\"", "XDG_DOWNLOAD_DIR", home), None);
        assert_eq!(user_dir("#XDG_DOWNLOAD_DIR=\"/x\"", "XDG_DOWNLOAD_DIR", home), None);
    }

    /// The phone declined the file: it says so, and the files don't count as sent (04-1).
    #[test]
    fn a_declined_file_isnt_sent() {
        let mut a = t("a.jpg", 4000, 1200, "active");
        progress(&mut a, Some("error"), None);
        let state =
            State { sending: Some(Sending { device: "Pixel 8".into(), transfers: vec![a] }), ..State::default() };
        let v = view(&state);
        assert!(v.send_done);
        assert_eq!(v.send_summary, "0 of 1 sent · 1 KB of 3 KB");
        assert_eq!(v.files.row_data(0).unwrap().status, "Failed");
    }

    /// Started in the background, or closed to the tray: a file a device sends brings the window
    /// up, with the question, or obexd gives up waiting (04-6).
    #[test]
    fn a_file_brings_up_a_hidden_window() {
        i_slint_backend_testing::init_no_event_loop();
        let window = MainWindow::new().unwrap();
        assert!(!window.window().is_visible());
        ask(&window, Incoming { address: "AA".into(), name: "PXL_1.jpg".into(), size: 3000, ..Incoming::default() });
        assert!(window.window().is_visible(), "the question waits in a hidden window");
        assert!(window.get_files().incoming);
    }
}
