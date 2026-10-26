//! DNP3 sessions over `mt-dnp3`: a master that polls an outstation over TCP
//! (integrity and event-class polls, flags, controls, time sync) and an
//! outstation simulator playing a plant; plus what both share: point naming,
//! flags, and reading frames the way a protocol analyser lists them.

pub(crate) mod master;
pub(crate) mod outstation;

use std::fmt::Write as _;

use mt_dnp3::TrafficRecord;
use mt_dnp3::application::Iin;
use mt_dnp3::control::{ControlObject, parse_control_objects};
use mt_dnp3::datalink::ControlByte;
use mt_dnp3::objects::{ObjectBlock, flag, parse_object_blocks, parse_points};
use vt_protocol::{Direction, Dnp3Iin, Dnp3PointKind, TrafficFrame};

use crate::emit::now_ms;

/// How engineers write a point: `AI 3`.
pub(crate) fn point_label(kind: Dnp3PointKind, index: u16) -> String {
    format!("{} {index}", kind.short())
}

/// The points table's type column.
pub(crate) fn data_type(kind: Dnp3PointKind) -> &'static str {
    match kind {
        Dnp3PointKind::BinaryInput | Dnp3PointKind::BinaryOutput => "Binary",
        Dnp3PointKind::DoubleBitInput => "Double-bit",
        Dnp3PointKind::Counter | Dnp3PointKind::FrozenCounter => "Counter",
        Dnp3PointKind::AnalogInput | Dnp3PointKind::AnalogOutput => "Analog",
    }
}

/// Whether a value with these flags can be trusted: online, not restarting,
/// communication not lost, and for analog inputs in range with a good
/// reference. Variations without flags are online by definition. Forced
/// values are good values; the flags text says they are forced.
pub(crate) fn flags_good(kind: Dnp3PointKind, flags: Option<u8>) -> bool {
    let Some(flags) = flags else { return true };
    let analog = matches!(kind, Dnp3PointKind::AnalogInput);
    flags & flag::ONLINE != 0
        && flags & (flag::RESTART | flag::COMM_LOST) == 0
        && !(analog && flags & (flag::BIT5 | flag::BIT6) != 0)
}

/// The set flags of a point by name, e.g. `ONLINE, LOCAL_FORCED`. The state
/// bit of binary points is the value, not a flag.
pub(crate) fn flags_text(kind: Dnp3PointKind, flags: Option<u8>) -> String {
    let Some(flags) = flags else {
        return "none sent (online)".into();
    };
    let (bit5, bit6) = match kind {
        Dnp3PointKind::AnalogInput | Dnp3PointKind::AnalogOutput => {
            ("OVER_RANGE", Some("REFERENCE_ERR"))
        }
        Dnp3PointKind::Counter | Dnp3PointKind::FrozenCounter => {
            ("ROLLOVER", Some("DISCONTINUITY"))
        }
        Dnp3PointKind::BinaryInput | Dnp3PointKind::DoubleBitInput => ("CHATTER_FILTER", None),
        Dnp3PointKind::BinaryOutput => ("RESERVED_5", None),
    };
    let names: Vec<&str> = [
        (flag::ONLINE, Some("ONLINE")),
        (flag::RESTART, Some("RESTART")),
        (flag::COMM_LOST, Some("COMM_LOST")),
        (flag::REMOTE_FORCED, Some("REMOTE_FORCED")),
        (flag::LOCAL_FORCED, Some("LOCAL_FORCED")),
        (flag::BIT5, Some(bit5)),
        (flag::BIT6, bit6),
    ]
    .into_iter()
    .filter_map(|(bit, name)| (flags & bit != 0).then_some(name).flatten())
    .collect();
    if names.is_empty() {
        "OFFLINE (no flag set)".into()
    } else if flags & flag::ONLINE == 0 {
        format!("OFFLINE, {}", names.join(", "))
    } else {
        names.join(", ")
    }
}

/// A time in milliseconds since the Unix epoch as UTC, e.g.
/// `2026-10-26 14:02:11.234 UTC`.
pub(crate) fn utc(ms: u64) -> String {
    let days = i64::try_from(ms / 86_400_000).unwrap_or(0);
    let in_day = ms % 86_400_000;
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03} UTC",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000
    )
}

pub(crate) fn iin_of(iin: &Iin) -> Dnp3Iin {
    let [iin1, iin2] = iin.to_bytes();
    Dnp3Iin { iin1, iin2 }
}

/// The names of the indications set, for the log.
pub(crate) fn iin_names(iin: Dnp3Iin) -> String {
    let names: Vec<&str> = iin.set().iter().map(|bit| bit.name).collect();
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}

fn function_name(code: u8) -> String {
    match code {
        0x00 => "CONFIRM".into(),
        0x01 => "READ".into(),
        0x02 => "WRITE".into(),
        0x03 => "SELECT".into(),
        0x04 => "OPERATE".into(),
        0x05 => "DIRECT OPERATE".into(),
        0x06 => "DIRECT OPERATE NR".into(),
        0x07 => "FREEZE".into(),
        0x08 => "FREEZE NR".into(),
        0x09 => "FREEZE CLEAR".into(),
        0x0A => "FREEZE CLEAR NR".into(),
        0x0D => "COLD RESTART".into(),
        0x0E => "WARM RESTART".into(),
        0x14 => "ENABLE UNSOLICITED".into(),
        0x15 => "DISABLE UNSOLICITED".into(),
        0x17 => "DELAY MEASURE".into(),
        0x81 => "RESPONSE".into(),
        0x82 => "UNSOLICITED".into(),
        code => format!("FUNCTION 0x{code:02X}"),
    }
}

fn link_name(link: ControlByte) -> &'static str {
    match (link.prm, link.function) {
        (true, ControlByte::RESET_LINK_STATES) => "RESET LINK STATES",
        (true, ControlByte::TEST_LINK_STATES) => "TEST LINK STATES",
        (true, ControlByte::REQUEST_LINK_STATUS) => "REQUEST LINK STATUS",
        (true, _) => "LINK DATA",
        (false, ControlByte::ACK) => "ACK",
        (false, ControlByte::NACK) => "NACK",
        (false, ControlByte::LINK_STATUS) => "LINK STATUS",
        (false, _) => "LINK NOT SUPPORTED",
    }
}

/// An object header as an analyser shows it: `g30v5 [0–9]`, `g60v2 all`,
/// `g32v7 ×3`.
fn block_text(block: &ObjectBlock<'_>) -> String {
    let range = match block.qualifier & 0x0F {
        0x06 => "all".to_owned(),
        0x00..=0x02 => {
            let stop = block.start + block.count.saturating_sub(1);
            if block.count == 1 {
                format!("[{}]", block.start)
            } else {
                format!("[{}–{stop}]", block.start)
            }
        }
        _ => format!("×{}", block.count),
    };
    format!("g{}v{} {range}", block.group, block.variation)
}

/// A READ's object headers in words: `class 1, 2, 3, 0` for class polls.
fn read_text(blocks: &[ObjectBlock<'_>]) -> String {
    let classes: Option<Vec<&str>> = blocks
        .iter()
        .map(|block| match (block.group, block.variation) {
            (60, 1) => Some("0"),
            (60, 2) => Some("1"),
            (60, 3) => Some("2"),
            (60, 4) => Some("3"),
            _ => None,
        })
        .collect();
    match classes {
        Some(classes) if !classes.is_empty() => format!("class {}", classes.join(", ")),
        _ => blocks.iter().map(block_text).collect::<Vec<_>>().join(", "),
    }
}

fn crob_text(code: u8) -> String {
    let operation = match code & 0x0F {
        0x01 => "PULSE ON",
        0x02 => "PULSE OFF",
        0x03 => "LATCH ON",
        0x04 => "LATCH OFF",
        _ => "NUL",
    };
    match code & 0xC0 {
        0x40 => format!("CLOSE {operation}"),
        0x80 => format!("TRIP {operation}"),
        _ => operation.to_owned(),
    }
}

/// A control object in words: `CROB BO 3 LATCH ON`, `AO 1 = 61.5`.
fn control_text(objects: &[u8]) -> Option<(String, String)> {
    let echo = parse_control_objects(objects).ok()??;
    let index = echo.request.index;
    let text = match echo.request.object {
        ControlObject::Crob(crob) => {
            let mut text = format!("BO {index} {}", crob_text(crob.code));
            if crob.code & 0x0F <= 0x02 {
                let _ = write!(
                    text,
                    " ×{} {}/{} ms",
                    crob.count, crob.on_time_ms, crob.off_time_ms
                );
            }
            text
        }
        ControlObject::AnalogOutput(value) => format!("AO {index} = {value}"),
    };
    Some((text, echo.status.to_string()))
}

/// Summary and detail of one frame of DNP3 traffic.
pub(crate) fn describe(record: &TrafficRecord) -> (String, String) {
    let asdu = record.asdu.as_slice();
    if asdu.is_empty() {
        return (link_name(record.link).into(), "link layer".into());
    }
    let control = asdu[0];
    let Some(&function) = asdu.get(1) else {
        return ("FRAGMENT".into(), "truncated".into());
    };
    let mut detail = format!("seq {}", control & 0x0F);
    for (bit, name) in [(0x80, "FIR"), (0x40, "FIN"), (0x20, "CON"), (0x10, "UNS")] {
        if control & bit != 0 {
            let _ = write!(detail, " {name}");
        }
    }
    let name = function_name(function);
    if function >= 0x81 {
        let (Some(&low), Some(&high)) = (asdu.get(2), asdu.get(3)) else {
            return (name, format!("{detail} · truncated"));
        };
        let iin = Dnp3Iin {
            iin1: low,
            iin2: high,
        };
        let objects = &asdu[4..];
        if !iin.set().is_empty() {
            let _ = write!(detail, " · IIN {}", iin_names(iin));
        }
        let parsed = parse_object_blocks(objects, true);
        if !parsed.blocks.is_empty() {
            let blocks: Vec<String> = parsed.blocks.iter().map(block_text).collect();
            let _ = write!(detail, " · {}", blocks.join(", "));
        }
        if let Some(issue) = parsed.issue {
            let _ = write!(detail, " · {issue}");
        }
        if matches!(
            parsed.blocks.first().map(|block| block.group),
            Some(12 | 41)
        ) && let Some((control, status)) = control_text(objects)
        {
            return (format!("{name} {control}"), format!("{status} · {detail}"));
        }
        let (points, _) = parse_points(objects);
        let events = points.iter().filter(|point| point.event).count();
        let summary = match (points.len(), events) {
            (0, _) => name,
            (n, 0) => format!("{name} · {n} point{}", if n == 1 { "" } else { "s" }),
            (n, e) => format!(
                "{name} · {n} point{}, {e} event{}",
                if n == 1 { "" } else { "s" },
                if e == 1 { "" } else { "s" }
            ),
        };
        return (summary, detail);
    }
    let objects = &asdu[2..];
    let summary = match function {
        0x01 => {
            let parsed = parse_object_blocks(objects, false);
            format!("{name} {}", read_text(&parsed.blocks))
        }
        0x02 => {
            let parsed = parse_object_blocks(objects, true);
            match parsed
                .blocks
                .first()
                .map(|block| (block.group, block.variation))
            {
                Some((50, 1)) => format!("{name} time"),
                Some((80, 1)) => format!("{name} IIN (clear restart)"),
                _ => format!(
                    "{name} {}",
                    parsed
                        .blocks
                        .iter()
                        .map(block_text)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
        0x03..=0x06 => match control_text(objects) {
            Some((control, _)) => format!("{name} {control}"),
            None => name,
        },
        _ => name,
    };
    if function != 0x00 && function != 0x01 {
        let parsed = parse_object_blocks(objects, true);
        if !parsed.blocks.is_empty() {
            let blocks: Vec<String> = parsed.blocks.iter().map(block_text).collect();
            let _ = write!(detail, " · {}", blocks.join(", "));
        }
    }
    (summary, detail)
}

pub(crate) fn traffic(record: &TrafficRecord) -> TrafficFrame {
    let (summary, detail) = describe(record);
    TrafficFrame {
        at_ms: now_ms(),
        direction: if record.sent {
            Direction::Tx
        } else {
            Direction::Rx
        },
        summary,
        detail,
        bytes: u32::try_from(record.octets).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_dnp3::application::FunctionCode;
    use mt_dnp3::application::{
        build_integrity_poll_request, build_response, build_time_sync_request_at,
    };
    use mt_dnp3::control::{
        ControlRequest, ControlStatus, Crob, build_control_request, build_control_response,
    };

    fn record(sent: bool, asdu: Vec<u8>) -> TrafficRecord {
        TrafficRecord {
            sent,
            link: ControlByte::master_data(),
            asdu,
            octets: 0,
        }
    }

    #[test]
    fn requests_and_responses_read_like_an_analyser() {
        assert_eq!(
            describe(&record(true, build_integrity_poll_request(3))),
            ("READ class 1, 2, 3, 0".into(), "seq 3 FIR FIN".into())
        );
        let time = describe(&record(
            true,
            build_time_sync_request_at(4, 1_700_000_000_000),
        ));
        assert_eq!(time.0, "WRITE time");

        let crob = ControlRequest {
            index: 3,
            object: ControlObject::Crob(Crob {
                code: 0x81,
                count: 1,
                on_time_ms: 1000,
                off_time_ms: 0,
            }),
        };
        let select = build_control_request(FunctionCode::Select, 5, &crob).unwrap();
        assert_eq!(
            describe(&record(true, select)).0,
            "SELECT BO 3 TRIP PULSE ON ×1 1000/0 ms"
        );
        let echo = build_control_response(5, &Iin::default(), &crob, ControlStatus::NoSelect);
        let (summary, detail) = describe(&record(false, echo));
        assert_eq!(summary, "RESPONSE BO 3 TRIP PULSE ON ×1 1000/0 ms");
        assert!(detail.starts_with("NO_SELECT"), "{detail}");

        // A response with IIN, static data and an event.
        let mut objects = vec![30, 5, 0x00, 0, 1, 0x01, 0, 0, 0, 0, 0x01, 0, 0, 0x80, 0x3F];
        objects.extend_from_slice(&[2, 1, 0x17, 1, 4, 0x81]);
        let response = build_response(6, &Iin::from_bytes(0x12, 0x00), &objects);
        let (summary, detail) = describe(&record(false, response));
        assert_eq!(summary, "RESPONSE · 3 points, 1 event");
        assert_eq!(
            detail,
            "seq 6 FIR FIN · IIN CLASS 1 EVENTS, NEED TIME · g30v5 [0–1], g2v1 ×1"
        );
    }

    #[test]
    fn link_frames_and_broken_fragments_never_panic() {
        let reset = TrafficRecord {
            sent: true,
            link: ControlByte::master_link(ControlByte::RESET_LINK_STATES),
            asdu: Vec::new(),
            octets: 10,
        };
        assert_eq!(describe(&reset).0, "RESET LINK STATES");
        let ack = TrafficRecord {
            link: ControlByte::outstation_link(ControlByte::ACK),
            sent: false,
            ..reset
        };
        assert_eq!(describe(&ack).0, "ACK");
        assert_eq!(describe(&record(false, vec![0xC0])).0, "FRAGMENT");
        assert_eq!(
            describe(&record(false, vec![0xC0, 0x81, 0x00])).1,
            "seq 0 FIR FIN · truncated"
        );
        // Object data cut short: reported, not decoded past.
        let (_, detail) = describe(&record(false, vec![0xC0, 0x81, 0, 0, 30, 5, 0x00, 0, 9, 1]));
        assert!(detail.contains("cut off"), "{detail}");
    }

    #[test]
    fn flags_decide_quality() {
        let ai = Dnp3PointKind::AnalogInput;
        assert!(flags_good(ai, None), "variations without flags are online");
        assert!(flags_good(ai, Some(flag::ONLINE)));
        assert!(flags_good(ai, Some(flag::ONLINE | flag::LOCAL_FORCED)));
        assert!(!flags_good(ai, Some(0)), "offline");
        assert!(!flags_good(ai, Some(flag::ONLINE | flag::COMM_LOST)));
        assert!(!flags_good(ai, Some(flag::ONLINE | flag::RESTART)));
        assert!(
            !flags_good(ai, Some(flag::ONLINE | flag::BIT5)),
            "over range"
        );
        // Bit 5 is CHATTER_FILTER on binary inputs: the state is still right.
        assert!(flags_good(
            Dnp3PointKind::BinaryInput,
            Some(flag::ONLINE | flag::BIT5)
        ));
        assert_eq!(
            flags_text(ai, Some(flag::ONLINE | flag::BIT5)),
            "ONLINE, OVER_RANGE"
        );
        assert_eq!(flags_text(ai, Some(flag::COMM_LOST)), "OFFLINE, COMM_LOST");
        assert_eq!(
            flags_text(Dnp3PointKind::BinaryInput, Some(flag::ONLINE | flag::STATE)),
            "ONLINE"
        );
        assert_eq!(flags_text(ai, Some(0)), "OFFLINE (no flag set)");
    }

    #[test]
    fn utc_times_follow_the_calendar() {
        assert_eq!(utc(0), "1970-01-01 00:00:00.000 UTC");
        assert_eq!(utc(951_782_400_000), "2000-02-29 00:00:00.000 UTC");
        assert_eq!(utc(1_793_017_331_234), "2026-10-26 12:22:11.234 UTC");
    }
}
