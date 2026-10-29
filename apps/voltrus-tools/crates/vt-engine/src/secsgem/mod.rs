//! SECS/GEM sessions over `mt-secsgem`: a host that reads a piece of
//! equipment's status variables and equipment constants (and writes the
//! constants), and a simulator that plays a wafer processing chamber.
//!
//! Status variables and equipment constants are points. S6F11 event reports
//! and S5F1 alarm reports go to the log, and every HSMS message in both
//! directions goes to the traffic view as SML.
//!
//! `mt-secsgem` has only been tested against its own simulator and one
//! third-party library, not real equipment; the app labels the protocol Beta.
//! SECS/GEM and HSMS are SEMI standards; this is an independent
//! implementation.

pub(crate) mod client;
pub(crate) mod simulator;

use mt_secsgem::{Direction as GemDirection, Eac, Item, LogEntry};
use vt_protocol::{Direction, PointInfo, Quality, Reading, TrafficFrame};

/// A point with what the equipment last reported for it.
pub(crate) struct Value {
    pub(crate) reading: Reading,
    /// What the point shows when it has no number: text, lists, arrays.
    pub(crate) text: String,
}

fn good(value: f64) -> Reading {
    Reading {
        value,
        raw: [0; 4],
        raw_len: 0,
        quality: Quality::Good,
    }
}

/// The number or text an item stands for. An empty list is how the
/// equipment answers an id it does not know, so it reads as bad.
pub(crate) fn value_of(item: &Item) -> Value {
    if item.len() == 1 {
        if let Some(number) = item.as_f64() {
            return Value {
                reading: good(number),
                text: String::new(),
            };
        }
        if let Some(flag) = item.as_bool() {
            return Value {
                reading: good(f64::from(u8::from(flag))),
                text: String::new(),
            };
        }
    }
    match item {
        Item::List(items) if items.is_empty() => Value {
            reading: Reading::bad(),
            text: String::new(),
        },
        Item::Ascii(text) => Value {
            reading: good(f64::NAN),
            text: text.clone(),
        },
        other => Value {
            reading: good(f64::NAN),
            text: other.to_sml(),
        },
    }
}

/// The data type column of a point holding `item`.
pub(crate) fn type_of(item: &Item) -> &'static str {
    match item {
        Item::Ascii(_) => "Text",
        Item::Boolean(flags) if flags.len() == 1 => "Bool",
        other => other.type_name(),
    }
}

pub(crate) fn is_boolean(item: &Item) -> bool {
    matches!(item, Item::Boolean(flags) if flags.len() == 1)
}

pub(crate) fn point(
    id: usize,
    name: impl Into<String>,
    address: impl Into<String>,
    data_type: &str,
    unit: &str,
    writable: bool,
    boolean: bool,
) -> PointInfo {
    PointInfo {
        id: u32::try_from(id).unwrap_or(u32::MAX),
        name: name.into(),
        address: address.into(),
        data_type: data_type.into(),
        unit: unit.into(),
        writable,
        boolean,
        states: Vec::new(),
    }
}

/// What the equipment said about a refused constant write.
pub(crate) fn eac_error(eac: Eac, id: u64, range: Option<(f64, f64)>) -> String {
    match eac {
        Eac::Accepted => String::new(),
        Eac::ConstantDoesNotExist => format!("The equipment has no equipment constant {id}"),
        Eac::Busy => format!(
            "The equipment cannot change constant {id} now (some are locked while a process runs)"
        ),
        Eac::OutOfRange => match range {
            Some((min, max)) => format!("Constant {id} takes values from {min} to {max}"),
            None => format!("The equipment refused the value for constant {id} as out of range"),
        },
        Eac::Other(code) => format!("The equipment refused constant {id} (EAC {code})"),
    }
}

/// What the traffic view calls a message: its stream and function, and what
/// it does in plain words.
fn describe(kind: &str) -> String {
    let name = match kind {
        "S1F1" => "Are you there",
        "S1F2" => "On-line data",
        "S1F3" => "Status variables request",
        "S1F4" => "Status variables data",
        "S1F11" => "Status variable names request",
        "S1F12" => "Status variable names",
        "S1F13" => "Establish communications request",
        "S1F14" => "Establish communications answer",
        "S1F15" => "Request offline",
        "S1F16" => "Offline answer",
        "S1F17" => "Request online",
        "S1F18" => "Online answer",
        "S2F13" => "Equipment constants request",
        "S2F14" => "Equipment constants data",
        "S2F15" => "Set equipment constants",
        "S2F16" => "Set equipment constants answer",
        "S2F29" => "Equipment constant names request",
        "S2F30" => "Equipment constant names",
        "S2F33" => "Define report",
        "S2F34" => "Define report answer",
        "S2F35" => "Link event report",
        "S2F36" => "Link event report answer",
        "S2F37" => "Enable or disable events",
        "S2F38" => "Enable or disable events answer",
        "S2F41" => "Host command",
        "S2F42" => "Host command answer",
        "S5F1" => "Alarm report",
        "S5F2" => "Alarm report answer",
        "S5F3" => "Enable or disable alarms",
        "S5F4" => "Enable or disable alarms answer",
        "S5F5" => "List alarms request",
        "S5F6" => "List alarms data",
        "S6F11" => "Event report",
        "S6F12" => "Event report answer",
        "S7F19" => "Process programs request",
        "S7F20" => "Process programs list",
        "S10F1" => "Terminal request",
        "S10F2" => "Terminal request answer",
        "S10F3" => "Terminal display",
        "S10F4" => "Terminal display answer",
        _ => return kind.to_owned(),
    };
    format!("{kind} {name}")
}

/// One line of the traffic view for a message `mt-secsgem` logged: the SML
/// text of a data message, or the status of an HSMS control message.
pub(crate) fn frame(entry: &LogEntry) -> TrafficFrame {
    let detail = if entry.text.is_empty() {
        entry.kind.clone()
    } else {
        entry.text.clone()
    };
    TrafficFrame {
        at_ms: entry.time_ms,
        direction: match entry.direction {
            GemDirection::Tx => Direction::Tx,
            GemDirection::Rx => Direction::Rx,
        },
        summary: describe(&entry.kind),
        bytes: u32::try_from(detail.len()).unwrap_or(u32::MAX),
        detail,
    }
}

/// `text` cut to `limit` characters, for a log line.
pub(crate) fn clip(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_become_numbers_or_text_and_unknown_ids_are_bad() {
        let number = value_of(&Item::f4(1.5));
        assert_eq!((number.reading.value, number.text.as_str()), (1.5, ""));
        assert_eq!(value_of(&Item::u1(4)).reading.value, 4.0);
        assert_eq!(value_of(&Item::Boolean(vec![true])).reading.value, 1.0);
        let text = value_of(&Item::ascii("ETCH_SIO2"));
        assert!(text.reading.value.is_nan() && text.reading.quality == Quality::Good);
        assert_eq!(text.text, "ETCH_SIO2");
        let array = value_of(&Item::U4(vec![1, 3]));
        assert!(array.reading.value.is_nan());
        assert!(array.text.starts_with("<U4"), "{}", array.text);
        assert_eq!(value_of(&Item::empty_list()).reading.quality, Quality::Bad);
    }

    #[test]
    fn traffic_names_the_message_and_carries_its_sml() {
        let entry = LogEntry {
            time_ms: 5,
            direction: GemDirection::Tx,
            session_id: 0,
            system: 1,
            kind: "S1F13".into(),
            text: "S1F13 W <L [0]>.".into(),
        };
        let frame = frame(&entry);
        assert_eq!(frame.summary, "S1F13 Establish communications request");
        assert_eq!(frame.detail, "S1F13 W <L [0]>.");
        assert_eq!(frame.direction, Direction::Tx);
        let control = LogEntry {
            kind: "Select.req".into(),
            text: String::new(),
            ..entry
        };
        let control = super::frame(&control);
        assert_eq!(
            (control.summary.as_str(), control.detail.as_str()),
            ("Select.req", "Select.req")
        );
    }

    #[test]
    fn long_text_is_clipped_for_the_log() {
        assert_eq!(clip("abcdef", 3), "abc...");
        assert_eq!(clip("abc", 3), "abc");
    }
}
