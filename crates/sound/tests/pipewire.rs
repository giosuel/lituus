//! Against a real PipeWire: set LITUUS_SOUND_TEST=1 with PULSE_SERVER (or XDG_RUNTIME_DIR)
//! pointing at a pipewire-pulse that has at least one output, such as the null sinks made by
//! `pw-cli create-node adapter '{ factory.name=support.null-audio-sink media.class=Audio/Sink … }'`.
//! Skipped otherwise.

use lituus_sound::Session;

#[test]
fn reads_and_changes_a_real_server() {
    if std::env::var_os("LITUUS_SOUND_TEST").is_none() {
        return;
    }
    let mut session = Session::connect().expect("pipewire-pulse");
    let snap = session.snapshot().unwrap();
    assert!(snap.server.contains("PipeWire") || snap.server.contains("pulse"), "{}", snap.server);
    let sink = snap.default_sink().expect("an output").clone();
    let before = sink.volume.clone();
    session.set_sink_volume(sink.index, &before.with_percent(37)).unwrap();
    session.set_sink_mute(sink.index, true).unwrap();
    let after = session.snapshot().unwrap();
    let changed = after.sink(&sink.name).unwrap();
    assert_eq!(changed.volume.percent(), 37);
    assert!(changed.muted);
    session.set_sink_volume(sink.index, &before).unwrap();
    session.set_sink_mute(sink.index, sink.muted).unwrap();
    if let Some(other) = snap.sinks.iter().find(|s| s.name != sink.name) {
        // Ubuntu 24.04's PipeWire 1.0 says "not supported" without a session manager's metadata.
        if let Err(lituus_sound::Error::Protocol(pulseaudio::protocol::ProtocolError::ServerError(e))) =
            session.set_default_sink(&other.name)
        {
            eprintln!("this PipeWire can't change the default output ({e:?}): skipped");
            return;
        }
        // The session manager (WirePlumber) makes the change, a moment later.
        let mut default = None;
        for _ in 0..40 {
            default = session.snapshot().unwrap().default_sink;
            if default.as_deref() == Some(other.name.as_str()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(default.as_deref(), Some(other.name.as_str()));
        session.set_default_sink(&sink.name).unwrap();
    }
}

#[test]
fn a_watcher_hears_changes() {
    if std::env::var_os("LITUUS_SOUND_TEST").is_none() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let watcher = Session::connect().unwrap();
    std::thread::spawn(move || watcher.watch(move || tx.send(()).is_ok()));
    // Give it time to subscribe.
    std::thread::sleep(std::time::Duration::from_millis(200));
    let mut session = Session::connect().unwrap();
    let sink = session.snapshot().unwrap().default_sink().unwrap().clone();
    session.set_sink_mute(sink.index, !sink.muted).unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(3)).expect("an event");
    session.set_sink_mute(sink.index, sink.muted).unwrap();
}

/// An app recording (pw-record, when it's installed) is listed with its name, and its volume,
/// mute and input change.
#[test]
fn reads_and_changes_a_recording() {
    if std::env::var_os("LITUUS_SOUND_TEST").is_none() {
        return;
    }
    let file = std::env::temp_dir().join(format!("lituus-recording-{}.wav", std::process::id()));
    let Ok(mut recorder) = std::process::Command::new("pw-record")
        .args(["-P", "{ application.name = \"Lituus test recorder\" }"])
        .arg(&file)
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        eprintln!("no pw-record: skipped");
        return;
    };
    let mut session = Session::connect().unwrap();
    let find = |session: &mut Session| {
        session.snapshot().unwrap().recordings.into_iter().find(|r| r.app == "Lituus test recorder")
    };
    // Listed as soon as it starts, and linked to an input or a monitor once the session manager
    // gets to it (a moment later on Ubuntu 24.04's PipeWire 1.0).
    let mut found = None;
    for _ in 0..100 {
        let snap = session.snapshot().unwrap();
        found = snap.recordings.iter().find(|r| r.app == "Lituus test recorder").cloned();
        if found.as_ref().is_some_and(|r| snap.sources.iter().chain(&snap.monitors).any(|d| d.index == r.device)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let rec = found.expect("the recording is listed");
        let snap = session.snapshot().unwrap();
        assert!(
            snap.sources.iter().chain(&snap.monitors).any(|d| d.index == rec.device),
            "it records from an input or a monitor, not {}",
            rec.device
        );
        session.set_recording_volume(rec.index, &rec.volume.with_percent(37)).unwrap();
        session.set_recording_mute(rec.index, true).unwrap();
        let after = find(&mut session).unwrap();
        assert_eq!(after.volume.percent(), 37);
        assert!(after.muted);
        // To another input, or an output's monitor; the session manager moves it a moment later.
        let Some(other) = snap.sources.iter().chain(&snap.monitors).find(|d| d.index != rec.device) else { return };
        session.move_recording(rec.index, other.index).unwrap();
        let mut device = rec.device;
        for _ in 0..40 {
            device = find(&mut session).unwrap().device;
            if device == other.index {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(device, other.index);
        // The session manager keeps an app's volume, mute and input for its next recordings
        // (WirePlumber's restore-stream, by media.role), so they go back as they were.
        session.move_recording(rec.index, rec.device).unwrap();
        session.set_recording_volume(rec.index, &rec.volume).unwrap();
        session.set_recording_mute(rec.index, rec.muted).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
    }));
    let _ = recorder.kill();
    let _ = recorder.wait();
    let _ = std::fs::remove_file(&file);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
