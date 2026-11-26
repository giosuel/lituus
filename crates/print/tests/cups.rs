//! lituus-print against a real cupsd with virtual printers: scripts/cups-test.sh here, CI's cups
//! job there. Skipped unless LITUUS_CUPS_SOCKET names cupsd's socket. LITUUS_CUPS_ADMIN=0 when
//! the user isn't one of CUPS's admins: then changes must be refused.
//!
//! The printers (scripts/cups-test.sh's reset_printers): Virtual (driverless, the system's
//! default), Held (stopped, so jobs wait), PDF (cups-pdf), Spare (to remove).
//!
//! LITUUS_CUPS_EMPTY=1: cupsd has no printers at all (scripts/cups-test.sh's no_printers), and
//! only the tests that say so run.

use std::time::Duration;

use lituus_print::{Client, Defaults, Driver, Error, JobState, NewPrinter, State, ppd};

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

fn empty() -> bool {
    std::env::var("LITUUS_CUPS_EMPTY").is_ok_and(|v| v == "1")
}

#[tokio::test]
async fn no_printers_is_an_empty_list_not_an_error() {
    let Some(c) = client() else { return };
    if !empty() {
        eprintln!("LITUUS_CUPS_EMPTY isn't 1: skipped");
        return;
    }
    // cupsd answers "No destinations added." (not found) here, not an empty list.
    let s = c.snapshot().await.unwrap();
    assert!(s.printers.is_empty(), "{:?}", s.printers);
    assert_eq!((s.default, s.jobs.len()), (None, 0));
    assert!(s.server.starts_with("CUPS "), "{}", s.server);
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
async fn cupsd_says_who_may_change_printers() {
    let Some(c) = client() else { return };
    assert_eq!(c.can_admin().await, admin());
}

#[tokio::test]
async fn an_unknown_printer_is_an_ipp_error() {
    let Some(c) = client() else { return };
    let got = c.options("Nothing-here").await;
    assert!(matches!(got, Err(Error::Ipp { .. })), "{got:?}");
}

#[tokio::test]
async fn new_defaults_reach_both_ipp_and_the_ppd() {
    let Some(c) = client() else { return };
    let d = Defaults {
        media: Some("iso_a4_210x297mm".into()),
        sides: Some("two-sided-long-edge".into()),
        color: Some("monochrome".into()),
        quality: Some("high".into()),
    };
    let got = c.set_options("Virtual", &d).await;
    if !admin() {
        assert!(matches!(got, Err(Error::NotAllowed(_))), "{got:?}");
        return;
    }
    got.unwrap();
    let o = c.options("Virtual").await.unwrap();
    assert_eq!(
        (o.media.default.as_str(), o.sides.default.as_str(), o.color.default.as_str(), o.quality.default.as_str()),
        ("iso_a4_210x297mm", "two-sided-long-edge", "monochrome", "high")
    );
    let file = c.ppd("Virtual").await.unwrap().expect("a driverless queue has a PPD");
    for (key, want) in [
        ("PageSize", "A4"),
        ("PageRegion", "A4"),
        ("Duplex", "DuplexNoTumble"),
        ("ColorModel", "Gray"),
        ("cupsPrintQuality", "High"),
    ] {
        assert_eq!(ppd::default(&file, key).as_deref(), Some(want), "{key}");
    }
    // Only what's given changes.
    c.set_options("Virtual", &Defaults { sides: Some("one-sided".into()), ..Defaults::default() }).await.unwrap();
    let o = c.options("Virtual").await.unwrap();
    assert_eq!((o.media.default.as_str(), o.sides.default.as_str()), ("iso_a4_210x297mm", "one-sided"));
}

#[tokio::test]
async fn a_raw_queue_has_no_ppd() {
    let Some(c) = client() else { return };
    assert!(c.ppd("Nothing-here").await.unwrap().is_none());
}

#[tokio::test]
async fn a_driverless_printer_is_added_with_the_local_paper_and_not_shared() {
    let Some(c) = client() else { return };
    let new = NewPrinter {
        name: "Added".into(),
        uri: "ipp://localhost:8631/ipp/print".into(),
        ppd: "everywhere".into(),
        info: "Added printer".into(),
        location: String::new(),
    };
    let got = c.add(&new, "iso_a4_210x297mm").await;
    if !admin() {
        assert!(matches!(got, Err(Error::NotAllowed(_))), "{got:?}");
        return;
    }
    got.unwrap();
    let p = c.printers().await.unwrap().into_iter().find(|p| p.name == "Added").expect("added");
    assert_eq!((p.title(), &p.driver, p.shared), ("Added printer", &Driver::Driverless, false));
    assert_eq!(c.options("Added").await.unwrap().media.default, "iso_a4_210x297mm");
    c.remove("Added").await.unwrap();
}

#[tokio::test]
async fn an_address_that_doesnt_answer_is_told_within_seconds() {
    let started = std::time::Instant::now();
    assert!(!lituus_print::answers("10.255.255.1", 631).await);
    assert!(started.elapsed() < Duration::from_secs(7));
}
