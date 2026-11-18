//! What a frame means under the higher-layer protocols seen on industrial and vehicle buses:
//! SAE J1939 (29-bit, trucks, gensets, off-highway), CANopen (11-bit, CiA 301 predefined
//! connection set) and OBD-II (ISO 15765-4, 11- and 29-bit).
//!
//! [`describe`] labels one frame on its own. [`Decoder`] keeps a little state between frames so it
//! can also reassemble ISO-TP messages (a VIN, a DTC list) and J1939 transport sessions (BAM and
//! RTS/CTS, such as a DM1 with several DTCs) and show the whole message on the frame that completes it.

use std::collections::HashMap;

use serde::Serialize;

use crate::frame::CanFrame;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct J1939Id {
    pub priority: u8,
    pub pgn: u32,
    pub source: u8,
    /// Destination for PDU1 (peer-to-peer) PGNs; `None` for broadcast PDU2.
    pub destination: Option<u8>,
}

/// Splits a 29-bit identifier into J1939 fields.
pub fn j1939(id: u32) -> J1939Id {
    let priority = ((id >> 26) & 0x7) as u8;
    let dp = (id >> 24) & 0x3;
    let pf = (id >> 16) & 0xFF;
    let ps = (id >> 8) & 0xFF;
    let source = (id & 0xFF) as u8;
    if pf < 240 {
        J1939Id { priority, pgn: (dp << 16) | (pf << 8), source, destination: Some(ps as u8) }
    } else {
        J1939Id { priority, pgn: (dp << 16) | (pf << 8) | ps, source, destination: None }
    }
}

const PGN_REQUEST: u32 = 59904;
const PGN_ACK: u32 = 59392;
const PGN_TP_DT: u32 = 60160;
const PGN_TP_CM: u32 = 60416;
const PGN_DM1: u32 = 65226;
const PGN_DM2: u32 = 65227;

/// Names of the J1939 parameter groups most often seen on a bus.
pub fn j1939_pgn_name(pgn: u32) -> Option<&'static str> {
    Some(match pgn {
        PGN_REQUEST => "Request",
        PGN_ACK => "Acknowledgement",
        PGN_TP_DT => "TP.DT (transport data)",
        PGN_TP_CM => "TP.CM (transport connection)",
        60928 => "Address Claimed",
        61443 => "EEC2 (electronic engine controller 2)",
        61444 => "EEC1 (electronic engine controller 1)",
        65226 => "DM1 (active diagnostic trouble codes)",
        65227 => "DM2 (previously active DTCs)",
        65242 => "SOFT (software identification)",
        65248 => "VD (vehicle distance)",
        65253 => "HOURS (engine hours)",
        65259 => "CI (component identification)",
        65260 => "VI (vehicle identification)",
        65262 => "ET1 (engine temperature 1)",
        65263 => "EFL/P1 (engine fluid level/pressure 1)",
        65265 => "CCVS1 (cruise control/vehicle speed 1)",
        65266 => "LFE1 (fuel economy)",
        65269 => "AMB (ambient conditions)",
        65270 => "IC1 (inlet/exhaust conditions 1)",
        65271 => "VEP1 (vehicle electrical power 1)",
        65276 => "DD1 (dash display 1)",
        _ => return None,
    })
}

fn pgn_text(pgn: u32) -> String {
    match j1939_pgn_name(pgn) {
        Some(n) => format!("PGN {pgn} {n}"),
        None => format!("PGN {pgn}"),
    }
}

/// CANopen function of an 11-bit COB-ID in the predefined connection set, with the node id.
pub fn canopen(id: u32) -> Option<(String, u8)> {
    if id > 0x7FF {
        return None;
    }
    match id {
        0 => return Some(("NMT".into(), 0)),
        0x80 => return Some(("SYNC".into(), 0)),
        0x100 => return Some(("TIME".into(), 0)),
        0x7E4 => return Some(("LSS response".into(), 0)),
        0x7E5 => return Some(("LSS request".into(), 0)),
        _ => {}
    }
    let node = (id & 0x7F) as u8;
    if node == 0 {
        return None;
    }
    let f = match id & 0x780 {
        0x080 => "EMCY",
        0x180 => "TPDO1",
        0x200 => "RPDO1",
        0x280 => "TPDO2",
        0x300 => "RPDO2",
        0x380 => "TPDO3",
        0x400 => "RPDO3",
        0x480 => "TPDO4",
        0x500 => "RPDO4",
        0x580 => "SDO tx",
        0x600 => "SDO rx",
        0x700 => "Heartbeat",
        _ => return None,
    };
    Some((f.into(), node))
}

/// Which higher layer to label frames with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    #[default]
    None,
    J1939,
    Canopen,
    Obd2,
}

/// A short label for a frame under `layer`, or `None` when it does not apply. Stateless: multi-frame
/// messages are labelled frame by frame (use a [`Decoder`] to reassemble them).
pub fn describe(frame: &CanFrame, layer: Layer) -> Option<String> {
    Decoder::default().describe(frame, layer)
}

/// Labels frames like [`describe`], and reassembles ISO-TP (OBD-II) and J1939 transport protocol
/// messages across frames. Feed it every frame of a stream, in order.
#[derive(Debug, Default)]
pub struct Decoder {
    isotp: HashMap<(u32, bool), IsoTpRx>,
    tp: HashMap<(u8, u8), TpSession>,
}

#[derive(Debug)]
struct IsoTpRx {
    total: usize,
    data: Vec<u8>,
    next_seq: u8,
}

#[derive(Debug)]
struct TpSession {
    pgn: u32,
    size: usize,
    packets: u8,
    data: Vec<u8>,
    received: Vec<bool>,
    bam: bool,
}

/// Sessions kept at most (a bus full of abandoned transfers must not grow the decoder forever).
const MAX_SESSIONS: usize = 256;

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets partial transfers (when the layer changes or the trace restarts).
    pub fn reset(&mut self) {
        self.isotp.clear();
        self.tp.clear();
    }

    pub fn describe(&mut self, frame: &CanFrame, layer: Layer) -> Option<String> {
        match layer {
            Layer::None => None,
            Layer::J1939 => self.j1939_label(frame),
            Layer::Canopen => canopen_label(frame),
            Layer::Obd2 => self.obd_label(frame),
        }
    }

    // ── J1939 ──

    fn j1939_label(&mut self, frame: &CanFrame) -> Option<String> {
        if !frame.extended {
            return None;
        }
        let j = j1939(frame.id);
        let name = j1939_pgn_name(j.pgn).map(|n| format!(" {n}")).unwrap_or_default();
        let head = match j.destination {
            Some(d) => format!("PGN {}{name} SA {:02X} DA {:02X} P{}", j.pgn, j.source, d, j.priority),
            None => format!("PGN {}{name} SA {:02X} P{}", j.pgn, j.source, j.priority),
        };
        if frame.rtr {
            return Some(head);
        }
        let d = &frame.data;
        let detail = match j.pgn {
            PGN_REQUEST if d.len() >= 3 => Some(format!("request for {}", pgn_text(pgn24(&d[0..3])))),
            PGN_ACK if d.len() >= 8 => {
                let what = match d[0] {
                    0 => "ACK",
                    1 => "NACK",
                    2 => "access denied",
                    3 => "cannot respond",
                    _ => "?",
                };
                Some(format!("{what} for {}", pgn_text(pgn24(&d[5..8]))))
            }
            PGN_TP_CM if d.len() >= 8 => Some(self.tp_cm(j.source, j.destination.unwrap_or(0xFF), d)),
            PGN_TP_DT if !d.is_empty() => Some(self.tp_dt(j.source, j.destination.unwrap_or(0xFF), d)),
            PGN_DM1 | PGN_DM2 => Some(dm_text(j.pgn, d)),
            _ => None,
        };
        Some(match detail {
            Some(t) => format!("{head}: {t}"),
            None => head,
        })
    }

    fn tp_cm(&mut self, sa: u8, da: u8, d: &[u8]) -> String {
        let pgn = pgn24(&d[5..8]);
        let size = u16::from_le_bytes([d[1], d[2]]) as usize;
        match d[0] {
            16 | 32 => {
                let bam = d[0] == 32;
                let packets = d[3];
                if self.tp.len() >= MAX_SESSIONS {
                    self.tp.clear();
                }
                self.tp.insert(
                    (sa, if bam { 0xFF } else { da }),
                    TpSession { pgn, size, packets, data: vec![0; packets as usize * 7], received: vec![false; packets as usize], bam },
                );
                format!("{}, {} {size} bytes in {packets} packets", if bam { "BAM" } else { "RTS" }, pgn_text(pgn))
            }
            17 => format!("CTS for {}, {} packets from packet {}", pgn_text(pgn), d[1], d[2]),
            19 => format!("end of message ACK for {}, {size} bytes in {} packets", pgn_text(pgn), d[3]),
            255 => {
                // The abort can come from either side: drop the session both ways.
                self.tp.remove(&(sa, da));
                self.tp.remove(&(da, sa));
                let reason = match d[1] {
                    1 => "already in a session",
                    2 => "resources needed elsewhere",
                    3 => "timeout",
                    4 => "CTS received while sending",
                    5 => "maximum retransmissions reached",
                    6 => "unexpected data transfer packet",
                    7 => "bad sequence number",
                    8 => "duplicate sequence number",
                    9 => "message too large",
                    _ => "other reason",
                };
                format!("abort for {}: {reason} ({})", pgn_text(pgn), d[1])
            }
            c => format!("control byte {c} for {}", pgn_text(pgn)),
        }
    }

    fn tp_dt(&mut self, sa: u8, da: u8, d: &[u8]) -> String {
        let seq = d[0];
        let key = if self.tp.contains_key(&(sa, da)) { (sa, da) } else { (sa, 0xFF) };
        let Some(s) = self.tp.get_mut(&key) else {
            return format!("packet {seq}");
        };
        if seq == 0 || seq > s.packets {
            return format!("packet {seq} (outside the {} of {})", s.packets, pgn_text(s.pgn));
        }
        let at = (seq as usize - 1) * 7;
        let part = &d[1..d.len().min(8)];
        s.data[at..at + part.len()].copy_from_slice(part);
        s.received[seq as usize - 1] = true;
        let mut out = format!("packet {seq} of {} ({})", s.packets, pgn_text(s.pgn));
        if s.received.iter().all(|&r| r) {
            let s = self.tp.remove(&key).unwrap_or_else(|| unreachable!());
            let mut data = s.data;
            data.truncate(s.size);
            let kind = if s.bam { "BAM" } else { "RTS/CTS" };
            out = format!("packet {seq} of {}, {kind} complete: {} {} bytes", s.packets, pgn_text(s.pgn), data.len());
            let body = match s.pgn {
                PGN_DM1 | PGN_DM2 => dm_text(s.pgn, &data),
                65260 => format!("VIN {}", ascii(&data).trim_end_matches('*')),
                _ => hex_preview(&data),
            };
            out.push_str(": ");
            out.push_str(&body);
        }
        out
    }

    // ── OBD-II over ISO-TP ──

    fn obd_label(&mut self, frame: &CanFrame) -> Option<String> {
        let (prefix, response) = obd_address(frame)?;
        if frame.rtr {
            return Some(prefix);
        }
        let d = &frame.data;
        let Some(&pci) = d.first() else { return Some(prefix) };
        let key = (frame.id, frame.extended);
        let text = match pci >> 4 {
            0 => {
                // Single frame; CAN FD frames over 8 bytes put the length in the next byte.
                let (len, from) = if pci & 0x0F == 0 && d.len() > 8 { (d.get(1).copied().unwrap_or(0) as usize, 2) } else { ((pci & 0x0F) as usize, 1) };
                let payload = &d[from.min(d.len())..(from + len).min(d.len())];
                obd_service(payload, response, true)
            }
            1 => {
                let mut total = (((pci & 0x0F) as usize) << 8) | d.get(1).copied().unwrap_or(0) as usize;
                let mut from = 2;
                if total == 0 && d.len() >= 6 {
                    total = u32::from_be_bytes([d[2], d[3], d[4], d[5]]) as usize;
                    from = 6;
                }
                let part = d[from.min(d.len())..].to_vec();
                let head = obd_service(&part, response, false);
                if self.isotp.len() >= MAX_SESSIONS {
                    self.isotp.clear();
                }
                self.isotp.insert(key, IsoTpRx { total, data: part, next_seq: 1 });
                format!("ISO-TP first frame, {total} bytes: {head}")
            }
            2 => {
                let seq = pci & 0x0F;
                match self.isotp.get_mut(&key) {
                    Some(rx) if rx.next_seq == seq => {
                        let want = rx.total - rx.data.len().min(rx.total);
                        rx.data.extend_from_slice(&d[1..d.len().min(1 + want)]);
                        rx.next_seq = (seq + 1) & 0x0F;
                        if rx.data.len() >= rx.total {
                            let rx = self.isotp.remove(&key).unwrap_or_else(|| unreachable!());
                            format!("ISO-TP consecutive frame {seq}, complete ({} bytes): {}", rx.total, obd_service(&rx.data, response, true))
                        } else {
                            format!("ISO-TP consecutive frame {seq} ({} of {} bytes)", rx.data.len(), rx.total)
                        }
                    }
                    Some(rx) => {
                        let expected = rx.next_seq;
                        self.isotp.remove(&key);
                        format!("ISO-TP consecutive frame {seq} (expected {expected}; message dropped)")
                    }
                    None => format!("ISO-TP consecutive frame {seq}"),
                }
            }
            3 => {
                let status = match pci & 0x0F {
                    0 => "continue",
                    1 => "wait",
                    2 => "overflow",
                    _ => "?",
                };
                let bs = d.get(1).copied().unwrap_or(0);
                let st = d.get(2).copied().unwrap_or(0);
                let stmin = match st {
                    0..=0x7F => format!("{st} ms"),
                    0xF1..=0xF9 => format!("{} us", (st - 0xF0) as u32 * 100),
                    _ => format!("0x{st:02X}"),
                };
                format!("ISO-TP flow control: {status}, block size {bs}, STmin {stmin}")
            }
            _ => format!("unknown ISO-TP frame type {:X}", pci >> 4),
        };
        Some(format!("{prefix}: {text}"))
    }
}

fn pgn24(b: &[u8]) -> u32 {
    b[0] as u32 | (b[1] as u32) << 8 | ((b[2] & 0x03) as u32) << 16
}

fn ascii(b: &[u8]) -> String {
    b.iter().map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '.' }).collect::<String>()
}

fn hex_preview(b: &[u8]) -> String {
    let n = b.len().min(16);
    let mut s = b[..n].iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ");
    if b.len() > n {
        s.push_str(" …");
    }
    s
}

/// DM1 / DM2: lamp status and the DTCs (SPN, FMI, occurrence count).
fn dm_text(pgn: u32, d: &[u8]) -> String {
    let what = if pgn == PGN_DM1 { "active" } else { "previously active" };
    if d.len() < 2 {
        return format!("no {what} DTCs");
    }
    let lamp = |shift: u8, name: &str| match (d[0] >> shift) & 3 {
        1 => Some(name.to_string()),
        _ => None,
    };
    let lamps: Vec<String> =
        [lamp(6, "MIL"), lamp(4, "red stop"), lamp(2, "amber warning"), lamp(0, "protect")].into_iter().flatten().collect();
    let dtcs: Vec<String> = d[2..]
        .chunks_exact(4)
        .filter_map(|c| {
            let spn = c[0] as u32 | (c[1] as u32) << 8 | ((c[2] >> 5) as u32) << 16;
            // SPN 0 is not a parameter: a DM1 with no fault carries SPN 0 (often with OC 0 or 0x7F, or all
            // ones) as filler, which J1939-73 reads as "no DTC".
            (spn != 0 && c != [0xFF; 4]).then(|| format!("SPN {spn} FMI {} OC {}", c[2] & 0x1F, c[3] & 0x7F))
        })
        .collect();
    let mut out = if dtcs.is_empty() { format!("no {what} DTCs") } else { format!("{} {what} DTC{}: {}", dtcs.len(), if dtcs.len() == 1 { "" } else { "s" }, dtcs.join("; ")) };
    if !lamps.is_empty() {
        out.push_str(&format!(" (lamps on: {})", lamps.join(", ")));
    }
    out
}

/// Who sends a frame in OBD-II, as a label prefix, and whether it is a response. 11-bit 7DF / 7E0-7EF;
/// 29-bit 18DB33F1 (functional), 18DA<ecu>F1 (to an ECU) and 18DAF1<ecu> (from an ECU).
fn obd_address(frame: &CanFrame) -> Option<(String, bool)> {
    if frame.extended {
        return match frame.id {
            0x18DB_33F1 => Some(("OBD-II request (all ECUs)".into(), false)),
            id if id & 0xFFFF_00FF == 0x18DA_00F1 => Some((format!("OBD-II request to ECU {:02X}", (id >> 8) & 0xFF), false)),
            id if id & 0xFFFF_FF00 == 0x18DA_F100 => Some((format!("OBD-II response from ECU {:02X}", id & 0xFF), true)),
            _ => None,
        };
    }
    match frame.id {
        0x7DF => Some(("OBD-II request (all ECUs)".into(), false)),
        0x7E0..=0x7E7 => Some((format!("OBD-II request to ECU {}", frame.id - 0x7E0), false)),
        0x7E8..=0x7EF => Some((format!("OBD-II response from ECU {}", frame.id - 0x7E8), true)),
        _ => None,
    }
}

fn pid_name(mode: u8, pid: u8) -> Option<&'static str> {
    Some(match (mode, pid) {
        (1 | 2, 0x00) => "supported PIDs 01-20",
        (1 | 2, 0x01) => "monitor status",
        (1 | 2, 0x04) => "engine load",
        (1 | 2, 0x05) => "coolant temperature",
        (1 | 2, 0x0C) => "engine speed",
        (1 | 2, 0x0D) => "vehicle speed",
        (1 | 2, 0x11) => "throttle position",
        (1 | 2, 0x2F) => "fuel level",
        (1 | 2, 0x20) => "supported PIDs 21-40",
        (1 | 2, 0x40) => "supported PIDs 41-60",
        (9, 0x00) => "supported PIDs",
        (9, 0x01) => "VIN message count",
        (9, 0x02) => "VIN",
        (9, 0x04) => "calibration ID",
        (9, 0x06) => "calibration verification numbers",
        (9, 0x0A) => "ECU name",
        _ => return None,
    })
}

/// UDS / OBD negative response codes (ISO 14229-1).
pub fn nrc_name(code: u8) -> &'static str {
    match code {
        0x10 => "general reject",
        0x11 => "service not supported",
        0x12 => "sub-function not supported",
        0x13 => "incorrect message length or invalid format",
        0x14 => "response too long",
        0x21 => "busy, repeat request",
        0x22 => "conditions not correct",
        0x24 => "request sequence error",
        0x25 => "no response from sub-net component",
        0x26 => "failure prevents execution",
        0x31 => "request out of range",
        0x33 => "security access denied",
        0x35 => "invalid key",
        0x36 => "exceeded number of attempts",
        0x37 => "required time delay not expired",
        0x70 => "upload/download not accepted",
        0x71 => "transfer data suspended",
        0x72 => "general programming failure",
        0x73 => "wrong block sequence counter",
        0x78 => "response pending",
        0x7E => "sub-function not supported in active session",
        0x7F => "service not supported in active session",
        _ => "unknown code",
    }
}

/// An OBD-II DTC (two bytes) as text, `P0301`.
pub fn obd_dtc(a: u8, b: u8) -> String {
    let letter = ['P', 'C', 'B', 'U'][(a >> 6) as usize];
    format!("{letter}{}{:X}{:X}{:X}", (a >> 4) & 3, a & 0x0F, b >> 4, b & 0x0F)
}

/// An OBD-II service payload (after the ISO-TP header) as text. `complete` is false for the start
/// of a longer message, where only the header can be shown.
fn obd_service(p: &[u8], response: bool, complete: bool) -> String {
    let Some(&sid) = p.first() else { return "empty".into() };
    if response && sid == 0x7F {
        let mode = p.get(1).map(|m| format!("{m:02X}")).unwrap_or_else(|| "?".into());
        return match p.get(2) {
            Some(&c) => format!("negative response to mode {mode}: {} (0x{c:02X})", nrc_name(c)),
            None => format!("negative response to mode {mode}"),
        };
    }
    let mode = if response { sid.wrapping_sub(0x40) } else { sid };
    let pid = p.get(1).copied();
    match mode {
        1 | 2 | 9 => {
            let Some(pid) = pid else { return format!("mode {mode:02X}") };
            let named = pid_name(mode, pid);
            let mut out = match (mode, named) {
                (1, Some(n)) => n.to_string(),
                (_, Some(n)) => format!("mode {mode:02X} PID {pid:02X} ({n})"),
                _ => format!("mode {mode:02X} PID {pid:02X}"),
            };
            if response && complete {
                if mode == 1
                    && let Some((_, v, u)) = obd_value(&[0, sid, pid, p.get(2).copied().unwrap_or(0), p.get(3).copied().unwrap_or(0)][..p.len().min(4) + 1])
                {
                    out = format!("{out} = {} {u}", (v * 10.0).round() / 10.0);
                } else if mode == 9 && matches!(pid, 0x02 | 0x04 | 0x0A) && p.len() > 3 {
                    // Byte 2 is the number of data items; the text follows.
                    let text = ascii(&p[3..]);
                    let text = text.trim_matches(|c: char| c == '.' || c == ' ');
                    out = match pid {
                        0x02 => format!("VIN {text}"),
                        0x04 => format!("calibration ID {text}"),
                        _ => format!("ECU name {text}"),
                    };
                }
            }
            out
        }
        3 | 7 | 0x0A => {
            let what = match mode {
                3 => "stored DTCs",
                7 => "pending DTCs",
                _ => "permanent DTCs",
            };
            if !response {
                return format!("mode {mode:02X} (read {what})");
            }
            if !complete {
                return format!("mode {mode:02X} {what}");
            }
            // CAN (ISO 15765-4): the number of DTCs, then two bytes each.
            let dtcs: Vec<String> = p.get(2..).unwrap_or(&[]).chunks_exact(2).filter(|c| c != &[0, 0]).map(|c| obd_dtc(c[0], c[1])).collect();
            if dtcs.is_empty() { format!("mode {mode:02X}: no {what}") } else { format!("mode {mode:02X} {what}: {}", dtcs.join(", ")) }
        }
        4 => if response { "mode 04: DTCs cleared".into() } else { "mode 04 (clear DTCs)".into() },
        _ => match pid {
            Some(pid) => format!("mode {mode:02X} PID {pid:02X}"),
            None => format!("mode {mode:02X}"),
        },
    }
}

/// Physical value of an OBD-II mode 01 response, for the PIDs the simulator answers.
pub fn obd_value(data: &[u8]) -> Option<(&'static str, f64, &'static str)> {
    if data.len() < 4 || data[1] != 0x41 {
        return None;
    }
    let a = data[3] as f64;
    let b = data.get(4).copied().unwrap_or(0) as f64;
    Some(match data[2] {
        0x04 => ("engine load", a * 100.0 / 255.0, "%"),
        0x05 => ("coolant temperature", a - 40.0, "degC"),
        0x0C => ("engine speed", (256.0 * a + b) / 4.0, "rpm"),
        0x0D => ("vehicle speed", a, "km/h"),
        0x11 => ("throttle position", a * 100.0 / 255.0, "%"),
        0x2F => ("fuel level", a * 100.0 / 255.0, "%"),
        _ => return None,
    })
}

// ── CANopen ──

fn nmt_state(b: u8) -> &'static str {
    match b & 0x7F {
        0 => "boot-up",
        4 => "stopped",
        5 => "operational",
        0x7F => "pre-operational",
        _ => "?",
    }
}

fn canopen_label(frame: &CanFrame) -> Option<String> {
    if frame.extended {
        return None;
    }
    let (f, node) = canopen(frame.id)?;
    let d = &frame.data;
    Some(match f.as_str() {
        "NMT" => {
            let cmd = match d.first() {
                Some(1) => "start",
                Some(2) => "stop",
                Some(0x80) => "pre-operational",
                Some(0x81) => "reset node",
                Some(0x82) => "reset communication",
                _ => "?",
            };
            let target = d.get(1).map(|n| if *n == 0 { "all nodes".to_string() } else { format!("node {n}") });
            format!("NMT {cmd} {}", target.unwrap_or_default()).trim_end().to_string()
        }
        "SYNC" => match d.first() {
            Some(c) => format!("SYNC counter {c}"),
            None => f,
        },
        "TIME" | "LSS request" | "LSS response" => f,
        "Heartbeat" if frame.rtr => format!("Node guarding request node {node}"),
        "Heartbeat" => match d.first() {
            Some(&b) if b & 0x80 != 0 => format!("Node guarding response node {node}: {} (toggle 1)", nmt_state(b)),
            Some(&b) => format!("Heartbeat node {node}: {}", nmt_state(b)),
            None => format!("Heartbeat node {node}: ?"),
        },
        _ if frame.rtr => format!("{f} node {node} (remote request)"),
        "SDO tx" | "SDO rx" => format!("{f} node {node}: {}", sdo_text(d, f == "SDO rx")),
        "EMCY" => format!("EMCY node {node}: {}", emcy_text(d)),
        _ => format!("{f} node {node}"),
    })
}

/// An expedited SDO value: the data bytes 4-7 that are in use, little-endian.
fn sdo_value(d: &[u8], cmd: u8) -> String {
    let size_set = cmd & 0x01 != 0;
    let n = if size_set { 4 - ((cmd >> 2) & 3) as usize } else { 4 };
    let bytes = &d[4..(4 + n).min(d.len())];
    let mut v: u32 = 0;
    for (i, b) in bytes.iter().enumerate() {
        v |= (*b as u32) << (8 * i);
    }
    format!("0x{v:0w$X} ({v})", w = bytes.len() * 2)
}

/// CiA 301 SDO protocol, client request (`rx`, 0x600 + node) or server response (0x580 + node).
fn sdo_text(d: &[u8], request: bool) -> String {
    let Some(&cmd) = d.first() else { return "empty".into() };
    let obj = || {
        if d.len() >= 4 {
            format!("{:04X}sub{:02X}", u16::from_le_bytes([d[1], d[2]]), d[3])
        } else {
            "?".to_string()
        }
    };
    let cs = cmd >> 5;
    if cs == 4 {
        let code = if d.len() >= 8 { u32::from_le_bytes([d[4], d[5], d[6], d[7]]) } else { 0 };
        return format!("abort {}: 0x{code:08X} {}", obj(), sdo_abort_text(code));
    }
    let segment = |toggle: bool| {
        let n = 7 - ((cmd >> 1) & 7) as usize;
        let last = if cmd & 1 != 0 { ", last" } else { "" };
        format!("toggle {}, {n} bytes{last}", toggle as u8)
    };
    let toggle = cmd & 0x10 != 0;
    if request {
        match cs {
            1 => {
                let e = cmd & 0x02 != 0;
                if e && d.len() >= 5 {
                    format!("write {} = {}", obj(), sdo_value(d, cmd))
                } else if cmd & 0x01 != 0 && d.len() >= 8 {
                    format!("write {}, {} bytes (segmented)", obj(), u32::from_le_bytes([d[4], d[5], d[6], d[7]]))
                } else {
                    format!("write {} (segmented)", obj())
                }
            }
            2 => format!("read {}", obj()),
            0 => format!("download segment ({})", segment(toggle)),
            3 => format!("upload segment request (toggle {})", toggle as u8),
            5 => "block upload".into(),
            6 => "block download".into(),
            _ => format!("command 0x{cmd:02X}"),
        }
    } else {
        match cs {
            2 => {
                let e = cmd & 0x02 != 0;
                if e && d.len() >= 5 {
                    format!("read {} = {}", obj(), sdo_value(d, cmd))
                } else if cmd & 0x01 != 0 && d.len() >= 8 {
                    format!("read {}: {} bytes follow (segmented)", obj(), u32::from_le_bytes([d[4], d[5], d[6], d[7]]))
                } else {
                    format!("read {}: segmented", obj())
                }
            }
            3 => format!("write {} confirmed", obj()),
            0 => format!("upload segment ({})", segment(toggle)),
            1 => format!("download segment confirmed (toggle {})", toggle as u8),
            5 => "block download".into(),
            6 => "block upload".into(),
            _ => format!("command 0x{cmd:02X}"),
        }
    }
}

/// CiA 301 SDO abort codes.
pub fn sdo_abort_text(code: u32) -> &'static str {
    match code {
        0x0503_0000 => "toggle bit not alternated",
        0x0504_0000 => "SDO protocol timed out",
        0x0504_0001 => "command specifier not valid or unknown",
        0x0504_0002 => "invalid block size",
        0x0504_0003 => "invalid sequence number",
        0x0504_0004 => "CRC error",
        0x0504_0005 => "out of memory",
        0x0601_0000 => "unsupported access to an object",
        0x0601_0001 => "attempt to read a write-only object",
        0x0601_0002 => "attempt to write a read-only object",
        0x0602_0000 => "object does not exist in the object dictionary",
        0x0604_0041 => "object cannot be mapped to the PDO",
        0x0604_0042 => "PDO length exceeded",
        0x0604_0043 => "general parameter incompatibility",
        0x0604_0047 => "general internal incompatibility in the device",
        0x0606_0000 => "access failed due to a hardware error",
        0x0607_0010 => "data type does not match, length of service parameter does not match",
        0x0607_0012 => "data type does not match, length of service parameter too high",
        0x0607_0013 => "data type does not match, length of service parameter too low",
        0x0609_0011 => "sub-index does not exist",
        0x0609_0030 => "invalid value for parameter",
        0x0609_0031 => "value of parameter written too high",
        0x0609_0032 => "value of parameter written too low",
        0x0609_0036 => "maximum value is less than minimum value",
        0x060A_0023 => "resource not available: SDO connection",
        0x0800_0000 => "general error",
        0x0800_0020 => "data cannot be transferred or stored to the application",
        0x0800_0021 => "data cannot be transferred or stored because of local control",
        0x0800_0022 => "data cannot be transferred or stored because of the present device state",
        0x0800_0023 => "object dictionary dynamic generation failed or no object dictionary present",
        0x0800_0024 => "no data available",
        _ => "unknown abort code",
    }
}

/// CiA 301 emergency message: error code, error register and the manufacturer bytes.
fn emcy_text(d: &[u8]) -> String {
    if d.len() < 3 {
        return "?".into();
    }
    let code = u16::from_le_bytes([d[0], d[1]]);
    let reg = d[2];
    let what = match code {
        0x0000 => "error reset or no error",
        0x8110 => "CAN overrun (objects lost)",
        0x8120 => "CAN in error passive mode",
        0x8130 => "life guard error or heartbeat error",
        0x8140 => "recovered from bus off",
        0x8150 => "CAN-ID collision",
        0x8210 => "PDO not processed due to length error",
        0x8220 => "PDO length exceeded",
        0x8230 => "DAM MPDO not processed, destination object not available",
        0x8240 => "unexpected SYNC data length",
        0x8250 => "RPDO timeout",
        _ => match code >> 8 {
            0x10 => "generic error",
            0x20..=0x23 => "current",
            0x30..=0x33 => "voltage",
            0x40..=0x42 => "temperature",
            0x50 => "device hardware",
            0x60..=0x63 => "device software",
            0x70 => "additional modules",
            0x80..=0x82 => "monitoring",
            0x90 => "external error",
            0xF0 => "additional functions",
            0xFF => "device specific",
            _ => "unknown",
        },
    };
    let bits: Vec<&str> = [(0, "generic"), (1, "current"), (2, "voltage"), (3, "temperature"), (4, "communication"), (5, "device profile"), (7, "manufacturer")]
        .into_iter()
        .filter(|(b, _)| reg & (1 << b) != 0)
        .map(|(_, n)| n)
        .collect();
    let mut out = format!("error 0x{code:04X} ({what}), register 0x{reg:02X}");
    if !bits.is_empty() {
        out.push_str(&format!(" ({})", bits.join(", ")));
    }
    if d.len() > 3 && d[3..].iter().any(|&b| b != 0) {
        out.push_str(&format!(", data {}", hex_preview(&d[3..])));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(id: u32, data: &[u8]) -> CanFrame {
        CanFrame::new(id, data).unwrap()
    }

    fn feed(dec: &mut Decoder, layer: Layer, frames: &[CanFrame]) -> Vec<String> {
        frames.iter().map(|fr| dec.describe(fr, layer).unwrap_or_default()).collect()
    }

    #[test]
    fn j1939_fields() {
        // EEC1 from the engine (SA 0x00), priority 3.
        let j = j1939(0x0CF0_0400);
        assert_eq!(j, J1939Id { priority: 3, pgn: 61444, source: 0, destination: None });
        // Request (PDU1) from 0xF9 to 0x00.
        let r = j1939(0x18EA_00F9);
        assert_eq!(r.pgn, 59904);
        assert_eq!(r.destination, Some(0));
        assert_eq!(describe(&f(0x18FE_EE00, &[0; 8]), Layer::J1939).unwrap(), "PGN 65262 ET1 (engine temperature 1) SA 00 P6");
        assert_eq!(describe(&CanFrame::new(0x100, &[]).unwrap(), Layer::J1939), None);
    }

    #[test]
    fn j1939_request_and_ack() {
        let req = f(0x18EA_00F9, &[0xEE, 0xFE, 0x00]);
        assert_eq!(describe(&req, Layer::J1939).unwrap(), "PGN 59904 Request SA F9 DA 00 P6: request for PGN 65262 ET1 (engine temperature 1)");
        let nack = f(0x18E8_F900, &[1, 0xFF, 0xFF, 0xFF, 0xF9, 0xEE, 0xFE, 0x00]);
        assert!(describe(&nack, Layer::J1939).unwrap().ends_with(": NACK for PGN 65262 ET1 (engine temperature 1)"));
    }

    #[test]
    fn j1939_dm1_single_frame() {
        // Amber lamp on, SPN 100 (oil pressure) FMI 1, occurrence count 5.
        let dm1 = f(0x18FE_CA00, &[0x04, 0xFF, 100, 0, 0x01, 5, 0xFF, 0xFF]);
        assert_eq!(
            describe(&dm1, Layer::J1939).unwrap(),
            "PGN 65226 DM1 (active diagnostic trouble codes) SA 00 P6: 1 active DTC: SPN 100 FMI 1 OC 5 (lamps on: amber warning)"
        );
        // High SPN bits live in the top 3 bits of byte 4: SPN 520192 = 0x7F000.
        let dm1 = f(0x18FE_CA00, &[0x00, 0xFF, 0x00, 0xF0, 0xE0 | 31, 0x01, 0xFF, 0xFF]);
        assert!(describe(&dm1, Layer::J1939).unwrap().ends_with("SPN 520192 FMI 31 OC 1"));
        let none = f(0x18FE_CA00, &[0x00, 0xFF, 0, 0, 0, 0, 0xFF, 0xFF]);
        assert!(describe(&none, Layer::J1939).unwrap().ends_with(": no active DTCs"));
        // The simulator's DM1 without a fault: SPN 0 with OC 127 is filler, not a DTC.
        let quiet = f(0x18FE_CA00, &[0x3F, 0xF3, 0x00, 0x00, 0x00, 0xFF, 0x80, 0xFF]);
        assert!(describe(&quiet, Layer::J1939).unwrap().contains(": no active DTCs"), "{:?}", describe(&quiet, Layer::J1939));
    }

    #[test]
    fn j1939_bam_dm1_reassembly() {
        // DM1 with 3 DTCs: 2 lamp bytes + 12 = 14 bytes in 2 packets, broadcast.
        let payload = [0x40u8, 0xFF, 110, 0, 0x00, 3, 190, 0, 0x02, 1, 0x23, 0x01, 0x8A, 7];
        let mut frames = vec![f(0x1CEC_FF00, &[32, 14, 0, 2, 0xFF, 0xCA, 0xFE, 0x00])];
        frames.push(f(0x1CEB_FF00, &[1, payload[0], payload[1], payload[2], payload[3], payload[4], payload[5], payload[6]]));
        frames.push(f(0x1CEB_FF00, &[2, payload[7], payload[8], payload[9], payload[10], payload[11], payload[12], payload[13]]));
        let mut dec = Decoder::new();
        let l = feed(&mut dec, Layer::J1939, &frames);
        assert_eq!(
            l[0],
            "PGN 60416 TP.CM (transport connection) SA 00 DA FF P7: BAM, PGN 65226 DM1 (active diagnostic trouble codes) 14 bytes in 2 packets"
        );
        assert_eq!(l[1], "PGN 60160 TP.DT (transport data) SA 00 DA FF P7: packet 1 of 2 (PGN 65226 DM1 (active diagnostic trouble codes))");
        assert_eq!(
            l[2],
            "PGN 60160 TP.DT (transport data) SA 00 DA FF P7: packet 2 of 2, BAM complete: PGN 65226 DM1 (active diagnostic trouble codes) 14 bytes: \
3 active DTCs: SPN 110 FMI 0 OC 3; SPN 190 FMI 2 OC 1; SPN 262435 FMI 10 OC 7 (lamps on: MIL)"
        );
        // Stateless: frame by frame only.
        assert!(describe(&frames[2], Layer::J1939).unwrap().ends_with(": packet 2"));
    }

    #[test]
    fn j1939_rts_cts_and_abort() {
        let mut dec = Decoder::new();
        let vin = b"1FUJGLDR5CLBP8834*";
        let rts = f(0x1CEC_F900, &[16, 18, 0, 3, 0xFF, 0xEC, 0xFE, 0x00]);
        let cts = f(0x1CEC_00F9, &[17, 3, 1, 0xFF, 0xFF, 0xEC, 0xFE, 0x00]);
        let mut frames = vec![rts, cts];
        for (i, chunk) in vin.chunks(7).enumerate() {
            let mut d = vec![i as u8 + 1];
            d.extend_from_slice(chunk);
            d.resize(8, 0xFF);
            frames.push(f(0x1CEB_F900, &d));
        }
        frames.push(f(0x1CEC_00F9, &[19, 18, 0, 3, 0xFF, 0xEC, 0xFE, 0x00]));
        let l = feed(&mut dec, Layer::J1939, &frames);
        assert!(l[0].ends_with(": RTS, PGN 65260 VI (vehicle identification) 18 bytes in 3 packets"), "{}", l[0]);
        assert!(l[1].ends_with(": CTS for PGN 65260 VI (vehicle identification), 3 packets from packet 1"), "{}", l[1]);
        assert!(l[4].ends_with("RTS/CTS complete: PGN 65260 VI (vehicle identification) 18 bytes: VIN 1FUJGLDR5CLBP8834"), "{}", l[4]);
        assert!(l[5].contains("end of message ACK"), "{}", l[5]);
        let abort = f(0x1CEC_00F9, &[255, 3, 0xFF, 0xFF, 0xFF, 0xCA, 0xFE, 0x00]);
        assert!(dec.describe(&abort, Layer::J1939).unwrap().ends_with("abort for PGN 65226 DM1 (active diagnostic trouble codes): timeout (3)"));
    }

    #[test]
    fn canopen_functions() {
        assert_eq!(canopen(0x181), Some(("TPDO1".into(), 1)));
        assert_eq!(canopen(0x705), Some(("Heartbeat".into(), 5)));
        assert_eq!(canopen(0x780), None);
        assert_eq!(describe(&f(0x705, &[5]), Layer::Canopen).unwrap(), "Heartbeat node 5: operational");
        assert_eq!(describe(&f(0, &[1, 0]), Layer::Canopen).unwrap(), "NMT start all nodes");
        // Node guarding: the RTR, and the answer with its toggle bit.
        let rtr = CanFrame { rtr: true, rtr_len: 1, ..f(0x705, &[]) };
        assert_eq!(describe(&rtr, Layer::Canopen).unwrap(), "Node guarding request node 5");
        assert_eq!(describe(&f(0x705, &[0x85]), Layer::Canopen).unwrap(), "Node guarding response node 5: operational (toggle 1)");
        assert_eq!(describe(&f(0x7E5, &[0x04, 1, 0, 0, 0, 0, 0, 0]), Layer::Canopen).unwrap(), "LSS request");
    }

    #[test]
    fn canopen_sdo() {
        let l = |id: u32, d: &[u8]| describe(&f(id, d), Layer::Canopen).unwrap();
        assert_eq!(l(0x602, &[0x40, 0x18, 0x10, 0x01, 0, 0, 0, 0]), "SDO rx node 2: read 1018sub01");
        assert_eq!(l(0x582, &[0x43, 0x18, 0x10, 0x01, 0x2C, 0x01, 0, 0]), "SDO tx node 2: read 1018sub01 = 0x0000012C (300)");
        assert_eq!(l(0x582, &[0x4B, 0x00, 0x10, 0x00, 0x92, 0x01, 0, 0]), "SDO tx node 2: read 1000sub00 = 0x0192 (402)");
        assert_eq!(l(0x602, &[0x2B, 0x40, 0x60, 0x00, 0x0F, 0x00, 0, 0]), "SDO rx node 2: write 6040sub00 = 0x000F (15)");
        assert_eq!(l(0x602, &[0x2F, 0x60, 0x60, 0x00, 0x01, 0, 0, 0]), "SDO rx node 2: write 6060sub00 = 0x01 (1)");
        assert_eq!(l(0x582, &[0x60, 0x40, 0x60, 0x00, 0, 0, 0, 0]), "SDO tx node 2: write 6040sub00 confirmed");
        assert_eq!(l(0x582, &[0x41, 0x08, 0x10, 0x00, 0x14, 0, 0, 0]), "SDO tx node 2: read 1008sub00: 20 bytes follow (segmented)");
        assert_eq!(l(0x602, &[0x60, 0, 0, 0, 0, 0, 0, 0]), "SDO rx node 2: upload segment request (toggle 0)");
        assert_eq!(l(0x582, &[0x0B, b'V', b'o', b'l', b't', 0, 0, 0]), "SDO tx node 2: upload segment (toggle 0, 2 bytes, last)");
        assert_eq!(
            l(0x582, &[0x80, 0x00, 0x20, 0x00, 0x00, 0x00, 0x02, 0x06]),
            "SDO tx node 2: abort 2000sub00: 0x06020000 object does not exist in the object dictionary"
        );
    }

    #[test]
    fn canopen_emcy() {
        let l = |d: &[u8]| describe(&f(0x83, d), Layer::Canopen).unwrap();
        assert_eq!(l(&[0x10, 0x32, 0x05, 0, 0, 0, 0, 0]), "EMCY node 3: error 0x3210 (voltage), register 0x05 (generic, voltage)");
        assert_eq!(l(&[0x30, 0x81, 0x11, 0, 0, 0, 0, 0]), "EMCY node 3: error 0x8130 (life guard error or heartbeat error), register 0x11 (generic, communication)");
        assert_eq!(l(&[0, 0, 0, 0, 0, 0, 0, 0]), "EMCY node 3: error 0x0000 (error reset or no error), register 0x00");
        assert_eq!(l(&[0x00, 0xFF, 0x81, 0xAA, 0, 0, 0, 0]), "EMCY node 3: error 0xFF00 (device specific), register 0x81 (generic, manufacturer), data AA 00 00 00 00");
    }

    #[test]
    fn obd() {
        let req = f(0x7DF, &[2, 1, 0x0C, 0, 0, 0, 0, 0]);
        assert_eq!(describe(&req, Layer::Obd2).unwrap(), "OBD-II request (all ECUs): engine speed");
        let rsp = [4, 0x41, 0x0C, 0x1A, 0xF8, 0, 0, 0];
        assert_eq!(describe(&f(0x7E8, &rsp), Layer::Obd2).unwrap(), "OBD-II response from ECU 0: engine speed = 1726 rpm");
        assert_eq!(obd_value(&rsp), Some(("engine speed", 1726.0, "rpm")));
    }

    /// A VIN read (mode 09 PID 02) across a first frame, a flow control and two consecutive frames.
    fn vin_frames() -> Vec<CanFrame> {
        let vin = b"1HGCM82633A004352";
        let mut p = vec![0x49, 0x02, 0x01];
        p.extend_from_slice(vin); // 20 bytes
        vec![
            f(0x7DF, &[0x02, 0x09, 0x02, 0, 0, 0, 0, 0]),
            f(0x7E8, &[0x10, 20, p[0], p[1], p[2], p[3], p[4], p[5]]),
            f(0x7E0, &[0x30, 0, 0, 0, 0, 0, 0, 0]),
            f(0x7E8, &[0x21, p[6], p[7], p[8], p[9], p[10], p[11], p[12]]),
            f(0x7E8, &[0x22, p[13], p[14], p[15], p[16], p[17], p[18], p[19]]),
        ]
    }

    #[test]
    fn obd_isotp_vin() {
        let frames = vin_frames();
        // Frame by frame: the first frame is no longer read as "mode D4 PID 49".
        assert_eq!(describe(&frames[0], Layer::Obd2).unwrap(), "OBD-II request (all ECUs): mode 09 PID 02 (VIN)");
        assert_eq!(describe(&frames[1], Layer::Obd2).unwrap(), "OBD-II response from ECU 0: ISO-TP first frame, 20 bytes: mode 09 PID 02 (VIN)");
        assert_eq!(describe(&frames[2], Layer::Obd2).unwrap(), "OBD-II request to ECU 0: ISO-TP flow control: continue, block size 0, STmin 0 ms");
        assert_eq!(describe(&frames[3], Layer::Obd2).unwrap(), "OBD-II response from ECU 0: ISO-TP consecutive frame 1");
        // Reassembled.
        let mut dec = Decoder::new();
        let l = feed(&mut dec, Layer::Obd2, &frames);
        assert_eq!(l[3], "OBD-II response from ECU 0: ISO-TP consecutive frame 1 (13 of 20 bytes)");
        assert_eq!(l[4], "OBD-II response from ECU 0: ISO-TP consecutive frame 2, complete (20 bytes): VIN 1HGCM82633A004352");
        // Out of sequence: the message is dropped.
        let mut dec = Decoder::new();
        let l = feed(&mut dec, Layer::Obd2, &[frames[1].clone(), frames[4].clone()]);
        assert_eq!(l[1], "OBD-II response from ECU 0: ISO-TP consecutive frame 2 (expected 1; message dropped)");
    }

    #[test]
    fn obd_dtcs_and_negative_responses() {
        let l = |id: u32, d: &[u8]| describe(&f(id, d), Layer::Obd2).unwrap();
        assert_eq!(l(0x7DF, &[1, 0x03, 0, 0, 0, 0, 0, 0]), "OBD-II request (all ECUs): mode 03 (read stored DTCs)");
        assert_eq!(l(0x7E8, &[6, 0x43, 2, 0x03, 0x01, 0x44, 0x20, 0]), "OBD-II response from ECU 0: mode 03 stored DTCs: P0301, C0420");
        assert_eq!(l(0x7E8, &[2, 0x43, 0, 0, 0, 0, 0, 0]), "OBD-II response from ECU 0: mode 03: no stored DTCs");
        assert_eq!(obd_dtc(0x81, 0x23), "B0123");
        assert_eq!(obd_dtc(0xC1, 0x55), "U0155");
        assert_eq!(l(0x7E9, &[3, 0x7F, 0x09, 0x11, 0, 0, 0, 0]), "OBD-II response from ECU 1: negative response to mode 09: service not supported (0x11)");
        assert_eq!(l(0x7E8, &[3, 0x7F, 0x22, 0x78, 0, 0, 0, 0]), "OBD-II response from ECU 0: negative response to mode 22: response pending (0x78)");
        assert_eq!(l(0x7E8, &[1, 0x44, 0, 0, 0, 0, 0, 0]), "OBD-II response from ECU 0: mode 04: DTCs cleared");

        // Four DTCs do not fit a single frame: reassembled from first + consecutive frame.
        let mut dec = Decoder::new();
        let frames = [f(0x7E8, &[0x10, 10, 0x43, 4, 0x01, 0x71, 0x01, 0x72]), f(0x7E8, &[0x21, 0x03, 0x00, 0x04, 0x20, 0, 0, 0])];
        let out = feed(&mut dec, Layer::Obd2, &frames);
        assert_eq!(out[0], "OBD-II response from ECU 0: ISO-TP first frame, 10 bytes: mode 03 stored DTCs");
        assert_eq!(out[1], "OBD-II response from ECU 0: ISO-TP consecutive frame 1, complete (10 bytes): mode 03 stored DTCs: P0171, P0172, P0300, P0420");
    }

    #[test]
    fn obd_29_bit() {
        let l = |id: u32, d: &[u8]| describe(&CanFrame::with_ext(id, true, d).unwrap(), Layer::Obd2);
        assert_eq!(l(0x18DB_33F1, &[2, 1, 0x0D, 0, 0, 0, 0, 0]).unwrap(), "OBD-II request (all ECUs): vehicle speed");
        assert_eq!(l(0x18DA_10F1, &[2, 1, 0x05, 0, 0, 0, 0, 0]).unwrap(), "OBD-II request to ECU 10: coolant temperature");
        assert_eq!(l(0x18DA_F110, &[3, 0x41, 0x05, 130, 0, 0, 0, 0]).unwrap(), "OBD-II response from ECU 10: coolant temperature = 90 degC");
        assert_eq!(l(0x18FE_EE00, &[0; 8]), None);
        // An 11-bit OBD identifier sent as 29-bit is something else.
        assert_eq!(l(0x7E8, &[0; 8]), None);
    }
}
