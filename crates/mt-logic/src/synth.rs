//! Waveform synthesis: drive channels at given times, then render a [`Capture`]. The simulator
//! and the decoder tests build their signals with it, so both speak exactly the same wire formats
//! as the decoders, including the deliberate faults (glitches, bad CRCs, missing ACKs).

use crate::capture::{Capture, ChannelInfo};
use crate::decode::can::{CanFrameSpec, frame_bits};
use crate::decode::modbus::crc16;
use crate::decode::uart::{Parity, UartConfig};

/// Channel levels over time, one sorted list of `(sample, level)` per channel.
#[derive(Debug, Clone)]
pub struct Synth {
    pub rate: u64,
    idle: [bool; 8],
    changes: [Vec<(u64, bool)>; 8],
    end: u64,
}

impl Synth {
    pub fn new(rate: u64, idle: [bool; 8]) -> Self {
        Self { rate, idle, changes: Default::default(), end: 0 }
    }

    /// Sample index of time `t` (seconds).
    pub fn at(&self, t: f64) -> u64 {
        (t * self.rate as f64).round().max(0.0) as u64
    }

    /// Sets `ch` to `level` from sample `s` on.
    pub fn set(&mut self, ch: u8, s: u64, level: bool) {
        self.changes[ch as usize].push((s, level));
        self.end = self.end.max(s + 1);
    }

    pub fn set_t(&mut self, ch: u8, t: f64, level: bool) {
        let s = self.at(t);
        self.set(ch, s, level);
    }

    /// A pulse of `width` samples at the opposite of the level `ch` has at `s`: a glitch.
    pub fn glitch(&mut self, ch: u8, s: u64, width: u64) {
        let lvl = self.level_at(ch, s);
        self.set(ch, s, !lvl);
        self.set(ch, s + width, lvl);
    }

    pub fn level_at(&self, ch: u8, s: u64) -> bool {
        let mut v = self.idle[ch as usize];
        let mut best = None::<u64>;
        for &(at, l) in &self.changes[ch as usize] {
            if at <= s && best.is_none_or(|b| at >= b) {
                best = Some(at);
                v = l;
            }
        }
        v
    }

    pub fn extend_to(&mut self, s: u64) {
        self.end = self.end.max(s);
    }

    pub fn len(&self) -> u64 {
        self.end
    }

    pub fn is_empty(&self) -> bool {
        self.end == 0
    }

    /// Drives `bits` on `ch`, `bit_s` seconds each, from time `t0`. Returns the end time.
    pub fn bits(&mut self, ch: u8, t0: f64, bit_s: f64, bits: &[bool]) -> f64 {
        for (i, &b) in bits.iter().enumerate() {
            self.set_t(ch, t0 + i as f64 * bit_s, b);
        }
        t0 + bits.len() as f64 * bit_s
    }

    /// Renders `[0, len)` (at least up to the last change).
    pub fn render(&self, len: u64) -> Vec<u8> {
        let len = len.max(self.end);
        let mut out = vec![0u8; len as usize];
        for ch in 0..8 {
            let mut ch_changes = self.changes[ch].clone();
            // Stable: later calls for the same sample win.
            ch_changes.sort_by_key(|c| c.0);
            let bit = 1u8 << ch;
            let mut lvl = self.idle[ch];
            let mut pos = 0usize;
            let mut idx = 0;
            while pos < len as usize {
                while idx < ch_changes.len() && ch_changes[idx].0 as usize <= pos {
                    lvl = ch_changes[idx].1;
                    idx += 1;
                }
                let next = ch_changes.get(idx).map(|c| (c.0 as usize).min(len as usize)).unwrap_or(len as usize);
                if lvl {
                    for s in &mut out[pos..next.max(pos + 1).min(len as usize)] {
                        *s |= bit;
                    }
                }
                pos = next.max(pos + 1);
            }
        }
        out
    }

    pub fn capture(&self, names: &[&str], len: u64) -> Capture {
        let channels = (0..8).map(|i| ChannelInfo::named(names.get(i).copied().unwrap_or(&format!("D{i}")).to_string())).collect();
        Capture::new(self.rate, channels, self.render(len))
    }
}

/// The bits of one UART character, start bit to stop bit(s), as line levels with idle high.
/// `stop_bits` is 1, 1.5 or 2; 1.5 is returned as two entries and handled by [`uart_tx`].
pub fn uart_char_bits(cfg: &UartConfig, value: u16) -> Vec<bool> {
    let mut v = vec![false];
    let mut ones = 0;
    for i in 0..cfg.data_bits {
        let idx = if cfg.msb_first { cfg.data_bits - 1 - i } else { i };
        let b = value >> idx & 1 == 1;
        ones += u32::from(b);
        v.push(b);
    }
    match cfg.parity {
        Parity::None => {}
        Parity::Even => v.push(ones % 2 == 1),
        Parity::Odd => v.push(ones % 2 == 0),
        Parity::Mark => v.push(true),
        Parity::Space => v.push(false),
    }
    v
}

/// Options for [`uart_tx`] beyond the line format: a deliberate fault.
#[derive(Debug, Clone, Copy, Default)]
pub struct UartFault {
    /// Flip the parity bit of the character with this index.
    pub bad_parity_at: Option<usize>,
    /// Pull the stop bit of the character with this index low (framing error).
    pub bad_stop_at: Option<usize>,
}

/// Sends `bytes` on `ch` from `t0` with `gap_s` of idle between characters, at `baud` (which may
/// differ from `cfg.baud` to simulate a wrong-baud device). Returns the end time of the last stop bit.
#[allow(clippy::too_many_arguments)]
pub fn uart_tx(s: &mut Synth, ch: u8, t0: f64, bytes: &[u8], cfg: &UartConfig, baud: f64, gap_s: f64, fault: UartFault) -> f64 {
    let bit = 1.0 / baud;
    let mark = !cfg.inverted;
    let mut t = t0;
    for (i, &b) in bytes.iter().enumerate() {
        let mut bits = uart_char_bits(cfg, b as u16);
        if fault.bad_parity_at == Some(i) && cfg.parity != Parity::None {
            let last = bits.len() - 1;
            bits[last] = !bits[last];
        }
        let levels: Vec<bool> = bits.iter().map(|&x| if cfg.inverted { !x } else { x }).collect();
        t = s.bits(ch, t, bit, &levels);
        let stop_level = if fault.bad_stop_at == Some(i) { !mark } else { mark };
        s.set_t(ch, t, stop_level);
        t += bit * cfg.stop_bits as f64;
        s.set_t(ch, t, mark);
        t += gap_s;
    }
    t - gap_s
}

/// A Modbus RTU frame: `payload` (address, function, data) followed by its CRC, low byte first.
pub fn modbus_frame(payload: &[u8]) -> Vec<u8> {
    let crc = crc16(payload);
    let mut v = payload.to_vec();
    v.extend_from_slice(&crc.to_le_bytes());
    v
}

/// SPI mode and line assignment for [`spi_transfer`].
#[derive(Debug, Clone, Copy)]
pub struct SpiLines {
    pub clk: u8,
    pub mosi: u8,
    pub miso: u8,
    pub cs: u8,
    pub cpol: bool,
    pub cpha: bool,
}

/// One chip-select-framed SPI transfer, MSB first, 8-bit words. Returns the end time.
pub fn spi_transfer(s: &mut Synth, l: SpiLines, t0: f64, clk_hz: f64, mosi: &[u8], miso: &[u8]) -> f64 {
    let half = 0.5 / clk_hz;
    s.set_t(l.clk, t0, l.cpol);
    s.set_t(l.cs, t0, false);
    let mut t = t0 + half;
    let n = mosi.len().max(miso.len());
    for w in 0..n {
        for bit in (0..8).rev() {
            let mo = mosi.get(w).is_some_and(|b| b >> bit & 1 == 1);
            let mi = miso.get(w).is_some_and(|b| b >> bit & 1 == 1);
            if l.cpha {
                // Data changes on the leading edge, sampled on the trailing one.
                s.set_t(l.clk, t, !l.cpol);
                s.set_t(l.mosi, t + half * 0.1, mo);
                s.set_t(l.miso, t + half * 0.1, mi);
                t += half;
                s.set_t(l.clk, t, l.cpol);
                t += half;
            } else {
                // Data set up half a clock before the leading (sampling) edge.
                s.set_t(l.mosi, t - half * 0.5, mo);
                s.set_t(l.miso, t - half * 0.5, mi);
                s.set_t(l.clk, t, !l.cpol);
                t += half;
                s.set_t(l.clk, t, l.cpol);
                t += half;
            }
        }
    }
    s.set_t(l.cs, t, true);
    t
}

/// I2C on two channels: open-drain lines idle high.
#[derive(Debug, Clone, Copy)]
pub struct I2cLines {
    pub scl: u8,
    pub sda: u8,
}

/// Writes I2C conditions and bytes at a fixed clock. Every call returns the new time.
pub struct I2cWriter<'a> {
    pub s: &'a mut Synth,
    pub l: I2cLines,
    pub half: f64,
    pub t: f64,
}

impl<'a> I2cWriter<'a> {
    pub fn new(s: &'a mut Synth, l: I2cLines, t0: f64, hz: f64) -> Self {
        Self { s, l, half: 0.5 / hz, t: t0 }
    }

    pub fn start(&mut self) {
        // From either idle or after a byte (SCL low): SDA high, SCL high, then SDA falls.
        self.s.set_t(self.l.sda, self.t, true);
        self.t += self.half * 0.5;
        self.s.set_t(self.l.scl, self.t, true);
        self.t += self.half * 0.5;
        self.s.set_t(self.l.sda, self.t, false);
        self.t += self.half * 0.5;
        self.s.set_t(self.l.scl, self.t, false);
        self.t += self.half * 0.5;
    }

    fn bit(&mut self, b: bool) {
        self.s.set_t(self.l.sda, self.t, b);
        self.t += self.half * 0.5;
        self.s.set_t(self.l.scl, self.t, true);
        self.t += self.half;
        self.s.set_t(self.l.scl, self.t, false);
        self.t += self.half * 0.5;
    }

    /// A byte and its ACK bit (`ack` = the receiver pulls SDA low).
    pub fn byte(&mut self, v: u8, ack: bool) {
        for i in (0..8).rev() {
            self.bit(v >> i & 1 == 1);
        }
        self.bit(!ack);
    }

    pub fn stop(&mut self) {
        self.s.set_t(self.l.sda, self.t, false);
        self.t += self.half * 0.5;
        self.s.set_t(self.l.scl, self.t, true);
        self.t += self.half * 0.5;
        self.s.set_t(self.l.sda, self.t, true);
        self.t += self.half;
    }
}

/// Sends one CAN frame on `ch` (dominant = low) at `bitrate` from `t0`. Returns the end time
/// (after EOF and the 3-bit intermission).
pub fn can_tx(s: &mut Synth, ch: u8, t0: f64, bitrate: f64, spec: &CanFrameSpec) -> f64 {
    let bits = frame_bits(spec);
    let mut levels = bits;
    levels.extend([true; 3]);
    s.bits(ch, t0, 1.0 / bitrate, &levels)
}

/// An active error flag (6 dominant bits) and the error delimiter (8 recessive) from `t0`.
pub fn can_error_frame(s: &mut Synth, ch: u8, t0: f64, bitrate: f64) -> f64 {
    let mut levels = vec![false; 6];
    levels.extend([true; 8 + 3]);
    s.bits(ch, t0, 1.0 / bitrate, &levels)
}

/// UART line settings for Modbus RTU at `baud`, 8 data bits, even parity, 1 stop bit (the spec default).
pub fn modbus_uart(baud: f64, ch: u8) -> UartConfig {
    UartConfig { channel: ch, baud, data_bits: 8, parity: Parity::Even, stop_bits: 1.0, inverted: false, msb_first: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_levels_and_glitches() {
        let mut s = Synth::new(1000, [true, false, false, false, false, false, false, false]);
        s.set(0, 10, false);
        s.set(0, 20, true);
        s.glitch(1, 5, 2);
        let d = s.render(30);
        assert_eq!(d[9] & 1, 1);
        assert_eq!(d[10] & 1, 0);
        assert_eq!(d[19] & 1, 0);
        assert_eq!(d[20] & 1, 1);
        assert_eq!(d[4] & 2, 0);
        assert_eq!(d[5] & 2, 2);
        assert_eq!(d[6] & 2, 2);
        assert_eq!(d[7] & 2, 0);
    }
}
