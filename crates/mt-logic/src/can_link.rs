//! The link with CAN Explorer (`mt-can`): the electrical view of a CAN bus here, the message view
//! there. Frames decoded from the wire convert to [`mt_can::CanFrame`], get message and signal
//! names from a DBC, export as a candump log that CAN Explorer replays, and can be compared with
//! a candump log CAN Explorer recorded from the same bus.

use std::collections::HashMap;

use mt_can::dbc::Dbc;
use mt_can::frame::{CanFrame, Direction, TimedFrame};
use mt_can::log::{LogRecord, candump_line};
use serde::Serialize;

use crate::capture::Capture;
use crate::decode::can::CanBitFrame;
use crate::decode::{Annotation, DecodeOutput, Level};

/// Annotation rows added after the CAN decoder's Bits, Fields, Frames and Errors.
pub const ROW_SIGNALS: u8 = 4;
pub const ROW_LOG: u8 = 5;

/// The CAN Explorer simulator's DBC, so the CAN scenario here decodes to the same names.
pub const SIM_DBC: &str = mt_can::sim::SIM_DBC;

/// A frame that made it intact through the wire: a known header and no error, so the other side
/// (an adapter, CAN Explorer) saw it too. Frames without an ACK are kept: a listen-only adapter
/// logs those as well.
pub fn to_can_frame(f: &CanBitFrame) -> Option<CanFrame> {
    if !f.header_ok || f.error_frame {
        return None;
    }
    match f.error.as_deref() {
        None => {}
        Some(e) if e.starts_with("No ACK") && f.crc_ok => {}
        Some(_) => return None,
    }
    let fr = if f.rtr { CanFrame::remote(f.id, f.extended, f.dlc) } else { CanFrame::with_ext(f.id, f.extended, &f.data) };
    fr.ok()
}

/// Seconds from the capture start to a sample.
fn secs(cap: &Capture, sample: u64) -> f64 {
    sample as f64 / cap.sample_rate as f64
}

/// The decoded frames as a candump log (`(seconds.micros) iface ID#DATA`), timed from
/// `start_us` (the capture's wall-clock start, or 0 for times relative to the capture).
pub fn to_candump(cap: &Capture, frames: &[CanBitFrame], start_us: u64, iface: &str) -> String {
    let mut out = String::new();
    for f in frames {
        let Some(frame) = to_can_frame(f) else { continue };
        let ts_us = start_us + (secs(cap, f.start) * 1e6).round() as u64;
        out.push_str(&candump_line(&TimedFrame { ts_us, dir: Direction::Rx, frame }, iface));
        out.push('\n');
    }
    out
}

fn fmt_value(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// `EEC1 EngineSpeed=1500 rpm ActualTorque=0 %`, or `None` when the DBC does not describe the frame.
pub fn describe(dbc: &Dbc, f: &CanFrame) -> Option<(String, String)> {
    let (msg, values) = dbc.decode(f)?;
    let signals = values
        .iter()
        .map(|v| {
            let val = v.label.clone().unwrap_or_else(|| fmt_value(v.value));
            if v.unit.is_empty() || v.label.is_some() { format!("{}={val}", v.name) } else { format!("{}={val} {}", v.name, v.unit) }
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some((msg.name.clone(), signals))
}

fn ensure_row(out: &mut DecodeOutput, row: u8, name: &str) {
    while out.rows.len() <= row as usize {
        out.rows.push(String::new());
    }
    out.rows[row as usize] = name.into();
}

/// Adds a Signals row and Message and Signals table columns from a DBC. `frames` must be the
/// frames `out` was built from (the table has one row per frame, in order, glitches left out).
pub fn add_signals(out: &mut DecodeOutput, frames: &[CanBitFrame], dbc: &Dbc) {
    ensure_row(out, ROW_SIGNALS, "Signals");
    let shown: Vec<&CanBitFrame> = frames.iter().filter(|f| f.error.as_deref() != Some("Glitch")).collect();
    let mut named = 0;
    let mut cells = Vec::with_capacity(shown.len());
    for f in &shown {
        let d = to_can_frame(f).and_then(|c| describe(dbc, &c));
        if let Some((msg, sig)) = &d {
            named += 1;
            let text = if sig.is_empty() { msg.clone() } else { format!("{msg} {sig}") };
            out.annotations.push(Annotation::new(ROW_SIGNALS, f.start, f.end.max(f.start + 1), Level::Data, text, msg.clone()));
        }
        cells.push(d.unwrap_or_default());
    }
    if let Some(t) = &mut out.table {
        let at = t.columns.len() - 1; // before Status
        t.columns.insert(at, "Message".into());
        t.columns.insert(at + 1, "Signals".into());
        for (row, (msg, sig)) in t.rows.iter_mut().zip(cells) {
            let at = row.cells.len() - 1;
            row.cells.insert(at, msg);
            row.cells.insert(at + 1, sig);
        }
    }
    out.annotations.sort_by_key(|a| (a.row, a.start));
    out.summary.push_str(&format!(", {named} named by the DBC"));
}

/// What the comparison with a CAN Explorer log found.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogComparison {
    pub matched: usize,
    /// Same ID at the same time, different data or length.
    pub differ: usize,
    pub wire_only: usize,
    pub log_only: usize,
    /// Log time minus capture time, in microseconds: where the capture start falls in the log.
    pub offset_us: Option<i64>,
    /// Log records that fall outside the captured time span.
    pub outside: usize,
}

/// Per frame of `out`'s table: how it compares with the log.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameMatch {
    /// Not compared (an error frame, a damaged frame).
    None,
    Matched { delta_us: i64 },
    Differs { log: String },
    WireOnly,
}

fn same(a: &CanFrame, b: &CanFrame) -> bool {
    a.id == b.id && a.extended == b.extended && a.rtr == b.rtr && (if a.rtr { true } else { a.data == b.data })
}

fn key(f: &CanFrame) -> (u32, bool) {
    (f.id, f.extended)
}

/// Finds the clock offset between the capture and the log: every pair of an identical wire frame
/// and log record votes for `log_ts - wire_ts`, and the offset that the most frames agree with,
/// within `window_us`, wins. Only the first frames are tried so a long log stays fast.
fn find_offset(wire: &[(i64, CanFrame)], log: &[LogRecord], window_us: i64) -> Option<i64> {
    let mut by_key: HashMap<(u32, bool), Vec<&LogRecord>> = HashMap::new();
    for r in log {
        by_key.entry(key(&r.frame)).or_default().push(r);
    }
    let mut candidates = Vec::new();
    for (t, f) in wire.iter().take(64) {
        for r in by_key.get(&key(f)).into_iter().flatten().take(4096) {
            if same(&r.frame, f) {
                candidates.push(r.ts_us as i64 - t);
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_unstable();
    // The candidate with the most others within the window (a sliding window over sorted values),
    // then the mean of that cluster.
    let (mut best, mut best_n, mut j) = ((0usize, 0usize), 0usize, 0usize);
    for i in 0..candidates.len() {
        while candidates[i] - candidates[j] > window_us {
            j += 1;
        }
        if i + 1 - j > best_n {
            best_n = i + 1 - j;
            best = (j, i);
        }
    }
    let cluster = &candidates[best.0..=best.1];
    Some(cluster.iter().sum::<i64>() / cluster.len() as i64)
}

/// Compares the frames on the wire with a candump log of the same bus (recorded by CAN Explorer
/// or any SocketCAN tool). The two clocks are aligned on the frames they share; then each wire
/// frame is matched with an identical log record within `window_us`. Adds a "CAN Explorer log"
/// row: frames the log also has, frames whose data differs, frames only the wire carried, and log
/// records the wire did not carry (a frame the adapter reported that never was on this wire).
pub fn compare_log(out: &mut DecodeOutput, cap: &Capture, frames: &[CanBitFrame], log: &[LogRecord], window_us: i64) -> (LogComparison, Vec<FrameMatch>) {
    ensure_row(out, ROW_LOG, "CAN Explorer log");
    let shown: Vec<&CanBitFrame> = frames.iter().filter(|f| f.error.as_deref() != Some("Glitch")).collect();
    let wire: Vec<(usize, i64, CanFrame)> =
        shown.iter().enumerate().filter_map(|(i, f)| to_can_frame(f).map(|c| (i, (secs(cap, f.start) * 1e6).round() as i64, c))).collect();
    let mut res = LogComparison::default();
    let mut matches = vec![FrameMatch::None; shown.len()];
    let pairs: Vec<(i64, CanFrame)> = wire.iter().map(|(_, t, c)| (*t, c.clone())).collect();
    let Some(offset) = find_offset(&pairs, log, window_us) else {
        res.wire_only = wire.len();
        res.outside = log.len();
        for (i, _, _) in &wire {
            matches[*i] = FrameMatch::WireOnly;
        }
        out.summary.push_str(", the log shares no frame with the capture");
        return (res, matches);
    };
    res.offset_us = Some(offset);
    // Log records on the capture's time line, sorted.
    let span_us = (secs(cap, cap.len()) * 1e6) as i64;
    let mut recs: Vec<(i64, &LogRecord)> = log.iter().map(|r| (r.ts_us as i64 - offset, r)).collect();
    recs.sort_by_key(|r| r.0);
    let mut used = vec![false; recs.len()];
    let nearest = |t: i64, used: &[bool], pred: &dyn Fn(&CanFrame) -> bool| -> Option<usize> {
        let lo = recs.partition_point(|r| r.0 < t - window_us);
        let mut best: Option<(i64, usize)> = None;
        for (k, r) in recs.iter().enumerate().skip(lo) {
            if r.0 > t + window_us {
                break;
            }
            if !used[k] && pred(&r.1.frame) {
                let d = (r.0 - t).abs();
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, k));
                }
            }
        }
        best.map(|b| b.1)
    };
    let mut left = Vec::new();
    for (i, t, c) in &wire {
        if let Some(k) = nearest(*t, &used, &|f| same(f, c)) {
            used[k] = true;
            res.matched += 1;
            matches[*i] = FrameMatch::Matched { delta_us: recs[k].0 - t };
        } else {
            left.push((*i, *t, c.clone()));
        }
    }
    for (i, t, c) in left {
        let f = shown[i];
        let end = f.end.max(f.start + 1);
        if let Some(k) = nearest(t, &used, &|g| key(g) == key(&c)) {
            used[k] = true;
            res.differ += 1;
            let log_text = mt_can::log::candump_frame(&recs[k].1.frame);
            out.annotations.push(Annotation::new(ROW_LOG, f.start, end, Level::Error, format!("Log has {log_text}, the wire {}", mt_can::log::candump_frame(&c)), "≠"));
            matches[i] = FrameMatch::Differs { log: log_text };
        } else {
            res.wire_only += 1;
            out.annotations.push(Annotation::new(ROW_LOG, f.start, end, Level::Warning, "On the wire, not in the log", "wire only"));
            matches[i] = FrameMatch::WireOnly;
        }
    }
    // Unused records inside the capture: the log has a frame the wire never carried.
    let rate = cap.sample_rate as f64;
    let mut lens: Vec<u64> = wire.iter().map(|(i, _, _)| shown[*i].end.saturating_sub(shown[*i].start)).collect();
    lens.sort_unstable();
    let typical_len = lens.get(lens.len() / 2).copied().unwrap_or(((window_us as f64) * rate / 1e6) as u64);
    for (k, (t, r)) in recs.iter().enumerate() {
        if used[k] {
            continue;
        }
        if *t < 0 || *t > span_us {
            res.outside += 1;
            continue;
        }
        res.log_only += 1;
        let s = ((*t as f64) * rate / 1e6) as u64;
        // About one frame long (the typical intact frame here), not the whole match window.
        let w = typical_len.max(1);
        out.annotations.push(Annotation::new(
            ROW_LOG,
            s,
            (s + w).min(cap.len()).max(s + 1),
            Level::Error,
            format!("In the log, not on the wire: {}", mt_can::log::candump_frame(&r.frame)),
            "log only",
        ));
    }
    for (i, m) in matches.iter().enumerate() {
        if let FrameMatch::Matched { delta_us } = m {
            let f = shown[i];
            out.annotations.push(Annotation::new(ROW_LOG, f.start, f.end.max(f.start + 1), Level::Info, format!("In the log ({delta_us:+} µs)"), "✓"));
        }
    }
    out.annotations.sort_by_key(|a| (a.row, a.start));
    if let Some(t) = &mut out.table {
        let at = t.columns.len();
        t.columns.insert(at, "Log".into());
        for (row, m) in t.rows.iter_mut().zip(&matches) {
            row.cells.push(match m {
                FrameMatch::None => String::new(),
                FrameMatch::Matched { delta_us } => format!("✓ {delta_us:+} µs"),
                FrameMatch::Differs { log } => format!("log: {log}"),
                FrameMatch::WireOnly => "not in log".into(),
            });
            if matches!(m, FrameMatch::Differs { .. }) {
                row.level = Level::Error;
            }
        }
    }
    out.summary.push_str(&format!(
        "; log: {} matched{}{}{}",
        res.matched,
        if res.differ > 0 { format!(", {} differ", res.differ) } else { String::new() },
        if res.wire_only > 0 { format!(", {} only on the wire", res.wire_only) } else { String::new() },
        if res.log_only > 0 { format!(", {} only in the log", res.log_only) } else { String::new() },
    ));
    (res, matches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::can::{self, CanConfig, CanFrameSpec};
    use crate::synth::{Synth, can_error_frame, can_tx};
    use mt_can::log::parse_candump;

    const RATE: u64 = 8_000_000;
    const BR: f64 = 500_000.0;

    /// Frames sent at the given times (seconds), returned with the capture.
    fn bus(frames: &[(f64, CanFrameSpec)]) -> Capture {
        let mut s = Synth::new(RATE, [true; 8]);
        let mut end = 0.0;
        for (t, f) in frames {
            end = can_tx(&mut s, 0, *t, BR, f);
        }
        s.extend_to(s.at(end + 0.0002));
        s.capture(&["CAN_RX"], 0)
    }

    fn cfg() -> CanConfig {
        CanConfig { channel: 0, bitrate: BR, ..Default::default() }
    }

    fn decoded(cap: &Capture) -> (Vec<CanBitFrame>, DecodeOutput) {
        let r = can::decode(cap, &cfg());
        let out = can::output(cap, &r);
        (r.frames, out)
    }

    fn eec1(rpm: f64) -> CanFrameSpec {
        let raw = (rpm / 0.125) as u16;
        CanFrameSpec::data(0x0CF00400, true, &[0xF0, 0x7D, 0x7D, raw as u8, (raw >> 8) as u8, 0, 0xF0, 0x7D])
    }

    #[test]
    fn converts_only_intact_frames() {
        let cap = bus(&[
            (0.0001, CanFrameSpec::data(0x123, false, &[1, 2, 3])),
            (0.0005, CanFrameSpec { rtr: true, dlc: 4, data: vec![], ..CanFrameSpec::data(0x285, false, &[]) }),
            (0.0009, CanFrameSpec { ack: false, ..CanFrameSpec::data(0x18FF5021, true, &[9]) }),
            (0.0013, CanFrameSpec { corrupt_crc: true, ack: false, ..CanFrameSpec::data(0x186, false, &[0x55]) }),
        ]);
        let (frames, _) = decoded(&cap);
        let got: Vec<CanFrame> = frames.iter().filter_map(to_can_frame).collect();
        assert_eq!(got.len(), 3, "{frames:#?}");
        assert_eq!(got[0], CanFrame::new(0x123, &[1, 2, 3]).unwrap());
        assert!(got[1].rtr && got[1].rtr_len == 4 && got[1].id == 0x285);
        assert!(got[2].extended && got[2].id == 0x18FF5021, "an unacknowledged frame is still on the wire");
    }

    #[test]
    fn error_frames_are_not_exported() {
        let mut s = Synth::new(RATE, [true; 8]);
        let t = can_tx(&mut s, 0, 0.0001, BR, &CanFrameSpec::data(0x100, false, &[1]));
        let t = can_error_frame(&mut s, 0, t + 0.00005, BR);
        let t = can_tx(&mut s, 0, t + 0.0001, BR, &CanFrameSpec::data(0x101, false, &[2]));
        s.extend_to(s.at(t + 0.0002));
        let cap = s.capture(&["CAN_RX"], 0);
        let (frames, _) = decoded(&cap);
        let log = to_candump(&cap, &frames, 0, "can0");
        assert_eq!(log.lines().count(), 2, "{log}");
        assert!(log.contains("100#01") && log.contains("101#02"));
    }

    #[test]
    fn candump_round_trips_through_can_explorers_parser() {
        let cap = bus(&[(0.0001, eec1(1500.0)), (0.001, CanFrameSpec::data(0x705, false, &[5])), (0.002, eec1(1501.0))]);
        let (frames, _) = decoded(&cap);
        let start = 1_794_000_000_000_000u64;
        let text = to_candump(&cap, &frames, start, "can0");
        let recs = parse_candump(&text).unwrap();
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].frame, CanFrame::with_ext(0x0CF00400, true, &eec1(1500.0).data).unwrap());
        assert_eq!(recs[1].frame.id_text(), "705");
        assert_eq!(recs[0].ts_us, start + 100, "SOF of the first frame at 100 µs");
        assert_eq!(recs[2].ts_us, start + 2000);
        assert!(recs.iter().all(|r| r.iface == "can0"));
    }

    #[test]
    fn dbc_names_signals_like_can_explorer() {
        let dbc = Dbc::parse(SIM_DBC).unwrap();
        let cap = bus(&[(0.0001, eec1(1500.0)), (0.001, CanFrameSpec::data(0x7AA, false, &[1]))]);
        let (frames, mut out) = decoded(&cap);
        add_signals(&mut out, &frames, &dbc);
        assert_eq!(out.rows[ROW_SIGNALS as usize], "Signals");
        let sig: Vec<&Annotation> = out.annotations.iter().filter(|a| a.row == ROW_SIGNALS).collect();
        assert_eq!(sig.len(), 1, "only the frame the DBC describes");
        assert!(sig[0].text.starts_with("EEC1 ") && sig[0].text.contains("EngineSpeed=1500 rpm"), "{}", sig[0].text);
        assert_eq!(sig[0].short, "EEC1");
        let t = out.table.as_ref().unwrap();
        assert_eq!(t.columns, ["Time", "ID", "Format", "DLC", "Data", "Stuff bits", "Message", "Signals", "Status"]);
        assert_eq!(t.rows[0].cells[6], "EEC1");
        assert!(t.rows[0].cells[7].contains("EngineSpeed=1500 rpm"));
        assert_eq!(t.rows[1].cells[6], "");
        assert!(t.rows.iter().all(|r| r.cells.len() == t.columns.len()));
        assert!(out.summary.ends_with("1 named by the DBC"), "{}", out.summary);
    }

    #[test]
    fn the_simulator_scenario_decodes_with_can_explorers_dbc() {
        use crate::sim::{ScenarioId, info, render};
        let i = info(ScenarioId::CanBus);
        let cap = Capture::new(i.default_rate, i.channels.clone(), render(ScenarioId::CanBus, i.default_rate));
        let c = i
            .decoders
            .iter()
            .find_map(|d| match d {
                crate::decode::DecoderConfig::Can(c) => Some(c.clone()),
                _ => None,
            })
            .unwrap();
        let r = can::decode(&cap, &c);
        let mut out = can::output(&cap, &r);
        add_signals(&mut out, &r.frames, &Dbc::parse(SIM_DBC).unwrap());
        let texts: Vec<&str> = out.annotations.iter().filter(|a| a.row == ROW_SIGNALS).map(|a| a.text.as_str()).collect();
        assert!(texts.iter().any(|t| t.starts_with("EEC1 ")), "{texts:?}");
        assert!(texts.iter().any(|t| t.starts_with("ET1 ") && t.contains("CoolantTemp=88 degC")), "{texts:?}");
    }

    /// A log that CAN Explorer could have recorded of the same bus, on its own clock.
    fn log_of(lines: &[(u64, &str)]) -> Vec<LogRecord> {
        let text: String = lines.iter().map(|(t, f)| format!("({}.{:06}) can0 {f}\n", t / 1_000_000, t % 1_000_000)).collect();
        parse_candump(&text).unwrap()
    }

    #[test]
    fn compare_aligns_clocks_and_flags_differences() {
        let cap = bus(&[
            (0.0001, CanFrameSpec::data(0x100, false, &[1])),
            (0.0010, CanFrameSpec::data(0x200, false, &[2])),
            (0.0020, CanFrameSpec::data(0x300, false, &[3])),
            (0.0030, CanFrameSpec::data(0x400, false, &[4])),
            (0.0040, CanFrameSpec::data(0x100, false, &[5])),
        ]);
        let (frames, mut out) = decoded(&cap);
        // The log's clock is 1 h ahead, with ±40 µs of host jitter. It has 0x300 with other data,
        // misses 0x400, and holds a 0x555 the wire never carried, plus a record from before the capture.
        let base = 3_600_000_000u64;
        let log = log_of(&[
            (base - 50_000, "7FF#00"),
            (base + 100 + 30, "100#01"),
            (base + 1000 - 40, "200#02"),
            (base + 2000 + 10, "300#33"),
            (base + 2500, "555#AA"),
            (base + 4000 + 20, "100#05"),
        ]);
        let (res, m) = compare_log(&mut out, &cap, &frames, &log, 500);
        assert_eq!(res.matched, 3, "{res:?}");
        assert_eq!(res.differ, 1);
        assert_eq!(res.wire_only, 1);
        assert_eq!(res.log_only, 1);
        assert_eq!(res.outside, 1);
        let off = res.offset_us.unwrap();
        assert!((off - base as i64).abs() < 50, "offset {off}");
        assert!(matches!(m[0], FrameMatch::Matched { .. }));
        assert_eq!(m[2], FrameMatch::Differs { log: "300#33".into() });
        assert_eq!(m[3], FrameMatch::WireOnly);
        let row: Vec<&Annotation> = out.annotations.iter().filter(|a| a.row == ROW_LOG).collect();
        let lo = row.iter().find(|a| a.text == "In the log, not on the wire: 555#AA").unwrap();
        assert_eq!(lo.level, Level::Error);
        assert!(lo.end - lo.start <= 1000, "about one frame wide (a 1-byte frame is ~800 samples), not the 4000-sample window");
        let t = out.table.as_ref().unwrap();
        assert_eq!(t.columns.last().unwrap(), "Log");
        assert_eq!(t.rows[2].cells.last().unwrap(), "log: 300#33");
        assert_eq!(t.rows[3].cells.last().unwrap(), "not in log");
        assert!(t.rows.iter().all(|r| r.cells.len() == t.columns.len()));
        assert!(out.summary.contains("log: 3 matched, 1 differ, 1 only on the wire, 1 only in the log"), "{}", out.summary);
    }

    #[test]
    fn repeated_frames_match_one_to_one() {
        // The same frame every millisecond: each wire frame takes the nearest log record once.
        let specs: Vec<(f64, CanFrameSpec)> = (0..6).map(|i| (0.0001 + i as f64 * 0.001, eec1(1500.0))).collect();
        let cap = bus(&specs);
        let (frames, mut out) = decoded(&cap);
        let data = "0CF00400#F07D7DE02E00F07D";
        let log = log_of(&(0..5).map(|i| (77_000_000 + 100 + i * 1000, data)).collect::<Vec<_>>());
        let (res, _) = compare_log(&mut out, &cap, &frames, &log, 300);
        assert_eq!((res.matched, res.wire_only, res.log_only), (5, 1, 0), "{res:?}");
    }

    #[test]
    fn an_unrelated_log_says_so() {
        let cap = bus(&[(0.0001, CanFrameSpec::data(0x100, false, &[1]))]);
        let (frames, mut out) = decoded(&cap);
        let (res, m) = compare_log(&mut out, &cap, &frames, &log_of(&[(5, "200#02")]), 500);
        assert_eq!(res.offset_us, None);
        assert_eq!(res.wire_only, 1);
        assert_eq!(m, vec![FrameMatch::WireOnly]);
        assert!(out.summary.contains("shares no frame"));
    }
}
