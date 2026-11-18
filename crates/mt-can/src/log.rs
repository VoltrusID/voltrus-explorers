//! Trace files: the candump log format (`candump -l`, read by `canplayer`, SavvyCAN and Wireshark's
//! importers) and a CSV layout for spreadsheets, written here; [`parse_log`] also reads candump's screen
//! output, Vector ASC and PCAN TRC traces.
//!
//! ```text
//! (1699999999.123456) can0 123#DEADBEEF
//! (1699999999.223456) can0 12345678#R
//! (1699999999.323456) can0 123##1AABBCCDDEEFF0011
//! ```

use crate::error::CanError;
use crate::frame::{CanFrame, Direction, TimedFrame, hex_bytes, parse_hex_bytes};

/// One candump line (without a newline).
pub fn candump_line(f: &TimedFrame, iface: &str) -> String {
    let secs = f.ts_us / 1_000_000;
    let micros = f.ts_us % 1_000_000;
    format!("({secs}.{micros:06}) {iface} {}", candump_frame(&f.frame))
}

/// `123#DEADBEEF`, `123#R`, `123##1AABB` (cansend's syntax too).
pub fn candump_frame(f: &CanFrame) -> String {
    let id = f.id_text();
    if f.rtr {
        return if f.rtr_len > 0 { format!("{id}#R{}", f.rtr_len) } else { format!("{id}#R") };
    }
    let data: String = f.data.iter().map(|b| format!("{b:02X}")).collect();
    if f.fd {
        // The flags nibble is the kernel's: 1 = bit rate switch, 2 = error state indicator.
        let flags = f.brs as u8 | (f.esi as u8) << 1;
        format!("{id}##{flags:X}{data}")
    } else {
        format!("{id}#{data}")
    }
}

/// Parses cansend syntax: `123#DEADBEEF`, `1F334455#11.22`, `123#R`, `123#R3`, `123##1AABB`
/// (CAN FD: the digit after `##` holds the flags, 1 = bit rate switch, 2 = error state indicator).
/// Three hex digits or fewer is an 11-bit identifier, eight is 29-bit (as cansend decides).
pub fn parse_frame(text: &str) -> Result<CanFrame, CanError> {
    let text = text.trim();
    let bad = |m: &str| CanError::Frame(format!("{m}: \"{text}\" (expected e.g. 123#DEADBEEF)"));
    let (id_s, rest) = text.split_once('#').ok_or_else(|| bad("no '#'"))?;
    if id_s.is_empty() || id_s.len() > 8 {
        return Err(bad("identifier must be 1 to 8 hex digits"));
    }
    let id = u32::from_str_radix(id_s, 16).map_err(|_| bad("identifier is not hex"))?;
    let extended = id_s.len() > 3;
    if rest.contains('_') {
        return Err(bad("a raw DLC (_9 to _F, candump's len8_dlc) cannot be sent; leave it out"));
    }
    if let Some(fd) = rest.strip_prefix('#') {
        let mut chars = fd.chars();
        let flags = chars.next().and_then(|c| c.to_digit(16)).ok_or_else(|| bad("CAN FD frames need a flags digit after ##"))?;
        let data = parse_hex_bytes(chars.as_str())?;
        let mut f = CanFrame::fd(id, extended, flags & 1 != 0, &data)?;
        f.esi = flags & 2 != 0;
        return Ok(f);
    }
    if let Some(r) = rest.strip_prefix('R').or_else(|| rest.strip_prefix('r')) {
        let len = if r.is_empty() { 0 } else { r.parse::<u8>().map_err(|_| bad("remote length must be 0 to 8"))? };
        return CanFrame::remote(id, extended, len);
    }
    CanFrame::with_ext(id, extended, &parse_hex_bytes(rest)?)
}

/// A frame read back from a log, with the interface name it was recorded on.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRecord {
    pub ts_us: u64,
    pub iface: String,
    /// Rx unless the log says the frame was sent (candump -x `T`, ASC/TRC `Tx`, CSV `tx`).
    pub dir: Direction,
    pub frame: CanFrame,
}

/// Reads a candump log strictly: the first line that is not a frame is an error. Blank lines and comments
/// (`#`, `;`, `//`) are skipped. See [`parse_log`] for the forgiving reader the app uses.
pub fn parse_candump(text: &str) -> Result<Vec<LogRecord>, CanError> {
    let mut out = Vec::new();
    let mut last_ts = 0u64;
    for (n, line) in text.lines().enumerate() {
        let line = clean_line(line, n == 0);
        if is_comment(line) {
            continue;
        }
        match candump_entry(line) {
            Ok(Entry::Frame { ts, iface, dir, frame }) => {
                let ts_us = ts.unwrap_or(last_ts);
                last_ts = ts_us;
                out.push(LogRecord { ts_us, iface: iface.unwrap_or_default(), dir, frame });
            }
            Ok(Entry::ErrorFrame) | Ok(Entry::Skip) => {}
            Err(msg) => return Err(CanError::Log { line: n + 1, msg }),
        }
    }
    Ok(out)
}

/// The trace file formats [`parse_log`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// candump: `candump -l` log files, candump's screen output (with or without `-t` times and `-x`), cansend syntax.
    Candump,
    /// Vector ASC (ASCII) logs.
    Asc,
    /// PEAK PCAN-View / PCAN-Basic TRC traces, versions 1.0 to 2.1.
    Trc,
    /// CSV as this app exports it (also python-can's CSV).
    Csv,
}

impl LogFormat {
    pub fn name(self) -> &'static str {
        match self {
            LogFormat::Candump => "candump",
            LogFormat::Asc => "Vector ASC",
            LogFormat::Trc => "PCAN TRC",
            LogFormat::Csv => "CSV",
        }
    }
}

/// A trace file read with [`parse_log`]: the frames, and what could not be read.
#[derive(Debug, Clone)]
pub struct ParsedLog {
    pub format: LogFormat,
    pub records: Vec<LogRecord>,
    /// Lines that should have been frames but could not be read.
    pub bad_lines: usize,
    /// The first few of them: line number (1-based) and why.
    pub bad_examples: Vec<(usize, String)>,
    /// Error frames in the file; they are not imported.
    pub error_frames: usize,
    /// Frames that had no time in the file; they were given one 1 ms after the previous frame.
    pub synthesized_times: usize,
    /// The file ends in the middle of a line that could not be read; it was left out.
    pub truncated_last_line: bool,
}

impl ParsedLog {
    /// Plain-language notes about anything that was not read as-is, for the user.
    pub fn notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.bad_lines > 0 {
            let lines: Vec<String> = self.bad_examples.iter().map(|(n, _)| n.to_string()).collect();
            let first = self.bad_examples.first().map(|(n, m)| format!(" Line {n}: {m}.")).unwrap_or_default();
            out.push(format!(
                "{} line{} could not be read and {} skipped (line{} {}{}).{first}",
                self.bad_lines,
                if self.bad_lines == 1 { "" } else { "s" },
                if self.bad_lines == 1 { "was" } else { "were" },
                if lines.len() == 1 { "" } else { "s" },
                lines.join(", "),
                if self.bad_lines > lines.len() { ", ..." } else { "" },
            ));
        }
        if self.truncated_last_line {
            out.push("The last line is cut off (was the file still being written?) and was skipped.".into());
        }
        if self.error_frames > 0 {
            out.push(format!("{} error frame{} not imported.", self.error_frames, if self.error_frames == 1 { "" } else { "s" }));
        }
        if self.synthesized_times > 0 {
            if self.synthesized_times == self.records.len() {
                out.push("The file has no times; the frames were spaced 1 ms apart in file order.".into());
            } else {
                out.push(format!("{} frames had no time and were placed 1 ms after the frame before.", self.synthesized_times));
            }
        }
        out
    }
}

/// How many bad lines [`ParsedLog::bad_examples`] keeps.
const BAD_EXAMPLES: usize = 5;

/// Reads a trace file: candump (log files and screen output), Vector ASC, PCAN TRC or CSV, detected from the content.
/// Lines that cannot be read are skipped and counted; it fails only when no frame at all could be read.
pub fn parse_log(text: &str) -> Result<ParsedLog, CanError> {
    let format = detect_format(text);
    let mut out = ParsedLog {
        format,
        records: Vec::new(),
        bad_lines: 0,
        bad_examples: Vec::new(),
        error_frames: 0,
        synthesized_times: 0,
        truncated_last_line: false,
    };
    let mut times: Vec<Option<u64>> = Vec::new();
    let ends_cleanly = text.is_empty() || text.ends_with('\n') || text.ends_with('\r');
    let mut asc = AscState::default();
    let mut trc = TrcState::default();
    let mut csv: Option<CsvLayout> = None;
    let line_count = text.lines().count();
    for (n, raw) in text.lines().enumerate() {
        let line = clean_line(raw, n == 0);
        let result = match format {
            LogFormat::Candump => {
                if is_comment(line) {
                    continue;
                }
                candump_entry(line)
            }
            LogFormat::Asc => asc.entry(line),
            LogFormat::Trc => trc.entry(line),
            LogFormat::Csv => {
                if line.is_empty() {
                    continue;
                }
                match &csv {
                    None => match CsvLayout::from_header(line) {
                        Some(l) => {
                            csv = Some(l);
                            continue;
                        }
                        None => Err("expected a CSV header line".to_string()),
                    },
                    Some(l) => l.entry(line),
                }
            }
        };
        match result {
            Ok(Entry::Frame { ts, iface, dir, frame }) => {
                times.push(ts);
                out.records.push(LogRecord { ts_us: 0, iface: iface.unwrap_or_default(), dir, frame });
            }
            Ok(Entry::ErrorFrame) => out.error_frames += 1,
            Ok(Entry::Skip) => {}
            Err(msg) => {
                if n + 1 == line_count && !ends_cleanly {
                    out.truncated_last_line = true;
                } else {
                    out.bad_lines += 1;
                    if out.bad_examples.len() < BAD_EXAMPLES {
                        out.bad_examples.push((n + 1, msg));
                    }
                }
            }
        }
    }
    if out.records.is_empty() {
        let why = match out.bad_examples.first() {
            Some((line, msg)) => format!(
                "No frames could be read from this {} file ({} bad line{}; line {line}: {msg}).",
                format.name(),
                out.bad_lines,
                if out.bad_lines == 1 { "" } else { "s" }
            ),
            None if out.truncated_last_line => "The file has only a cut-off line.".into(),
            None => format!("The {} file has no frames.", format.name()),
        };
        return Err(CanError::Unsupported(why));
    }
    // Frames without a time: 1 ms after the frame before.
    let mut prev: Option<u64> = None;
    for (r, t) in out.records.iter_mut().zip(times) {
        r.ts_us = match t {
            Some(t) => t,
            None => {
                out.synthesized_times += 1;
                prev.map(|p| p + 1000).unwrap_or(0)
            }
        };
        prev = Some(r.ts_us);
    }
    Ok(out)
}

/// One line of a trace file.
enum Entry {
    Frame { ts: Option<u64>, iface: Option<String>, dir: Direction, frame: CanFrame },
    ErrorFrame,
    /// Headers, comments, events that are not frames.
    Skip,
}

fn clean_line(line: &str, first: bool) -> &str {
    let line = if first { line.trim_start_matches('\u{FEFF}') } else { line };
    line.trim_matches(|c: char| c.is_whitespace() || c == '\0')
}

fn is_comment(line: &str) -> bool {
    line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with("//")
}

fn detect_format(text: &str) -> LogFormat {
    for (n, raw) in text.lines().take(40).enumerate() {
        let l = clean_line(raw, n == 0);
        if l.is_empty() {
            continue;
        }
        let lower = l.to_ascii_lowercase();
        if lower.starts_with(";$fileversion") || lower.starts_with(";$starttime") || (l.starts_with(';') && lower.contains("message number")) {
            return LogFormat::Trc;
        }
        if lower.starts_with("date ") || lower.starts_with("base hex") || lower.starts_with("base dec") || lower.starts_with("begin triggerblock") {
            return LogFormat::Asc;
        }
        if CsvLayout::from_header(l).is_some() {
            return LogFormat::Csv;
        }
        if is_comment(l) {
            continue;
        }
        // TRC 1.x without its header: "1)  1841.0  Rx  0123  8  ..."
        if l.split_whitespace().next().is_some_and(|t| t.len() > 1 && t.ends_with(')') && t[..t.len() - 1].chars().all(|c| c.is_ascii_digit())) {
            return LogFormat::Trc;
        }
        return LogFormat::Candump;
    }
    LogFormat::Candump
}

/// `(1699999999.123456)`, `(1699999999,123456)`, `(2023-11-14 22:13:20.000100)` (candump -tA, read as UTC).
fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.len() >= 19 && s.as_bytes().get(4) == Some(&b'-') && s.as_bytes().get(10) == Some(&b' ') {
        let (date, time) = s.split_once(' ')?;
        let mut d = date.split('-');
        let (y, m, dd) = (d.next()?.parse::<i64>().ok()?, d.next()?.parse::<u32>().ok()?, d.next()?.parse::<u32>().ok()?);
        let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
        let mut t = hms.split(':');
        let (h, mi, sec) = (t.next()?.parse::<u64>().ok()?, t.next()?.parse::<u64>().ok()?, t.next()?.parse::<u64>().ok()?);
        let days = u64::try_from(days_from_civil(y, m, dd)?).ok()?;
        let secs = days.checked_mul(86_400)? + h * 3600 + mi * 60 + sec;
        return secs.checked_mul(1_000_000)?.checked_add(frac_micros(frac)?);
    }
    let (sec, frac) = s.split_once(['.', ',']).unwrap_or((s, "0"));
    let sec = sec.trim();
    if sec.is_empty() || !sec.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    sec.parse::<u64>().ok()?.checked_mul(1_000_000)?.checked_add(frac_micros(frac.trim())?)
}

/// Fraction digits to microseconds (digits past the sixth are dropped).
fn frac_micros(frac: &str) -> Option<u64> {
    if frac.is_empty() || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut digits: String = frac.chars().take(6).collect();
    while digits.len() < 6 {
        digits.push('0');
    }
    digits.parse().ok()
}

/// Seconds as a decimal number (`12.345678`) to microseconds; negative or unreadable gives `None`.
fn secs_to_us(s: &str) -> Option<u64> {
    let v: f64 = s.trim().parse().ok()?;
    (v.is_finite() && (0.0..1.8e13).contains(&v)).then(|| (v * 1e6).round() as u64)
}

/// Milliseconds as TRC files write them (`1841`, `1841.3`) to microseconds.
fn ms_to_us(s: &str) -> Option<u64> {
    let v: f64 = s.trim().parse().ok()?;
    (v.is_finite() && (0.0..1.8e16).contains(&v)).then(|| (v * 1000.0).round() as u64)
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn is_hex_byte(t: &str) -> bool {
    t.len() == 2 && t.chars().all(|c| c.is_ascii_hexdigit())
}

fn hex_tokens(toks: &[&str], len: usize) -> Result<Vec<u8>, String> {
    if toks.len() < len || !toks[..len].iter().all(|b| is_hex_byte(b)) {
        return Err(format!("{len} data bytes expected"));
    }
    Ok(toks[..len].iter().map(|b| u8::from_str_radix(b, 16).unwrap_or(0)).collect())
}

/// An identifier in hex (or decimal), with an optional `0x` prefix and `x` suffix (29-bit, in ASC files).
fn parse_id(s: &str, radix: u32) -> Result<(u32, bool), String> {
    let s = s.trim();
    let (s, x) = match s.strip_suffix(['x', 'X']) {
        Some(r) => (r, true),
        None => (s, false),
    };
    let digits = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    let id = u32::from_str_radix(digits, radix).map_err(|_| format!("bad identifier \"{s}\""))?;
    Ok((id, x))
}

fn fd_frame(id: u32, extended: bool, brs: bool, esi: bool, data: &[u8]) -> Result<CanFrame, CanError> {
    let mut f = CanFrame::fd(id, extended, brs, data)?;
    f.esi = esi;
    Ok(f)
}

/// candump: `(ts) iface 123#11 [T|R]`, `iface 123#11`, `123#11`, or the screen form
/// `[(ts)] iface [TX|RX - -] 123 [2] 11 22`, `iface 7DF [8] remote request`, `iface 123 [08] ...` (FD).
fn candump_entry(line: &str) -> Result<Entry, String> {
    let mut rest = line;
    let mut ts = None;
    if let Some(r) = rest.strip_prefix('(') {
        let (t, r) = r.split_once(')').ok_or("no closing ')' after the time")?;
        ts = Some(parse_time(t).ok_or_else(|| format!("bad time ({t})"))?);
        rest = r;
    }
    let toks: Vec<&str> = rest.split_whitespace().collect();
    if toks.is_empty() {
        return Err("no frame after the time".into());
    }
    let (iface, i) = if toks[0].contains('#') { (None, 0) } else { (Some(toks[0].to_string()), 1) };
    if let Some(ft) = toks.get(i).filter(|t| t.contains('#')) {
        let Some(frame) = log_frame(ft)? else { return Ok(Entry::ErrorFrame) };
        let dir = match toks.get(i + 1).copied() {
            None | Some("R") => Direction::Rx,
            Some("T") => Direction::Tx,
            Some(x) => return Err(format!("unexpected \"{x}\" after the frame")),
        };
        if let Some(x) = toks.get(i + 2) {
            return Err(format!("unexpected \"{x}\" after the frame"));
        }
        return Ok(Entry::Frame { ts, iface, dir, frame });
    }
    if iface.is_none() || toks.len() < 3 {
        return Err("expected \"(seconds.micros) iface id#data\" or candump's \"iface id [len] data\"".into());
    }
    // Screen form.
    let mut j = 1;
    let mut dir = Direction::Rx;
    let (mut brs, mut esi) = (false, false);
    if matches!(toks[1], "TX" | "RX") {
        dir = if toks[1] == "TX" { Direction::Tx } else { Direction::Rx };
        j = 2;
        if toks.len() > j + 1 && matches!(toks[j], "B" | "-") && matches!(toks[j + 1], "E" | "-") {
            brs = toks[j] == "B";
            esi = toks[j + 1] == "E";
            j += 2;
        }
    }
    let id_s = toks.get(j).ok_or("no identifier")?;
    let len_s = toks.get(j + 1).ok_or("no [length]")?;
    let inner = len_s.strip_prefix('[').and_then(|s| s.strip_suffix(']')).ok_or_else(|| format!("expected [length], not \"{len_s}\""))?;
    let len: usize = inner.parse().map_err(|_| format!("bad length \"{len_s}\""))?;
    // candump writes FD lengths with two digits ([08], [64]) and classic ones with one ([8]).
    let fd = inner.len() == 2;
    if id_s.is_empty() || id_s.len() > 8 {
        return Err(format!("bad identifier \"{id_s}\""));
    }
    let (id, _) = parse_id(id_s, 16)?;
    if id_s.len() == 8 && id & 0x2000_0000 != 0 {
        return Ok(Entry::ErrorFrame);
    }
    let extended = id_s.len() > 3;
    let tail = &toks[j + 2..];
    if tail.contains(&"ERRORFRAME") {
        return Ok(Entry::ErrorFrame);
    }
    if !fd && tail.first() == Some(&"remote") {
        let f = CanFrame::remote(id, extended, len.min(255) as u8).map_err(|e| e.to_string())?;
        return Ok(Entry::Frame { ts, iface, dir, frame: f });
    }
    let data = hex_tokens(tail, len).map_err(|_| format!("[{len}] but fewer data bytes"))?;
    let frame = if fd { fd_frame(id, extended, brs, esi, &data) } else { CanFrame::with_ext(id, extended, &data) }.map_err(|e| e.to_string())?;
    Ok(Entry::Frame { ts, iface, dir, frame })
}

/// A frame in a log in cansend syntax, also with candump's raw DLC suffix (`123#1122334455667788_9`, read as
/// the 8-byte frame). `None` for error frames (CAN_ERR_FLAG set in the identifier).
fn log_frame(text: &str) -> Result<Option<CanFrame>, String> {
    let (id_s, _) = text.split_once('#').unwrap_or((text, ""));
    if id_s.len() == 8 && u32::from_str_radix(id_s, 16).is_ok_and(|id| id & 0x2000_0000 != 0) {
        return Ok(None);
    }
    let text = match text.rsplit_once('_') {
        Some((f, dlc)) if !f.contains("##") && dlc.len() == 1 && u8::from_str_radix(dlc, 16).is_ok_and(|d| d > 8) => f,
        Some(_) => return Err(format!("bad raw DLC in \"{text}\" (expected _9 to _F)")),
        None => text,
    };
    parse_frame(text).map(Some).map_err(|e| e.to_string())
}

// ── Vector ASC ──

#[derive(Default)]
struct AscState {
    dec: bool,
    /// `timestamps relative`: each time counts from the previous event.
    relative: bool,
    /// Measurement start (from `date` or `Begin Triggerblock`), µs since the epoch.
    start_us: u64,
    last_us: u64,
}

impl AscState {
    fn entry(&mut self, line: &str) -> Result<Entry, String> {
        if line.is_empty() || line.starts_with("//") {
            return Ok(Entry::Skip);
        }
        let lower = line.to_ascii_lowercase();
        if let Some(d) = lower.strip_prefix("date ") {
            if let Some(t) = asc_date(d) {
                self.start_us = t;
            }
            return Ok(Entry::Skip);
        }
        if let Some(b) = lower.strip_prefix("begin triggerblock") {
            if let Some(t) = asc_date(b) {
                self.start_us = t;
            }
            self.last_us = 0;
            return Ok(Entry::Skip);
        }
        if let Some(b) = lower.strip_prefix("base ") {
            self.dec = b.trim_start().starts_with("dec");
            self.relative = b.contains("relative");
            return Ok(Entry::Skip);
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        let Some(t) = toks.first().and_then(|t| secs_to_us(t)) else {
            // Header lines ("internal events logged", "End TriggerBlock", ...).
            return Ok(Entry::Skip);
        };
        let rel = if self.relative { self.last_us + t } else { t };
        self.last_us = rel;
        let ts = Some(self.start_us + rel);
        let radix = if self.dec { 10 } else { 16 };
        let dir_of = |d: &str| if d.eq_ignore_ascii_case("tx") { Direction::Tx } else { Direction::Rx };
        if toks.get(1) == Some(&"CANFD") {
            // time CANFD ch dir id [name] brs esi dlc len data... [duration length flags crc ...]
            if toks.get(4).is_some_and(|t| t.eq_ignore_ascii_case("errorframe")) || toks.get(3).is_some_and(|t| t.eq_ignore_ascii_case("errorframe")) {
                return Ok(Entry::ErrorFrame);
            }
            if toks.len() < 9 {
                return Err("CAN FD line too short".into());
            }
            let dir = dir_of(toks[3]);
            let (id, x) = parse_id(toks[4], radix)?;
            let mut p = 5;
            if !(matches!(toks.get(p), Some(&"0" | &"1")) && matches!(toks.get(p + 1), Some(&"0" | &"1"))) {
                p += 1; // symbolic name
            }
            let brs = toks.get(p) == Some(&"1");
            let esi = toks.get(p + 1) == Some(&"1");
            let dlc = toks.get(p + 2).and_then(|t| u8::from_str_radix(t, 16).ok()).ok_or("bad DLC")?;
            let len: usize = toks.get(p + 3).and_then(|t| t.parse().ok()).ok_or("bad data length")?;
            let bytes = &toks[(p + 4).min(toks.len())..];
            let data = hex_tokens(bytes, len)?;
            let extended = x || id > 0x7FF;
            // The flags field (bit 12: EDL, bit 4: remote) tells FD frames from classic ones on an FD channel.
            let flags = bytes.get(len + 2).and_then(|f| u32::from_str_radix(f, 16).ok());
            let is_fd = flags.map(|f| f & 0x1000 != 0).unwrap_or(true) || len > 8;
            let frame = if len == 0 && !is_fd && flags.is_some_and(|f| f & 0x10 != 0) {
                CanFrame::remote(id, extended, dlc.min(8))
            } else if is_fd {
                fd_frame(id, extended, brs, esi, &data)
            } else {
                CanFrame::with_ext(id, extended, &data)
            }
            .map_err(|e| e.to_string())?;
            return Ok(Entry::Frame { ts, iface: None, dir, frame });
        }
        // time ch id dir d dlc data... | time ch id dir r [dlc] | time ch ErrorFrame
        let Some(ch) = toks.get(1) else { return Ok(Entry::Skip) };
        if !ch.chars().all(|c| c.is_ascii_digit()) {
            return Ok(Entry::Skip); // "Start of measurement", statistics, J1939TP and other events
        }
        let Some(id_s) = toks.get(2) else { return Ok(Entry::Skip) };
        if id_s.eq_ignore_ascii_case("errorframe") {
            return Ok(Entry::ErrorFrame);
        }
        let Some(dir_s) = toks.get(3) else { return Err("no direction".into()) };
        if !matches!(dir_s.to_ascii_lowercase().as_str(), "rx" | "tx") {
            return Ok(Entry::Skip); // TxRq and other events
        }
        let (id, x) = parse_id(id_s, radix)?;
        let extended = x || id > 0x7FF;
        let dir = dir_of(dir_s);
        let kind = toks.get(4).copied().unwrap_or("");
        let frame = if kind.eq_ignore_ascii_case("r") {
            let dlc = toks.get(5).and_then(|t| u8::from_str_radix(t, radix).ok()).unwrap_or(0);
            CanFrame::remote(id, extended, dlc.min(8))
        } else if kind.eq_ignore_ascii_case("d") {
            let dlc = toks.get(5).and_then(|t| u8::from_str_radix(t, radix).ok()).ok_or("bad DLC")?;
            let data = hex_tokens(&toks[6.min(toks.len())..], (dlc as usize).min(8))?;
            CanFrame::with_ext(id, extended, &data)
        } else {
            return Err(format!("expected d or r after the direction, not \"{kind}\""));
        }
        .map_err(|e| e.to_string())?;
        Ok(Entry::Frame { ts, iface: None, dir, frame })
    }
}

/// `Wed Nov 18 06:58:28.371 2026` (also with `am`/`pm`) to µs since the epoch, read as UTC.
fn asc_date(s: &str) -> Option<u64> {
    let toks: Vec<String> = s.split_whitespace().map(|t| t.to_ascii_lowercase()).collect();
    let months = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let mi = toks.iter().position(|t| t.len() >= 3 && months.contains(&&t[..3]))?;
    let month = months.iter().position(|m| toks[mi].starts_with(m))? as u32 + 1;
    let day: u32 = toks.get(mi + 1)?.parse().ok()?;
    let time = toks.get(mi + 2)?;
    let mut rest = toks.get(mi + 3..)?;
    let mut pm = None;
    if let Some(ap) = rest.first().filter(|t| *t == "am" || *t == "pm") {
        pm = Some(ap == "pm");
        rest = &rest[1..];
    }
    let year: i64 = rest.first()?.parse().ok()?;
    let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
    let mut t = hms.split(':');
    let (mut h, m, sec): (u64, u64, u64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    match pm {
        Some(true) if h < 12 => h += 12,
        Some(false) if h == 12 => h = 0,
        _ => {}
    }
    let days = u64::try_from(days_from_civil(year, month, day)?).ok()?;
    Some((days * 86_400 + h * 3600 + m * 60 + sec) * 1_000_000 + frac_micros(frac)?)
}

// ── PCAN TRC ──

#[derive(Default)]
struct TrcState {
    /// `$STARTTIME`, µs since the epoch.
    start_us: u64,
    /// `$COLUMNS` (2.x): the column letter of each position.
    columns: Vec<char>,
    version: String,
}

impl TrcState {
    fn entry(&mut self, line: &str) -> Result<Entry, String> {
        if line.is_empty() {
            return Ok(Entry::Skip);
        }
        if let Some(h) = line.strip_prefix(";$") {
            let (k, v) = h.split_once('=').unwrap_or((h, ""));
            match k.trim().to_ascii_uppercase().as_str() {
                "FILEVERSION" => self.version = v.trim().to_string(),
                // Days since 1899-12-30 (an OLE date), UTC.
                "STARTTIME" => {
                    if let Ok(days) = v.trim().parse::<f64>() {
                        let s = (days - 25_569.0) * 86_400.0;
                        if s.is_finite() && s > 0.0 {
                            self.start_us = (s * 1e6).round() as u64;
                        }
                    }
                }
                "COLUMNS" => self.columns = v.split(',').filter_map(|c| c.trim().chars().next()).collect(),
                _ => {}
            }
            return Ok(Entry::Skip);
        }
        if line.starts_with(';') {
            return Ok(Entry::Skip);
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        if self.version.starts_with('2') { self.entry_v2(&toks) } else { self.entry_v1(&toks) }
    }

    fn entry_v2(&self, toks: &[&str]) -> Result<Entry, String> {
        const DEFAULT: [char; 7] = ['N', 'O', 'T', 'I', 'd', 'l', 'D'];
        let cols: &[char] = if self.columns.is_empty() { &DEFAULT } else { &self.columns };
        let col = |c: char| cols.iter().position(|x| *x == c).and_then(|i| toks.get(i).copied());
        let kind = col('T').ok_or("no message type")?;
        let (fd, brs, esi, rtr) = match kind {
            "DT" => (false, false, false, false),
            "RR" => (false, false, false, true),
            "FD" => (true, false, false, false),
            "FB" => (true, true, false, false),
            "FE" => (true, false, true, false),
            "BI" => (true, true, true, false),
            "ER" => return Ok(Entry::ErrorFrame),
            _ => return Ok(Entry::Skip), // ST, EC, EV: status, error counters, events
        };
        let off = col('O').ok_or("no time offset")?;
        let ts = Some(self.start_us + ms_to_us(off).ok_or_else(|| format!("bad time offset \"{off}\""))?);
        let id_s = col('I').ok_or("no identifier")?;
        let (id, _) = parse_id(id_s, 16)?;
        let extended = id_s.len() > 4 || id > 0x7FF;
        let dir = if col('d') == Some("Tx") { Direction::Tx } else { Direction::Rx };
        let len = if let Some(l) = col('l') {
            l.parse::<usize>().map_err(|_| format!("bad length \"{l}\""))?
        } else if let Some(dl) = col('L') {
            let dlc: u8 = dl.parse().map_err(|_| format!("bad DLC \"{dl}\""))?;
            if fd { crate::frame::dlc_to_len(dlc) } else { (dlc as usize).min(8) }
        } else {
            return Err("no length column".into());
        };
        let frame = if rtr {
            CanFrame::remote(id, extended, len.min(8) as u8)
        } else {
            let start = cols.iter().position(|c| *c == 'D').ok_or("no data column")?;
            let data = hex_tokens(&toks[start.min(toks.len())..], len)?;
            if fd { fd_frame(id, extended, brs, esi, &data) } else { CanFrame::with_ext(id, extended, &data) }
        }
        .map_err(|e| e.to_string())?;
        Ok(Entry::Frame { ts, iface: None, dir, frame })
    }

    /// 1.0: `1) 1841 0123 8 data`; 1.1: `1) 1841.0 Rx 0123 8 data`; 1.2/1.3: `1) 1841.0 1 Rx 0123 [-] 8 data`.
    fn entry_v1(&self, toks: &[&str]) -> Result<Entry, String> {
        if toks.len() < 3 || !toks[0].ends_with(')') {
            return Err("expected \"N) time ...\"".into());
        }
        let ts = Some(self.start_us + ms_to_us(toks[1]).ok_or_else(|| format!("bad time offset \"{}\"", toks[1]))?);
        let mut i = 2;
        if toks[i].chars().all(|c| c.is_ascii_digit()) && toks.get(i + 1).is_some_and(|t| t.chars().all(|c| c.is_ascii_alphabetic())) {
            i += 1; // bus
        }
        let mut dir = Direction::Rx;
        if toks[i].chars().all(|c| c.is_ascii_alphabetic()) {
            match toks[i] {
                "Rx" => {}
                "Tx" => dir = Direction::Tx,
                "Error" => return Ok(Entry::ErrorFrame),
                _ => return Ok(Entry::Skip), // Warng, Overrun and other events
            }
            i += 1;
        }
        let id_s = toks.get(i).ok_or("no identifier")?;
        if *id_s == "FFFFFFFF" {
            return Ok(Entry::Skip); // bus status line
        }
        let (id, _) = parse_id(id_s, 16)?;
        let extended = id_s.len() > 4 || id > 0x7FF;
        i += 1;
        if toks.get(i) == Some(&"-") {
            i += 1; // reserved
        }
        let dlc: usize = toks.get(i).and_then(|t| t.parse().ok()).ok_or("bad DLC")?;
        let rest = &toks[(i + 1).min(toks.len())..];
        let frame = if rest.first() == Some(&"RTR") {
            CanFrame::remote(id, extended, dlc.min(8) as u8)
        } else {
            CanFrame::with_ext(id, extended, &hex_tokens(rest, dlc.min(8))?)
        }
        .map_err(|e| e.to_string())?;
        Ok(Entry::Frame { ts, iface: None, dir, frame })
    }
}

// ── CSV ──

/// Column positions of a CSV trace, from its header.
struct CsvLayout {
    time: usize,
    id: usize,
    data: usize,
    ext: Option<usize>,
    rtr: Option<usize>,
    fd: Option<usize>,
    brs: Option<usize>,
    esi: Option<usize>,
    dlc: Option<usize>,
    dir: Option<usize>,
    error: Option<usize>,
    /// python-can writes the data base64-encoded.
    base64: bool,
}

impl CsvLayout {
    fn from_header(line: &str) -> Option<CsvLayout> {
        let cols: Vec<String> = csv_fields(line).into_iter().map(|c| c.trim().to_ascii_lowercase()).collect();
        let find = |names: &[&str]| cols.iter().position(|c| names.contains(&c.as_str()));
        let time = find(&["time_s", "timestamp", "time"])?;
        let id = find(&["id", "arbitration_id"])?;
        let data = find(&["data"])?;
        Some(CsvLayout {
            time,
            id,
            data,
            ext: find(&["extended", "is_extended_id"]),
            rtr: find(&["rtr", "remote", "is_remote_frame"]),
            fd: find(&["fd", "is_fd"]),
            brs: find(&["brs", "bitrate_switch"]),
            esi: find(&["esi", "error_state_indicator"]),
            dlc: find(&["dlc"]),
            dir: find(&["direction", "dir"]),
            error: find(&["error", "is_error_frame"]),
            base64: cols[id] == "arbitration_id",
        })
    }

    fn entry(&self, line: &str) -> Result<Entry, String> {
        let f = csv_fields(line);
        let get = |i: usize| f.get(i).map(|s| s.trim()).unwrap_or("");
        let flag = |i: Option<usize>| i.is_some_and(|i| matches!(get(i).to_ascii_lowercase().as_str(), "1" | "true"));
        if f.len() <= self.id.max(self.time) {
            return Err("too few columns".into());
        }
        if flag(self.error) {
            return Ok(Entry::ErrorFrame);
        }
        let ts = Some(secs_to_us(get(self.time)).ok_or_else(|| format!("bad time \"{}\"", get(self.time)))?);
        let (id, _) = parse_id(get(self.id), 16)?;
        let extended = match self.ext {
            Some(i) => flag(Some(i)),
            None => id > 0x7FF,
        };
        let dir = if self.dir.is_some_and(|i| get(i).eq_ignore_ascii_case("tx")) { Direction::Tx } else { Direction::Rx };
        let raw = get(self.data);
        let data = if self.base64 { base64_decode(raw).ok_or("data is not base64")? } else { parse_hex_bytes(raw).map_err(|e| e.to_string())? };
        let dlc: Option<u8> = self.dlc.and_then(|i| get(i).parse().ok());
        let frame = if flag(self.rtr) {
            CanFrame::remote(id, extended, dlc.unwrap_or(0).min(8))
        } else if flag(self.fd) || data.len() > 8 {
            fd_frame(id, extended, flag(self.brs), flag(self.esi), &data)
        } else {
            CanFrame::with_ext(id, extended, &data)
        }
        .map_err(|e| e.to_string())?;
        Ok(Entry::Frame { ts, iface: None, dir, frame })
    }
}

/// Splits one CSV line, with `"quoted, fields"` and `""` escapes.
fn csv_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if cur.is_empty() => quoted = true,
            (',', false) => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

pub const CSV_HEADER: &str = "time_s,direction,id,extended,fd,brs,rtr,dlc,data";

/// One CSV row (no newline). `t0_us` is subtracted from the time.
pub fn csv_row(f: &TimedFrame, t0_us: u64) -> String {
    let t = f.ts_us.saturating_sub(t0_us) as f64 / 1e6;
    format!(
        "{t:.6},{},{},{},{},{},{},{},{}",
        match f.dir {
            Direction::Rx => "rx",
            Direction::Tx => "tx",
        },
        f.frame.id_text(),
        f.frame.extended as u8,
        f.frame.fd as u8,
        f.frame.brs as u8,
        f.frame.rtr as u8,
        f.frame.dlc(),
        hex_bytes(&f.frame.data)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_syntax() {
        assert_eq!(parse_frame("123#DEADBEEF").unwrap(), CanFrame::new(0x123, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap());
        let e = parse_frame("00000123#11.22").unwrap();
        assert!(e.extended && e.id == 0x123 && e.data == vec![0x11, 0x22]);
        assert_eq!(parse_frame("7DF#R").unwrap(), CanFrame::remote(0x7DF, false, 0).unwrap());
        assert_eq!(parse_frame("7DF#R8").unwrap().rtr_len, 8);
        let fd = parse_frame("123##1AABB").unwrap();
        assert!(fd.fd && fd.brs && fd.data == vec![0xAA, 0xBB]);
        assert!(parse_frame("123").is_err());
        assert!(parse_frame("123#123").is_err());
        assert!(parse_frame("800#00").is_err());
        assert!(parse_frame("123456789#00").is_err());
        let esi = parse_frame("123##3").unwrap();
        assert!(esi.fd && esi.brs && esi.esi && esi.data.is_empty());
        assert!(!parse_frame("123##1").unwrap().esi);
        assert!(parse_frame("123##").is_err());
        assert!(parse_frame("123##1AABBCCDDEEFF001122").is_err(), "9 bytes is not an FD length");
        for f in ["123#DEADBEEF", "12345678#R", "123#R3", "123##0", "001##1", "123##2", "1ABCDEF0##3"] {
            assert_eq!(candump_frame(&parse_frame(f).unwrap()), f);
        }
    }

    #[test]
    fn candump_round_trip() {
        let frames = [TimedFrame { ts_us: 1_699_999_999_123_456, dir: Direction::Rx, frame: CanFrame::new(0x123, &[1, 2]).unwrap() },
            TimedFrame { ts_us: 1_700_000_000_000_001, dir: Direction::Tx, frame: CanFrame::fd(0x1234, true, true, &[0; 12]).unwrap() }];
        let text: String = frames.iter().map(|f| candump_line(f, "can0") + "\n").collect();
        assert!(text.starts_with("(1699999999.123456) can0 123#0102\n"));
        let back = parse_candump(&format!("# comment\n\n{text}")).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].ts_us, frames[0].ts_us);
        assert_eq!(back[1].frame, frames[1].frame);
        assert_eq!(back[1].iface, "can0");
    }

    #[test]
    fn candump_errors_carry_lines() {
        let e = parse_candump("(1.0) can0 123#00\nbad line\n").unwrap_err();
        assert!(matches!(e, CanError::Log { line: 2, .. }));
        assert_eq!(parse_candump("(1.5) vcan0 1#").unwrap()[0].ts_us, 1_500_000);
        assert!(parse_candump("(x.1) can0 1#").is_err());
    }

    #[test]
    fn candump_fd_64_bytes() {
        // A line as the yocto image's can-demo telemetry shows up in `candump -l` on the host.
        let data: String = (0..64).map(|i| format!("{:02X}", i as u8)).collect();
        let line = format!("(1762851234.000100) vcan0 201##1{data}");
        let r = &parse_candump(&line).unwrap()[0];
        assert!(r.frame.fd && r.frame.brs && !r.frame.esi);
        assert_eq!(r.frame.data.len(), 64);
        assert_eq!(r.frame.dlc(), 15);
        assert_eq!(candump_line(&TimedFrame { ts_us: r.ts_us, dir: Direction::Rx, frame: r.frame.clone() }, "vcan0"), line);
        let row = csv_row(&TimedFrame { ts_us: r.ts_us, dir: Direction::Rx, frame: r.frame.clone() }, r.ts_us);
        assert!(row.starts_with("0.000000,rx,201,0,1,1,0,15,00 01 02 "), "{row}");
        assert!(row.ends_with(" 3E 3F"));
    }

    #[test]
    fn csv() {
        let f = TimedFrame { ts_us: 2_500_000, dir: Direction::Tx, frame: CanFrame::new(0x7E8, &[0x41, 0x0C]).unwrap() };
        assert_eq!(csv_row(&f, 1_000_000), "1.500000,tx,7E8,0,0,0,0,2,41 0C");
    }

    /// `(time, id, ext, rtr dlc, fd, brs, esi, tx, data)` of each record, for comparing with what python-can reads.
    fn summary(log: &ParsedLog) -> Vec<(u64, u32, bool, Option<u8>, bool, bool, bool, bool, Vec<u8>)> {
        log.records
            .iter()
            .map(|r| {
                let f = &r.frame;
                (r.ts_us, f.id, f.extended, f.rtr.then_some(f.rtr_len), f.fd, f.brs, f.esi, r.dir == Direction::Tx, f.data.clone())
            })
            .collect()
    }

    /// The six frames `tests/fixtures/logs/pycan.*` were written from with python-can 4.6.1 (see the test below):
    /// classic, 29-bit sent, remote (DLC 8), FD 64 bytes with BRS, FD 12 bytes with ESI, empty.
    fn pycan_frames(t0: u64) -> Vec<(u64, u32, bool, Option<u8>, bool, bool, bool, bool, Vec<u8>)> {
        vec![
            (t0, 0x123, false, None, false, false, false, false, vec![1, 2, 3]),
            (t0 + 10_000, 0x18FE_F100, true, None, false, false, false, true, (0..8).collect()),
            (t0 + 20_000, 0x7DF, false, Some(8), false, false, false, false, vec![]),
            (t0 + 30_000, 0x4A0, false, None, true, true, false, false, (0..64).collect()),
            (t0 + 40_000, 0x4A1, false, None, true, false, true, false, (0..12).collect()),
            (t0 + 50_000, 0x001, false, None, false, false, false, false, vec![]),
        ]
    }

    #[test]
    fn reads_vector_asc() {
        let log = parse_log(include_str!("../tests/fixtures/logs/pycan.asc")).unwrap();
        assert_eq!(log.format, LogFormat::Asc);
        // Times count from "Begin Triggerblock Wed Nov 15 05:13:20.0 2023" (read as UTC).
        assert_eq!(summary(&log), pycan_frames(1_700_025_200_000_000));
        assert_eq!((log.bad_lines, log.error_frames), (0, 0));
        let dec = "date Mon Jan 1 10:00:00.000 am 2024\nbase dec  timestamps relative\n0.5 1 291 Rx d 2 01 02\n0.25 1 291 Tx d 1 03\n0.1 1 ErrorFrame\n0.1 1 Statistic: D 0 R 0\n";
        let log = parse_log(dec).unwrap();
        let t0 = 1_704_103_200_000_000;
        assert_eq!(log.records.iter().map(|r| (r.ts_us - t0, r.frame.id, r.dir)).collect::<Vec<_>>(), vec![(500_000, 0x123, Direction::Rx), (750_000, 0x123, Direction::Tx)]);
        assert_eq!(log.error_frames, 1);
    }

    #[test]
    fn reads_pcan_trc() {
        // python-can 4.6.1 reads these files with the same frames (it does not write FD or remote frames to TRC, so
        // the FD file is hand-written after PEAK's TRC 2.1 specification).
        let t0 = 1_700_000_000_000_100;
        let fd = parse_log(include_str!("../tests/fixtures/logs/fd21.trc")).unwrap();
        assert_eq!(fd.format, LogFormat::Trc);
        let want: Vec<_> = pycan_frames(t0 + 100).into_iter().collect();
        assert_eq!(summary(&fd), want);
        assert_eq!(fd.error_frames, 1);
        let v11 = parse_log(include_str!("../tests/fixtures/logs/v11.trc")).unwrap();
        let want: Vec<_> = pycan_frames(t0 + 100).into_iter().filter(|f| !f.4).collect();
        assert_eq!(summary(&v11), want);
        let py = parse_log(include_str!("../tests/fixtures/logs/pycan.trc")).unwrap();
        assert_eq!(summary(&py), pycan_frames(1_700_000_000_000_100).into_iter().filter(|f| !f.4 && f.3.is_none()).collect::<Vec<_>>());
    }

    #[test]
    fn reads_csv() {
        let py = parse_log(include_str!("../tests/fixtures/logs/pycan.csv")).unwrap();
        assert_eq!(py.format, LogFormat::Csv);
        // python-can's CSV has no direction, BRS or ESI columns.
        let want: Vec<_> = pycan_frames(1_700_000_000_000_100).into_iter().map(|mut f| {
            f.5 = false;
            f.6 = false;
            f.7 = false;
            f
        }).collect();
        assert_eq!(summary(&py), want);
        // Our own export (the app adds message, signals and label columns, quoted when they hold commas).
        let ours = "time_s,direction,id,extended,fd,brs,rtr,dlc,data,message,signals,label\n\
            0.000000,rx,0CF00400,1,0,0,0,8,F0 7D 7D 40 1F 00 F0 7D,EEC1,\"EngineSpeed=1000 rpm, Torque=5 %\",PGN 61444\n\
            0.010000,tx,7DF,0,0,0,1,8,,,,\n\
            0.020000,rx,4A0,0,1,1,0,9,00 01 02 03 04 05 06 07 08 09 0A 0B,,,\n";
        let log = parse_log(ours).unwrap();
        assert_eq!(summary(&log), vec![
            (0, 0x0CF0_0400, true, None, false, false, false, false, vec![0xF0, 0x7D, 0x7D, 0x40, 0x1F, 0x00, 0xF0, 0x7D]),
            (10_000, 0x7DF, false, Some(8), false, false, false, true, vec![]),
            (20_000, 0x4A0, false, None, true, true, false, false, (0..12).collect()),
        ]);
        // And the rows csv_row writes.
        let f = TimedFrame { ts_us: 2_500_000, dir: Direction::Tx, frame: CanFrame::new(0x7E8, &[0x41, 0x0C]).unwrap() };
        let log = parse_log(&format!("{CSV_HEADER}\n{}\n", csv_row(&f, 0))).unwrap();
        assert_eq!((log.records[0].ts_us, log.records[0].dir, &log.records[0].frame), (2_500_000, Direction::Tx, &f.frame));
    }

    #[test]
    fn reads_candump_screen_output() {
        let plain = "  can0  123   [8]  11 22 33 44 55 66 77 88\n  can0  18FEF100   [3]  01 02 03\n  can0  7DF   [8]  remote request\n  can0  4A0  [12]  00 01 02 03 04 05 06 07 08 09 0A 0B\n  can0  4A1  [08]  01 02 03 04 05 06 07 08\n";
        let log = parse_log(plain).unwrap();
        assert_eq!(log.format, LogFormat::Candump);
        assert_eq!(summary(&log), vec![
            (0, 0x123, false, None, false, false, false, false, vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]),
            (1000, 0x18FE_F100, true, None, false, false, false, false, vec![1, 2, 3]),
            (2000, 0x7DF, false, Some(8), false, false, false, false, vec![]),
            (3000, 0x4A0, false, None, true, false, false, false, (0..12).collect()),
            (4000, 0x4A1, false, None, true, false, false, false, (1..9).collect()),
        ]);
        assert_eq!(log.synthesized_times, 5);
        assert!(log.notes().iter().any(|n| n.contains("no times")), "{:?}", log.notes());

        // -ta and -tA times, -x direction and FD flags, -a ASCII column, error frames.
        let ta = " (1700000000.000100)  can0  123   [8]  11 22 33 44 55 66 77 88   '.\"3DUfw.'\n \
            (2023-11-14 22:13:20.000200)  can0  456   [2]  01 02\n \
            (1700000000.000300)  can0  TX B E  4A0  [12]  00 01 02 03 04 05 06 07 08 09 0A 0B\n \
            (1700000000.000400)  can0  RX - -  123   [1]  09\n \
            (1700000000.000500)  can0  20000080   [8]  00 00 00 00 00 00 00 00   ERRORFRAME\n";
        let log = parse_log(ta).unwrap();
        assert_eq!(log.records.iter().map(|r| r.ts_us).collect::<Vec<_>>(), vec![1_700_000_000_000_100, 1_700_000_000_000_200, 1_700_000_000_000_300, 1_700_000_000_000_400]);
        let fd = &log.records[2];
        assert!(fd.dir == Direction::Tx && fd.frame.fd && fd.frame.brs && fd.frame.esi);
        assert_eq!(log.records[3].dir, Direction::Rx);
        assert_eq!(log.error_frames, 1);
        assert_eq!(log.synthesized_times, 0);
    }

    #[test]
    fn forgiving_candump_logs() {
        // -x direction suffix, ; comments, BOM, CRLF, NUL padding, raw DLC, comma decimal point.
        let text = "\u{FEFF}; made by hand\r\n(1700000000.000100) can0 123#0102 T\r\n(1700000000.001100) can0 456#0304 R\r\n\
            (1700000000.002100) can0 789#1122334455667788_9\0\n(1700000000,003100) can0 001#\n";
        let log = parse_log(text).unwrap();
        assert_eq!(log.records.len(), 4, "{:?}", log.notes());
        assert_eq!((log.records[0].dir, log.records[1].dir), (Direction::Tx, Direction::Rx));
        assert_eq!(log.records[2].frame.data.len(), 8);
        assert_eq!(log.records[3].ts_us, 1_700_000_000_003_100);
        assert!(log.notes().is_empty(), "{:?}", log.notes());

        // Lines without times get one 1 ms after the frame before; the interface name is optional.
        let log = parse_log("can0 123#0102\n456#0304\n").unwrap();
        assert_eq!(log.records.iter().map(|r| (r.ts_us, r.iface.as_str())).collect::<Vec<_>>(), vec![(0, "can0"), (1000, "")]);

        // Bad lines are skipped and counted, with their numbers; a cut-off last line is noted separately.
        let mut text = String::from("(1.000000) can0 123#01\n");
        for i in 0..7 {
            text.push_str(&format!("(-1.0) can0 123#0{i}\n"));
        }
        text.push_str("(99999999999999.000000) can0 123#0102\n(1.5) can0 124#03\n(1.6) can0 124#0");
        let log = parse_log(&text).unwrap();
        assert_eq!(log.records.len(), 2);
        assert_eq!(log.bad_lines, 8);
        assert_eq!(log.bad_examples.iter().map(|b| b.0).collect::<Vec<_>>(), vec![2, 3, 4, 5, 6]);
        assert!(log.truncated_last_line);
        let notes = log.notes().join(" ");
        assert!(notes.contains("8 lines could not be read") && notes.contains("lines 2, 3, 4, 5, 6, ...") && notes.contains("cut off"), "{notes}");

        // A cut-off last line that still reads as a frame is kept (there is no way to tell).
        assert_eq!(parse_log("(1.0) can0 123#0102\n(1.1) can0 124#03").unwrap().records.len(), 2);
        // Nothing readable: an error that says why.
        let e = parse_log("hello\nworld\n").unwrap_err().to_string();
        assert!(e.contains("No frames") && e.contains("line 1"), "{e}");
        assert!(parse_log("").is_err());
        assert!(parse_log("# only a comment\n").is_err());
        // Strict reading for the other users of the candump reader is unchanged.
        assert!(parse_candump("(1.0) can0 123#00\n(x) can0 1#\n").is_err());
        assert_eq!(parse_candump("(1.0) can0 123#00 T\n").unwrap()[0].dir, Direction::Tx);
    }

    #[test]
    fn python_can_candump_log() {
        let log = parse_log(include_str!("../tests/fixtures/logs/pycan.log")).unwrap();
        let mut want = pycan_frames(1_700_000_000_000_100);
        want[2].3 = Some(0); // python-can writes "7DF#R" without the length
        assert_eq!(summary(&log), want);
    }

    #[test]
    fn raw_dlc_is_not_sent() {
        let e = parse_frame("123#1122334455667788_9").unwrap_err().to_string();
        assert!(e.contains("raw DLC"), "{e}");
    }
}
