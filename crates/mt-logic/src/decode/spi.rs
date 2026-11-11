//! SPI: clock, MOSI and/or MISO, optional chip select; modes 0 to 3, 4 to 32-bit words, MSB or
//! LSB first. Words are grouped into transfers by chip select (or, without one, by clock silence).
//!
//! On top of the words, [`SpiDevice::Mcp2515`] names the instructions and registers of the
//! Microchip MCP2515 CAN controller, the SPI CAN chip of countless industrial boards.

use serde::{Deserialize, Serialize};

use super::{Annotation, DecodeOutput, Level, Table, TableRow, hex, time_cell};
use crate::capture::Capture;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SpiDevice {
    #[default]
    None,
    Mcp2515,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpiConfig {
    pub clk: u8,
    pub mosi: Option<u8>,
    pub miso: Option<u8>,
    pub cs: Option<u8>,
    /// SPI mode 0 to 3 (CPOL = bit 1, CPHA = bit 0).
    pub mode: u8,
    #[serde(default = "eight")]
    pub word_bits: u8,
    #[serde(default)]
    pub lsb_first: bool,
    #[serde(default)]
    pub cs_active_high: bool,
    #[serde(default)]
    pub device: SpiDevice,
}

fn eight() -> u8 {
    8
}

impl Default for SpiConfig {
    fn default() -> Self {
        Self { clk: 1, mosi: Some(2), miso: Some(3), cs: Some(0), mode: 0, word_bits: 8, lsb_first: false, cs_active_high: false, device: SpiDevice::None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpiWord {
    pub start: u64,
    pub end: u64,
    pub mosi: Option<u32>,
    pub miso: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpiTransfer {
    pub start: u64,
    pub end: u64,
    pub words: Vec<SpiWord>,
    /// Chip select went inactive in the middle of a word.
    pub partial_bits: u8,
}

impl SpiTransfer {
    pub fn mosi(&self) -> Vec<u8> {
        self.words.iter().filter_map(|w| w.mosi.map(|v| v as u8)).collect()
    }

    pub fn miso(&self) -> Vec<u8> {
        self.words.iter().filter_map(|w| w.miso.map(|v| v as u8)).collect()
    }
}

pub fn decode(cap: &Capture, cfg: &SpiConfig) -> Vec<SpiTransfer> {
    let cpol = cfg.mode & 2 != 0;
    let cpha = cfg.mode & 1 != 0;
    // Sampling edge: rising for modes 0 and 3, falling for 1 and 2.
    let sample_rising = cpol == cpha;
    let bits_per_word = cfg.word_bits.clamp(4, 32);
    let cs_active = |i: u64| cfg.cs.is_none_or(|cs| cap.level(cs, i) == cfg.cs_active_high);

    let mut transfers = Vec::new();
    let mut cur: Option<SpiTransfer> = None;
    let (mut mo, mut mi, mut nbits, mut wstart) = (0u32, 0u32, 0u8, 0u64);
    let mut last_clk = 0u64;
    let mut avg_period = 0f64;

    // Chip select edges and clock edges, merged in time order.
    let mut events: Vec<(u64, u8)> = cap.edges(cfg.clk, 0, cap.len()).filter(|e| e.1 == sample_rising).map(|e| (e.0, 0)).collect();
    if let Some(cs) = cfg.cs {
        events.extend(cap.edges(cs, 0, cap.len()).map(|e| (e.0, 1)));
        if cs_active(0) {
            events.push((0, 1));
        }
    }
    // A chip-select edge at the same sample as a clock edge is handled first.
    events.sort_by_key(|&(s, kind)| (s, std::cmp::Reverse(kind)));

    let finish = |cur: &mut Option<SpiTransfer>, transfers: &mut Vec<SpiTransfer>, nbits: u8, end: u64| {
        if let Some(mut t) = cur.take() {
            t.partial_bits = nbits;
            t.end = end.max(t.start);
            if !t.words.is_empty() || nbits > 0 {
                transfers.push(t);
            }
        }
    };

    for (s, kind) in events {
        if kind == 1 {
            if cs_active(s) {
                finish(&mut cur, &mut transfers, nbits, s);
                cur = Some(SpiTransfer { start: s, end: s, words: vec![], partial_bits: 0 });
            } else {
                finish(&mut cur, &mut transfers, nbits, s);
            }
            nbits = 0;
            continue;
        }
        if !cs_active(s) {
            continue;
        }
        if cfg.cs.is_none() {
            // Without chip select, a clock pause of 20 periods ends the transfer.
            let gap = s - last_clk;
            if cur.is_some() && avg_period > 0.0 && gap as f64 > avg_period * 20.0 {
                finish(&mut cur, &mut transfers, nbits, last_clk + 1);
                nbits = 0;
            }
            if nbits > 0 {
                avg_period = if avg_period == 0.0 { gap as f64 } else { avg_period * 0.9 + gap as f64 * 0.1 };
            }
            if cur.is_none() {
                cur = Some(SpiTransfer { start: s, end: s, words: vec![], partial_bits: 0 });
            }
        }
        last_clk = s;
        if nbits == 0 {
            wstart = s;
            mo = 0;
            mi = 0;
        }
        // Sample just after the edge: data is stable then in every mode.
        let at = s;
        let shift = if cfg.lsb_first { nbits as u32 } else { (bits_per_word - 1 - nbits) as u32 };
        if let Some(c) = cfg.mosi
            && cap.level(c, at)
        {
            mo |= 1 << shift;
        }
        if let Some(c) = cfg.miso
            && cap.level(c, at)
        {
            mi |= 1 << shift;
        }
        nbits += 1;
        if nbits == bits_per_word {
            let t = cur.get_or_insert(SpiTransfer { start: wstart, end: s, words: vec![], partial_bits: 0 });
            t.words.push(SpiWord { start: wstart, end: s + 1, mosi: cfg.mosi.map(|_| mo), miso: cfg.miso.map(|_| mi) });
            t.end = s + 1;
            nbits = 0;
        }
    }
    let end = cur.as_ref().map(|t| t.end).unwrap_or(0);
    finish(&mut cur, &mut transfers, nbits, end);
    transfers
}

// ── MCP2515 ──

pub fn mcp2515_register(addr: u8) -> String {
    let named = match addr {
        0x0E => Some("CANSTAT"),
        0x0F => Some("CANCTRL"),
        0x1C => Some("TEC"),
        0x1D => Some("REC"),
        0x28 => Some("CNF3"),
        0x29 => Some("CNF2"),
        0x2A => Some("CNF1"),
        0x2B => Some("CANINTE"),
        0x2C => Some("CANINTF"),
        0x2D => Some("EFLG"),
        0x0C => Some("BFPCTRL"),
        0x0D => Some("TXRTSCTRL"),
        0x30 => Some("TXB0CTRL"),
        0x40 => Some("TXB1CTRL"),
        0x50 => Some("TXB2CTRL"),
        0x60 => Some("RXB0CTRL"),
        0x70 => Some("RXB1CTRL"),
        _ => None,
    };
    if let Some(n) = named {
        return n.to_string();
    }
    let hi = addr & 0xF0;
    let lo = addr & 0x0F;
    let buf = match hi {
        0x30 => "TXB0",
        0x40 => "TXB1",
        0x50 => "TXB2",
        0x60 => "RXB0",
        0x70 => "RXB1",
        _ => return format!("0x{addr:02X}"),
    };
    let field = match lo {
        1 => "SIDH".to_string(),
        2 => "SIDL".to_string(),
        3 => "EID8".to_string(),
        4 => "EID0".to_string(),
        5 => "DLC".to_string(),
        6..=13 => format!("D{}", lo - 6),
        _ => format!("+{lo}"),
    };
    format!("{buf}{field}")
}

/// SIDH, SIDL, EID8, EID0, DLC and data of an MCP2515 buffer: the CAN frame they hold.
pub fn mcp2515_frame(b: &[u8]) -> Option<String> {
    if b.len() < 5 {
        return None;
    }
    let ext = b[1] & 0x08 != 0;
    let sid = (b[0] as u32) << 3 | (b[1] as u32) >> 5;
    let id = if ext { sid << 18 | ((b[1] & 3) as u32) << 16 | (b[2] as u32) << 8 | b[3] as u32 } else { sid };
    let rtr = b[4] & 0x40 != 0;
    let dlc = (b[4] & 0x0F).min(8) as usize;
    let data = &b[5..b.len().min(5 + dlc)];
    Some(if ext {
        format!("ID 0x{id:08X} [{dlc}]{} {}", if rtr { " RTR" } else { "" }, hex(data))
    } else {
        format!("ID 0x{id:03X} [{dlc}]{} {}", if rtr { " RTR" } else { "" }, hex(data))
    })
}

/// What one MCP2515 transfer did.
pub fn mcp2515_describe(t: &SpiTransfer) -> String {
    let mo = t.mosi();
    let mi = t.miso();
    let Some(&op) = mo.first() else { return String::new() };
    let reg_list = |start: u8, vals: &[u8]| {
        vals.iter().enumerate().map(|(i, v)| format!("{}={v:02X}", mcp2515_register(start.wrapping_add(i as u8)))).collect::<Vec<_>>().join(" ")
    };
    match op {
        0xC0 => "RESET".into(),
        0x03 if mo.len() >= 2 => {
            let a = mo[1];
            if (a & 0x0F) == 1 && matches!(a & 0xF0, 0x60 | 0x70) && mi.len() > 2 {
                let f = mcp2515_frame(&mi[2..]).unwrap_or_default();
                format!("READ {} … {}", mcp2515_register(a), f)
            } else {
                format!("READ {}", reg_list(a, mi.get(2..).unwrap_or(&[])))
            }
        }
        0x02 if mo.len() >= 3 => format!("WRITE {}", reg_list(mo[1], &mo[2..])),
        0x05 if mo.len() >= 4 => format!("BIT MODIFY {} mask {:02X} data {:02X}", mcp2515_register(mo[1]), mo[2], mo[3]),
        0xA0 => format!("READ STATUS {}", mi.get(1).map(|v| format!("{v:08b}")).unwrap_or_default()),
        0xB0 => format!("RX STATUS {}", mi.get(1).map(|v| format!("{v:08b}")).unwrap_or_default()),
        0x80..=0x87 => {
            let bufs: Vec<String> = (0..3).filter(|b| op & (1 << b) != 0).map(|b| format!("TXB{b}")).collect();
            format!("RTS {}", if bufs.is_empty() { "(none)".into() } else { bufs.join(" ") })
        }
        0x40..=0x45 => {
            let buf = (op >> 1) & 3;
            if op & 1 == 0 {
                format!("LOAD TX BUFFER {buf}: {}", mcp2515_frame(&mo[1..]).unwrap_or_else(|| hex(&mo[1..])))
            } else {
                format!("LOAD TX BUFFER {buf} data: {}", hex(&mo[1..]))
            }
        }
        0x90 | 0x92 | 0x94 | 0x96 => {
            let buf = (op >> 2) & 1;
            if op & 2 == 0 {
                format!("READ RX BUFFER {buf}: {}", mcp2515_frame(mi.get(1..).unwrap_or(&[])).unwrap_or_default())
            } else {
                format!("READ RX BUFFER {buf} data: {}", hex(mi.get(1..).unwrap_or(&[])))
            }
        }
        _ => format!("Unknown instruction 0x{op:02X}"),
    }
}

pub fn output(cfg: &SpiConfig, ts: &[SpiTransfer]) -> DecodeOutput {
    let mut ann = Vec::new();
    let fmt = |v: u32| if cfg.word_bits <= 8 { format!("{v:02X}") } else { format!("{v:0w$X}", w = cfg.word_bits.div_ceil(4) as usize) };
    for t in ts {
        for w in &t.words {
            if let Some(v) = w.mosi {
                ann.push(Annotation::new(0, w.start, w.end, Level::Data, format!("0x{}", fmt(v)), fmt(v)));
            }
            if let Some(v) = w.miso {
                ann.push(Annotation::new(1, w.start, w.end, Level::Data, format!("0x{}", fmt(v)), fmt(v)));
            }
        }
        let desc = if cfg.device == SpiDevice::Mcp2515 { mcp2515_describe(t) } else { format!("{} words", t.words.len()) };
        ann.push(Annotation::new(2, t.start, t.end, Level::Info, desc.clone(), desc.split(' ').next().unwrap_or("").to_string()));
        if t.partial_bits > 0 {
            ann.push(Annotation::new(3, t.end.saturating_sub(1), t.end, Level::Warning, format!("Chip select ended after {} bits of a word", t.partial_bits), "!"));
        }
    }
    let table = cfg.cs.is_some().then(|| Table {
        columns: ["Time", "MOSI", "MISO", "Meaning"].map(String::from).to_vec(),
        rows: ts
            .iter()
            .map(|t| TableRow {
                start: t.start,
                end: t.end,
                level: if t.partial_bits > 0 { Level::Warning } else { Level::Data },
                cells: vec![
                    String::new(),
                    t.words.iter().filter_map(|w| w.mosi).map(fmt).collect::<Vec<_>>().join(" "),
                    t.words.iter().filter_map(|w| w.miso).map(fmt).collect::<Vec<_>>().join(" "),
                    if cfg.device == SpiDevice::Mcp2515 { mcp2515_describe(t) } else { String::new() },
                ],
            })
            .collect(),
    });
    DecodeOutput {
        rows: vec!["MOSI".into(), "MISO".into(), "Transfers".into(), "Warnings".into()],
        annotations: ann,
        table,
        summary: format!("{} transfers, {} words, mode {}", ts.len(), ts.iter().map(|t| t.words.len()).sum::<usize>(), cfg.mode),
    }
}

/// Fills in the time column, which needs the capture.
pub fn output_with_times(cap: &Capture, cfg: &SpiConfig, ts: &[SpiTransfer]) -> DecodeOutput {
    let mut o = output(cfg, ts);
    if let Some(t) = o.table.as_mut() {
        for r in &mut t.rows {
            r.cells[0] = time_cell(cap, r.start);
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{SpiLines, Synth, spi_transfer};

    fn lines(cpol: bool, cpha: bool) -> SpiLines {
        SpiLines { cs: 0, clk: 1, mosi: 2, miso: 3, cpol, cpha }
    }

    #[test]
    fn all_four_modes() {
        for mode in 0..4u8 {
            let (cpol, cpha) = (mode & 2 != 0, mode & 1 != 0);
            let mut s = Synth::new(16_000_000, [true, cpol, false, false, false, false, false, false]);
            let t = spi_transfer(&mut s, lines(cpol, cpha), 0.00001, 1_000_000.0, &[0xA5, 0x3C, 0xFF, 0x00], &[0x5A, 0xC3, 0x00, 0xFF]);
            spi_transfer(&mut s, lines(cpol, cpha), t + 0.00001, 1_000_000.0, &[0x81], &[0x7E]);
            s.extend_to(s.at(t + 0.00005));
            let cap = s.capture(&[], 0);
            let cfg = SpiConfig { mode, ..SpiConfig::default() };
            let r = decode(&cap, &cfg);
            assert_eq!(r.len(), 2, "mode {mode}");
            assert_eq!(r[0].mosi(), vec![0xA5, 0x3C, 0xFF, 0x00], "mode {mode}");
            assert_eq!(r[0].miso(), vec![0x5A, 0xC3, 0x00, 0xFF], "mode {mode}");
            assert_eq!(r[1].mosi(), vec![0x81]);
            assert_eq!(r[1].miso(), vec![0x7E]);
        }
    }

    #[test]
    fn cs_ends_partial_word_and_clock_outside_cs_is_ignored() {
        let mut s = Synth::new(8_000_000, [true, false, false, false, false, false, false, false]);
        // Stray clocks with CS high.
        for i in 0..4 {
            s.set_t(1, 0.000001 * (2 * i + 1) as f64, true);
            s.set_t(1, 0.000001 * (2 * i + 2) as f64, false);
        }
        let t = spi_transfer(&mut s, lines(false, false), 0.00002, 500_000.0, &[0x12], &[0x34]);
        // CS low again, 3 bits, CS high.
        s.set_t(0, t + 0.00001, false);
        for i in 0..3 {
            s.set_t(1, t + 0.000012 + 0.000002 * i as f64, true);
            s.set_t(1, t + 0.000013 + 0.000002 * i as f64, false);
        }
        s.set_t(0, t + 0.00002, true);
        s.extend_to(s.at(t + 0.00003));
        let r = decode(&s.capture(&[], 0), &SpiConfig::default());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].mosi(), vec![0x12]);
        assert_eq!(r[1].words.len(), 0);
        assert_eq!(r[1].partial_bits, 3);
    }

    #[test]
    fn no_chip_select_splits_on_clock_pause() {
        let mut s = Synth::new(8_000_000, [true, false, false, false, false, false, false, false]);
        let l = lines(false, false);
        let t = spi_transfer(&mut s, l, 0.00001, 500_000.0, &[0xDE, 0xAD], &[]);
        spi_transfer(&mut s, l, t + 0.001, 500_000.0, &[0xBE, 0xEF], &[]);
        s.extend_to(s.at(t + 0.0011));
        let cfg = SpiConfig { cs: None, miso: None, ..SpiConfig::default() };
        let r = decode(&s.capture(&[], 0), &cfg);
        assert_eq!(r.iter().map(|t| t.mosi()).collect::<Vec<_>>(), vec![vec![0xDE, 0xAD], vec![0xBE, 0xEF]]);
    }

    #[test]
    fn lsb_first_and_16_bit_words() {
        let mut s = Synth::new(8_000_000, [true, false, false, false, false, false, false, false]);
        spi_transfer(&mut s, lines(false, false), 0.00001, 500_000.0, &[0x12, 0x34], &[]);
        s.extend_to(s.at(0.0001));
        let cap = s.capture(&[], 0);
        let r = decode(&cap, &SpiConfig { word_bits: 16, ..SpiConfig::default() });
        assert_eq!(r[0].words[0].mosi, Some(0x1234));
        let r = decode(&cap, &SpiConfig { lsb_first: true, ..SpiConfig::default() });
        assert_eq!(r[0].mosi(), vec![0x48, 0x2C]);
    }

    #[test]
    fn mcp2515_instructions() {
        let t = |mo: &[u8], mi: &[u8]| SpiTransfer {
            start: 0,
            end: 1,
            partial_bits: 0,
            words: (0..mo.len().max(mi.len())).map(|i| SpiWord { start: 0, end: 1, mosi: mo.get(i).map(|&v| v as u32), miso: mi.get(i).map(|&v| v as u32) }).collect(),
        };
        assert_eq!(mcp2515_describe(&t(&[0xC0], &[])), "RESET");
        assert_eq!(mcp2515_describe(&t(&[0x03, 0x0E, 0], &[0, 0, 0x80])), "READ CANSTAT=80");
        assert_eq!(mcp2515_describe(&t(&[0x02, 0x28, 0x00, 0x90, 0x02], &[])), "WRITE CNF3=00 CNF2=90 CNF1=02");
        assert_eq!(mcp2515_describe(&t(&[0x05, 0x0F, 0xE0, 0x00], &[])), "BIT MODIFY CANCTRL mask E0 data 00");
        // 0x123 standard, 2 bytes: SIDH 0x24, SIDL 0x60.
        assert_eq!(mcp2515_describe(&t(&[0x40, 0x24, 0x60, 0, 0, 2, 0xAB, 0xCD], &[])), "LOAD TX BUFFER 0: ID 0x123 [2] AB CD");
        assert_eq!(mcp2515_describe(&t(&[0x81], &[])), "RTS TXB0");
        assert_eq!(mcp2515_describe(&t(&[0x90, 0, 0, 0, 0, 0, 0], &[0, 0x24, 0x60, 0, 0, 1, 0x55])), "READ RX BUFFER 0: ID 0x123 [1] 55");
        assert_eq!(mcp2515_register(0x36), "TXB0D0");
    }
}
