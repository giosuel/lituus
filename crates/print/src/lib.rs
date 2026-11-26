//! Printers through CUPS. cupsd is spoken to with IPP over its domain socket (not port 631, not
//! libcups): reading needs nothing, and CUPS's admins (wheel, lpadmin) change printers without a
//! password through PeerCred. Only the parts that are CUPS 2's alone (PPD defaults, the
//! `CUPS-*` operations, PeerCred) are in modules of their own, so CUPS 3 can be added beside them.

mod client;
mod http;
pub mod lpoptions;
pub mod media;
mod model;
pub mod pkhelper;
pub mod ppd;
mod proto;
pub mod user;

pub use client::{Client, Error, SOCKET, elsewhere, installed, printer_uri};
pub use model::{Choices, Defaults, Driver, Job, JobState, Options, Printer, Snapshot, State};
