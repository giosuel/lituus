//! cupsd through its domain socket. Reads need nothing; changes need CUPS's SystemGroup, proved by
//! PeerCred: the name sent must be the one the socket's peer uid has.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ipp::model::{DelimiterTag, Operation};
use ipp::prelude::IppValue;

use crate::http;
use crate::lpoptions;
use crate::model::{Defaults, Job, JobState, Options, Printer, Snapshot, State, quality_name, quality_value};
use crate::ppd;
use crate::proto::{self, Answer, Group, Request};
use crate::user;

pub const SOCKET: &str = "/run/cups/cups.sock";
const READ: Duration = Duration::from_secs(10);
/// cupsd waits 30 s for a printer that doesn't answer before it gives up adding it.
const ADD: Duration = Duration::from_secs(40);
/// IPP's client-error-not-found.
const NOT_FOUND: u16 = 0x0406;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Nothing listens on the socket.
    #[error("CUPS isn't running")]
    NotRunning,
    /// Only CUPS's admins (or root, through cups-pk-helper) may.
    #[error("not allowed{}", if .0.is_empty() { String::new() } else { format!(": {}", .0) })]
    NotAllowed(String),
    #[error("{message} (IPP status {status:#06x})")]
    Ipp { status: u16, message: String },
    #[error("CUPS answered HTTP {0}")]
    Http(u16),
    #[error("CUPS didn't answer in time")]
    Timeout,
    #[error("{0}")]
    Parse(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    DBus(#[from] zbus::Error),
}

/// What printers' attributes are read for the list.
const PRINTER_ATTRS: &[&str] = &[
    "printer-name",
    "printer-info",
    "printer-location",
    "printer-make-and-model",
    "printer-state",
    "printer-state-reasons",
    "printer-state-message",
    "printer-type",
    "printer-is-accepting-jobs",
    "printer-is-shared",
    "printer-is-temporary",
    "device-uri",
    "queued-job-count",
];

const JOB_ATTRS: &[&str] = &[
    "job-id",
    "job-name",
    "job-state",
    "job-originating-user-name",
    "job-printer-uri",
    "job-impressions",
    "job-impressions-completed",
    "job-media-sheets-completed",
    "time-at-creation",
];

const OPTION_ATTRS: &[&str] = &[
    "media-supported",
    "media-default",
    "sides-supported",
    "sides-default",
    "print-color-mode-supported",
    "print-color-mode-default",
    "print-quality-supported",
    "print-quality-default",
    "color-supported",
];

/// CUPS's own test page: the banner file cupsd's filters turn into a page with the printer's
/// details. Sent as it is, so a Flatpak needs no access to /usr/share/cups.
const TEST_PAGE: &str = "#PDF-BANNER\nTemplate default-testpage.pdf\nShow printer-name printer-info printer-location printer-make-and-model printer-driver-name printer-driver-version paper-size imageable-area\n";

/// "CUPS 2.4" from "CUPS/2.4 IPP/2.1".
fn server_name(header: &str) -> String {
    header.split_whitespace().next().unwrap_or(header).replace('/', " ")
}

pub fn printer_uri(name: &str) -> String {
    format!("ipp://localhost/printers/{}", encode_path(name))
}

fn encode_path(s: &str) -> String {
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

/// The queue's name in a job's or printer's URI.
fn name_in(uri: &str) -> String {
    let last = uri.rsplit('/').next().unwrap_or(uri);
    let bytes = last.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = last.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn printer_from(g: &Group) -> Option<Printer> {
    let name = g.text("printer-name")?;
    let kind = g.int("printer-type").unwrap_or(0);
    // Classes (groups of printers) and the queues CUPS makes on its own while an app prints to a
    // printer on the network aren't printers to manage.
    if kind & 0x1 != 0 || g.bool("printer-is-temporary").unwrap_or(false) {
        return None;
    }
    let make_and_model = g.text("printer-make-and-model").unwrap_or_default();
    let device_uri = g.text("device-uri").unwrap_or_default();
    Some(Printer {
        info: g.text("printer-info").unwrap_or_default(),
        location: g.text("printer-location").unwrap_or_default(),
        driver: Printer::driver_of(&make_and_model, &device_uri),
        make_and_model,
        device_uri,
        state: State::from_ipp(g.int("printer-state").unwrap_or(3)),
        reasons: g.texts("printer-state-reasons").into_iter().filter(|r| r != "none").collect(),
        message: g.text("printer-state-message").unwrap_or_default(),
        accepting: g.bool("printer-is-accepting-jobs").unwrap_or(true),
        shared: g.bool("printer-is-shared").unwrap_or(false),
        queued: g.int("queued-job-count").unwrap_or(0).max(0) as u32,
        name,
    })
}

pub(crate) fn job_from(g: &Group, me: &str) -> Option<Job> {
    let id = g.int("job-id")?;
    let user = g.text("job-originating-user-name").unwrap_or_default();
    let done = g.int("job-impressions-completed").unwrap_or(0).max(g.int("job-media-sheets-completed").unwrap_or(0));
    Some(Job {
        id,
        printer: g.text("job-printer-uri").map(|u| name_in(&u)).unwrap_or_default(),
        name: g.text("job-name").unwrap_or_default(),
        mine: user == me,
        user,
        state: JobState::from_ipp(g.int("job-state").unwrap_or(3)),
        pages_done: done.max(0) as u32,
        pages: g.int("job-impressions").filter(|n| *n > 0).map(|n| n as u32),
        created: g.int("time-at-creation").unwrap_or(0) as i64,
    })
}

pub(crate) fn options_from(g: &Group) -> Options {
    let choices = |sup: &str, def: &str| crate::model::Choices {
        supported: g.texts(sup),
        default: g.text(def).unwrap_or_default(),
    };
    let quality = crate::model::Choices {
        supported: g.ints("print-quality-supported").into_iter().filter_map(quality_name).map(String::from).collect(),
        default: g.int("print-quality-default").and_then(quality_name).unwrap_or_default().to_string(),
    };
    let color = choices("print-color-mode-supported", "print-color-mode-default");
    Options {
        color_supported: g.bool("color-supported").unwrap_or_else(|| color.supported.iter().any(|c| c == "color")),
        media: choices("media-supported", "media-default"),
        sides: choices("sides-supported", "sides-default"),
        color,
        quality,
    }
}

/// A local cupsd: its socket, and the user the requests are made as.
#[derive(Debug, Clone)]
pub struct Client {
    socket: PathBuf,
    user: String,
}

impl Client {
    /// cupsd's usual socket, or the one CUPS_SERVER names when it's a path.
    pub fn local() -> Self {
        let socket = std::env::var("CUPS_SERVER")
            .ok()
            .filter(|s| s.starts_with('/'))
            .map(PathBuf::from)
            .unwrap_or_else(|| SOCKET.into());
        Self { socket, user: user::name() }
    }

    pub fn new(socket: impl Into<PathBuf>, user: impl Into<String>) -> Self {
        Self { socket: socket.into(), user: user.into() }
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Sends a request; on a PeerCred challenge, once more with the user's name. Returns the
    /// answer and the server's name, or the error the answer reports.
    async fn call(
        &self,
        path: &str,
        req: &Request,
        document: &[u8],
        limit: Duration,
    ) -> Result<(Answer, String), Error> {
        let (a, server) = self.answer(path, req, document, limit).await?;
        Ok((a.checked()?, server))
    }

    /// As [`Self::call`], with the answer unchecked: IPP's errors are the caller's to read.
    async fn answer(
        &self,
        path: &str,
        req: &Request,
        document: &[u8],
        limit: Duration,
    ) -> Result<(Answer, String), Error> {
        let body = req.bytes(document);
        let mut resp =
            http::send(&self.socket, &http::encode("POST", path, "application/ipp", None, &body), limit).await?;
        if resp.status == 401 && resp.header("WWW-Authenticate").is_some_and(|h| h.contains("PeerCred")) {
            let auth = format!("PeerCred {}", self.user);
            resp = http::send(&self.socket, &http::encode("POST", path, "application/ipp", Some(&auth), &body), limit)
                .await?;
        }
        match resp.status {
            200 => {}
            401 | 403 => return Err(Error::NotAllowed(String::new())),
            s => return Err(Error::Http(s)),
        }
        let server = resp.header("Server").map(server_name).unwrap_or_default();
        Ok((proto::parse(&resp.body)?, server))
    }

    async fn printers_and_server(&self) -> Result<(Vec<Printer>, String), Error> {
        let req = Request::new(Operation::CupsGetPrinters, None, &self.user).wanted(PRINTER_ATTRS);
        let (a, server) = self.answer("/", &req, b"", READ).await?;
        // With no printers, cupsd answers "not found" ("No destinations added.") rather than with
        // an empty list.
        if a.status == NOT_FOUND {
            return Ok((Vec::new(), server));
        }
        let a = a.checked()?;
        let mut list: Vec<Printer> = a.of(DelimiterTag::PrinterAttributes).filter_map(printer_from).collect();
        list.sort_by_key(|p| p.title().to_lowercase());
        Ok((list, server))
    }

    pub async fn printers(&self) -> Result<Vec<Printer>, Error> {
        Ok(self.printers_and_server().await?.0)
    }

    /// The system's default printer.
    pub async fn server_default(&self) -> Result<Option<String>, Error> {
        let req = Request::new(Operation::CupsGetDefault, None, &self.user).wanted(&["printer-name"]);
        match self.call("/", &req, b"", READ).await {
            Ok((a, _)) => Ok(a.of(DelimiterTag::PrinterAttributes).find_map(|g| g.text("printer-name"))),
            Err(Error::Ipp { status: NOT_FOUND, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Every printer's jobs that aren't done.
    pub async fn jobs(&self) -> Result<Vec<Job>, Error> {
        let req = Request::new(Operation::GetJobs, Some("ipp://localhost/"), &self.user)
            .op("which-jobs", proto::keyword("not-completed"))
            .wanted(JOB_ATTRS);
        let (a, _) = self.call("/", &req, b"", READ).await?;
        let mut jobs: Vec<Job> = a.of(DelimiterTag::JobAttributes).filter_map(|g| job_from(g, &self.user)).collect();
        jobs.sort_by_key(|j| j.id);
        Ok(jobs)
    }

    /// A printer's options: what it can do and its defaults.
    pub async fn options(&self, name: &str) -> Result<Options, Error> {
        let req =
            Request::new(Operation::GetPrinterAttributes, Some(&printer_uri(name)), &self.user).wanted(OPTION_ATTRS);
        let (a, _) = self.call("/", &req, b"", READ).await?;
        Ok(a.of(DelimiterTag::PrinterAttributes).next().map(options_from).unwrap_or_default())
    }

    /// The printers, the user's default and the jobs, read afresh.
    pub async fn snapshot(&self) -> Result<Snapshot, Error> {
        let (printers, server) = self.printers_and_server().await?;
        let own = lpoptions::read_default().filter(|d| printers.iter().any(|p| &p.name == d));
        let default = match own {
            Some(d) => Some(d),
            None => self.server_default().await?,
        };
        let jobs = self.jobs().await?;
        Ok(Snapshot { server, printers, default, jobs })
    }

    /// Cancels a job: the user's own always; others' only as an admin.
    pub async fn cancel(&self, job: i32) -> Result<(), Error> {
        let req = Request::new(Operation::CancelJob, Some("ipp://localhost/"), &self.user)
            .op("job-id", IppValue::Integer(job));
        self.call("/jobs/", &req, b"", READ).await.map(drop)
    }

    /// Prints CUPS's test page. Returns the job's id.
    pub async fn print_test_page(&self, name: &str) -> Result<i32, Error> {
        let req = Request::new(Operation::PrintJob, Some(&printer_uri(name)), &self.user)
            .op("job-name", proto::name("Test page"))
            .op("document-format", IppValue::new_mime_media_type("application/octet-stream").expect("short"));
        let (a, _) = self.call(&format!("/printers/{}", encode_path(name)), &req, TEST_PAGE.as_bytes(), READ).await?;
        Ok(a.of(DelimiterTag::JobAttributes).find_map(|g| g.int("job-id")).unwrap_or(0))
    }

    /// Removes a printer (admins only).
    pub async fn remove(&self, name: &str) -> Result<(), Error> {
        let req = Request::new(Operation::CupsDeletePrinter, Some(&printer_uri(name)), &self.user);
        self.call("/admin/", &req, b"", READ).await.map(drop)
    }

    /// The printer's PPD (CUPS 2), if it has one: driverless queues have one cupsd made, raw ones none.
    pub async fn ppd(&self, name: &str) -> Result<Option<String>, Error> {
        let path = format!("/printers/{}.ppd", encode_path(name));
        let resp = http::send(&self.socket, &http::encode("GET", &path, "", None, b""), READ).await?;
        match resp.status {
            200 => Ok(Some(String::from_utf8_lossy(&resp.body).into_owned())),
            404 => Ok(None),
            s => Err(Error::Http(s)),
        }
    }

    /// The PPD's choices for new defaults, where it has them.
    pub async fn ppd_choices(
        &self,
        name: &str,
        d: &Defaults,
    ) -> Result<(Option<String>, Vec<(String, String)>), Error> {
        let Some(file) = self.ppd(name).await? else { return Ok((None, Vec::new())) };
        let set =
            ppd::choices_for(&file, d.media.as_deref(), d.sides.as_deref(), d.color.as_deref(), d.quality.as_deref());
        Ok((Some(file), set))
    }

    /// New defaults for what the printer prints (admins only): the IPP `*-default` attributes and
    /// the PPD's defaults in one request, so apps that read either agree.
    pub async fn set_options(&self, name: &str, d: &Defaults) -> Result<(), Error> {
        let mut req = Request::new(Operation::CupsAddModifyPrinter, Some(&printer_uri(name)), &self.user);
        if let Some(m) = &d.media {
            req = req.printer("media-default", proto::keyword(m));
        }
        if let Some(s) = &d.sides {
            req = req.printer("sides-default", proto::keyword(s));
        }
        if let Some(c) = &d.color {
            req = req.printer("print-color-mode-default", proto::keyword(c));
        }
        if let Some(q) = d.quality.as_deref().and_then(quality_value) {
            req = req.printer("print-quality-default", IppValue::Enum(q));
        }
        let (file, set) = self.ppd_choices(name, d).await?;
        let document = match file {
            Some(f) if !set.is_empty() => ppd::with_defaults(&f, &set).into_bytes(),
            _ => Vec::new(),
        };
        self.call("/admin/", &req, &document, READ).await.map(drop)
    }

    /// Adds a printer (admins only): `ppd` is a driver's name, or "everywhere" for a driverless
    /// one, which cupsd asks the printer about itself. Not shared on the network; the paper is
    /// the one this computer's locale uses, when the printer has it.
    pub async fn add(&self, new: &NewPrinter, paper: &str) -> Result<(), Error> {
        let req = Request::new(Operation::CupsAddModifyPrinter, Some(&printer_uri(&new.name)), &self.user)
            .printer("device-uri", proto::uri(&new.uri))
            // A name, not a keyword: as a keyword cupsd quietly makes a raw queue.
            .printer("ppd-name", proto::name(&new.ppd))
            .printer("printer-info", proto::text(&new.info))
            .printer("printer-location", proto::text(&new.location))
            .printer("printer-is-accepting-jobs", IppValue::Boolean(true))
            .printer("printer-state", IppValue::Enum(3))
            .printer("printer-is-shared", IppValue::Boolean(false));
        self.call("/admin/", &req, b"", ADD).await?;
        if let Ok(o) = self.options(&new.name).await
            && o.media.supported.iter().any(|m| m == paper)
            && o.media.default != paper
        {
            let _ =
                self.set_options(&new.name, &Defaults { media: Some(paper.to_string()), ..Defaults::default() }).await;
        }
        Ok(())
    }

    /// A printer's name for people (printer-info; admins only).
    pub async fn set_info(&self, name: &str, info: &str) -> Result<(), Error> {
        let req = Request::new(Operation::CupsAddModifyPrinter, Some(&printer_uri(name)), &self.user)
            .printer("printer-info", proto::text(info));
        self.call("/admin/", &req, b"", READ).await.map(drop)
    }

    /// USB printers CUPS sees (admins only), with their IEEE 1284 device IDs.
    pub async fn usb_devices(&self) -> Result<Vec<Device>, Error> {
        let req = Request::new(Operation::CupsGetDevices, None, &self.user)
            .op("include-schemes", IppValue::Array(vec![proto::name("usb")]))
            .op("timeout", IppValue::Integer(5));
        let (a, _) = self.call("/", &req, b"", READ).await?;
        Ok(a.of(DelimiterTag::PrinterAttributes)
            .filter_map(|g| {
                Some(Device {
                    uri: g.text("device-uri")?,
                    id: g.text("device-id").unwrap_or_default(),
                    make_and_model: g.text("device-make-and-model").unwrap_or_default(),
                    info: g.text("device-info").unwrap_or_default(),
                })
            })
            .filter(|d| d.uri.starts_with("usb://"))
            .collect())
    }

    /// An installed driver for a device ID, by its PPD name.
    pub async fn ppd_for(&self, device_id: &str) -> Result<Option<String>, Error> {
        let req = Request::new(Operation::CupsGetPPDs, None, &self.user)
            .op("ppd-device-id", proto::text(device_id))
            .op("limit", IppValue::Integer(1))
            .wanted(&["ppd-name"]);
        match self.call("/", &req, b"", READ).await {
            Ok((a, _)) => Ok(a.of(DelimiterTag::PrinterAttributes).find_map(|g| g.text("ppd-name"))),
            Err(Error::Ipp { status: NOT_FOUND, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// cupsd lets this user make admin changes: asked with an admin-only request that does
    /// nothing (a device list for a scheme no backend has). For CUPS admin groups besides wheel,
    /// sys, root and lpadmin, or for root.
    pub async fn can_admin(&self) -> bool {
        let req = Request::new(Operation::CupsGetDevices, None, &self.user)
            .op("include-schemes", IppValue::Array(vec![proto::name("lituus-none")]))
            .op("timeout", IppValue::Integer(1));
        self.call("/", &req, b"", READ).await.is_ok()
    }

    /// Makes `name` this user's default printer (~/.cups/lpoptions); other users keep theirs.
    pub fn set_default(&self, name: &str) -> Result<(), Error> {
        Ok(lpoptions::write_default(name)?)
    }
}

/// A printer to add.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewPrinter {
    /// The queue's name: letters, digits, `_` and `-` (see `queue_name`).
    pub name: String,
    pub uri: String,
    /// "everywhere" (driverless), or an installed driver's PPD name.
    pub ppd: String,
    pub info: String,
    pub location: String,
}

/// A USB printer as CUPS's usb backend sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Device {
    pub uri: String,
    /// IEEE 1284: "MFG:Samsung;MDL:ML-2160 Series;CMD:SPL;".
    pub id: String,
    pub make_and_model: String,
    pub info: String,
}

/// A queue name from a printer's name ("Canon_TS5300_series"): letters, digits, `_` and `-`, at
/// most 127; `_2` and on when it's taken.
pub fn queue_name(title: &str, taken: &[String]) -> String {
    let mut base = String::new();
    for c in title.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' };
        if !(c == '_' && base.ends_with('_')) {
            base.push(c);
        }
    }
    let base: String = base.trim_matches('_').chars().take(120).collect();
    let base = if base.is_empty() { "Printer".to_string() } else { base };
    let free = |n: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(n));
    if free(&base) {
        return base;
    }
    (2..).map(|i| format!("{base}_{i}")).find(|n| free(n)).expect("a free name")
}

/// What was typed for "Add by its address": an IP or a name ("192.168.1.40", "printer.local",
/// "printer:8631"), or a whole ipp:// or ipps:// URI. Returns the URI, and the host and port to
/// check it answers.
pub fn address(typed: &str) -> Option<(String, String, u16)> {
    let t = typed.trim();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    let (scheme, rest) = match t.split_once("://") {
        Some((s, r)) if s == "ipp" || s == "ipps" => (s, r),
        Some(_) => return None,
        None => ("ipp", t),
    };
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) if !p.is_empty() => (a, format!("/{p}")),
        Some((a, _)) => (a, "/ipp/print".to_string()),
        None => (rest, "/ipp/print".to_string()),
    };
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        (h.to_string(), after.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(Some(631))?)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().ok()?),
            None => (authority.to_string(), 631),
        }
    };
    if host.is_empty() {
        return None;
    }
    let shown = if host.contains(':') { format!("[{host}]") } else { host.clone() };
    let port_part = if port == 631 { String::new() } else { format!(":{port}") };
    Some((format!("{scheme}://{shown}{port_part}{path}"), host, port))
}

/// Something answers at the printer's address within 5 seconds (before cupsd's 30 s try).
pub async fn answers(host: &str, port: u16) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(5), tokio::net::TcpStream::connect((host, port))).await,
        Ok(Ok(_))
    )
}

/// cupsd is installed here, so starting it is enough (else it's to install).
pub fn installed() -> bool {
    ["/usr/bin/cupsd", "/usr/sbin/cupsd"].iter().any(|p| Path::new(p).exists())
}

/// Lituus manages only the local cupsd; when apps are told to print through another server
/// (CUPS_SERVER or ~/.cups/client.conf), its name.
pub fn elsewhere() -> Option<String> {
    let named = std::env::var("CUPS_SERVER").ok().or_else(|| {
        let home = std::env::var_os("HOME")?;
        let conf = std::fs::read_to_string(PathBuf::from(home).join(".cups/client.conf")).ok()?;
        conf.lines().find_map(|l| l.trim().strip_prefix("ServerName").map(|v| v.trim().to_string()))
    })?;
    let host = named.split(':').next().unwrap_or(&named).to_string();
    (!named.starts_with('/') && !["localhost", "127.0.0.1", "::1", ""].contains(&host.as_str())).then_some(named)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Driver;

    /// A request's head and the body its Content-Length gives have all come.
    fn whole(got: &[u8]) -> bool {
        let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") else { return false };
        let length = String::from_utf8_lossy(&got[..end])
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length: ").and_then(|v| v.trim().parse::<usize>().ok()))
            .unwrap_or(0);
        got.len() >= end + 4 + length
    }

    /// A cupsd on a socket of its own that answers every request with `ipp`.
    async fn fake_cupsd(name: &str, ipp: Vec<u8>) -> PathBuf {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = std::env::temp_dir().join(format!("lituus-print-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("cups.sock");
        let _ = std::fs::remove_file(&socket);
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                // The whole request first, so the answer isn't cut off by an unread body.
                let mut got = Vec::new();
                let mut chunk = [0u8; 4096];
                while let Ok(n @ 1..) = stream.read(&mut chunk).await {
                    got.extend_from_slice(&chunk[..n]);
                    if whole(&got) {
                        break;
                    }
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nServer: CUPS/2.4 IPP/2.1\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n",
                    ipp.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(&ipp).await;
            }
        });
        socket
    }

    #[tokio::test]
    async fn no_printers_is_an_empty_list_not_an_error() {
        // What cupsd answers CUPS-Get-Printers before any printer is added.
        let none = proto::tests::response(
            NOT_FOUND,
            vec![(DelimiterTag::OperationAttributes, vec![("status-message", proto::text("No destinations added."))])],
        );
        let socket = fake_cupsd("no-printers", none).await;
        let c = Client::new(&socket, "anna");
        let (printers, server) = c.printers_and_server().await.unwrap();
        assert!(printers.is_empty(), "{printers:?}");
        assert_eq!(server, "CUPS 2.4");
        assert_eq!(c.server_default().await.unwrap(), None);
        let _ = std::fs::remove_dir_all(socket.parent().unwrap());
    }

    #[test]
    fn names_and_uris_round_trip() {
        assert_eq!(printer_uri("Office"), "ipp://localhost/printers/Office");
        assert_eq!(printer_uri("Büro 2"), "ipp://localhost/printers/B%C3%BCro%202");
        assert_eq!(name_in("ipp://localhost/printers/B%C3%BCro%202"), "Büro 2");
        assert_eq!(name_in("ipp://host:631/printers/Office"), "Office");
        assert_eq!(server_name("CUPS/2.4 IPP/2.1"), "CUPS 2.4");
    }

    #[test]
    fn queue_names_are_cupss_and_free() {
        assert_eq!(queue_name("Canon TS5300 series", &[]), "Canon_TS5300_series");
        assert_eq!(queue_name("Büro (2. Stock)", &[]), "B_ro_2_Stock");
        assert_eq!(queue_name("Office", &["office".into(), "Office_2".into()]), "Office_3");
        assert_eq!(queue_name("???", &[]), "Printer");
    }

    #[test]
    fn typed_addresses_become_uris() {
        assert_eq!(address("192.168.1.40"), Some(("ipp://192.168.1.40/ipp/print".into(), "192.168.1.40".into(), 631)));
        assert_eq!(
            address("printer.local:8631"),
            Some(("ipp://printer.local:8631/ipp/print".into(), "printer.local".into(), 8631))
        );
        assert_eq!(
            address("ipps://p.local/ipp/print"),
            Some(("ipps://p.local/ipp/print".into(), "p.local".into(), 631))
        );
        assert_eq!(address("[fe80::1]"), Some(("ipp://[fe80::1]/ipp/print".into(), "fe80::1".into(), 631)));
        assert_eq!(address("http://x"), None);
        assert_eq!(address("a b"), None);
        assert_eq!(address(""), None);
    }

    #[test]
    fn drivers_are_told_apart_by_the_make_and_model() {
        assert_eq!(Printer::driver_of("Brother HL-L2350DW - IPP Everywhere", "ipp://x"), Driver::Driverless);
        assert_eq!(
            Printer::driver_of("Canon TS5300 series, driverless, cups-filters 2.0.1", "ipp://x"),
            Driver::Driverless
        );
        assert_eq!(Printer::driver_of("Generic CUPS-PDF Printer (w/ options)", "cups-pdf:/"), Driver::Pdf);
        assert_eq!(Printer::driver_of("Local Raw Printer", "socket://x"), Driver::Raw);
        assert_eq!(
            Printer::driver_of("Samsung ML-2160, SpliX V. 2.0.1", "usb://x"),
            Driver::Ppd("Samsung ML-2160, SpliX V. 2.0.1".into())
        );
        let p = |m: &str| Printer { make_and_model: m.into(), ..Printer::default() };
        assert_eq!(p("Lituus Virtual Duplex - IPP Everywhere").model_name(), "Lituus Virtual Duplex");
        assert_eq!(p("Canon TS5300 series, driverless, cups-filters 2.0.1").model_name(), "Canon TS5300 series");
    }
}
