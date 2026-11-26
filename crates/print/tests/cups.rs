//! lituus-print against a real cupsd with virtual printers: scripts/cups-test.sh here, CI's cups
//! job there. Skipped unless LITUUS_CUPS_SOCKET names cupsd's socket. LITUUS_CUPS_ADMIN=0 when
//! the user isn't one of CUPS's admins: then changes must be refused.
//!
//! The printers (scripts/cups-test.sh's reset_printers): Virtual (driverless, the system's
//! default), Held (stopped, so jobs wait), PDF (cups-pdf), Spare (to remove).

use std::time::Duration;

use lituus_print::{Client, Driver, Error, JobState, State};

fn client() -> Option<Client> {
    let Ok(socket) = std::env::var("LITUUS_CUPS_SOCKET") else {
        eprintln!("LITUUS_CUPS_SOCKET isn't set: skipped");
        return None;
    };
    Some(Client::new(socket, lituus_print::user::name()))
}

fn admin() -> bool {
    std::env::var("LITUUS_CUPS_ADMIN").map(|v| v != "0").unwrap_or(true)
}

#[tokio::test]
async fn the_printers_their_options_and_the_default_are_read() {
    let Some(c) = client() else { return };
    let snap = c.snapshot().await.unwrap();
    assert!(snap.server.starts_with("CUPS 2"), "{}", snap.server);
    let names: Vec<_> = snap.printers.iter().map(|p| p.name.as_str()).collect();
    for n in ["Virtual", "Held", "PDF"] {
        assert!(names.contains(&n), "{names:?}");
    }
    let virt = snap.printers.iter().find(|p| p.name == "Virtual").unwrap();
    assert_eq!((virt.title(), virt.location.as_str(), &virt.driver), ("Virtual Duplex", "Lab", &Driver::Driverless));
    assert!(virt.networked() && virt.accepting);
    let held = snap.printers.iter().find(|p| p.name == "Held").unwrap();
    assert_eq!(held.state, State::Stopped);
    let pdf = snap.printers.iter().find(|p| p.name == "PDF").unwrap();
    assert_eq!(pdf.driver, Driver::Pdf);
    // The server's default, unless this user has their own in ~/.cups/lpoptions.
    if lituus_print::lpoptions::read_default().is_none() {
        assert_eq!(snap.default.as_deref(), Some("Virtual"));
    }
    assert_eq!(c.server_default().await.unwrap().as_deref(), Some("Virtual"));

    let o = c.options("Virtual").await.unwrap();
    assert!(o.media.supported.iter().any(|m| m.starts_with("iso_a4")), "{:?}", o.media);
    assert!(o.media.supported.contains(&o.media.default), "{:?}", o.media);
    assert!(o.sides.supported.iter().any(|s| s == "two-sided-long-edge"), "{:?}", o.sides);
    assert!(o.quality.supported.iter().any(|q| q == "normal"), "{:?}", o.quality);
    assert!(!o.color.supported.is_empty(), "{:?}", o.color);
}

#[tokio::test]
async fn a_test_page_waits_on_a_stopped_printer_and_can_be_cancelled() {
    let Some(c) = client() else { return };
    let id = c.print_test_page("Held").await.unwrap();
    assert!(id > 0);
    let jobs = c.jobs().await.unwrap();
    let job = jobs.iter().find(|j| j.id == id).unwrap_or_else(|| panic!("{jobs:?}"));
    assert_eq!((job.printer.as_str(), job.name.as_str(), job.mine), ("Held", "Test page", true));
    assert!(matches!(job.state, JobState::Pending | JobState::Held), "{job:?}");
    assert!(job.created > 1_700_000_000, "{job:?}");
    let held = c.printers().await.unwrap().into_iter().find(|p| p.name == "Held").unwrap();
    assert!(held.queued >= 1, "{held:?}");

    // Anyone may cancel their own.
    c.cancel(id).await.unwrap();
    let mut gone = false;
    for _ in 0..20 {
        if !c.jobs().await.unwrap().iter().any(|j| j.id == id) {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(gone);
}

#[tokio::test]
async fn removing_a_printer_needs_an_admin() {
    let Some(c) = client() else { return };
    let got = c.remove("Spare").await;
    let still = c.printers().await.unwrap().iter().any(|p| p.name == "Spare");
    if admin() {
        got.unwrap();
        assert!(!still);
    } else {
        assert!(matches!(got, Err(Error::NotAllowed(_))), "{got:?}");
        assert!(still);
    }
}

#[tokio::test]
async fn an_unknown_printer_is_an_ipp_error() {
    let Some(c) = client() else { return };
    let got = c.options("Nothing-here").await;
    assert!(matches!(got, Err(Error::Ipp { .. })), "{got:?}");
}
