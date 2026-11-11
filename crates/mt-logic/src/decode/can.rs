//! Classic CAN at bit level, from the logic side of a transceiver (CAN_RX or CAN_TX; dominant =
//! low unless inverted): hard sync on start of frame, resynchronisation on every recessive to
//! dominant edge, bit destuffing, 11- and 29-bit identifiers, remote frames, DLC, data, CRC-15,
//! ACK slot, end of frame, stuff errors and error flags. CAN FD frames are recognised by their FDF
//! bit and reported, not decoded.

use serde::{Deserialize, Serialize};

use super::{Annotation, DecodeOutput, Level, Table, TableRow, hex, time_cell};
use crate::capture::Capture;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanConfig {
    pub channel: u8,
    pub bitrate: f64,
    /// Sample point as a fraction of the bit, 0.5 to 0.95.
    #[serde(default = "sp")]
    pub sample_point: f64,
    /// Dominant is high (a non-inverting probe on CAN_H through a comparator, for instance).
    #[serde(default)]
    pub inverted: bool,
    /// A DBC file whose message and signal names label the frames (CAN Explorer link), or
    /// `builtin:can-explorer-sim` for CAN Explorer's simulator DBC. Read by the app, not here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dbc: Option<String>,
    /// A candump log of the same bus (recorded by CAN Explorer) to compare the frames with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    /// How far apart a wire frame and its log record may be once the clocks are aligned.
    #[serde(default = "log_window")]
    pub log_window_ms: f64,
}

fn log_window() -> f64 {
    2.0
}

fn sp() -> f64 {
    0.75
}

impl Default for CanConfig {
    fn default() -> Self {
        Self { channel: 0, bitrate: 500_000.0, sample_point: 0.75, inverted: false, dbc: None, log: None, log_window_ms: log_window() }
    }
}

impl CanConfig {
    pub fn validate(&self, cap: &Capture) -> Result<(), String> {
        if !(0.5..=0.95).contains(&self.sample_point) {
            return Err("the CAN sample point must be between 50 % and 95 %".into());
        }
        if self.bitrate <= 0.0 {
            return Err("the bit rate must be above zero".into());
        }
        if (cap.sample_rate as f64) / self.bitrate < 8.0 {
            return Err(format!(
                "CAN at {} kbit/s needs at least {} (8 samples per bit); this capture has {}",
                self.bitrate / 1000.0,
                crate::fmt_rate((self.bitrate * 8.0) as u64),
                crate::fmt_rate(cap.sample_rate)
            ));
        }
        Ok(())
    }
}

/// CRC-15/CAN over a bit sequence.
pub fn crc15(bits: &[bool]) -> u16 {
    let mut crc = 0u16;
    for &b in bits {
        let nxt = b ^ (crc >> 14 & 1 == 1);
        crc = (crc << 1) & 0x7FFF;
        if nxt {
            crc ^= 0x4599;
        }
    }
    crc
}

/// What to put on the wire, for the synthesizer and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanFrameSpec {
    pub id: u32,
    pub extended: bool,
    pub rtr: bool,
    pub dlc: u8,
    pub data: Vec<u8>,
    /// Some node acknowledges (drives the ACK slot dominant).
    pub ack: bool,
    /// Flip one CRC bit.
    pub corrupt_crc: bool,
}

impl CanFrameSpec {
    pub fn data(id: u32, extended: bool, data: &[u8]) -> Self {
        Self { id, extended, rtr: false, dlc: data.len() as u8, data: data.to_vec(), ack: true, corrupt_crc: false }
    }
}

fn push_bits(v: &mut Vec<bool>, value: u32, n: u32) {
    for i in (0..n).rev() {
        v.push(value >> i & 1 == 1);
    }
}

/// The unstuffed bits from SOF to the end of the CRC sequence.
pub fn unstuffed_bits(f: &CanFrameSpec) -> Vec<bool> {
    let mut v = vec![false];
    if f.extended {
        push_bits(&mut v, f.id >> 18, 11);
        v.push(true); // SRR
        v.push(true); // IDE
        push_bits(&mut v, f.id & 0x3FFFF, 18);
        v.push(f.rtr);
        v.push(false); // r1 (FDF)
        v.push(false); // r0
    } else {
        push_bits(&mut v, f.id, 11);
        v.push(f.rtr);
        v.push(false); // IDE
        v.push(false); // r0 (FDF)
    }
    push_bits(&mut v, f.dlc as u32, 4);
    if !f.rtr {
        for &b in f.data.iter().take(f.dlc.min(8) as usize) {
            push_bits(&mut v, b as u32, 8);
        }
    }
    let mut crc = crc15(&v);
    if f.corrupt_crc {
        crc ^= 0x0010;
    }
    push_bits(&mut v, crc as u32, 15);
    v
}

/// Inserts a stuff bit after every 5 equal bits.
pub fn stuff(bits: &[bool]) -> Vec<bool> {
    let mut out = Vec::with_capacity(bits.len() + bits.len() / 4);
    let (mut last, mut run) = (None::<bool>, 0);
    for &b in bits {
        if Some(b) == last {
            run += 1;
        } else {
            last = Some(b);
            run = 1;
        }
        out.push(b);
        if run == 5 {
            out.push(!b);
            last = Some(!b);
            run = 1;
        }
    }
    out
}

/// The whole frame on the wire (recessive = true), SOF to the end of EOF.
pub fn frame_bits(f: &CanFrameSpec) -> Vec<bool> {
    let mut v = stuff(&unstuffed_bits(f));
    v.push(true); // CRC delimiter
    v.push(!f.ack); // ACK slot
    v.push(true); // ACK delimiter
    v.extend([true; 7]);
    v
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanBitFrame {
    pub start: u64,
    pub end: u64,
    pub id: u32,
    pub extended: bool,
    pub rtr: bool,
    pub dlc: u8,
    pub data: Vec<u8>,
    pub crc: u16,
    pub crc_ok: bool,
    pub acked: bool,
    /// Stuff error, form error, error flag, CAN FD; `None` for a good frame.
    pub error: Option<String>,
    pub stuff_bits: u32,
    /// The frame ended in an error flag (6 or more dominant bits).
    pub error_frame: bool,
    /// Identifier, format and DLC were read before any error.
    pub header_ok: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CanResult {
    pub frames: Vec<CanBitFrame>,
    pub annotations: Vec<Annotation>,
}

struct Reader<'a> {
    cap: &'a Capture,
    ch: u8,
    dom_level: bool,
    bit: f64,
    sp: f64,
    sync: f64,
    k_sync: i64,
    last_sample: u64,
}

impl Reader<'_> {
    fn dominant(&self, i: u64) -> bool {
        self.cap.level(self.ch, i) == self.dom_level
    }

    fn next_dominant_edge(&self, from: u64) -> Option<u64> {
        let mut p = from;
        loop {
            let e = self.cap.next_edge(self.ch, p)?;
            if self.dominant(e) {
                return Some(e);
            }
            p = e;
        }
    }

    /// Start, sample point and value (true = recessive) of raw bit `k`.
    fn read(&mut self, k: i64) -> Option<(u64, u64, bool)> {
        let mut at = self.sync + (k - self.k_sync) as f64 * self.bit + self.sp * self.bit;
        if k > 0
            && let Some(e) = self.next_dominant_edge(self.last_sample)
            && (e as f64) < at
        {
            // Resynchronise on the edge: it starts the bit nearest to it.
            let kk = self.k_sync + ((e as f64 - self.sync) / self.bit).round() as i64;
            self.sync = e as f64;
            self.k_sync = kk;
            at = self.sync + (k - self.k_sync) as f64 * self.bit + self.sp * self.bit;
        }
        let s = at.round() as u64;
        if s >= self.cap.len() {
            return None;
        }
        self.last_sample = s;
        let start = (self.sync + (k - self.k_sync) as f64 * self.bit).round().max(0.0) as u64;
        Some((start, s, !self.dominant(s)))
    }

    fn bit_end(&self, k: i64) -> u64 {
        (self.sync + (k + 1 - self.k_sync) as f64 * self.bit).round() as u64
    }
}

// Annotation rows.
const ROW_BITS: u8 = 0;
const ROW_FIELDS: u8 = 1;
const ROW_FRAMES: u8 = 2;
const ROW_ERRORS: u8 = 3;

pub fn decode(cap: &Capture, cfg: &CanConfig) -> CanResult {
    let mut res = CanResult::default();
    let bit = cap.sample_rate as f64 / cfg.bitrate;
    let dom_level = cfg.inverted;
    let idle_min = (10.5 * bit) as u64;
    let mut pos = 0u64;
    let mut r = Reader { cap, ch: cfg.channel, dom_level, bit, sp: cfg.sample_point, sync: 0.0, k_sync: 0, last_sample: 0 };
    while let Some(sof) = r.next_dominant_edge(pos) {
        // Bus idle before SOF: at least 11 recessive bits (ACK delimiter, EOF and intermission).
        let rec_start = cap.prev_edge(cfg.channel, sof - 1).unwrap_or(0);
        if sof - rec_start < idle_min {
            pos = sof;
            continue;
        }
        r.sync = sof as f64;
        r.k_sync = 0;
        r.last_sample = sof;
        match read_frame(&mut r, &mut res.annotations) {
            Some((frame, end)) => {
                res.frames.push(frame);
                pos = end.max(sof + 1);
            }
            None => break,
        }
    }
    res
}

/// Reads one frame from SOF. Returns the frame and where to resume.
fn read_frame(r: &mut Reader, ann: &mut Vec<Annotation>) -> Option<(CanBitFrame, u64)> {
    let mut bits: Vec<bool> = Vec::new();
    let mut spans: Vec<(u64, u64)> = Vec::new();
    let mut k: i64 = 0;
    let (mut last, mut run) = (None::<bool>, 0u32);
    let mut stuff_bits = 0u32;
    let mut total: Option<usize> = None;
    let mut frame = CanBitFrame {
        start: r.sync as u64,
        end: 0,
        id: 0,
        extended: false,
        rtr: false,
        dlc: 0,
        data: vec![],
        crc: 0,
        crc_ok: false,
        acked: false,
        error: None,
        stuff_bits: 0,
        error_frame: false,
        header_ok: false,
    };
    let fail = |frame: &mut CanBitFrame, ann: &mut Vec<Annotation>, r: &Reader, at: u64, msg: String, flag: bool| {
        let end = if flag {
            // The error flag lasts until the line goes recessive.
            r.cap.next_edge(r.ch, at).unwrap_or(r.cap.len())
        } else {
            at
        };
        frame.end = end;
        frame.error = Some(msg.clone());
        frame.error_frame = flag;
        ann.push(Annotation::new(ROW_ERRORS, at.min(end.saturating_sub(1)), end, Level::Error, msg, if flag { "ERR" } else { "!" }));
        end
    };

    // Stuffed part: SOF to the end of the CRC sequence.
    while total.is_none_or(|t| bits.len() < t) {
        let (s, _at, b) = r.read(k)?;
        let e = r.bit_end(k);
        k += 1;
        if run == 5 {
            if Some(b) == last {
                let flag = !b;
                let msg = if flag { "Error frame (6 dominant bits)".to_string() } else { "Stuff error (6 recessive bits)".to_string() };
                let end = fail(&mut frame, ann, r, s, msg, flag);
                frame.stuff_bits = stuff_bits;
                return Some((frame, end));
            }
            stuff_bits += 1;
            ann.push(Annotation::new(ROW_BITS, s, e, Level::Info, "Stuff bit", "S"));
            last = Some(b);
            run = 1;
            continue;
        }
        if Some(b) == last {
            run += 1;
        } else {
            last = Some(b);
            run = 1;
        }
        bits.push(b);
        spans.push((s, e));
        ann.push(Annotation::new(ROW_BITS, s, e, Level::Data, if b { "1" } else { "0" }, if b { "1" } else { "0" }));
        let n = bits.len();
        if n == 1 && b {
            // Not a SOF after all (the edge was a glitch).
            return Some((frame_glitch(frame, s), e));
        }
        if total.is_none() {
            let ide = n > 13 && bits[13];
            let header = if ide { 39 } else { 19 };
            if n == header {
                let field = |a: usize, len: usize| bits[a..a + len].iter().fold(0u32, |v, &b| v << 1 | b as u32);
                let fdf = if ide { bits[33] } else { bits[14] };
                if fdf {
                    let end = skip_to_idle(r, e);
                    frame.end = end;
                    frame.error = Some("CAN FD frame (not decoded)".into());
                    ann.push(Annotation::new(ROW_ERRORS, frame.start, end, Level::Warning, "CAN FD frame (not decoded)", "FD"));
                    return Some((frame, end));
                }
                frame.extended = ide;
                frame.id = if ide { field(1, 11) << 18 | field(14, 18) } else { field(1, 11) };
                frame.rtr = if ide { bits[32] } else { bits[12] };
                frame.dlc = field(header - 4, 4) as u8;
                frame.header_ok = true;
                let data_bits = if frame.rtr { 0 } else { frame.dlc.min(8) as usize * 8 };
                total = Some(header + data_bits + 15);
                // Field annotations for the header.
                let f = |a: usize, b: usize| (spans[a].0, spans[b].1);
                let (s0, e0) = f(0, 0);
                ann.push(Annotation::new(ROW_FIELDS, s0, e0, Level::Control, "Start of frame", "SOF"));
                if ide {
                    let (a, b) = f(1, 11);
                    ann.push(Annotation::new(ROW_FIELDS, a, b, Level::Control, format!("Base ID 0x{:03X}", field(1, 11)), "ID"));
                    let (a, b) = f(12, 13);
                    ann.push(Annotation::new(ROW_FIELDS, a, b, Level::Control, "SRR, IDE", "SI"));
                    let (a, b) = f(14, 31);
                    ann.push(Annotation::new(ROW_FIELDS, a, b, Level::Control, format!("ID extension 0x{:05X}", field(14, 18)), "EID"));
                    let (a, b) = f(32, 32);
                    ann.push(Annotation::new(ROW_FIELDS, a, b, Level::Control, if frame.rtr { "RTR (remote)" } else { "RTR (data)" }, "R"));
                    let (a, b) = f(33, 34);
                    ann.push(Annotation::new(ROW_FIELDS, a, b, Level::Control, "r1, r0", "r"));
                } else {
                    let (a, b) = f(1, 11);
                    ann.push(Annotation::new(ROW_FIELDS, a, b, Level::Control, format!("ID 0x{:03X}", frame.id), "ID"));
                    let (a, b) = f(12, 12);
                    ann.push(Annotation::new(ROW_FIELDS, a, b, Level::Control, if frame.rtr { "RTR (remote)" } else { "RTR (data)" }, "R"));
                    let (a, b) = f(13, 14);
                    ann.push(Annotation::new(ROW_FIELDS, a, b, Level::Control, "IDE, r0", "I"));
                }
                let (a, b) = f(header - 4, header - 1);
                ann.push(Annotation::new(ROW_FIELDS, a, b, Level::Control, format!("DLC {}", frame.dlc), format!("{}", frame.dlc)));
            }
        } else if let Some(t) = total {
            let header = if frame.extended { 39 } else { 19 };
            let data_end = t - 15;
            if n > header && n <= data_end && (n - header).is_multiple_of(8) {
                let i = (n - header) / 8 - 1;
                let a = header + i * 8;
                let v = bits[a..a + 8].iter().fold(0u8, |v, &b| v << 1 | b as u8);
                frame.data.push(v);
                ann.push(Annotation::new(ROW_FIELDS, spans[a].0, spans[a + 7].1, Level::Data, format!("Data {i}: 0x{v:02X}"), format!("{v:02X}")));
            }
        }
    }
    let t = total.unwrap();
    let crc_rx = bits[t - 15..].iter().fold(0u16, |v, &b| v << 1 | b as u16);
    let crc_calc = crc15(&bits[..t - 15]);
    frame.crc = crc_rx;
    frame.crc_ok = crc_rx == crc_calc;
    frame.stuff_bits = stuff_bits;
    let (cs, ce) = (spans[t - 15].0, spans[t - 1].1);
    ann.push(Annotation::new(
        ROW_FIELDS,
        cs,
        ce,
        if frame.crc_ok { Level::Control } else { Level::Error },
        if frame.crc_ok { format!("CRC 0x{crc_rx:04X}") } else { format!("CRC 0x{crc_rx:04X} wrong (expected 0x{crc_calc:04X})") },
        "CRC",
    ));

    // Fixed form: CRC delimiter, ACK slot, ACK delimiter, 7 EOF bits.
    let (s, _, crc_delim) = r.read(k)?;
    let e = r.bit_end(k);
    k += 1;
    if !crc_delim {
        let end = fail(&mut frame, ann, r, s, "Form error: CRC delimiter dominant".into(), false);
        return Some((frame, skip_to_idle(r, end)));
    }
    ann.push(Annotation::new(ROW_FIELDS, s, e, Level::Control, "CRC delimiter", "D"));
    let (s, _, ack_slot) = r.read(k)?;
    let e = r.bit_end(k);
    k += 1;
    frame.acked = !ack_slot;
    ann.push(Annotation::new(ROW_FIELDS, s, e, if frame.acked { Level::Control } else { Level::Error }, if frame.acked { "ACK" } else { "No ACK" }, "A"));
    let (s, _, ack_delim) = r.read(k)?;
    let e = r.bit_end(k);
    k += 1;
    if !ack_delim {
        let end = fail(&mut frame, ann, r, s, "Form error: ACK delimiter dominant".into(), true);
        return Some((frame, end));
    }
    ann.push(Annotation::new(ROW_FIELDS, s, e, Level::Control, "ACK delimiter", "D"));
    let eof_start = r.bit_end(k - 1);
    for i in 0..7 {
        let (s, _, b) = r.read(k)?;
        k += 1;
        if !b {
            // A dominant bit in EOF: an error flag from a node that saw a problem (missing ACK,
            // CRC error), or an overload frame on the last bit.
            let msg = if !frame.crc_ok {
                "CRC error, error flag follows".to_string()
            } else if !frame.acked {
                "No ACK, error flag follows".to_string()
            } else if i == 6 {
                "Overload or error flag in the last EOF bit".to_string()
            } else {
                "Error flag in end of frame".to_string()
            };
            let end = fail(&mut frame, ann, r, s, msg, true);
            frame.end = end;
            return Some((frame, end));
        }
    }
    let end = r.bit_end(k - 1);
    ann.push(Annotation::new(ROW_FIELDS, eof_start, end, Level::Control, "End of frame", "EOF"));
    frame.end = end;
    if !frame.crc_ok {
        frame.error = Some("CRC error".into());
    } else if !frame.acked {
        frame.error = Some("No ACK".into());
    }
    Some((frame, end))
}

fn frame_glitch(mut f: CanBitFrame, at: u64) -> CanBitFrame {
    f.end = at;
    f.error = Some("Glitch".into());
    f
}

fn skip_to_idle(r: &Reader, from: u64) -> u64 {
    // Resume after the first dominant edge; the idle check in `decode` waits for 11 recessive bits.
    r.cap.next_edge(r.ch, from).unwrap_or(r.cap.len()).max(from)
}

pub fn frame_text(f: &CanBitFrame) -> String {
    let id = if f.extended { format!("0x{:08X}", f.id) } else { format!("0x{:03X}", f.id) };
    if f.rtr { format!("{id} RTR [{}]", f.dlc) } else { format!("{id} [{}] {}", f.dlc, hex(&f.data)) }
}

pub fn output(cap: &Capture, r: &CanResult) -> DecodeOutput {
    let frames: Vec<&CanBitFrame> = r.frames.iter().filter(|f| f.error.as_deref() != Some("Glitch")).collect();
    let mut ann = r.annotations.clone();
    for f in &frames {
        let lvl = match f.error.as_deref() {
            None => Level::Info,
            Some(e) if e.starts_with("No ACK") => Level::Warning,
            Some(e) if e.starts_with("CAN FD") => Level::Warning,
            Some(_) => Level::Error,
        };
        let text = match &f.error {
            Some(e) if !f.header_ok => e.clone(),
            Some(e) => format!("{} — {e}", frame_text(f)),
            None => frame_text(f),
        };
        ann.push(Annotation::new(ROW_FRAMES, f.start, f.end.max(f.start + 1), lvl, text, if f.extended { format!("{:X}", f.id) } else { format!("{:03X}", f.id) }));
    }
    ann.sort_by_key(|a| (a.row, a.start));
    let rows = frames
        .iter()
        .map(|f| TableRow {
            start: f.start,
            end: f.end,
            level: if f.error.is_none() { Level::Data } else if f.error.as_deref().is_some_and(|e| e.starts_with("No ACK")) { Level::Warning } else { Level::Error },
            cells: vec![
                time_cell(cap, f.start),
                if !f.header_ok {
                    "—".into()
                } else if f.extended {
                    format!("0x{:08X}", f.id)
                } else {
                    format!("0x{:03X}", f.id)
                },
                if f.extended { "29-bit".into() } else { "11-bit".into() },
                if f.rtr { format!("{} RTR", f.dlc) } else { f.dlc.to_string() },
                hex(&f.data),
                f.stuff_bits.to_string(),
                f.error.clone().unwrap_or_else(|| "OK".into()),
            ],
        })
        .collect();
    let errors = frames.iter().filter(|f| f.error.is_some()).count();
    DecodeOutput {
        rows: vec!["Bits".into(), "Fields".into(), "Frames".into(), "Errors".into()],
        annotations: ann,
        table: Some(Table { columns: ["Time", "ID", "Format", "DLC", "Data", "Stuff bits", "Status"].map(String::from).to_vec(), rows }),
        summary: format!("{} frames{}", frames.len(), if errors > 0 { format!(", {errors} with errors") } else { String::new() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{Synth, can_error_frame, can_tx};

    const RATE: u64 = 8_000_000;
    const BR: f64 = 500_000.0;

    fn bus(frames: &[CanFrameSpec]) -> Capture {
        let mut s = Synth::new(RATE, [true; 8]);
        let mut t = 0.0001;
        for f in frames {
            t = can_tx(&mut s, 0, t, BR, f) + 0.00005;
        }
        s.extend_to(s.at(t + 0.0001));
        s.capture(&["CAN_RX"], 0)
    }

    fn cfg() -> CanConfig {
        CanConfig { channel: 0, bitrate: BR, ..Default::default() }
    }

    #[test]
    fn stuffing_reference() {
        assert_eq!(stuff(&[false; 5]), vec![false, false, false, false, false, true]);
        assert_eq!(stuff(&[true, true, true, true, true, true]), vec![true, true, true, true, true, false, true]);
        // The stuff bit starts a new run: 5 zeros, stuff 1, then 4 ones and another 1 → stuff 0.
        assert_eq!(stuff(&[false, false, false, false, false, true, true, true, true]).len(), 11);
    }

    #[test]
    fn decodes_standard_extended_and_remote_frames() {
        let frames = [
            CanFrameSpec::data(0x123, false, &[0x11, 0x22, 0x33]),
            CanFrameSpec::data(0x0CF00400, true, &[0xF0, 0x7D, 0x7D, 0x00, 0x00, 0x00, 0xF0, 0xFF]),
            // Lots of equal bits: many stuff bits.
            CanFrameSpec::data(0x000, false, &[0x00; 8]),
            CanFrameSpec::data(0x7FF, false, &[0xFF; 8]),
            CanFrameSpec { rtr: true, dlc: 4, data: vec![], ..CanFrameSpec::data(0x18FEF100, true, &[]) },
            CanFrameSpec::data(0x080, false, &[]),
        ];
        let r = decode(&bus(&frames), &cfg());
        assert_eq!(r.frames.len(), frames.len());
        for (got, want) in r.frames.iter().zip(&frames) {
            assert_eq!(got.error, None, "{got:?}");
            assert_eq!((got.id, got.extended, got.rtr, got.dlc), (want.id, want.extended, want.rtr, want.dlc));
            assert_eq!(got.data, if want.rtr { vec![] } else { want.data.clone() });
            assert!(got.crc_ok && got.acked);
        }
        assert!(r.frames[2].stuff_bits >= 10);
    }

    #[test]
    fn crc_error_and_missing_ack() {
        let frames = [
            CanFrameSpec { corrupt_crc: true, ..CanFrameSpec::data(0x321, false, &[1, 2, 3, 4]) },
            CanFrameSpec { ack: false, ..CanFrameSpec::data(0x322, false, &[5]) },
            CanFrameSpec::data(0x323, false, &[6]),
        ];
        let r = decode(&bus(&frames), &cfg());
        assert_eq!(r.frames.len(), 3);
        assert!(!r.frames[0].crc_ok);
        assert_eq!(r.frames[0].error.as_deref(), Some("CRC error"));
        assert!(!r.frames[1].acked);
        assert_eq!(r.frames[1].error.as_deref(), Some("No ACK"));
        assert_eq!(r.frames[2].error, None);
        let out = output(&bus(&frames), &r);
        assert_eq!(out.table.unwrap().rows[1].cells[6], "No ACK");
    }

    #[test]
    fn error_frame_in_the_middle_of_a_frame() {
        let mut s = Synth::new(RATE, [true; 8]);
        let good = CanFrameSpec::data(0x100, false, &[0xAA, 0x55]);
        let mut t = can_tx(&mut s, 0, 0.0001, BR, &good) + 0.00003;
        // A frame cut by an error flag after 20 bits.
        let bits = frame_bits(&CanFrameSpec::data(0x101, false, &[1, 2, 3]));
        t = s.bits(0, t, 1.0 / BR, &bits[..20]);
        t = can_error_frame(&mut s, 0, t, BR);
        t = can_tx(&mut s, 0, t + 0.00001, BR, &CanFrameSpec::data(0x102, false, &[9])) + 0.0001;
        s.extend_to(s.at(t));
        let r = decode(&s.capture(&[], 0), &cfg());
        let ids: Vec<_> = r.frames.iter().map(|f| (f.id, f.error.is_some())).collect();
        assert_eq!(r.frames.len(), 3, "{ids:?}");
        assert_eq!(r.frames[0].error, None);
        assert!(r.frames[1].error_frame);
        assert_eq!(r.frames[2].id, 0x102);
        assert_eq!(r.frames[2].error, None);
    }

    #[test]
    fn tolerates_clock_mismatch_and_glitches() {
        // The transmitter runs 1.5 % fast; resynchronisation keeps the decoder on the bits.
        let mut s = Synth::new(RATE, [true; 8]);
        let f = CanFrameSpec::data(0x1ABCDE12, true, &[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]);
        let t = can_tx(&mut s, 0, 0.0001, BR * 1.015, &f);
        // A 1-sample glitch on the idle bus first: it must not start a frame.
        s.glitch(0, s.at(0.00005), 1);
        s.extend_to(s.at(t + 0.0001));
        let r = decode(&s.capture(&[], 0), &cfg());
        let good: Vec<_> = r.frames.iter().filter(|f| f.error.is_none()).collect();
        assert_eq!(good.len(), 1);
        assert_eq!(good[0].data, f.data);
    }

    #[test]
    fn wrong_bitrate_gives_errors() {
        let frames = [CanFrameSpec::data(0x123, false, &[0x11, 0x22, 0x33]), CanFrameSpec::data(0x456, false, &[0x44])];
        let r = decode(&bus(&frames), &CanConfig { bitrate: 250_000.0, ..cfg() });
        assert!(r.frames.iter().all(|f| f.error.is_some()));
    }

    #[test]
    fn crc15_reference() {
        // Standard ID 0x000, DLC 0: CRC from a known CAN trace (bit sequence SOF+ID+RTR+IDE+r0+DLC all zero).
        let bits = vec![false; 19];
        assert_eq!(crc15(&bits), 0);
        let f = CanFrameSpec::data(0x7FF, false, &[]);
        let u = unstuffed_bits(&f);
        assert_eq!(crc15(&u), 0, "a sequence followed by its own CRC has remainder 0");
    }
}
