//! What CUPS knows, in standard IPP terms (CUPS 3 and Printer Applications use the same).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Idle,
    Printing,
    Stopped,
}

impl State {
    pub(crate) fn from_ipp(v: i32) -> Self {
        match v {
            4 => Self::Printing,
            5 => Self::Stopped,
            _ => Self::Idle,
        }
    }
}

/// How the queue makes what the printer gets.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Driver {
    /// IPP Everywhere, AirPrint: nothing installed.
    #[default]
    Driverless,
    /// A PPD driver, by its name ("Brother HL-L2350DW Foomatic/brlaser").
    Ppd(String),
    /// Sends the bytes as they are.
    Raw,
    /// cups-pdf: writes PDF files.
    Pdf,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Printer {
    /// The queue's name: CUPS's id for it.
    pub name: String,
    /// What it's called ("Brother HL-L2350DW"); the name when empty.
    pub info: String,
    pub location: String,
    pub make_and_model: String,
    pub device_uri: String,
    pub state: State,
    /// printer-state-reasons without "none".
    pub reasons: Vec<String>,
    pub message: String,
    pub accepting: bool,
    pub shared: bool,
    pub driver: Driver,
    pub queued: u32,
}

impl Printer {
    pub fn title(&self) -> &str {
        if self.info.is_empty() { &self.name } else { &self.info }
    }

    /// A printer on the network (not USB, not a file writer): the ones that can go missing.
    pub fn networked(&self) -> bool {
        ["ipp://", "ipps://", "dnssd://", "socket://", "lpd://", "http://", "https://"]
            .iter()
            .any(|s| self.device_uri.starts_with(s))
    }

    pub(crate) fn driver_of(make_and_model: &str, device_uri: &str) -> Driver {
        let m = make_and_model.to_ascii_lowercase();
        if device_uri.starts_with("cups-pdf:") {
            Driver::Pdf
        } else if m.contains("ipp everywhere") || m.contains("driverless") || m.contains("airprint") {
            Driver::Driverless
        } else if m.is_empty() || m.contains("local raw printer") {
            Driver::Raw
        } else {
            Driver::Ppd(make_and_model.to_string())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JobState {
    #[default]
    Pending,
    Held,
    Processing,
    Stopped,
    Canceled,
    Aborted,
    Completed,
}

impl JobState {
    pub(crate) fn from_ipp(v: i32) -> Self {
        match v {
            4 => Self::Held,
            5 => Self::Processing,
            6 => Self::Stopped,
            7 => Self::Canceled,
            8 => Self::Aborted,
            9 => Self::Completed,
            _ => Self::Pending,
        }
    }

    pub fn done(self) -> bool {
        matches!(self, Self::Canceled | Self::Aborted | Self::Completed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Job {
    pub id: i32,
    /// The queue's name.
    pub printer: String,
    /// The document's title; CUPS hides other users' titles from those who aren't admins.
    pub name: String,
    pub user: String,
    /// Sent by this user.
    pub mine: bool,
    pub state: JobState,
    pub pages_done: u32,
    /// Known only when the app said so: CUPS counts pages as they print.
    pub pages: Option<u32>,
    /// When it was sent, in seconds since 1970.
    pub created: i64,
}

/// One of a printer's options: what it can do and what it does by default.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Choices {
    pub supported: Vec<String>,
    pub default: String,
}

/// Paper (PWG media names), two-sided (`one-sided`, `two-sided-long-edge`, `two-sided-short-edge`),
/// colour (`color`, `monochrome`, …) and quality (`draft`, `normal`, `high`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    pub media: Choices,
    pub sides: Choices,
    pub color: Choices,
    pub quality: Choices,
    pub color_supported: bool,
}

pub(crate) fn quality_name(v: i32) -> Option<&'static str> {
    match v {
        3 => Some("draft"),
        4 => Some("normal"),
        5 => Some("high"),
        _ => None,
    }
}

/// Everything the page shows, read at once.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    /// "CUPS 2.4", from cupsd's Server header.
    pub server: String,
    pub printers: Vec<Printer>,
    /// The user's default printer: theirs in ~/.cups/lpoptions, else the system's.
    pub default: Option<String>,
    pub jobs: Vec<Job>,
}
