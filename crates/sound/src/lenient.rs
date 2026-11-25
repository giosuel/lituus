//! The output and input lists, read as the `pulseaudio` crate reads them except for each one's
//! state. pipewire-pulse gives a device that's still being created state -2 and one that failed
//! to start (PipeWire's error state) -3, where the crate knows only 0 to 2: one such device made
//! the whole list unreadable, and the Sound page waited for PipeWire until it recovered. Lituus
//! doesn't use the state, so any value is read and left at its default.

use pulseaudio::protocol::port_info::{PortAvailable, PortDirection, PortInfo, PortType};
use pulseaudio::protocol::{
    CommandReply, ProtocolError, SinkFlags, SinkInfo, SourceFlags, SourceInfo, TagStructRead, TagStructReader,
    TagStructWrite, TagStructWriter,
};

/// The reply to `GetSinkInfoList`.
pub struct Sinks(pub Vec<SinkInfo>);

/// The reply to `GetSourceInfoList`.
pub struct Sources(pub Vec<SourceInfo>);

impl TagStructRead for Sinks {
    fn read(ts: &mut TagStructReader<'_>, version: u16) -> Result<Self, ProtocolError> {
        let mut sinks = Vec::new();
        while ts.has_data_left()? {
            sinks.push(sink(ts, version)?);
        }
        Ok(Sinks(sinks))
    }
}

impl TagStructRead for Sources {
    fn read(ts: &mut TagStructReader<'_>, version: u16) -> Result<Self, ProtocolError> {
        let mut sources = Vec::new();
        while ts.has_data_left()? {
            sources.push(source(ts, version)?);
        }
        Ok(Sources(sources))
    }
}

// A reply is read, never written; the crate asks for both.
impl TagStructWrite for Sinks {
    fn write(&self, _: &mut TagStructWriter<'_>, _: u16) -> Result<(), ProtocolError> {
        Err(ProtocolError::Invalid("only read".into()))
    }
}

impl TagStructWrite for Sources {
    fn write(&self, _: &mut TagStructWriter<'_>, _: u16) -> Result<(), ProtocolError> {
        Err(ProtocolError::Invalid("only read".into()))
    }
}

impl CommandReply for Sinks {}
impl CommandReply for Sources {}

/// As `SinkInfo::read` in pulseaudio 0.3.1.
fn sink(ts: &mut TagStructReader<'_>, version: u16) -> Result<SinkInfo, ProtocolError> {
    let mut sink = SinkInfo {
        index: ts.read_index()?.ok_or_else(|| ProtocolError::Invalid("invalid sink index".into()))?,
        name: ts.read_string_non_null()?,
        description: ts.read_string()?,
        sample_spec: ts.read()?,
        channel_map: ts.read()?,
        owner_module_index: ts.read_index()?,
        cvolume: ts.read()?,
        muted: ts.read_bool()?,
        monitor_source_index: ts.read_index()?,
        monitor_source_name: ts.read_string()?,
        actual_latency: ts.read_usec()?,
        driver: ts.read_string()?,
        flags: SinkFlags::from_bits_truncate(ts.read_u32()?),
        props: ts.read()?,
        configured_latency: ts.read_usec()?,
        ..Default::default()
    };
    if version >= 15 {
        sink.base_volume = ts.read()?;
        any_state(ts)?;
        sink.volume_steps = Some(ts.read_u32()?).filter(|&n| n != 0);
        sink.card_index = ts.read_index()?;
    }
    if version >= 16 {
        (sink.ports, sink.active_port) = ports(ts, version, PortDirection::Input)?;
    }
    if version >= 21 {
        for _ in 0..ts.read_u8()? {
            sink.formats.push(ts.read()?);
        }
    }
    Ok(sink)
}

/// As `SourceInfo::read` in pulseaudio 0.3.1.
fn source(ts: &mut TagStructReader<'_>, version: u16) -> Result<SourceInfo, ProtocolError> {
    let mut source = SourceInfo {
        index: ts.read_index()?.ok_or_else(|| ProtocolError::Invalid("invalid source index".into()))?,
        name: ts.read_string_non_null()?,
        description: ts.read_string()?,
        sample_spec: ts.read()?,
        channel_map: ts.read()?,
        owner_module_index: ts.read_index()?,
        cvolume: ts.read()?,
        muted: ts.read_bool()?,
        monitor_of_sink_index: ts.read_index()?,
        monitor_of_sink_name: ts.read_string()?,
        actual_latency: ts.read_usec()?,
        driver: ts.read_string()?,
        flags: SourceFlags::from_bits_truncate(ts.read_u32()?),
        ..Default::default()
    };
    if version >= 13 {
        source.props = ts.read()?;
        source.configured_latency = ts.read_usec()?;
    }
    if version >= 15 {
        source.base_volume = ts.read()?;
        any_state(ts)?;
        source.volume_steps = Some(ts.read_u32()?).filter(|&n| n != 0);
        source.card_index = ts.read_index()?;
    }
    if version >= 16 {
        (source.ports, source.active_port) = ports(ts, version, PortDirection::Output)?;
    }
    if version >= 21 {
        for _ in 0..ts.read_u8()? {
            source.formats.push(ts.read()?);
        }
    }
    Ok(source)
}

/// The state, whatever it is.
fn any_state(ts: &mut TagStructReader<'_>) -> Result<(), ProtocolError> {
    ts.read_u32().map(drop)
}

/// A device's ports and the active one's place among them.
fn ports(
    ts: &mut TagStructReader<'_>,
    version: u16,
    dir: PortDirection,
) -> Result<(Vec<PortInfo>, usize), ProtocolError> {
    let mut ports = Vec::new();
    for _ in 0..ts.read_u32()? {
        let name = ts.read_string()?.unwrap_or_default();
        let description = ts.read_string()?;
        let priority = ts.read_u32()?;
        let available = if version >= 24 { ts.read_enum()? } else { PortAvailable::Unknown };
        let (availability_group, port_type) =
            if version >= 34 { (ts.read_string()?, ts.read_enum()?) } else { (None, PortType::Unknown) };
        ports.push(PortInfo { name, description, dir, priority, available, availability_group, port_type });
    }
    let active = match ts.read_string()? {
        Some(port) => ports.iter().position(|p| port.as_bytes() == p.name.as_bytes()).unwrap_or(0),
        None => 0,
    };
    Ok((ports, active))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulseaudio::protocol::{SinkState, SourceState, read_reply_message, write_reply_message};

    fn source_named(name: &str, index: u32) -> SourceInfo {
        SourceInfo {
            index,
            name: std::ffi::CString::new(name).unwrap(),
            description: Some(std::ffi::CString::new(name.to_uppercase()).unwrap()),
            state: SourceState::Idle,
            ..Default::default()
        }
    }

    /// A reply as the server sends it, with the first device's state replaced by `state`.
    fn reply_with_state<T: CommandReply>(list: &T, version: u16, written: u32, state: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        write_reply_message(&mut buf, 7, list, version).unwrap();
        // The state is a u32 tag ('L', then 4 bytes big-endian); the first one holding `written`
        // after the base volume is the first device's.
        let tag = [b'L', 0, 0, 0, written as u8];
        let at = buf.windows(5).position(|w| w == tag).expect("the state");
        buf[at + 1..at + 5].copy_from_slice(&state.to_be_bytes());
        buf
    }

    /// The crate reads the same lists the same way when the states are ones it knows.
    #[test]
    fn known_states_read_as_the_crate_reads_them() {
        let version = pulseaudio::protocol::MAX_VERSION;
        let sources = vec![source_named("mic", 3), source_named("speakers.monitor", 4)];
        let mut buf = Vec::new();
        write_reply_message(&mut buf, 7, &sources, version).unwrap();
        let (_, ours): (u32, Sources) = read_reply_message(&mut buf.as_slice(), version).unwrap();
        let (_, theirs): (u32, Vec<SourceInfo>) = read_reply_message(&mut buf.as_slice(), version).unwrap();
        let names =
            |l: &[SourceInfo]| l.iter().map(|s| (s.index, s.name.clone(), s.description.clone())).collect::<Vec<_>>();
        assert_eq!(names(&ours.0), names(&theirs));

        let sinks =
            vec![SinkInfo { index: 9, name: c"speakers".into(), state: SinkState::Running, ..Default::default() }];
        let mut buf = Vec::new();
        write_reply_message(&mut buf, 7, &sinks, version).unwrap();
        let (_, ours): (u32, Sinks) = read_reply_message(&mut buf.as_slice(), version).unwrap();
        assert_eq!((ours.0[0].index, ours.0[0].name.as_c_str()), (9, c"speakers"));
    }

    /// A device that failed (-3) or is still being created (-2) leaves the list readable.
    #[test]
    fn a_failed_device_leaves_the_list_readable() {
        let version = pulseaudio::protocol::MAX_VERSION;
        let sources = vec![source_named("mic", 3), source_named("webcam", 5)];
        for state in [-3i32, -2] {
            let buf = reply_with_state(&sources, version, SourceState::Idle as u32, state as u32);
            assert!(
                read_reply_message::<Vec<SourceInfo>>(&mut buf.as_slice(), version).is_err(),
                "the crate still can't read it: if it can, this module can go"
            );
            let (_, ours): (u32, Sources) = read_reply_message(&mut buf.as_slice(), version).unwrap();
            let names: Vec<&std::ffi::CStr> = ours.0.iter().map(|s| s.name.as_c_str()).collect();
            assert_eq!(names, [c"mic", c"webcam"]);
        }
        let sinks = vec![SinkInfo { index: 9, name: c"speakers".into(), state: SinkState::Idle, ..Default::default() }];
        let buf = reply_with_state(&sinks, version, SinkState::Idle as u32, -3i32 as u32);
        let (_, ours): (u32, Sinks) = read_reply_message(&mut buf.as_slice(), version).unwrap();
        assert_eq!(ours.0[0].name.as_c_str(), c"speakers");
    }
}
