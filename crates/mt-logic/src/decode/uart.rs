//! Asynchronous serial (UART, and the logic side of RS-232 and RS-485): start bit, 5 to 9 data
//! bits, optional parity, 1, 1.5 or 2 stop bits, normal or inverted idle level.
//!
//! Each bit is sampled at its centre, as a majority of three samples a sixteenth of a bit apart, so
//! a single-sample glitch inside a bit does not flip it. A start edge is only accepted if the line
//! is still at the start level half a bit later; anything shorter is reported as a glitch.

use serde::{Deserialize, Serialize};

use super::{Annotation, DecodeOutput, Level};
use crate::capture::Capture;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Parity {
    #[default]
    None,
    Even,
    Odd,
    Mark,
    Space,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UartConfig {
    pub channel: u8,
    pub baud: f64,
    pub data_bits: u8,
    pub parity: Parity,
    /// 1, 1.5 or 2.
    pub stop_bits: f32,
    /// Idle low (the line is inverted, as on the RS-232 side of a level shifter).
    #[serde(default)]
    pub inverted: bool,
    #[serde(default)]
    pub msb_first: bool,
}

impl Default for UartConfig {
    fn default() -> Self {
        Self { channel: 0, baud: 9600.0, data_bits: 8, parity: Parity::None, stop_bits: 1.0, inverted: false, msb_first: false }
    }
}

impl UartConfig {
    pub fn validate(&self, cap: &Capture) -> Result<(), String> {
        if !(5..=9).contains(&self.data_bits) {
            return Err("UART data bits must be 5 to 9".into());
        }
        if ![1.0, 1.5, 2.0].contains(&self.stop_bits) {
            return Err("UART stop bits must be 1, 1.5 or 2".into());
        }
        if self.baud <= 0.0 {
            return Err("the baud rate must be above zero".into());
        }
        let spb = cap.sample_rate as f64 / self.baud;
        if spb < 4.0 {
            return Err(format!(
                "{} baud needs a sample rate of at least {} (4 samples per bit); this capture has {}",
                self.baud,
                crate::fmt_rate((self.baud * 4.0) as u64),
                crate::fmt_rate(cap.sample_rate)
            ));
        }
        Ok(())
    }

    /// Bits on the wire per character, start and stop included.
    pub fn char_bits(&self) -> f64 {
        1.0 + self.data_bits as f64 + if self.parity == Parity::None { 0.0 } else { 1.0 } + self.stop_bits as f64
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UartChar {
    /// Sample of the start edge.
    pub start: u64,
    /// End of the (first) stop bit.
    pub end: u64,
    pub value: u16,
    /// Centre samples of the data bits, for bit-level annotations.
    pub bit_centres: Vec<u64>,
    /// `Some(false)` on a parity error.
    pub parity_ok: Option<bool>,
    pub framing_error: bool,
    /// The line stayed at the start level for the whole character (a break condition).
    pub brk: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UartResult {
    pub chars: Vec<UartChar>,
    /// Start edges rejected because the line went back to idle within half a bit.
    pub glitches: Vec<u64>,
}

impl UartResult {
    pub fn bytes(&self) -> Vec<u8> {
        self.chars.iter().map(|c| c.value as u8).collect()
    }

    pub fn errors(&self) -> usize {
        self.chars.iter().filter(|c| c.framing_error || c.parity_ok == Some(false)).count()
    }
}

/// Majority of three samples around `centre`.
fn vote(cap: &Capture, ch: u8, centre: f64, spread: f64) -> bool {
    let n = cap.len().saturating_sub(1) as f64;
    let at = |x: f64| cap.level(ch, x.clamp(0.0, n).round() as u64);
    let v = u8::from(at(centre - spread)) + u8::from(at(centre)) + u8::from(at(centre + spread));
    v >= 2
}

pub fn decode(cap: &Capture, cfg: &UartConfig) -> UartResult {
    decode_range(cap, cfg, 0, cap.len())
}

pub fn decode_range(cap: &Capture, cfg: &UartConfig, from: u64, to: u64) -> UartResult {
    let mut out = UartResult::default();
    let ch = cfg.channel;
    let bit = cap.sample_rate as f64 / cfg.baud;
    let spread = (bit / 16.0).max(1.0).min(bit / 4.0);
    let idle = !cfg.inverted;
    let to = to.min(cap.len());
    let mut pos = from;
    // Start searching from an idle stretch: if the line is active at `from`, wait for it to go idle.
    if pos < to && cap.level(ch, pos) != idle {
        match cap.next_edge(ch, pos) {
            Some(e) => pos = e,
            None => return out,
        }
    }
    let total = cfg.char_bits();
    let n_par = usize::from(cfg.parity != Parity::None);
    while pos < to {
        let Some(start) = cap.next_edge(ch, pos) else { break };
        if start >= to {
            break;
        }
        if cap.level(ch, start) == idle {
            // An edge back to idle (we were somewhere active); keep looking.
            pos = start;
            continue;
        }
        let s = start as f64;
        // The frame must fit in the capture.
        if s + (total - cfg.stop_bits as f64 + 0.5) * bit >= cap.len() as f64 {
            break;
        }
        if vote(cap, ch, s + bit / 2.0, spread) == idle {
            out.glitches.push(start);
            pos = cap.next_edge(ch, start).unwrap_or(to);
            continue;
        }
        let mut value = 0u16;
        let mut ones = 0u32;
        let mut bit_centres = Vec::with_capacity(cfg.data_bits as usize);
        let mut all_active = true;
        for i in 0..cfg.data_bits as usize {
            let c = s + (1.5 + i as f64) * bit;
            bit_centres.push(c.round() as u64);
            let line = vote(cap, ch, c, spread);
            let b = if cfg.inverted { !line } else { line };
            all_active &= line != idle;
            ones += u32::from(b);
            let idx = if cfg.msb_first { cfg.data_bits as usize - 1 - i } else { i };
            if b {
                value |= 1 << idx;
            }
        }
        let parity_ok = if n_par == 1 {
            let c = s + (1.5 + cfg.data_bits as f64) * bit;
            let line = vote(cap, ch, c, spread);
            all_active &= line != idle;
            let p = if cfg.inverted { !line } else { line };
            Some(match cfg.parity {
                Parity::Even => (ones + u32::from(p)).is_multiple_of(2),
                Parity::Odd => (ones + u32::from(p)) % 2 == 1,
                Parity::Mark => p,
                Parity::Space => !p,
                Parity::None => true,
            })
        } else {
            None
        };
        let stop_c = s + (1.5 + cfg.data_bits as f64 + n_par as f64) * bit;
        let stop_ok = vote(cap, ch, stop_c, spread) == idle;
        let brk = all_active && !stop_ok;
        let end = (s + (2.0 + cfg.data_bits as f64 + n_par as f64) * bit).round() as u64;
        out.chars.push(UartChar { start, end, value, bit_centres, parity_ok, framing_error: !stop_ok, brk });
        // Resume at the stop bit's centre: the next start edge can come right after it.
        pos = stop_c.round() as u64;
        if !stop_ok {
            // Wait until the line is idle again before looking for a start bit.
            let p = pos.min(cap.len().saturating_sub(1));
            if cap.level(ch, p) != idle {
                match cap.next_edge(ch, p) {
                    Some(e) => pos = e,
                    None => break,
                }
            }
        }
    }
    out
}

pub fn char_text(c: &UartChar) -> String {
    let v = c.value;
    let printable = (0x20..0x7F).contains(&v);
    if printable { format!("0x{v:02X} '{}'", v as u8 as char) } else { format!("0x{v:02X}") }
}

pub fn output(cfg: &UartConfig, r: &UartResult) -> DecodeOutput {
    let mut ann = Vec::new();
    for c in &r.chars {
        let (level, text) = if c.brk {
            (Level::Warning, "Break".to_string())
        } else if c.framing_error {
            (Level::Error, format!("{} framing error", char_text(c)))
        } else if c.parity_ok == Some(false) {
            (Level::Error, format!("{} parity error", char_text(c)))
        } else {
            (Level::Data, char_text(c))
        };
        let short = if c.brk { "BRK".to_string() } else { format!("{:02X}", c.value) };
        ann.push(Annotation::new(0, c.start, c.end, level, text, short));
    }
    for &g in &r.glitches {
        ann.push(Annotation::new(1, g, g + 1, Level::Warning, "Glitch (shorter than half a bit)", "!"));
    }
    let errors = r.errors();
    let summary = format!(
        "{} characters at {} baud{}{}",
        r.chars.len(),
        cfg.baud,
        if errors > 0 { format!(", {errors} errors") } else { String::new() },
        if r.glitches.is_empty() { String::new() } else { format!(", {} glitches", r.glitches.len()) }
    );
    DecodeOutput { rows: vec!["Data".into(), "Warnings".into()], annotations: ann, table: None, summary }
}

/// Standard baud rates, for [`estimate_baud`].
pub const STANDARD_BAUDS: [u32; 15] = [300, 600, 1200, 2400, 4800, 9600, 14400, 19200, 38400, 57600, 76800, 115200, 230400, 460800, 921600];

/// Guesses the baud rate of `ch` from its shortest pulses, snapped to a standard rate when within 5 %.
pub fn estimate_baud(cap: &Capture, ch: u8) -> Option<f64> {
    let edges: Vec<u64> = cap.edges(ch, 0, cap.len()).map(|e| e.0).take(20_000).collect();
    let mut widths: Vec<u64> = edges.windows(2).map(|w| w[1] - w[0]).filter(|&w| w > 1).collect();
    if widths.len() < 4 {
        return None;
    }
    widths.sort_unstable();
    // Ignore the shortest 2 % (glitches), then average the pulses close to the shortest.
    let base = widths[widths.len() / 50] as f64;
    let near: Vec<f64> = widths.iter().map(|&w| w as f64).filter(|&w| w < base * 1.5).collect();
    let bit = near.iter().sum::<f64>() / near.len() as f64;
    let raw = cap.sample_rate as f64 / bit;
    let snap = STANDARD_BAUDS.iter().map(|&b| b as f64).min_by(|a, b| ((a - raw).abs()).total_cmp(&(b - raw).abs()))?;
    Some(if (snap - raw).abs() / snap < 0.05 { snap } else { raw.round() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{Synth, UartFault, uart_tx};

    fn cfg(baud: f64, parity: Parity, stop: f32) -> UartConfig {
        UartConfig { channel: 0, baud, data_bits: 8, parity, stop_bits: stop, inverted: false, msb_first: false }
    }

    fn line(c: &UartConfig, baud: f64, rate: u64, bytes: &[u8], fault: UartFault) -> Capture {
        let mut s = Synth::new(rate, [!c.inverted; 8]);
        let end = uart_tx(&mut s, 0, 0.001, bytes, c, baud, 0.0, fault);
        s.extend_to(s.at(end + 0.002));
        s.capture(&[], 0)
    }

    #[test]
    fn decodes_every_format() {
        let text = b"Hello, RS-485 \x00\xFF\x55";
        for parity in [Parity::None, Parity::Even, Parity::Odd, Parity::Mark, Parity::Space] {
            for stop in [1.0, 1.5, 2.0] {
                for baud in [9600.0, 19200.0, 115200.0] {
                    let c = cfg(baud, parity, stop);
                    let cap = line(&c, baud, 2_000_000, text, UartFault::default());
                    let r = decode(&cap, &c);
                    assert_eq!(r.bytes(), text, "{parity:?} {stop} {baud}");
                    assert_eq!(r.errors(), 0);
                }
            }
        }
    }

    #[test]
    fn inverted_msb_first_and_seven_bits() {
        let c = UartConfig { channel: 0, baud: 9600.0, data_bits: 7, parity: Parity::Even, stop_bits: 2.0, inverted: true, msb_first: true };
        let cap = line(&c, 9600.0, 1_000_000, b"AT+OK", UartFault::default());
        let r = decode(&cap, &c);
        assert_eq!(r.bytes(), b"AT+OK");
        assert_eq!(r.errors(), 0);
    }

    #[test]
    fn nine_data_bits() {
        let c = UartConfig { data_bits: 9, ..cfg(19200.0, Parity::None, 1.0) };
        let mut s = Synth::new(1_000_000, [true; 8]);
        // 9-bit multidrop: the address byte has bit 8 set.
        let bits = crate::synth::uart_char_bits(&c, 0x1A5);
        let t = s.bits(0, 0.001, 1.0 / 19200.0, &bits);
        s.set_t(0, t, true);
        s.extend_to(s.at(t + 0.001));
        let r = decode(&s.capture(&[], 0), &c);
        assert_eq!(r.chars.len(), 1);
        assert_eq!(r.chars[0].value, 0x1A5);
    }

    #[test]
    fn flags_parity_and_framing_errors() {
        let c = cfg(9600.0, Parity::Even, 1.0);
        let cap = line(&c, 9600.0, 1_000_000, &[1, 2, 3, 4], UartFault { bad_parity_at: Some(1), bad_stop_at: Some(3) });
        let r = decode(&cap, &c);
        assert_eq!(r.bytes(), [1, 2, 3, 4]);
        assert_eq!(r.chars[1].parity_ok, Some(false));
        assert!(r.chars[3].framing_error);
        assert_eq!(r.errors(), 2);
        let out = output(&c, &r);
        assert!(out.annotations.iter().any(|a| a.text.contains("parity error")));
        assert!(out.annotations.iter().any(|a| a.text.contains("framing error")));
    }

    #[test]
    fn rejects_glitches_on_idle_and_inside_bits() {
        let c = cfg(9600.0, Parity::None, 1.0);
        let mut s = Synth::new(1_000_000, [true; 8]);
        let end = uart_tx(&mut s, 0, 0.002, b"OK", &c, 9600.0, 0.0005, UartFault::default());
        // A 1-sample glitch on the idle line before the data, and one inside the first data bit.
        s.glitch(0, 1000, 1);
        s.glitch(0, s.at(0.002 + 1.5 / 9600.0) + 3, 1);
        s.extend_to(s.at(end + 0.001));
        let r = decode(&s.capture(&[], 0), &c);
        assert_eq!(r.bytes(), b"OK");
        assert_eq!(r.glitches, vec![1000]);
    }

    #[test]
    fn wrong_baud_gives_errors_not_silence() {
        let c = cfg(9600.0, Parity::None, 1.0);
        let cap = line(&c, 9600.0, 1_000_000, b"Modbus RTU sample text", UartFault::default());
        let wrong = cfg(19200.0, Parity::None, 1.0);
        let r = decode(&cap, &wrong);
        assert_ne!(r.bytes(), b"Modbus RTU sample text");
        assert!(r.errors() > 0, "decoding at twice the baud rate must show framing errors");
        // ... and the estimate finds the real rate.
        assert_eq!(estimate_baud(&cap, 0), Some(9600.0));
    }

    #[test]
    fn break_condition() {
        let c = cfg(9600.0, Parity::None, 1.0);
        let mut s = Synth::new(1_000_000, [true; 8]);
        s.set_t(0, 0.001, false);
        s.set_t(0, 0.001 + 20.0 / 9600.0, true);
        let t = uart_tx(&mut s, 0, 0.005, b"A", &c, 9600.0, 0.0, UartFault::default());
        s.extend_to(s.at(t + 0.001));
        let r = decode(&s.capture(&[], 0), &c);
        assert!(r.chars[0].brk);
        assert_eq!(r.chars.last().unwrap().value, b'A' as u16);
        assert!(!r.chars.last().unwrap().framing_error);
    }

    #[test]
    fn rejects_undersampled_config() {
        let c = cfg(115200.0, Parity::None, 1.0);
        let cap = Capture::new(200_000, crate::capture::default_channels(8), vec![0xFF; 1000]);
        assert!(c.validate(&cap).unwrap_err().contains("4 samples per bit"));
    }
}
