//! I2C: start, repeated start and stop conditions, 7- and 10-bit addresses, read/write, data
//! bytes with ACK or NACK. Pulses shorter than `min_pulse` samples on either line are ignored
//! (spike filter, like the 50 ns filter of Fast-mode receivers).

use serde::{Deserialize, Serialize};

use super::{Annotation, DecodeOutput, Level, Table, TableRow, hex};
use crate::capture::Capture;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct I2cConfig {
    pub scl: u8,
    pub sda: u8,
    /// Ignore pulses up to this many samples wide.
    #[serde(default)]
    pub min_pulse: u64,
}

impl Default for I2cConfig {
    fn default() -> Self {
        Self { scl: 0, sda: 1, min_pulse: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct I2cByte {
    pub start: u64,
    pub end: u64,
    pub value: u8,
    pub ack: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct I2cTransfer {
    pub start: u64,
    pub end: u64,
    /// Began with a repeated start.
    pub repeated: bool,
    pub address: u16,
    pub ten_bit: bool,
    pub read: bool,
    pub address_ack: bool,
    pub data: Vec<I2cByte>,
    /// Ended with a stop condition (otherwise by a repeated start or the end of the capture).
    pub stopped: bool,
    /// Bits left over when the transfer ended (not a multiple of 9).
    pub partial_bits: u8,
}

impl I2cTransfer {
    pub fn bytes(&self) -> Vec<u8> {
        self.data.iter().map(|b| b.value).collect()
    }
}

/// Edges of one line with pulses of `min` samples or fewer removed.
fn filtered(cap: &Capture, ch: u8, min: u64) -> Vec<(u64, bool)> {
    let raw: Vec<(u64, bool)> = cap.edges(ch, 0, cap.len()).collect();
    if min == 0 {
        return raw;
    }
    let mut out: Vec<(u64, bool)> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if i + 1 < raw.len() && raw[i + 1].0 - raw[i].0 <= min {
            i += 2;
            continue;
        }
        if out.last().is_some_and(|l| l.1 == raw[i].1) {
            i += 1;
            continue;
        }
        out.push(raw[i]);
        i += 1;
    }
    out
}

pub fn decode(cap: &Capture, cfg: &I2cConfig) -> Vec<I2cTransfer> {
    let scl_e = filtered(cap, cfg.scl, cfg.min_pulse);
    let sda_e = filtered(cap, cfg.sda, cfg.min_pulse);
    // (sample, line 0 = scl / 1 = sda, new level)
    let mut ev: Vec<(u64, u8, bool)> = scl_e.iter().map(|&(s, l)| (s, 0, l)).chain(sda_e.iter().map(|&(s, l)| (s, 1, l))).collect();
    ev.sort_by_key(|e| (e.0, e.1));
    let mut scl = cap.level(cfg.scl, 0);
    let mut sda = cap.level(cfg.sda, 0);

    let mut out = Vec::new();
    let mut cur: Option<I2cTransfer> = None;
    let mut bits: Vec<(u64, bool)> = Vec::new();
    // Byte index inside the transfer, address bytes included.
    let mut byte_idx = 0usize;

    let flush = |cur: &mut Option<I2cTransfer>, bits: &mut Vec<(u64, bool)>, out: &mut Vec<I2cTransfer>, end: u64, stopped: bool| {
        if let Some(mut t) = cur.take() {
            t.end = end;
            t.stopped = stopped;
            t.partial_bits = bits.len() as u8;
            out.push(t);
        }
        bits.clear();
    };

    for (s, line, level) in ev {
        if line == 1 {
            if scl {
                if !level {
                    // START or repeated START.
                    let repeated = cur.is_some();
                    flush(&mut cur, &mut bits, &mut out, s, false);
                    cur = Some(I2cTransfer { start: s, end: s, repeated, address: 0, ten_bit: false, read: false, address_ack: false, data: vec![], stopped: false, partial_bits: 0 });
                    byte_idx = 0;
                } else {
                    flush(&mut cur, &mut bits, &mut out, s, true);
                }
            }
            sda = level;
            continue;
        }
        scl = level;
        if !level || cur.is_none() {
            continue;
        }
        // SCL rising: sample SDA.
        bits.push((s, sda));
        if bits.len() < 9 {
            continue;
        }
        let value = bits[..8].iter().fold(0u8, |v, b| v << 1 | u8::from(b.1));
        let ack = !bits[8].1;
        let start = bits[0].0;
        let end = bits[8].0 + 1;
        bits.clear();
        let t = cur.as_mut().unwrap();
        match byte_idx {
            0 => {
                t.read = value & 1 == 1;
                t.address_ack = ack;
                if value >> 3 == 0b11110 {
                    t.ten_bit = true;
                    t.address = ((value as u16 >> 1) & 3) << 8;
                } else {
                    t.address = value as u16 >> 1;
                }
            }
            1 if t.ten_bit && !t.read => {
                t.address |= value as u16;
                t.address_ack &= ack;
            }
            _ => t.data.push(I2cByte { start, end, value, ack }),
        }
        t.end = end;
        byte_idx += 1;
    }
    let end = cur.as_ref().map(|t| t.end).unwrap_or(0);
    flush(&mut cur, &mut bits, &mut out, end, false);
    out
}

pub fn output(ts: &[I2cTransfer]) -> DecodeOutput {
    let mut ann = Vec::new();
    for t in ts {
        let kind = if t.repeated { "Sr" } else { "S" };
        ann.push(Annotation::new(0, t.start, t.start + 1, Level::Control, if t.repeated { "Repeated start" } else { "Start" }, kind));
        let a = if t.ten_bit { format!("0x{:03X}", t.address) } else { format!("0x{:02X}", t.address) };
        let rw = if t.read { "read" } else { "write" };
        let first_data = t.data.first().map(|b| b.start).unwrap_or(t.end);
        ann.push(Annotation::new(
            0,
            t.start + 1,
            first_data.max(t.start + 2),
            if t.address_ack { Level::Control } else { Level::Warning },
            format!("Address {a} {rw}{}", if t.address_ack { "" } else { " NACK" }),
            format!("{a}{}", if t.read { "R" } else { "W" }),
        ));
        for b in &t.data {
            ann.push(Annotation::new(1, b.start, b.end, Level::Data, format!("0x{:02X} {}", b.value, if b.ack { "ACK" } else { "NACK" }), format!("{:02X}", b.value)));
        }
        if t.stopped {
            ann.push(Annotation::new(0, t.end.saturating_sub(1), t.end, Level::Control, "Stop", "P"));
        }
    }
    let table = Table {
        columns: ["Time", "Address", "R/W", "Data", "ACK"].map(String::from).to_vec(),
        rows: ts
            .iter()
            .map(|t| TableRow {
                start: t.start,
                end: t.end,
                level: if t.address_ack { Level::Data } else { Level::Warning },
                cells: vec![
                    String::new(),
                    if t.ten_bit { format!("0x{:03X}", t.address) } else { format!("0x{:02X}", t.address) },
                    if t.read { "R".into() } else { "W".into() },
                    hex(&t.bytes()),
                    if !t.address_ack { "Address NACK".into() } else if t.data.last().is_some_and(|b| !b.ack) { "Last byte NACK".into() } else { "ACK".into() },
                ],
            })
            .collect(),
    };
    let nack = ts.iter().filter(|t| !t.address_ack).count();
    DecodeOutput {
        rows: vec!["Address".into(), "Data".into()],
        annotations: ann,
        table: Some(table),
        summary: format!("{} transfers{}", ts.len(), if nack > 0 { format!(", {nack} not acknowledged") } else { String::new() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{I2cLines, I2cWriter, Synth};

    const L: I2cLines = I2cLines { scl: 0, sda: 1 };

    fn sensor_read(glitch: bool) -> Capture {
        let mut s = Synth::new(4_000_000, [true; 8]);
        let mut w = I2cWriter::new(&mut s, L, 0.00002, 100_000.0);
        // Write register pointer 0x05, repeated start, read 2 bytes (last NACK).
        w.start();
        w.byte(0x48 << 1, true);
        w.byte(0x05, true);
        w.start();
        w.byte(0x48 << 1 | 1, true);
        w.byte(0x1A, true);
        w.byte(0xC0, false);
        w.stop();
        // A device that does not answer.
        w.t += 0.0001;
        w.start();
        w.byte(0x23 << 1, false);
        w.stop();
        let end = w.t;
        if glitch {
            // 1-sample spikes on SCL while it is low and on SDA while SCL is high in a data bit.
            s.glitch(0, s.at(0.00002 + 0.000011), 1);
        }
        s.extend_to(s.at(end + 0.00002));
        s.capture(&["SCL", "SDA"], 0)
    }

    #[test]
    fn write_then_repeated_start_read() {
        let r = decode(&sensor_read(false), &I2cConfig::default());
        assert_eq!(r.len(), 3);
        assert_eq!((r[0].address, r[0].read, r[0].bytes()), (0x48, false, vec![0x05]));
        assert!(!r[0].stopped);
        assert!(r[1].repeated);
        assert_eq!((r[1].address, r[1].read, r[1].bytes()), (0x48, true, vec![0x1A, 0xC0]));
        assert!(!r[1].data[1].ack);
        assert!(r[1].stopped);
        assert_eq!(r[2].address, 0x23);
        assert!(!r[2].address_ack);
        let out = output(&r);
        assert!(out.summary.contains("1 not acknowledged"));
    }

    #[test]
    fn spike_filter_removes_glitches() {
        let cap = sensor_read(true);
        let unfiltered = decode(&cap, &I2cConfig::default());
        assert_ne!(unfiltered.first().map(|t| t.address), Some(0x48), "the glitch adds a clock and must corrupt the address");
        let r = decode(&cap, &I2cConfig { min_pulse: 2, ..I2cConfig::default() });
        assert_eq!(r[0].address, 0x48);
        assert_eq!(r[1].bytes(), vec![0x1A, 0xC0]);
    }

    #[test]
    fn ten_bit_address() {
        let mut s = Synth::new(4_000_000, [true; 8]);
        let mut w = I2cWriter::new(&mut s, L, 0.00002, 100_000.0);
        let addr: u16 = 0x2B7;
        w.start();
        w.byte(0b1111_0000 | ((addr >> 8) as u8 & 3) << 1, true);
        w.byte(addr as u8, true);
        w.byte(0x99, true);
        w.stop();
        let end = w.t;
        s.extend_to(s.at(end + 0.00001));
        let r = decode(&s.capture(&[], 0), &I2cConfig::default());
        assert_eq!(r.len(), 1);
        assert!(r[0].ten_bit);
        assert_eq!(r[0].address, addr);
        assert_eq!(r[0].bytes(), vec![0x99]);
    }
}
